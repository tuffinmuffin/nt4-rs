//! Outgoing queue scenarios ported from ntcore's NetworkOutgoingQueueTest.

// Ported from allwpilib ntcore/src/test/native/cpp/net/NetworkOutgoingQueueTest.cpp
// at commit 4ccfe967ba1903b1f409b04b99db0922dd3b7318.
// Copyright (c) FIRST and other WPILib contributors.
// The scenarios and expected output come from that file and stay under the
// WPILib BSD license; see tests/LICENSE-WPILIB.md. The Rust test code was
// written by Claude.
//
// Upstream's RecordingWireConnection is `RecordingWire` here. Upstream
// `ServerMessage` queues become `OutgoingQueue<ServerMessage>`; values carry
// their timestamp beside them instead of inside the value. ntcore 2027 keeps
// times in nanoseconds; the numbers are kept and read as microseconds.

use nt4_core::outgoing::{
    MIN_PERIOD_MS, OutgoingQueue, RecordingWire, SendMode, calculate_period, update_period_calc,
};
use nt4_core::{ClientCore, Event, ProtocolVersion};
use nt4_proto::{
    BinaryMessage, BinaryReader, ClientMessage, NtType, Properties, ServerMessage, Value,
    encode_binary,
};

/// Decode one value message; `local_time_offset` is added to its timestamp.
fn decode_binary(data: &[u8], local_time_offset: i64) -> (i32, i64, Value) {
    let mut reader = BinaryReader::new(data);
    let msg = reader.next().expect("one message").expect("decodes");
    assert!(reader.next().is_none(), "exactly one message");
    (msg.id, msg.timestamp_us + local_time_offset, msg.value)
}

fn publish(pubuid: i32, name: &str) -> ClientMessage {
    ClientMessage::Publish {
        name: name.into(),
        pubuid,
        ty: NtType::Double,
        properties: Properties::new(),
    }
}

fn publish_json(pubuid: i32, name: &str) -> String {
    format!(
        r#"{{"method":"publish","params":{{"name":"{name}","properties":{{}},"pubuid":{pubuid},"type":"double"}}}}"#
    )
}

fn server_queue() -> OutgoingQueue<ServerMessage> {
    OutgoingQueue::new(false)
}

fn client_queue() -> OutgoingQueue<ClientMessage> {
    OutgoingQueue::new(false)
}

fn double(d: &Value) -> f64 {
    match d {
        Value::Double(d) => *d,
        other => panic!("expected double, got {other:?}"),
    }
}

fn raw_len(d: &Value) -> usize {
    match d {
        Value::Raw(r) => r.len(),
        other => panic!("expected raw, got {other:?}"),
    }
}

// upstream: "NetworkOutgoingQueueTest UpdatePeriodCalcUsesGcdAndMinimum"
#[test]
fn update_period_calc_uses_gcd_and_minimum() {
    assert_eq!(update_period_calc(u32::MAX, 100), 100);
    assert_eq!(update_period_calc(100, 40), 20);
    assert_eq!(update_period_calc(6, 4), MIN_PERIOD_MS);
}

// upstream: "ClientImpl rejects overflowing RTT timestamps"
#[test]
fn client_rejects_overflowing_rtt_timestamps() {
    let mut client = ClientCore::new();
    client.connect(0, ProtocolVersion::V4_1);
    let mut encoded = Vec::new();
    encode_binary(
        &mut encoded,
        &BinaryMessage {
            id: -1,
            timestamp_us: 1,
            value: Value::Int(i64::MIN),
        },
    );

    client.handle_binary(0, &encoded);

    let mut time_sync_updates = 0;
    while let Some(e) = client.poll_event() {
        if matches!(e, Event::TimeSync(_)) {
            time_sync_updates += 1;
        }
    }
    assert_eq!(time_sync_updates, 0);
    assert_eq!(client.server_time_offset(), None);
}

// upstream: "NetworkOutgoingQueueTest CalculatePeriodUsesGcdAndMinimum"
#[test]
fn calculate_period_uses_gcd_and_minimum() {
    assert_eq!(calculate_period([100, 40, 60]), 20);
    assert_eq!(calculate_period([2, 4]), MIN_PERIOD_MS);
}

