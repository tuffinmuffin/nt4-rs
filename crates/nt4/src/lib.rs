//! NetworkTables 4.1 (NT4) for tokio.
//!
//! With the `client` feature (on by default), [`Client`] connects to an NT4
//! server such as robot code, keeps the connection and time sync alive,
//! reconnects when it drops, and gives typed [`Publisher`]s and
//! [`Subscriber`]s.
//!
//! The protocol work happens in [`nt4_core`], which has no I/O; this crate
//! adds the WebSocket (tokio-tungstenite) and the clock.
//!
//! ```no_run
//! # async fn demo() {
//! use nt4::{Client, ClientConfig};
//!
//! let client = Client::start(ClientConfig::server("viewer", "127.0.0.1", 5810));
//! let x = client.subscribe::<f64>("/robot/x", Default::default());
//! client.wait_for_sync(std::time::Duration::from_secs(5)).await;
//! println!("x = {:?}", x.get());
//! # }
//! ```
//!
//! Re-exports: [`Event`], [`PublishOptions`], [`TopicValue`] from
//! [`nt4_core`], and [`NtType`], [`Value`], [`Properties`],
//! [`SubscriptionOptions`] from [`nt4_proto`].

#[cfg(feature = "client")]
pub mod client;
pub mod typed;

#[cfg(feature = "client")]
pub use client::{
    Client, ClientConfig, ClientStopped, Events, GenericPublisher, Publisher, Subscriber,
    SubscriptionHandle,
};
pub use nt4_core::{ClientError, Event, ProtocolVersion, PublishOptions, Topic, TopicValue};
pub use nt4_proto::{NtType, Properties, SubscriptionOptions, Value};
pub use typed::NtValue;

/// The wire format crate, [`nt4_proto`], for constants such as
/// [`ROBOT_CODE_PORT`](nt4_proto::ROBOT_CODE_PORT) and lower-level types.
pub use nt4_proto as proto;

/// The sans-I/O protocol crate, [`nt4_core`], for driving the protocol
/// yourself.
pub use nt4_core as core;

// Compile and run the code in README.md as doc tests.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;
