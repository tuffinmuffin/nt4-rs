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

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};
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
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
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

/// Where to listen, and where to keep persistent values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    /// Address to listen on; `0.0.0.0` (the default) is every IPv4
    /// interface.
    pub listen_address: IpAddr,
    /// TCP port; 5810 is robot code's, and 0 picks a free one (see
    /// [`Server::local_addr`]).
    pub port: u16,
    /// The file for topics with the `persistent` property, in ntcore's
    /// format (robot code uses `networktables.json`). Loaded at start and
    /// saved within a second of a change, and when the server stops.
    /// `None` (the default) keeps them in memory only.
    pub persist_file: Option<PathBuf>,
}

impl Default for ServerConfig {
    /// Every interface, port 5810, no persistent file.
    fn default() -> Self {
        Self {
            listen_address: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: nt4_proto::ROBOT_CODE_PORT,
            persist_file: None,
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
    /// Schemas published so far (by name, without `/.schema/`).
    schemas: HashSet<String>,
}

struct Shared {
    state: Mutex<State>,
    start: Instant,
    /// Problems loading the persistent file at start.
    load_errors: Vec<String>,
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
/// handle is dropped, and saves persistent values one last time.
struct StopGuard {
    stop: watch::Sender<bool>,
    persist: Option<(Arc<Shared>, PathBuf)>,
}

impl Drop for StopGuard {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        // ntcore saves only on its 1 s timer; save what changed since, so
        // the last second isn't lost. It's a small file, written here
        // rather than in a task the runtime may no longer run.
        if let Some((shared, path)) = &self.persist
            && let Some(data) = shared.take_persistent()
        {
            let _ = save_persistent(path, &data);
        }
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
        let start = Instant::now();
        let mut core = ServerCore::new();
        let mut load_errors = Vec::new();
        if let Some(path) = config.persist_file.clone() {
            let (data, notes) = tokio::task::spawn_blocking(move || read_persistent(&path))
                .await
                .unwrap_or_default();
            load_errors = notes;
            let now = i64::try_from(start.elapsed().as_micros()).unwrap_or(i64::MAX) + 1;
            load_errors.extend(core.load_persistent(&data, now));
        }
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                core,
                conns: HashMap::new(),
                watchers: HashMap::new(),
                listeners: Vec::new(),
                schemas: HashSet::new(),
            }),
            start,
            load_errors,
        });
        let (stop_tx, stop_rx) = watch::channel(false);
        tokio::spawn(accept_loop(shared.clone(), listener, stop_rx.clone()));
        let persist = config.persist_file.map(|path| {
            tokio::spawn(save_loop(shared.clone(), path.clone(), stop_rx));
            (shared.clone(), path)
        });
        Ok(Self {
            shared,
            local_addr,
            _stop: Arc::new(StopGuard {
                stop: stop_tx,
                persist,
            }),
        })
    }

    /// Problems found loading the persistent file at start, in ntcore's
    /// words (`"<index>: <error>"` for a bad entry). Entries with a problem
    /// were skipped; the rest loaded.
    pub fn persistent_load_errors(&self) -> &[String] {
        &self.shared.load_errors
    }

    /// The address it listens on, with the real port when started with 0.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Publish a typed topic from the server. For a struct or protobuf
    /// type, its schemas are published first (see
    /// [`add_schema`](Self::add_schema)).
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")] async fn main() {
    /// use nt4::{Server, ServerConfig};
    /// let server = Server::start(ServerConfig::port(0)).await.unwrap();
    /// let mode = server.publish::<String>("/robot/mode");
    /// mode.set("disabled".into()); // clients that subscribe get it
    ///
    /// use nt4::wpimath::SwerveModuleVelocity;
    /// let modules = server.publish::<Vec<SwerveModuleVelocity>>("/drive/modules");
    /// assert!(server.has_schema("struct:Rotation2d")); // nested schemas too
    /// modules.set(vec![SwerveModuleVelocity::default(); 4]);
    /// # }
    /// ```
    pub fn publish<T: NtValue>(&self, name: &str) -> Publisher<T> {
        T::for_each_schema(&mut |name, ty, data| self.add_schema(name, ty, data));
        Publisher::new(self.publish_generic(name, T::nt_type(), Properties::new()))
    }

    /// Publish a data schema on `/.schema/<name>` (retained, with the schema
    /// as a default value), once: later calls with the same name do
    /// nothing, as with ntcore's `AddSchema`. [`publish`](Self::publish)
    /// does this for struct and protobuf types; call it yourself for raw
    /// topics published with [`publish_generic`](Self::publish_generic).
    ///
    /// The schema stays published until the server stops.
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")] async fn main() {
    /// use nt4::{Server, ServerConfig};
    /// let server = Server::start(ServerConfig::port(0)).await.unwrap();
    /// server.add_schema("struct:Point", "structschema", b"double x;double y");
    /// assert!(server.has_schema("struct:Point"));
    /// # }
    /// ```
    pub fn add_schema(&self, name: &str, ty: &str, schema: &[u8]) {
        let mut state = self.shared.lock();
        if !state.schemas.insert(name.to_owned()) {
            return;
        }
        let id = state.core.publish(
            &format!("{}{name}", crate::structs::SCHEMA_PREFIX),
            NtType::from(ty),
            crate::structs::schema_properties(),
        );
        // The type is raw (id 5) whatever the schema type, so this can't fail.
        let _ = state.core.set_value(id, Value::Raw(schema.to_vec()), 0);
        state.settle();
    }

    /// Whether [`add_schema`](Self::add_schema) (or a typed
    /// [`publish`](Self::publish)) has published schema `name`.
    pub fn has_schema(&self, name: &str) -> bool {
        self.shared.lock().schemas.contains(name)
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

// ----- Persistent file -----

impl Shared {
    /// The persistent values to save, if they changed since the last time.
    fn take_persistent(&self) -> Option<String> {
        let mut state = self.lock();
        state
            .core
            .persistent_changed()
            .then(|| state.core.dump_persistent())
    }

    /// Tell [`Server::events`] listeners about a problem outside the core.
    fn warn(&self, msg: String) {
        self.lock()
            .listeners
            .retain(|l| l.send(ServerEvent::Warning(msg.clone())).is_ok());
    }
}

/// Save once a second when something changed, as ntcore does.
async fn save_loop(shared: Arc<Shared>, path: PathBuf, mut stop: watch::Receiver<bool>) {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = tick.tick() => {}
            _ = stop.changed() => return,
        }
        if let Some(data) = shared.take_persistent() {
            let p = path.clone();
            if let Ok(Err(e)) =
                tokio::task::spawn_blocking(move || save_persistent(&p, &data)).await
            {
                shared.warn(format!(
                    "could not write persistent file '{}': {e}",
                    path.display()
                ));
            }
        }
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = OsString::from(path.as_os_str());
    s.push(suffix);
    s.into()
}

