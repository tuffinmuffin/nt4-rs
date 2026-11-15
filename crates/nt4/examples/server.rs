//! An NT4 server with a few topics, for trying dashboards against.
//!
//! ```text
//! cargo run -p nt4 --features server --example server [port]
//! ```
//!
//! Publishes `/server/uptime` (seconds), `/server/sine` and a retained
//! `/server/name`, watches `/dashboard/gain` (set it from a dashboard), and
//! prints the connected clients once a second.

use std::time::Duration;

use nt4::{NtType, Properties, Server, ServerConfig, ServerEvent, Value};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let port = std::env::args()
        .nth(1)
        .map(|p| p.parse().expect("port number"))
        .unwrap_or(nt4::proto::ROBOT_CODE_PORT);
    let server = Server::start(ServerConfig::port(port)).await?;
    println!("listening on {}", server.local_addr());

    let uptime = server.publish::<f64>("/server/uptime");
    let sine = server.publish::<f64>("/server/sine");
    let mut retained = Properties::new();
    retained.insert("retained".into(), true.into());
    let name = server.publish_generic("/server/name", NtType::String, retained);
    name.set(Value::String("nt4 example server".into()))
        .unwrap();
    let gain = server.subscribe::<f64>("/dashboard/gain", Default::default());

    let mut events = server.events();
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    let mut last_print = 0.0;
    loop {
        tick.tick().await;
        let t = server.now_us() as f64 / 1e6;
        uptime.set(t);
        sine.set((t * std::f64::consts::TAU / 4.0).sin());
        while let Some(e) = events.try_recv() {
            if let ServerEvent::Warning(w) = e {
                eprintln!("warning: {w}");
            }
        }
        if t - last_print >= 1.0 {
            last_print = t;
            let clients: Vec<String> = server
                .clients()
                .into_iter()
                .map(|c| format!("{} ({})", c.name, c.address))
                .collect();
            println!("t={t:.0}s gain={:?} clients: {clients:?}", gain.get());
        }
    }
}
