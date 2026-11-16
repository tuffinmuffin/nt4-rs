# nt4-proto

The [NetworkTables 4.1](https://github.com/wpilibsuite/allwpilib/blob/main/ntcore/doc/networktables4.adoc)
wire format for Rust, with no I/O: JSON control messages in WebSocket text
frames and MessagePack value updates in binary frames.

Most programs want the [`nt4`](https://docs.rs/nt4) client crate. This one is
for building your own client, server or tooling on the raw messages.

```rust
use nt4_proto::{decode_client_text, ClientMessage};

let frame = r#"[{"method":"unsubscribe","params":{"subuid":5}}]"#;
let decoded = decode_client_text(frame);
assert_eq!(decoded.messages, vec![ClientMessage::Unsubscribe { subuid: 5 }]);
```

Encoding matches WPILib's ntcore byte for byte, and decoding accepts and
rejects the same input with the same warning text. Every case of ntcore's
`WireEncoderTest` and `WireDecoderTest` is ported as a test.

`nt4_proto::structs` reads WPILib struct values whose type is only known at
run time: it parses the schemas from `/.schema/struct:*` topics (a port of
wpiutil's `SchemaParser` and `DynamicStruct`, with their tests) and reads
and writes fields by name.

License: MIT. Ported WPILib tests keep WPILib's BSD license
(`tests/LICENSE-WPILIB.md`).
