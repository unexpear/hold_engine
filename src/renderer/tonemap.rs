//! Tonemap operators in CPU-evaluable form.
//!
//! The OpenGL renderer applies tonemapping inline in the fragment
//! shader (see `opengl_shaders::TEXTURED_FRAG` / `TEXTURED_NORMAL_FRAG`),
//! but the engine's discipline is that every shading operation has a
//! CPU mirror. Two reasons:
//!
//! 1. **Testing.** Shader math is hard to test without a GPU; CPU
//!    mirrors give us deterministic unit tests that lock down the
//!    operator's output for known inputs. If a future shader edit
//!    drifts from the canonical math, the CPU test catches it via
//!    the principle that the two implementations should agree on
//!    every reference point.
//! 2. **Software-renderer parity.** When the software backend grows
//!    a lit shading path (M5+), it consumes these helpers directly
//!    so HDR→LDR mapping is bit-identical to the GPU output.
//!
//! See ADR 0002 Q3 for the shading-model rationale and ADR 0003
//! pillar 4 for why backend parity is sacred here.

/// Narkowicz ACES tonemap fit for `[0, ∞]` linear input → `[0, 1]`
/// linear output. The five constants `a..e` are the published curve
/// coefficients; the function is identical to the GLSL implementation
/// in `opengl_shaders::TEXTURED_FRAG::aces_tonemap`.
///
/// The output is *linear* (still in linear space). Display gamma
/// encoding (sRGB OETF) is applied later — by `GL_FRAMEBUFFER_SRGB`
/// on the GPU path, or by an explicit `linear_to_srgb` step when a
/// CPU consumer needs it (not exposed here yet; will land alongside
/// the software backend's lit path).
pub fn aces_tonemap_rgb(rgb: [f32; 3]) -> [f32; 3] {
    [
        aces_channel(rgb[0]),
        aces_channel(rgb[1]),
        aces_channel(rgb[2]),
    ]
}

#[inline]
fn aces_channel(x: f32) -> f32 {
    const A: f32 = 2.51;
    const B: f32 = 0.03;
    const C: f32 = 2.43;
    const D: f32 = 0.59;
    const E: f32 = 0.14;
    let numerator = x * (A * x + B);
    let denominator = x * (C * x + D) + E;
    (numerator / denominator).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() < eps
    }

    /// Tonemapping zero linear input must yield zero. A non-zero result
    /// here would mean the operator has a black-point bias that would
    /// lift shadows visibly across every rendered frame.
    #[test]
    fn aces_zero_input_is_black() {
        let out = aces_tonemap_rgb([0.0, 0.0, 0.0]);
        assert!(approx(out[0], 0.0, 1e-6), "R: {}", out[0]);
        assert!(approx(out[1], 0.0, 1e-6), "G: {}", out[1]);
        assert!(approx(out[2], 0.0, 1e-6), "B: {}", out[2]);
    }

    /// Tonemapping a very bright input must clamp at 1.0 in every
    /// channel. The Narkowicz fit asymptotes at `a / c ≈ 1.033`, which
    /// the explicit `clamp(0, 1)` brings down to 1.0.
    #[test]
    fn aces_clips_bright_input_to_one() {
        let out = aces_tonemap_rgb([1000.0, 1000.0, 1000.0]);
        assert!(approx(out[0], 1.0, 1e-4), "R: {}", out[0]);
        assert!(approx(out[1], 1.0, 1e-4), "G: {}", out[1]);
        assert!(approx(out[2], 1.0, 1e-4), "B: {}", out[2]);
    }

    /// Tonemapping is monotonically non-decreasing in each channel —
    /// brighter inputs must never produce darker outputs. This locks
    /// down a property the operator was specifically designed to hold.
    #[test]
    fn aces_is_monotonic_per_channel() {
        let mut prev = -1.0;
        for i in 0..=200 {
            let x = i as f32 / 50.0; // 0.0 .. 4.0
            let mapped = aces_channel(x);
            assert!(
                mapped + 1e-6 >= prev,
                "non-monotonic at x = {}: {} < {}",
                x,
                mapped,
                prev
            );
            prev = mapped;
        }
    }

    /// Per-channel independence: feeding a vector through
    /// `aces_tonemap_rgb` is the same as mapping each channel
    /// individually through `aces_channel`. Documents that the
    /// operator carries no cross-channel coupling — important
    /// because callers sometimes need to tonemap a single luminance
    /// channel and reuse the result.
    #[test]
    fn aces_rgb_matches_per_channel() {
        let probes = [0.1f32, 0.5, 1.0, 2.0, 5.5];
        for r in probes {
            for g in probes {
                for b in probes {
                    let combined = aces_tonemap_rgb([r, g, b]);
                    assert!(approx(combined[0], aces_channel(r), 1e-6));
                    assert!(approx(combined[1], aces_channel(g), 1e-6));
                    assert!(approx(combined[2], aces_channel(b), 1e-6));
                }
            }
        }
    }

    /// Specific reference point: an input of 1.0 (mid-grey on a
    /// linear sensor) lands at roughly 0.79 after the Narkowicz fit.
    /// This is the canonical "exposure-correct mid-grey" output and
    /// is a useful eyeball check that the math is the right curve.
    #[test]
    fn aces_unit_input_lands_near_canonical_value() {
        let out = aces_channel(1.0);
        // Numerator: 1 * (2.51 + 0.03) = 2.54
        // Denominator: 1 * (2.43 + 0.59) + 0.14 = 3.16
        // 2.54 / 3.16 ≈ 0.8038
        assert!(
            approx(out, 0.8038, 1e-3),
            "expected ~0.8038, got {}",
            out
        );
    }
}
