//! Per-pass framebuffer capture for the editor frame inspector.
//!
//! ADR 0002 Q8 calls for "the editor's frame inspector panel" with
//! per-pass thumbnails — the Substance Designer "view a node" gesture.
//! This module ships v1 of that capability:
//!
//! - After every pass's [`super::Pass::record`] runs, the engine
//!   synchronously reads the current colour framebuffer back to the CPU
//!   (`glReadPixels`).
//! - The full-resolution capture is then box-averaged on the CPU down to
//!   a small thumbnail (default 128×96, configurable per call).
//! - The result is a `Vec<PassThumbnail>` keyed by pass name + index.
//!
//! Performance posture: `glReadPixels` is synchronous and stalls the GPU
//! pipeline; expect tens of milliseconds per pass at HD resolution. This
//! is fine for an editor-on-demand capture (press a button, see the
//! frame's intermediate states) but unacceptable for steady-state runtime
//! rendering. Callers are expected to invoke
//! [`super::FrameGraph::execute_with_capture`] only when the inspector is
//! actively listening; the plain
//! [`super::FrameGraph::execute`] path remains the default for production.
//!
//! v1 captures the *default framebuffer* contents — for a forward
//! renderer where every pass writes to the backbuffer, this gives the
//! "scene state after pass N ran" view that's the editor's most useful
//! debug information. When the graph grows multiple render targets, this
//! module will gain per-target capture (one thumbnail per declared
//! [`super::ResourceId`] write) and the data structure here will extend
//! correspondingly.

/// A single per-pass thumbnail captured from the framebuffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassThumbnail {
    /// Index of the pass within the compiled frame graph's execution
    /// order. Stable across frames as long as the graph topology
    /// doesn't change.
    pub pass_index: usize,
    /// Pass name as reported by [`super::Pass::name`]. Owned because
    /// the editor displays it in a panel that outlives the borrow scope.
    pub pass_name: String,
    pub width: u32,
    pub height: u32,
    /// RGBA8 pixel data, top-down row order, `width * height * 4` bytes.
    pub data: Vec<u8>,
}

/// Default thumbnail dimensions — small enough for an editor strip,
/// large enough to read the scene at a glance. Roughly 16:9 to match
/// most viewports.
pub const DEFAULT_THUMBNAIL_SIZE: (u32, u32) = (128, 72);

