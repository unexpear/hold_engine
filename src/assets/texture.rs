//! BMP and TGA image loaders — written from scratch, zero dependencies.
//!
//! Both loaders decode to RGBA8 pixel data and return a [`Texture`].
//!
//! # Supported formats
//!
//! ## BMP
//! - 24-bit BGR and 32-bit BGRA, uncompressed (`BI_RGB`).
//! - Bottom-up (positive height) and top-down (negative height) scanlines.
//! - Row padding to the 4-byte boundary is handled automatically.
//!
//! ## TGA
//! - Uncompressed true-colour (type 2) and RLE true-colour (type 10).
//! - 24-bit BGR and 32-bit BGRA.
//! - Origin bit (top-left vs bottom-left) from the image descriptor.

use std::fmt;
use std::fs;
use std::path::Path;

use crate::assets::types::{Texture, TextureFormat};

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum TexError {
    Io(std::io::Error),
    /// The file header magic bytes did not match the expected format.
    BadMagic,
    /// A required header field has an unsupported value.
    Unsupported(String),
    /// The file is truncated or a length field is inconsistent.
    Truncated,
}

impl fmt::Display for TexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TexError::Io(e) => write!(f, "IO error: {e}"),
            TexError::BadMagic => write!(f, "unrecognised file header"),
            TexError::Unsupported(s) => write!(f, "unsupported: {s}"),
            TexError::Truncated => write!(f, "file truncated"),
        }
    }
}

impl std::error::Error for TexError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let TexError::Io(e) = self {
            Some(e)
        } else {
            None
        }
    }
}

// ── Little-endian helpers ─────────────────────────────────────────────────────

#[inline]
fn u16_le(buf: &[u8], off: usize) -> u16 {
    (buf[off] as u16) | ((buf[off + 1] as u16) << 8)
}

#[inline]
fn u32_le(buf: &[u8], off: usize) -> u32 {
    (buf[off] as u32)
        | ((buf[off + 1] as u32) << 8)
        | ((buf[off + 2] as u32) << 16)
        | ((buf[off + 3] as u32) << 24)
}

#[inline]
fn i32_le(buf: &[u8], off: usize) -> i32 {
    u32_le(buf, off) as i32
}

// ═══════════════════════════════════════════════════════════════════════════════
//  BMP
// ═══════════════════════════════════════════════════════════════════════════════

/// Load a BMP file and decode it into an RGBA8 [`Texture`].
///
/// Only uncompressed 24-bit and 32-bit BMPs are supported (the vast majority
/// of BMP files in the wild).
///
/// # Errors
/// Returns [`TexError`] on I/O failure, bad magic, unsupported compression, or
/// truncated data.
pub fn load_bmp<P: AsRef<Path>>(path: P) -> Result<Texture, TexError> {
    let data = fs::read(path).map_err(TexError::Io)?;
    decode_bmp(&data)
}

