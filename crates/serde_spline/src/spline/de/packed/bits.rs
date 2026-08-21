// SPDX-License-Identifier: GPL-3.0-or-later

//! Compile-time descriptions of packed bit fields.

/// A compile-time description of a contiguous bit field.
///
/// `OFFSET` specifies the least-significant bit of the field and `WIDTH`
/// specifies the number of bits occupied by the field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Field<const OFFSET: u8, const WIDTH: u8>;

impl<const OFFSET: u8, const WIDTH: u8> Field<OFFSET, WIDTH> {
    /// Creates a field descriptor.
    ///
    /// # Panics
    ///
    /// Panics during constant evaluation if the field does not fit inside
    /// a 32-bit integer.
    #[inline]
    pub const fn new() -> Self {
        const {
            assert!(WIDTH > 0);
            assert!(WIDTH <= 32);
            assert!(OFFSET < 32);
            assert!((OFFSET as u16 + WIDTH as u16) <= 32);
        }
        Self
    }

    /// Extracts this field from a `u32`.
    #[allow(clippy::unused_self)]
    #[inline]
    pub const fn extract(self, value: u32) -> u32 {
        (value >> OFFSET) & Self::value_mask()
    }

    /// Returns the mask covering this field before shifting.
    #[inline]
    const fn value_mask() -> u32 {
        if WIDTH == 32 {
            u32::MAX
        } else {
            (1u32 << WIDTH) - 1
        }
    }
}