/// Box-average downsample an RGBA8 source image to the requested
/// thumbnail size. Operates entirely on the CPU; uses unweighted area
/// averaging (each destination pixel is the mean of every source pixel
/// whose centre falls inside its destination cell).
///
/// Edge cases:
/// - If `dst_w == 0` or `dst_h == 0`, returns an empty vector.
/// - If `src` is shorter than `src_w * src_h * 4`, the function clamps
///   reads at the slice boundary so it cannot panic on malformed input.
/// - If a destination cell maps to zero source pixels (which can only
///   happen when `dst_w > src_w` or `dst_h > src_h`), the cell ends up
///   black with zero alpha. This is honest: upsampling isn't supported,
///   the caller asked for a bigger thumbnail than the source.
pub fn downsample_rgba8(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Vec<u8> {
    if dst_w == 0 || dst_h == 0 || src_w == 0 || src_h == 0 {
        return Vec::new();
    }
    let mut dst = vec![0u8; (dst_w * dst_h * 4) as usize];

    for dy in 0..dst_h {
        // Source row range that maps into this destination row.
        let sy_start = (dy as u64 * src_h as u64 / dst_h as u64) as u32;
        let sy_end = ((dy as u64 + 1) * src_h as u64 / dst_h as u64) as u32;
        let sy_end = sy_end.min(src_h);

        for dx in 0..dst_w {
            let sx_start = (dx as u64 * src_w as u64 / dst_w as u64) as u32;
            let sx_end = ((dx as u64 + 1) * src_w as u64 / dst_w as u64) as u32;
            let sx_end = sx_end.min(src_w);

            let mut sum_r: u64 = 0;
            let mut sum_g: u64 = 0;
            let mut sum_b: u64 = 0;
            let mut sum_a: u64 = 0;
            let mut count: u64 = 0;

            for sy in sy_start..sy_end {
                let row_base = (sy * src_w * 4) as usize;
                for sx in sx_start..sx_end {
                    let i = row_base + (sx * 4) as usize;
                    if i + 3 >= src.len() {
                        continue;
                    }
                    sum_r += src[i] as u64;
                    sum_g += src[i + 1] as u64;
                    sum_b += src[i + 2] as u64;
                    sum_a += src[i + 3] as u64;
                    count += 1;
                }
            }

            if count > 0 {
                let di = ((dy * dst_w + dx) * 4) as usize;
                dst[di] = (sum_r / count) as u8;
                dst[di + 1] = (sum_g / count) as u8;
                dst[di + 2] = (sum_b / count) as u8;
                dst[di + 3] = (sum_a / count) as u8;
            }
        }
    }

    dst
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Downsampling a uniform-colour image must produce a uniform-colour
    /// thumbnail at the destination size. Tests both the colour fidelity
    /// of the box average and the dimensional correctness.
    #[test]
    fn downsample_uniform_color_preserves_color() {
        let src_w = 32u32;
        let src_h = 16u32;
        let mut src = Vec::with_capacity((src_w * src_h * 4) as usize);
        for _ in 0..(src_w * src_h) {
            src.extend_from_slice(&[200, 100, 50, 255]);
        }
        let dst = downsample_rgba8(&src, src_w, src_h, 8, 4);
        assert_eq!(dst.len(), 8 * 4 * 4);
        for chunk in dst.chunks_exact(4) {
            assert_eq!(chunk, &[200, 100, 50, 255]);
        }
    }

    /// Downsampling a checkerboard halves the contrast — opposing pixels
    /// average to mid-grey when paired. This proves the averaging is
    /// actually averaging (not picking the top-left source pixel).
    #[test]
    fn downsample_checkerboard_averages_to_grey() {
        let src_w = 4u32;
        let src_h = 4u32;
        let mut src = Vec::with_capacity((src_w * src_h * 4) as usize);
        for y in 0..src_h {
            for x in 0..src_w {
                let v = if (x + y) % 2 == 0 { 0u8 } else { 255u8 };
                src.extend_from_slice(&[v, v, v, 255]);
            }
        }
        // 4×4 -> 2×2: each destination pixel averages four source
        // pixels, two black and two white, giving 127.
        let dst = downsample_rgba8(&src, src_w, src_h, 2, 2);
        assert_eq!(dst.len(), 16);
        for chunk in dst.chunks_exact(4) {
            assert_eq!(chunk[0], 127);
            assert_eq!(chunk[1], 127);
            assert_eq!(chunk[2], 127);
            assert_eq!(chunk[3], 255);
        }
    }

    /// Zero-sized destination requests are honoured with an empty
    /// vector rather than a panic.
    #[test]
    fn zero_destination_returns_empty() {
        let src = vec![255u8; 16 * 16 * 4];
        assert!(downsample_rgba8(&src, 16, 16, 0, 16).is_empty());
        assert!(downsample_rgba8(&src, 16, 16, 16, 0).is_empty());
    }

    /// Same-size "downsample" must round-trip pixels exactly when the
    /// dimensions match (each destination cell maps to exactly one
    /// source pixel). Guards against off-by-one drift in the cell
    /// boundary math.
    #[test]
    fn downsample_to_same_size_round_trips_pixels() {
        let src_w = 4u32;
        let src_h = 3u32;
        let mut src = Vec::with_capacity((src_w * src_h * 4) as usize);
        for i in 0..(src_w * src_h) {
            src.extend_from_slice(&[i as u8, (i * 3) as u8, (i * 7) as u8, 255]);
        }
        let dst = downsample_rgba8(&src, src_w, src_h, src_w, src_h);
        assert_eq!(dst, src);
    }

    /// Truncated source data: function must not panic even if the
    /// caller hands in a buffer shorter than `width * height * 4`.
    #[test]
    fn truncated_source_does_not_panic() {
        let src_w = 8u32;
        let src_h = 8u32;
        // Only half the rows worth of data.
        let src = vec![128u8; (src_w * (src_h / 2) * 4) as usize];
        let dst = downsample_rgba8(&src, src_w, src_h, 4, 4);
        // Length must match the requested destination size even when
        // the source was incomplete.
        assert_eq!(dst.len(), 4 * 4 * 4);
    }
}
