//! Binary FBX reader (format: see `crate::ser` binary encoder docs).
//!
//! Version 7500+ widens the three node header fields to `u64`; everything
//! else is identical, so one reader handles both.

use std::borrow::Cow;

use super::element::{Element, Value, load_error};
use crate::Error;

pub const MAGIC: &[u8] = b"Kaydara FBX Binary  \x00";

/// Magic (21 bytes) + `\x1a\x00` before the version.
const HEADER_LEN: usize = 23;

/// Parses a binary FBX file into its top-level nodes.
///
/// # Errors
/// If the data is truncated or uses an unknown property type.
pub fn parse(data: &[u8]) -> Result<Vec<Element<'_>>, Error> {
    let mut reader = Reader {
        data,
        pos: HEADER_LEN,
        wide: false,
    };
    reader.wide = reader.u32()? >= 7500;

    let mut nodes = Vec::new();
    while let Some(node) = reader.node()? {
        nodes.push(node);
    }
    Ok(nodes)
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    /// 64-bit node header fields (version >= 7500).
    wide: bool,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], Error> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|&end| end <= self.data.len())
            .ok_or_else(|| load_error(format!("unexpected end of file at {:#x}", self.pos)))?;
        let bytes = &self.data[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let mut array = [0; N];
        array.copy_from_slice(self.take(N)?);
        Ok(array)
    }

    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, Error> {
        self.array().map(u32::from_le_bytes)
    }

    fn offset(&mut self) -> Result<u64, Error> {
        if self.wide {
            self.array().map(u64::from_le_bytes)
        } else {
            self.u32().map(u64::from)
        }
    }

    /// Reads one node, or `None` for the null record that ends a child list.
    fn node(&mut self) -> Result<Option<Element<'a>>, Error> {
        if self.pos >= self.data.len() {
            return Ok(None);
        }

        let end = self.offset()? as usize;
        let num_props = self.offset()?;
        let _props_len = self.offset()?;
        let name_len = usize::from(self.u8()?);
        if end == 0 {
            return Ok(None);
        }
        if end > self.data.len() {
            return Err(load_error(format!("node at {:#x} exceeds the file", self.pos)));
        }

        let name = String::from_utf8_lossy(self.take(name_len)?);
        // `num_props` comes from the file: cap the preallocation.
        let mut props = Vec::with_capacity(num_props.min(64) as usize);
        for _ in 0..num_props {
            props.push(self.value()?);
        }

        let mut children = Vec::new();
        while self.pos < end {
            match self.node()? {
                Some(child) => children.push(child),
                None => break,
            }
        }
        self.pos = end;

        Ok(Some(Element { name, props, children }))
    }

    fn value(&mut self) -> Result<Value<'a>, Error> {
        fn le<const N: usize>(b: &[u8]) -> [u8; N] {
            let mut array = [0; N];
            array.copy_from_slice(&b[..N]);
            array
        }

        Ok(match self.u8()? {
            b'C' => Value::Bool(self.u8()? != 0),
            b'Y' => Value::I64(i64::from(i16::from_le_bytes(self.array()?))),
            b'I' => Value::I64(i64::from(i32::from_le_bytes(self.array()?))),
            b'L' => Value::I64(i64::from_le_bytes(self.array()?)),
            b'F' => Value::F64(f64::from(f32::from_le_bytes(self.array()?))),
            b'D' => Value::F64(f64::from_le_bytes(self.array()?)),
            b'S' => {
                let len = self.u32()? as usize;
                Value::String(String::from_utf8_lossy(self.take(len)?))
            }
            b'R' => {
                let len = self.u32()? as usize;
                Value::Raw(self.take(len)?)
            }
            b'b' => Value::I64Array(self.array_values(1, |b| i64::from(b[0]))?),
            b'i' => Value::I64Array(self.array_values(4, |b| i64::from(i32::from_le_bytes(le(b))))?),
            b'l' => Value::I64Array(self.array_values(8, |b| i64::from_le_bytes(le(b)))?),
            b'f' => Value::F64Array(self.array_values(4, |b| f64::from(f32::from_le_bytes(le(b))))?),
            b'd' => Value::F64Array(self.array_values(8, |b| f64::from_le_bytes(le(b)))?),
            code => {
                return Err(load_error(format!(
                    "unknown property type {:?} at {:#x}",
                    code as char,
                    self.pos - 1
                )));
            }
        })
    }

    /// `u32 count, u32 encoding (0 raw / 1 zlib), u32 byte length, data`.
    fn array_values<T>(&mut self, stride: usize, decode: impl Fn(&[u8]) -> T) -> Result<Vec<T>, Error> {
        let count = self.u32()? as usize;
        let encoding = self.u32()?;
        let len = self.u32()? as usize;
        let raw = self.take(len)?;

        let bytes: Cow<'_, [u8]> = match encoding {
            0 => Cow::Borrowed(raw),
            1 => Cow::Owned(
                miniz_oxide::inflate::decompress_to_vec_zlib(raw)
                    .map_err(|error| load_error(format!("failed to inflate array: {error:?}")))?,
            ),
            _ => return Err(load_error(format!("unknown array encoding {encoding}"))),
        };
        if bytes.len() < count.saturating_mul(stride) {
            return Err(load_error("array data is shorter than its element count"));
        }
        Ok(bytes.chunks_exact(stride).take(count).map(decode).collect())
    }
}
