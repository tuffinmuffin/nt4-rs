//! NetworkTables 4.1 (NT4) for tokio.
//!
//! With the `client` feature (on by default), [`Client`] connects to an NT4
//! server such as robot code, keeps the connection and time sync alive,
//! reconnects when it drops, and gives typed [`Publisher`]s and
//! [`Subscriber`]s. With the `server` feature, [`Server`] is an NT4 server
//! that ntcore clients (dashboards, robot code, coprocessors) connect to,
//! with the same typed handles for the server's own topics.
//!
//! Struct and protobuf topics (`struct:Pose2d`, ...) are typed too:
//! implement [`NtStruct`] or [`NtProtobuf`], or use the WPILib types in
//! the [`nt4-wpimath`](https://docs.rs/nt4-wpimath) crate. Publishing one registers its schemas under `/.schema/`, as
//! ntcore does; see [`structs`].
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
//! Re-exports: [`Event`], [`ServerEvent`], [`PublishOptions`], [`TopicValue`] from
//! [`nt4_core`], and [`NtType`], [`Value`], [`Properties`],
//! [`SubscriptionOptions`] from [`nt4_proto`].

// The docs link the client and server to each other. With only one of the
// two features those links have nothing to point at; docs.rs builds with
// all features, where they resolve.
#![cfg_attr(
    not(all(feature = "client", feature = "server")),
    allow(rustdoc::broken_intra_doc_links)
)]

#[cfg(feature = "client")]
pub mod client;
#[cfg(any(feature = "client", feature = "server"))]
pub mod handles;
#[cfg(feature = "server")]
pub mod server;

/// How much tungstenite reads from the socket at a time. Its default,
/// 128 KiB, is zero-filled before every read; NT frames are small (on
/// localhost ntcore sends one ~50-byte frame per value), so that fill was
/// the largest single cost of receiving. Bigger messages still arrive, over
/// several reads.
#[cfg(any(feature = "client", feature = "server"))]
pub(crate) const READ_BUFFER_SIZE: usize = 8 * 1024;

pub mod structs;
pub mod typed;

#[cfg(feature = "client")]
pub use client::{Client, ClientConfig};
#[cfg(any(feature = "client", feature = "server"))]
pub use handles::{
    ClientStopped, Events, GenericPublisher, Publisher, Subscriber, SubscriptionHandle,
};
pub use nt4_core::{
    ClientError, Event, ProtocolVersion, PublishOptions, ServerEvent, Topic, TopicValue,
};
pub use nt4_proto::{NtType, Properties, SubscriptionOptions, Value};
#[cfg(feature = "server")]
pub use server::{ConnectedClient, Server, ServerConfig};
pub use structs::{NtProtobuf, NtStruct, Proto};
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
