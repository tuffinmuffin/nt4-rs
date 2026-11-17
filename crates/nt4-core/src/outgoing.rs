//! Outgoing message queue, ported from ntcore's `NetworkOutgoingQueue`.
//!
//! Every message a peer sends goes through an [`OutgoingQueue`]. Messages are
//! grouped into one queue per send period (100 ms by default). When a queue
//! comes due, its messages are written to a [`Wire`] in order. Value updates
//! sent with [`SendMode::Normal`] replace the queued value for the same id, so
//! a publisher that sets a value 1000 times between sends only sends the
//! newest one; [`SendMode::All`] keeps every value until about 1 MiB is queued,
//! then falls back to replacing.
//!
//! Times in this module are milliseconds for scheduling (`now_ms`) and integer
//! microseconds for value timestamps, as on the wire.
//!
//! ```
//! use nt4_core::outgoing::{OutgoingQueue, RecordingWire, SendMode};
//! use nt4_proto::{ClientMessage, Value};
//!
//! let mut wire = RecordingWire::default();
//! let mut queue: OutgoingQueue<ClientMessage> = OutgoingQueue::new(false);
//! queue.send_value(&mut wire, 1, 10, &Value::Double(1.0), SendMode::Normal);
//! queue.send_value(&mut wire, 1, 20, &Value::Double(2.0), SendMode::Normal);
//! queue.send_outgoing(&mut wire, 5, false);
//! // Only the newer value was sent.
//! assert_eq!(wire.binary_writes.len(), 1);
//! ```

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use nt4_proto::{TextMessage, Value, encode_binary_parts};

/// Shortest time between two sends, and the shortest period a queue can
/// have, in milliseconds.
pub const MIN_PERIOD_MS: u32 = 5;

/// Period of the default queue, in milliseconds.
pub const DEFAULT_PERIOD_MS: u32 = 100;

/// Rough queued size, in bytes, above which [`SendMode::All`] values start
/// replacing each other like [`SendMode::Normal`] ones.
pub const OUTGOING_LIMIT: usize = 1024 * 1024;

/// Bytes counted for each queued message on top of its value's size. ntcore
/// counts `sizeof(Message)`; the exact number only changes when the limit
/// kicks in.
const MESSAGE_OVERHEAD: usize = 64;

/// The connection the queue writes to.
///
/// "Write" calls buffer messages into frames and may report that some were
/// not accepted; "send" calls bypass the queue and go out right away. A
/// return value is the number of messages from the end of this batch that
/// were *not* sent (0 means all of them were), so they stay queued and are
/// tried again next time.
pub trait Wire {
    /// Whether the connection can take more data now. When false, nothing is
    /// written and everything stays queued.
    fn ready(&self) -> bool;
    /// Buffer one JSON message (an object, without the surrounding `[]`).
    fn write_text(&mut self, json: &str) -> usize;
    /// Buffer one MessagePack value message.
    fn write_binary(&mut self, data: &[u8]) -> usize;
    /// Send whatever is buffered.
    fn flush(&mut self) -> usize;
    /// Send one JSON message now, in a frame of its own.
    fn send_text(&mut self, json: &str);
    /// Send one MessagePack value message now, in a frame of its own.
    fn send_binary(&mut self, data: &[u8]);
}

/// A [`Wire`] that records every call, for tests and examples.
///
/// The `*_returns` lists are popped one per call to fake partial writes;
/// when empty, calls return 0 (everything sent).
#[derive(Debug, Clone)]
pub struct RecordingWire {
    /// Returned by [`Wire::ready`].
    pub ready: bool,
    /// Each [`Wire::write_text`] call.
    pub text_writes: Vec<String>,
    /// Each [`Wire::write_binary`] call.
    pub binary_writes: Vec<Vec<u8>>,
    /// Each [`Wire::send_text`] call.
    pub text_sends: Vec<String>,
    /// Each [`Wire::send_binary`] call.
    pub binary_sends: Vec<Vec<u8>>,
    /// Number of [`Wire::flush`] calls.
    pub flush_count: usize,
    /// Return values for the next [`Wire::write_text`] calls.
    pub text_write_returns: std::collections::VecDeque<usize>,
    /// Return values for the next [`Wire::write_binary`] calls.
    pub binary_write_returns: std::collections::VecDeque<usize>,
    /// Return values for the next [`Wire::flush`] calls.
    pub flush_returns: std::collections::VecDeque<usize>,
}

