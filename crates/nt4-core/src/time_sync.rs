//! Server time offset from RTT pings, as ntcore's `ClientImpl` computes it.
//!
//! NT4 value timestamps are in the server's clock. A client learns the
//! offset between its clock and the server's by sending a binary message on
//! id `-1` whose value is its own current time. The server answers with the
//! same value, stamped with the server's time. Half the round trip
//! (`rtt2`) estimates how long ago the server stamped it, so
//!
//! ```text
//! offset = server_timestamp + rtt2 - now
//! server_time = local_time + offset
//! ```
//!
//! All times are integer microseconds. Like ntcore, the first reply that
//! gives a valid offset is used for the rest of the connection.
//!
//! ```
//! use nt4_core::TimeSync;
//! use nt4_proto::{BinaryMessage, Value};
//!
//! let mut sync = TimeSync::new();
//! let ping = TimeSync::ping(1_000); // sent at local time 1000 us
//! // The server stamped its reply at 50_000 us; it arrives at local 1_200 us.
//! let reply = BinaryMessage { id: -1, timestamp_us: 50_000, value: ping.value };
//! let update = sync.handle_reply(1_200, &reply).unwrap().unwrap();
//! assert_eq!(update.rtt2_us, 100);
//! assert_eq!(update.server_time_offset_us, 50_000 + 100 - 1_200);
//! assert_eq!(sync.offset(), Some(48_900));
//! ```

use nt4_proto::{BinaryMessage, RTT_ID, Value};

/// A change in the server time offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSyncUpdate {
    /// Server time minus local time, in microseconds. 0 when not valid.
    pub server_time_offset_us: i64,
    /// Half the measured round trip time, in microseconds. 0 when not valid.
    pub rtt2_us: i64,
    /// False when the offset is no longer known (disconnected).
    pub valid: bool,
}

impl TimeSyncUpdate {
    /// The update sent when the offset stops being known.
    pub const INVALID: TimeSyncUpdate = TimeSyncUpdate {
        server_time_offset_us: 0,
        rtt2_us: 0,
        valid: false,
    };
}

/// Time sync state for one connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeSync {
    rtt2_us: i64,
    offset_us: Option<i64>,
}

impl Default for TimeSync {
    fn default() -> Self {
        Self::new()
    }
}

impl TimeSync {
    /// No offset known yet.
    pub fn new() -> Self {
        Self {
            rtt2_us: 0,
            offset_us: None,
        }
    }

    /// The RTT ping to send at local time `now_us`: id `-1`, timestamp 0,
    /// and `now_us` as an integer value.
    pub fn ping(now_us: i64) -> BinaryMessage {
        BinaryMessage {
            id: RTT_ID,
            timestamp_us: 0,
            value: Value::Int(now_us),
        }
    }

    /// The server time offset in microseconds, once a reply has set it.
    pub fn offset(&self) -> Option<i64> {
        self.offset_us
    }

    /// Half the round trip time of the reply that set the offset, in
    /// microseconds.
    pub fn rtt2(&self) -> Option<i64> {
        self.offset_us.map(|_| self.rtt2_us)
    }

    /// Handle an RTT reply that arrived at local time `now_us`.
    ///
    /// Returns `Ok(Some(update))` when the offset changed, `Ok(None)` when
    /// the reply was fine but not used (the offset is already known), and
    /// `Err(warning)` with ntcore's warning text for a bad reply.
    pub fn handle_reply(
        &mut self,
        now_us: i64,
        reply: &BinaryMessage,
    ) -> Result<Option<TimeSyncUpdate>, String> {
        if self.offset_us.is_some() {
            return Ok(None);
        }
        let Value::Int(sent_us) = reply.value else {
            return Err(format!(
                "RTT ping response with non-integer type {}",
                reply.value.type_id()
            ));
        };
        const INVALID: &str = "RTT ping response has invalid timestamp values";
        let rtt = match now_us.checked_sub(sent_us) {
            Some(r) if r >= 0 => r,
            _ => return Err(INVALID.into()),
        };
        let rtt2 = rtt / 2;
        let offset = reply
            .timestamp_us
            .checked_add(rtt2)
            .and_then(|t| t.checked_sub(now_us))
            .filter(|&o| o != i64::MIN)
            .ok_or_else(|| INVALID.to_string())?;
        self.rtt2_us = rtt2;
        self.offset_us = Some(offset);
        Ok(Some(TimeSyncUpdate {
            server_time_offset_us: offset,
            rtt2_us: rtt2,
            valid: true,
        }))
    }
}
