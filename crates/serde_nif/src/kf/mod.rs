//! KF animation files: one `NiControllerSequence` and the keys it drives.
//!
//! A KF file is an ordinary NIF file (same header and footer) whose root is a
//! sequence instead of a scene graph. Skyrim does not play KF files itself;
//! they are the interchange format of hkxcmd, the Blender/Max NIF tools and
//! niflib, which write 20.2.0.7 with user version 11 and BS version 83.
//!
//! ```text
//! NiControllerSequence "animation"            root (block 0)
//! ├─ controlled blocks: per animated node
//! │   ├─ node name "Pelvis"                   matched against the skeleton bone names
//! │   └─ interpolator → NiTransformInterpolator
//! │        ├─ pose: translation / rotation / scale   used for channels without keys
//! │        └─ data → NiTransformData                 the keys (see `TransformData`)
//! └─ text keys → NiTextKeyExtraData           annotations, e.g. (0.53, "HitFrame")
//! ```
//!
//! Only these four block types are decoded; others (controllers of embedded
//! scenes, BSAnimNotes, ...) are skipped by their stored size.
//!
//! Layouts follow niftools' `nif.xml` for 20.2.0.7 and were checked against
//! files written by niflib.

mod de;
mod key;
mod ser;

use std::borrow::Cow;

pub use self::key::{Key, KeyGroup, KeyType, Quaternion};

/// `NiControllerSequence`: a named animation clip.
#[derive(Debug, Clone, PartialEq)]
pub struct Sequence<'a> {
    pub name: Cow<'a, str>,
    /// Blend weight when several sequences play at once (1.0 = full).
    pub weight: f32,
    pub cycle_type: CycleType,
    /// Playback speed multiplier.
    pub frequency: f32,
    /// Seconds; key times are absolute, so the clip spans `start_time..=stop_time`.
    pub start_time: f32,
    pub stop_time: f32,
    /// Node whose motion is accumulated into the actor position (root motion).
    pub accum_root_name: Option<Cow<'a, str>>,
    pub controlled_blocks: Vec<ControlledBlock<'a>>,
    /// `NiTextKeyExtraData`: annotations (`start`, `end`, `HitFrame`, sound events, ...).
    pub text_keys: Vec<TextKey<'a>>,
}

impl Default for Sequence<'_> {
    fn default() -> Self {
        Self {
            name: Cow::Borrowed(""),
            weight: 1.0,
            cycle_type: CycleType::Clamp,
            frequency: 1.0,
            start_time: 0.0,
            stop_time: 0.0,
            accum_root_name: None,
            controlled_blocks: Vec::new(),
            text_keys: Vec::new(),
        }
    }
}

/// What happens after `stop_time`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum CycleType {
    Loop = 0,
    /// Ping-pong.
    Reverse = 1,
    /// Hold the last frame.
    #[default]
    Clamp = 2,
}

/// One animated node (`ControllerLink` in `nif.xml`).
#[derive(Debug, Clone, PartialEq)]
pub struct ControlledBlock<'a> {
    pub node_name: Cow<'a, str>,
    /// Blending priority against other sequences.
    pub priority: u8,
    /// `None` when the link drives something other than a transform
    /// (material colors, visibility, ...).
    pub interpolator: Option<TransformInterpolator>,
}

/// `NiTransformInterpolator`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TransformInterpolator {
    /// Pose used for channels that have no keys. Components equal to
    /// [`INVALID_POSE`] mean "unset".
    pub translation: [f32; 3],
    pub rotation: Quaternion,
    pub scale: f32,
    pub data: Option<TransformData>,
}

/// Marks an unset pose component (`-FLT_MAX`, as Gamebryo writes it).
pub const INVALID_POSE: f32 = f32::MIN;

/// `NiTransformData` (a.k.a. `NiKeyframeData`).
///
/// ```text
/// u32 rotation key count
/// u32 rotation key type          if count > 0
/// QuatKey x count                if type != XYZ_ROTATION (time, w x y z [, TBC])
/// KeyGroup<f32> x 3              if type == XYZ_ROTATION (X, Y, Z angles in radians)
/// KeyGroup<Vector3> translations
/// KeyGroup<f32> scales           uniform scale
/// ```
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TransformData {
    pub rotations: Rotations,
    pub translations: KeyGroup<[f32; 3]>,
    pub scales: KeyGroup<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Rotations {
    Quaternion(KeyGroup<Quaternion>),
    /// Separate X, Y and Z Euler angle curves (radians).
    Euler(Box<[KeyGroup<f32>; 3]>),
}

impl Default for Rotations {
    fn default() -> Self {
        Self::Quaternion(KeyGroup::default())
    }
}

/// An annotation at `time` seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct TextKey<'a> {
    pub time: f32,
    pub text: Cow<'a, str>,
}
