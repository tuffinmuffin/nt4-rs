//! PhotonVision's pipeline results as typed NetworkTables topics, for the
//! [`nt4`] crate.
//!
//! Unofficial: not affiliated with or endorsed by PhotonVision, WPILib or
//! FIRST.
//!
//! A PhotonVision camera publishes each frame's result on
//! `/photonvision/<camera>/rawBytes`, typed
//! `photonstruct:PhotonPipelineResult:<hash>`, in PhotonVision's own
//! "photonstruct" packet format: WPILib's packed little-endian struct
//! layout plus variable-length lists and optional fields. The hash is the
//! MD5 of the message format and changes whenever the format does.
//!
//! This crate has the format of PhotonVision 2027 (hash
//! [`PhotonPipelineResult::MESSAGE_VERSION`], from PhotonVision's `main`
//! at `dev-v2027.0.0-alpha-2-89`; the 2027 alpha-2 release still has the
//! 2026 format). A result of another version won't match:
//! [`check_version`] tells you, using the topic's `message_uuid` property.
//!
//! ```no_run
//! # async fn demo() {
//! use nt4::{Client, ClientConfig};
//! use nt4_photon::{PhotonPipelineResult, result_topic};
//!
//! let client = Client::start(ClientConfig::team("vision-reader", 3200));
//! let mut results =
//!     client.subscribe::<PhotonPipelineResult>(&result_topic("Front"), Default::default());
//! while let Ok(Some(result)) = results.changed().await {
//!     if let Some(best) = result.best_target() {
//!         println!("tag {} at yaw {:.1} deg", best.fiducial_id, best.yaw);
//!     }
//! }
//! # }
//! ```
//!
//! Publishing a [`PhotonPipelineResult`] (from a coprocessor of your own)
//! registers the photonstruct schemas under `/.schema/` and sets the
//! `message_uuid` property, as PhotonVision's `PacketPublisher` does, so
//! robot code's photonlib accepts it.

use std::fmt;

use nt4::structs::NtStruct;
use nt4::{NtType, NtValue, Properties, PublishOptions, Value};
pub use nt4_wpimath::Transform3d;

/// Schema type of photonstruct schemas, on
/// `/.schema/photonstruct:<name>:<hash>`.
pub const PHOTONSTRUCT_SCHEMA_TYPE: &str = "photonstructschema";

/// The topic a PhotonVision camera publishes its results on:
/// `/photonvision/<camera>/rawBytes`.
///
/// ```
/// assert_eq!(nt4_photon::result_topic("Front"), "/photonvision/Front/rawBytes");
/// ```
pub fn result_topic(camera: &str) -> String {
    format!("/photonvision/{camera}/rawBytes")
}

/// Publish options PhotonVision uses for results: every value sent
/// (`sendAll`), checked every 10 ms.
///
/// ```
/// assert!(nt4_photon::publish_options().send_all);
/// ```
pub fn publish_options() -> PublishOptions {
    PublishOptions {
        periodic: Some(0.01),
        send_all: true,
    }
}

/// Whether a result topic has the format this crate decodes, from its
/// `message_uuid` property (as PhotonVision's own clients check it). Gives
/// the topic's version when it differs.
///
/// ```
/// use nt4::Properties;
/// use nt4_photon::{PhotonPipelineResult, PhotonStruct, check_version};
///
/// let mut props = Properties::new();
/// props.insert("message_uuid".into(), PhotonPipelineResult::MESSAGE_VERSION.into());
/// assert!(check_version(&props).is_ok());
/// props.insert("message_uuid".into(), "4b2ff16a964b5e2bf04be0c1454d91c4".into());
/// assert_eq!(check_version(&props).unwrap_err(), Some("4b2ff16a964b5e2bf04be0c1454d91c4".into()));
/// ```
pub fn check_version(properties: &Properties) -> Result<(), Option<String>> {
    match properties.get("message_uuid").and_then(|v| v.as_str()) {
        Some(v) if v == PhotonPipelineResult::MESSAGE_VERSION => Ok(()),
        other => Err(other.map(str::to_owned)),
    }
}

