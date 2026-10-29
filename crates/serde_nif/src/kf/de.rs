//! KF reader.

use std::borrow::Cow;

use super::key::{KeyGroup, KeyType, KeyValue as _, Quaternion, read_group, read_keys};
use super::{
    ControlledBlock, CycleType, Rotations, Sequence, TextKey, TransformData, TransformInterpolator,
};
use crate::error::{Error, InvalidDataSnafu};
use crate::header::{Header, read_roots};
use crate::reader::Reader;

/// Decoded blocks, still referring to each other and to the string table by index.
enum Block {
    Sequence(RawSequence),
    Interpolator {
        translation: [f32; 3],
        rotation: Quaternion,
        scale: f32,
        data: Option<usize>,
    },
    Data(TransformData),
    TextKeys(Vec<(f32, Option<usize>)>),
    Other,
}

struct RawSequence {
    name: Option<usize>,
    /// `(interpolator, priority, node name)`
    links: Vec<(Option<usize>, u8, Option<usize>)>,
    weight: f32,
    text_keys: Option<usize>,
    cycle_type: CycleType,
    frequency: f32,
    start_time: f32,
    stop_time: f32,
    accum_root_name: Option<usize>,
}

impl<'a> Sequence<'a> {
    /// Reads the single `NiControllerSequence` of a KF file. Strings borrow `bytes`.
    ///
    /// # Errors
    /// If the file is not a 20.2.0.7 NIF, a block is malformed, or the file
    /// does not contain exactly one sequence.
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, Error> {
        let mut reader = Reader::new(bytes);
        let header = Header::read(&mut reader)?;
        let blocks = header.read_blocks(&mut reader, read_block)?;
        let _roots = read_roots(&mut reader);

        let mut sequences = blocks.iter().filter_map(|block| match block {
            Block::Sequence(sequence) => Some(sequence),
            _ => None,
        });
        let (Some(sequence), None) = (sequences.next(), sequences.next()) else {
            return InvalidDataSnafu {
                message: "a KF file must contain exactly one NiControllerSequence".to_owned(),
            }
            .fail();
        };

        let string = |index: Option<usize>| index.and_then(|i| header.strings.get(i)).cloned();
        let controlled_blocks = sequence
            .links
            .iter()
            .map(|&(interpolator, priority, node_name)| ControlledBlock {
                node_name: string(node_name).unwrap_or_default(),
                priority,
                interpolator: resolve_interpolator(&blocks, interpolator),
            })
            .collect();
        let text_keys = match sequence.text_keys.and_then(|i| blocks.get(i)) {
            Some(Block::TextKeys(keys)) => keys
                .iter()
                .map(|&(time, text)| TextKey {
                    time,
                    text: string(text).unwrap_or_default(),
                })
                .collect(),
            _ => Vec::new(),
        };

        Ok(Self {
            name: string(sequence.name).unwrap_or(Cow::Borrowed("")),
            weight: sequence.weight,
            cycle_type: sequence.cycle_type,
            frequency: sequence.frequency,
            start_time: sequence.start_time,
            stop_time: sequence.stop_time,
            accum_root_name: string(sequence.accum_root_name),
            controlled_blocks,
            text_keys,
        })
    }
}

fn resolve_interpolator(blocks: &[Block], index: Option<usize>) -> Option<TransformInterpolator> {
    let Some(&Block::Interpolator {
        translation,
        rotation,
        scale,
        data,
    }) = index.and_then(|i| blocks.get(i))
    else {
        return None;
    };
    // Cloned rather than moved: one data block may be shared by several interpolators.
    let data = match data.and_then(|i| blocks.get(i)) {
        Some(Block::Data(data)) => Some(data.clone()),
        _ => None,
    };
    Some(TransformInterpolator {
        translation,
        rotation,
        scale,
        data,
    })
}