impl Default for RecordingWire {
    fn default() -> Self {
        Self {
            ready: true,
            text_writes: Vec::new(),
            binary_writes: Vec::new(),
            text_sends: Vec::new(),
            binary_sends: Vec::new(),
            flush_count: 0,
            text_write_returns: Default::default(),
            binary_write_returns: Default::default(),
            flush_returns: Default::default(),
        }
    }
}

impl Wire for RecordingWire {
    fn ready(&self) -> bool {
        self.ready
    }
    fn write_text(&mut self, json: &str) -> usize {
        self.text_writes.push(json.to_owned());
        self.text_write_returns.pop_front().unwrap_or(0)
    }
    fn write_binary(&mut self, data: &[u8]) -> usize {
        self.binary_writes.push(data.to_vec());
        self.binary_write_returns.pop_front().unwrap_or(0)
    }
    fn flush(&mut self) -> usize {
        self.flush_count += 1;
        self.flush_returns.pop_front().unwrap_or(0)
    }
    fn send_text(&mut self, json: &str) {
        self.text_sends.push(json.to_owned());
    }
    fn send_binary(&mut self, data: &[u8]) {
        self.binary_sends.push(data.to_vec());
    }
}

/// How a value update is queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendMode {
    /// Drop it.
    Disabled,
    /// Queue every value (a publisher with `sendAll`).
    All,
    /// Replace the queued value for this id, if it is not newer.
    Normal,
    /// Send right away, skipping the queue.
    Immediate,
}

/// Combine a period with another one: their greatest common divisor, at
/// least [`MIN_PERIOD_MS`]. `u32::MAX` means "no period yet".
///
/// ```
/// use nt4_core::outgoing::update_period_calc;
/// assert_eq!(update_period_calc(100, 40), 20);
/// ```
pub fn update_period_calc(period: u32, other: u32) -> u32 {
    let p = if period == u32::MAX {
        other
    } else {
        gcd(period, other)
    };
    p.max(MIN_PERIOD_MS)
}

/// The period that hits every period in `periods`: their greatest common
/// divisor, at least [`MIN_PERIOD_MS`]. An empty list gives `u32::MAX`,
/// meaning "no period".
///
/// ```
/// use nt4_core::outgoing::calculate_period;
/// assert_eq!(calculate_period([100, 40, 60]), 20);
/// ```
pub fn calculate_period(periods: impl IntoIterator<Item = u32>) -> u32 {
    let mut period = u32::MAX;
    for p in periods {
        period = if period == u32::MAX {
            p
        } else {
            gcd(period, p)
        };
    }
    period.max(MIN_PERIOD_MS)
}

/// Round a period to the nearest 10 ms, as ntcore's `RoundPeriodicMs` does.
pub(crate) fn round_periodic_ms(ms: u32) -> u32 {
    let mut rounded = ms - ms % 10;
    if ms % 10 >= 5 && rounded <= u32::MAX - 10 {
        rounded += 10;
    }
    rounded
}

pub(crate) fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Approximate size of a value in bytes, for the backpressure limit.
fn value_size(v: &Value) -> usize {
    16 + match v {
        Value::Boolean(_) | Value::Double(_) | Value::Int(_) | Value::Float(_) => 0,
        Value::String(s) => s.len(),
        Value::Raw(b) => b.len(),
        Value::BooleanArray(a) => a.len(),
        Value::DoubleArray(a) => a.len() * 8,
        Value::IntArray(a) => a.len() * 8,
        Value::FloatArray(a) => a.len() * 4,
        Value::StringArray(a) => a.iter().map(|s| s.len() + 16).sum(),
    }
}