// upstream: "NetworkOutgoingQueueTest RemoteTextMessageWaitsForReadyAndMinimumPeriod"
#[test]
fn remote_text_message_waits_for_ready_and_minimum_period() {
    let mut wire = RecordingWire::default();
    let mut queue = client_queue();

    queue.send_message(&mut wire, 5, publish(5, "test"));

    queue.send_outgoing(&mut wire, 4, false);
    assert!(wire.text_writes.is_empty());

    wire.ready = false;
    queue.send_outgoing(&mut wire, 5, false);
    assert!(wire.text_writes.is_empty());

    wire.ready = true;
    queue.send_outgoing(&mut wire, 5, false);
    assert_eq!(wire.text_writes, vec![publish_json(5, "test")]);
    assert_eq!(wire.flush_count, 1);

    queue.send_outgoing(&mut wire, 10, false);
    assert_eq!(wire.text_writes.len(), 1);
}

// upstream: "NetworkOutgoingQueueTest FlushWaitsForMinimumPeriod"
#[test]
fn flush_waits_for_minimum_period() {
    let mut wire = RecordingWire::default();
    let mut queue = client_queue();

    queue.send_message(&mut wire, 1, publish(1, "first"));
    queue.send_outgoing(&mut wire, 5, false);
    assert_eq!(wire.text_writes.len(), 1);

    queue.send_message(&mut wire, 2, publish(2, "second"));
    queue.send_outgoing(&mut wire, 6, true);
    assert_eq!(wire.text_writes.len(), 1);

    queue.send_outgoing(&mut wire, 10, true);
    assert_eq!(wire.text_writes.len(), 2);
    assert_eq!(wire.text_writes[1], publish_json(2, "second"));
}

// upstream: "NetworkOutgoingQueueTest NormalValueKeepsOnlyNewestQueuedTimestamp"
#[test]
fn normal_value_keeps_only_newest_queued_timestamp() {
    let mut wire = RecordingWire::default();
    let mut queue = server_queue();

    queue.send_value(&mut wire, 7, 100_000, &Value::Double(1.0), SendMode::Normal);
    queue.send_value(&mut wire, 7, 200_000, &Value::Double(2.0), SendMode::Normal);
    queue.send_value(&mut wire, 7, 150_000, &Value::Double(3.0), SendMode::Normal);

    queue.send_outgoing(&mut wire, 5, true);

    assert_eq!(wire.binary_writes.len(), 1);
    let (id, time, value) = decode_binary(&wire.binary_writes[0], 0);
    assert_eq!(id, 7);
    assert_eq!(time, 200_000);
    assert_eq!(double(&value), 2.0);
}

// upstream: "NetworkOutgoingQueueTest NormalValueShrinkUpdatesTotalSize"
#[test]
fn normal_value_shrink_updates_total_size() {
    let mut wire = RecordingWire::default();
    let mut queue = server_queue();

    let big = Value::String("x".repeat(900 * 1024));
    queue.send_value(&mut wire, 1, 10, &big, SendMode::Normal);
    queue.send_value(&mut wire, 1, 20, &Value::Double(1.0), SendMode::Normal);
    queue.send_value(&mut wire, 1, 30, &Value::Double(2.0), SendMode::All);
    queue.send_value(&mut wire, 1, 40, &Value::Double(3.0), SendMode::All);

    queue.send_outgoing(&mut wire, 5, true);

    assert_eq!(wire.binary_writes.len(), 3);
    for (i, expected) in [1.0, 2.0, 3.0].into_iter().enumerate() {
        let (id, _, value) = decode_binary(&wire.binary_writes[i], 0);
        assert_eq!(id, 1);
        assert_eq!(double(&value), expected);
    }
}

// upstream: "NetworkOutgoingQueueTest NormalValueShrinkBelowBackpressureUpdatesTotalSize"
#[test]
fn normal_value_shrink_below_backpressure_updates_total_size() {
    let mut wire = RecordingWire::default();
    let mut queue = server_queue();

    let raw = |n: usize, b: u8| Value::Raw(vec![b; n]);
    queue.send_value(
        &mut wire,
        1,
        10_000,
        &raw(1100 * 1024, b'x'),
        SendMode::Normal,
    );
    queue.send_value(&mut wire, 1, 20_000, &raw(32, b'y'), SendMode::Normal);
    queue.send_value(&mut wire, 1, 30_000, &raw(32, b'z'), SendMode::All);
    queue.send_value(&mut wire, 1, 40_000, &raw(32, b'w'), SendMode::All);

    queue.send_outgoing(&mut wire, 5, true);

    assert_eq!(wire.binary_writes.len(), 3);
    for (i, expected_time) in [20_000, 30_000, 40_000].into_iter().enumerate() {
        let (id, time, value) = decode_binary(&wire.binary_writes[i], 0);
        assert_eq!(id, 1);
        assert_eq!(time, expected_time);
        assert_eq!(raw_len(&value), 32);
    }
}

