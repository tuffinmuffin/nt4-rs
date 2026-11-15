//! The NT4 server state machine, with no sockets.
//!
//! [`ServerCore`] holds every topic, the last value of each, and the
//! publishers and subscriptions of every connected client. It decides what
//! each client is sent and when, and keeps the server's meta topics
//! (`$clients`, `$pub$<topic>`, `$sub$<topic>`, `$clientpub$<client>` and so
//! on) up to date. The caller owns the sockets and the clock:
//!
//! 1. When a WebSocket client connects, call
//!    [`add_client`](ServerCore::add_client). It returns the client's id and
//!    its unique name (`<name>@<n>`).
//! 2. Pass every frame it sends to [`handle_text`](ServerCore::handle_text)
//!    or [`handle_binary`](ServerCore::handle_binary), and its pongs to
//!    [`handle_pong`](ServerCore::handle_pong).
//! 3. Every [`tick_period_ms`](ServerCore::tick_period_ms) of that client,
//!    call [`send_outgoing`](ServerCore::send_outgoing) for it (or
//!    [`send_all_outgoing`](ServerCore::send_all_outgoing) for everyone),
//!    then send what [`poll_transmit`](ServerCore::poll_transmit) returns.
//!    Values, announcements and RTT replies for a client can also appear
//!    after any `handle_*` or local call, so drain after those too.
//! 4. When the socket closes, call
//!    [`remove_client`](ServerCore::remove_client). Its topics that nobody
//!    else publishes are deleted.
//!
//! The server's own code publishes and subscribes through the same core
//! ([`publish`](ServerCore::publish), [`subscribe`](ServerCore::subscribe),
//! ...), as client 0 does in ntcore; what it receives comes out of
//! [`poll_event`](ServerCore::poll_event).
//!
//! All times are the caller's monotonic clock in integer microseconds,
//! which is also the server time sent to clients.
//!
//! The behavior is ported from ntcore's `ServerImpl`, `ServerStorage` and
//! `ServerClient4`; `tests/server.rs` ports ntcore's `ServerImplTest`.
//!
//! ```
//! use nt4_core::{ProtocolVersion, ServerCore, Transmit};
//! use nt4_proto::{decode_server_text, BinaryReader, NtType, Properties, ServerMessage, Value};
//!
//! let mut server = ServerCore::new();
//! let speed = server.publish("/robot/speed", NtType::Double, Properties::new());
//! server.set_value(speed, Value::Double(2.5), 1_000).unwrap();
//!
//! let (dash, name) = server.add_client("dashboard", "10.0.0.5:51234", false, ProtocolVersion::V4_1);
//! assert_eq!(name, "dashboard@1");
//! server.handle_text(dash, 2_000, r#"[{"method":"subscribe","params":{"topics":["/robot/"],"subuid":1,"options":{"prefix":true}}}]"#);
//!
//! server.send_outgoing(dash, 3_000, false);
//! assert_eq!(server.poll_transmit(dash), Some(Transmit::Ping));
//! let Some(Transmit::Text(announce)) = server.poll_transmit(dash) else { panic!() };
//! let ServerMessage::Announce { name, id, .. } = &decode_server_text(&announce).messages[0] else { panic!() };
//! assert_eq!(name, "/robot/speed");
//! // The topic's last value follows its announcement.
//! let Some(Transmit::Binary(value)) = server.poll_transmit(dash) else { panic!() };
//! let value = BinaryReader::new(&value).next().unwrap().unwrap();
//! assert_eq!((value.id, value.timestamp_us, value.value), (*id, 1_000, Value::Double(2.5)));
//! ```

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use nt4_proto::{
    BinaryMessage, BinaryReader, ClientMessage, NtType, Properties, RTT_ID, ServerMessage,
    SubscriptionOptions, Value, decode_client_text, encode_binary,
};
use serde_json::Value as Json;

use crate::Transmit;
use crate::client::{PING_INTERVAL_MS, PING_TIMEOUT_MS, ProtocolVersion, PubId, SubId, TopicValue};
use crate::frames::{FrameBuffer, queue_ms, to_ms};
use crate::meta::Writer;
use crate::outgoing::{self, DEFAULT_PERIOD_MS, OutgoingQueue, SendMode};
use crate::persistent;

/// The slot of the server's own (local) client.
const LOCAL: usize = 0;

/// Freed topic ids are reused once this many are waiting, oldest first, as
/// in ntcore's `UidVector`. The ids a client sees match ntcore's.
const TOPIC_ID_REUSE_THRESHOLD: usize = 16;

/// A connected client, returned by [`ServerCore::add_client`].
///
/// Ids are reused: after [`remove_client`](ServerCore::remove_client), a
/// later client can get the same one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientId(usize);

impl ClientId {
    /// The client's slot number, the `<n>` in its name `<name>@<n>`.
    ///
    /// ```
    /// use nt4_core::{ProtocolVersion, ServerCore};
    /// let mut server = ServerCore::new();
    /// let (id, name) = server.add_client("cam", "10.0.0.9:5000", false, ProtocolVersion::V4_1);
    /// assert_eq!(name, format!("cam@{}", id.index()));
    /// ```
    pub fn index(self) -> usize {
        self.0
    }
}

/// Why a [`ServerCore`] local call was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ServerError {
    /// The publisher was unpublished, or never existed.
    #[error("unknown publisher {0}")]
    UnknownPublisher(i32),
    /// The value's type doesn't match the publisher's type.
    #[error("value type {got} does not match topic type {expected}")]
    TypeMismatch {
        /// The publisher's type.
        expected: NtType,
        /// The binary type id of the value given.
        got: u8,
    },
}

/// Something that happened, read with [`ServerCore::poll_event`].
///
/// Everything but [`Warning`](Event::Warning) is for the server's own
/// subscriptions and publishers.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A topic the server subscribes to now exists, or the server published
    /// it.
    Announced {
        /// Topic name.
        name: Arc<str>,
        /// Topic id (the one clients see too).
        id: i32,
        /// Topic type.
        ty: NtType,
        /// Topic properties.
        properties: Properties,
        /// Set when this answers one of the server's own `publish` calls.
        publisher: Option<PubId>,
    },
    /// An announced topic was deleted.
    Unannounced {
        /// Topic name.
        name: Arc<str>,
        /// The id it had.
        id: i32,
    },
    /// An announced topic's properties changed. `null` values mean deleted.
    Properties {
        /// Topic name.
        name: String,
        /// The changed properties.
        update: Properties,
        /// True when this answers the server's own `set_properties`.
        ack: bool,
    },
    /// A client set a value on a topic the server subscribes to.
    Value {
        /// Topic name.
        name: Arc<str>,
        /// Topic id.
        id: i32,
        /// The value. Local and server time are the same on the server.
        value: TopicValue,
    },
    /// A client sent something that couldn't be used, or the server ignored
    /// a request. The text matches ntcore's log messages.
    Warning(String),
}