/// Decode BMP from an in-memory byte slice.
pub fn decode_bmp(data: &[u8]) -> Result<Texture, TexError> {
    // ── File header (14 bytes) ────────────────────────────────────────────
    if data.len() < 54 {
        return Err(TexError::Truncated);
    }
    if data[0] != b'B' || data[1] != b'M' {
        return Err(TexError::BadMagic);
    }
    let pixel_offset = u32_le(data, 10) as usize;

    // ── DIB header (BITMAPINFOHEADER, 40 bytes minimum) ───────────────────
    let dib_size = u32_le(data, 14);
    if dib_size < 40 {
        return Err(TexError::Unsupported(format!(
            "DIB header size {dib_size} (only BITMAPINFOHEADER supported)"
        )));
    }
    let raw_width = i32_le(data, 18);
    let raw_height = i32_le(data, 22);
    let bpp = u16_le(data, 28);
    let compression = u32_le(data, 30);

    if compression != 0 {
        return Err(TexError::Unsupported(format!(
            "compression type {compression} (only BI_RGB = 0 supported)"
        )));
    }
    if bpp != 24 && bpp != 32 {
        return Err(TexError::Unsupported(format!(
            "{bpp}-bit BMP (only 24 and 32 supported)"
        )));
    }

    let width = raw_width.unsigned_abs();
    let height = raw_height.unsigned_abs();
    // Positive height = bottom-up; negative = top-down.
    let bottom_up = raw_height > 0;

    let bytes_per_pixel = (bpp / 8) as usize;
    let row_bytes = width as usize * bytes_per_pixel;
    // BMP rows are padded to a 4-byte boundary.
    let stride = (row_bytes + 3) & !3;

    let pixel_data = data.get(pixel_offset..).ok_or(TexError::Truncated)?;
    let needed = stride * height as usize;
    if pixel_data.len() < needed {
        return Err(TexError::Truncated);
    }

    // ── Decode to RGBA8 ───────────────────────────────────────────────────
    let w = width as usize;
    let h = height as usize;
    let mut rgba = vec![255u8; w * h * 4];

    for row in 0..h {
        let src_row = if bottom_up { h - 1 - row } else { row };
        let src_off = src_row * stride;
        let dst_off = row * w * 4;

        for col in 0..w {
            let si = src_off + col * bytes_per_pixel;
            let di = dst_off + col * 4;
            // BMP stores pixels as BGR(A).
            rgba[di] = pixel_data[si + 2]; // R
            rgba[di + 1] = pixel_data[si + 1]; // G
            rgba[di + 2] = pixel_data[si]; // B
            rgba[di + 3] = if bpp == 32 { pixel_data[si + 3] } else { 255 };
        }
    }

    // Default new textures to sRGB on import. Most authored content
    // arriving via BMP/TGA is colour data; data-bearing maps
    // (roughness, AO, height, masks) need to be re-tagged via
    // `Texture { color_space: ColorSpace::Linear, .. }` after load. See
    // ADR 0002 Q3.
    Ok(Texture {
        width,
        height,
        format: TextureFormat::Rgba8,
        color_space: super::types::ColorSpace::Srgb,
        data: rgba,
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
//  TGA
// ═══════════════════════════════════════════════════════════════════════════════

/// Load a TGA file and decode it into an RGBA8 [`Texture`].
///
/// Supports uncompressed true-colour (type 2) and RLE true-colour (type 10),
/// in both 24-bit and 32-bit pixel depths.
///
/// # Errors
/// Returns [`TexError`] on I/O failure, unsupported image type, or truncated
/// data.
pub fn load_tga<P: AsRef<Path>>(path: P) -> Result<Texture, TexError> {
    let data = fs::read(path).map_err(TexError::Io)?;
    decode_tga(&data)
}

/// Decode TGA from an in-memory byte slice.
pub fn decode_tga(data: &[u8]) -> Result<Texture, TexError> {
    // ── Header (18 bytes) ─────────────────────────────────────────────────
    if data.len() < 18 {
        return Err(TexError::Truncated);
    }
    let id_len = data[0] as usize;
    let cmap_type = data[1];
    let img_type = data[2];
    let width = u16_le(data, 12) as u32;
    let height = u16_le(data, 14) as u32;
    let bpp = data[16];
    let descriptor = data[17];

    if cmap_type != 0 {
        return Err(TexError::Unsupported(
            "colour-mapped TGA not supported".into(),
        ));
    }
    if img_type != 2 && img_type != 10 {
        return Err(TexError::Unsupported(format!(
            "TGA image type {img_type} (only 2=uncompressed and 10=RLE supported)"
        )));
    }
    if bpp != 24 && bpp != 32 {
        return Err(TexError::Unsupported(format!(
            "{bpp}-bit TGA (only 24 and 32 supported)"
        )));
    }

    let bytes_per_pixel = (bpp / 8) as usize;
    let pixel_count = (width as usize) * (height as usize);
    let origin_top = (descriptor & 0x20) != 0;

    // Skip past the image ID field.
    let pixel_start = 18 + id_len;
    let pixel_data = data.get(pixel_start..).ok_or(TexError::Truncated)?;

    // ── Decode raw BGR(A) pixels ──────────────────────────────────────────
    let bgra: Vec<u8> = if img_type == 2 {
        // Uncompressed
        let needed = pixel_count * bytes_per_pixel;
        if pixel_data.len() < needed {
            return Err(TexError::Truncated);
        }
        pixel_data[..needed].to_vec()
    } else {
        // RLE (type 10)
        decode_tga_rle(pixel_data, pixel_count, bytes_per_pixel)?
    };

    // ── Convert BGR(A) → RGBA8 and flip rows if needed ────────────────────
    let w = width as usize;
    let h = height as usize;
    let mut rgba = vec![255u8; pixel_count * 4];

    for row in 0..h {
        let src_row = if origin_top { row } else { h - 1 - row };
        let src_off = src_row * w * bytes_per_pixel;
        let dst_off = row * w * 4;

        for col in 0..w {
            let si = src_off + col * bytes_per_pixel;
            let di = dst_off + col * 4;
            rgba[di] = bgra[si + 2]; // R
            rgba[di + 1] = bgra[si + 1]; // G
            rgba[di + 2] = bgra[si]; // B
            rgba[di + 3] = if bpp == 32 { bgra[si + 3] } else { 255 };
        }
    }

    // Default new textures to sRGB on import. Most authored content
    // arriving via BMP/TGA is colour data; data-bearing maps
    // (roughness, AO, height, masks) need to be re-tagged via
    // `Texture { color_space: ColorSpace::Linear, .. }` after load. See
    // ADR 0002 Q3.
    Ok(Texture {
        width,
        height,
        format: TextureFormat::Rgba8,
        color_space: super::types::ColorSpace::Srgb,
        data: rgba,
    })
}

/// Decode TGA run-length-encoded pixel data.
fn decode_tga_rle(
    src: &[u8],
    pixel_count: usize,
    bytes_per_pixel: usize,
) -> Result<Vec<u8>, TexError> {
    let mut out = Vec::with_capacity(pixel_count * bytes_per_pixel);
    let mut cur = 0usize;
    let mut pixels_decoded = 0usize;

    while pixels_decoded < pixel_count {
        if cur >= src.len() {
            return Err(TexError::Truncated);
        }
        let header = src[cur];
        cur += 1;
        let count = (header & 0x7F) as usize + 1;

        if header & 0x80 != 0 {
            // RLE packet: one pixel repeated `count` times.
            if cur + bytes_per_pixel > src.len() {
                return Err(TexError::Truncated);
            }
            let pixel = &src[cur..cur + bytes_per_pixel];
            cur += bytes_per_pixel;
            for _ in 0..count {
                out.extend_from_slice(pixel);
            }
        } else {
            // Raw packet: `count` literal pixels.
            let need = count * bytes_per_pixel;
            if cur + need > src.len() {
                return Err(TexError::Truncated);
            }
            out.extend_from_slice(&src[cur..cur + need]);
            cur += need;
        }
        pixels_decoded += count;
    }

    Ok(out)
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Auto-detect by extension
// ═══════════════════════════════════════════════════════════════════════════════

/// Load a texture file, choosing the decoder based on the file extension.
///
/// Recognised extensions: `.bmp`, `.tga` (case-insensitive).
pub fn load_texture<P: AsRef<Path>>(path: P) -> Result<Texture, TexError> {
    let p = path.as_ref();
    match p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("bmp") => load_bmp(p),
        Some("tga") => load_tga(p),
        Some(ext) => Err(TexError::Unsupported(format!("unknown extension .{ext}"))),
        None => Err(TexError::Unsupported("no file extension".into())),
    }
}
