#[derive(Clone, Debug, Default)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Quaternion {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Transform {
    pub translation: Vec4,
    pub rotation: Quaternion,
    pub scale: Vec4,
}

impl Transform {
    pub const fn identity() -> Self {
        Self {
            translation: Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 0.0,
            },
            rotation: Quaternion {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            },
            scale: Vec4 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
                w: 1.0,
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct Bone {
    pub name: String,
    pub parent_index: i16,
    pub reference_pose: Transform,
}

#[derive(Clone, Debug)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
}

#[derive(Clone, Debug)]
pub struct AnimationAnnotation {
    pub time: f32,
    pub text: String,
    pub track_index: u32,
}

#[derive(Clone, Debug)]
pub struct AnimationFrame {
    pub transforms: Vec<Transform>,
}

#[derive(Clone, Debug)]
pub struct Animation {
    pub duration: f32,
    pub num_frames: u32,
    pub num_tracks: u32,
    pub frames: Vec<AnimationFrame>,
    pub annotations: Vec<AnimationAnnotation>,
}
