//! The tokio NT4 server.
//!
//! [`Server::start`] listens for NT4 clients (port 5810 by default), the way
//! robot code's ntcore server does. Each connection runs in its own task;
//! the protocol itself is [`nt4_core::ServerCore`], shared by all of them.
//! The server's own code publishes and subscribes through the same typed
//! handles as [`Client`](crate::Client).
//!
//! ```no_run
//! # async fn demo() -> std::io::Result<()> {
//! use nt4::{Server, ServerConfig};
//!
//! let server = Server::start(ServerConfig::default()).await?;
//! let mode = server.publish::<String>("/robot/mode");
//! let speed = server.subscribe::<f64>("/dashboard/speed", Default::default());
//! mode.set("teleop".into());
//! loop {
//!     println!("speed = {:?}, clients = {}", speed.get(), server.clients().len());
//!     tokio::time::sleep(std::time::Duration::from_millis(100)).await;
//! }
//! # }
//! ```

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nt4_core::{
    ClientError, ClientId, ProtocolVersion, PubId, ServerCore, ServerError, ServerEvent, SubId,
    TopicValue, Transmit,
};
use nt4_proto::{
    BinaryMessage, BinaryReader, NtType, Properties, RTT_ID, SUBPROTOCOL_RTT, SUBPROTOCOL_V4_0,
    SUBPROTOCOL_V4_1, SubscriptionOptions, Value, encode_binary,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, mpsc, watch};
use tokio::time::Instant;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{
    Message,
    handshake::server::{ErrorResponse, Request, Response},
    http::{HeaderValue, StatusCode, header::SEC_WEBSOCKET_PROTOCOL},
    protocol::WebSocketConfig,
};

use crate::NtValue;
use crate::handles::{
    Backend, Events, GenericPublisher, Publisher, Subscriber, SubscriptionHandle,
};

/// Largest WebSocket message accepted, as in ntcore (2 MiB).
const MAX_MESSAGE_SIZE: usize = 2 * 1024 * 1024;

/// Where to listen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    /// Address to listen on; `0.0.0.0` (the default) is every IPv4
    /// interface.
    pub listen_address: IpAddr,
    /// TCP port; 5810 is robot code's, and 0 picks a free one (see
    /// [`Server::local_addr`]).
    pub port: u16,
}

impl Default for ServerConfig {
    /// Every interface, port 5810.
    fn default() -> Self {
        Self {
            listen_address: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: nt4_proto::ROBOT_CODE_PORT,
        }
    }
}

impl ServerConfig {
    /// Listen on every interface on `port`.
    ///
    /// ```
    /// let c = nt4::ServerConfig::port(5811);
    /// assert_eq!(c.port, 5811);
    /// ```
    pub fn port(port: u16) -> Self {
        Self {
            port,
            ..Self::default()
        }
    }
}

/// A connected client, from [`Server::clients`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectedClient {
    /// Its unique name, `<name>@<n>`.
    pub name: String,
    /// Its address.
    pub address: SocketAddr,
    /// The protocol version it speaks.
    pub version: ProtocolVersion,
}

struct Conn {
    info: ConnectedClient,
    /// Wakes the connection's task when it has frames to send.
    notify: Arc<Notify>,
}

struct State {
    core: ServerCore,
    conns: HashMap<ClientId, Conn>,
    /// Latest value per subscribed topic name, for typed subscribers.
    watchers: HashMap<String, watch::Sender<Option<TopicValue>>>,
    /// Queues for [`Server::events`] listeners.
    listeners: Vec<mpsc::UnboundedSender<ServerEvent>>,
}

struct Shared {
    state: Mutex<State>,
    start: Instant,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock leaves no broken invariant worth
        // stopping for, so keep going.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The server clock in microseconds, never 0 (0 means "default"). This
    /// is the server time clients sync to.
    fn now_us(&self) -> i64 {
        i64::try_from(self.start.elapsed().as_micros()).unwrap_or(i64::MAX) + 1
    }
}

impl State {
    /// Call after every change to the core: hand out its events and wake
    /// every connection that has frames to send.
    fn settle(&mut self) {
        while let Some(event) = self.core.poll_event() {
            if let ServerEvent::Value { name, value, .. } = &event
                && let Some(w) = self.watchers.get(&**name)
            {
                w.send_replace(Some(value.clone()));
            }
            // Drop listeners whose `Events` was dropped.
            self.listeners.retain(|l| l.send(event.clone()).is_ok());
        }
        for (id, conn) in &self.conns {
            if self.core.has_transmit(*id) {
                conn.notify.notify_one();
            }
        }
    }
}

