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

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures_util::{FutureExt, SinkExt, StreamExt};
use nt4_core::{
    ClientCore, ClientError, Event, ProtocolVersion, PubId, PublishOptions, SubId, Topic,
    TopicValue, Transmit,
};
use nt4_proto::{NtType, Properties, SubscriptionOptions, Value};
use tokio::sync::{Notify, mpsc, watch};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::{
    self, Message,
    client::IntoClientRequest,
    http::{HeaderValue, header::SEC_WEBSOCKET_PROTOCOL},
};

use crate::NtValue;
use crate::handles::{
    Backend, Events, GenericPublisher, Publisher, Subscriber, SubscriptionHandle,
};

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
    /// Queues for [`Client::events`] listeners.
    listeners: Vec<mpsc::UnboundedSender<Event>>,
    /// Schemas published so far (by name, without `/.schema/`).
    schemas: HashSet<String>,
}

struct Shared {
    state: Mutex<State>,
    flush: Notify,
    /// A publish, subscribe or property change is queued: send it without
    /// waiting for the next tick.
    wake: Notify,
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
            if let Event::Value { name, value, .. } = &event {
                if let Some(w) = state.watchers.get(&**name) {
                    w.send_replace(Some(value.clone()));
                }
            }
            // Drop listeners whose `Events` was dropped.
            state.listeners.retain(|l| l.send(event.clone()).is_ok());
        }
    }
}