#[derive(Debug, Clone)]
struct Publisher {
    topic: usize,
    ty: NtType,
}

#[derive(Debug, Clone)]
struct Subscriber {
    topics: Vec<String>,
    options: SubscriptionOptions,
    /// The requested period in ms, unrounded (ntcore's `periodicMs`); only
    /// the meta topics show it.
    periodic_ms: u32,
    /// The period used for sending: rounded to 10 ms and at least 5 ms.
    period_ms: u32,
}

impl Subscriber {
    fn new(topics: Vec<String>, options: SubscriptionOptions) -> Self {
        // As ntcore's decoder sets it: periodic * 1000, truncated. `as`
        // saturates; the decoder has already rejected out-of-range values.
        let periodic_ms = match options.periodic {
            None => DEFAULT_PERIOD_MS,
            Some(p) => (p * 1000.0) as u32,
        };
        let period_ms = outgoing::round_periodic_ms(periodic_ms).max(outgoing::MIN_PERIOD_MS);
        Self {
            topics,
            options,
            periodic_ms,
            period_ms,
        }
    }

    /// Whether this subscription covers a topic. An empty prefix matches
    /// every topic except the meta topics, which need `$` or longer.
    fn matches(&self, name: &str, special: bool) -> bool {
        self.topics.iter().any(|t| {
            if self.options.prefix {
                (!special || !t.is_empty()) && name.starts_with(t.as_str())
            } else {
                name == t
            }
        })
    }
}

/// The connection of a network client.
#[derive(Debug)]
struct Remote {
    version: ProtocolVersion,
    conn_info: String,
    outgoing: OutgoingQueue<ServerMessage>,
    wire: FrameBuffer,
    // NT 4.1 WebSocket keepalive, as ntcore's NetworkPing.
    next_ping_ms: u64,
    ping_sent_ms: Option<u64>,
    last_received_ms: Option<u64>,
    /// A close was sent; nothing more goes out.
    closing: bool,
}

#[derive(Debug)]
struct Client {
    name: String,
    /// `None` for the server's own client.
    remote: Option<Remote>,
    /// GCD of the subscription periods; `u32::MAX` with none.
    period_ms: u32,
    publishers: BTreeMap<i32, Publisher>,
    subscribers: BTreeMap<i32, Subscriber>,
    announce_sent: HashMap<usize, bool>,
    meta_pub: Option<usize>,
    meta_sub: Option<usize>,
}

/// One client's publishers and subscribers of one topic.
#[derive(Debug, Clone)]
struct TopicClient {
    publishers: BTreeSet<i32>,
    subscribers: BTreeSet<i32>,
    send_mode: SendMode,
}

impl Default for TopicClient {
    fn default() -> Self {
        Self {
            publishers: BTreeSet::new(),
            subscribers: BTreeSet::new(),
            send_mode: SendMode::Disabled,
        }
    }
}

impl TopicClient {
    /// Send every value if any value subscription asks for all, the newest
    /// per period if any wants values, else nothing.
    fn recalculate_send_mode(&mut self, subs: &BTreeMap<i32, Subscriber>) {
        self.send_mode = SendMode::Disabled;
        for o in self.subscribers.iter().filter_map(|uid| subs.get(uid)) {
            if o.options.topics_only {
                continue;
            }
            if o.options.all {
                self.send_mode = SendMode::All;
                return;
            }
            self.send_mode = SendMode::Normal;
        }
    }
}

#[derive(Debug)]
struct Topic {
    name: Arc<str>,
    ty: NtType,
    properties: Properties,
    /// (time in µs, value).
    last_value: Option<(i64, Value)>,
    /// Who set the last value; `None` for the server itself (meta topics).
    last_value_client: Option<usize>,
    publisher_count: usize,
    persistent: bool,
    retained: bool,
    cached: bool,
    /// A meta topic.
    special: bool,
    clients: BTreeMap<usize, TopicClient>,
    meta_pub: Option<usize>,
    meta_sub: Option<usize>,
}

impl Topic {
    fn is_published(&self) -> bool {
        self.persistent || self.retained || self.publisher_count != 0
    }

    /// Re-read `persistent`, `retained` and `cached`. Returns ntcore's
    /// warning, if any.
    fn refresh_properties(&mut self) -> Option<String> {
        let flag = |k: &str| self.properties.get(k).and_then(Json::as_bool);
        self.persistent = flag("persistent").unwrap_or(false);
        self.retained = flag("retained").unwrap_or(false);
        self.cached = flag("cached").unwrap_or(true);
        if !self.cached {
            self.last_value = None;
            self.last_value_client = None;
        }
        (!self.cached && self.persistent).then(|| {
            format!(
                "topic {}: disabling cached property disables persistent storage",
                self.name
            )
        })
    }
}

/// The NT4 server state machine. See the [module docs](self) for how to
/// drive it.
#[derive(Debug)]
pub struct ServerCore {
    /// Slot 0 is the server's own client.
    clients: Vec<Option<Client>>,
    topics: Vec<Option<Topic>>,
    free_topic_ids: VecDeque<usize>,
    topic_names: HashMap<Arc<str>, usize>,
    meta_clients: usize,
    next_pubuid: i32,
    next_subuid: i32,
    events: VecDeque<Event>,
    /// A persistent topic's value or properties changed since the last
    /// [`ServerCore::persistent_changed`].
    persistent_changed: bool,
}

impl Default for ServerCore {
    fn default() -> Self {
        Self::new()
    }
}

fn new_client(name: String, remote: Option<Remote>) -> Client {
    Client {
        name,
        remote,
        period_ms: u32::MAX,
        publishers: BTreeMap::new(),
        subscribers: BTreeMap::new(),
        announce_sent: HashMap::new(),
        meta_pub: None,
        meta_sub: None,
    }
}

fn meta_properties() -> Properties {
    let mut p = Properties::new();
    p.insert("retained".into(), Json::Bool(true));
    p
}

fn wire_version(v: ProtocolVersion) -> u64 {
    match v {
        ProtocolVersion::V4_0 => 0x0400,
        ProtocolVersion::V4_1 => 0x0401,
    }
}

impl ServerCore {
    /// A server with only its meta topics and no clients.
    ///
    /// ```
    /// let server = nt4_core::ServerCore::new();
    /// assert_eq!(server.clients().count(), 0);
    /// ```
    pub fn new() -> Self {
        let mut s = Self {
            clients: vec![Some(new_client(String::new(), None))],
            topics: Vec::new(),
            free_topic_ids: VecDeque::new(),
            topic_names: HashMap::new(),
            meta_clients: 0,
            next_pubuid: 1,
            next_subuid: 1,
            events: VecDeque::new(),
            persistent_changed: false,
        };
        // Same order as ntcore, so topic ids match: the local client's meta
        // topics, then $clients.
        let meta_pub = s.create_meta_topic("$serverpub");
        let meta_sub = s.create_meta_topic("$serversub");
        let local = s.client_mut(LOCAL);
        local.meta_pub = Some(meta_pub);
        local.meta_sub = Some(meta_sub);
        s.update_meta_client_pub(LOCAL);
        s.update_meta_client_sub(LOCAL);
        s.meta_clients = s.create_meta_topic("$clients");
        s
    }

