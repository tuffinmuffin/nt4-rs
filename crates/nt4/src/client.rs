//! The tokio NT4 client.
//!
//! [`Client::start`] spawns a task that connects to the first server that
//! answers, keeps the connection alive, and reconnects when it drops. The
//! protocol itself is [`nt4_core::ClientCore`]; this module only moves
//! frames between it and a WebSocket and keeps the clock.
//!
//! Publishers and subscribers are plain handles. They work before the first
//! connection and across reconnects; dropping one unpublishes or
//! unsubscribes.
//!
//! ```no_run
//! # async fn demo() {
//! use nt4::{Client, ClientConfig};
//!
//! let client = Client::start(ClientConfig::team("my-dashboard", 3200));
//! let speed = client.subscribe::<f64>("/SmartDashboard/speed", Default::default());
//! let heartbeat = client.publish::<i64>("/dashboard/heartbeat", Default::default());
//! for i in 0.. {
//!     heartbeat.set(i);
//!     println!("speed = {:?}", speed.get());
//!     tokio::time::sleep(std::time::Duration::from_millis(100)).await;
//! }
//! # }
//! ```

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nt4_core::{
    ClientCore, ClientError, Event, ProtocolVersion, PubId, PublishOptions, SubId, TopicValue,
    Transmit,
};
use nt4_proto::{NtType, Properties, SubscriptionOptions, Value};
use tokio::sync::{Notify, broadcast, watch};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::{
    self, Message,
    client::IntoClientRequest,
    http::{HeaderValue, header::SEC_WEBSOCKET_PROTOCOL},
};

use crate::NtValue;

/// How to find and identify with the server.
#[derive(Debug, Clone, PartialEq)]
pub struct ClientConfig {
    /// Client name, sent in the URL (`/nt/<name>`). ntcore servers add an
    /// `@<n>` suffix to keep names unique.
    pub name: String,
    /// Servers to try, in order, as (host, port).
    pub servers: Vec<(String, u16)>,
    /// How long to wait for each server to accept a connection.
    pub connect_timeout: Duration,
    /// How long to wait after every server failed before trying again.
    pub reconnect_delay: Duration,
}

impl ClientConfig {
    /// Connect to one server.
    ///
    /// ```
    /// let c = nt4::ClientConfig::server("sim-viewer", "127.0.0.1", 5810);
    /// assert_eq!(c.servers, vec![("127.0.0.1".to_string(), 5810)]);
    /// ```
    pub fn server(name: impl Into<String>, host: impl Into<String>, port: u16) -> Self {
        Self {
            name: name.into(),
            servers: vec![(host.into(), port)],
            connect_timeout: Duration::from_secs(1),
            reconnect_delay: Duration::from_secs(1),
        }
    }

    /// Connect to a team's robot, trying the addresses ntcore's
    /// `SetServerTeam` tries: `10.TE.AM.2`, mDNS names, and the USB address,
    /// all on port 5810.
    ///
    /// ```
    /// let c = nt4::ClientConfig::team("dash", 3200);
    /// assert_eq!(c.servers[0], ("10.32.0.2".to_string(), 5810));
    /// ```
    pub fn team(name: impl Into<String>, team: u16) -> Self {
        let port = nt4_proto::ROBOT_CODE_PORT;
        let hosts = [
            format!("10.{}.{}.2", team / 100, team % 100),
            format!("roborio-{team}-frc.local"),
            "172.22.11.2".to_string(),
            format!("roborio-{team}-frc.lan"),
            format!("roborio-{team}-frc.frc-field.local"),
        ];
        Self {
            servers: hosts.into_iter().map(|h| (h, port)).collect(),
            ..Self::server(name, "", port)
        }
    }
}

struct State {
    core: ClientCore,
    /// Latest value per subscribed topic name, for typed subscribers.
    watchers: HashMap<String, watch::Sender<Option<TopicValue>>>,
}