#[derive(Debug, Clone)]
enum Item<M> {
    Text(M),
    Value { time_us: i64, value: Value },
}

#[derive(Debug, Clone)]
struct Entry<M> {
    id: i32,
    item: Item<M>,
}

impl<M> Entry<M> {
    fn size(&self) -> usize {
        match &self.item {
            Item::Text(_) => MESSAGE_OVERHEAD,
            Item::Value { value, .. } => MESSAGE_OVERHEAD + value_size(value),
        }
    }
}

#[derive(Debug, Clone)]
struct Queue<M> {
    msgs: Vec<Entry<M>>,
    next_send_ms: u64,
    period_ms: u32,
}

impl<M> Queue<M> {
    fn new(period_ms: u32) -> Self {
        Self {
            msgs: Vec::new(),
            next_send_ms: 0,
            period_ms,
        }
    }
}

/// Hasher for the queue's ids. They are always assigned on this side (a
/// client's own pubuids, a server's own topic ids), never by the peer, so
/// SipHash's protection against chosen keys isn't needed, and it was a
/// noticeable part of the cost of each value. Multiplying by an odd
/// constant spreads small, dense ids over the table's high bits.
#[derive(Debug, Clone, Copy, Default)]
struct IdHasher(u64);

impl Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(8) ^ u64::from(b)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        }
    }
    fn write_i32(&mut self, id: i32) {
        self.0 = u64::from(id as u32).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
}

type IdMap = HashMap<i32, IdInfo, BuildHasherDefault<IdHasher>>;

#[derive(Debug, Clone, Copy, Default)]
struct IdInfo {
    queue_index: usize,
    /// Index of this id's newest queued value in its queue, if any.
    value_pos: Option<usize>,
}

/// Queued outgoing messages for one connection.
///
/// `M` is the text message type: [`nt4_proto::ClientMessage`] on a client,
/// [`nt4_proto::ServerMessage`] on a server. Ids are publisher ids on a
/// client and topic ids on a server; control messages that belong to no
/// publisher use id 0.
#[derive(Debug, Clone)]
pub struct OutgoingQueue<M> {
    queues: Vec<Queue<M>>,
    id_map: IdMap,
    total_size: usize,
    last_send_ms: u64,
    time_offset_us: i64,
    last_set_period: u32,
    last_set_period_queue_index: usize,
    local: bool,
    /// Reused buffer for encoding one value message.
    scratch: Vec<u8>,
}

impl<M: TextMessage> OutgoingQueue<M> {
    /// Create a queue. A `local` queue (a peer on the same machine) sends
    /// everything immediately instead of queueing.
    pub fn new(local: bool) -> Self {
        Self {
            queues: vec![Queue::new(DEFAULT_PERIOD_MS)],
            id_map: IdMap::default(),
            total_size: 0,
            last_send_ms: 0,
            time_offset_us: 0,
            last_set_period: DEFAULT_PERIOD_MS,
            last_set_period_queue_index: 0,
            local,
            scratch: Vec::new(),
        }
    }