/// Why bytes didn't decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The data ended `needed` bytes short, at byte `offset`. (PhotonVision's
    /// Java decoder reads missing bytes as zeros; this one refuses.)
    Truncated {
        /// Bytes still needed.
        needed: usize,
        /// Where the data ended.
        offset: usize,
    },
    /// A list count byte above 127, at byte `offset`. PhotonVision never
    /// writes one (its lists hold at most 127 items); its Java decoder reads
    /// the count as a signed byte, so it would decode the list as empty and
    /// misread everything after it.
    ListTooLong {
        /// The count byte.
        count: u8,
        /// Where it is.
        offset: usize,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Truncated { needed, offset } => {
                write!(f, "data ends at byte {offset}, {needed} bytes short")
            }
            DecodeError::ListTooLong { count, offset } => {
                write!(f, "list count {count} at byte {offset} is over 127")
            }
        }
    }
}

impl std::error::Error for DecodeError {}

/// A list had more than 127 items, which the format can't hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListTooLong {
    /// The list's field, e.g. `PhotonPipelineResult.targets`.
    pub field: &'static str,
    /// How many items it had.
    pub len: usize,
}

impl fmt::Display for ListTooLong {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} has {} items; at most 127 fit", self.field, self.len)
    }
}

impl std::error::Error for ListTooLong {}

/// Reads a photonstruct packet front to back.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Start at the beginning of `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Bytes read so far.
    pub fn position(&self) -> usize {
        self.pos
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let rest = &self.data[self.pos..];
        if rest.len() < N {
            return Err(DecodeError::Truncated {
                needed: N - rest.len(),
                offset: self.data.len(),
            });
        }
        let mut out = [0u8; N];
        out.copy_from_slice(&rest[..N]);
        self.pos += N;
        Ok(out)
    }

    fn f64(&mut self) -> Result<f64, DecodeError> {
        self.take().map(f64::from_le_bytes)
    }
    fn f32(&mut self) -> Result<f32, DecodeError> {
        self.take().map(f32::from_le_bytes)
    }
    fn i64(&mut self) -> Result<i64, DecodeError> {
        self.take().map(i64::from_le_bytes)
    }
    fn i32(&mut self) -> Result<i32, DecodeError> {
        self.take().map(i32::from_le_bytes)
    }
    fn i16(&mut self) -> Result<i16, DecodeError> {
        self.take().map(i16::from_le_bytes)
    }

    /// True only for a 1 byte, as PhotonVision decodes it.
    fn bool(&mut self) -> Result<bool, DecodeError> {
        self.take::<1>().map(|[b]| b == 1)
    }

    fn transform(&mut self) -> Result<Transform3d, DecodeError> {
        // PhotonVision packs Transform3d as WPILib's struct does: x, y, z,
        // then the quaternion's w, x, y, z.
        let bytes: [u8; 56] = self.take()?;
        Ok(<Transform3d as NtStruct>::unpack(&bytes))
    }

    fn count(&mut self) -> Result<usize, DecodeError> {
        let offset = self.pos;
        let [count] = self.take::<1>()?;
        if count > 127 {
            return Err(DecodeError::ListTooLong { count, offset });
        }
        Ok(count as usize)
    }

    fn list<T>(
        &mut self,
        mut item: impl FnMut(&mut Self) -> Result<T, DecodeError>,
    ) -> Result<Vec<T>, DecodeError> {
        let n = self.count()?;
        (0..n).map(|_| item(self)).collect()
    }
}

fn write_count(out: &mut Vec<u8>, field: &'static str, len: usize) -> Result<(), ListTooLong> {
    if len > 127 {
        return Err(ListTooLong { field, len });
    }
    out.push(len as u8);
    Ok(())
}

fn write_transform(out: &mut Vec<u8>, t: &Transform3d) {
    NtStruct::pack(t, out);
}

/// A PhotonVision message type: its name, format and version hash, and
/// how to read and write it.
pub trait PhotonStruct: Sized {
    /// The type name, e.g. `PhotonTrackedTarget`.
    const TYPE_NAME: &'static str;
    /// The message format (photonstruct schema), as PhotonVision's
    /// `MESSAGE_FORMAT`.
    const MESSAGE_FORMAT: &'static str;
    /// The MD5 of [`MESSAGE_FORMAT`](Self::MESSAGE_FORMAT), as PhotonVision's
    /// `MESSAGE_VERSION`.
    const MESSAGE_VERSION: &'static str;