struct Shared {
    state: Mutex<State>,
    events: broadcast::Sender<Event>,
    flush: Notify,
    start: Instant,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock leaves no broken invariant worth
        // stopping for, so keep going.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Local monotonic time in microseconds, never 0 (0 means "default").
    fn now_us(&self) -> i64 {
        i64::try_from(self.start.elapsed().as_micros()).unwrap_or(i64::MAX) + 1
    }

    /// Take the core's pending events: update typed subscribers and send
    /// each to [`Client::events`] listeners.
    fn dispatch_events(&self, state: &mut State) {
        while let Some(event) = state.core.poll_event() {
            if let Event::Value { name, value, .. } = &event
                && let Some(w) = state.watchers.get(&**name)
            {
                w.send_replace(Some(value.clone()));
            }
            // No listeners is fine.
            let _ = self.events.send(event);
        }
    }
}

/// Stops the background task when the last [`Client`] handle is dropped.
struct StopGuard(watch::Sender<bool>);

impl Drop for StopGuard {
    fn drop(&mut self) {
        let _ = self.0.send(true);
    }
}

/// An NT4 client running on the current tokio runtime. Cheap to clone; the
/// connection stops when the last clone is dropped.
#[derive(Clone)]
pub struct Client {
    shared: Arc<Shared>,
    _stop: Arc<StopGuard>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").finish_non_exhaustive()
    }
}

