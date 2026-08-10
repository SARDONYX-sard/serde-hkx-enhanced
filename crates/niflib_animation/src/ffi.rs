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

impl From<havok_types::QsTransform> for Transform {
    fn from(value: havok_types::QsTransform) -> Self {
        Self {
            translation: value.transition.into(),
            rotation: value.quaternion.into(),
            scale: value.scale.into(),
        }
    }
}

impl From<havok_types::Vector4> for Vec4 {
    fn from(value: havok_types::Vector4) -> Self {
        Self {
            x: value.x,
            y: value.y,
            z: value.z,
            w: value.w,
        }
    }
}

impl From<havok_types::Quaternion> for Quaternion {
    fn from(value: havok_types::Quaternion) -> Self {
        Self {
            x: value.x,
            y: value.y,
            z: value.z,
            w: value.scaler,
        }
    }
}

impl From<serde_spline::hkx::Bone> for Bone {
    fn from(value: serde_spline::hkx::Bone) -> Self {
        Self {
            name: value.name,
            parent_index: value.parent_index,
            reference_pose: value.reference_pose.into(),
        }
    }
}

impl From<&serde_spline::hkx::Skeleton> for Skeleton {
    fn from(value: &serde_spline::hkx::Skeleton) -> Self {
        Self {
            bones: value.bones.iter().map(|bone| bone.clone().into()).collect(),
        }
    }
}

impl From<serde_spline::hkx::AnimationAnnotation> for AnimationAnnotation {
    fn from(value: serde_spline::hkx::AnimationAnnotation) -> Self {
        Self {
            time: value.time,
            text: value.text,
            track_index: value.track_index,
        }
    }
}

impl From<serde_spline::hkx::AnimationFrame> for AnimationFrame {
    fn from(value: serde_spline::hkx::AnimationFrame) -> Self {
        Self {
            transforms: value.transforms.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<serde_spline::hkx::Animation> for Animation {
    fn from(value: serde_spline::hkx::Animation) -> Self {
        Self {
            duration: value.duration,
            num_frames: value.num_frames,
            num_tracks: value.num_tracks,
            frames: value.frames.into_iter().map(Into::into).collect(),
            annotations: value.annotations.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<Animation> for serde_spline::hkx::Animation {
    fn from(value: Animation) -> Self {
        Self {
            duration: value.duration,
            num_frames: value.num_frames,
            num_tracks: value.num_tracks,
            frames: value.frames.into_iter().map(Into::into).collect(),
            annotations: value.annotations.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<AnimationFrame> for serde_spline::hkx::AnimationFrame {
    fn from(value: AnimationFrame) -> Self {
        Self {
            transforms: value.transforms.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<AnimationAnnotation> for serde_spline::hkx::AnimationAnnotation {
    fn from(value: AnimationAnnotation) -> Self {
        Self {
            time: value.time,
            text: value.text,
            track_index: value.track_index,
        }
    }
}

impl From<Transform> for havok_types::QsTransform {
    fn from(value: Transform) -> Self {
        Self {
            transition: value.translation.into(),
            quaternion: value.rotation.into(),
            scale: value.scale.into(),
        }
    }
}

impl From<Vec4> for havok_types::Vector4 {
    fn from(value: Vec4) -> Self {
        Self {
            x: value.x,
            y: value.y,
            z: value.z,
            w: value.w,
        }
    }
}

impl From<Quaternion> for havok_types::Quaternion {
    fn from(value: Quaternion) -> Self {
        Self {
            x: value.x,
            y: value.y,
            z: value.z,
            scaler: value.w,
        }
    }
}
