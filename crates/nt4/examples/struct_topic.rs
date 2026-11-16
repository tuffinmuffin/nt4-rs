//! Publish WPILib structs (a `Pose2d` and four swerve module states) that
//! AdvantageScope and Elastic decode.
//!
//! ```text
//! cargo run -p nt4 --example struct_topic -- 127.0.0.1
//! ```
//!
//! A struct topic has type `struct:<Name>` and carries the fields packed
//! little-endian. Publishing a typed struct topic also publishes its schema
//! (and those of the structs inside it) at `/.schema/struct:<Name>`, once,
//! as ntcore does.

use std::time::Duration;

use nt4::wpimath::{Pose2d, Rotation2d, SwerveModuleVelocity, Translation2d};
use nt4::{Client, ClientConfig, PublishOptions};

#[tokio::main]
async fn main() {
    let host = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1".into());
    let client = Client::start(ClientConfig::server("nt4-struct", host, 5810));

    let pose = client.publish::<Pose2d>("/example/pose", PublishOptions::default());
    let modules =
        client.publish::<Vec<SwerveModuleVelocity>>("/example/modules", PublishOptions::default());

    let mut t: f64 = 0.0;
    loop {
        t += 0.05;
        pose.set(Pose2d {
            translation: Translation2d {
                x: 4.0 + 2.0 * t.cos(),
                y: 4.0 + 2.0 * t.sin(),
            },
            rotation: Rotation2d {
                value: t + std::f64::consts::FRAC_PI_2,
            },
        });
        modules.set(vec![
            SwerveModuleVelocity {
                velocity: 2.0,
                angle: Rotation2d { value: t },
            };
            4
        ]);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