fn to_client_error(e: ServerError) -> ClientError {
    match e {
        ServerError::UnknownPublisher(id) => ClientError::UnknownPublisher(id),
        ServerError::TypeMismatch { expected, got } => ClientError::TypeMismatch { expected, got },
    }
}

impl Backend for Shared {
    fn now_us(&self) -> i64 {
        Shared::now_us(self)
    }
    fn set_value(&self, id: PubId, value: Value, time_us: i64) -> Result<(), ClientError> {
        let mut state = self.lock();
        let r = state.core.set_value(id, value, time_us);
        state.settle();
        r.map_err(to_client_error)
    }
    fn unpublish(&self, id: PubId) {
        let mut state = self.lock();
        state.core.unpublish(id);
        state.settle();
    }
    fn unsubscribe(&self, id: SubId) {
        let mut state = self.lock();
        state.core.unsubscribe(id);
        state.settle();
    }
}

/// Stops the accept loop and every connection when the last [`Server`]
/// handle is dropped.
struct StopGuard(watch::Sender<bool>);

impl Drop for StopGuard {
    fn drop(&mut self) {
        let _ = self.0.send(true);
    }
}

/// An NT4 server running on the current tokio runtime. Cheap to clone; it
/// stops (closing every connection) when the last clone is dropped.
#[derive(Clone)]
pub struct Server {
    shared: Arc<Shared>,
    local_addr: SocketAddr,
    _stop: Arc<StopGuard>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("local_addr", &self.local_addr)
            .finish_non_exhaustive()
    }
}

impl Server {
    /// Start listening. Must be called inside a tokio runtime; it spawns the
    /// accept loop and returns once the port is bound.
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")] async fn main() {
    /// use nt4::{Server, ServerConfig};
    /// let server = Server::start(ServerConfig { port: 0, ..Default::default() }).await.unwrap();
    /// assert_ne!(server.local_addr().port(), 0);
    /// # }
    /// ```
    pub async fn start(config: ServerConfig) -> std::io::Result<Self> {
        let listener = TcpListener::bind((config.listen_address, config.port)).await?;
        let local_addr = listener.local_addr()?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                core: ServerCore::new(),
                conns: HashMap::new(),
                watchers: HashMap::new(),
                listeners: Vec::new(),
            }),
            start: Instant::now(),
        });
        let (stop_tx, stop_rx) = watch::channel(false);
        tokio::spawn(accept_loop(shared.clone(), listener, stop_rx));
        Ok(Self {
            shared,
            local_addr,
            _stop: Arc::new(StopGuard(stop_tx)),
        })
    }

    /// The address it listens on, with the real port when started with 0.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Publish a typed topic from the server.
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")] async fn main() {
    /// use nt4::{Server, ServerConfig};
    /// let server = Server::start(ServerConfig::port(0)).await.unwrap();
    /// let mode = server.publish::<String>("/robot/mode");
    /// mode.set("disabled".into()); // clients that subscribe get it
    /// # }
    /// ```
    pub fn publish<T: NtValue>(&self, name: &str) -> Publisher<T> {
        Publisher::new(self.publish_generic(name, T::nt_type(), Properties::new()))
    }

    /// Publish a topic with any type string and initial properties, for
    /// structs and other raw types, or to make a topic `retained` or
    /// `persistent`.
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")] async fn main() {
    /// use nt4::{NtType, Properties, Server, ServerConfig, Value};
    /// let server = Server::start(ServerConfig::port(0)).await.unwrap();
    /// let mut props = Properties::new();
    /// props.insert("retained".into(), true.into());
    /// let p = server.publish_generic("/config/name", NtType::String, props);
    /// p.set(Value::String("bot".into())).unwrap();
    /// # }
    /// ```
    pub fn publish_generic(
        &self,
        name: &str,
        ty: NtType,
        properties: Properties,
    ) -> GenericPublisher {
        let mut state = self.shared.lock();
        let id = state.core.publish(name, ty, properties);
        state.settle();
        drop(state);
        GenericPublisher::new(self.shared.clone(), id)
    }

    /// Subscribe from the server to one topic and keep its latest value.
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")] async fn main() {
    /// use nt4::{Server, ServerConfig};
    /// let server = Server::start(ServerConfig::port(0)).await.unwrap();
    /// let speed = server.subscribe::<f64>("/dashboard/speed", Default::default());
    /// assert_eq!(speed.get(), None); // until a client sets it
    /// # }
    /// ```
    pub fn subscribe<T: NtValue>(&self, name: &str, options: SubscriptionOptions) -> Subscriber<T> {
        let mut state = self.shared.lock();
        let rx = state
            .watchers
            .entry(name.to_owned())
            .or_insert_with(|| watch::channel(None).0)
            .subscribe();
        // The topic's last value, if any, arrives as an event right away.
        let id = state.core.subscribe(vec![name.to_owned()], options);
        state.settle();
        drop(state);
        Subscriber::new(SubscriptionHandle::new(self.shared.clone(), id), rx)
    }

    /// Subscribe from the server to several topics or prefixes (with
    /// `options.prefix`). Values arrive as [`ServerEvent::Value`] on
    /// [`events`](Self::events).
    pub fn subscribe_multiple(
        &self,
        topics: Vec<String>,
        options: SubscriptionOptions,
    ) -> SubscriptionHandle {
        let mut state = self.shared.lock();
        let id = state.core.subscribe(topics, options);
        state.settle();
        drop(state);
        SubscriptionHandle::new(self.shared.clone(), id)
    }

    /// Change a published topic's properties (`null` deletes one).
    pub fn set_properties(&self, name: &str, update: Properties) {
        let mut state = self.shared.lock();
        state.core.set_properties(name, update);
        state.settle();
    }

    /// Everything that happens to the server's own subscriptions and
    /// publishers from now on, plus warnings about what clients send, in
    /// order and without gaps. The queue has no size limit: read it
    /// steadily, or drop the [`Events`] to stop collecting.
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")] async fn main() {
    /// use nt4::{Server, ServerConfig, ServerEvent};
    /// let server = Server::start(ServerConfig::port(0)).await.unwrap();
    /// let mut events = server.events();
    /// let _p = server.publish::<i64>("/x");
    /// assert!(matches!(events.try_recv(), Some(ServerEvent::Announced { .. })));
    /// # }
    /// ```
    pub fn events(&self) -> Events<ServerEvent> {
        let (tx, events) = Events::new();
        self.shared.lock().listeners.push(tx);
        events
    }

    /// Send everything queued for every client now instead of on each
    /// client's next period.
    pub fn flush(&self) {
        let now = self.shared.now_us();
        let mut state = self.shared.lock();
        state.core.send_all_outgoing(now, true);
        state.settle();
    }

    /// The clients connected now, in connection order.
    pub fn clients(&self) -> Vec<ConnectedClient> {
        let state = self.shared.lock();
        state
            .core
            .clients()
            .filter_map(|id| state.conns.get(&id).map(|c| c.info.clone()))
            .collect()
    }

    /// The server clock, in microseconds since it started. This is the
    /// server time clients sync to, and the clock that timestamps passed to
    /// [`Publisher::set_at`] use.
    pub fn now_us(&self) -> i64 {
        self.shared.now_us()
    }
}

