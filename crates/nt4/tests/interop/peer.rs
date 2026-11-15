//! The pyntcore peer process, shared by the interop tests.
//!
//! Written by Claude. Starts `tests/interop/ntcore_peer.py` with the Python
//! in `NT4_INTEROP_PYTHON` and sends it one JSON command per line.

#![allow(dead_code)] // each test file uses part of it

use std::process::Stdio;
use std::time::Duration;

use nt4::{Client, ClientConfig};
use serde_json::{Value as Json, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

pub struct Peer {
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    /// The port its ntcore server listens on, if it runs one.
    pub port4: u16,
}

impl Peer {
    /// Start the peer process, or `None` if `NT4_INTEROP_PYTHON` isn't set.
    pub async fn spawn() -> Option<Peer> {
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
        Some(Peer {
            _child: child,
            stdin,
            stdout,
            port4: free_port(),
        })
    }

    /// Send a command and return its reply, which must be `ok`.
    pub async fn cmd(&mut self, cmd: Json) -> Json {
        let reply = self.try_cmd(cmd.clone()).await;
        assert_eq!(reply["ok"], true, "{cmd} -> {reply}");
        reply
    }

    /// Send a command and return its reply, `ok` or not.
    pub async fn try_cmd(&mut self, cmd: Json) -> Json {
        let mut line = cmd.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await.unwrap();
        let reply = self.stdout.next_line().await.unwrap().expect("peer reply");
        serde_json::from_str(&reply).unwrap()
    }

    pub async fn start_server(&mut self) {
        let port = self.port4;
        self.cmd(json!({"cmd": "start_server", "port": port})).await;
    }

    pub async fn stop_server(&mut self) {
        self.cmd(json!({"cmd": "stop_server"})).await;
    }

    /// Make the peer an ntcore client of the server on `host:port`.
    pub async fn start_client(&mut self, name: &str, port: u16) {
        self.cmd(json!({"cmd": "start_client", "name": name, "port": port}))
            .await;
    }

    /// Whether the peer's ntcore client is connected.
    pub async fn connected(&mut self) -> bool {
        self.cmd(json!({"cmd": "connected"})).await["connected"] == true
    }

    /// Our client, connected to the peer's ntcore server.
    pub fn client(&self, name: &str) -> Client {
        let mut config = ClientConfig::server(name, "127.0.0.1", self.port4);
        config.reconnect_delay = Duration::from_millis(100);
        Client::start(config)
    }
}

pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Poll `f` every 10 ms until it returns `Some`, for up to 5 s.
pub async fn eventually<T>(mut f: impl FnMut() -> Option<T>) -> T {
    for _ in 0..500 {
        if let Some(v) = f() {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out");
}