    /// Send messages for `id` every `period_ms`. Anything already queued for
    /// `id` moves to the new queue, keeping its order.
    pub fn set_period(&mut self, id: i32, period_ms: u32) {
        // It's common to set many ids in a row to the same period.
        let queue_index = if self.last_set_period == period_ms {
            self.last_set_period_queue_index
        } else {
            let i = match self.queues.iter().position(|q| q.period_ms == period_ms) {
                Some(i) => i,
                None => {
                    self.queues.push(Queue::new(period_ms));
                    self.queues.len() - 1
                }
            };
            self.last_set_period_queue_index = i;
            self.last_set_period = period_ms;
            i
        };

        match self.id_map.get_mut(&id) {
            None => {
                self.id_map.insert(
                    id,
                    IdInfo {
                        queue_index,
                        value_pos: None,
                    },
                );
            }
            Some(info) => {
                if info.queue_index != queue_index {
                    let old = std::mem::take(&mut self.queues[info.queue_index].msgs);
                    let (moved, kept): (Vec<_>, Vec<_>) = old.into_iter().partition(|e| e.id == id);
                    self.queues[info.queue_index].msgs = kept;
                    let new_msgs = &mut self.queues[queue_index].msgs;
                    let mut value_pos = None;
                    for e in moved {
                        if matches!(e.item, Item::Value { .. }) {
                            value_pos = Some(new_msgs.len());
                        }
                        new_msgs.push(e);
                    }
                    info.value_pos = value_pos;
                    // Other ids in the old queue may have shifted.
                    let old_index = info.queue_index;
                    info.queue_index = queue_index;
                    self.reindex_values(old_index);
                }
            }
        }
    }

    /// Recompute `value_pos` for every id in one queue after entries moved.
    fn reindex_values(&mut self, queue_index: usize) {
        let msgs = &self.queues[queue_index].msgs;
        for (id, info) in self.id_map.iter_mut() {
            if info.queue_index == queue_index {
                info.value_pos = msgs
                    .iter()
                    .rposition(|e| e.id == *id && matches!(e.item, Item::Value { .. }));
            }
        }
    }

    /// Forget `id`'s period, after an unpublish. Queued messages still go out.
    pub fn erase_id(&mut self, id: i32) {
        self.id_map.remove(&id);
    }

    /// Queue a control message for `id`.
    pub fn send_message(&mut self, wire: &mut impl Wire, id: i32, msg: M) {
        if self.local {
            wire.send_text(&encode_text(&msg));
            return;
        }
        let info = *self.id_map.entry(id).or_default();
        let entry = Entry {
            id,
            item: Item::Text(msg),
        };
        self.total_size += entry.size();
        self.queues[info.queue_index].msgs.push(entry);
    }

    /// Queue a value update for `id`, stamped `time_us` (local time, integer
    /// microseconds; 0 means "set default").
    pub fn send_value(
        &mut self,
        wire: &mut impl Wire,
        id: i32,
        time_us: i64,
        value: &Value,
        mut mode: SendMode,
    ) {
        if self.local {
            mode = SendMode::Immediate;
        }
        // Backpressure: stop sending every value if the queue is too full.
        if mode == SendMode::All && self.total_size >= OUTGOING_LIMIT {
            mode = SendMode::Normal;
        }
        match mode {
            SendMode::Disabled => {}
            SendMode::Immediate => {
                encode_value(&mut self.scratch, self.time_offset_us, id, time_us, value);
                wire.send_binary(&self.scratch);
            }
            SendMode::All => self.append_value(id, time_us, value),
            SendMode::Normal => {
                let info = *self.id_map.entry(id).or_default();
                let queue = &mut self.queues[info.queue_index];
                if let Some(pos) = info.value_pos
                    && let Some(elem) = queue.msgs.get_mut(pos)
                    && elem.id == id
                    && let Item::Value {
                        time_us: old_time,
                        value: old_value,
                    } = &mut elem.item
                {
                    // Only replace with a newer (or equal) timestamp.
                    if *old_time == 0 || time_us >= *old_time {
                        let old_size = value_size(old_value);
                        *old_time = time_us;
                        *old_value = value.clone();
                        self.total_size =
                            (self.total_size + value_size(value)).saturating_sub(old_size);
                    }
                    return;
                }
                self.append_value(id, time_us, value);
            }
        }
    }

    fn append_value(&mut self, id: i32, time_us: i64, value: &Value) {
        let info = self.id_map.entry(id).or_default();
        let queue = &mut self.queues[info.queue_index];
        info.value_pos = Some(queue.msgs.len());
        let entry = Entry {
            id,
            item: Item::Value {
                time_us,
                value: value.clone(),
            },
        };
        self.total_size += entry.size();
        queue.msgs.push(entry);
    }

