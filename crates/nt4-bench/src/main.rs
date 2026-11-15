//! Benchmark client using nt4, for comparison with ntcore.
//!
//! ```text
//! cargo run --release -p nt4-bench -- --port 5810 --seconds 20
//! ```
//!
//! Subscribes to `/robot/` with `all` and a 5 ms period, waits until every
//! topic has a value (the snapshot), then for `--seconds` counts updates,
//! lost updates (gaps in the sequence numbers bench/robot_sim.py encodes),
//! out-of-order updates and latency (our estimate of server time on arrival
//! minus the value's server timestamp). Prints one JSON line, with the same
//! fields as bench/ntcore_client.py. See bench/README.md.
//!
//! `--mode robot` is the other side: it connects to a server and publishes
//! the same topics as bench/robot_sim.py (config, every 20 ms, and a tight
//! loop), with every publisher sending all values every 20 ms. Run
//! `ntcore_client.py` against the same server to check what arrives.
//!
//! `--mode native` reads no events: the client caches every value and the
//! bench checks the cache at the start and end (like `ntcore_client.py
//! --mode native`), to measure the library alone.
//!
//! Written by Claude.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use nt4::{
    Client, ClientConfig, Event, GenericPublisher, NtType, Properties, PublishOptions,
    SubscriptionOptions, Value,
};
use serde_json::json;

fn arg(name: &str, default: &str) -> String {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| default.to_owned())
}

/// The sequence number a robot_sim value carries, if any.
fn seq_of(v: &Value) -> Option<i64> {
    match v {
        Value::Double(d) => Some(*d as i64),
        Value::Int(i) => Some(*i),
        Value::DoubleArray(a) => a.first().map(|d| *d as i64),
        Value::Raw(b) if b.len() >= 8 => Some(i64::from_le_bytes(b[..8].try_into().ok()?)),
        _ => None,
    }
}