    /// Append the packed message.
    fn pack(&self, out: &mut Vec<u8>) -> Result<(), ListTooLong>;

    /// Read the message from `r`.
    fn unpack(r: &mut Reader<'_>) -> Result<Self, DecodeError>;

    /// The photonstruct types this one contains, and the WPILib structs, as
    /// `for_each_schema` reports them. The default is for none.
    fn for_each_nested(f: &mut dyn FnMut(&str, &str, &[u8])) {
        let _ = f;
    }

    /// `photonstruct:<name>:<hash>`, the topic type and schema name.
    fn type_string() -> String {
        format!("photonstruct:{}:{}", Self::TYPE_NAME, Self::MESSAGE_VERSION)
    }

    /// Each schema this type needs, as (schema name, schema type, schema),
    /// in the order PhotonVision's `PacketPublisher` registers them: this
    /// type's, then its photonstruct types' (each followed by theirs), then
    /// the WPILib structs it uses.
    fn for_each_schema(f: &mut dyn FnMut(&str, &str, &[u8])) {
        f(
            &Self::type_string(),
            PHOTONSTRUCT_SCHEMA_TYPE,
            Self::MESSAGE_FORMAT.as_bytes(),
        );
        Self::for_each_nested(f);
    }

    /// Decode a whole packet. Bytes after the message are ignored, as
    /// PhotonVision ignores them.
    fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        Self::unpack(&mut Reader::new(data))
    }

    /// Encode to a new packet.
    fn encode(&self) -> Result<Vec<u8>, ListTooLong> {
        let mut out = Vec::new();
        self.pack(&mut out)?;
        Ok(out)
    }
}

/// The WPILib structs `Transform3d` needs, as schema triples.
fn transform3d_schemas(f: &mut dyn FnMut(&str, &str, &[u8])) {
    Transform3d::for_each_struct_schema(&mut |ty, schema| {
        f(ty, nt4::structs::STRUCT_SCHEMA_TYPE, schema.as_bytes())
    });
}

/// When and in what order a frame was captured and published.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PhotonPipelineMetadata {
    /// Frame counter, the same as the camera's `heartbeat` topic.
    pub sequence_id: i64,
    /// When the image was captured: nanoseconds of the time sync server's
    /// clock (the robot's NT server time, times 1000).
    pub capture_timestamp_ns: i64,
    /// When the result was published, on the same clock.
    pub publish_timestamp_ns: i64,
    /// Nanoseconds from the last time sync pong to this result.
    pub time_since_last_pong_ns: i64,
}

impl PhotonStruct for PhotonPipelineMetadata {
    const TYPE_NAME: &'static str = "PhotonPipelineMetadata";
    const MESSAGE_FORMAT: &'static str = "int64 sequenceID;int64 captureTimestampNanos;int64 publishTimestampNanos;int64 timeSinceLastPong;";
    const MESSAGE_VERSION: &'static str = "5231116d7f87ff560e6aa5263d0eee02";

    fn pack(&self, out: &mut Vec<u8>) -> Result<(), ListTooLong> {
        out.extend_from_slice(&self.sequence_id.to_le_bytes());
        out.extend_from_slice(&self.capture_timestamp_ns.to_le_bytes());
        out.extend_from_slice(&self.publish_timestamp_ns.to_le_bytes());
        out.extend_from_slice(&self.time_since_last_pong_ns.to_le_bytes());
        Ok(())
    }

    fn unpack(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            sequence_id: r.i64()?,
            capture_timestamp_ns: r.i64()?,
            publish_timestamp_ns: r.i64()?,
            time_since_last_pong_ns: r.i64()?,
        })
    }
}

/// A corner of a target, in image pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TargetCorner {
    /// Pixels from the left.
    pub x: f64,
    /// Pixels from the top.
    pub y: f64,
}

impl PhotonStruct for TargetCorner {
    const TYPE_NAME: &'static str = "TargetCorner";
    const MESSAGE_FORMAT: &'static str = "float64 x;float64 y;";
    const MESSAGE_VERSION: &'static str = "16f6ac0dedc8eaccb951f4895d9e18b6";