    /// Write every queue that is due (or every non-empty queue if `flush`)
    /// to `wire`, oldest-due first. Does nothing if called again within
    /// [`MIN_PERIOD_MS`] of the last send, or while the wire isn't ready.
    pub fn send_outgoing(&mut self, wire: &mut impl Wire, now_ms: u64, flush: bool) {
        if self.local || self.total_size == 0 {
            return;
        }
        // Rate limit how often remote connections send.
        if now_ms < self.last_send_ms + u64::from(MIN_PERIOD_MS) {
            return;
        }
        if !wire.ready() {
            return; // still sending the last batch
        }

        let mut due: Vec<usize> = (0..self.queues.len())
            .filter(|&i| {
                let q = &self.queues[i];
                !q.msgs.is_empty() && (flush || now_ms >= q.next_send_ms)
            })
            .collect();
        if due.is_empty() {
            return;
        }
        // Send the queue that has waited longest first.
        due.sort_by_key(|&i| self.queues[i].next_send_ms);

        let offset = self.time_offset_us;
        for queue_index in due {
            let queue = &mut self.queues[queue_index];
            let mut written = 0;
            let mut unsent = 0;
            while written < queue.msgs.len() && unsent == 0 {
                let e = &queue.msgs[written];
                unsent = match &e.item {
                    Item::Value { time_us, value } => {
                        encode_value(&mut self.scratch, offset, e.id, *time_us, value);
                        wire.write_binary(&self.scratch)
                    }
                    Item::Text(m) => wire.write_text(&encode_text(m)),
                };
                written += 1;
            }
            if unsent == 0 {
                // Finish writing any partial buffers.
                unsent = wire.flush();
            }
            let sent = written.saturating_sub(unsent);
            let freed: usize = queue.msgs[..sent].iter().map(Entry::size).sum();
            queue.msgs.drain(..sent);
            self.total_size = self.total_size.saturating_sub(freed);
            for info in self.id_map.values_mut() {
                if info.queue_index == queue_index {
                    info.value_pos = info.value_pos.and_then(|p| p.checked_sub(sent));
                }
            }

            // Stay on periodic timing, unless that has fallen behind.
            if unsent == 0 {
                let queue = &mut self.queues[queue_index];
                queue.next_send_ms += u64::from(queue.period_ms);
                if queue.next_send_ms < now_ms {
                    queue.next_send_ms = now_ms + u64::from(queue.period_ms);
                }
            }
        }
        self.last_send_ms = now_ms;
    }

    /// Set the server time offset (server time minus local time, in
    /// microseconds) added to value timestamps as they are written. Only a
    /// client sets this.
    pub fn set_time_offset(&mut self, offset_us: i64) {
        self.time_offset_us = offset_us;
    }

    /// The offset set by [`set_time_offset`](Self::set_time_offset).
    pub fn time_offset(&self) -> i64 {
        self.time_offset_us
    }

    /// Whether anything is queued.
    pub fn is_empty(&self) -> bool {
        self.queues.iter().all(|q| q.msgs.is_empty())
    }
}

fn encode_text<M: TextMessage>(m: &M) -> String {
    let mut s = String::new();
    m.encode(&mut s);
    s
}

/// Encode one value message into `out` (cleared first), adding the time
/// offset to its timestamp.
fn encode_value(out: &mut Vec<u8>, offset_us: i64, id: i32, time_us: i64, value: &Value) {
    let mut timestamp_us = time_us;
    if timestamp_us != 0 {
        timestamp_us = timestamp_us.wrapping_add(offset_us);
        // 0 means "set default", so never produce it by accident.
        if timestamp_us == 0 {
            timestamp_us = 1;
        }
    }
    out.clear();
    encode_binary_parts(out, id, timestamp_us, value);
}
