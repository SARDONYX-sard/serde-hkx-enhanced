//! KF writer.
//!
//! The output matches niflib byte for byte (block order, string table order,
//! header values), so files stay interchangeable with tools built on it.

use std::borrow::Cow;
use std::collections::HashMap;

use super::key::{KeyType, KeyValue as _, write_group};
use super::{Rotations, Sequence, TransformData, TransformInterpolator};
use crate::header::{Header, write_roots};
use crate::writer::Writer;

/// niflib's KF header values (Skyrim LE stream).
const USER_VERSION: u32 = 11;
const BS_VERSION: u32 = 83;

impl Sequence<'_> {
    /// Writes a KF file with this sequence as its only root.
    ///
    /// Blocks are laid out as niflib does: the sequence, then an interpolator
    /// followed by its data for each controlled block, then the text keys.
    pub fn to_bytes(&self) -> Vec<u8> {
        // Block indices have to be known before the sequence that refers to them is written.
        let mut next = 1;
        let mut take = || {
            next += 1;
            next - 1
        };
        let refs: Vec<_> = self
            .controlled_blocks
            .iter()
            .map(|block| {
                block.interpolator.as_ref().map(|interpolator| {
                    let interpolator_ref = take();
                    (interpolator_ref, interpolator.data.as_ref().map(|_| take()))
                })
            })
            .collect();
        let text_keys_ref = (!self.text_keys.is_empty()).then(&mut take);

        let mut strings = StringTable::default();
        let mut blocks = Vec::new();

        let mut w = Writer::default();
        self.write_sequence(&mut w, &mut strings, &refs, text_keys_ref);
        blocks.push(("NiControllerSequence", w));

        for (block, refs) in self.controlled_blocks.iter().zip(&refs) {
            let (Some(interpolator), Some((_, data_ref))) = (&block.interpolator, refs) else {
                continue;
            };
            let mut w = Writer::default();
            write_interpolator(&mut w, interpolator, *data_ref);
            blocks.push(("NiTransformInterpolator", w));
            if let Some(data) = &interpolator.data {
                let mut w = Writer::default();
                write_transform_data(&mut w, data);
                blocks.push(("NiTransformData", w));
            }
        }

        if text_keys_ref.is_some() {
            // NiTextKeyExtraData: i32 name string, u32 count, (f32 time, i32 string) x count
            let mut w = Writer::default();
            w.index(None);
            w.u32(self.text_keys.len() as u32);
            for key in &self.text_keys {
                w.f32(key.time);
                w.index(strings.intern(&key.text));
            }
            blocks.push(("NiTextKeyExtraData", w));
        }

        let header = Header {
            user_version: USER_VERSION,
            bs_version: BS_VERSION,
            block_types: blocks
                .iter()
                .map(|(name, _)| Cow::Borrowed(*name))
                .collect(),
            block_sizes: blocks.iter().map(|(_, w)| w.buf.len() as u32).collect(),
            strings: strings.strings.into_iter().map(Cow::Borrowed).collect(),
        };
        let mut out = Writer::default();
        header.write(&mut out);
        for (_, block) in &blocks {
            out.bytes(&block.buf);
        }
        write_roots(&mut out, &[0]);
        out.buf
    }

    /// Layout: see `read_sequence` in the reader.
    fn write_sequence<'s>(
        &'s self,
        w: &mut Writer,
        strings: &mut StringTable<'s>,
        refs: &[Option<(usize, Option<usize>)>],
        text_keys_ref: Option<usize>,
    ) {
        w.index(strings.intern(&self.name));
        w.u32(self.controlled_blocks.len() as u32);
        w.u32(0); // array grow by
        for (block, refs) in self.controlled_blocks.iter().zip(refs) {
            w.index(refs.map(|(interpolator, _)| interpolator));
            w.index(None); // controller
            w.u8(block.priority);
            w.index(strings.intern(&block.node_name));
            for _ in 0..4 {
                w.index(None); // property type, controller type, controller id, interpolator id
            }
        }
        w.f32(self.weight);
        w.index(text_keys_ref);
        w.u32(self.cycle_type as u32);
        w.f32(self.frequency);
        w.f32(self.start_time);
        w.f32(self.stop_time);
        w.index(None); // manager
        w.index(
            self.accum_root_name
                .as_deref()
                .and_then(|name| strings.intern(name)),
        );
        w.u16(0); // BSAnimNotes arrays (BS version > 28)
    }
}

fn write_interpolator(
    w: &mut Writer,
    interpolator: &TransformInterpolator,
    data_ref: Option<usize>,
) {
    w.vec3(interpolator.translation);
    interpolator.rotation.write(w);
    w.f32(interpolator.scale);
    w.index(data_ref);
}

fn write_transform_data(w: &mut Writer, data: &TransformData) {
    match &data.rotations {
        Rotations::Quaternion(group) => write_group(w, group),
        Rotations::Euler(axes) => {
            w.u32(1); // the shared count only has to be non-zero for the type to follow
            w.u32(KeyType::XyzRotation as u32);
            axes.iter().for_each(|axis| write_group(w, axis));
        }
    }
    write_group(w, &data.translations);
    write_group(w, &data.scales);
}

/// Header string table in first-use order; empty strings are written as `-1`.
#[derive(Default)]
struct StringTable<'s> {
    strings: Vec<&'s str>,
    indices: HashMap<&'s str, usize>,
}

impl<'s> StringTable<'s> {
    fn intern(&mut self, string: &'s str) -> Option<usize> {
        if string.is_empty() {
            return None;
        }
        Some(*self.indices.entry(string).or_insert_with(|| {
            self.strings.push(string);
            self.strings.len() - 1
        }))
    }
}