    fn pack(&self, out: &mut Vec<u8>) -> Result<(), ListTooLong> {
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
        Ok(())
    }

    fn unpack(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            x: r.f64()?,
            y: r.f64()?,
        })
    }
}

/// One target the pipeline found.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PhotonTrackedTarget {
    /// Degrees, positive to the right (PhotonVision's convention).
    pub yaw: f64,
    /// Degrees, positive up.
    pub pitch: f64,
    /// Percent of the image the target covers (0 to 100).
    pub area: f64,
    /// Degrees of skew.
    pub skew: f64,
    /// AprilTag id, or -1 if not a tag.
    pub fiducial_id: i32,
    /// Object detection class id, or -1.
    pub obj_detect_id: i32,
    /// Object detection confidence (0 to 1), or -1.
    pub obj_detect_conf: f32,
    /// The camera-to-target transform with the lowest reprojection error.
    pub best_camera_to_target: Transform3d,
    /// The other solution of the pose ambiguity.
    pub alt_camera_to_target: Transform3d,
    /// Ratio of best to alternate reprojection error (0 to 1), or -1.
    pub pose_ambiguity: f64,
    /// Corners of the minimum-area rectangle around the target.
    pub min_area_rect_corners: Vec<TargetCorner>,
    /// Corners as detected (for an AprilTag, its four corners).
    pub detected_corners: Vec<TargetCorner>,
}

impl PhotonStruct for PhotonTrackedTarget {
    const TYPE_NAME: &'static str = "PhotonTrackedTarget";
    const MESSAGE_FORMAT: &'static str = "float64 yaw;float64 pitch;float64 area;float64 skew;int32 fiducialId;int32 objDetectId;float32 objDetectConf;Transform3d bestCameraToTarget;Transform3d altCameraToTarget;float64 poseAmbiguity;TargetCorner:16f6ac0dedc8eaccb951f4895d9e18b6 minAreaRectCorners[?];TargetCorner:16f6ac0dedc8eaccb951f4895d9e18b6 detectedCorners[?];";
    const MESSAGE_VERSION: &'static str = "cc6dbb5c5c1e0fa808108019b20863f1";

    fn pack(&self, out: &mut Vec<u8>) -> Result<(), ListTooLong> {
        out.extend_from_slice(&self.yaw.to_le_bytes());
        out.extend_from_slice(&self.pitch.to_le_bytes());
        out.extend_from_slice(&self.area.to_le_bytes());
        out.extend_from_slice(&self.skew.to_le_bytes());
        out.extend_from_slice(&self.fiducial_id.to_le_bytes());
        out.extend_from_slice(&self.obj_detect_id.to_le_bytes());
        out.extend_from_slice(&self.obj_detect_conf.to_le_bytes());
        write_transform(out, &self.best_camera_to_target);
        write_transform(out, &self.alt_camera_to_target);
        out.extend_from_slice(&self.pose_ambiguity.to_le_bytes());
        write_count(
            out,
            "PhotonTrackedTarget.min_area_rect_corners",
            self.min_area_rect_corners.len(),
        )?;
        for c in &self.min_area_rect_corners {
            c.pack(out)?;
        }
        write_count(
            out,
            "PhotonTrackedTarget.detected_corners",
            self.detected_corners.len(),
        )?;
        for c in &self.detected_corners {
            c.pack(out)?;
        }
        Ok(())
    }

    fn unpack(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            yaw: r.f64()?,
            pitch: r.f64()?,
            area: r.f64()?,
            skew: r.f64()?,
            fiducial_id: r.i32()?,
            obj_detect_id: r.i32()?,
            obj_detect_conf: r.f32()?,
            best_camera_to_target: r.transform()?,
            alt_camera_to_target: r.transform()?,
            pose_ambiguity: r.f64()?,
            min_area_rect_corners: r.list(TargetCorner::unpack)?,
            detected_corners: r.list(TargetCorner::unpack)?,
        })
    }

    fn for_each_nested(f: &mut dyn FnMut(&str, &str, &[u8])) {
        TargetCorner::for_each_schema(f);
        transform3d_schemas(f);
    }
}

