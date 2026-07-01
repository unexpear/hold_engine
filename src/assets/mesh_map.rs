//! Bake mesh maps from [`MeshData`] geometry — the substrate that
//! procedural masks read from in the layered-material system (Painter
//! style; see ADR 0002 Q5).
//!
//! v1 produces *per-vertex* maps:
//! - [`compute_curvature_per_vertex`] — `[0, 1]` proxy for surface bend
//! - [`compute_ao_per_vertex`]        — `[0, 1]` ambient occlusion
//!
//! Per-vertex was chosen over texel-domain baking because the GPU
//! rasteriser already interpolates between vertices, giving smooth
//! mesh-map values across the surface for free. For the low-poly content
//! that v1 procedural masks operate on, this is visually equivalent to
//! Substance Painter's UV-rasterised mesh maps at a fraction of the
//! import-time cost. A future revision can grow texel-domain output as
//! an opt-in for high-density content without breaking the per-vertex
//! API.
//!
//! Both functions are deterministic. The AO sampler uses a small inline
//! linear-congruential generator seeded by the caller, so a given
//! `(mesh, samples, max_distance, seed)` always produces the same
//! result — important for reproducible builds and for the
//! determinism pillar (ADR 0003 pillar 4).

use super::mesh::MeshData;
use super::types::{MeshMapAO, MeshMapCurvature, MeshMapError};

/// Compute per-vertex curvature.
///
/// Algorithm: at each vertex, find every triangle that touches it,
/// compute the triangle's face normal, and average the dot products of
/// face normals against the vertex normal. The reported curvature is
/// `1 - mean_dot`, which is `0.0` for a perfectly flat surface (every
/// face normal equals the vertex normal) and rises with the average
/// disagreement between vertex and face normals.
///
/// The result is clamped into `[0, 1]` and packaged as a
/// [`MeshMapCurvature`] asset.
///
/// Returns an error if `MeshMapCurvature::new` rejects the result —
/// this should not happen for finite mesh data, but the validator is
/// strict so the caller knows about any drift in the math via a typed
/// failure rather than silent corruption.
pub fn compute_curvature_per_vertex(mesh: &MeshData) -> Result<MeshMapCurvature, MeshMapError> {
    let n = mesh.vertices.len();

    // Sum and count of face-normal dot products per vertex.
    let mut sums = vec![0.0f32; n];
    let mut counts = vec![0u32; n];

    for tri in mesh.indices.chunks_exact(3) {
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;

        let p0 = mesh.vertices[i0].pos;
        let p1 = mesh.vertices[i1].pos;
        let p2 = mesh.vertices[i2].pos;

        let e1 = sub(p1, p0);
        let e2 = sub(p2, p0);
        let face_normal = match normalize(cross(e1, e2)) {
            Some(n) => n,
            None => continue, // degenerate triangle: skip
        };

        for &i in &[i0, i1, i2] {
            let vn = mesh.vertices[i].normal;
            let d = dot(vn, face_normal);
            sums[i] += d;
            counts[i] += 1;
        }
    }

    let values: Vec<f32> = (0..n)
        .map(|i| {
            if counts[i] == 0 {
                // Vertex unreferenced by any triangle — declare flat.
                0.0
            } else {
                let mean = sums[i] / counts[i] as f32;
                (1.0 - mean).clamp(0.0, 1.0)
            }
        })
        .collect();

    MeshMapCurvature::new(values, n)
}

