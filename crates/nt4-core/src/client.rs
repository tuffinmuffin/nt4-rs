//! The NT4 client state machine, with no sockets.
//!
//! [`ClientCore`] holds what a client publishes and subscribes to, and the
//! state of the current connection: announced topics, time sync, and the
//! outgoing queue. The caller owns the socket and the clock, and drives it:
//!
//! 1. When the WebSocket opens, call [`connect`](ClientCore::connect).
//! 2. Pass every received frame to [`handle_text`](ClientCore::handle_text)
//!    or [`handle_binary`](ClientCore::handle_binary), and WebSocket pongs to
//!    [`handle_pong`](ClientCore::handle_pong).
//! 3. Every [`tick_period_ms`](ClientCore::tick_period_ms), call
//!    [`send_outgoing`](ClientCore::send_outgoing), then send everything
//!    [`poll_transmit`](ClientCore::poll_transmit) returns.
//! 4. Read what happened with [`poll_event`](ClientCore::poll_event).
//! 5. When the socket closes, call [`disconnect`](ClientCore::disconnect).
//!    Publishers and subscriptions stay, and the next `connect` sends them
//!    again along with the last value of each published topic.
//!
//! All times are the caller's monotonic clock in integer microseconds.
//!
//! ```
//! use nt4_core::{ClientCore, Event, ProtocolVersion, PublishOptions, Transmit};
//! use nt4_proto::{NtType, Properties, SubscriptionOptions, Value};
//!
//! let mut client = ClientCore::new();
//! let pose = client.publish("/robot/x", NtType::Double, Properties::new(), PublishOptions::default());
//! client.set_value(pose, Value::Double(1.5), 0).unwrap();
//! client.subscribe(vec!["/robot/".into()], SubscriptionOptions { prefix: true, ..Default::default() });
//!
//! client.connect(1_000, ProtocolVersion::V4_1);
//! // The first frame is the RTT ping that starts time sync.
//! assert!(matches!(client.poll_transmit(), Some(Transmit::Binary(_))));
//! assert_eq!(client.poll_event(), Some(Event::Connected { version: ProtocolVersion::V4_1 }));
//! ```

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;

use nt4_proto::{
    BinaryMessage, BinaryReader, ClientMessage, NtType, Properties, RTT_ID, SUBPROTOCOL_V4_0,
    SUBPROTOCOL_V4_1, ServerMessage, SubscriptionOptions, Value, decode_server_text, encode_binary,
};

use crate::frames::{FrameBuffer, queue_ms, to_ms};
use crate::outgoing::{self, OutgoingQueue, SendMode, Wire};
use crate::time_sync::{TimeSync, TimeSyncUpdate};

/// Interval between WebSocket pings on an NT 4.1 connection, in milliseconds.
pub const PING_INTERVAL_MS: u32 = 200;
/// An NT 4.1 connection with no data for this long is closed, in milliseconds.
pub const PING_TIMEOUT_MS: u32 = 1000;
/// Interval between RTT pings on an NT 4.0 connection, in milliseconds.
pub const RTT_INTERVAL_MS: u32 = 3000;
/// Longest [`ClientCore::tick_period_ms`], in milliseconds.
pub const MAX_PERIOD_MS: u32 = PING_INTERVAL_MS;

/// NT protocol version, chosen by the WebSocket subprotocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProtocolVersion {
    /// NT 4.0 (`networktables.first.wpi.edu`). Keepalive uses RTT pings,
    /// since WebSocket pings are unsafe with some 4.0 servers.
    V4_0,
    /// NT 4.1 (`v4.1.networktables.first.wpi.edu`). Keepalive uses WebSocket
    /// pings.
    V4_1,
}

impl ProtocolVersion {
    /// The version for a negotiated subprotocol, or `None` if it isn't NT4.
    ///
    /// ```
    /// use nt4_core::ProtocolVersion;
    /// assert_eq!(
    ///     ProtocolVersion::from_subprotocol("v4.1.networktables.first.wpi.edu"),
    ///     Some(ProtocolVersion::V4_1)
    /// );
    /// ```
    pub fn from_subprotocol(protocol: &str) -> Option<Self> {
        match protocol {
            SUBPROTOCOL_V4_1 => Some(Self::V4_1),
            SUBPROTOCOL_V4_0 => Some(Self::V4_0),
            _ => None,
        }
    }

