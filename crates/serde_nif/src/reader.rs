//! Little-endian cursor over NIF bytes.

use crate::error::{Error, UnexpectedEofSnafu};

pub struct Reader<'a> {
    data: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub const fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn take(&mut self, len: usize) -> Result<&'a [u8], Error> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|&end| end <= self.data.len());
        let Some(end) = end else {
            return UnexpectedEofSnafu {
                offset: self.pos,
                len,
            }
            .fail();
        };
        let bytes = &self.data[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    pub fn skip(&mut self, len: usize) -> Result<(), Error> {
        self.take(len).map(|_| ())
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let mut array = [0; N];
        array.copy_from_slice(self.take(N)?);
        Ok(array)
    }

    pub fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }

    pub fn bool(&mut self) -> Result<bool, Error> {
        Ok(self.u8()? != 0)
    }

    pub fn u16(&mut self) -> Result<u16, Error> {
        self.array().map(u16::from_le_bytes)
    }

    pub fn u32(&mut self) -> Result<u32, Error> {
        self.array().map(u32::from_le_bytes)
    }

    pub fn i32(&mut self) -> Result<i32, Error> {
        self.array().map(i32::from_le_bytes)
    }

    pub fn u64(&mut self) -> Result<u64, Error> {
        self.array().map(u64::from_le_bytes)
    }

    pub fn f32(&mut self) -> Result<f32, Error> {
        self.array().map(f32::from_le_bytes)
    }

    /// IEEE 754 half precision.
    pub fn f16(&mut self) -> Result<f32, Error> {
        self.u16().map(half_to_f32)
    }

    pub fn vec2(&mut self) -> Result<[f32; 2], Error> {
        Ok([self.f32()?, self.f32()?])
    }

    pub fn vec3(&mut self) -> Result<[f32; 3], Error> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }

    pub fn matrix33(&mut self) -> Result<[[f32; 3]; 3], Error> {
        Ok([self.vec3()?, self.vec3()?, self.vec3()?])
    }

    /// Block reference or string index; `-1` is none.
    pub fn index(&mut self) -> Result<Option<usize>, Error> {
        Ok(usize::try_from(self.i32()?).ok())
    }

    /// `u32 count` followed by `count` references.
    pub fn index_list(&mut self) -> Result<Vec<Option<usize>>, Error> {
        let count = self.u32()? as usize;
        self.check_len(count, 4)?;
        (0..count).map(|_| self.index()).collect()
    }

    /// `u8 length` + bytes (the length includes a trailing NUL).
    pub fn short_string(&mut self) -> Result<String, Error> {
        let len = self.u8()? as usize;
        Ok(trim_nul(self.take(len)?))
    }

    /// `u32 length` + bytes.
    pub fn sized_string(&mut self) -> Result<String, Error> {
        let len = self.u32()? as usize;
        Ok(trim_nul(self.take(len)?))
    }

    /// Fails early when `count` elements of `size` bytes cannot fit.
    pub fn check_len(&self, count: usize, size: usize) -> Result<(), Error> {
        let len = count.saturating_mul(size);
        if self.data.len().saturating_sub(self.pos) < len {
            return UnexpectedEofSnafu {
                offset: self.pos,
                len,
            }
            .fail();
        }
        Ok(())
    }
}

fn trim_nul(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn half_to_f32(half: u16) -> f32 {
    let sign = u32::from(half >> 15) << 31;
    let exponent = u32::from((half >> 10) & 0x1f);
    let mantissa = u32::from(half & 0x3ff);

    let bits = match (exponent, mantissa) {
        (0, 0) => sign,
        (0, _) => {
            // Subnormal: normalize the mantissa.
            let shift = mantissa.leading_zeros() - 21;
            let mantissa = (mantissa << shift) & 0x3ff;
            sign | ((113 - shift) << 23) | (mantissa << 13)
        }
        (0x1f, _) => sign | 0x7f80_0000 | (mantissa << 13),
        _ => sign | ((exponent + 112) << 23) | (mantissa << 13),
    };
    f32::from_bits(bits)
}

#[cfg(test)]
mod tests {
    use super::half_to_f32;

    #[test]
    fn half_floats() {
        assert_eq!(half_to_f32(0x3c00), 1.0);
        assert_eq!(half_to_f32(0xc000), -2.0);
        assert_eq!(half_to_f32(0x3555), 0.333_251_95);
        assert_eq!(half_to_f32(0x0001), 5.960_464_5e-8);
    }
}