impl Client {
    /// Start the client. Must be called inside a tokio runtime; it spawns
    /// the connection task and returns at once.
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")] async fn main() {
    /// use nt4::{Client, ClientConfig};
    /// // Port 1 has no server: handles work before any connection.
    /// let client = Client::start(ClientConfig::server("docs", "127.0.0.1", 1));
    /// assert!(!client.is_connected());
    /// # }
    /// ```
    pub fn start(config: ClientConfig) -> Self {
        let (events, _) = broadcast::channel(1024);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                core: ClientCore::new(),
                watchers: HashMap::new(),
            }),
            events,
            flush: Notify::new(),
            start: Instant::now(),
        });
        let (stop_tx, stop_rx) = watch::channel(false);
        tokio::spawn(run(shared.clone(), config, stop_rx));
        Self {
            shared,
            _stop: Arc::new(StopGuard(stop_tx)),
        }
    }

    /// Publish a typed topic.
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")] async fn main() {
    /// use nt4::{Client, ClientConfig};
    /// // Port 1 has no server: handles work before any connection.
    /// let client = Client::start(ClientConfig::server("docs", "127.0.0.1", 1));
    /// let speed = client.publish::<f64>("/robot/speed", Default::default());
    /// speed.set(1.5); // sent once connected
    /// # }
    /// ```
    pub fn publish<T: NtValue>(&self, name: &str, options: PublishOptions) -> Publisher<T> {
        Publisher {
            inner: self.publish_generic(name, T::nt_type(), Properties::new(), options),
            _ty: PhantomData,
        }
    }

    /// Publish a topic with any type string and initial properties, for
    /// structs (`struct:Pose2d`) and other raw types.
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")] async fn main() {
    /// use nt4::{Client, ClientConfig};
    /// // Port 1 has no server: handles work before any connection.
    /// let client = Client::start(ClientConfig::server("docs", "127.0.0.1", 1));
    /// use nt4::{NtType, Properties, Value};
    /// let ty = NtType::Struct { name: "Translation2d".into(), array: false };
    /// let target = client.publish_generic("/target", ty, Properties::new(), Default::default());
    /// let bytes = [1.0f64.to_le_bytes(), 2.0f64.to_le_bytes()].concat();
    /// target.set(Value::Raw(bytes)).unwrap();
    /// # }
    /// ```
    pub fn publish_generic(
        &self,
        name: &str,
        ty: NtType,
        properties: Properties,
        options: PublishOptions,
    ) -> GenericPublisher {
        let id = self
            .shared
            .lock()
            .core
            .publish(name, ty, properties, options);
        GenericPublisher {
            shared: self.shared.clone(),
            id,
        }
    }

    /// Subscribe to one topic and keep its latest value.
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")] async fn main() {
    /// use nt4::{Client, ClientConfig};
    /// // Port 1 has no server: handles work before any connection.
    /// let client = Client::start(ClientConfig::server("docs", "127.0.0.1", 1));
    /// let speed = client.subscribe::<f64>("/robot/speed", Default::default());
    /// println!("{}", speed.get_or(0.0)); // 0.0 until a value arrives
    /// # }
    /// ```
    pub fn subscribe<T: NtValue>(&self, name: &str, options: SubscriptionOptions) -> Subscriber<T> {
        let mut state = self.shared.lock();
        let id = state.core.subscribe(vec![name.to_owned()], options);
        // Start from the value we already have, if any.
        let current = state.core.topic_by_name(name).and_then(|t| t.value.clone());
        let rx = state
            .watchers
            .entry(name.to_owned())
            .or_insert_with(|| watch::channel(current).0)
            .subscribe();
        drop(state);
        Subscriber {
            handle: SubscriptionHandle {
                shared: self.shared.clone(),
                id,
            },
            rx,
            _ty: PhantomData,
        }
    }

    /// Subscribe to several topics or prefixes (with `options.prefix`).
    /// Values arrive as [`Event::Value`] on [`events`](Self::events).
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")] async fn main() {
    /// use nt4::{Client, ClientConfig};
    /// // Port 1 has no server: handles work before any connection.
    /// let client = Client::start(ClientConfig::server("docs", "127.0.0.1", 1));
    /// use nt4::SubscriptionOptions;
    /// let options = SubscriptionOptions { prefix: true, ..Default::default() };
    /// let _sub = client.subscribe_multiple(vec!["/SmartDashboard/".into()], options);
    /// let mut events = client.events(); // values arrive here as Event::Value
    /// # }
    /// ```
    pub fn subscribe_multiple(
        &self,
        topics: Vec<String>,
        options: SubscriptionOptions,
    ) -> SubscriptionHandle {
        let id = self.shared.lock().core.subscribe(topics, options);
        SubscriptionHandle {
            shared: self.shared.clone(),
            id,
        }
    }

    /// Change a topic's properties on the server (`null` deletes one).
    pub fn set_properties(&self, name: &str, update: Properties) {
        self.shared.lock().core.set_properties(name, update);
    }

    /// Everything that happens from now on: connections, announcements,
    /// values, time sync and warnings. A listener that falls more than 1024
    /// events behind misses the oldest ones.
    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.shared.events.subscribe()
    }

    /// Send everything queued now instead of on the next period.
    pub fn flush(&self) {
        self.shared.flush.notify_one();
    }

    /// Whether a server connection is open.
    pub fn is_connected(&self) -> bool {
        self.shared.lock().core.is_connected()
    }

    /// Server time minus local time, in microseconds, once known.
    pub fn server_time_offset(&self) -> Option<i64> {
        self.shared.lock().core.server_time_offset()
    }

    /// The current server time in microseconds, once the offset is known.
    pub fn server_time(&self) -> Option<i64> {
        let now = self.shared.now_us();
        self.shared.lock().core.server_time(now)
    }

    /// Local time as this client counts it, in microseconds since it
    /// started. Timestamps passed to [`Publisher::set_at`] use this clock.
    pub fn now_us(&self) -> i64 {
        self.shared.now_us()
    }

    /// Wait until connected and time-synced, or until `timeout` passes.
    /// Returns whether it got there.
    pub async fn wait_for_sync(&self, timeout: Duration) -> bool {
        let mut events = self.events();
        let deadline = Instant::now() + timeout;
        loop {
            if self.server_time_offset().is_some() {
                return true;
            }
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Err(_) => return self.server_time_offset().is_some(),
                Ok(Err(broadcast::error::RecvError::Closed)) => return false,
                Ok(_) => {}
            }
        }
    }
}

/// A publisher of any type, from [`Client::publish_generic`]. Dropping it
/// unpublishes.
pub struct GenericPublisher {
    shared: Arc<Shared>,
    id: PubId,
}

