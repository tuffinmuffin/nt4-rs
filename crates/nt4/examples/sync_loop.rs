//! Use the client from ordinary (non-async) code, like a robot or game loop.
//!
//! ```text
//! cargo run -p nt4 --example sync_loop -- 127.0.0.1
//! ```
//!
//! This is how ntcore works in Java and C++: a background network task keeps
//! a local copy of every subscribed topic up to date, and your loop reads
//! and writes that copy. `get()` and `set()` never touch the network or
//! block; the background task sends queued values on each publish period
//! (100 ms by default).
//!
//! The only async part is the tokio runtime the background task runs on.
//! Here it is created once at the top of `main` and then left alone.

use std::time::{Duration, Instant};

use nt4::{Client, ClientConfig, PublishOptions};

fn main() {
    let host = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1".into());

    // A runtime for the network task. Keep it alive as long as the client.
    let runtime = tokio::runtime::Runtime::new().expect("start tokio runtime");
    let _enter = runtime.enter();
    let client = Client::start(ClientConfig::server("nt4-sync-loop", host, 5810));

    let speed = client.subscribe::<f64>("/SmartDashboard/speed", Default::default());
    let loop_count = client.publish::<i64>("/example/loopCount", PublishOptions::default());

    // A 50 Hz loop, like a robot's periodic().
    let period = Duration::from_millis(20);
    let start = Instant::now();
    let mut next = start;
    let mut count = 0i64;
    while start.elapsed() < Duration::from_secs(60) {
        // Reads the latest value the network task has received: instant.
        let s = speed.get_or(0.0);
        // Queues the value; the network task sends it.
        loop_count.set(count);

        if count % 50 == 0 {
            println!(
                "connected={} speed={s} server time={:?}",
                client.is_connected(),
                client.server_time()
            );
        }
        count += 1;
        next += period;
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
}
