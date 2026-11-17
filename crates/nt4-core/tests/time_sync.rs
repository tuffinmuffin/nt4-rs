//! Time sync cases ported from ntcore's TimeSyncTest.

// Ported from allwpilib ntcore/src/test/native/cpp/TimeSyncTest.cpp
// at commit 4ccfe967ba1903b1f409b04b99db0922dd3b7318.
// Copyright (c) FIRST and other WPILib contributors.
// The test cases come from that file and stay under the WPILib BSD license;
// see tests/LICENSE-WPILIB.md. The Rust test code was written by Claude.
//
// Upstream drives a whole NetworkTableInstance. Here `ClientCore` stands in
// for the instance: StartClient is `connect` (the socket is open but no RTT
// reply has arrived), StopClient is `disconnect`. "TimeSyncTest TestServer"
// is about an instance acting as the server and waits for the server core
// (see scripts/upstream_na.txt).

use nt4_core::{ClientCore, Event, ProtocolVersion, TimeSyncUpdate};
use nt4_proto::{BinaryMessage, Value, encode_binary};

// upstream: "TimeSyncTest TestLocal"
#[test]
fn test_local() {
    let client = ClientCore::new();
    assert_eq!(client.server_time_offset(), None);
}

// upstream: "TimeSyncTest TestClient"
#[test]
fn test_client() {
    let mut client = ClientCore::new();
    client.connect(0, ProtocolVersion::V4_1);
    assert_eq!(client.server_time_offset(), None);

    client.disconnect("stopped");
    assert_eq!(client.server_time_offset(), None);
}

// The rest of this file was written by Claude, not ported.

fn rtt_reply(sent_us: i64, server_us: i64) -> Vec<u8> {
    let mut out = Vec::new();
    encode_binary(
        &mut out,
        &BinaryMessage {
            id: -1,
            timestamp_us: server_us,
            value: Value::Int(sent_us),
        },
    );
    out
}

fn time_sync_events(client: &mut ClientCore) -> Vec<TimeSyncUpdate> {
    std::iter::from_fn(|| client.poll_event())
        .filter_map(|e| match e {
            Event::TimeSync(u) => Some(u),
            _ => None,
        })
        .collect()
}

#[test]
fn first_reply_sets_offset_and_disconnect_clears_it() {
    let mut client = ClientCore::new();
    client.connect(1_000, ProtocolVersion::V4_1);
    // Ping sent at local 1000 us, server stamped 1_000_000 us, reply at 1400 us.
    client.handle_binary(1_400, &rtt_reply(1_000, 1_000_000));
    assert_eq!(client.server_time_offset(), Some(1_000_000 + 200 - 1_400));
    assert_eq!(client.server_time(10_000), Some(10_000 + 998_800));

    // A later reply doesn't move it, as in ntcore.
    client.handle_binary(2_000, &rtt_reply(1_990, 2_000_000));
    assert_eq!(client.server_time_offset(), Some(998_800));

    client.disconnect("stopped");
    assert_eq!(client.server_time_offset(), None);
    assert_eq!(
        time_sync_events(&mut client),
        vec![
            TimeSyncUpdate {
                server_time_offset_us: 998_800,
                rtt2_us: 200,
                valid: true
            },
            TimeSyncUpdate::INVALID,
        ]
    );
}

#[test]
fn bad_replies_warn_and_are_ignored() {
    let mut client = ClientCore::new();
    client.connect(0, ProtocolVersion::V4_1);

    // Reply claims a send time in the future: negative round trip.
    client.handle_binary(100, &rtt_reply(500, 1_000));
    // Not an integer.
    let mut frame = Vec::new();
    encode_binary(
        &mut frame,
        &BinaryMessage {
            id: -1,
            timestamp_us: 1_000,
            value: Value::Double(1.0),
        },
    );
    client.handle_binary(100, &frame);

    assert_eq!(client.server_time_offset(), None);
    let warnings: Vec<_> = std::iter::from_fn(|| client.poll_event())
        .filter_map(|e| match e {
            Event::Warning(w) => Some(w),
            _ => None,
        })
        .collect();
    assert_eq!(
        warnings,
        vec![
            "RTT ping response has invalid timestamp values",
            "RTT ping response with non-integer type 1",
        ]
    );
}

#[test]
fn reconnect_measures_again() {
    let mut client = ClientCore::new();
    client.connect(0, ProtocolVersion::V4_1);
    client.handle_binary(100, &rtt_reply(0, 10_000));
    assert_eq!(client.server_time_offset(), Some(10_000 + 50 - 100));

    client.disconnect("server restarted");
    client.connect(5_000, ProtocolVersion::V4_1);
    assert_eq!(client.server_time_offset(), None);
    client.handle_binary(5_020, &rtt_reply(5_000, 90));
    assert_eq!(client.server_time_offset(), Some(90 + 10 - 5_020));
}

// `TimeSync` on its own, without a client around it.
#[test]
fn time_sync_reports_the_reply_that_set_it() {
    use nt4_core::time_sync::TimeSync;
    let mut sync = TimeSync::default();
    assert_eq!(sync, TimeSync::new());
    assert_eq!((sync.offset(), sync.rtt2()), (None, None));

    let reply = BinaryMessage {
        id: -1,
        timestamp_us: 50_000,
        value: Value::Int(1_000),
    };
    let update = sync.handle_reply(1_300, &reply).unwrap().unwrap();
    assert_eq!(update.rtt2_us, 150);
    assert_eq!(sync.offset(), Some(50_000 + 150 - 1_300));
    assert_eq!(sync.rtt2(), Some(150));
    // Once set, later replies are fine but unused.
    assert_eq!(sync.handle_reply(2_000, &reply), Ok(None));
}

#[test]
fn time_sync_rejects_an_offset_that_overflows() {
    use nt4_core::time_sync::TimeSync;
    const INVALID: &str = "RTT ping response has invalid timestamp values";
    let reply = |server_us, sent_us| BinaryMessage {
        id: -1,
        timestamp_us: server_us,
        value: Value::Int(sent_us),
    };
    // Server time + rtt/2 overflows.
    let mut sync = TimeSync::new();
    assert_eq!(
        sync.handle_reply(10, &reply(i64::MAX, 0)),
        Err(INVALID.into())
    );
    // Server time - now overflows.
    assert_eq!(
        sync.handle_reply(1, &reply(i64::MIN, 1)),
        Err(INVALID.into())
    );
    // The round trip itself overflows.
    assert_eq!(
        sync.handle_reply(i64::MAX, &reply(0, -1)),
        Err(INVALID.into())
    );
    assert_eq!(sync.offset(), None);
}
