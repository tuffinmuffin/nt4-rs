//! Print each result a PhotonVision camera publishes.
//!
//! ```text
//! cargo run -p nt4-photon --example photon_watch -- 10.32.0.11 Front
//! cargo run -p nt4-photon --example photon_watch -- 3200 Front 20
//! ```
//!
//! Arguments: the NT server (a host, or a team number for the robot), the
//! camera name, and how many results to print (all of them by default).

use std::time::Duration;

use nt4::{Client, ClientConfig};
use nt4_photon::{PhotonPipelineResult, check_version, result_topic};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let server = args.next().unwrap_or_else(|| "127.0.0.1".into());
    let camera = args.next().unwrap_or_else(|| "photonvision".into());
    let count: Option<usize> = args.next().and_then(|n| n.parse().ok());

    let config = match server.parse::<u16>() {
        Ok(team) => ClientConfig::team("nt4-photon-watch", team),
        Err(_) => ClientConfig::server("nt4-photon-watch", server, nt4::proto::ROBOT_CODE_PORT),
    };
    let client = Client::start(config);
    let topic = result_topic(&camera);
    let mut results = client.subscribe::<PhotonPipelineResult>(&topic, Default::default());

    // Say so if the camera runs a PhotonVision version with another format.
    tokio::time::sleep(Duration::from_secs(2)).await;
    match client.topic(&topic) {
        None => eprintln!("{topic} isn't published (yet); waiting"),
        Some(t) => match check_version(&t.properties) {
            Ok(()) => eprintln!("{topic}: {} (our format)", t.ty),
            Err(theirs) => {
                eprintln!(
                    "{topic} has message_uuid {theirs:?}; this build reads only {}",
                    t.ty
                );
            }
        },
    }

    let mut seen = 0;
    while let Ok(value) = results.changed().await {
        let Some(result) = value else {
            eprintln!("a value that didn't decode (another format?)");
            continue;
        };
        let m = &result.metadata;
        let latency_ms = (m.publish_timestamp_ns - m.capture_timestamp_ns) as f64 / 1e6;
        print!(
            "#{} {} targets, {latency_ms:.1} ms",
            m.sequence_id,
            result.targets.len()
        );
        for t in &result.targets {
            let p = &t.best_camera_to_target.translation;
            print!(
                " | tag {} yaw {:.2} pitch {:.2} at ({:.2}, {:.2}, {:.2}) m, ambiguity {:.3}",
                t.fiducial_id, t.yaw, t.pitch, p.x, p.y, p.z, t.pose_ambiguity
            );
        }
        if let Some(mt) = &result.multitag_result {
            let p = &mt.estimated_pose.best.translation;
            print!(
                " | multi-tag {:?} camera at ({:.2}, {:.2}, {:.2})",
                mt.fiducial_ids_used, p.x, p.y, p.z
            );
        }
        println!();
        seen += 1;
        if count.is_some_and(|n| seen >= n) {
            break;
        }
    }
}
