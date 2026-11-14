//! Publish a counter and a sine wave, and read a value back.
//!
//! ```text
//! cargo run -p nt4 --example publish -- 127.0.0.1
//! ```
//!
//! Start a server first, for example robot code in simulation, or with
//! pyntcore: `python -c "import ntcore,time; i=ntcore.NetworkTableInstance.getDefault(); i.startServer(); time.sleep(1e9)"`.
//! Watch the values in AdvantageScope, Elastic, OutlineViewer or the `watch`
//! example. Set `/example/gain` from any of them to scale the wave.

use std::time::Duration;

use nt4::{Client, ClientConfig, PublishOptions};

#[tokio::main]
async fn main() {
    let host = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1".into());
    let client = Client::start(ClientConfig::server("nt4-publish", host, 5810));

    // Values are sent every 20 ms instead of the default 100 ms.
    let fast = PublishOptions {
        periodic: Some(0.02),
        ..Default::default()
    };
    let counter = client.publish::<i64>("/example/counter", PublishOptions::default());
    let wave = client.publish::<f64>("/example/wave", fast);
    let gain_pub = client.publish::<f64>("/example/gain", PublishOptions::default());
    let gain = client.subscribe::<f64>("/example/gain", Default::default());

    // A default (timestamp 0): any other client's value wins over it.
    gain_pub.set_default(1.0);

    if !client.wait_for_sync(Duration::from_secs(10)).await {
        eprintln!("no server yet; values will be sent once one answers");
    }

    let mut ticker = tokio::time::interval(Duration::from_millis(20));
    for i in 0i64.. {
        ticker.tick().await;
        let t = i as f64 * 0.02;
        wave.set(gain.get_or(1.0) * t.sin());
        if i % 50 == 0 {
            counter.set(i / 50);
            println!("t = {t:.0} s, gain = {}", gain.get_or(1.0));
        }
    }
}
