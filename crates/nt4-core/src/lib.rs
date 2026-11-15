//! NetworkTables 4.1 (NT4) client and server cores, with no sockets
//! (sans-I/O).
//!
//! [`ClientCore`] is the whole NT4 client protocol as a plain state machine:
//! frames and a clock go in, frames and events come out. It handles topic
//! announcements, publish and subscribe, server time sync, keepalive, and
//! sending publishers and subscriptions again after a reconnect.
//! [`ServerCore`] is the server side: topics and their last values, every
//! client's publishers and subscriptions, per-client send queues, meta
//! topics, and the server's own publishers and subscriptions. Neither opens
//! a socket or reads a clock, so they run the same under tokio, in a browser
//! (WASM), or in a unit test with a fake clock.
//!
//! - [`client`]: [`ClientCore`] and its [`Event`]s and [`Transmit`] frames.
//! - [`server`]: [`ServerCore`] and its [`ServerEvent`]s.
//! - [`time_sync`]: the server time offset from RTT pings ([`TimeSync`]).
//! - [`outgoing`]: the per-connection send queue, ported from ntcore's
//!   `NetworkOutgoingQueue`, used by both cores.
//!
//! The behavior follows ntcore's `ClientImpl`, `ServerImpl`, `NetworkPing`
//! and `NetworkOutgoingQueue`; the tests in `tests/` port ntcore's
//! `ServerImplTest`, `TimeSyncTest` and `NetworkOutgoingQueueTest`.
//!
//! ```
//! use nt4_core::{ClientCore, Event, ProtocolVersion, Transmit};
//! use nt4_proto::{encode_binary, BinaryMessage, SubscriptionOptions, Value};
//!
//! let mut client = ClientCore::new();
//! client.subscribe(vec!["/robot/speed".into()], SubscriptionOptions::default());
//! client.connect(0, ProtocolVersion::V4_1);
//! let Some(Transmit::Binary(_ping)) = client.poll_transmit() else { panic!() };
//!
//! // The server answers the RTT ping (it was sent at local time 0) at server
//! // time 5000 us, and the reply arrives at local time 200 us.
//! let mut reply = Vec::new();
//! encode_binary(&mut reply, &BinaryMessage { id: -1, timestamp_us: 5_000, value: Value::Int(0) });
//! client.handle_binary(200, &reply);
//! assert_eq!(client.server_time_offset(), Some(5_000 + 100 - 200));
//!
//! // The subscription was waiting for the time offset and goes out with it.
//! let Some(Transmit::Text(sub)) = client.poll_transmit() else { panic!() };
//! assert!(sub.contains(r#""method":"subscribe""#));
//!
//! // Call `send_outgoing` on every tick: it sends keepalive pings and
//! // whatever is due.
//! client.send_outgoing(10_000, false);
//! assert_eq!(client.poll_transmit(), Some(Transmit::Ping));
//! ```

pub mod client;
mod frames;
mod meta;
pub mod outgoing;
mod persistent;
pub mod server;
pub mod time_sync;

pub use client::{
    ClientCore, ClientError, Event, ProtocolVersion, PubId, PublishOptions, SubId, Topic,
    TopicValue, Transmit,
};
pub use server::{ClientId, Event as ServerEvent, ServerCore, ServerError};
pub use time_sync::{TimeSync, TimeSyncUpdate};

// Compile and run the code in README.md as doc tests.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;