// upstream: "NetworkOutgoingQueueTest NormalValueGrowThenDrainClearsTotalSize"
#[test]
fn normal_value_grow_then_drain_clears_total_size() {
    let mut wire = RecordingWire::default();
    let mut queue = server_queue();

    let raw = |n: usize, b: u8| Value::Raw(vec![b; n]);
    queue.send_value(&mut wire, 1, 10_000, &raw(32, b'x'), SendMode::Normal);
    queue.send_value(&mut wire, 1, 20_000, &raw(4096, b'y'), SendMode::Normal);
    queue.send_outgoing(&mut wire, 5, true);

    wire.binary_writes.clear();

    queue.send_value(&mut wire, 1, 30_000, &raw(32, b'z'), SendMode::All);
    queue.send_value(&mut wire, 1, 40_000, &raw(32, b'w'), SendMode::All);
    queue.send_outgoing(&mut wire, 10, true);

    assert_eq!(wire.binary_writes.len(), 2);
    for (i, expected_time) in [30_000, 40_000].into_iter().enumerate() {
        let (id, time, value) = decode_binary(&wire.binary_writes[i], 0);
        assert_eq!(id, 1);
        assert_eq!(time, expected_time);
        assert_eq!(raw_len(&value), 32);
    }
}

// upstream: "NetworkOutgoingQueueTest NormalValueGrowAcrossBackpressureLimitCoalesces"
#[test]
fn normal_value_grow_across_backpressure_limit_coalesces() {
    let mut wire = RecordingWire::default();
    let mut queue = server_queue();

    let raw = |n: usize, b: u8| Value::Raw(vec![b; n]);
    queue.send_value(&mut wire, 1, 10_000, &raw(32, b'x'), SendMode::Normal);
    queue.send_value(
        &mut wire,
        1,
        20_000,
        &raw(1024 * 1024, b'y'),
        SendMode::Normal,
    );
    queue.send_value(&mut wire, 1, 30_000, &raw(64, b'z'), SendMode::All);

    queue.send_outgoing(&mut wire, 5, true);

    assert_eq!(wire.binary_writes.len(), 1);
    let (id, time, value) = decode_binary(&wire.binary_writes[0], 0);
    assert_eq!(id, 1);
    assert_eq!(time, 30_000);
    assert_eq!(raw_len(&value), 64);
}

// upstream: "NetworkOutgoingQueueTest OlderNormalValueDoesNotChangeTotalSize"
#[test]
fn older_normal_value_does_not_change_total_size() {
    let mut wire = RecordingWire::default();
    let mut queue = server_queue();

    let raw = |n: usize, b: u8| Value::Raw(vec![b; n]);
    queue.send_value(
        &mut wire,
        1,
        20_000,
        &raw(1100 * 1024, b'x'),
        SendMode::Normal,
    );
    queue.send_value(&mut wire, 1, 10_000, &raw(32, b'y'), SendMode::Normal);
    queue.send_value(&mut wire, 1, 30_000, &raw(64, b'z'), SendMode::All);

    queue.send_outgoing(&mut wire, 5, true);

    assert_eq!(wire.binary_writes.len(), 1);
    let (id, time, value) = decode_binary(&wire.binary_writes[0], 0);
    assert_eq!(id, 1);
    assert_eq!(time, 30_000);
    assert_eq!(raw_len(&value), 64);
}

// upstream: "NetworkOutgoingQueueTest NormalValueReplacesAfterPeriodChange"
#[test]
fn normal_value_replaces_after_period_change() {
    let mut wire = RecordingWire::default();
    let mut queue = server_queue();

    queue.set_period(1, 100);
    queue.send_value(&mut wire, 1, 10_000, &Value::Double(1.0), SendMode::Normal);

    queue.set_period(2, 50);
    queue.send_value(&mut wire, 2, 20_000, &Value::Double(2.0), SendMode::Normal);

    queue.set_period(1, 50);
    queue.send_value(&mut wire, 1, 30_000, &Value::Double(3.0), SendMode::Normal);

    queue.send_outgoing(&mut wire, 5, true);

    assert_eq!(wire.binary_writes.len(), 2);
    let (id0, _, value0) = decode_binary(&wire.binary_writes[0], 0);
    assert_eq!(id0, 2);
    assert_eq!(double(&value0), 2.0);

    let (id1, time1, value1) = decode_binary(&wire.binary_writes[1], 0);
    assert_eq!(id1, 1);
    assert_eq!(time1, 30_000);
    assert_eq!(double(&value1), 3.0);
}