// ----- Connections -----

async fn accept_loop(shared: Arc<Shared>, listener: TcpListener, mut stop: watch::Receiver<bool>) {
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                // Errors here are per connection (e.g. too many open files);
                // keep listening.
                if let Ok((tcp, addr)) = accepted {
                    // TCP_NODELAY, as ntcore does: values are small frames.
                    let _ = tcp.set_nodelay(true);
                    tokio::spawn(connection(shared.clone(), tcp, addr, stop.clone()));
                }
            }
            _ = stop.changed() => return,
        }
    }
}

/// What the WebSocket handshake chose.
#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Nt4 {
        name: String,
        version: ProtocolVersion,
    },
    /// `rtt.networktables.first.wpi.edu`: time sync pings only, not a client.
    Rtt,
}

/// Why an upgrade request was refused: the HTTP status and a message.
type Rejection = (StatusCode, String);

/// Decode `%xx` escapes in a URL path segment, as ntcore's `UnescapeURI`
/// does (`+` stays `+` in a path). `None` for a bad escape or bad UTF-8.
fn unescape(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Check the upgrade request like ntcore: the path must be `/nt/<name>`,
/// and the subprotocol NT 4.1, NT 4.0 or RTT (in that order of preference).
fn choose(req: &Request) -> Result<(Kind, Option<&'static str>), Rejection> {
    let path = req.uri().path();
    let name = path
        .strip_prefix("/nt/")
        .and_then(unescape)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("invalid path '{path}', must match /nt/[clientId]"),
            )
        })?;
    let offered: Vec<&str> = req
        .headers()
        .get_all(SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    let has = |p: &str| offered.contains(&p);
    Ok(if has(SUBPROTOCOL_V4_1) {
        (
            Kind::Nt4 {
                name,
                version: ProtocolVersion::V4_1,
            },
            Some(SUBPROTOCOL_V4_1),
        )
    } else if has(SUBPROTOCOL_V4_0) {
        (
            Kind::Nt4 {
                name,
                version: ProtocolVersion::V4_0,
            },
            Some(SUBPROTOCOL_V4_0),
        )
    } else if has(SUBPROTOCOL_RTT) {
        (Kind::Rtt, Some(SUBPROTOCOL_RTT))
    } else if offered.is_empty() {
        // No subprotocol at all: ntcore treats it as NT 4.0.
        (
            Kind::Nt4 {
                name,
                version: ProtocolVersion::V4_0,
            },
            None,
        )
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("unsupported subprotocols {offered:?}"),
        ));
    })
}

