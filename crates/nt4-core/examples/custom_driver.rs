//! Advanced: drive `ClientCore` yourself, for writing your own runtime.
//!
//! Most programs should not do this. Use the `nt4` crate's `Client`, which
//! runs this loop in a background task for you (see its `watch` and
//! `sync_loop` examples). This example is for porting the protocol to
//! another runtime, an embedded target, or a browser, and shows the whole
//! contract in one place. It uses a blocking socket only to keep the code
//! short; a real driver would wait on the socket and the tick together.
//!
//! ```text
//! cargo run -p nt4-core --example custom_driver -- 127.0.0.1 /SmartDashboard/
//! ```
//!
//! The loop is: read a frame (with a short timeout) and hand it to the
//! core, call `send_outgoing` on the core's tick, then send whatever
//! `poll_transmit` returns, and print the events.

use std::net::TcpStream;
use std::time::{Duration, Instant};

use nt4_core::{ClientCore, Event, ProtocolVersion, Transmit};
use nt4_proto::SubscriptionOptions;
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

fn main() {
    let mut args = std::env::args().skip(1);
    let host = args.next().unwrap_or_else(|| "127.0.0.1".into());
    let prefix = args.next().unwrap_or_else(|| "/".into());

    let mut core = ClientCore::new();
    core.subscribe(
        vec![prefix],
        SubscriptionOptions {
            prefix: true,
            ..Default::default()
        },
    );

    let start = Instant::now();
    // The core never reads a clock: we give it microseconds since start.
    let now_us = || start.elapsed().as_micros() as i64 + 1;

    let url = nt4_proto::client_url(&host, nt4_proto::ROBOT_CODE_PORT, "custom-driver");
    let mut request = url.into_client_request().expect("valid URL");
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        nt4_proto::SUBPROTOCOL_V4_1.parse().unwrap(),
    );
    let (mut ws, _) = tungstenite::connect(request).expect("connect to the NT server");
    set_read_timeout(&mut ws, Duration::from_millis(5));

    core.connect(now_us(), ProtocolVersion::V4_1);
    let mut next_tick = Instant::now();
    loop {
        // 1. Incoming frames.
        match ws.read() {
            Ok(Message::Text(t)) => core.handle_text(now_us(), &t),
            Ok(Message::Binary(b)) => core.handle_binary(now_us(), &b),
            Ok(Message::Pong(_)) => core.handle_pong(now_us()),
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                eprintln!("read error: {e}");
                break;
            }
        }

        // 2. The periodic tick.
        if Instant::now() >= next_tick {
            core.send_outgoing(now_us(), false);
            next_tick += Duration::from_millis(u64::from(core.tick_period_ms()));
        }

        // 3. Outgoing frames.
        while let Some(t) = core.poll_transmit() {
            let msg = match t {
                Transmit::Text(s) => Message::Text(s.into()),
                Transmit::Binary(b) => Message::Binary(b.into()),
                Transmit::Ping => Message::Ping(Vec::new().into()),
                Transmit::Close { reason } => {
                    eprintln!("closing: {reason}");
                    let _ = ws.close(None);
                    return;
                }
            };
            ws.send(msg).expect("send");
        }

        // 4. What happened.
        while let Some(e) = core.poll_event() {
            match e {
                Event::Announced { name, ty, .. } => println!("+ {name} ({ty})"),
                Event::Value { name, value, .. } => println!("{name} = {:?}", value.value),
                Event::TimeSync(t) if t.valid => println!("offset {} us", t.server_time_offset_us),
                other => println!("{other:?}"),
            }
        }
    }
    core.disconnect("socket closed");
}

fn set_read_timeout(ws: &mut WebSocket<MaybeTlsStream<TcpStream>>, timeout: Duration) {
    if let MaybeTlsStream::Plain(s) = ws.get_mut() {
        s.set_read_timeout(Some(timeout)).expect("set timeout");
    }
}