    /// The subprotocol string for this version.
    pub fn subprotocol(self) -> &'static str {
        match self {
            Self::V4_0 => SUBPROTOCOL_V4_0,
            Self::V4_1 => SUBPROTOCOL_V4_1,
        }
    }
}

/// A publisher, returned by [`ClientCore::publish`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PubId(pub(crate) i32);

impl PubId {
    /// The `pubuid` sent to the server.
    pub fn uid(self) -> i32 {
        self.0
    }
}

/// A subscription, returned by [`ClientCore::subscribe`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubId(pub(crate) i32);

impl SubId {
    /// The `subuid` sent to the server.
    pub fn uid(self) -> i32 {
        self.0
    }
}

/// Options for [`ClientCore::publish`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PublishOptions {
    /// How often to send changes, in seconds. `None` means 0.1 s. Rounded
    /// to 10 ms, and at least 5 ms.
    pub periodic: Option<f64>,
    /// Send every value set, not only the newest one each period.
    pub send_all: bool,
}

impl PublishOptions {
    /// The send period in milliseconds, as ntcore rounds it.
    ///
    /// ```
    /// use nt4_core::PublishOptions;
    /// let o = PublishOptions { periodic: Some(0.025), ..Default::default() };
    /// assert_eq!(o.period_ms(), 30);
    /// ```
    pub fn period_ms(&self) -> u32 {
        let periodic = match self.periodic {
            None | Some(0.0) => 0.1,
            Some(p) => p,
        };
        // `as` saturates, like ntcore's cast for any sane input.
        let ms = (periodic * 1000.0) as u32;
        outgoing::round_periodic_ms(ms).max(outgoing::MIN_PERIOD_MS)
    }
}

/// Why a [`ClientCore`] call was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ClientError {
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

/// A topic value with its timestamps.
#[derive(Debug, Clone, PartialEq)]
pub struct TopicValue {
    /// The value.
    pub value: Value,
    /// When it was set, in server time (microseconds).
    pub server_time_us: i64,
    /// The same time in the local clock (microseconds).
    pub local_time_us: i64,
}

/// A topic the server has announced on the current connection.
#[derive(Debug, Clone, PartialEq)]
pub struct Topic {
    /// Topic name.
    pub name: Arc<str>,
    /// Server-chosen id, valid until the topic is unannounced or the
    /// connection drops.
    pub id: i32,
    /// Topic type.
    pub ty: NtType,
    /// Current properties.
    pub properties: Properties,
    /// The newest value received, if any.
    pub value: Option<TopicValue>,
}

/// Something that happened, read with [`ClientCore::poll_event`].
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// [`ClientCore::connect`] was called.
    Connected {
        /// The protocol version in use.
        version: ProtocolVersion,
    },
    /// The connection ended, by [`ClientCore::disconnect`] or a timeout.
    Disconnected {
        /// Why.
        reason: String,
    },
    /// The server announced a topic.
    Announced {
        /// Topic name.
        name: Arc<str>,
        /// Server-chosen id.
        id: i32,
        /// Topic type.
        ty: NtType,
        /// Topic properties.
        properties: Properties,
        /// Set when this answers one of our `publish` calls.
        publisher: Option<PubId>,
    },
    /// A topic is gone: the server unannounced it, or the connection ended.
    Unannounced {
        /// Topic name.
        name: Arc<str>,
        /// The id it had.
        id: i32,
    },
    /// A topic's properties changed. `null` values mean deleted.
    Properties {
        /// Topic name.
        name: String,
        /// The changed properties.
        update: Properties,
        /// True when this answers our own `set_properties`.
        ack: bool,
    },
    /// A value arrived.
    Value {
        /// Topic name.
        name: Arc<str>,
        /// Topic id.
        id: i32,
        /// The value and its timestamps.
        value: TopicValue,
    },
    /// The server time offset changed.
    TimeSync(TimeSyncUpdate),
    /// The server sent something that couldn't be used. The text matches
    /// ntcore's log messages where ntcore has one.
    Warning(String),
}

/// A frame to send, read with [`ClientCore::poll_transmit`].
#[derive(Debug, Clone, PartialEq)]
pub enum Transmit {
    /// A WebSocket text frame.
    Text(String),
    /// A WebSocket binary frame.
    Binary(Vec<u8>),
    /// A WebSocket ping (NT 4.1 keepalive).
    Ping,
    /// Close the socket. The core has already disconnected.
    Close {
        /// Why, e.g. `"connection timed out"`.
        reason: String,
    },
}