impl Backend for Shared {
    fn now_us(&self) -> i64 {
        Shared::now_us(self)
    }
    fn set_value(&self, id: PubId, value: Value, time_us: i64) -> Result<(), ClientError> {
        self.lock().core.set_value(id, value, time_us)
    }
    fn unpublish(&self, id: PubId) {
        self.lock().core.unpublish(id);
        self.wake.notify_one();
    }
    fn unsubscribe(&self, id: SubId) {
        self.lock().core.unsubscribe(id);
        self.wake.notify_one();
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
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                core: ClientCore::new(),
                watchers: HashMap::new(),
                listeners: Vec::new(),
                schemas: HashSet::new(),
            }),
            flush: Notify::new(),
            wake: Notify::new(),
            start: Instant::now(),
        });
        let (stop_tx, stop_rx) = watch::channel(false);
        tokio::spawn(run(shared.clone(), config, stop_rx));
        Self {
            shared,
            _stop: Arc::new(StopGuard(stop_tx)),
        }
    }

    /// Publish a typed topic. For a struct or protobuf type, its schemas
    /// are published first (see [`add_schema`](Self::add_schema)).
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")] async fn main() {
    /// use nt4::{Client, ClientConfig};
    /// // Port 1 has no server: handles work before any connection.
    /// let client = Client::start(ClientConfig::server("docs", "127.0.0.1", 1));
    /// let speed = client.publish::<f64>("/robot/speed", Default::default());
    /// speed.set(1.5); // sent once connected
    ///
    /// // `Counts` implements `NtStruct` (see `nt4::structs`).
    /// # use std::borrow::Cow;
    /// # use nt4::structs::{NtStruct, StructReader};
    /// # #[derive(Clone, Default)]
    /// # struct Counts { good: i32, bad: i32 }
    /// # impl NtStruct for Counts {
    /// #     const SIZE: usize = 8;
    /// #     fn type_name() -> Cow<'static, str> { "Counts".into() }
    /// #     fn schema() -> Cow<'static, str> { "int32 good;int32 bad".into() }
    /// #     fn pack(&self, out: &mut Vec<u8>) {
    /// #         out.extend(self.good.to_le_bytes());
    /// #         out.extend(self.bad.to_le_bytes());
    /// #     }
    /// #     fn unpack(data: &[u8]) -> Self {
    /// #         let mut r = StructReader::new(data);
    /// #         Counts { good: r.read(), bad: r.read() }
    /// #     }
    /// # }
    /// let counts = client.publish::<Counts>("/vision/counts", Default::default());
    /// assert!(client.has_schema("struct:Counts"));
    /// counts.set(Counts::default());
    /// # }
    /// ```
    pub fn publish<T: NtValue>(&self, name: &str, options: PublishOptions) -> Publisher<T> {
        T::for_each_schema(&mut |name, ty, data| self.add_schema(name, ty, data));
        Publisher::new(self.publish_generic(name, T::nt_type(), T::properties(), options))
    }

    /// Publish a data schema on `/.schema/<name>` (retained, with the schema
    /// as a default value), once: later calls with the same name do
    /// nothing, as with ntcore's `AddSchema`. [`publish`](Self::publish)
    /// does this for struct and protobuf types; call it yourself for raw
    /// topics published with [`publish_generic`](Self::publish_generic).
    ///
    /// The schema stays published until the client stops.
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")] async fn main() {
    /// use nt4::{Client, ClientConfig};
    /// let client = Client::start(ClientConfig::server("docs", "127.0.0.1", 1));
    /// client.add_schema("struct:Point", "structschema", b"double x;double y");
    /// assert!(client.has_schema("struct:Point"));
    /// # }
    /// ```
    pub fn add_schema(&self, name: &str, ty: &str, schema: &[u8]) {
        let mut state = self.shared.lock();
        if !state.schemas.insert(name.to_owned()) {
            return;
        }
        let id = state.core.publish(
            format!("{}{name}", crate::structs::SCHEMA_PREFIX),
            NtType::from(ty),
            crate::structs::schema_properties(),
            PublishOptions::default(),
        );
        // The type is raw (id 5) whatever the schema type, so this can't fail.
        let _ = state.core.set_value(id, Value::Raw(schema.to_vec()), 0);
        drop(state);
        self.shared.wake.notify_one();
    }

    /// Whether [`add_schema`](Self::add_schema) (or a typed
    /// [`publish`](Self::publish)) has published schema `name`.
    pub fn has_schema(&self, name: &str) -> bool {
        self.shared.lock().schemas.contains(name)
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
        self.shared.wake.notify_one();
        GenericPublisher::new(self.shared.clone(), id)
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
        self.shared.wake.notify_one();
        Subscriber::new(SubscriptionHandle::new(self.shared.clone(), id), rx)
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
        self.shared.wake.notify_one();
        SubscriptionHandle::new(self.shared.clone(), id)
    }

    /// Change a topic's properties on the server (`null` deletes one).
    pub fn set_properties(&self, name: &str, update: Properties) {
        self.shared.lock().core.set_properties(name, update);
        self.shared.wake.notify_one();
    }

    /// Everything that happens from now on: connections, announcements,
    /// values, time sync and warnings, in order and without gaps.
    ///
    /// Like ntcore's listener queues, the queue has no size limit: read it
    /// steadily, or drop the [`Events`] to stop collecting.
    pub fn events(&self) -> Events {
        let (tx, events) = Events::new();
        self.shared.lock().listeners.push(tx);
        events
    }

    /// Send everything queued now instead of on the next period.
    pub fn flush(&self) {
        self.shared.flush.notify_one();
    }

    /// Whether a server connection is open.
    pub fn is_connected(&self) -> bool {
        self.shared.lock().core.is_connected()
    }

    /// A topic the server announced, with its newest value, if any. Values
    /// arrive only for topics a subscription covers.
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")] async fn main() {
    /// use nt4::{Client, ClientConfig};
    /// // Port 1 has no server, so nothing is announced.
    /// let client = Client::start(ClientConfig::server("docs", "127.0.0.1", 1));
    /// assert!(client.topic("/robot/speed").is_none());
    /// # }
    /// ```
    pub fn topic(&self, name: &str) -> Option<Topic> {
        self.shared.lock().core.topic_by_name(name).cloned()
    }

    /// Every topic the server announced, with its newest value, in no set
    /// order. Copies them all: for values as they arrive, use
    /// [`events`](Self::events) or [`subscribe`](Self::subscribe).
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")] async fn main() {
    /// use nt4::{Client, ClientConfig};
    /// let client = Client::start(ClientConfig::server("docs", "127.0.0.1", 1));
    /// for topic in client.topics() {
    ///     println!("{} ({:?}): {:?}", topic.name, topic.ty, topic.value);
    /// }
    /// # }
    /// ```
    pub fn topics(&self) -> Vec<Topic> {
        self.shared.lock().core.topics().cloned().collect()
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
                Ok(None) => return false,
                Ok(Some(_)) => {}
            }
        }
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
    // TCP_NODELAY, as ntcore does: values are small frames that should not
    // wait for the previous one's ACK.
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .read_buffer_size(crate::READ_BUFFER_SIZE);
    let (ws, response) =
        tokio_tungstenite::connect_async_with_config(request, Some(config), true).await?;
    let version = response
        .headers()
        .get(SEC_WEBSOCKET_PROTOCOL)
        .and_then(|v| v.to_str().ok())
        .and_then(ProtocolVersion::from_subprotocol)
        .unwrap_or(ProtocolVersion::V4_0);
    Ok((ws, version))
}

