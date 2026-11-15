//! Instruction-count benchmarks for nt4's hot paths, using gungraun
//! (Callgrind). Counts are the same on every run of the same code, so CI can
//! compare a pull request with `main` on a shared runner without timing noise.
//!
//! Run locally (Linux, needs valgrind and `cargo install gungraun-runner
//! --version 0.20.0`):
//!
//! ```text
//! cargo bench -p nt4-perf
//! ```
//!
//! Each benchmark is one 20 ms loop of the simulated robot in `bench/`. The
//! setup (building inputs, connecting the client) is not counted.
//! Benchmarks written by Claude.

// The gungraun macros generate undocumented items.
#![allow(missing_docs)]

use std::hint::black_box;

use gungraun::prelude::*;
use nt4_core::{ClientCore, ClientId, PubId, ServerCore};
use nt4_proto::{
    BinaryMessage, BinaryReader, ClientMessage, Value, decode_server_text, encode_binary,
    encode_text_frame,
};

// Decode one loop's 150 values from a binary frame.
#[library_benchmark]
#[bench::periodic(nt4_perf::periodic_frame())]
fn decode_binary(frame: Vec<u8>) -> Vec<BinaryMessage> {
    BinaryReader::new(black_box(&frame))
        .collect::<Result<_, _>>()
        .expect("valid frame")
}

// Encode one loop's 150 values into a binary frame.
#[library_benchmark]
#[bench::periodic(nt4_perf::periodic_batch())]
fn encode_binary_batch(batch: Vec<BinaryMessage>) -> Vec<u8> {
    let mut out = Vec::with_capacity(4096);
    for m in black_box(&batch) {
        encode_binary(&mut out, m);
    }
    out
}

// Decode the server's announce frame for 200 topics.
#[library_benchmark]
#[bench::announce_200(nt4_perf::announce_frame())]
fn decode_announce(text: String) -> usize {
    decode_server_text(black_box(&text)).messages.len()
}

// Encode the client's publish frame for 200 topics.
#[library_benchmark]
#[bench::publish_200(nt4_perf::publish_messages())]
fn encode_publish(msgs: Vec<ClientMessage>) -> String {
    encode_text_frame(black_box(&msgs))
}

fn receive_setup() -> (ClientCore, Vec<u8>) {
    (nt4_perf::receiving_client(), nt4_perf::periodic_frame())
}

// A subscribed client takes one loop's values and hands out the events.
#[library_benchmark]
#[bench::periodic(setup = receive_setup)]
fn client_receive((mut core, frame): (ClientCore, Vec<u8>)) -> usize {
    core.handle_binary(20_000, black_box(&frame));
    std::iter::from_fn(|| core.poll_event()).count()
}

// A robot client sets one loop's 150 values and builds the frames to send.
#[library_benchmark]
#[bench::periodic(setup = nt4_perf::publishing_client)]
fn client_publish((mut core, values): (ClientCore, Vec<(PubId, Value)>)) -> usize {
    for (id, value) in values {
        core.set_value(id, value, 20_000).expect("published");
    }
    core.send_outgoing(20_000, true);
    std::iter::from_fn(|| core.poll_transmit()).count()
}

fn relay_setup() -> ((ServerCore, ClientId, ClientId), Vec<u8>) {
    (nt4_perf::relaying_server(), nt4_perf::periodic_frame())
}

// The server takes a robot's loop of 150 values and builds the frames for a
// dashboard subscribed to all of them.
#[library_benchmark]
#[bench::periodic(setup = relay_setup)]
fn server_relay(
    ((mut server, robot, dash), frame): ((ServerCore, ClientId, ClientId), Vec<u8>),
) -> usize {
    server.handle_binary(robot, 20_000, black_box(&frame));
    server.send_outgoing(dash, 20_000, true);
    std::iter::from_fn(|| server.poll_transmit(dash)).count()
}

library_benchmark_group!(
    name = proto,
    benchmarks = [
        decode_binary,
        encode_binary_batch,
        decode_announce,
        encode_publish
    ]
);

library_benchmark_group!(
    name = client_core,
    benchmarks = [client_receive, client_publish]
);

library_benchmark_group!(name = server_core, benchmarks = [server_relay]);

main!(library_benchmark_groups = proto, client_core, server_core);
