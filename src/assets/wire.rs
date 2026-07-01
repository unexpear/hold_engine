//! Shared little-endian wire primitives for the compiled-asset format.
//!
//! Before M10.5 the envelope (`compiled.rs`) and the payloads
//! (`runtime_loader.rs`) each carried their own copy of the write
//! helpers and their own cursor, so the byte-layout invariants lived in
//! two places and could silently drift. This module is the single
//! source of truth for how a primitive is framed on disk; both callers
//! use it, and the golden-byte tests guard that the framing is
//! unchanged.
//!
//! All integers and `f32`s are little-endian. Strings are a `u32`
//! byte-length prefix + raw UTF-8. See `docs/formats/compiled-asset.md`.

use super::CompiledAssetError;

pub(crate) fn write_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn write_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn write_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn write_bool(out: &mut Vec<u8>, value: bool) {
    out.push(if value { 1 } else { 0 });
}

pub(crate) fn write_string(out: &mut Vec<u8>, value: &str) -> Result<(), CompiledAssetError> {
    let len = u32::try_from(value.len()).map_err(|_| CompiledAssetError::LengthOverflow)?;
    write_u32(out, len);
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

pub(crate) fn write_option_string(
    out: &mut Vec<u8>,
    value: Option<&str>,
) -> Result<(), CompiledAssetError> {
    write_bool(out, value.is_some());
    if let Some(value) = value {
        write_string(out, value)?;
    }
    Ok(())
}

/// Sequential little-endian reader over a byte slice with bounds
/// checking, allocation-DoS guards, and an exact-length assertion.
/// Every read is bounds-checked; a declared count/length is validated
/// against the remaining buffer before any allocation.
pub(crate) struct ByteReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    /// Bytes not yet consumed.
    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    /// Reject a declared element count that could not fit in the
    /// remaining buffer, given each element's minimum wire size. Call
    /// before `Vec::with_capacity` to close the allocation-DoS surface.
    /// Reports the element `count` (not the byte product) in the error,
    /// matching the `declared`-is-a-count contract callers rely on.
    pub(crate) fn ensure_count_bound(
        &self,
        count: usize,
        min_elem_size: usize,
    ) -> Result<(), CompiledAssetError> {
        let needed = (count as u64).saturating_mul(min_elem_size as u64);
        let remaining = self.remaining() as u64;
        if needed > remaining {
            return Err(CompiledAssetError::DeclaredLengthExceedsBuffer {
                declared: count as u64,
                remaining,
            });
        }
        Ok(())
    }

    /// Reject a decode that would need more bytes than remain. `needed`
    /// is the caller-computed total minimum wire size of the elements
    /// about to be read (use saturating arithmetic so an overflowing
    /// product still trips the guard).
    pub(crate) fn ensure_bytes_available(&self, needed: u64) -> Result<(), CompiledAssetError> {
        let remaining = self.remaining() as u64;
        if needed > remaining {
            return Err(CompiledAssetError::DeclaredLengthExceedsBuffer {
                declared: needed,
                remaining,
            });
        }
        Ok(())
    }

    /// Assert every byte has been consumed — the exact-length contract.
    pub(crate) fn expect_end(&self) -> Result<(), CompiledAssetError> {
        let extra = self.remaining() as u64;
        if extra != 0 {
            return Err(CompiledAssetError::TrailingBytes { extra });
        }
        Ok(())
    }

    pub(crate) fn read_u32(&mut self) -> Result<u32, CompiledAssetError> {
        Ok(u32::from_le_bytes(self.read_array::<4>()?))
    }

    pub(crate) fn read_u64(&mut self) -> Result<u64, CompiledAssetError> {
        Ok(u64::from_le_bytes(self.read_array::<8>()?))
    }

    pub(crate) fn read_f32(&mut self) -> Result<f32, CompiledAssetError> {
        Ok(f32::from_le_bytes(self.read_array::<4>()?))
    }

    pub(crate) fn read_bool(&mut self) -> Result<bool, CompiledAssetError> {
        Ok(self.read_array::<1>()?[0] != 0)
    }

    pub(crate) fn read_array<const N: usize>(&mut self) -> Result<[u8; N], CompiledAssetError> {
        let end = self
            .pos
            .checked_add(N)
            .ok_or(CompiledAssetError::UnexpectedEof)?;
        if end > self.bytes.len() {
            return Err(CompiledAssetError::UnexpectedEof);
        }
        let mut out = [0u8; N];
        out.copy_from_slice(&self.bytes[self.pos..end]);
        self.pos = end;
        Ok(out)
    }

    pub(crate) fn read_vec(&mut self, len: usize) -> Result<Vec<u8>, CompiledAssetError> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or(CompiledAssetError::UnexpectedEof)?;
        if end > self.bytes.len() {
            return Err(CompiledAssetError::UnexpectedEof);
        }
        let out = self.bytes[self.pos..end].to_vec();
        self.pos = end;
        Ok(out)
    }

    pub(crate) fn read_string(&mut self) -> Result<String, CompiledAssetError> {
        let len = self.read_u32()? as usize;
        let bytes = self.read_vec(len)?;
        String::from_utf8(bytes).map_err(|_| CompiledAssetError::Utf8)
    }

    pub(crate) fn read_option_string(&mut self) -> Result<Option<String>, CompiledAssetError> {
        if self.read_bool()? {
            Ok(Some(self.read_string()?))
        } else {
            Ok(None)
        }
    }
}