/// ntcore 2027 treats peers on `127.x` as local and sends to them without
/// batching. IPv6 loopback counts too (ntcore listens on IPv4 only).
fn is_local(addr: &SocketAddr) -> bool {
    match addr.ip() {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
    }
}

async fn connection(
    shared: Arc<Shared>,
    tcp: TcpStream,
    addr: SocketAddr,
    stop: watch::Receiver<bool>,
) {
    let mut kind = None;
    // tungstenite fixes this callback's error type.
    #[allow(clippy::result_large_err)]
    let callback = |req: &Request, mut resp: Response| -> Result<Response, ErrorResponse> {
        let (k, protocol) = choose(req).map_err(|(status, msg)| {
            let mut r = ErrorResponse::new(Some(msg));
            *r.status_mut() = status;
            r
        })?;
        if let Some(p) = protocol {
            resp.headers_mut()
                .insert(SEC_WEBSOCKET_PROTOCOL, HeaderValue::from_static(p));
        }
        kind = Some(k);
        Ok(resp)
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_SIZE))
        .max_frame_size(Some(MAX_MESSAGE_SIZE));
    let Ok(ws) = tokio_tungstenite::accept_hdr_async_with_config(tcp, callback, Some(config)).await
    else {
        return; // not a valid upgrade; tungstenite has answered it
    };
    match kind {
        Some(Kind::Nt4 { name, version }) => {
            run_client(&shared, ws, addr, &name, version, stop).await;
        }
        Some(Kind::Rtt) => run_rtt(&shared, ws, stop).await,
        None => {}
    }
}

type Ws = WebSocketStream<TcpStream>;

/// Answer RTT pings until the connection closes.
async fn run_rtt(shared: &Shared, ws: Ws, mut stop: watch::Receiver<bool>) {
    let (mut sink, mut stream) = ws.split();
    loop {
        let msg = tokio::select! {
            m = stream.next() => m,
            _ = stop.changed() => {
                let _ = sink.send(Message::Close(None)).await;
                return;
            }
        };
        let data = match msg {
            Some(Ok(Message::Binary(b))) => b,
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
            // tungstenite queues pongs to pings; the flush below sends them.
            Some(Ok(_)) => {
                if sink.flush().await.is_err() {
                    return;
                }
                continue;
            }
        };
        let now = shared.now_us();
        for m in BinaryReader::new(&data) {
            let Ok(m) = m else {
                // As ntcore: a frame that doesn't decode ends the connection.
                let _ = sink.send(Message::Close(None)).await;
                return;
            };
            if m.id == RTT_ID {
                let mut out = Vec::new();
                encode_binary(
                    &mut out,
                    &BinaryMessage {
                        id: RTT_ID,
                        timestamp_us: now,
                        value: m.value,
                    },
                );
                if sink.send(Message::Binary(out.into())).await.is_err() {
                    return;
                }
            }
        }
    }
}