    // ----- Clients -----

    /// A client connected. `name` is from its URL (`/nt/<name>`; empty
    /// becomes `NT4`), `conn_info` is shown in `$clients` (typically
    /// `host:port`), and `local` means it is on the same machine, so
    /// everything is sent to it immediately instead of batched (ntcore 2027
    /// does this for peers on `127.x`).
    ///
    /// Returns the client's id and its unique name, `<name>@<n>`.
    ///
    /// ```
    /// use nt4_core::{ProtocolVersion, ServerCore};
    /// let mut server = ServerCore::new();
    /// let (_, a) = server.add_client("sim", "127.0.0.1:1", true, ProtocolVersion::V4_1);
    /// let (_, b) = server.add_client("sim", "127.0.0.1:2", true, ProtocolVersion::V4_1);
    /// assert_eq!((a.as_str(), b.as_str()), ("sim@1", "sim@2"));
    /// ```
    pub fn add_client(
        &mut self,
        name: &str,
        conn_info: &str,
        local: bool,
        version: ProtocolVersion,
    ) -> (ClientId, String) {
        let name = if name.is_empty() { "NT4" } else { name };
        let slot = match self.clients.iter().position(Option::is_none) {
            Some(i) => i,
            None => {
                self.clients.push(None);
                self.clients.len() - 1
            }
        };
        let dedup = format!("{name}@{slot}");
        // ntcore creates the client's meta topics before the client is in
        // its slot; the ids come out the same either way.
        let meta_pub = self.create_meta_topic(&format!("$clientpub${dedup}"));
        let meta_sub = self.create_meta_topic(&format!("$clientsub${dedup}"));
        let mut client = new_client(
            dedup.clone(),
            Some(Remote {
                version,
                conn_info: conn_info.to_owned(),
                outgoing: OutgoingQueue::new(local),
                wire: FrameBuffer::default(),
                next_ping_ms: 0,
                ping_sent_ms: None,
                last_received_ms: None,
                closing: false,
            }),
        );
        client.meta_pub = Some(meta_pub);
        client.meta_sub = Some(meta_sub);
        self.clients[slot] = Some(client);
        self.update_meta_client_pub(slot);
        self.update_meta_client_sub(slot);
        self.update_meta_clients();
        (ClientId(slot), dedup)
    }

    /// A client disconnected. Its publishers and subscriptions end, and its
    /// topics that are no longer published (and not persistent or retained)
    /// are deleted. Frames not yet taken from
    /// [`poll_transmit`](Self::poll_transmit) are dropped.
    pub fn remove_client(&mut self, id: ClientId) {
        let slot = id.0;
        if slot == LOCAL || self.remote(slot).is_none() {
            return;
        }
        let mut to_delete = Vec::new();
        for tid in 0..self.topics.len() {
            let Some(t) = self.topics[tid].as_mut() else {
                continue;
            };
            // ntcore compares client pointers here; a later client in the
            // same slot must not count as the one that set the value.
            if t.last_value_client == Some(slot) {
                t.last_value_client = None;
            }
            let (pub_changed, sub_changed) = match t.clients.remove(&slot) {
                Some(tcd) => {
                    t.publisher_count -= tcd.publishers.len();
                    (!tcd.publishers.is_empty(), !tcd.subscribers.is_empty())
                }
                None => (false, false),
            };
            if !t.is_published() {
                to_delete.push(tid);
            } else {
                if pub_changed {
                    self.update_meta_topic_pub(tid);
                }
                if sub_changed {
                    self.update_meta_topic_sub(tid);
                }
            }
        }
        for tid in to_delete {
            self.delete_topic(tid);
        }
        let client = self.clients[slot].take().expect("checked above");
        if let Some(t) = client.meta_pub {
            self.delete_topic(t);
        }
        if let Some(t) = client.meta_sub {
            self.delete_topic(t);
        }
        self.update_meta_clients();
    }

