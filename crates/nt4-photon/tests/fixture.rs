//! A real PhotonVision 2027 result (see tests/data/*.json for where it came
//! from): decoding gives the best target PhotonVision published on its own
//! topics, encoding gives the same bytes back, and our message formats and
//! hashes are PhotonVision's.
//!
//! Written by Claude.

use md5::{Digest, Md5};
use nt4::structs::NtStruct;
use nt4::{NtValue, Properties};
use nt4_photon::{
    MultiTargetPnpResult, PhotonPipelineMetadata, PhotonPipelineResult, PhotonStruct,
    PhotonTrackedTarget, PnpResult, TargetCorner, Transform3d, check_version,
};
use serde_json::Value as J;

fn fixture() -> J {
    serde_json::from_str(include_str!("data/pipeline_result_fd353b11.json")).unwrap()
}

fn bytes(j: &J) -> Vec<u8> {
    hex::decode(j.as_str().unwrap()).unwrap()
}

#[test]
fn decodes_to_the_best_target_photonvision_published() {
    let fx = fixture();
    let result = PhotonPipelineResult::decode(&bytes(&fx["raw_bytes"])).unwrap();
    assert!(result.has_targets());
    let best = result.best_target().unwrap();
    let topic = |k: &str| fx["best_target"][k].as_f64().unwrap();
    assert_eq!(best.yaw, topic("targetYaw"));
    assert_eq!(best.pitch, topic("targetPitch"));
    assert_eq!(best.area, topic("targetArea"));
    assert_eq!(best.skew, topic("targetSkew"));
    let pose = <Transform3d as NtStruct>::unpack(&bytes(&fx["best_target_pose"]));
    assert_eq!(best.best_camera_to_target, pose);
    // An AprilTag pipeline: tags, four corners each.
    assert!(best.fiducial_id > 0, "{best:?}");
    assert_eq!(best.detected_corners.len(), 4);
    assert!(result.metadata.sequence_id > 0);
    assert!(result.metadata.publish_timestamp_ns >= result.metadata.capture_timestamp_ns);
}

#[test]
fn encodes_back_to_the_same_bytes() {
    let raw = bytes(&fixture()["raw_bytes"]);
    let result = PhotonPipelineResult::decode(&raw).unwrap();
    assert_eq!(result.encode().unwrap(), raw);
}

#[test]
fn topic_type_and_properties_match() {
    let fx = fixture();
    assert_eq!(
        PhotonPipelineResult::nt_type().to_string(),
        fx["type"].as_str().unwrap()
    );
    let props: Properties = serde_json::from_value(fx["properties"].clone()).unwrap();
    assert!(check_version(&props).is_ok());
    assert_eq!(<PhotonPipelineResult as NtValue>::properties(), props);
}

#[test]
fn schemas_are_the_ones_photonvision_publishes() {
    let fx = fixture();
    let published = fx["schemas"].as_object().unwrap();
    let mut ours = Vec::new();
    <PhotonPipelineResult as PhotonStruct>::for_each_schema(&mut |name, ty, schema| {
        if ty == nt4_photon::PHOTONSTRUCT_SCHEMA_TYPE {
            ours.push((name.to_owned(), String::from_utf8(schema.to_vec()).unwrap()));
        }
    });
    assert_eq!(ours.len(), published.len());
    for (name, schema) in ours {
        assert_eq!(published[&name].as_str(), Some(schema.as_str()), "{name}");
    }
}

fn md5_hex(s: &str) -> String {
    hex::encode(Md5::digest(s.as_bytes()))
}

#[test]
fn versions_are_the_md5_of_the_formats() {
    fn check<T: PhotonStruct>() {
        assert_eq!(
            md5_hex(T::MESSAGE_FORMAT),
            T::MESSAGE_VERSION,
            "{}",
            T::TYPE_NAME
        );
    }
    check::<PhotonPipelineResult>();
    check::<PhotonPipelineMetadata>();
    check::<PhotonTrackedTarget>();
    check::<TargetCorner>();
    check::<MultiTargetPnpResult>();
    check::<PnpResult>();
}