/// Run one NT4 client connection until it ends.
async fn run_client(
    shared: &Shared,
    ws: Ws,
    addr: SocketAddr,
    name: &str,
    version: ProtocolVersion,
    mut stop: watch::Receiver<bool>,
) {
    let notify = Arc::new(Notify::new());
    let id = {
        let mut state = shared.lock();
        let (id, unique) = state
            .core
            .add_client(name, &addr.to_string(), is_local(&addr), version);
        state.conns.insert(
            id,
            Conn {
                info: ConnectedClient {
                    name: unique,
                    address: addr,
                    version,
                },
                notify: notify.clone(),
            },
        );
        state.settle();
        id
    };

    let (mut sink, mut stream) = ws.split();
    let mut next_tick = Instant::now();
    'conn: loop {
        // Take frames only when the socket has room (this loop is not
        // waiting on a write), so while a slow client catches up its values
        // keep coalescing in the core's queue.
        let (out, period) = {
            let mut state = shared.lock();
            let out: Vec<Transmit> = std::iter::from_fn(|| state.core.poll_transmit(id)).collect();
            (out, state.core.tick_period_ms(id))
        };
        for t in out {
            let msg = match t {
                Transmit::Text(s) => Message::Text(s.into()),
                Transmit::Binary(b) => Message::Binary(b.into()),
                Transmit::Ping => Message::Ping(Vec::new().into()),
                Transmit::Close { .. } => {
                    let _ = sink.send(Message::Close(None)).await;
                    break 'conn;
                }
            };
            if sink.feed(msg).await.is_err() {
                break 'conn;
            }
        }
        if sink.flush().await.is_err() {
            break;
        }

        let period = Duration::from_millis(u64::from(period));
        tokio::select! {
            _ = stop.changed() => {
                let _ = sink.send(Message::Close(None)).await;
                break;
            }
            msg = stream.next() => {
                let now = shared.now_us();
                let mut state = shared.lock();
                match msg {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                    Some(Ok(Message::Text(t))) => state.core.handle_text(id, now, &t),
                    Some(Ok(Message::Binary(b))) => state.core.handle_binary(id, now, &b),
                    // tungstenite queues the pong to a ping; the flush above
                    // sends it. Either counts as hearing from the client.
                    Some(Ok(Message::Pong(_) | Message::Ping(_))) => state.core.handle_pong(id, now),
                    Some(Ok(Message::Frame(_))) => {}
                }
                state.settle();
            }
            _ = tokio::time::sleep_until(next_tick) => {
                let now = shared.now_us();
                let mut state = shared.lock();
                state.core.send_outgoing(id, now, false);
                state.settle();
                next_tick = Instant::now() + period;
            }
            _ = notify.notified() => {}
        }
    }

    let mut state = shared.lock();
    state.conns.remove(&id);
    state.core.remove_client(id);
    state.settle();
}

#[cfg(test)]
mod tests {
    // Written by Claude.
    use super::*;

    fn request(path: &str, protocols: Option<&str>) -> Request {
        let mut b = Request::builder().uri(path);
        if let Some(p) = protocols {
            b = b.header(SEC_WEBSOCKET_PROTOCOL, p);
        }
        b.body(()).unwrap()
    }

    #[test]
    fn handshake_prefers_4_1_then_4_0_then_rtt() {
        let both = format!("{SUBPROTOCOL_V4_0}, {SUBPROTOCOL_V4_1}");
        let (k, p) = choose(&request("/nt/dash", Some(&both))).unwrap();
        assert_eq!(
            k,
            Kind::Nt4 {
                name: "dash".into(),
                version: ProtocolVersion::V4_1
            }
        );
        assert_eq!(p, Some(SUBPROTOCOL_V4_1));
        let (k, _) = choose(&request("/nt/dash", Some(SUBPROTOCOL_V4_0))).unwrap();
        assert_eq!(
            k,
            Kind::Nt4 {
                name: "dash".into(),
                version: ProtocolVersion::V4_0
            }
        );
        let (k, p) = choose(&request("/nt/dash", Some(SUBPROTOCOL_RTT))).unwrap();
        assert_eq!((k, p), (Kind::Rtt, Some(SUBPROTOCOL_RTT)));
        let (k, p) = choose(&request("/nt/dash", None)).unwrap();
        assert_eq!(
            k,
            Kind::Nt4 {
                name: "dash".into(),
                version: ProtocolVersion::V4_0
            }
        );
        assert_eq!(p, None);
    }

    #[test]
    fn handshake_rejects_bad_paths_and_protocols() {
        for path in ["/", "/nt/", "/other/x", "/nt/%zz"] {
            let (status, _) = choose(&request(path, Some(SUBPROTOCOL_V4_1))).unwrap_err();
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        }
        let (status, _) = choose(&request("/nt/x", Some("chat"))).unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn names_are_unescaped() {
        let (k, _) = choose(&request("/nt/my%20dash", Some(SUBPROTOCOL_V4_1))).unwrap();
        assert_eq!(
            k,
            Kind::Nt4 {
                name: "my dash".into(),
                version: ProtocolVersion::V4_1
            }
        );
    }

    #[test]
    fn loopback_is_local() {
        assert!(is_local(&"127.0.0.1:1".parse().unwrap()));
        assert!(is_local(&"127.4.5.6:1".parse().unwrap()));
        assert!(is_local(&"[::1]:1".parse().unwrap()));
        assert!(is_local(&"[::ffff:127.0.0.1]:1".parse().unwrap()));
        assert!(!is_local(&"10.0.0.2:1".parse().unwrap()));
    }
}
