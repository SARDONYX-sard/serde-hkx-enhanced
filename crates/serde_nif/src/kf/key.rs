//! Animation keys (`Key<T>`, `KeyGroup<T>`, `QuatKey`).
//!
//! ```text
//! KeyGroup<T>:
//!   u32 key count
//!   u32 key type      only when count > 0 (see `KeyType`)
//!   keys:
//!     f32 time        seconds
//!     T   value
//!     T   forward, T backward   if type == QUADRATIC (Hermite tangents; never for quaternions)
//!     f32 tension, bias, continuity   if type == TBC
//! ```

use crate::error::{Error, InvalidDataSnafu};
use crate::reader::Reader;
use crate::writer::Writer;

/// How values between two keys are interpolated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum KeyType {
    #[default]
    Linear = 1,
    /// Hermite with explicit tangents; for quaternions, squad without tangents.
    Quadratic = 2,
    /// Kochanek–Bartels (tension, bias, continuity).
    Tbc = 3,
    /// Rotation only: three separate float groups (X, Y, Z Euler angles) instead of quaternions.
    XyzRotation = 4,
    /// Step: hold the previous key.
    Constant = 5,
}

impl TryFrom<u32> for KeyType {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self, Error> {
        Ok(match value {
            1 => Self::Linear,
            2 => Self::Quadratic,
            3 => Self::Tbc,
            4 => Self::XyzRotation,
            5 => Self::Constant,
            _ => {
                return InvalidDataSnafu {
                    message: format!("unknown key type {value}"),
                }
                .fail();
            }
        })
    }
}

/// A rotation, stored `w, x, y, z` in the file and `x, y, z, w` here (Havok order).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quaternion {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for Quaternion {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        }
    }
}

/// One key. `forward`/`backward` are only stored for [`KeyType::Quadratic`]
/// (not for quaternions) and `tbc` only for [`KeyType::Tbc`]; otherwise they
/// keep their defaults.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Key<T> {
    pub time: f32,
    pub value: T,
    pub forward: T,
    pub backward: T,
    pub tbc: [f32; 3],
}

impl<T: Default> Key<T> {
    pub fn new(time: f32, value: T) -> Self {
        Self {
            time,
            value,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct KeyGroup<T> {
    pub key_type: KeyType,
    /// Sorted by time.
    pub keys: Vec<Key<T>>,
}

/// A value that keys can hold (encoding is internal to the crate).
pub(crate) trait KeyValue: Copy + Default {
    /// Size in bytes, to validate key counts before allocating.
    const SIZE: usize;
    /// Quaternion keys never store tangents, even when quadratic.
    const HAS_TANGENTS: bool = true;

    fn read(r: &mut Reader<'_>) -> Result<Self, Error>;
    fn write(self, w: &mut Writer);
}

impl KeyValue for f32 {
    const SIZE: usize = 4;

    fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        r.f32()
    }

    fn write(self, w: &mut Writer) {
        w.f32(self);
    }
}

impl KeyValue for [f32; 3] {
    const SIZE: usize = 12;

    fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        r.vec3()
    }

    fn write(self, w: &mut Writer) {
        w.vec3(self);
    }
}

impl KeyValue for Quaternion {
    const SIZE: usize = 16;
    const HAS_TANGENTS: bool = false;

    fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        let w = r.f32()?;
        Ok(Self {
            x: r.f32()?,
            y: r.f32()?,
            z: r.f32()?,
            w,
        })
    }

    fn write(self, w: &mut Writer) {
        for v in [self.w, self.x, self.y, self.z] {
            w.f32(v);
        }
    }
}

// Free functions rather than `KeyGroup` methods: an impl on the public type
// must not carry the crate-private `KeyValue` bound.
pub(crate) fn read_group<T: KeyValue>(r: &mut Reader<'_>) -> Result<KeyGroup<T>, Error> {
    let count = r.u32()? as usize;
    if count == 0 {
        return Ok(KeyGroup::default());
    }
    let key_type = KeyType::try_from(r.u32()?)?;
    Ok(KeyGroup {
        key_type,
        keys: read_keys(r, count, key_type)?,
    })
}

pub(crate) fn write_group<T: KeyValue>(w: &mut Writer, group: &KeyGroup<T>) {
    w.u32(group.keys.len() as u32);
    if group.keys.is_empty() {
        return;
    }
    w.u32(group.key_type as u32);
    write_keys(w, &group.keys, group.key_type);
}

/// Keys whose count and type were read by the caller (rotations share the
/// count/type header with the Euler form, see `TransformData`).
pub(crate) fn read_keys<T: KeyValue>(
    r: &mut Reader<'_>,
    count: usize,
    key_type: KeyType,
) -> Result<Vec<Key<T>>, Error> {
    r.check_len(count, 4 + T::SIZE)?;
    (0..count)
        .map(|_| {
            let mut key = Key::new(r.f32()?, T::read(r)?);
            match key_type {
                KeyType::Quadratic if T::HAS_TANGENTS => {
                    key.forward = T::read(r)?;
                    key.backward = T::read(r)?;
                }
                KeyType::Tbc => key.tbc = [r.f32()?, r.f32()?, r.f32()?],
                _ => {}
            }
            Ok(key)
        })
        .collect()
}

pub(crate) fn write_keys<T: KeyValue>(w: &mut Writer, keys: &[Key<T>], key_type: KeyType) {
    for key in keys {
        w.f32(key.time);
        key.value.write(w);
        match key_type {
            KeyType::Quadratic if T::HAS_TANGENTS => {
                key.forward.write(w);
                key.backward.write(w);
            }
            KeyType::Tbc => key.tbc.into_iter().for_each(|v| w.f32(v)),
            _ => {}
        }
    }
}