/// Most frames handled per wakeup before the timers and wakeups get a turn.
const MAX_BATCH: usize = 256;

/// Run one connection until it ends; returns why it ended.
async fn run_connection(
    shared: &Shared,
    ws: Ws,
    version: ProtocolVersion,
    stop: &mut watch::Receiver<bool>,
) -> String {
    let (mut sink, mut stream) = ws.split();
    shared.lock().core.connect(shared.now_us(), version);
    // One timer for the whole connection, moved forward when it fires;
    // creating one per received frame costs a timer-wheel insert each time.
    let tick = tokio::time::sleep_until(Instant::now());
    tokio::pin!(tick);
    let mut out = Vec::new();
    // tungstenite queues the pong to a ping; the next flush sends it.
    let mut pong_queued = false;
    loop {
        let period = {
            let mut state = shared.lock();
            shared.dispatch_events(&mut state);
            out.extend(std::iter::from_fn(|| state.core.poll_transmit()));
            Duration::from_millis(u64::from(state.core.tick_period_ms()))
        };
        if !out.is_empty() || pong_queued {
            for t in out.drain(..) {
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
            pong_queued = false;
        }

        tokio::select! {
            _ = stop.changed() => {
                let _ = sink.send(Message::Close(None)).await;
                return "client stopped".into();
            }
            msg = stream.next() => {
                let now = shared.now_us();
                let mut state = shared.lock();
                let mut msg = msg;
                // Handle every frame tungstenite already has, under one lock,
                // before going back around the loop: on localhost ntcore
                // sends each value in its own frame.
                let mut handled = 0;
                loop {
                    match msg {
                        None => return "connection closed".into(),
                        Some(Err(e)) => return e.to_string(),
                        Some(Ok(Message::Text(t))) => state.core.handle_text(now, &t),
                        Some(Ok(Message::Binary(b))) => state.core.handle_binary(now, &b),
                        Some(Ok(Message::Pong(_))) => state.core.handle_pong(now),
                        Some(Ok(Message::Ping(_))) => {
                            state.core.handle_pong(now);
                            pong_queued = true;
                        }
                        Some(Ok(Message::Close(_))) => return "closed by server".into(),
                        Some(Ok(Message::Frame(_))) => {}
                    }
                    // Stop before taking a frame off the stream, never
                    // after: a frame taken here must be handled.
                    handled += 1;
                    if handled == MAX_BATCH {
                        break;
                    }
                    match stream.next().now_or_never() {
                        Some(m) => msg = m,
                        None => break,
                    }
                }
            }
            () = &mut tick => {
                shared.lock().core.send_outgoing(shared.now_us(), false);
                tick.as_mut().reset(Instant::now() + period);
            }
            _ = shared.wake.notified() => {
                shared.lock().core.send_outgoing(shared.now_us(), false);
            }
            _ = shared.flush.notified() => {
                shared.lock().core.send_outgoing(shared.now_us(), true);
                // A flush within the minimum period of the last send waits
                // in the queue; come back as soon as it can go.
                let soon = Instant::now()
                    + Duration::from_millis(u64::from(nt4_core::outgoing::MIN_PERIOD_MS));
                if tick.deadline() > soon {
                    tick.as_mut().reset(soon);
                }
            }
        }
    }
}