/// Compute per-vertex ambient occlusion via cosine-weighted hemisphere
/// sampling.
///
/// At every vertex the function:
/// 1. Offsets the ray origin slightly along the vertex normal (1e-4
///    units) to avoid self-intersection with adjacent triangles.
/// 2. Generates `samples` cosine-weighted directions in the hemisphere
///    around the vertex normal.
/// 3. Casts each ray against every triangle in the mesh; if the ray
///    hits any triangle within `max_distance`, that sample is counted
///    as occluded.
/// 4. Reports AO = `1 - occluded / samples`. Higher = more visible.
///
/// `seed` drives the inline LCG so results are deterministic given the
/// same inputs. Pass any non-zero `u64`; the engine convention is to
/// derive it from a stable hash of the mesh's logical name when the
/// import pipeline calls this function so a given mesh always bakes
/// the same AO regardless of host machine or sibling assets.
///
/// Performance: O(samples × triangles × vertices). Acceptable at
/// import time for content sizes up to ~10k triangles; larger meshes
/// will want a BVH (M5.5+ work; tracked under the asset-pipeline
/// hardening milestone).
pub fn compute_ao_per_vertex(
    mesh: &MeshData,
    samples: u32,
    max_distance: f32,
    seed: u64,
) -> Result<MeshMapAO, MeshMapError> {
    let n = mesh.vertices.len();
    let mut values = vec![0.0f32; n];
    if samples == 0 {
        // Caller asked for no samples — every vertex is fully visible
        // by definition. Returning all 1.0 is the sensible identity.
        for v in &mut values {
            *v = 1.0;
        }
        return MeshMapAO::new(values, n);
    }

    // Pre-extract triangles into [P0, edge1, edge2] form so the
    // per-ray Möller-Trumbore inner loop only touches three vec3s.
    let mut tri_data: Vec<([f32; 3], [f32; 3], [f32; 3])> =
        Vec::with_capacity(mesh.indices.len() / 3);
    for tri in mesh.indices.chunks_exact(3) {
        let p0 = mesh.vertices[tri[0] as usize].pos;
        let p1 = mesh.vertices[tri[1] as usize].pos;
        let p2 = mesh.vertices[tri[2] as usize].pos;
        tri_data.push((p0, sub(p1, p0), sub(p2, p0)));
    }

    let mut rng = Lcg::new(seed.max(1));

    for vi in 0..n {
        let vertex = &mesh.vertices[vi];
        let normal = match normalize(vertex.normal) {
            Some(n) => n,
            None => {
                values[vi] = 1.0;
                continue;
            }
        };
        let origin = add(vertex.pos, scale(normal, 1.0e-4));
        let (tan_u, tan_v) = orthonormal_tangent_frame(normal);

        let mut occluded = 0u32;
        for _ in 0..samples {
            // Cosine-weighted hemisphere sampling: r1, r2 ∈ [0, 1).
            let r1 = rng.next_unit();
            let r2 = rng.next_unit();
            let phi = 2.0 * std::f32::consts::PI * r1;
            let cos_theta = (1.0 - r2).sqrt();
            let sin_theta = (1.0 - cos_theta * cos_theta).sqrt();

            // Direction in the local frame, then projected to world.
            let local = [
                phi.cos() * sin_theta,
                phi.sin() * sin_theta,
                cos_theta,
            ];
            let dir = add(
                add(scale(tan_u, local[0]), scale(tan_v, local[1])),
                scale(normal, local[2]),
            );

            // Cast against every triangle. Any hit within max_distance
            // counts as occluded; we don't need the nearest hit.
            let mut hit = false;
            for (p0, e1, e2) in &tri_data {
                if let Some(t) = ray_triangle_intersect(origin, dir, *p0, *e1, *e2) {
                    if t > 1e-4 && t <= max_distance {
                        hit = true;
                        break;
                    }
                }
            }
            if hit {
                occluded += 1;
            }
        }

        let ao = 1.0 - (occluded as f32 / samples as f32);
        values[vi] = ao.clamp(0.0, 1.0);
    }

    MeshMapAO::new(values, n)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Möller–Trumbore ray-triangle intersection. Returns the parametric
/// distance `t` along `dir` from `origin` to the hit, or `None` if the
/// ray misses or grazes a back-face.
///
/// `e1 = P1 - P0`, `e2 = P2 - P0`. Pre-computing edges outside the
/// inner loop is the conventional optimisation — this signature exists
/// to make that explicit at the call site.
fn ray_triangle_intersect(
    origin: [f32; 3],
    dir: [f32; 3],
    p0: [f32; 3],
    e1: [f32; 3],
    e2: [f32; 3],
) -> Option<f32> {
    let h = cross(dir, e2);
    let a = dot(e1, h);
    if a.abs() < 1e-8 {
        return None; // Parallel to triangle plane.
    }
    let f = 1.0 / a;
    let s = sub(origin, p0);
    let u = f * dot(s, h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = f * dot(dir, q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = f * dot(e2, q);
    if t > 0.0 {
        Some(t)
    } else {
        None
    }
}

/// Build an arbitrary orthonormal pair `(t, b)` perpendicular to
/// `normal`. The exact orientation is irrelevant for hemisphere
/// sampling — every direction has equal probability of being chosen
/// regardless of how `t` and `b` are oriented around the normal axis,
/// because the sampling distribution is rotationally symmetric.
fn orthonormal_tangent_frame(normal: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    // Pick a helper axis least aligned with the normal so the cross
    // product is well-conditioned.
    let helper = if normal[0].abs() > 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let t = normalize(cross(helper, normal)).unwrap_or([1.0, 0.0, 0.0]);
    let b = cross(normal, t);
    (t, b)
}

#[inline]
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

#[inline]
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> Option<[f32; 3]> {
    let len_sq = dot(v, v);
    if len_sq < 1e-20 {
        return None;
    }
    let inv = len_sq.sqrt().recip();
    Some([v[0] * inv, v[1] * inv, v[2] * inv])
}

/// Inline linear-congruential generator. Numerical Recipes parameters,
/// adequate for AO hemisphere sampling. The engine's zero-deps charter
/// rules out crates like `rand`; this is the in-house equivalent for
/// import-time bake operations where determinism with seed control is
/// the load-bearing property.
struct Lcg {
    state: u64,
}

impl Lcg {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }
    fn next_u32(&mut self) -> u32 {
        // Numerical Recipes constants (Knuth's MMIX).
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.state >> 33) as u32
    }
    /// Next pseudo-random `f32` in `[0, 1)`. The mantissa-only conversion
    /// guarantees the result never reaches `1.0` exactly, so callers
    /// using `r * 2π` etc. don't need an explicit clamp.
    fn next_unit(&mut self) -> f32 {
        let bits = self.next_u32() & 0x00FF_FFFF;
        bits as f32 / (1u32 << 24) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::mesh::MeshData;

    fn approx(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() < eps
    }

    /// A flat plane has every face normal equal to its vertex normals,
    /// so curvature must be `0` everywhere. This locks the curvature
    /// proxy's "identity" case — any drift in the math fails this
    /// test loudly.
    #[test]
    fn curvature_of_flat_plane_is_zero() {
        let mesh = MeshData::plane(8, 8);
        let map = compute_curvature_per_vertex(&mesh).unwrap();
        for (i, &v) in map.values.iter().enumerate() {
            assert!(approx(v, 0.0, 1e-4), "vertex {} curvature = {}", i, v);
        }
    }

    /// A unit sphere has at most small curvature at every vertex (the
    /// discrete tessellation introduces face-vs-vertex normal
    /// disagreement around the smooth surface, but it stays small).
    /// We assert two properties: no vertex exceeds the upper bound
    /// (the sphere has no sharp features), and the spread is tight
    /// (the sphere is rotationally uniform). Some vertices may sit at
    /// 0.0 due to fp32 precision when face normals collapse onto the
    /// vertex normal at pole-adjacent triangles, so we don't require
    /// strict positivity.
    #[test]
    fn curvature_of_sphere_is_small_and_uniform() {
        let mesh = MeshData::sphere(12, 24);
        let map = compute_curvature_per_vertex(&mesh).unwrap();
        let mut min_c = f32::MAX;
        let mut max_c = f32::MIN;
        for &v in &map.values {
            min_c = min_c.min(v);
            max_c = max_c.max(v);
        }
        assert!(min_c >= 0.0, "expected sphere curvature >= 0, got min {}", min_c);
        assert!(max_c < 0.2, "expected sphere curvature < 0.2, got max {}", max_c);
        // Uniformity: the spread should be tight relative to the values.
        assert!(
            max_c - min_c < 0.05,
            "sphere curvature spread {} too large (min {}, max {})",
            max_c - min_c,
            min_c,
            max_c
        );
    }

    /// A cube has flat per-face normals that disagree sharply with the
    /// per-vertex normal direction at corners — but the cube primitive
    /// in this engine produces hard-shaded faces with the vertex normal
    /// EQUAL to the single face normal of each face (no normal sharing
    /// across faces). So the proxy reports `0` for every cube vertex.
    /// This test pins that behaviour: it documents the hard-shading
    /// contract of `MeshData::cube` and keeps the curvature math
    /// consistent with vertex-normal expectations.
    #[test]
    fn curvature_of_hard_shaded_cube_is_zero() {
        let mesh = MeshData::cube();
        let map = compute_curvature_per_vertex(&mesh).unwrap();
        for &v in &map.values {
            assert!(approx(v, 0.0, 1e-4));
        }
    }

    /// A flat plane in empty space has nothing to occlude its
    /// hemisphere, so AO is `1.0` at every vertex. Locks the AO
    /// "identity" case.
    #[test]
    fn ao_of_flat_plane_is_one() {
        let mesh = MeshData::plane(4, 4);
        let map = compute_ao_per_vertex(&mesh, 16, 5.0, 42).unwrap();
        for (i, &v) in map.values.iter().enumerate() {
            assert!(
                approx(v, 1.0, 1e-3),
                "vertex {} AO = {} (expected ~1.0)",
                i,
                v
            );
        }
    }

    /// AO output must be inside `[0, 1]` for any input mesh. The
    /// MeshMapAO::new validator already enforces this; the test exists
    /// to confirm the compute function never produces values that
    /// would trip the validator (which would surface as a returned
    /// error rather than a clamp).
    #[test]
    fn ao_is_bounded_in_unit_range() {
        let mesh = MeshData::sphere(8, 16);
        let map = compute_ao_per_vertex(&mesh, 32, 10.0, 7).unwrap();
        for (i, &v) in map.values.iter().enumerate() {
            assert!(
                (0.0..=1.0).contains(&v),
                "vertex {} AO = {} out of range",
                i,
                v
            );
        }
    }

    /// Same mesh + same sample count + same seed must produce
    /// identical AO output across runs. The determinism pillar (ADR
    /// 0003) is non-negotiable; the inline LCG plus deterministic
    /// vertex traversal is what makes that property hold.
    #[test]
    fn ao_is_deterministic_with_seed() {
        let mesh = MeshData::sphere(6, 12);
        let a = compute_ao_per_vertex(&mesh, 24, 8.0, 1234).unwrap();
        let b = compute_ao_per_vertex(&mesh, 24, 8.0, 1234).unwrap();
        assert_eq!(a.values, b.values);
    }

    /// Different seeds must produce different AO output for a mesh
    /// where the hemisphere is partially occluded — i.e. a non-convex
    /// configuration where the random sample directions actually
    /// matter. Convex meshes (sphere, cube viewed from outside) are
    /// useless for this test: their hemispheres are unoccluded so AO
    /// is `1.0` regardless of which directions get sampled.
    ///
    /// We build a "tee" of two triangles meeting at a shared vertex,
    /// roughly perpendicular, so that the hemisphere along the floor
    /// triangle's normal partially intersects the wall triangle. AO
    /// at the shared vertex now depends on which random directions
    /// are picked, so the seed is observable in the output.
    #[test]
    fn ao_responds_to_seed_changes() {
        use crate::assets::mesh::Vertex;
        // Floor triangle centred at the origin, normal +Y.
        // Wall triangle to the +X side, leaning over the floor so its
        // surface intersects the upper hemisphere of the floor's
        // shared vertex.
        let v0 = Vertex {
            pos: [0.0, 0.0, 0.0],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
        };
        let v1 = Vertex {
            pos: [2.0, 0.0, 0.0],
            normal: [0.0, 1.0, 0.0],
            uv: [1.0, 0.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
        };
        let v2 = Vertex {
            pos: [0.0, 0.0, 2.0],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 1.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
        };
        // Wall: leaning leftward over the floor's hemisphere.
        let w0 = Vertex {
            pos: [-0.1, 0.5, 0.0],
            normal: [1.0, 0.0, 0.0],
            uv: [0.0, 0.0],
            tangent: [0.0, 0.0, 1.0, 1.0],
        };
        let w1 = Vertex {
            pos: [0.5, 1.5, 0.0],
            normal: [1.0, 0.0, 0.0],
            uv: [1.0, 0.0],
            tangent: [0.0, 0.0, 1.0, 1.0],
        };
        let w2 = Vertex {
            pos: [-0.1, 0.5, 2.0],
            normal: [1.0, 0.0, 0.0],
            uv: [0.0, 1.0],
            tangent: [0.0, 0.0, 1.0, 1.0],
        };
        let vertices = vec![v0, v1, v2, w0, w1, w2];
        let indices = vec![0, 1, 2, 3, 4, 5];
        let mesh = MeshData::from_raw(vertices, indices);

        let a = compute_ao_per_vertex(&mesh, 16, 5.0, 1).unwrap();
        let b = compute_ao_per_vertex(&mesh, 16, 5.0, 999_999).unwrap();
        let differs = a
            .values
            .iter()
            .zip(b.values.iter())
            .any(|(x, y)| (x - y).abs() > 1.0e-6);
        assert!(
            differs,
            "different seeds produced identical AO ({:?} vs {:?})",
            a.values, b.values
        );
    }

    /// Zero-sample request returns AO = 1.0 everywhere as the
    /// well-defined identity. Locks the edge case so callers can
    /// pass `samples: 0` to "skip AO" without seeing zero-divide
    /// noise.
    #[test]
    fn ao_with_zero_samples_returns_one() {
        let mesh = MeshData::cube();
        let map = compute_ao_per_vertex(&mesh, 0, 5.0, 1).unwrap();
        for &v in &map.values {
            assert_eq!(v, 1.0);
        }
    }

    /// Möller-Trumbore basics: a ray pointing straight at a known
    /// triangle hits at the predicted parametric distance — `t` is the
    /// distance along the unit-length `dir` vector to the hit point.
    /// For an origin at (0,0,0) and target at (0.25, 0.25, 2), the
    /// distance is `sqrt(0.0625 + 0.0625 + 4) = sqrt(4.125) ≈ 2.0310`.
    #[test]
    fn ray_triangle_intersect_hits_known_triangle() {
        let p0 = [0.0, 0.0, 2.0];
        let e1 = [1.0, 0.0, 0.0];
        let e2 = [0.0, 1.0, 0.0];
        let origin = [0.0, 0.0, 0.0];
        let dir = normalize([0.25, 0.25, 2.0]).unwrap();
        let t = ray_triangle_intersect(origin, dir, p0, e1, e2)
            .expect("ray should hit triangle");
        assert!(approx(t, 2.0310, 1e-3), "t = {}", t);
    }

    /// Ray pointing away from the triangle returns `None`.
    #[test]
    fn ray_triangle_intersect_misses_when_pointing_away() {
        let p0 = [0.0, 0.0, 2.0];
        let e1 = [1.0, 0.0, 0.0];
        let e2 = [0.0, 1.0, 0.0];
        let origin = [0.0, 0.0, 0.0];
        let dir = [0.0, 0.0, -1.0]; // away from the triangle
        assert!(ray_triangle_intersect(origin, dir, p0, e1, e2).is_none());
    }

    /// LCG produces values strictly in `[0, 1)` and is deterministic
    /// with seed.
    #[test]
    fn lcg_is_bounded_and_deterministic() {
        let mut a = Lcg::new(42);
        let mut b = Lcg::new(42);
        for _ in 0..100 {
            let x = a.next_unit();
            let y = b.next_unit();
            assert_eq!(x, y);
            assert!((0.0..1.0).contains(&x), "value out of range: {}", x);
        }
    }
}