/// ntcore opens the file in text mode, so it has CRLF line ends on
/// Windows; write it the same way.
fn platform_newlines(data: &str) -> String {
    if cfg!(windows) {
        data.replace('\n', "\r\n")
    } else {
        data.to_owned()
    }
}

/// Read the persistent file as ntcore's `LoadPersistent` does: if a save
/// was interrupted after the old file became `<file>.bck`, restore that
/// first. If the file can't be read, back it up to `<file>.bak` and write
/// an empty list so this doesn't happen again. Returns the contents and
/// notes about what went wrong.
fn read_persistent(path: &Path) -> (String, Vec<String>) {
    let mut notes = Vec::new();
    let bck = with_suffix(path, ".bck");
    if !path.exists()
        && bck.exists()
        && let Err(e) = std::fs::rename(&bck, path)
    {
        notes.push(format!("failed to restore backup '{}': {e}", bck.display()));
    }
    match std::fs::read(path) {
        // Bad UTF-8 becomes replacement characters (and those entries
        // likely fail to parse) rather than losing the whole file.
        Ok(bytes) => (String::from_utf8_lossy(&bytes).into_owned(), notes),
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                notes.push(format!(
                    "could not open persistent file '{}': {e}",
                    path.display()
                ));
                let _ = std::fs::copy(path, with_suffix(path, ".bak"));
            }
            let _ = std::fs::write(path, platform_newlines("[]\n"));
            (String::new(), notes)
        }
    }
}

/// Save as ntcore's `SavePersistent` does: write `<file>.tmp`, move the old
/// file to `<file>.bck`, then move the new one in place (and put the old
/// one back if that fails).
fn save_persistent(path: &Path, data: &str) -> std::io::Result<()> {
    let tmp = with_suffix(path, ".tmp");
    if let Err(e) = std::fs::write(&tmp, platform_newlines(data)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    let bck = with_suffix(path, ".bck");
    let _ = std::fs::remove_file(&bck);
    let _ = std::fs::rename(path, &bck);
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::rename(&bck, path);
        return Err(e);
    }
    Ok(())
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

// ----- Plain HTTP -----

/// Largest HTTP request head read before giving up on a connection.
const MAX_REQUEST_HEAD: usize = 8 * 1024;

/// Read up to the end of the request head (the blank line). `None` if the
/// peer closes first or the head is too long.
async fn read_head(tcp: &mut TcpStream) -> Option<Vec<u8>> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = tcp.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        let searched = buf.len().saturating_sub(3);
        buf.extend_from_slice(&chunk[..n]);
        if buf[searched..].windows(4).any(|w| w == b"\r\n\r\n") {
            return Some(buf);
        }
        if buf.len() > MAX_REQUEST_HEAD {
            return None;
        }
    }
}