#[derive(Debug, Clone)]
struct Publisher {
    name: String,
    ty: NtType,
    properties: Properties,
    options: PublishOptions,
    period_ms: u32,
}

#[derive(Debug, Clone)]
struct Subscription {
    topics: Vec<String>,
    options: SubscriptionOptions,
}

/// State that lasts for one connection.
#[derive(Debug)]
struct Connection {
    version: ProtocolVersion,
    topics: HashMap<i32, Topic>,
    topic_ids: HashMap<Arc<str>, i32>,
    outgoing: OutgoingQueue<ClientMessage>,
    time_sync: TimeSync,
    // NT 4.0 RTT keepalive.
    next_rtt_ping_ms: u64,
    rtt_answered: bool,
    // NT 4.1 WebSocket keepalive.
    next_ws_ping_ms: u64,
    ws_ping_sent_ms: Option<u64>,
    last_received_ms: Option<u64>,
}

/// The NT4 client state machine. See the [module docs](self) for how to
/// drive it.
#[derive(Debug)]
pub struct ClientCore {
    publishers: BTreeMap<i32, Publisher>,
    subscriptions: BTreeMap<i32, Subscription>,
    /// Newest value per published topic name, as (local time, value), for
    /// replay after a reconnect.
    last_values: HashMap<String, (i64, Value)>,
    next_pubuid: i32,
    next_subuid: i32,
    period_ms: u32,
    conn: Option<Connection>,
    events: VecDeque<Event>,
    wire: FrameBuffer,
}

impl Default for ClientCore {
    fn default() -> Self {
        Self::new()
    }
}

fn apply_update(props: &mut Properties, update: &Properties) {
    for (k, v) in update {
        if v.is_null() {
            props.remove(k);
        } else {
            props.insert(k.clone(), v.clone());
        }
    }
}

impl ClientCore {
    /// A client with nothing published or subscribed, not connected.
    pub fn new() -> Self {
        Self {
            publishers: BTreeMap::new(),
            subscriptions: BTreeMap::new(),
            last_values: HashMap::new(),
            next_pubuid: 1,
            next_subuid: 1,
            period_ms: MAX_PERIOD_MS,
            conn: None,
            events: VecDeque::new(),
            wire: FrameBuffer::default(),
        }
    }

    // ----- Local API -----

    /// Start publishing a topic. Works while disconnected; the publish is
    /// sent when connected, and again after every reconnect.
    pub fn publish(
        &mut self,
        name: impl Into<String>,
        ty: NtType,
        properties: Properties,
        options: PublishOptions,
    ) -> PubId {
        let pubuid = self.next_pubuid;
        self.next_pubuid += 1;
        let period_ms = options.period_ms();
        self.publishers.insert(
            pubuid,
            Publisher {
                name: name.into(),
                ty,
                properties,
                options,
                period_ms,
            },
        );
        self.period_ms = outgoing::update_period_calc(self.period_ms, period_ms);
        self.clamp_period();
        if self.conn.is_some() {
            self.send_publish(pubuid);
        }
        PubId(pubuid)
    }

    /// Stop publishing. The topic's last value is kept only while another
    /// publisher of the same name remains.
    pub fn unpublish(&mut self, id: PubId) {
        let Some(publisher) = self.publishers.remove(&id.0) else {
            return;
        };
        if !self.publishers.values().any(|p| p.name == publisher.name) {
            self.last_values.remove(&publisher.name);
        }
        self.period_ms = self
            .publishers
            .values()
            .fold(MAX_PERIOD_MS, |acc, p| outgoing::gcd(acc, p.period_ms));
        self.clamp_period();
        if let Some(conn) = &mut self.conn {
            conn.outgoing.send_message(
                &mut self.wire,
                id.0,
                ClientMessage::Unpublish { pubuid: id.0 },
            );
            conn.outgoing.erase_id(id.0);
        }
    }

