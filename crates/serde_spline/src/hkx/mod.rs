pub mod de;
mod decomposer_from;
pub mod ser;

use havok_types::QsTransform;

#[derive(Clone, Debug, PartialEq)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bone {
    pub name: String,
    pub parent_index: i16,
    pub reference_pose: QsTransform,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Animation {
    pub duration: f32,
    pub num_frames: u32,
    pub num_tracks: u32,
    pub frames: Vec<AnimationFrame>,
    pub annotations: Vec<AnimationAnnotation>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnimationFrame {
    pub transforms: Vec<QsTransform>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnimationAnnotation {
    pub time: f32,
    pub text: String,
    pub track_index: u32,
}
