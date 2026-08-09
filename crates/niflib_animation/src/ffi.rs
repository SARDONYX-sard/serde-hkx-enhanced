#[allow(clippy::module_inception, clippy::missing_errors_doc)]
#[cxx::bridge(namespace = "niflib_animation")]
mod ffi {
    #[derive(Clone, Debug)]
    pub struct Vec4 {
        pub x: f32,
        pub y: f32,
        pub z: f32,
        pub w: f32,
    }

    #[derive(Clone, Debug)]
    pub struct Quaternion {
        pub x: f32,
        pub y: f32,
        pub z: f32,
        pub w: f32,
    }

    #[derive(Clone, Debug)]
    pub struct Transform {
        pub translation: Vec4,
        pub rotation: Quaternion,
        pub scale: Vec4,
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

    #[derive(Clone, Debug)]
    pub struct Kf {
        pub skeleton: Skeleton,
        pub animation: Animation,
    }

    unsafe extern "C++" {
        include!("niflib_animation.h");

        fn export_kf(input: &Kf) -> Result<Vec<u8>>;
        /// - input: kf file bytes
        ///
        /// NOTE: C++, no touch `Animation.annotation`
        fn convert_kf(input: &[u8], skeleton: &Skeleton, fps: f32) -> Result<Animation>;
    }
}

pub(crate) use self::ffi::*;