/// A parsed request line, plus whether the headers ask for a WebSocket.
struct HttpRequest<'a> {
    method: &'a str,
    target: &'a str,
    version: &'a str,
    upgrade: bool,
}

fn parse_head(head: &[u8]) -> Option<HttpRequest<'_>> {
    let text = std::str::from_utf8(head).ok()?;
    let mut lines = text.split("\r\n");
    let mut parts = lines.next()?.split(' ');
    let (method, target, version) = (parts.next()?, parts.next()?, parts.next()?);
    let version = version.strip_prefix("HTTP/")?;
    if parts.next().is_some() || version.len() != 3 {
        return None;
    }
    // wpinet's HttpWebSocketServerConnection takes the WebSocket path when
    // there's an `Upgrade: websocket` header; everything else is plain HTTP.
    let upgrade = lines.take_while(|l| !l.is_empty()).any(|l| {
        l.split_once(':').is_some_and(|(name, value)| {
            name.trim().eq_ignore_ascii_case("upgrade")
                && value.trim().eq_ignore_ascii_case("websocket")
        })
    });
    Some(HttpRequest {
        method,
        target,
        version,
        upgrade,
    })
}

/// Answer a plain HTTP request as ntcore's `ServerConnection4::ProcessRequest`
/// does: a short page at `/`, the persistent values at `/nt/persistent.json`,
/// and 404 for anything else. AdvantageScope fetches `/` and only opens its
/// WebSocket once that answers, so this is needed, not a nicety.
fn http_response(shared: &Shared, req: &HttpRequest) -> Vec<u8> {
    let path = req
        .target
        .strip_prefix("http://")
        .map(|rest| rest.find('/').map_or("/", |i| &rest[i..]))
        .unwrap_or(req.target);
    let path = path.split(['?', '#']).next().unwrap_or("");
    let get = req.method == "GET";
    let (code, reason, content_type, body) = if !path.starts_with('/') {
        // ntcore's 400 page says "400: Not Found!"; say what it is.
        (
            400,
            "Bad Request",
            "text/plain",
            "400: Bad Request!\r\n".to_string(),
        )
    } else if get && path == "/" {
        (
            200,
            "OK",
            "text/html",
            "<html><head><title>NetworkTables</title></head>\
             <body><p>WebSockets must be used to access NetworkTables.\
             </body></html>"
                .to_string(),
        )
    } else if get && path == "/nt/persistent.json" {
        (
            200,
            "OK",
            "application/json",
            shared.lock().core.dump_persistent(),
        )
    } else {
        (
            404,
            "Not Found",
            "text/plain",
            "404: Not Found!\r\nResource not found".to_string(),
        )
    };
    // wpinet's headers, except that we always close after one response
    // (wpinet keeps HTTP/1.1 connections open for more requests; nobody
    // needs that here).
    let mut out = format!(
        "HTTP/{} {code} {reason}\r\n\
         Connection: close\r\n\
         Server: WebServer/1.0\r\n\
         Cache-Control: no-store, no-cache, must-revalidate, pre-check=0, post-check=0, max-age=0\r\n\
         Pragma: no-cache\r\n\
         Expires: Mon, 3 Jan 2000 12:34:56 GMT\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Methods: *\r\n\
         \r\n",
        req.version,
        body.len()
    )
    .into_bytes();
    // ntcore sends the body for HEAD too, which HTTP forbids; leave it off.
    if req.method != "HEAD" {
        out.extend_from_slice(body.as_bytes());
    }
    out
}

/// The TCP stream with the request head we already read put back in front,
/// so tungstenite can do the WebSocket handshake from the start.
struct Prefixed {
    head: Vec<u8>,
    pos: usize,
    tcp: TcpStream,
}

impl AsyncRead for Prefixed {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = &mut *self;
        if this.pos < this.head.len() {
            let n = buf.remaining().min(this.head.len() - this.pos);
            buf.put_slice(&this.head[this.pos..this.pos + n]);
            this.pos += n;
            if this.pos == this.head.len() {
                this.head = Vec::new();
                this.pos = 0;
            }
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.tcp).poll_read(cx, buf)
    }
}

impl AsyncWrite for Prefixed {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.tcp).poll_write(cx, data)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.tcp).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.tcp).poll_shutdown(cx)
    }
}

async fn connection(
    shared: Arc<Shared>,
    mut tcp: TcpStream,
    addr: SocketAddr,
    stop: watch::Receiver<bool>,
) {
    let Some(head) = read_head(&mut tcp).await else {
        return;
    };
    match parse_head(&head) {
        // Unparseable: wpinet just closes the connection.
        None => return,
        Some(req) if !req.upgrade => {
            let response = http_response(&shared, &req);
            let _ = tcp.write_all(&response).await;
            let _ = tcp.shutdown().await;
            return;
        }
        Some(_) => {}
    }
    let tcp = Prefixed { head, pos: 0, tcp };
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

type Ws = WebSocketStream<Prefixed>;

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
