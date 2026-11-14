//! NetworkTables 4.1 (NT4) wire format, with no I/O.
//!
//! NT4 runs over a WebSocket. Text frames carry JSON control messages
//! (publish, subscribe, announce, ...) and binary frames carry MessagePack
//! value updates. This crate turns those frames into Rust types and back. It
//! never opens a socket, so the client and server built on it can be tested
//! without a network.
//!
//! - [`NtType`]: the type strings a topic announces (`"double"`, `"struct:Pose2d"`, ...).
//! - [`ClientMessage`] / [`ServerMessage`]: the JSON control messages, with
//!   [`decode_client_text`] / [`decode_server_text`] to read a text frame.
//! - [`Value`] and [`BinaryMessage`]: value updates, with
//!   [`encode_binary`] and [`BinaryReader`] for binary frames.
//!
//! Encoding matches ntcore byte for byte, and decoding accepts and rejects
//! the same input with the same warning text. The tests in `tests/` are
//! ported from ntcore's `WireEncoderTest` and `WireDecoderTest`.
//!
//! Protocol reference: <https://github.com/wpilibsuite/allwpilib/blob/main/ntcore/doc/networktables4.adoc>
//!
//! ```
//! use nt4_proto::{decode_client_text, ClientMessage};
//! let frame = r#"[{"method":"unsubscribe","params":{"subuid":5}}]"#;
//! let decoded = decode_client_text(frame);
//! assert!(decoded.warnings.is_empty());
//! assert_eq!(decoded.messages, vec![ClientMessage::Unsubscribe { subuid: 5 }]);
//! ```

pub mod text;
pub mod types;
pub mod value;

pub use text::{
    ClientMessage, Properties, ServerMessage, SubscriptionOptions, TextDecode, TextMessage,
    decode_client_text, decode_server_text, encode_text_frame,
};
pub use types::NtType;
pub use value::{BinaryError, BinaryMessage, BinaryReader, Value, encode_binary};

/// WebSocket subprotocol for NT 4.1, tried first.
pub const SUBPROTOCOL_V4_1: &str = "v4.1.networktables.first.wpi.edu";
/// WebSocket subprotocol for NT 4.0, the fallback.
pub const SUBPROTOCOL_V4_0: &str = "networktables.first.wpi.edu";
/// WebSocket subprotocol for the RTT-only side channel (binary pings only).
pub const SUBPROTOCOL_RTT: &str = "rtt.networktables.first.wpi.edu";
/// NT4 port served by robot code.
pub const ROBOT_CODE_PORT: u16 = 5810;
/// Topic id used in binary frames for time sync (RTT) pings and replies.
pub const RTT_ID: i32 = -1;

/// Build the WebSocket URL a client connects to, e.g. `ws://10.32.0.2:5810/nt/my-client`.
///
/// ```
/// assert_eq!(
///     nt4_proto::client_url("10.32.0.2", 5810, "my-client"),
///     "ws://10.32.0.2:5810/nt/my-client"
/// );
/// ```
pub fn client_url(host: &str, port: u16, client_name: &str) -> String {
    format!("ws://{host}:{port}/nt/{client_name}")
}