impl std::fmt::Debug for GenericPublisher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenericPublisher")
            .field("id", &self.id)
            .finish()
    }
}

impl GenericPublisher {
    /// Set the value, timestamped now.
    pub fn set(&self, value: Value) -> Result<(), ClientError> {
        let now = self.shared.now_us();
        self.set_at(value, now)
    }

    /// Set the value with a timestamp from [`Client::now_us`]; 0 sets it as
    /// a default that any other value overrides.
    pub fn set_at(&self, value: Value, time_us: i64) -> Result<(), ClientError> {
        self.shared.lock().core.set_value(self.id, value, time_us)
    }
}

impl Drop for GenericPublisher {
    fn drop(&mut self) {
        self.shared.lock().core.unpublish(self.id);
    }
}

/// A typed publisher, from [`Client::publish`]. Dropping it unpublishes.
#[derive(Debug)]
pub struct Publisher<T> {
    inner: GenericPublisher,
    _ty: PhantomData<fn(T)>,
}

impl<T: NtValue> Publisher<T> {
    /// Set the value, timestamped now.
    pub fn set(&self, value: T) {
        // The type always matches, so this cannot fail.
        let _ = self.inner.set(value.into_value());
    }

    /// Set the value with a timestamp from [`Client::now_us`].
    pub fn set_at(&self, value: T, time_us: i64) {
        let _ = self.inner.set_at(value.into_value(), time_us);
    }

    /// Set a default: sent with timestamp 0, so any other publisher's value
    /// wins over it.
    pub fn set_default(&self, value: T) {
        let _ = self.inner.set_at(value.into_value(), 0);
    }
}

/// A subscription; dropping it unsubscribes.
pub struct SubscriptionHandle {
    shared: Arc<Shared>,
    id: SubId,
}

impl std::fmt::Debug for SubscriptionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubscriptionHandle")
            .field("id", &self.id)
            .finish()
    }
}

impl Drop for SubscriptionHandle {
    fn drop(&mut self) {
        self.shared.lock().core.unsubscribe(self.id);
    }
}

/// Returned by [`Subscriber::changed`] after the client stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientStopped;

impl std::fmt::Display for ClientStopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the NT client has stopped")
    }
}

impl std::error::Error for ClientStopped {}

/// A typed subscriber to one topic, from [`Client::subscribe`].
#[derive(Debug)]
pub struct Subscriber<T> {
    handle: SubscriptionHandle,
    rx: watch::Receiver<Option<TopicValue>>,
    _ty: PhantomData<fn() -> T>,
}

impl<T: NtValue> Subscriber<T> {
    /// The latest value, or `None` if nothing (of type `T`) has arrived.
    /// The last value is kept after a disconnect.
    pub fn get(&self) -> Option<T> {
        self.rx
            .borrow()
            .as_ref()
            .and_then(|v| T::from_value(&v.value))
    }

    /// The latest value, or `default`.
    pub fn get_or(&self, default: T) -> T {
        self.get().unwrap_or(default)
    }

    /// The latest value with its timestamps.
    pub fn get_timestamped(&self) -> Option<(T, TopicValue)> {
        let v = self.rx.borrow().clone()?;
        Some((T::from_value(&v.value)?, v))
    }

    /// Wait for a value newer than the latest one when called, then return
    /// it (`Ok(None)` if it has another type than `T`). Errors once the
    /// client has stopped.
    pub async fn changed(&mut self) -> Result<Option<T>, ClientStopped> {
        // Only values that arrive after this call count.
        self.rx.borrow_and_update();
        self.rx.changed().await.map_err(|_| ClientStopped)?;
        Ok(self.get())
    }

    /// The handle that keeps this subscription alive.
    pub fn handle(&self) -> &SubscriptionHandle {
        &self.handle
    }
}

