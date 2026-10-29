// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: (C) 2016-2023 Lukas Cone
//
// Reference:
// - https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.cpp

//! Byte cursor over one spline block, carrying context for error messages.

use crate::error::{Error, ReadContext};

pub(super) struct Reader<'a> {
    pub(super) data: &'a [u8],
    pub(super) position: usize,
    // For error reporting detail info
    pub(super) context: ReadContext,
}

impl<'a> Reader<'a> {
    pub(super) fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            position: 0,
            context: ReadContext::default(),
        }
    }

    const fn unexpected_eof(&self, requested: usize) -> Error {
        Error::UnexpectedEof {
            position: self.position,
            requested,
            remaining: self.data.len().saturating_sub(self.position),
            context: self.context,
        }
    }

    pub(super) fn read_u8(&mut self) -> Result<u8, Error> {
        #[cfg(feature = "tracing")]
        tracing::trace!(
            position = format_args!("{:#06x}", self.position),
            "enter read_u8"
        );
        let value = *self
            .data
            .get(self.position)
            .ok_or_else(|| self.unexpected_eof(1))?;

        self.position += 1;
        Ok(value)
    }

    pub(super) fn read_u16_le(&mut self) -> Result<u16, Error> {
        #[cfg(feature = "tracing")]
        tracing::trace!(
            position = format_args!("{:#06x}", self.position),
            "enter read_u16_le"
        );

        let bytes = self.read_array::<2>()?;
        Ok(u16::from_le_bytes(bytes))
    }

    pub(super) fn read_u32_le(&mut self) -> Result<u32, Error> {
        #[cfg(feature = "tracing")]
        tracing::trace!(
            position = format_args!("{:#06x}", self.position),
            "enter read_u32_le"
        );

        let bytes = self.read_array::<4>()?;
        Ok(u32::from_le_bytes(bytes))
    }

    pub(super) fn read_f32_le(&mut self) -> Result<f32, Error> {
        #[cfg(feature = "tracing")]
        tracing::trace!(
            position = format_args!("{:#06x}", self.position),
            "enter read_f32_le"
        );
        Ok(f32::from_bits(self.read_u32_le()?))
    }

    pub(super) fn read_array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        #[cfg(feature = "tracing")]
        tracing::trace!(
            position = format_args!("{:#06x}", self.position),
            size = N,
            "enter read_array"
        );

        let end = self
            .position
            .checked_add(N)
            .ok_or_else(|| self.unexpected_eof(N))?;

        let bytes = self
            .data
            .get(self.position..end)
            .ok_or_else(|| self.unexpected_eof(N))?;

        let mut result = [0u8; N];
        result.copy_from_slice(bytes);

        self.position = end;

        Ok(result)
    }

    /// Skip `<count>` bytes.
    pub(super) fn skip(&mut self, count: usize) -> Result<(), Error> {
        #[cfg(feature = "tracing")]
        tracing::trace!(
            position = format_args!("{:#06x}", self.position),
            count,
            "enter skip"
        );

        let end = self
            .position
            .checked_add(count)
            .ok_or_else(|| self.unexpected_eof(count))?;

        if end > self.data.len() {
            return Err(self.unexpected_eof(count));
        }

        self.position = end;
        Ok(())
    }

    pub(super) fn align(&mut self, alignment: usize) -> Result<(), Error> {
        #[cfg(feature = "tracing")]
        tracing::trace!(
            position = format_args!("{:#06x}", self.position),
            alignment,
            "enter align"
        );
        debug_assert!(alignment.is_power_of_two());

        let mask = alignment - 1;
        let aligned = (self.position + mask) & !mask;

        self.skip(aligned.saturating_sub(self.position))
    }

    pub(super) fn read_bytes(&mut self, count: usize) -> Result<&'a [u8], Error> {
        #[cfg(feature = "tracing")]
        tracing::trace!(
            position = format_args!("{:#06x}", self.position),
            count,
            "enter read_bytes"
        );

        let end = self
            .position
            .checked_add(count)
            .ok_or_else(|| self.unexpected_eof(count))?;

        let result = self
            .data
            .get(self.position..end)
            .ok_or_else(|| self.unexpected_eof(count))?;

        self.position = end;

        Ok(result)
    }
}
