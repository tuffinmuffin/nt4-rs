//! Interop tests: our client against a real ntcore server (pyntcore).
//!
//! Written by Claude. Each test starts `tests/interop/ntcore_peer.py` with
//! the Python in `NT4_INTEROP_PYTHON` (which must have `pyntcore`
//! installed). Without that variable the tests print a note and pass, so
//! `cargo test` works without Python; CI sets it.
//!
//! ```text
//! python -m venv .venv && .venv/bin/pip install pyntcore==2026.2.2
//! NT4_INTEROP_PYTHON=.venv/bin/python cargo test -p nt4 --test interop
//! ```

use std::process::Stdio;
use std::time::Duration;

use nt4::{Client, ClientConfig, Event, PublishOptions, SubscriptionOptions};
use serde_json::{Value as Json, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

struct Peer {
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    port4: u16,
}

impl Peer {
    /// Start the peer, or `None` if `NT4_INTEROP_PYTHON` isn't set.
    async fn start() -> Option<Peer> {
        let Ok(python) = std::env::var("NT4_INTEROP_PYTHON") else {
            eprintln!("NT4_INTEROP_PYTHON not set; skipping interop test");
            return None;
        };
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/interop/ntcore_peer.py");
        let mut child = Command::new(python)
            .args(["-I", "-u", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("start python");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut peer = Peer {
            _child: child,
            stdin,
            stdout,
            port4: free_port(),
        };
        peer.start_server().await;
        Some(peer)
    }

    async fn cmd(&mut self, cmd: Json) -> Json {
        let mut line = cmd.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await.unwrap();
        let reply = self.stdout.next_line().await.unwrap().expect("peer reply");
        let reply: Json = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["ok"], true, "{cmd} -> {reply}");
        reply
    }

    async fn start_server(&mut self) {
        let port3 = free_port();
        let port4 = self.port4;
        self.cmd(json!({"cmd": "start_server", "port4": port4, "port3": port3}))
            .await;
    }

    async fn stop_server(&mut self) {
        self.cmd(json!({"cmd": "stop_server"})).await;
    }

    fn client(&self, name: &str) -> Client {
        let mut config = ClientConfig::server(name, "127.0.0.1", self.port4);
        config.reconnect_delay = Duration::from_millis(100);
        Client::start(config)
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Poll `f` every 10 ms until it returns `Some`, for up to 5 s.
async fn eventually<T>(mut f: impl FnMut() -> Option<T>) -> T {
    for _ in 0..500 {
        if let Some(v) = f() {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out");
}

#[tokio::test(flavor = "multi_thread")]
async fn connects_and_syncs_time() {
    let Some(mut peer) = Peer::start().await else {
        return;
    };
    let client = peer.client("interop-sync");
    assert!(client.wait_for_sync(Duration::from_secs(5)).await);
    assert!(client.is_connected());

    // ntcore lists us by our name plus its own `@<n>` suffix.
    let names = peer.cmd(json!({"cmd": "connections"})).await;
    let names = names["names"].as_array().unwrap();
    assert_eq!(names.len(), 1);
    assert!(
        names[0].as_str().unwrap().starts_with("interop-sync@"),
        "{names:?}"
    );

    // A value the server sets now is stamped close to our idea of server time.
    let sub = client.subscribe::<f64>("/sync/x", SubscriptionOptions::default());
    peer.cmd(json!({"cmd": "publish", "name": "/sync/x", "type": "double", "value": 1.0}))
        .await;
    let (_, stamped) = eventually(|| sub.get_timestamped()).await;
    let ours = client.server_time().unwrap();
    let diff = ours - stamped.server_time_us;
    assert!((0..500_000).contains(&diff), "clock difference {diff} us");
}

#[tokio::test(flavor = "multi_thread")]
async fn reads_server_values() {
    let Some(mut peer) = Peer::start().await else {
        return;
    };
    peer.cmd(json!({"cmd": "publish", "name": "/srv/d", "type": "double", "value": 1.5}))
        .await;
    peer.cmd(json!({"cmd": "publish", "name": "/srv/s", "type": "string", "value": "hi"}))
        .await;
    peer.cmd(json!({"cmd": "publish", "name": "/srv/da", "type": "double[]", "value": [1.0, 2.0]}))
        .await;

    let client = peer.client("interop-read");
    let mut d = client.subscribe::<f64>("/srv/d", SubscriptionOptions::default());
    let s = client.subscribe::<String>("/srv/s", SubscriptionOptions::default());
    let da = client.subscribe::<Vec<f64>>("/srv/da", SubscriptionOptions::default());
    assert_eq!(eventually(|| d.get()).await, 1.5);
    assert_eq!(eventually(|| s.get()).await, "hi");
    assert_eq!(eventually(|| da.get()).await, vec![1.0, 2.0]);

    // Later changes arrive too, and wake `changed`.
    peer.cmd(json!({"cmd": "publish", "name": "/srv/d", "type": "double", "value": 2.5}))
        .await;
    let next = tokio::time::timeout(Duration::from_secs(5), d.changed()).await;
    assert_eq!(next, Ok(Ok(Some(2.5))));
}

#[tokio::test(flavor = "multi_thread")]
async fn prefix_subscription_sees_announcements() {
    let Some(mut peer) = Peer::start().await else {
        return;
    };
    let client = peer.client("interop-prefix");
    let mut events = client.events();
    let _sub = client.subscribe_multiple(
        vec!["/tree/".into()],
        SubscriptionOptions {
            prefix: true,
            ..Default::default()
        },
    );
    assert!(client.wait_for_sync(Duration::from_secs(5)).await);
    peer.cmd(json!({"cmd": "publish", "name": "/tree/a", "type": "int", "value": 7}))
        .await;
    peer.cmd(json!({"cmd": "publish", "name": "/other", "type": "int", "value": 8}))
        .await;

    let got = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(Event::Value { name, value, .. }) = events.recv().await {
                return (name, value.value);
            }
        }
    })
    .await
    .expect("value event");
    assert_eq!(&*got.0, "/tree/a");
    assert_eq!(got.1, nt4::Value::Int(7));
}

#[tokio::test(flavor = "multi_thread")]
async fn server_reads_every_type() {
    let Some(mut peer) = Peer::start().await else {
        return;
    };
    let client = peer.client("interop-types");
    let o = PublishOptions::default;
    let b = client.publish::<bool>("/t/boolean", o());
    let d = client.publish::<f64>("/t/double", o());
    let i = client.publish::<i64>("/t/int", o());
    let f = client.publish::<f32>("/t/float", o());
    let s = client.publish::<String>("/t/string", o());
    let r = client.publish::<Vec<u8>>("/t/raw", o());
    let ba = client.publish::<Vec<bool>>("/t/boolean[]", o());
    let da = client.publish::<Vec<f64>>("/t/double[]", o());
    let ia = client.publish::<Vec<i64>>("/t/int[]", o());
    let fa = client.publish::<Vec<f32>>("/t/float[]", o());
    let sa = client.publish::<Vec<String>>("/t/string[]", o());
    b.set(true);
    d.set(1.25);
    i.set(-42);
    f.set(0.5);
    s.set("text".into());
    r.set(vec![1, 2, 255]);
    ba.set(vec![true, false]);
    da.set(vec![0.5, -1.0]);
    ia.set(vec![1, i64::MAX]);
    fa.set(vec![0.25]);
    sa.set(vec!["a".into(), "b".into()]);

    let expected = [
        ("boolean", json!(true)),
        ("double", json!(1.25)),
        ("int", json!(-42)),
        ("float", json!(0.5)),
        ("string", json!("text")),
        ("raw", json!([1, 2, 255])),
        ("boolean[]", json!([true, false])),
        ("double[]", json!([0.5, -1.0])),
        ("int[]", json!([1, i64::MAX])),
        ("float[]", json!([0.25])),
        ("string[]", json!(["a", "b"])),
    ];
    for (ty, value) in expected {
        let got = peer
            .cmd(json!({"cmd": "get", "name": format!("/t/{ty}"), "timeout": 5}))
            .await;
        assert_eq!(got["type"], ty);
        assert_eq!(got["value"], value, "{ty}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn republishes_after_server_restart() {
    let Some(mut peer) = Peer::start().await else {
        return;
    };
    let client = peer.client("interop-reconnect");
    let x = client.publish::<f64>("/re/x", PublishOptions::default());
    let echo = client.subscribe::<f64>("/re/echo", SubscriptionOptions::default());
    x.set(1.0);
    let got = peer
        .cmd(json!({"cmd": "get", "name": "/re/x", "timeout": 5}))
        .await;
    assert_eq!(got["value"], 1.0);

    peer.stop_server().await;
    eventually(|| (!client.is_connected()).then_some(())).await;
    // Set while disconnected: kept and sent after the reconnect.
    x.set(2.0);
    peer.start_server().await;

    let got = peer
        .cmd(json!({"cmd": "get", "name": "/re/x", "timeout": 5, "not": 1.0}))
        .await;
    assert_eq!(got["value"], 2.0);

    // The subscription came back too.
    peer.cmd(json!({"cmd": "publish", "name": "/re/echo", "type": "double", "value": 3.0}))
        .await;
    assert_eq!(eventually(|| echo.get()).await, 3.0);
}