/// A camera pose solved from target corners.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PnpResult {
    /// The solution with the lowest reprojection error.
    pub best: Transform3d,
    /// The alternate solution (the same as `best` for multi-tag results).
    pub alt: Transform3d,
    /// Reprojection error of `best`, in pixels.
    pub best_reproj_err: f64,
    /// Reprojection error of `alt`, in pixels.
    pub alt_reproj_err: f64,
    /// `best_reproj_err / alt_reproj_err` (0 to 1).
    pub ambiguity: f64,
}

impl PhotonStruct for PnpResult {
    const TYPE_NAME: &'static str = "PnpResult";
    const MESSAGE_FORMAT: &'static str = "Transform3d best;Transform3d alt;float64 bestReprojErr;float64 altReprojErr;float64 ambiguity;";
    const MESSAGE_VERSION: &'static str = "ae4d655c0a3104d88df4f5db144c1e86";

    fn pack(&self, out: &mut Vec<u8>) -> Result<(), ListTooLong> {
        write_transform(out, &self.best);
        write_transform(out, &self.alt);
        out.extend_from_slice(&self.best_reproj_err.to_le_bytes());
        out.extend_from_slice(&self.alt_reproj_err.to_le_bytes());
        out.extend_from_slice(&self.ambiguity.to_le_bytes());
        Ok(())
    }

    fn unpack(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            best: r.transform()?,
            alt: r.transform()?,
            best_reproj_err: r.f64()?,
            alt_reproj_err: r.f64()?,
            ambiguity: r.f64()?,
        })
    }

    fn for_each_nested(f: &mut dyn FnMut(&str, &str, &[u8])) {
        transform3d_schemas(f);
    }
}

/// The field-to-camera pose solved from every AprilTag in view at once.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MultiTargetPnpResult {
    /// The solved pose (field to camera).
    pub estimated_pose: PnpResult,
    /// The tags it used.
    pub fiducial_ids_used: Vec<i16>,
}

impl PhotonStruct for MultiTargetPnpResult {
    const TYPE_NAME: &'static str = "MultiTargetPNPResult";
    const MESSAGE_FORMAT: &'static str =
        "PnpResult:ae4d655c0a3104d88df4f5db144c1e86 estimatedPose;int16 fiducialIDsUsed[?];";
    const MESSAGE_VERSION: &'static str = "541096947e9f3ca2d3f425ff7b04aa7b";

    fn pack(&self, out: &mut Vec<u8>) -> Result<(), ListTooLong> {
        self.estimated_pose.pack(out)?;
        write_count(
            out,
            "MultiTargetPnpResult.fiducial_ids_used",
            self.fiducial_ids_used.len(),
        )?;
        for id in &self.fiducial_ids_used {
            out.extend_from_slice(&id.to_le_bytes());
        }
        Ok(())
    }

    fn unpack(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            estimated_pose: PnpResult::unpack(r)?,
            fiducial_ids_used: r.list(Reader::i16)?,
        })
    }

    fn for_each_nested(f: &mut dyn FnMut(&str, &str, &[u8])) {
        PnpResult::for_each_schema(f);
    }
}

/// One frame's result from a PhotonVision pipeline: the value of
/// `/photonvision/<camera>/rawBytes`.
///
/// ```
/// use nt4::NtValue;
/// use nt4_photon::{PhotonPipelineResult, PhotonStruct, PhotonTrackedTarget};
///
/// let result = PhotonPipelineResult {
///     targets: vec![PhotonTrackedTarget { fiducial_id: 7, yaw: -3.5, ..Default::default() }],
///     ..Default::default()
/// };
/// let bytes = result.encode().unwrap();
/// assert_eq!(PhotonPipelineResult::decode(&bytes).unwrap(), result);
/// assert_eq!(
///     PhotonPipelineResult::nt_type().to_string(),
///     "photonstruct:PhotonPipelineResult:fd353b11f3a818e9c5b457c7dc5a8f5c"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PhotonPipelineResult {
    /// Frame counter and timestamps.
    pub metadata: PhotonPipelineMetadata,
    /// The targets found, best first.
    pub targets: Vec<PhotonTrackedTarget>,
    /// The multi-tag pose, when the pipeline solved one.
    pub multitag_result: Option<MultiTargetPnpResult>,
}

