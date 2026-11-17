//! Publishing results through nt4 gives the topic PhotonVision's server
//! gives (type, `message_uuid`, `/.schema/` topics), and a subscriber reads
//! them back. The PhotonVision side is the real capture in tests/data.
//!
//! Written by Claude.

use std::time::Duration;

use nt4::{Client, ClientConfig, NtType, Server, ServerConfig, SubscriptionOptions};
use nt4_photon::{PhotonPipelineResult, PhotonStruct, publish_options, result_topic};
use serde_json::Value as J;

async fn pair() -> (Server, Client, Client) {
    let server = Server::start(ServerConfig {
        listen_address: "127.0.0.1".parse().unwrap(),
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap();
    let port = server.local_addr().port();
    let a = Client::start(ClientConfig::server("coprocessor", "127.0.0.1", port));
    let b = Client::start(ClientConfig::server("robot", "127.0.0.1", port));
    assert!(a.wait_for_sync(Duration::from_secs(5)).await);
    assert!(b.wait_for_sync(Duration::from_secs(5)).await);
    (server, a, b)
}

async fn eventually<T>(mut f: impl FnMut() -> Option<T>) -> T {
    for _ in 0..500 {
        if let Some(v) = f() {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out");
}

fn captured() -> (PhotonPipelineResult, J) {
    let fx: J = serde_json::from_str(include_str!("data/pipeline_result_fd353b11.json")).unwrap();
    let raw = hex::decode(fx["raw_bytes"].as_str().unwrap()).unwrap();
    (PhotonPipelineResult::decode(&raw).unwrap(), fx)
}

#[tokio::test]
async fn published_topic_looks_like_photonvisions() {
    let (_server, coprocessor, robot) = pair().await;
    let (result, fx) = captured();
    let topic = result_topic("Front");

    let sub = robot.subscribe::<PhotonPipelineResult>(&topic, Default::default());
    // Everything, to look at the schema topics too.
    let _all = robot.subscribe_multiple(
        vec!["".into()],
        SubscriptionOptions {
            prefix: true,
            ..Default::default()
        },
    );
    let publisher = coprocessor.publish::<PhotonPipelineResult>(&topic, publish_options());
    publisher.set(result.clone());

    let got = eventually(|| sub.get()).await;
    assert_eq!(got, result);

    // The type and message_uuid PhotonVision sets.
    let t = eventually(|| robot.topic(&topic)).await;
    assert_eq!(t.ty, NtType::Other(fx["type"].as_str().unwrap().into()));
    assert_eq!(J::Object(t.properties.clone()), fx["properties"]);

    // And the same photonstruct schemas, retained, as PhotonVision publishes.
    for (name, schema) in fx["schemas"].as_object().unwrap() {
        let path = format!("/.schema/{name}");
        let t = eventually(|| robot.topic(&path).filter(|t| t.value.is_some())).await;
        assert_eq!(t.ty, NtType::Other("photonstructschema".into()), "{path}");
        assert_eq!(t.properties.get("retained"), Some(&J::Bool(true)), "{path}");
        assert_eq!(
            t.value.unwrap().value,
            nt4::Value::Raw(schema.as_str().unwrap().as_bytes().to_vec()),
            "{path}"
        );
    }
    assert!(coprocessor.has_schema("struct:Transform3d"));
    assert!(coprocessor.has_schema(&PhotonPipelineResult::type_string()));
}
