//! Fixed inputs for the instruction-count benchmarks in `benches/hot_paths.rs`.
//!
//! The inputs copy one 20 ms loop of the simulated robot in `bench/` (200
//! topics: 40 one-time values, 150 periodic values, 10 busy-loop values), so
//! the numbers track what a real robot connection costs. Everything here is
//! deterministic: the same inputs on every run, so instruction counts only
//! change when the code does.
//!
//! ```
//! let batch = nt4_perf::periodic_batch();
//! assert_eq!(batch.len(), 150);
//! ```

use nt4_core::{ClientCore, ClientId, ProtocolVersion, PubId, PublishOptions, ServerCore};
use nt4_proto::{
    BinaryMessage, ClientMessage, NtType, Properties, ServerMessage, SubscriptionOptions, Value,
    encode_binary, encode_text_frame,
};

/// Number of topics the server announces, as in `bench/robot_sim.py`.
pub const TOPICS: usize = 200;

/// Server time used for every value, in microseconds.
const SERVER_TIME_US: i64 = 1_000_000;

/// The type and a sample value of topic `i` (0 to 199), in the same mix as
/// `bench/robot_sim.py`.
///
/// ```
/// use nt4_proto::NtType;
/// assert_eq!(nt4_perf::topic(0).1, NtType::Double);
/// ```
pub fn topic(i: usize) -> (String, NtType, Value) {
    let seq = i as f64;
    match i {
        // One-time config values.
        0..40 => match i % 4 {
            0 => (
                format!("/robot/config/d{i}"),
                NtType::Double,
                Value::Double(seq),
            ),
            1 => (
                format!("/robot/config/s{i}"),
                NtType::String,
                Value::String(format!("build-{i}-2026.11.15")),
            ),
            2 => (
                format!("/robot/config/b{i}"),
                NtType::Boolean,
                Value::Boolean(true),
            ),
            _ => (
                format!("/robot/config/a{i}"),
                NtType::DoubleArray,
                Value::DoubleArray(vec![seq; 4]),
            ),
        },
        // 150 periodic values: 100 doubles, 20 ints, 10 booleans,
        // 10 double[8], 10 48-byte structs.
        40..140 => (
            format!("/robot/periodic/d{i}"),
            NtType::Double,
            Value::Double(seq * 0.5),
        ),
        140..160 => (
            format!("/robot/periodic/i{i}"),
            NtType::Int,
            Value::Int(i as i64 * 1000),
        ),
        160..170 => (
            format!("/robot/periodic/b{i}"),
            NtType::Boolean,
            Value::Boolean(i % 2 == 0),
        ),
        170..180 => (
            format!("/robot/periodic/a{i}"),
            NtType::DoubleArray,
            Value::DoubleArray(vec![seq; 8]),
        ),
        180..190 => (
            format!("/robot/periodic/pose{i}"),
            NtType::Raw,
            Value::Raw(vec![i as u8; 48]),
        ),
        // Busy-loop values.
        _ => (
            format!("/robot/loop/i{i}"),
            NtType::Int,
            Value::Int(i as i64),
        ),
    }
}

/// Topic id the server gives topic `i`.
fn server_id(i: usize) -> i32 {
    i as i32 + 1
}

/// One 20 ms loop's values: the 150 periodic topics, as the server sends them.
pub fn periodic_batch() -> Vec<BinaryMessage> {
    (40..190)
        .map(|i| BinaryMessage {
            id: server_id(i),
            timestamp_us: SERVER_TIME_US,
            value: topic(i).2,
        })
        .collect()
}

/// [`periodic_batch`] encoded as one binary frame.
pub fn periodic_frame() -> Vec<u8> {
    let mut out = Vec::new();
    for m in &periodic_batch() {
        encode_binary(&mut out, m);
    }
    out
}

/// The server's `announce` text frame for all [`TOPICS`] topics.
pub fn announce_frame() -> String {
    let msgs: Vec<ServerMessage> = (0..TOPICS)
        .map(|i| {
            let (name, ty, _) = topic(i);
            ServerMessage::Announce {
                name,
                id: server_id(i),
                ty,
                pubuid: None,
                properties: Properties::new(),
            }
        })
        .collect();
    encode_text_frame(&msgs)
}