fn read_block(r: &mut Reader<'_>, block_type: &str) -> Result<Block, Error> {
    Ok(match block_type {
        "NiControllerSequence" => Block::Sequence(read_sequence(r)?),
        // NiTransformInterpolator: NiQuatTransform pose, then the data ref.
        "NiTransformInterpolator" => Block::Interpolator {
            translation: r.vec3()?,
            rotation: Quaternion::read(r)?,
            scale: r.f32()?,
            data: r.index()?,
        },
        "NiTransformData" | "NiKeyframeData" => Block::Data(read_transform_data(r)?),
        // NiTextKeyExtraData: i32 name string, u32 count, (f32 time, i32 string) x count
        "NiTextKeyExtraData" => {
            r.index()?; // name
            let count = r.u32()? as usize;
            r.check_len(count, 8)?;
            Block::TextKeys(
                (0..count)
                    .map(|_| Ok((r.f32()?, r.index()?)))
                    .collect::<Result<_, Error>>()?,
            )
        }
        _ => Block::Other,
    })
}

/// `NiSequence` + `NiControllerSequence` (20.2.0.7, Bethesda).
///
/// ```text
/// i32 string  name
/// u32         controlled block count
/// u32         array grow by                    (allocation hint)
/// per controlled block:
///   i32 ref     interpolator                   NiTransformInterpolator, ...
///   i32 ref     controller                     usually -1 in KF files
///   u8          priority
///   i32 string  node name, property type, controller type, controller id, interpolator id
/// f32         weight
/// i32 ref     text keys                        NiTextKeyExtraData
/// u32         cycle type                       0 loop, 1 reverse, 2 clamp
/// f32         frequency, start time, stop time
/// i32 ref     manager                          NiControllerManager (-1 in KF files)
/// i32 string  accum root name
/// u16 count, i32 refs  BSAnimNotes             BS version > 28 (not read)
/// ```
fn read_sequence(r: &mut Reader<'_>) -> Result<RawSequence, Error> {
    let name = r.index()?;
    let count = r.u32()? as usize;
    let _array_grow_by = r.u32()?;
    r.check_len(count, 29)?;
    let links = (0..count)
        .map(|_| {
            let interpolator = r.index()?;
            let _controller = r.index()?;
            let priority = r.u8()?;
            let node_name = r.index()?;
            r.skip(16)?; // property type, controller type, controller id, interpolator id
            Ok((interpolator, priority, node_name))
        })
        .collect::<Result<_, Error>>()?;

    let weight = r.f32()?;
    let text_keys = r.index()?;
    let cycle_type = match r.u32()? {
        0 => CycleType::Loop,
        1 => CycleType::Reverse,
        2 => CycleType::Clamp,
        other => {
            return InvalidDataSnafu {
                message: format!("unknown cycle type {other}"),
            }
            .fail();
        }
    };
    let frequency = r.f32()?;
    let start_time = r.f32()?;
    let stop_time = r.f32()?;
    let _manager = r.index()?;
    let accum_root_name = r.index()?;

    Ok(RawSequence {
        name,
        links,
        weight,
        text_keys,
        cycle_type,
        frequency,
        start_time,
        stop_time,
        accum_root_name,
    })
}

/// See [`TransformData`] for the layout.
fn read_transform_data(r: &mut Reader<'_>) -> Result<TransformData, Error> {
    let count = r.u32()? as usize;
    let rotations = if count == 0 {
        Rotations::default()
    } else {
        // The count/type header is shared by both forms; for Euler rotations
        // the count is meaningless (usually 1) and the axes carry their own.
        match KeyType::try_from(r.u32()?)? {
            KeyType::XyzRotation => {
                Rotations::Euler(Box::new([read_group(r)?, read_group(r)?, read_group(r)?]))
            }
            key_type => Rotations::Quaternion(KeyGroup {
                key_type,
                keys: read_keys(r, count, key_type)?,
            }),
        }
    };

    Ok(TransformData {
        rotations,
        translations: read_group(r)?,
        scales: read_group(r)?,
    })
}