impl PhotonPipelineResult {
    /// The best target (the first), if any were found.
    pub fn best_target(&self) -> Option<&PhotonTrackedTarget> {
        self.targets.first()
    }

    /// Whether any targets were found.
    pub fn has_targets(&self) -> bool {
        !self.targets.is_empty()
    }
}

impl PhotonStruct for PhotonPipelineResult {
    const TYPE_NAME: &'static str = "PhotonPipelineResult";
    const MESSAGE_FORMAT: &'static str = "PhotonPipelineMetadata:5231116d7f87ff560e6aa5263d0eee02 metadata;PhotonTrackedTarget:cc6dbb5c5c1e0fa808108019b20863f1 targets[?];optional MultiTargetPNPResult:541096947e9f3ca2d3f425ff7b04aa7b multitagResult;";
    const MESSAGE_VERSION: &'static str = "fd353b11f3a818e9c5b457c7dc5a8f5c";

    fn pack(&self, out: &mut Vec<u8>) -> Result<(), ListTooLong> {
        self.metadata.pack(out)?;
        write_count(out, "PhotonPipelineResult.targets", self.targets.len())?;
        for t in &self.targets {
            t.pack(out)?;
        }
        match &self.multitag_result {
            Some(m) => {
                out.push(1);
                m.pack(out)?;
            }
            None => out.push(0),
        }
        Ok(())
    }

    fn unpack(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            metadata: PhotonPipelineMetadata::unpack(r)?,
            targets: r.list(PhotonTrackedTarget::unpack)?,
            multitag_result: if r.bool()? {
                Some(MultiTargetPnpResult::unpack(r)?)
            } else {
                None
            },
        })
    }

    fn for_each_nested(f: &mut dyn FnMut(&str, &str, &[u8])) {
        PhotonPipelineMetadata::for_each_schema(f);
        PhotonTrackedTarget::for_each_schema(f);
        MultiTargetPnpResult::for_each_schema(f);
    }
}

impl NtValue for PhotonPipelineResult {
    fn nt_type() -> NtType {
        NtType::Other(Self::type_string())
    }

    /// Encodes the result. A list over 127 items (which PhotonVision can't
    /// send either) is cut to its first 127; use
    /// [`encode`](PhotonStruct::encode) to get an error instead.
    fn into_value(mut self) -> Value {
        self.targets.truncate(127);
        for t in &mut self.targets {
            t.min_area_rect_corners.truncate(127);
            t.detected_corners.truncate(127);
        }
        if let Some(m) = &mut self.multitag_result {
            m.fiducial_ids_used.truncate(127);
        }
        Value::Raw(self.encode().expect("lists were cut to 127"))
    }

    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Raw(data) => Self::decode(data).ok(),
            _ => None,
        }
    }

    fn for_each_schema(f: &mut dyn FnMut(&str, &str, &[u8])) {
        <Self as PhotonStruct>::for_each_schema(f);
    }

    fn properties() -> Properties {
        let mut props = Properties::new();
        props.insert("message_uuid".into(), Self::MESSAGE_VERSION.into());
        props
    }
}

#[cfg(test)]
mod tests {
    // Written by Claude.
    use super::*;

    fn sample() -> PhotonPipelineResult {
        let corner = |x, y| TargetCorner { x, y };
        PhotonPipelineResult {
            metadata: PhotonPipelineMetadata {
                sequence_id: 42,
                capture_timestamp_ns: 1_000_000,
                publish_timestamp_ns: 1_020_000,
                time_since_last_pong_ns: 5_000,
            },
            targets: vec![PhotonTrackedTarget {
                yaw: -2.0,
                pitch: 1.5,
                area: 0.25,
                fiducial_id: 3,
                obj_detect_id: -1,
                obj_detect_conf: -1.0,
                pose_ambiguity: 0.1,
                detected_corners: vec![corner(1.0, 2.0), corner(3.0, 4.0)],
                ..Default::default()
            }],
            multitag_result: Some(MultiTargetPnpResult {
                fiducial_ids_used: vec![3, 4],
                ..Default::default()
            }),
        }
    }