/// The client's `publish` messages for all [`TOPICS`] topics.
pub fn publish_messages() -> Vec<ClientMessage> {
    (0..TOPICS)
        .map(|i| {
            let (name, ty, _) = topic(i);
            ClientMessage::Publish {
                name,
                pubuid: i as i32 + 1,
                ty,
                properties: Properties::new(),
            }
        })
        .collect()
}

/// Answer the client's first RTT ping so it has a time offset and starts
/// sending. `now_us` is the client's local time.
fn sync_time(core: &mut ClientCore, now_us: i64) {
    let reply = BinaryMessage {
        id: -1,
        timestamp_us: SERVER_TIME_US,
        value: Value::Int(now_us),
    };
    let mut frame = Vec::new();
    encode_binary(&mut frame, &reply);
    core.handle_binary(now_us, &frame);
}

/// Throw away queued frames and events.
fn drain(core: &mut ClientCore) {
    while core.poll_transmit().is_some() {}
    while core.poll_event().is_some() {}
}

/// A dashboard-style client: connected, time synced, subscribed to
/// `/robot/` and with all [`TOPICS`] topics announced. Feed it
/// [`periodic_frame`].
///
/// ```
/// let mut core = nt4_perf::receiving_client();
/// core.handle_binary(0, &nt4_perf::periodic_frame());
/// assert_eq!(std::iter::from_fn(|| core.poll_event()).count(), 150);
/// ```
pub fn receiving_client() -> ClientCore {
    let mut core = ClientCore::new();
    core.subscribe(
        vec!["/robot/".into()],
        SubscriptionOptions {
            prefix: true,
            all: true,
            ..Default::default()
        },
    );
    core.connect(0, ProtocolVersion::V4_1);
    sync_time(&mut core, 0);
    core.handle_text(0, &announce_frame());
    drain(&mut core);
    core
}

/// A robot-style client: connected, time synced, publishing all [`TOPICS`]
/// topics with `send_all`. Returns the client and the publisher ids of the
/// 150 periodic topics, in [`periodic_batch`] order.
pub fn publishing_client() -> (ClientCore, Vec<(PubId, Value)>) {
    let mut core = ClientCore::new();
    let mut periodic = Vec::new();
    for i in 0..TOPICS {
        let (name, ty, value) = topic(i);
        let id = core.publish(
            name,
            ty,
            Properties::new(),
            PublishOptions {
                periodic: Some(0.02),
                send_all: true,
            },
        );
        if (40..190).contains(&i) {
            periodic.push((id, value));
        }
    }
    core.connect(0, ProtocolVersion::V4_1);
    sync_time(&mut core, 0);
    core.send_outgoing(0, true);
    drain(&mut core);
    (core, periodic)
}

/// A server relaying a robot to a dashboard: the robot client publishes all
/// [`TOPICS`] topics (pubuids 1 to 200, so [`periodic_frame`] is also its
/// value frame) and the dashboard subscribes to `/robot/` with `all`.
/// Returns the server, the robot and the dashboard.
///
/// ```
/// let (mut server, robot, dash) = nt4_perf::relaying_server();
/// server.handle_binary(robot, 20_000, &nt4_perf::periodic_frame());
/// server.send_outgoing(dash, 20_000, true);
/// assert!(server.poll_transmit(dash).is_some());
/// ```
pub fn relaying_server() -> (ServerCore, ClientId, ClientId) {
    let mut server = ServerCore::new();
    let (robot, _) = server.add_client("robot", "10.0.0.2:1", false, ProtocolVersion::V4_1);
    let (dash, _) = server.add_client("dash", "10.0.0.5:2", false, ProtocolVersion::V4_1);
    let subscribe = ClientMessage::Subscribe {
        subuid: 1,
        topics: vec!["/robot/".into()],
        options: SubscriptionOptions {
            prefix: true,
            all: true,
            ..Default::default()
        },
    };
    server.handle_text(dash, 0, &encode_text_frame(&[subscribe]));
    server.handle_text(robot, 0, &encode_text_frame(&publish_messages()));
    server.send_all_outgoing(0, true);
    for id in [robot, dash] {
        while server.poll_transmit(id).is_some() {}
    }
    while server.poll_event().is_some() {}
    (server, robot, dash)
}