    /// Set a published value. `time_us` is the local time it was measured,
    /// or 0 to set it as a default that any other value overrides.
    ///
    /// While disconnected the value is kept and sent after the next connect.
    pub fn set_value(&mut self, id: PubId, value: Value, time_us: i64) -> Result<(), ClientError> {
        let publisher = self
            .publishers
            .get(&id.0)
            .ok_or(ClientError::UnknownPublisher(id.0))?;
        if value.type_id() != publisher.ty.type_id() {
            return Err(ClientError::TypeMismatch {
                expected: publisher.ty.clone(),
                got: value.type_id(),
            });
        }
        let mode = if publisher.options.send_all {
            SendMode::All
        } else {
            SendMode::Normal
        };
        if let Some(conn) = &mut self.conn {
            conn.outgoing
                .send_value(&mut self.wire, id.0, time_us, &value, mode);
        }
        // Look up before inserting, so the name is only copied the first time.
        match self.last_values.get_mut(&publisher.name) {
            Some(last) => *last = (time_us, value),
            None => {
                self.last_values
                    .insert(publisher.name.clone(), (time_us, value));
            }
        }
        Ok(())
    }

    /// Change a topic's properties on the server. A `null` value deletes
    /// that property. Our own publishers of the topic keep the change for
    /// replay after a reconnect.
    pub fn set_properties(&mut self, name: &str, update: Properties) {
        let mut queue_id = 0;
        for (&pubuid, p) in self.publishers.iter_mut().filter(|(_, p)| p.name == name) {
            apply_update(&mut p.properties, &update);
            queue_id = pubuid;
        }
        if let Some(conn) = &mut self.conn {
            // ntcore queues this under id 0 (the default period's queue)
            // while the `publish` it follows waits in the publisher's own
            // queue. With a publisher period other than the default, both
            // queues come due in the same send and the `setproperties` can
            // go first, and a server then ignores it (the topic isn't
            // published yet). Queue it behind our own `publish` instead.
            conn.outgoing.send_message(
                &mut self.wire,
                queue_id,
                ClientMessage::SetProperties {
                    name: name.to_owned(),
                    update,
                },
            );
        }
    }

    /// Subscribe to topics (names, or prefixes with `options.prefix`). Works
    /// while disconnected, and is sent again after every reconnect.
    pub fn subscribe(&mut self, topics: Vec<String>, options: SubscriptionOptions) -> SubId {
        let subuid = self.next_subuid;
        self.next_subuid += 1;
        self.subscriptions
            .insert(subuid, Subscription { topics, options });
        if self.conn.is_some() {
            self.send_subscribe(subuid);
        }
        SubId(subuid)
    }

    /// End a subscription.
    pub fn unsubscribe(&mut self, id: SubId) {
        if self.subscriptions.remove(&id.0).is_none() {
            return;
        }
        if let Some(conn) = &mut self.conn {
            conn.outgoing.send_message(
                &mut self.wire,
                0,
                ClientMessage::Unsubscribe { subuid: id.0 },
            );
        }
    }

    /// How often to call [`send_outgoing`](Self::send_outgoing), in
    /// milliseconds: the greatest common divisor of the publish periods,
    /// between 5 and 100 ms.
    ///
    /// The cap is the 100 ms period of the queue that carries publish,
    /// subscribe and property messages; ntcore gets the same from a separate
    /// 100 ms timer.
    pub fn tick_period_ms(&self) -> u32 {
        self.period_ms.min(outgoing::DEFAULT_PERIOD_MS)
    }

    fn clamp_period(&mut self) {
        self.period_ms = self.period_ms.clamp(outgoing::MIN_PERIOD_MS, MAX_PERIOD_MS);
    }

    // ----- Connection -----