// ----- Connection task -----

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn run(shared: Arc<Shared>, config: ClientConfig, mut stop: watch::Receiver<bool>) {
    loop {
        for (host, port) in &config.servers {
            if *stop.borrow() {
                return;
            }
            let url = nt4_proto::client_url(host, *port, &config.name);
            let attempt = tokio::time::timeout(config.connect_timeout, connect(&url));
            let (ws, version) = tokio::select! {
                r = attempt => match r {
                    Ok(Ok(c)) => c,
                    _ => continue,
                },
                _ = stop.changed() => return,
            };
            let reason = run_connection(&shared, ws, version, &mut stop).await;
            {
                let mut state = shared.lock();
                state.core.disconnect(&reason);
                shared.dispatch_events(&mut state);
            }
            if *stop.borrow() {
                return;
            }
            break; // after a connection ends, start again from the first server
        }
        tokio::select! {
            _ = tokio::time::sleep(config.reconnect_delay) => {}
            _ = stop.changed() => return,
        }
    }
}

async fn connect(url: &str) -> Result<(Ws, ProtocolVersion), tungstenite::Error> {
    let mut request = url.into_client_request()?;
    let protocols = format!(
        "{}, {}",
        nt4_proto::SUBPROTOCOL_V4_1,
        nt4_proto::SUBPROTOCOL_V4_0
    );
    request.headers_mut().insert(
        SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_str(&protocols).expect("constant header is valid"),
    );
    let (ws, response) = tokio_tungstenite::connect_async(request).await?;
    let version = response
        .headers()
        .get(SEC_WEBSOCKET_PROTOCOL)
        .and_then(|v| v.to_str().ok())
        .and_then(ProtocolVersion::from_subprotocol)
        .unwrap_or(ProtocolVersion::V4_0);
    Ok((ws, version))
}

/// Run one connection until it ends; returns why it ended.
async fn run_connection(
    shared: &Shared,
    ws: Ws,
    version: ProtocolVersion,
    stop: &mut watch::Receiver<bool>,
) -> String {
    let (mut sink, mut stream) = ws.split();
    shared.lock().core.connect(shared.now_us(), version);
    let mut next_tick = Instant::now();
    loop {
        let out = {
            let mut state = shared.lock();
            shared.dispatch_events(&mut state);
            std::iter::from_fn(|| state.core.poll_transmit()).collect::<Vec<_>>()
        };
        for t in out {
            let msg = match t {
                Transmit::Text(s) => Message::Text(s.into()),
                Transmit::Binary(b) => Message::Binary(b.into()),
                Transmit::Ping => Message::Ping(Vec::new().into()),
                Transmit::Close { reason } => {
                    let _ = sink.send(Message::Close(None)).await;
                    return reason;
                }
            };
            if let Err(e) = sink.feed(msg).await {
                return e.to_string();
            }
        }
        if let Err(e) = sink.flush().await {
            return e.to_string();
        }

        let period = Duration::from_millis(u64::from(shared.lock().core.tick_period_ms()));
        tokio::select! {
            _ = stop.changed() => {
                let _ = sink.send(Message::Close(None)).await;
                return "client stopped".into();
            }
            msg = stream.next() => {
                let now = shared.now_us();
                match msg {
                    None => return "connection closed".into(),
                    Some(Err(e)) => return e.to_string(),
                    Some(Ok(Message::Text(t))) => shared.lock().core.handle_text(now, &t),
                    Some(Ok(Message::Binary(b))) => shared.lock().core.handle_binary(now, &b),
                    Some(Ok(Message::Pong(_))) => shared.lock().core.handle_pong(now),
                    // tungstenite queues the pong; the flush above sends it.
                    Some(Ok(Message::Ping(_))) => shared.lock().core.handle_pong(now),
                    Some(Ok(Message::Close(_))) => return "closed by server".into(),
                    Some(Ok(Message::Frame(_))) => {}
                }
            }
            _ = tokio::time::sleep_until(next_tick) => {
                shared.lock().core.send_outgoing(shared.now_us(), false);
                next_tick = Instant::now() + period;
            }
            _ = shared.flush.notified() => {
                shared.lock().core.send_outgoing(shared.now_us(), true);
            }
        }
    }
}
