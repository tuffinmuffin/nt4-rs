//! Publish a WPILib struct (`Translation2d`) with its schema, so dashboards
//! such as AdvantageScope can decode it.
//!
//! ```text
//! cargo run -p nt4 --example struct_topic -- 127.0.0.1
//! ```
//!
//! A struct topic has type `struct:<Name>` and carries the struct's fields
//! packed little-endian. Its schema is published once at
//! `/.schema/struct:<Name>` with type `structschema`. (Schema helpers are
//! planned; this shows the raw form.)

use std::time::Duration;

use nt4::{Client, ClientConfig, NtType, Properties, PublishOptions, Value};

/// WPILib's Translation2d: two doubles, in meters.
fn translation2d(x: f64, y: f64) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(16);
    bytes.extend_from_slice(&x.to_le_bytes());
    bytes.extend_from_slice(&y.to_le_bytes());
    bytes
}

#[tokio::main]
async fn main() {
    let host = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1".into());
    let client = Client::start(ClientConfig::server("nt4-struct", host, 5810));

    let schema = client.publish_generic(
        "/.schema/struct:Translation2d",
        NtType::StructSchema,
        Properties::new(),
        PublishOptions::default(),
    );
    schema
        .set(Value::Raw(b"double x;double y".to_vec()))
        .expect("raw matches structschema");

    let ty = NtType::Struct {
        name: "Translation2d".into(),
        array: false,
    };
    let target = client.publish_generic(
        "/example/target",
        ty,
        Properties::new(),
        PublishOptions::default(),
    );

    for i in 0.. {
        let angle = f64::from(i) * 0.05;
        target
            .set(Value::Raw(translation2d(
                2.0 * angle.cos(),
                2.0 * angle.sin(),
            )))
            .expect("raw matches struct");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