    /// The WebSocket opened at local time `now_us`. Sends the first RTT ping
    /// right away, then every publisher (with its topic's last value) and
    /// subscription. Those wait in the queue until the first RTT reply sets
    /// the time offset, so value timestamps are right.
    ///
    /// Calling this while connected disconnects first.
    pub fn connect(&mut self, now_us: i64, version: ProtocolVersion) {
        if self.conn.is_some() {
            self.disconnect("reconnecting");
        }
        let now_ms = to_ms(now_us);
        self.conn = Some(Connection {
            version,
            topics: HashMap::new(),
            topic_ids: HashMap::new(),
            outgoing: OutgoingQueue::new(false),
            time_sync: TimeSync::new(),
            next_rtt_ping_ms: now_ms
                + u64::from(match version {
                    ProtocolVersion::V4_1 => PING_INTERVAL_MS,
                    ProtocolVersion::V4_0 => RTT_INTERVAL_MS,
                }),
            rtt_answered: false,
            next_ws_ping_ms: 0,
            ws_ping_sent_ms: None,
            last_received_ms: None,
        });
        self.events.push_back(Event::Connected { version });
        self.send_rtt_ping(now_us);

        // Replay: each publisher, then one last value per topic (sent by the
        // topic's newest publisher), then each subscription.
        let mut names: Vec<&str> = Vec::new();
        for p in self.publishers.values() {
            if !names.contains(&p.name.as_str()) {
                names.push(&p.name);
            }
        }
        let names: Vec<String> = names.into_iter().map(str::to_owned).collect();
        for name in names {
            let uids: Vec<i32> = self
                .publishers
                .iter()
                .filter(|(_, p)| p.name == name)
                .map(|(&uid, _)| uid)
                .collect();
            for &uid in &uids {
                self.send_publish(uid);
            }
            if let (Some(&uid), Some((time_us, value))) = (uids.last(), self.last_values.get(&name))
            {
                let mode = if self.publishers[&uid].options.send_all {
                    SendMode::All
                } else {
                    SendMode::Normal
                };
                let conn = self.conn.as_mut().expect("connected above");
                conn.outgoing
                    .send_value(&mut self.wire, uid, *time_us, value, mode);
            }
        }
        let subs: Vec<i32> = self.subscriptions.keys().copied().collect();
        for uid in subs {
            self.send_subscribe(uid);
        }
    }

    /// The connection ended. Every announced topic gets an
    /// [`Event::Unannounced`], the time offset becomes unknown, and frames not
    /// yet taken from [`poll_transmit`](Self::poll_transmit) are dropped.
    pub fn disconnect(&mut self, reason: &str) {
        let Some(conn) = self.conn.take() else {
            return;
        };
        self.wire.clear();
        let mut topics: Vec<Topic> = conn.topics.into_values().collect();
        topics.sort_by_key(|t| t.id);
        for t in topics {
            self.events.push_back(Event::Unannounced {
                name: t.name,
                id: t.id,
            });
        }
        if conn.time_sync.offset().is_some() {
            self.events
                .push_back(Event::TimeSync(TimeSyncUpdate::INVALID));
        }
        self.events.push_back(Event::Disconnected {
            reason: reason.to_owned(),
        });
    }

    /// Whether [`connect`](Self::connect) was called and the connection
    /// hasn't ended.
    pub fn is_connected(&self) -> bool {
        self.conn.is_some()
    }

    /// The protocol version of the current connection.
    pub fn version(&self) -> Option<ProtocolVersion> {
        self.conn.as_ref().map(|c| c.version)
    }

    /// Server time minus local time, in microseconds, once the first RTT
    /// reply on this connection has arrived.
    pub fn server_time_offset(&self) -> Option<i64> {
        self.conn.as_ref().and_then(|c| c.time_sync.offset())
    }

    /// Convert a local time to server time, if the offset is known.
    pub fn server_time(&self, local_us: i64) -> Option<i64> {
        self.server_time_offset()
            .and_then(|o| local_us.checked_add(o))
    }

    /// Mark the socket as unable to take more data (`false`) or able again
    /// (`true`). While blocked, queued messages stay queued.
    pub fn set_writable(&mut self, writable: bool) {
        self.wire.blocked = !writable;
    }

    // ----- Incoming -----

    /// A text frame arrived at local time `now_us`.
    pub fn handle_text(&mut self, now_us: i64, text: &str) {
        let Some(conn) = &mut self.conn else {
            return;
        };
        conn.last_received_ms = Some(to_ms(now_us));
        let decoded = decode_server_text(text);
        for w in decoded.warnings {
            self.events.push_back(Event::Warning(w));
        }
        for msg in decoded.messages {
            self.handle_server_message(msg);
        }
    }

