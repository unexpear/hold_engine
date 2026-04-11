//! Chunked heightmap terrain.
//!
//! The world is split into square chunks on the XZ plane.  Each chunk is a
//! regular grid of quads, triangulated and lit with per-vertex normals.
//! [`Terrain`] manages which chunks are alive based on the camera position
//! so only nearby geometry is kept in memory and rendered.

use crate::math::Vec3;
use crate::renderer::Color;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Number of height samples along one edge of a chunk.
/// Actual quads per chunk edge = CHUNK_RES - 1.
const CHUNK_RES: usize = 17;

/// World-space size of one chunk along X and Z.
const CHUNK_SIZE: f32 = 16.0;

// ── Heightmap ─────────────────────────────────────────────────────────────────

/// A procedural or loaded heightmap that can be sampled at any (x, z).
pub struct Heightmap {
    /// Amplitude multiplier applied to the raw noise value.
    pub amplitude: f32,
    /// Frequency (inverse wavelength) of the base octave.
    pub frequency: f32,
    /// Number of noise octaves.  More = finer detail.
    pub octaves: u32,
}

impl Default for Heightmap {
    fn default() -> Self {
        Self {
            amplitude: 6.0,
            frequency: 0.06,
            octaves: 4,
        }
    }
}

impl Heightmap {
    pub fn new(amplitude: f32, frequency: f32, octaves: u32) -> Self {
        Self {
            amplitude,
            frequency,
            octaves,
        }
    }

    /// Sample the height at world-space (x, z).
    pub fn sample(&self, x: f32, z: f32) -> f32 {
        let mut value = 0.0_f32;
        let mut amp = self.amplitude;
        let mut freq = self.frequency;
        for _ in 0..self.octaves {
            value += amp * Self::noise2d(x * freq, z * freq);
            amp *= 0.5;
            freq *= 2.0;
        }
        value
    }

    /// Simple value-noise (smooth random interpolation on a grid).
    fn noise2d(x: f32, z: f32) -> f32 {
        let xi = x.floor() as i32;
        let zi = z.floor() as i32;
        let fx = x - x.floor();
        let fz = z - z.floor();
        // Smoothstep for continuity.
        let sx = fx * fx * (3.0 - 2.0 * fx);
        let sz = fz * fz * (3.0 - 2.0 * fz);

        let n00 = Self::hash(xi, zi);
        let n10 = Self::hash(xi + 1, zi);
        let n01 = Self::hash(xi, zi + 1);
        let n11 = Self::hash(xi + 1, zi + 1);

        let nx0 = n00 + (n10 - n00) * sx;
        let nx1 = n01 + (n11 - n01) * sx;
        nx0 + (nx1 - nx0) * sz
    }

    /// Pseudo-random hash returning a value in [-1, 1].
    #[inline]
    fn hash(x: i32, z: i32) -> f32 {
        // Robert Jenkins style integer hash.
        let mut n = (x.wrapping_mul(374761393)).wrapping_add(z.wrapping_mul(668265263));
        n = (n ^ (n >> 13)).wrapping_mul(1274126177);
        n = n ^ (n >> 16);
        // Map to [-1, 1].
        (n as f32) / (i32::MAX as f32)
    }
}

// ── TerrainChunk ──────────────────────────────────────────────────────────────

/// Pre-built triangle mesh for a single terrain chunk.
pub struct TerrainChunk {
    pub verts: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub faces: Vec<[usize; 3]>,
    /// Chunk grid coordinate (not world-space).
    pub cx: i32,
    pub cz: i32,
}

impl TerrainChunk {
    /// Build geometry for chunk at grid position (cx, cz).
    pub fn build(hmap: &Heightmap, cx: i32, cz: i32) -> Self {
        let origin_x = cx as f32 * CHUNK_SIZE;
        let origin_z = cz as f32 * CHUNK_SIZE;
        let step = CHUNK_SIZE / (CHUNK_RES - 1) as f32;

        // Sample heights on a grid.
        let mut verts = Vec::with_capacity(CHUNK_RES * CHUNK_RES);
        for iz in 0..CHUNK_RES {
            for ix in 0..CHUNK_RES {
                let wx = origin_x + ix as f32 * step;
                let wz = origin_z + iz as f32 * step;
                let wy = hmap.sample(wx, wz);
                verts.push(Vec3::new(wx, wy, wz));
            }
        }

        // Triangulate: two triangles per quad.
        let quads = CHUNK_RES - 1;
        let mut faces = Vec::with_capacity(quads * quads * 2);
        for iz in 0..quads {
            for ix in 0..quads {
                let i00 = iz * CHUNK_RES + ix;
                let i10 = i00 + 1;
                let i01 = i00 + CHUNK_RES;
                let i11 = i01 + 1;
                faces.push([i00, i01, i10]);
                faces.push([i10, i01, i11]);
            }
        }

        // Compute smooth normals by averaging adjacent face normals.
        let mut normals = vec![Vec3::ZERO; verts.len()];
        for face in &faces {
            let a = verts[face[0]];
            let b = verts[face[1]];
            let c = verts[face[2]];
            let e1 = b - a;
            let e2 = c - a;
            let fn_ = e1.cross(e2); // un-normalized = area-weighted
            normals[face[0]] = normals[face[0]] + fn_;
            normals[face[1]] = normals[face[1]] + fn_;
            normals[face[2]] = normals[face[2]] + fn_;
        }
        for n in &mut normals {
            *n = n.normalize();
        }

        Self {
            verts,
            normals,
            faces,
            cx,
            cz,
        }
    }
}

// ── Terrain (chunk manager) ───────────────────────────────────────────────────

/// Manages a set of terrain chunks around the camera.
pub struct Terrain {
    pub heightmap: Heightmap,
    /// Loaded chunks, keyed implicitly by (cx, cz) stored inside each chunk.
    pub chunks: Vec<TerrainChunk>,
    /// How many chunks out from the camera chunk to keep loaded (Chebyshev
    /// distance).  `view_radius = 3` means a 7x7 grid.
    pub view_radius: i32,
    /// Base colour of the terrain surface.
    pub color: Color,
}

impl Terrain {
    pub fn new(heightmap: Heightmap, view_radius: i32, color: Color) -> Self {
        Self {
            heightmap,
            chunks: Vec::new(),
            view_radius,
            color,
        }
    }

    /// Call once per frame with the camera's world-space position.
    /// Loads new chunks and unloads chunks that have moved out of range.
    pub fn update(&mut self, camera_pos: Vec3) {
        let cam_cx = (camera_pos.x / CHUNK_SIZE).floor() as i32;
        let cam_cz = (camera_pos.z / CHUNK_SIZE).floor() as i32;
        let r = self.view_radius;

        // Remove chunks outside the radius.
        self.chunks
            .retain(|c| (c.cx - cam_cx).abs() <= r && (c.cz - cam_cz).abs() <= r);

        // Add missing chunks.
        for cz in (cam_cz - r)..=(cam_cz + r) {
            for cx in (cam_cx - r)..=(cam_cx + r) {
                if !self.chunks.iter().any(|c| c.cx == cx && c.cz == cz) {
                    self.chunks
                        .push(TerrainChunk::build(&self.heightmap, cx, cz));
                }
            }
        }
    }

    /// Total number of currently loaded chunks.
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Height at an arbitrary world-space (x, z) — delegates to the heightmap.
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        self.heightmap.sample(x, z)
    }

    /// World-space size of one chunk edge.
    pub fn chunk_size() -> f32 {
        CHUNK_SIZE
    }
}