fn pct(sorted: &[i64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let i = ((p / 100.0) * sorted.len() as f64) as usize;
    Some(sorted[i.min(sorted.len() - 1)] as f64 / 1000.0)
}

fn config_ok(values: &HashMap<String, Value>) -> bool {
    (0..20).all(|i| {
        values.get(&format!("/robot/config/str{i}")) == Some(&Value::String(format!("value-{i}")))
    }) && (0..10).all(|i| {
        values.get(&format!("/robot/config/num{i}")) == Some(&Value::Double(f64::from(i) * 1.5))
    })
}

fn cached_values(client: &Client) -> HashMap<String, Value> {
    client
        .topics()
        .into_iter()
        .filter_map(|t| Some((t.name.to_string(), t.value?.value)))
        .collect()
}

async fn run_native(client: Client, t0: Instant, seconds: f64, expected_topics: usize) {
    let _sub = client.subscribe_multiple(
        vec!["/robot/".into()],
        SubscriptionOptions {
            prefix: true,
            all: true,
            periodic: Some(0.005),
            ..Default::default()
        },
    );
    let hard_stop = t0 + Duration::from_secs_f64(seconds + 30.0);
    let mut synced_ms = None;
    let mut snapshot_ms = None;
    while Instant::now() < hard_stop {
        if synced_ms.is_none() && client.server_time_offset().is_some() {
            synced_ms = Some(t0.elapsed().as_secs_f64() * 1000.0);
        }
        if cached_values(&client).len() >= expected_topics {
            snapshot_ms = Some(t0.elapsed().as_secs_f64() * 1000.0);
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let start = Instant::now();
    if snapshot_ms.is_some() {
        tokio::time::sleep(Duration::from_secs_f64(seconds)).await;
    }
    let values = cached_values(&client);
    println!(
        "{}",
        json!({
            "client": "nt4-native",
            "synced_ms": synced_ms,
            "snapshot_ms": snapshot_ms,
            "topics_seen": values.len(),
            "config_ok": config_ok(&values),
            "seconds": start.elapsed().as_secs_f64(),
            "received": null, "rate": null, "seq_values": null,
            "lost": null, "out_of_order": null,
            "latency_ms": {"p50": null, "p95": null, "p99": null, "max": null},
        })
    );
}

/// The value robot_sim.py sets for a topic kind at sequence `seq`.
fn robot_value(ty: NtType, seq: i64) -> Value {
    match ty {
        NtType::Double => Value::Double(seq as f64),
        NtType::Int => Value::Int(seq),
        NtType::Boolean => Value::Boolean(seq % 2 == 0),
        NtType::DoubleArray => {
            let mut a = vec![0.5; 8];
            a[0] = seq as f64;
            Value::DoubleArray(a)
        }
        _ => {
            let mut b = seq.to_le_bytes().to_vec();
            b.resize(48, 0);
            Value::Raw(b)
        }
    }
}

async fn run_robot(port: u16, seconds: f64, scale: usize) {
    let client = Client::start(ClientConfig::server("nt4-robot", "127.0.0.1", port));
    assert!(
        client.wait_for_sync(Duration::from_secs(10)).await,
        "no server"
    );
    let options = PublishOptions {
        periodic: Some(0.02),
        send_all: true,
    };
    let publish = |name: String, ty: NtType| -> (NtType, GenericPublisher) {
        (
            ty.clone(),
            client.publish_generic(&name, ty, Properties::new(), options.clone()),
        )
    };
    let raw = || NtType::from("struct:Pose3d");

    let mut config = Vec::new();
    for i in 0..20 {
        let (_, p) = publish(format!("/robot/config/str{i}"), NtType::String);
        p.set(Value::String(format!("value-{i}"))).unwrap();
        config.push(p);
    }
    for i in 0..10 {
        let (_, p) = publish(format!("/robot/config/num{i}"), NtType::Double);
        p.set(Value::Double(f64::from(i) * 1.5)).unwrap();
        config.push(p);
    }
    for i in 0..5 {
        let (_, p) = publish(format!("/robot/config/flag{i}"), NtType::Boolean);
        p.set(Value::Boolean(i % 2 == 0)).unwrap();
        config.push(p);
    }
    for i in 0..5 {
        let (_, p) = publish(format!("/robot/config/arr{i}"), NtType::DoubleArray);
        p.set(Value::DoubleArray((0..10).map(f64::from).collect()))
            .unwrap();
        config.push(p);
    }

    let mut periodic = Vec::new();
    for (prefix, n, ty) in [
        ("double", 100, NtType::Double),
        ("int", 20, NtType::Int),
        ("bool", 10, NtType::Boolean),
        ("darr", 10, NtType::DoubleArray),
        ("pose", 10, raw()),
    ] {
        for i in 0..n * scale {
            periodic.push(publish(format!("/robot/periodic/{prefix}{i}"), ty.clone()));
        }
    }
    let mut looped = Vec::new();
    for (prefix, ty) in [("int", NtType::Int), ("double", NtType::Double)] {
        for i in 0..5 * scale {
            looped.push(publish(format!("/robot/loop/{prefix}{i}"), ty.clone()));
        }
    }
    let topics = config.len() + periodic.len() + looped.len();
    println!("{}", json!({"ready": true, "topics": topics}));

    // A busy thread, like vision or logging code.
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let loop_thread = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut seq = 0;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                seq += 1;
                for (ty, p) in &looped {
                    p.set(robot_value(ty.clone(), seq)).unwrap();
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            seq as usize * looped.len()
        })
    };

    let start = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    let (mut seq, mut late) = (0i64, 0u64);
    while start.elapsed().as_secs_f64() < seconds {
        let t = tick.tick().await;
        if t.elapsed() > Duration::from_millis(5) {
            late += 1;
        }
        seq += 1;
        for (ty, p) in &periodic {
            p.set(robot_value(ty.clone(), seq)).unwrap();
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let loop_sets = loop_thread.join().unwrap();
    let elapsed = start.elapsed().as_secs_f64();
    println!(
        "{}",
        json!({
            "done": true,
            "seconds": elapsed,
            "periodic_sets": seq as usize * periodic.len(),
            "loop_sets": loop_sets,
            "periodic_rate": (seq as usize * periodic.len()) as f64 / elapsed,
            "loop_rate": loop_sets as f64 / elapsed,
            "late_cycles": late,
        })
    );
    drop(config);
}

#[tokio::main]
async fn main() {
    let port: u16 = arg("--port", "5810").parse().expect("--port");
    let seconds: f64 = arg("--seconds", "20").parse().expect("--seconds");
    let expected_topics: usize = arg("--topics", "200").parse().expect("--topics");
    let mode = arg("--mode", "events");
    if mode == "robot" {
        let scale: usize = arg("--scale", "1").parse().expect("--scale");
        return run_robot(port, seconds, scale).await;
    }
    let native = mode == "native";

    let t0 = Instant::now();
    let mut config = ClientConfig::server("nt4-bench", "127.0.0.1", port);
    config.reconnect_delay = Duration::from_millis(100);
    let client = Client::start(config);
    if native {
        return run_native(client, t0, seconds, expected_topics).await;
    }
    let mut events = client.events();
    let _sub = client.subscribe_multiple(
        vec!["/robot/".into()],
        SubscriptionOptions {
            prefix: true,
            all: true,
            periodic: Some(0.005),
            ..Default::default()
        },
    );

    let mut values: HashMap<String, Value> = HashMap::new();
    let mut last_seq: HashMap<String, i64> = HashMap::new();
    let mut latencies: Vec<i64> = Vec::new();
    let (mut received, mut seq_values, mut lost, mut out_of_order) = (0u64, 0u64, 0i64, 0u64);
    let mut synced_ms = None;
    let mut snapshot_ms = None;
    let mut measure_start: Option<Instant> = None;
    let hard_stop = t0 + Duration::from_secs_f64(seconds + 30.0);

    loop {
        let deadline = match measure_start {
            Some(s) => s + Duration::from_secs_f64(seconds),
            None => hard_stop,
        };
        let event = match tokio::time::timeout_at(deadline.into(), events.recv()).await {
            Err(_) => break,
            Ok(None) => break,
            Ok(Some(e)) => e,
        };
        match event {
            Event::TimeSync(t) if t.valid && synced_ms.is_none() => {
                synced_ms = Some(t0.elapsed().as_secs_f64() * 1000.0);
            }
            Event::Value { name, value, .. } => {
                let recv_server = client.server_time();
                values.insert(name.to_string(), value.value.clone());
                if snapshot_ms.is_none() && values.len() >= expected_topics {
                    snapshot_ms = Some(t0.elapsed().as_secs_f64() * 1000.0);
                    measure_start = Some(Instant::now());
                    last_seq.clear();
                    continue;
                }
                if measure_start.is_none() || name.starts_with("/robot/config/") {
                    continue;
                }
                received += 1;
                let Some(seq) = seq_of(&value.value) else {
                    continue;
                };
                seq_values += 1;
                if let Some(&prev) = last_seq.get(&*name) {
                    if seq > prev + 1 {
                        lost += seq - prev - 1;
                    } else if seq <= prev {
                        out_of_order += 1;
                    }
                    last_seq.insert(name.to_string(), seq.max(prev));
                } else {
                    last_seq.insert(name.to_string(), seq);
                }
                if let Some(now) = recv_server {
                    latencies.push(now - value.server_time_us);
                }
            }
            _ => {}
        }
    }

    let elapsed = measure_start.map_or(0.0, |s| s.elapsed().as_secs_f64());
    latencies.sort_unstable();
    let config_ok = config_ok(&values);
    println!(
        "{}",
        json!({
            "client": "nt4",
            "synced_ms": synced_ms,
            "snapshot_ms": snapshot_ms,
            "topics_seen": values.len(),
            "config_ok": config_ok,
            "seconds": elapsed,
            "received": received,
            "rate": if elapsed > 0.0 { received as f64 / elapsed } else { 0.0 },
            "seq_values": seq_values,
            "lost": lost,
            "out_of_order": out_of_order,
            "events_queued": events.len(),
            "latency_ms": {
                "p50": pct(&latencies, 50.0),
                "p95": pct(&latencies, 95.0),
                "p99": pct(&latencies, 99.0),
                "max": pct(&latencies, 100.0),
            },
        })
    );
}