    fn handle_server_message(&mut self, msg: ServerMessage) {
        let conn = self.conn.as_mut().expect("checked by caller");
        match msg {
            ServerMessage::Announce {
                name,
                id,
                ty,
                pubuid,
                properties,
            } => {
                let name: Arc<str> = name.into();
                // A re-announce under a new id replaces the old one.
                let replaced = conn.topic_ids.insert(name.clone(), id);
                if let Some(old) = replaced.filter(|&old| old != id) {
                    conn.topics.remove(&old);
                }
                conn.topics.insert(
                    id,
                    Topic {
                        name: name.clone(),
                        id,
                        ty: ty.clone(),
                        properties: properties.clone(),
                        value: None,
                    },
                );
                let publisher = pubuid
                    .filter(|p| self.publishers.contains_key(p))
                    .map(PubId);
                self.events.push_back(Event::Announced {
                    name,
                    id,
                    ty,
                    properties,
                    publisher,
                });
            }
            // The id identifies the topic; the name is only for logging.
            ServerMessage::Unannounce { name: _, id } => {
                let Some(topic) = conn.topics.remove(&id) else {
                    self.events
                        .push_back(Event::Warning(format!("unannounce of unknown id {id}")));
                    return;
                };
                if conn.topic_ids.get(&topic.name) == Some(&id) {
                    conn.topic_ids.remove(&topic.name);
                }
                self.events.push_back(Event::Unannounced {
                    name: topic.name,
                    id,
                });
            }
            ServerMessage::Properties { name, update, ack } => {
                // Acks echo our own change, which is already applied.
                if !ack {
                    let topic = conn
                        .topic_ids
                        .get(name.as_str())
                        .and_then(|id| conn.topics.get_mut(id));
                    if let Some(topic) = topic {
                        apply_update(&mut topic.properties, &update);
                    }
                    for p in self.publishers.values_mut().filter(|p| p.name == name) {
                        apply_update(&mut p.properties, &update);
                    }
                }
                self.events
                    .push_back(Event::Properties { name, update, ack });
            }
        }
    }

    /// A binary frame arrived at local time `now_us`.
    pub fn handle_binary(&mut self, now_us: i64, data: &[u8]) {
        let Some(conn) = &mut self.conn else {
            return;
        };
        let now_ms = to_ms(now_us);
        conn.last_received_ms = Some(now_ms);
        for msg in BinaryReader::new(data) {
            let msg = match msg {
                Ok(m) => m,
                Err(e) => {
                    self.events
                        .push_back(Event::Warning(format!("binary decode error: {e}")));
                    break;
                }
            };
            if msg.id == RTT_ID {
                self.handle_rtt_reply(now_us, &msg);
            } else {
                self.handle_value(msg);
            }
        }
    }

    fn handle_rtt_reply(&mut self, now_us: i64, msg: &BinaryMessage) {
        let conn = self.conn.as_mut().expect("checked by caller");
        // ntcore records the pong only for the first reply, which would make
        // a 4.0 connection time out once the offset is known; record every one.
        conn.rtt_answered = true;
        match conn.time_sync.handle_reply(now_us, msg) {
            Ok(Some(update)) => {
                // Only the first reply gives an update. Everything queued since
                // connect (publishers, values, subscriptions) was waiting for
                // it; ntcore leaves that for its next timer (up to 100 ms),
                // we send it now.
                conn.outgoing.set_time_offset(update.server_time_offset_us);
                conn.outgoing
                    .send_outgoing(&mut self.wire, queue_ms(now_us), true);
                self.wire.finish();
                self.events.push_back(Event::TimeSync(update));
            }
            Ok(None) => {}
            Err(w) => self.events.push_back(Event::Warning(w)),
        }
    }

    fn handle_value(&mut self, msg: BinaryMessage) {
        let conn = self.conn.as_mut().expect("checked by caller");
        let Some(topic) = conn.topics.get_mut(&msg.id) else {
            self.events
                .push_back(Event::Warning(format!("received unknown id {}", msg.id)));
            return;
        };
        let offset = conn.time_sync.offset().unwrap_or(0);
        let value = TopicValue {
            local_time_us: msg.timestamp_us.saturating_sub(offset),
            server_time_us: msg.timestamp_us,
            value: msg.value,
        };
        topic.value = Some(value.clone());
        let name = topic.name.clone();
        // Like ntcore, the newest value of a topic we publish is replayed on
        // reconnect, even when it came from another client.
        if let Some(entry) = self.last_values.get_mut(&*name) {
            *entry = (value.local_time_us, value.value.clone());
        } else if self.publishers.values().any(|p| *p.name == *name) {
            self.last_values
                .insert(name.to_string(), (value.local_time_us, value.value.clone()));
        }
        self.events.push_back(Event::Value {
            name,
            id: msg.id,
            value,
        });
    }