// upstream: "NetworkOutgoingQueueTest NormalValueReplacesLastValueAfterPeriodChange"
#[test]
fn normal_value_replaces_last_value_after_period_change() {
    let mut wire = RecordingWire::default();
    let mut queue = server_queue();

    queue.set_period(1, 100);
    queue.send_value(&mut wire, 1, 10_000, &Value::Double(1.0), SendMode::All);
    queue.send_value(&mut wire, 1, 20_000, &Value::Double(2.0), SendMode::All);

    queue.set_period(2, 50);
    queue.send_value(&mut wire, 2, 40_000, &Value::Double(4.0), SendMode::Normal);

    queue.set_period(1, 50);
    queue.send_value(&mut wire, 1, 30_000, &Value::Double(3.0), SendMode::Normal);

    queue.send_outgoing(&mut wire, 5, true);

    assert_eq!(wire.binary_writes.len(), 3);
    let (id0, _, value0) = decode_binary(&wire.binary_writes[0], 0);
    assert_eq!(id0, 2);
    assert_eq!(double(&value0), 4.0);

    let (id1, time1, value1) = decode_binary(&wire.binary_writes[1], 0);
    assert_eq!(id1, 1);
    assert_eq!(time1, 10_000);
    assert_eq!(double(&value1), 1.0);

    let (id2, time2, value2) = decode_binary(&wire.binary_writes[2], 0);
    assert_eq!(id2, 1);
    assert_eq!(time2, 30_000);
    assert_eq!(double(&value2), 3.0);
}

// upstream: "NetworkOutgoingQueueTest PeriodChangeMovesMixedMessagesInStableOrder"
#[test]
fn period_change_moves_mixed_messages_in_stable_order() {
    let mut wire = RecordingWire::default();
    let mut queue = client_queue();

    queue.set_period(1, 100);
    queue.send_message(&mut wire, 1, publish(1, "first"));
    queue.send_value(&mut wire, 1, 10_000, &Value::Double(1.0), SendMode::All);
    queue.send_message(&mut wire, 1, publish(1, "second"));

    queue.set_period(2, 50);
    queue.send_message(&mut wire, 2, publish(2, "dest"));

    queue.set_period(1, 50);
    queue.send_value(&mut wire, 1, 20_000, &Value::Double(2.0), SendMode::Normal);

    queue.send_outgoing(&mut wire, 5, true);

    assert_eq!(
        wire.text_writes,
        vec![
            publish_json(2, "dest"),
            publish_json(1, "first"),
            publish_json(1, "second"),
        ]
    );

    assert_eq!(wire.binary_writes.len(), 1);
    let (id, time, value) = decode_binary(&wire.binary_writes[0], 0);
    assert_eq!(id, 1);
    assert_eq!(time, 20_000);
    assert_eq!(double(&value), 2.0);
}

// upstream: "NetworkOutgoingQueueTest SendAllValuesAppendBelowBackpressureLimit"
#[test]
fn send_all_values_append_below_backpressure_limit() {
    let mut wire = RecordingWire::default();
    let mut queue = server_queue();

    queue.send_value(&mut wire, 1, 10, &Value::Double(1.0), SendMode::All);
    queue.send_value(&mut wire, 1, 20, &Value::Double(2.0), SendMode::All);

    queue.send_outgoing(&mut wire, 5, true);

    assert_eq!(wire.binary_writes.len(), 2);
    let (id0, _, value0) = decode_binary(&wire.binary_writes[0], 0);
    assert_eq!(id0, 1);
    assert_eq!(double(&value0), 1.0);
    let (id1, _, value1) = decode_binary(&wire.binary_writes[1], 0);
    assert_eq!(id1, 1);
    assert_eq!(double(&value1), 2.0);
}

