pub mod de;
mod decomposer_from;
pub mod ser;

use havok_types::QsTransform;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Bone {
    pub name: String,
    /// An index representing a tree-like structure. The tree root is -1
    pub parent_index: i16,
    pub reference_pose: QsTransform,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Animation {
    pub duration: f32,
    pub num_frames: u32,
    pub num_tracks: u32,
    pub frames: Vec<AnimationFrame>,
    pub annotations: Vec<AnimationAnnotation>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AnimationFrame {
    pub transforms: Vec<QsTransform>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AnimationAnnotation {
    pub track_index: u32,

    pub time: f32,
    pub text: String,
}