    /// A WebSocket pong arrived at local time `now_us`.
    pub fn handle_pong(&mut self, now_us: i64) {
        if let Some(conn) = &mut self.conn {
            conn.last_received_ms = Some(to_ms(now_us));
        }
    }

    // ----- Outgoing -----

    /// Run keepalive and move due messages from the queue into frames for
    /// [`poll_transmit`](Self::poll_transmit). Call it every
    /// [`tick_period_ms`](Self::tick_period_ms); `flush` sends everything
    /// queued, not only what is due.
    ///
    /// Nothing but pings is sent until the first RTT reply sets the time
    /// offset.
    pub fn send_outgoing(&mut self, now_us: i64, flush: bool) {
        let Some(conn) = &mut self.conn else {
            return;
        };
        let now_ms = to_ms(now_us);
        match conn.version {
            ProtocolVersion::V4_1 => {
                if now_ms >= conn.next_ws_ping_ms {
                    // No data for a while (we should at least get pongs).
                    let silent = conn.ws_ping_sent_ms.is_some_and(|sent| {
                        now_ms > conn.last_received_ms.unwrap_or(sent) + u64::from(PING_TIMEOUT_MS)
                    });
                    if silent {
                        self.timed_out();
                        return;
                    }
                    self.wire.finish();
                    self.wire.out.push_back(Transmit::Ping);
                    conn.next_ws_ping_ms = now_ms + u64::from(PING_INTERVAL_MS);
                    conn.ws_ping_sent_ms = Some(now_ms);
                }
            }
            ProtocolVersion::V4_0 => {
                if now_ms >= conn.next_rtt_ping_ms {
                    // No reply to the last ping.
                    if !conn.rtt_answered {
                        self.timed_out();
                        return;
                    }
                    conn.next_rtt_ping_ms = now_ms + u64::from(RTT_INTERVAL_MS);
                    conn.rtt_answered = false;
                    self.send_rtt_ping(now_us);
                }
            }
        }
        let conn = self.conn.as_mut().expect("still connected");
        if conn.time_sync.offset().is_none() {
            return;
        }
        conn.outgoing
            .send_outgoing(&mut self.wire, queue_ms(now_us), flush);
        self.wire.finish();
    }

    fn timed_out(&mut self) {
        const REASON: &str = "connection timed out";
        self.disconnect(REASON);
        self.wire.out.push_back(Transmit::Close {
            reason: REASON.into(),
        });
    }

    /// The next frame to send, oldest first.
    pub fn poll_transmit(&mut self) -> Option<Transmit> {
        self.wire.out.pop_front()
    }

    /// The next event, oldest first.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    // ----- Topics -----

    /// A topic announced on the current connection, by id.
    pub fn topic(&self, id: i32) -> Option<&Topic> {
        self.conn.as_ref()?.topics.get(&id)
    }

    /// A topic announced on the current connection, by name.
    pub fn topic_by_name(&self, name: &str) -> Option<&Topic> {
        let conn = self.conn.as_ref()?;
        conn.topics.get(conn.topic_ids.get(name)?)
    }

    /// Every topic announced on the current connection, in no set order.
    pub fn topics(&self) -> impl Iterator<Item = &Topic> {
        self.conn.iter().flat_map(|c| c.topics.values())
    }

    // ----- Helpers -----

    fn send_rtt_ping(&mut self, now_us: i64) {
        let mut out = Vec::new();
        encode_binary(&mut out, &TimeSync::ping(now_us));
        self.wire.send_binary(&out);
    }

    fn send_publish(&mut self, pubuid: i32) {
        let Some(conn) = &mut self.conn else {
            return;
        };
        let p = &self.publishers[&pubuid];
        conn.outgoing.set_period(pubuid, p.period_ms);
        conn.outgoing.send_message(
            &mut self.wire,
            pubuid,
            ClientMessage::Publish {
                name: p.name.clone(),
                pubuid,
                ty: p.ty.clone(),
                properties: p.properties.clone(),
            },
        );
    }

    fn send_subscribe(&mut self, subuid: i32) {
        let Some(conn) = &mut self.conn else {
            return;
        };
        let s = &self.subscriptions[&subuid];
        conn.outgoing.send_message(
            &mut self.wire,
            0,
            ClientMessage::Subscribe {
                subuid,
                topics: s.topics.clone(),
                options: s.options.clone(),
            },
        );
    }
}