// upstream: "NetworkOutgoingQueueTest SendAllValuesCoalesceAfterBackpressureLimit"
#[test]
fn send_all_values_coalesce_after_backpressure_limit() {
    let mut wire = RecordingWire::default();
    let mut queue = server_queue();

    let x = Value::String("x".repeat(1024 * 1024));
    let y = Value::String("y".repeat(1024 * 1024));
    queue.send_value(&mut wire, 1, 10, &x, SendMode::All);
    queue.send_value(&mut wire, 1, 20, &y, SendMode::All);
    queue.send_value(&mut wire, 1, 30, &Value::Double(3.0), SendMode::All);

    queue.send_outgoing(&mut wire, 5, true);

    assert_eq!(wire.binary_writes.len(), 1);
    let (id, _, value) = decode_binary(&wire.binary_writes[0], 0);
    assert_eq!(id, 1);
    assert_eq!(double(&value), 3.0);
}

// upstream: "NetworkOutgoingQueueTest PartialWriteRetainsUnsentValueForReplacement"
#[test]
fn partial_write_retains_unsent_value_for_replacement() {
    let mut wire = RecordingWire {
        binary_write_returns: [0, 1].into(),
        ..Default::default()
    };
    let mut queue = server_queue();

    queue.send_value(&mut wire, 1, 10, &Value::Double(1.0), SendMode::Normal);
    queue.send_value(&mut wire, 2, 20, &Value::Double(2.0), SendMode::Normal);

    queue.send_outgoing(&mut wire, 5, true);
    assert_eq!(wire.binary_writes.len(), 2);

    queue.send_value(&mut wire, 2, 30_000, &Value::Double(3.0), SendMode::Normal);
    queue.send_outgoing(&mut wire, 10, true);

    assert_eq!(wire.binary_writes.len(), 3);
    let (id, time, value) = decode_binary(&wire.binary_writes[2], 0);
    assert_eq!(id, 2);
    assert_eq!(time, 30_000);
    assert_eq!(double(&value), 3.0);
}

// upstream: "NetworkOutgoingQueueTest FlushBackpressureRetainsLastUnsentValue"
#[test]
fn flush_backpressure_retains_last_unsent_value() {
    let mut wire = RecordingWire {
        flush_returns: [1].into(),
        ..Default::default()
    };
    let mut queue = server_queue();

    queue.send_value(&mut wire, 1, 10, &Value::Double(1.0), SendMode::Normal);
    queue.send_value(&mut wire, 2, 20, &Value::Double(2.0), SendMode::Normal);

    queue.send_outgoing(&mut wire, 5, true);
    assert_eq!(wire.binary_writes.len(), 2);

    queue.send_outgoing(&mut wire, 10, true);

    assert_eq!(wire.binary_writes.len(), 3);
    let (id, _, value) = decode_binary(&wire.binary_writes[2], 0);
    assert_eq!(id, 2);
    assert_eq!(double(&value), 2.0);
}

// upstream: "NetworkOutgoingQueueTest ClientImmediateValueAppliesTimeOffset"
#[test]
fn client_immediate_value_applies_time_offset() {
    let mut wire = RecordingWire::default();
    let mut queue = client_queue();
    queue.set_time_offset(5_000);

    queue.send_value(
        &mut wire,
        3,
        10_000,
        &Value::Double(1.0),
        SendMode::Immediate,
    );

    assert_eq!(wire.binary_sends.len(), 1);
    let (id, local_time, value) = decode_binary(&wire.binary_sends[0], -5_000);
    let (_, server_time, _) = decode_binary(&wire.binary_sends[0], 0);
    assert_eq!(id, 3);
    assert_eq!(local_time, 10_000);
    assert_eq!(server_time, 15_000);
    assert_eq!(double(&value), 1.0);
}

// upstream: "NetworkOutgoingQueueTest LocalQueueSendsImmediately"
#[test]
fn local_queue_sends_immediately() {
    let mut wire = RecordingWire::default();
    let mut queue: OutgoingQueue<ClientMessage> = OutgoingQueue::new(true);

    queue.send_message(&mut wire, 5, publish(5, "local"));
    queue.send_value(&mut wire, 5, 10, &Value::Double(1.0), SendMode::Normal);
    queue.send_outgoing(&mut wire, 5, true);

    assert_eq!(wire.text_sends, vec![publish_json(5, "local")]);
    assert_eq!(wire.binary_sends.len(), 1);
    assert!(wire.text_writes.is_empty());
    assert!(wire.binary_writes.is_empty());
}
