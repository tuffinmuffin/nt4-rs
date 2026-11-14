//! Print every topic and value under a prefix, like a tiny OutlineViewer.
//!
//! ```text
//! cargo run -p nt4 --example watch -- 127.0.0.1 /SmartDashboard/
//! cargo run -p nt4 --example watch -- 3200          # a team number
//! ```
//!
//! The first argument is a host name or address, or a team number (then
//! the robot's usual addresses are tried). The second is a topic prefix,
//! `/` (everything) by default.

use nt4::{Client, ClientConfig, Event, SubscriptionOptions};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let target = args.next().unwrap_or_else(|| "127.0.0.1".into());
    let prefix = args.next().unwrap_or_else(|| "/".into());

    let config = match target.parse::<u16>() {
        Ok(team) => ClientConfig::team("nt4-watch", team),
        Err(_) => ClientConfig::server("nt4-watch", target, nt4::proto::ROBOT_CODE_PORT),
    };
    let client = Client::start(config);
    let mut events = client.events();

    // Keep the handle alive: dropping it unsubscribes.
    let _sub = client.subscribe_multiple(
        vec![prefix],
        SubscriptionOptions {
            prefix: true,
            ..Default::default()
        },
    );

    loop {
        match events.recv().await {
            Ok(Event::Connected { version }) => println!("connected ({version:?})"),
            Ok(Event::Disconnected { reason }) => println!("disconnected: {reason}"),
            Ok(Event::Announced { name, ty, .. }) => println!("+ {name} ({ty})"),
            Ok(Event::Unannounced { name, .. }) => println!("- {name}"),
            Ok(Event::Value { name, value, .. }) => {
                println!("{name} = {:?} @ {} us", value.value, value.server_time_us)
            }
            Ok(Event::TimeSync(t)) if t.valid => {
                println!(
                    "server clock offset {} us (rtt/2 {} us)",
                    t.server_time_offset_us, t.rtt2_us
                )
            }
            Ok(Event::Warning(w)) => eprintln!("warning: {w}"),
            Ok(_) => {}
            // Printing fell behind; some events were skipped.
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => eprintln!("(skipped {n})"),
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}