    #[test]
    fn round_trips() {
        let r = sample();
        assert_eq!(
            PhotonPipelineResult::decode(&r.encode().unwrap()).unwrap(),
            r
        );
        let empty = PhotonPipelineResult::default();
        let bytes = empty.encode().unwrap();
        // Metadata, a 0 target count and a 0 "no multitag" flag.
        assert_eq!(bytes.len(), 32 + 1 + 1);
        assert_eq!(PhotonPipelineResult::decode(&bytes).unwrap(), empty);
    }

    #[test]
    fn sizes() {
        let t = PhotonTrackedTarget::default().encode().unwrap();
        // 4 doubles, 2 int32, a float, 2 transforms, a double, 2 counts.
        assert_eq!(t.len(), 32 + 8 + 4 + 112 + 8 + 2);
        assert_eq!(PnpResult::default().encode().unwrap().len(), 112 + 24);
    }

    #[test]
    fn truncated_data_is_refused_at_every_length() {
        let bytes = sample().encode().unwrap();
        for len in 0..bytes.len() {
            assert!(
                matches!(
                    PhotonPipelineResult::decode(&bytes[..len]),
                    Err(DecodeError::Truncated { .. })
                ),
                "length {len}"
            );
        }
    }

    #[test]
    fn trailing_bytes_are_ignored() {
        let mut bytes = sample().encode().unwrap();
        bytes.extend_from_slice(&[9, 9, 9]);
        assert_eq!(PhotonPipelineResult::decode(&bytes).unwrap(), sample());
    }

    #[test]
    fn a_count_over_127_is_refused() {
        let mut bytes = PhotonPipelineResult::default().encode().unwrap();
        bytes[32] = 200;
        assert_eq!(
            PhotonPipelineResult::decode(&bytes),
            Err(DecodeError::ListTooLong {
                count: 200,
                offset: 32
            })
        );
    }

    #[test]
    fn optional_flag_is_true_only_for_1() {
        let mut bytes = PhotonPipelineResult::default().encode().unwrap();
        bytes[33] = 2; // as in PhotonVision: not 1 means absent
        assert_eq!(
            PhotonPipelineResult::decode(&bytes)
                .unwrap()
                .multitag_result,
            None
        );
    }

    #[test]
    fn long_lists() {
        let r = PhotonPipelineResult {
            targets: vec![PhotonTrackedTarget::default(); 128],
            ..Default::default()
        };
        assert_eq!(
            r.encode(),
            Err(ListTooLong {
                field: "PhotonPipelineResult.targets",
                len: 128
            })
        );
        // As a topic value it is cut to 127 instead.
        let v = r.into_value();
        assert_eq!(
            PhotonPipelineResult::from_value(&v).unwrap().targets.len(),
            127
        );
    }

    #[test]
    fn schemas_in_packet_publisher_order() {
        let mut names = Vec::new();
        <PhotonPipelineResult as PhotonStruct>::for_each_schema(&mut |n, ty, _| {
            names.push(format!("{ty} {n}"))
        });
        let photon = |n: &str, h: &str| format!("photonstructschema photonstruct:{n}:{h}");
        let wpilib = |n: &str| format!("structschema struct:{n}");
        let transform = [
            wpilib("Translation3d"),
            wpilib("Quaternion"),
            wpilib("Rotation3d"),
            wpilib("Transform3d"),
        ];
        let mut expected = vec![
            photon(
                "PhotonPipelineResult",
                PhotonPipelineResult::MESSAGE_VERSION,
            ),
            photon(
                "PhotonPipelineMetadata",
                PhotonPipelineMetadata::MESSAGE_VERSION,
            ),
            photon("PhotonTrackedTarget", PhotonTrackedTarget::MESSAGE_VERSION),
            photon("TargetCorner", TargetCorner::MESSAGE_VERSION),
        ];
        expected.extend(transform.iter().cloned());
        expected.push(photon(
            "MultiTargetPNPResult",
            MultiTargetPnpResult::MESSAGE_VERSION,
        ));
        expected.push(photon("PnpResult", PnpResult::MESSAGE_VERSION));
        expected.extend(transform.iter().cloned());
        assert_eq!(names, expected);
    }

    #[test]
    fn publishes_message_uuid() {
        let props = <PhotonPipelineResult as NtValue>::properties();
        assert!(check_version(&props).is_ok());
        assert!(check_version(&Properties::new()) == Err(None));
    }
}
