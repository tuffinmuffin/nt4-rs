//! Turning an [`OutgoingQueue`](crate::outgoing::OutgoingQueue)'s output into
//! WebSocket frames, shared by the client and server cores.

use std::collections::VecDeque;

use crate::client::Transmit;
use crate::outgoing::{self, Wire};

/// Collects the queue's output into WebSocket frames: consecutive text
/// messages share one JSON array frame, consecutive values one binary frame.
#[derive(Debug, Default)]
pub(crate) struct FrameBuffer {
    pub(crate) out: VecDeque<Transmit>,
    text: Option<String>,
    binary: Option<Vec<u8>>,
    pub(crate) blocked: bool,
}

impl FrameBuffer {
    /// End the frame being built, if any.
    pub(crate) fn finish(&mut self) {
        if let Some(mut t) = self.text.take() {
            t.push(']');
            self.out.push_back(Transmit::Text(t));
        }
        if let Some(b) = self.binary.take() {
            self.out.push_back(Transmit::Binary(b));
        }
    }

    /// Drop everything, sent or not.
    pub(crate) fn clear(&mut self) {
        self.out.clear();
        self.text = None;
        self.binary = None;
    }

    /// Queue a frame that isn't from the queue (a ping or a close), after
    /// whatever is being built.
    pub(crate) fn push(&mut self, t: Transmit) {
        self.finish();
        self.out.push_back(t);
    }
}

impl Wire for FrameBuffer {
    fn ready(&self) -> bool {
        !self.blocked
    }
    fn write_text(&mut self, json: &str) -> usize {
        if self.binary.is_some() {
            self.finish();
        }
        match &mut self.text {
            Some(t) => {
                t.push(',');
                t.push_str(json);
            }
            None => self.text = Some(format!("[{json}")),
        }
        0
    }
    fn write_binary(&mut self, data: &[u8]) -> usize {
        if self.text.is_some() {
            self.finish();
        }
        self.binary
            .get_or_insert_with(Vec::new)
            .extend_from_slice(data);
        0
    }
    fn flush(&mut self) -> usize {
        self.finish();
        0
    }
    fn send_text(&mut self, json: &str) {
        self.finish();
        self.out.push_back(Transmit::Text(format!("[{json}]")));
    }
    fn send_binary(&mut self, data: &[u8]) {
        self.finish();
        self.out.push_back(Transmit::Binary(data.to_vec()));
    }
}

/// The time to give [`OutgoingQueue::send_outgoing`](crate::outgoing::OutgoingQueue::send_outgoing).
/// The queue (like ntcore's) starts with "last sent at 0 ms" and sends at
/// most every `MIN_PERIOD_MS`, so on a clock that starts at 0 it would hold
/// everything back for the first few milliseconds, and the messages waiting
/// for the first RTT reply would miss it and wait for the next tick.
/// ntcore's clock never reads that low; shift ours so it doesn't either.
pub(crate) fn queue_ms(now_us: i64) -> u64 {
    to_ms(now_us) + u64::from(outgoing::MIN_PERIOD_MS)
}

/// Microseconds to milliseconds, with times before 0 as 0.
pub(crate) fn to_ms(us: i64) -> u64 {
    u64::try_from(us / 1000).unwrap_or(0)
}