    /// The connected clients, in slot order.
    pub fn clients(&self) -> impl Iterator<Item = ClientId> + '_ {
        self.clients
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, c)| c.is_some())
            .map(|(i, _)| ClientId(i))
    }

    /// A connected client's unique name (`<name>@<n>`).
    pub fn client_name(&self, id: ClientId) -> Option<&str> {
        self.remote(id.0)?;
        self.clients[id.0].as_ref().map(|c| c.name.as_str())
    }

    /// How often to call [`send_outgoing`](Self::send_outgoing) for this
    /// client, in milliseconds: the greatest common divisor of its
    /// subscription periods, at most 100 ms.
    ///
    /// ntcore runs a client's timer at its subscription period only, and
    /// not at all before it subscribes, so a client that only publishes
    /// gets its announcements and pings only when someone flushes. The
    /// 100 ms cap avoids that.
    pub fn tick_period_ms(&self, id: ClientId) -> u32 {
        match self.remote(id.0) {
            Some(_) => self.clients[id.0]
                .as_ref()
                .map_or(DEFAULT_PERIOD_MS, |c| c.period_ms.min(DEFAULT_PERIOD_MS)),
            None => DEFAULT_PERIOD_MS,
        }
    }

    /// Mark a client's socket as unable to take more data (`false`) or able
    /// again (`true`). While blocked, its queued messages stay queued.
    pub fn set_writable(&mut self, id: ClientId, writable: bool) {
        if let Some(r) = self.remote_mut(id.0) {
            r.wire.blocked = !writable;
        }
    }

    // ----- Incoming -----

    /// A client sent a text frame at `now_us`.
    pub fn handle_text(&mut self, id: ClientId, now_us: i64, text: &str) {
        let slot = id.0;
        let Some(r) = self.remote_mut(slot) else {
            return;
        };
        if r.closing {
            return;
        }
        r.last_received_ms = Some(to_ms(now_us));
        let decoded = decode_client_text(text);
        self.events
            .extend(decoded.warnings.into_iter().map(Event::Warning));
        // ntcore updates the client's meta topics once per batch of
        // messages; here a batch is one frame.
        let (mut pubs, mut subs) = (false, false);
        for msg in decoded.messages {
            match msg {
                ClientMessage::Publish {
                    name,
                    pubuid,
                    ty,
                    properties,
                } => {
                    self.client_publish(slot, pubuid, &name, ty, &properties);
                    pubs = true;
                }
                ClientMessage::Unpublish { pubuid } => {
                    self.client_unpublish(slot, pubuid);
                    pubs = true;
                }
                ClientMessage::SetProperties { name, update } => {
                    self.client_set_properties(slot, &name, &update);
                }
                ClientMessage::Subscribe {
                    subuid,
                    topics,
                    options,
                } => {
                    self.client_subscribe(slot, subuid, topics, options);
                    subs = true;
                }
                ClientMessage::Unsubscribe { subuid } => {
                    self.client_unsubscribe(slot, subuid);
                    subs = true;
                }
            }
        }
        if pubs {
            self.update_meta_client_pub(slot);
        }
        if subs {
            self.update_meta_client_sub(slot);
        }
    }

    /// A client sent a binary frame at `now_us`. RTT pings are answered
    /// right away, stamped `now_us`. A frame that doesn't decode closes the
    /// connection, as in ntcore.
    pub fn handle_binary(&mut self, id: ClientId, now_us: i64, data: &[u8]) {
        let slot = id.0;
        let Some(r) = self.remote_mut(slot) else {
            return;
        };
        if r.closing {
            return;
        }
        r.last_received_ms = Some(to_ms(now_us));
        for msg in BinaryReader::new(data) {
            let msg = match msg {
                Ok(m) => m,
                Err(e) => {
                    self.close(slot, format!("binary decode error: {e}"));
                    break;
                }
            };
            if msg.id == RTT_ID {
                let mut out = Vec::new();
                encode_binary(
                    &mut out,
                    &BinaryMessage {
                        id: RTT_ID,
                        timestamp_us: now_us,
                        value: msg.value,
                    },
                );
                let r = self.remote_mut(slot).expect("checked above");
                outgoing::Wire::send_binary(&mut r.wire, &out);
            } else {
                self.client_set_value(slot, msg.id, msg.timestamp_us, &msg.value);
            }
        }
    }

    /// A client's WebSocket pong arrived at `now_us`.
    pub fn handle_pong(&mut self, id: ClientId, now_us: i64) {
        if let Some(r) = self.remote_mut(id.0) {
            r.last_received_ms = Some(to_ms(now_us));
        }
    }

    // ----- Outgoing -----

    /// Run keepalive for one client and move its due messages into frames
    /// for [`poll_transmit`](Self::poll_transmit); `flush` sends everything
    /// queued, not only what is due.
    ///
    /// On NT 4.1 this sends a WebSocket ping every 200 ms, and closes the
    /// connection (a [`Transmit::Close`]) if nothing arrived for a second.
    pub fn send_outgoing(&mut self, id: ClientId, now_us: i64, flush: bool) {
        let Some(r) = self.remote_mut(id.0) else {
            return;
        };
        if r.closing {
            return;
        }
        let now_ms = to_ms(now_us);
        if r.version == ProtocolVersion::V4_1 && now_ms >= r.next_ping_ms {
            // No data for a while (we should at least get pongs).
            if let Some(sent) = r.ping_sent_ms
                && now_ms > r.last_received_ms.unwrap_or(sent) + u64::from(PING_TIMEOUT_MS)
            {
                self.close(id.0, "connection timed out".into());
                return;
            }
            r.wire.push(Transmit::Ping);
            r.next_ping_ms = now_ms + u64::from(PING_INTERVAL_MS);
            r.ping_sent_ms = Some(now_ms);
        }
        r.outgoing
            .send_outgoing(&mut r.wire, queue_ms(now_us), flush);
        r.wire.finish();
    }

    /// [`send_outgoing`](Self::send_outgoing) for every client.
    pub fn send_all_outgoing(&mut self, now_us: i64, flush: bool) {
        for slot in 1..self.clients.len() {
            self.send_outgoing(ClientId(slot), now_us, flush);
        }
    }

    /// The next frame to send to a client, oldest first.
    pub fn poll_transmit(&mut self, id: ClientId) -> Option<Transmit> {
        self.remote_mut(id.0)?.wire.out.pop_front()
    }

    /// Whether [`poll_transmit`](Self::poll_transmit) has a frame for this
    /// client, so a runtime can wake only the connections with work.
    ///
    /// ```
    /// use nt4_core::{ProtocolVersion, ServerCore};
    /// let mut server = ServerCore::new();
    /// let (id, _) = server.add_client("c", "10.0.0.2:1", false, ProtocolVersion::V4_1);
    /// assert!(!server.has_transmit(id));
    /// server.send_outgoing(id, 1_000, false); // the first keepalive ping
    /// assert!(server.has_transmit(id));
    /// ```
    pub fn has_transmit(&self, id: ClientId) -> bool {
        self.remote(id.0).is_some_and(|r| !r.wire.out.is_empty())
    }

    /// The next event, oldest first.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    fn close(&mut self, slot: usize, reason: String) {
        if let Some(r) = self.remote_mut(slot) {
            r.closing = true;
            r.wire.push(Transmit::Close { reason });
        }
    }

    // ----- Local API -----

    /// Publish a topic from the server itself. Unlike clients, the server
    /// may publish `$` topics.
    ///
    /// ```
    /// use nt4_core::{ServerCore, ServerEvent};
    /// use nt4_proto::{NtType, Properties};
    /// let mut server = ServerCore::new();
    /// let p = server.publish("/match/time", NtType::Double, Properties::new());
    /// assert!(matches!(server.poll_event(), Some(ServerEvent::Announced { publisher: Some(x), .. }) if x == p));
    /// ```
    pub fn publish(&mut self, name: &str, ty: NtType, properties: Properties) -> PubId {
        let pubuid = self.next_pubuid;
        self.next_pubuid += 1;
        self.client_publish(LOCAL, pubuid, name, ty, &properties);
        self.update_meta_client_pub(LOCAL);
        PubId(pubuid)
    }

    /// Stop publishing. The topic is deleted if nobody else publishes it and
    /// it isn't persistent or retained.
    pub fn unpublish(&mut self, id: PubId) {
        if self.client(LOCAL).publishers.contains_key(&id.0) {
            self.client_unpublish(LOCAL, id.0);
            self.update_meta_client_pub(LOCAL);
        }
    }

    /// Set a value from the server. `time_us` is when it was measured, or 0
    /// to set it as a default that any other value overrides.
    ///
    /// ```
    /// use nt4_core::{ServerCore, ServerError};
    /// use nt4_proto::{NtType, Properties, Value};
    /// let mut server = ServerCore::new();
    /// let p = server.publish("/x", NtType::Int, Properties::new());
    /// assert!(server.set_value(p, Value::Int(3), 10).is_ok());
    /// assert!(matches!(server.set_value(p, Value::Double(3.0), 10), Err(ServerError::TypeMismatch { .. })));
    /// ```
    pub fn set_value(&mut self, id: PubId, value: Value, time_us: i64) -> Result<(), ServerError> {
        let publisher = self
            .client(LOCAL)
            .publishers
            .get(&id.0)
            .ok_or(ServerError::UnknownPublisher(id.0))?;
        if value.type_id() != publisher.ty.type_id() {
            return Err(ServerError::TypeMismatch {
                expected: publisher.ty.clone(),
                got: value.type_id(),
            });
        }
        self.client_set_value(LOCAL, id.0, time_us, &value);
        Ok(())
    }

    /// Change a published topic's properties. A `null` value deletes that
    /// property.
    pub fn set_properties(&mut self, name: &str, update: Properties) {
        self.client_set_properties(LOCAL, name, &update);
    }

    /// Subscribe from the server. What arrives comes out of
    /// [`poll_event`](Self::poll_event); the server's own values are not
    /// echoed back.
    ///
    /// ```
    /// use nt4_core::{ProtocolVersion, ServerCore, ServerEvent};
    /// use nt4_proto::SubscriptionOptions;
    /// let mut server = ServerCore::new();
    /// server.subscribe(vec!["/vision/".into()], SubscriptionOptions { prefix: true, ..Default::default() });
    /// let (cam, _) = server.add_client("cam", "10.0.0.11:4000", false, ProtocolVersion::V4_1);
    /// server.handle_text(cam, 0, r#"[{"method":"publish","params":{"name":"/vision/tx","pubuid":1,"type":"double","properties":{}}}]"#);
    /// assert!(matches!(server.poll_event(), Some(ServerEvent::Announced { publisher: None, .. })));
    /// ```
    pub fn subscribe(&mut self, topics: Vec<String>, mut options: SubscriptionOptions) -> SubId {
        // ntcore's local API treats a period of 0 as the default.
        if options.periodic == Some(0.0) {
            options.periodic = None;
        }
        let subuid = self.next_subuid;
        self.next_subuid += 1;
        self.client_subscribe(LOCAL, subuid, topics, options);
        self.update_meta_client_sub(LOCAL);
        SubId(subuid)
    }

    /// End a subscription.
    pub fn unsubscribe(&mut self, id: SubId) {
        if self.client(LOCAL).subscribers.contains_key(&id.0) {
            self.client_unsubscribe(LOCAL, id.0);
            self.update_meta_client_sub(LOCAL);
        }
    }

    // ----- Persistent values -----

    /// Whether a persistent topic's value or properties changed since the
    /// last call; the flag clears. ntcore checks it once a second and saves
    /// [`dump_persistent`](Self::dump_persistent) when it is set.
    ///
    /// ```
    /// use nt4_core::ServerCore;
    /// use nt4_proto::{NtType, Properties, Value};
    /// let mut server = ServerCore::new();
    /// let mut props = Properties::new();
    /// props.insert("persistent".into(), true.into());
    /// let p = server.publish("/gain", NtType::Double, props);
    /// assert!(!server.persistent_changed());
    /// server.set_value(p, Value::Double(0.5), 1).unwrap();
    /// assert!(server.persistent_changed());
    /// assert!(!server.persistent_changed());
    /// ```
    pub fn persistent_changed(&mut self) -> bool {
        std::mem::take(&mut self.persistent_changed)
    }

    /// Every persistent topic that has a value, as the JSON file ntcore
    /// saves (`networktables.json`), byte for byte.
    ///
    /// ```
    /// use nt4_core::ServerCore;
    /// use nt4_proto::{NtType, Properties, Value};
    /// let mut server = ServerCore::new();
    /// let mut props = Properties::new();
    /// props.insert("persistent".into(), true.into());
    /// let p = server.publish("/gain", NtType::Double, props);
    /// server.set_value(p, Value::Double(0.5), 1).unwrap();
    /// assert_eq!(
    ///     server.dump_persistent(),
    ///     "[\n  {\n    \"name\": \"\\/gain\",\n    \"type\": \"double\",\n    \"value\": 0.5,\n    \"properties\": {\"persistent\": true}\n  }\n]\n"
    /// );
    /// ```
    pub fn dump_persistent(&self) -> String {
        let mut out = String::from("[\n");
        let mut first = true;
        for t in self.topics.iter().flatten() {
            let (true, Some((_, value))) = (t.persistent, &t.last_value) else {
                continue;
            };
            if !first {
                out.push_str(",\n");
            }
            first = false;
            out.push_str("  {\n    \"name\": ");
            persistent::push_string(&mut out, &t.name);
            out.push_str(",\n    \"type\": ");
            persistent::push_string(&mut out, &t.ty.to_string());
            out.push_str(",\n    \"value\": ");
            persistent::push_value(&mut out, value);
            out.push_str(",\n    \"properties\": ");
            persistent::push_json(&mut out, &Json::Object(t.properties.clone()), 0);
            out.push_str("\n  }");
        }
        out.push_str("\n]\n");
        out
    }

    /// Load a persistent file (as [`dump_persistent`](Self::dump_persistent)
    /// writes it): each entry becomes a topic with its value, stamped
    /// `now_us`. Returns one line per problem, `"<index>: <error>"` in
    /// ntcore's words; entries with a problem are skipped. Loading doesn't
    /// count as a change for [`persistent_changed`](Self::persistent_changed).
    ///
    /// ```
    /// use nt4_core::ServerCore;
    /// let mut server = ServerCore::new();
    /// let errors = server.load_persistent(
    ///     r#"[{"name": "/a", "type": "int", "value": 3, "properties": {"persistent": true}},
    ///         {"name": "/b", "type": "int", "value": 4, "properties": {}}]"#,
    ///     1,
    /// );
    /// assert_eq!(errors, ["1: no persistent property"]);
    /// assert!(server.dump_persistent().contains(r#""\/a""#));
    /// ```
    pub fn load_persistent(&mut self, json: &str, now_us: i64) -> Vec<String> {
        if json.is_empty() {
            return Vec::new();
        }
        let items = match serde_json::from_str::<Json>(json) {
            Err(e) => return vec![format!("could not decode JSON: {e}")],
            Ok(Json::Array(items)) => items,
            Ok(_) => return vec!["expected JSON array at top level".into()],
        };
        let changed = self.persistent_changed;
        let mut errors = Vec::new();
        for (i, item) in items.iter().enumerate() {
            match persistent::parse_entry(item) {
                Err(e) => errors.push(format!("{i}: {e}")),
                Ok(entry) => {
                    if let Some(e) = entry.skipped {
                        errors.push(format!("{i}: {e}"));
                    }
                    let topic =
                        self.create_topic(None, &entry.name, entry.ty, &entry.properties, false);
                    self.set_value_from(None, topic, now_us, &entry.value);
                }
            }
        }
        self.persistent_changed = changed;
        errors
    }

    // ----- Client requests (ServerClient4Base) -----

    fn client_publish(
        &mut self,
        slot: usize,
        pubuid: i32,
        name: &str,
        ty: NtType,
        properties: &Properties,
    ) {
        if slot != LOCAL && name.starts_with('$') {
            self.warn(format!(
                "client {slot} publish of reserved topic '{name}' ignored"
            ));
            return;
        }
        let topic = self.create_topic(Some(slot), name, ty.clone(), properties, false);
        let client = self.client_mut(slot);
        if let std::collections::btree_map::Entry::Vacant(e) = client.publishers.entry(pubuid) {
            e.insert(Publisher { topic, ty });
            let t = self.topic_mut(topic);
            if t.clients.entry(slot).or_default().publishers.insert(pubuid) {
                t.publisher_count += 1;
            }
            self.update_meta_topic_pub(topic);
        } else {
            self.warn(format!(
                "client {slot} duplicate publish of pubuid {pubuid}"
            ));
        }
        // Always answer a publish with an announce carrying the pubuid, even
        // if a subscription already announced the topic.
        self.send_announce(slot, topic, Some(pubuid));
    }

    fn client_unpublish(&mut self, slot: usize, pubuid: i32) {
        let Some(publisher) = self.client_mut(slot).publishers.remove(&pubuid) else {
            return;
        };
        let topic = publisher.topic;
        let t = self.topic_mut(topic);
        if t.clients
            .entry(slot)
            .or_default()
            .publishers
            .remove(&pubuid)
        {
            t.publisher_count -= 1;
        }
        self.update_meta_topic_pub(topic);
        if !self.topic(topic).is_published() {
            self.delete_topic(topic);
        }
    }

    fn client_set_properties(&mut self, slot: usize, name: &str, update: &Properties) {
        let topic = self
            .topic_names
            .get(name)
            .copied()
            .filter(|&t| self.topic(t).is_published());
        let update_json = || serde_json::to_string(update).unwrap_or_default();
        let Some(topic) = topic else {
            self.warn(format!(
                "server ignoring SetProperties({}) from client {slot} on unpublished topic '{name}'; publish or set a value first",
                update_json()
            ));
            return;
        };
        if self.topic(topic).special {
            self.warn(format!(
                "server ignoring SetProperties({}) from client {slot} on meta topic '{name}'",
                update_json()
            ));
            return;
        }
        // ntcore passes no client here, so the sender gets `ack: false`;
        // the spec says the sender's copy has `ack: true`. ntcore clients
        // ignore acks (they already applied the change), so this is safe.
        self.set_properties_from(Some(slot), topic, update);
    }

    fn client_subscribe(
        &mut self,
        slot: usize,
        subuid: i32,
        topics: Vec<String>,
        options: SubscriptionOptions,
    ) {
        let sub = Subscriber::new(topics, options);
        let sub_period = sub.period_ms;
        let topics_only = sub.options.topics_only;
        let client = self.client_mut(slot);
        let replace = client.subscribers.insert(subuid, sub).is_some();
        if client.remote.is_some() {
            client.period_ms = outgoing::update_period_calc(client.period_ms, sub_period);
        }

        // Announce in this loop and send last values after it, so the
        // announcements and the values each go out together.
        let mut data_to_send = Vec::new();
        for tid in 0..self.topics.len() {
            let Some(t) = self.topics[tid].as_mut() else {
                continue;
            };
            let subs = &self.clients[slot]
                .as_ref()
                .expect("client exists")
                .subscribers;
            let (removed, was_subscribed, was_subscribed_value) = match t.clients.get_mut(&slot) {
                Some(tcd) => {
                    let removed = replace && tcd.subscribers.remove(&subuid);
                    if removed {
                        tcd.recalculate_send_mode(subs);
                    }
                    let was = !tcd.subscribers.is_empty();
                    (removed, was, was && tcd.send_mode != SendMode::Disabled)
                }
                None => (false, false, false),
            };
            let added = subs[&subuid].matches(&t.name, t.special);
            if added {
                let tcd = t.clients.entry(slot).or_default();
                tcd.subscribers.insert(subuid);
                tcd.recalculate_send_mode(subs);
            }
            let has_value = t.last_value.is_some();

            // ntcore had `added ^ removed` here, which skipped re-subscribes
            // with a new period; upstream fixed it to `||`.
            if added || removed {
                self.update_period(slot, tid);
                self.update_meta_topic_sub(tid);
            }
            if added && !removed && !was_subscribed {
                self.send_announce(slot, tid, None);
            }
            if added && !topics_only && !was_subscribed_value && has_value {
                data_to_send.push(tid);
            }
        }
        for tid in data_to_send {
            if let Some((time_us, value)) = self.topic(tid).last_value.clone() {
                self.send_value(slot, tid, time_us, &value, SendMode::All);
            }
        }
    }

    fn client_unsubscribe(&mut self, slot: usize, subuid: i32) {
        if !self.client(slot).subscribers.contains_key(&subuid) {
            return;
        }
        for tid in 0..self.topics.len() {
            let Some(t) = self.topics[tid].as_mut() else {
                continue;
            };
            let subs = &self.clients[slot]
                .as_ref()
                .expect("client exists")
                .subscribers;
            let removed = match t.clients.get_mut(&slot) {
                Some(tcd) => {
                    let removed = tcd.subscribers.remove(&subuid);
                    if removed {
                        tcd.recalculate_send_mode(subs);
                    }
                    removed
                }
                None => false,
            };
            if removed {
                self.update_period(slot, tid);
                self.update_meta_topic_sub(tid);
            }
        }
        // Later values for it are no longer sent.
        let client = self.client_mut(slot);
        client.subscribers.remove(&subuid);
        if client.remote.is_some() {
            client.period_ms =
                outgoing::calculate_period(client.subscribers.values().map(|s| s.period_ms));
        }
    }

    fn client_set_value(&mut self, slot: usize, pubuid: i32, time_us: i64, value: &Value) {
        let Some(topic) = self.client(slot).publishers.get(&pubuid).map(|p| p.topic) else {
            self.warn(format!(
                "unrecognized client {slot} pubuid {pubuid}, ignoring set"
            ));
            return;
        };
        self.set_value_from(Some(slot), topic, time_us, value);
    }

    // ----- Sending to one client (ServerClient4, ServerClientLocal) -----

    fn send_announce(&mut self, slot: usize, topic: usize, pubuid: Option<i32>) {
        let client = self.clients[slot].as_mut().expect("client exists");
        let sent = client.announce_sent.entry(topic).or_insert(false);
        // A publish always gets its announce; a subscription only once.
        if *sent && pubuid.is_none() {
            return;
        }
        *sent = true;
        let t = self.topics[topic].as_ref().expect("topic exists");
        match &mut client.remote {
            None => self.events.push_back(Event::Announced {
                name: t.name.clone(),
                id: topic as i32,
                ty: t.ty.clone(),
                properties: t.properties.clone(),
                publisher: pubuid.map(PubId),
            }),
            Some(r) => r.outgoing.send_message(
                &mut r.wire,
                topic as i32,
                ServerMessage::Announce {
                    name: t.name.to_string(),
                    id: topic as i32,
                    ty: t.ty.clone(),
                    pubuid,
                    properties: t.properties.clone(),
                },
            ),
        }
    }

    fn send_unannounce(&mut self, slot: usize, topic: usize) {
        let client = self.clients[slot].as_mut().expect("client exists");
        let sent = client.announce_sent.entry(topic).or_insert(false);
        if !*sent {
            return;
        }
        *sent = false;
        let name = self.topics[topic]
            .as_ref()
            .expect("topic exists")
            .name
            .clone();
        match &mut client.remote {
            None => self.events.push_back(Event::Unannounced {
                name,
                id: topic as i32,
            }),
            Some(r) => {
                r.outgoing.send_message(
                    &mut r.wire,
                    topic as i32,
                    ServerMessage::Unannounce {
                        name: name.to_string(),
                        id: topic as i32,
                    },
                );
                r.outgoing.erase_id(topic as i32);
            }
        }
    }

    fn send_properties_update(
        &mut self,
        slot: usize,
        topic: usize,
        update: &Properties,
        ack: bool,
    ) {
        let client = self.clients[slot].as_mut().expect("client exists");
        if client.announce_sent.get(&topic) != Some(&true) {
            return;
        }
        let name = self.topics[topic]
            .as_ref()
            .expect("topic exists")
            .name
            .to_string();
        match &mut client.remote {
            None => self.events.push_back(Event::Properties {
                name,
                update: update.clone(),
                ack,
            }),
            Some(r) => r.outgoing.send_message(
                &mut r.wire,
                topic as i32,
                ServerMessage::Properties {
                    name,
                    update: update.clone(),
                    ack,
                },
            ),
        }
    }

    fn send_value(
        &mut self,
        slot: usize,
        topic: usize,
        time_us: i64,
        value: &Value,
        mode: SendMode,
    ) {
        let client = self.clients[slot].as_mut().expect("client exists");
        match &mut client.remote {
            None => {
                let name = self.topics[topic]
                    .as_ref()
                    .expect("topic exists")
                    .name
                    .clone();
                self.events.push_back(Event::Value {
                    name,
                    id: topic as i32,
                    value: TopicValue {
                        value: value.clone(),
                        server_time_us: time_us,
                        local_time_us: time_us,
                    },
                });
            }
            Some(r) => r
                .outgoing
                .send_value(&mut r.wire, topic as i32, time_us, value, mode),
        }
    }

    /// Set the send period of a topic for a client to that of its
    /// subscriptions.
    fn update_period(&mut self, slot: usize, topic: usize) {
        let t = self.topics[topic].as_ref().expect("topic exists");
        let Some(tcd) = t.clients.get(&slot) else {
            return;
        };
        let client = self.clients[slot].as_mut().expect("client exists");
        let Some(r) = &mut client.remote else {
            return;
        };
        let period = outgoing::calculate_period(
            tcd.subscribers
                .iter()
                .filter_map(|uid| client.subscribers.get(uid))
                .map(|s| s.period_ms),
        );
        r.outgoing.set_period(topic as i32, period);
    }

    // ----- Storage (ServerStorage) -----

    fn create_topic(
        &mut self,
        client: Option<usize>,
        name: &str,
        ty: NtType,
        properties: &Properties,
        special: bool,
    ) -> usize {
        if let Some(&id) = self.topic_names.get(name) {
            let current = &self.topic(id).ty;
            if let Some(slot) = client
                && ty != *current
            {
                let msg = format!(
                    "client {} publish '{name}' conflicting type '{ty}' (currently '{current}')",
                    self.client(slot).name
                );
                self.warn(msg);
            }
            return id;
        }

        let id = if self.free_topic_ids.len() < TOPIC_ID_REUSE_THRESHOLD {
            self.topics.push(None);
            self.topics.len() - 1
        } else {
            self.free_topic_ids.pop_front().expect("not empty")
        };
        let name: Arc<str> = name.into();
        let mut topic = Topic {
            name: name.clone(),
            ty,
            properties: properties.clone(),
            last_value: None,
            last_value_client: None,
            publisher_count: 0,
            persistent: false,
            retained: false,
            cached: true,
            special,
            clients: BTreeMap::new(),
            meta_pub: None,
            meta_sub: None,
        };
        if let Some(w) = topic.refresh_properties() {
            self.warn(w);
        }
        self.topics[id] = Some(topic);
        self.topic_names.insert(name.clone(), id);

        self.announce_new_topic(id, client);

        // Meta topics don't get meta topics.
        if !special {
            let meta_pub = self.create_meta_topic(&format!("$pub${name}"));
            let meta_sub = self.create_meta_topic(&format!("$sub${name}"));
            let t = self.topic_mut(id);
            t.meta_pub = Some(meta_pub);
            t.meta_sub = Some(meta_sub);
            self.update_meta_topic_pub(id);
            self.update_meta_topic_sub(id);
        }
        id
    }

    fn create_meta_topic(&mut self, name: &str) -> usize {
        self.create_topic(None, name, NtType::MsgPack, &meta_properties(), true)
    }

    /// Add a new topic to every matching subscription and announce it,
    /// except to `requester`, which gets the announce for its publish.
    fn announce_new_topic(&mut self, topic: usize, requester: Option<usize>) {
        for slot in 0..self.clients.len() {
            let Some(client) = self.clients[slot].as_ref() else {
                continue;
            };
            let t = self.topics[topic].as_mut().expect("topic exists");
            let matching: Vec<i32> = client
                .subscribers
                .iter()
                .filter(|(_, s)| s.matches(&t.name, t.special))
                .map(|(&uid, _)| uid)
                .collect();
            if matching.is_empty() {
                continue;
            }
            let tcd = t.clients.entry(slot).or_default();
            let mut added = false;
            for uid in matching {
                added |= tcd.subscribers.insert(uid);
            }
            tcd.recalculate_send_mode(&client.subscribers);
            if added {
                self.update_period(slot, topic);
            }
            if requester != Some(slot) {
                self.send_announce(slot, topic, None);
            }
        }
    }

    fn delete_topic(&mut self, topic: usize) {
        let Some(t) = self.topics.get(topic).and_then(Option::as_ref) else {
            return;
        };
        let (meta_pub, meta_sub) = (t.meta_pub, t.meta_sub);
        if let Some(m) = meta_pub {
            self.delete_topic(m);
        }
        if let Some(m) = meta_sub {
            self.delete_topic(m);
        }

        let subscribed: Vec<usize> = self
            .topic(topic)
            .clients
            .iter()
            .filter(|(_, tcd)| !tcd.subscribers.is_empty())
            .map(|(&slot, _)| slot)
            .collect();
        for slot in subscribed {
            self.update_period(slot, topic);
            self.send_unannounce(slot, topic);
        }
        // ntcore keys "announced" by topic pointer and leaves it set for
        // clients that only published; with ids reused, a stale flag would
        // hide the next topic with this id. Forget it everywhere.
        for client in self.clients.iter_mut().flatten() {
            if client.announce_sent.remove(&topic) == Some(true)
                && let Some(r) = &mut client.remote
            {
                r.outgoing.erase_id(topic as i32);
            }
        }

        let t = self.topics[topic].take().expect("checked above");
        self.topic_names.remove(&t.name);
        self.free_topic_ids.push_back(topic);
    }

    fn set_properties_from(&mut self, client: Option<usize>, topic: usize, update: &Properties) {
        if update.is_empty() {
            return;
        }
        let t = self.topic_mut(topic);
        let was_persistent = t.persistent;
        for (k, v) in update {
            if v.is_null() {
                t.properties.remove(k);
            } else {
                t.properties.insert(k.clone(), v.clone());
            }
        }
        let warning = t.refresh_properties();
        if t.persistent || was_persistent {
            self.persistent_changed = true;
        }
        if let Some(w) = warning {
            self.warn(w);
        }
        // Removing `persistent` or `retained` can leave it unpublished.
        if !self.topic(topic).is_published() {
            self.delete_topic(topic);
            return;
        }
        let slots: Vec<usize> = self.topic(topic).clients.keys().copied().collect();
        for slot in slots {
            self.send_properties_update(slot, topic, update, Some(slot) == client);
        }
    }

    /// Store a value (if newer) and send it to every subscribed client but
    /// the sender. `sender` is `None` for the server's meta topics.
    fn set_value_from(&mut self, sender: Option<usize>, topic: usize, time_us: i64, value: &Value) {
        let t = self.topic_mut(topic);
        // Keep the value if it comes from the same client, or is not older.
        let newer = match &t.last_value {
            None => true,
            Some((last_time, _)) => {
                t.last_value_client == sender || *last_time == 0 || time_us >= *last_time
            }
        };
        if t.cached && newer {
            t.last_value = Some((time_us, value.clone()));
            t.last_value_client = sender;
            if t.persistent {
                self.persistent_changed = true;
            }
        }
        let t = self.topic(topic);
        let targets: Vec<(usize, SendMode)> = t
            .clients
            .iter()
            .filter(|(slot, tcd)| Some(**slot) != sender && tcd.send_mode != SendMode::Disabled)
            .map(|(&slot, tcd)| (slot, tcd.send_mode))
            .collect();
        for (slot, mode) in targets {
            self.send_value(slot, topic, time_us, value, mode);
        }
    }

    // ----- Meta topics -----

    fn set_meta(&mut self, topic: Option<usize>, w: Writer) {
        if let Some(t) = topic {
            self.set_value_from(None, t, 0, &Value::Raw(w.0));
        }
    }

    fn update_meta_clients(&mut self) {
        let mut w = Writer::default();
        let remotes: Vec<(&str, &Remote)> = self
            .clients
            .iter()
            .flatten()
            .filter_map(|c| c.remote.as_ref().map(|r| (c.name.as_str(), r)))
            .collect();
        w.array(remotes.len());
        for (name, r) in remotes {
            w.map(3);
            w.str("id");
            w.str(name);
            w.str("conn");
            w.str(&r.conn_info);
            w.str("ver");
            w.uint(wire_version(r.version));
        }
        self.set_meta(Some(self.meta_clients), w);
    }

    fn update_meta_client_pub(&mut self, slot: usize) {
        let client = self.client(slot);
        let mut w = Writer::default();
        w.array(client.publishers.len());
        for (&uid, p) in &client.publishers {
            w.map(2);
            w.str("uid");
            w.int(uid.into());
            w.str("topic");
            w.str(&self.topic(p.topic).name);
        }
        self.set_meta(client.meta_pub, w);
    }

    fn update_meta_client_sub(&mut self, slot: usize) {
        let client = self.client(slot);
        let mut w = Writer::default();
        w.array(client.subscribers.len());
        for (&uid, s) in &client.subscribers {
            w.map(3);
            w.str("uid");
            w.int(uid.into());
            w.str("topics");
            w.array(s.topics.len());
            for t in &s.topics {
                w.str(t);
            }
            w.str("options");
            w.options(&s.options, s.periodic_ms);
        }
        self.set_meta(client.meta_sub, w);
    }

    fn update_meta_topic_pub(&mut self, topic: usize) {
        let t = self.topic(topic);
        let Some(meta) = t.meta_pub else {
            return;
        };
        let mut w = Writer::default();
        w.array(t.clients.values().map(|tcd| tcd.publishers.len()).sum());
        for (&slot, tcd) in &t.clients {
            for &uid in &tcd.publishers {
                w.map(2);
                w.str("client");
                w.str(&self.client(slot).name);
                w.str("pubuid");
                w.int(uid.into());
            }
        }
        self.set_meta(Some(meta), w);
    }

    fn update_meta_topic_sub(&mut self, topic: usize) {
        let t = self.topic(topic);
        let Some(meta) = t.meta_sub else {
            return;
        };
        let mut w = Writer::default();
        w.array(t.clients.values().map(|tcd| tcd.subscribers.len()).sum());
        for (&slot, tcd) in &t.clients {
            let client = self.client(slot);
            for uid in &tcd.subscribers {
                let s = &client.subscribers[uid];
                w.map(3);
                w.str("client");
                w.str(&client.name);
                w.str("subuid");
                w.int((*uid).into());
                w.str("options");
                w.options(&s.options, s.periodic_ms);
            }
        }
        self.set_meta(Some(meta), w);
    }

    // ----- Helpers -----

    fn warn(&mut self, msg: String) {
        self.events.push_back(Event::Warning(msg));
    }

    fn client(&self, slot: usize) -> &Client {
        self.clients[slot].as_ref().expect("client exists")
    }

    fn client_mut(&mut self, slot: usize) -> &mut Client {
        self.clients[slot].as_mut().expect("client exists")
    }

    fn remote(&self, slot: usize) -> Option<&Remote> {
        self.clients.get(slot)?.as_ref()?.remote.as_ref()
    }

    fn remote_mut(&mut self, slot: usize) -> Option<&mut Remote> {
        self.clients.get_mut(slot)?.as_mut()?.remote.as_mut()
    }

    fn topic(&self, id: usize) -> &Topic {
        self.topics[id].as_ref().expect("topic exists")
    }

    fn topic_mut(&mut self, id: usize) -> &mut Topic {
        self.topics[id].as_mut().expect("topic exists")
    }
}
