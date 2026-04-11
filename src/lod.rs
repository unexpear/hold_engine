//! Level-of-Detail system.
//!
//! Two complementary pieces:
//!
//! 1. [LodMesh] \u2014 a generic multi-level mesh container.  Each level stores
//!    verts/normals/faces and a maximum view distance.  At render time you call
//!    [LodMesh::select] with the camera-to-object distance and get back the
//!    appropriate level (or None if the object is too far away to draw).
//!
//! 2. [LodChunk] \u2014 a terrain-specific multi-resolution chunk.  It stores 2\u20133
//!    pre-built resolutions of the same chunk and picks one based on Chebyshev
//!    distance from the camera chunk.

use crate::math::Vec3;
use crate::terrain::Heightmap;

// ============================================================================
//  Generic LOD mesh
// ============================================================================

/// A single level within a [LodMesh].
pub struct LodLevel {
    pub verts: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub faces: Vec<[usize; 3]>,
    /// Objects farther than this use the *next* (coarser) level.
    /// The last level has max_dist = f32::INFINITY.
    pub max_dist: f32,
}

/// Multi-level mesh.  Levels are sorted finest (index 0) to coarsest.
pub struct LodMesh {
    pub levels: Vec<LodLevel>,
}

impl LodMesh {
    /// Create from a list of (verts, normals, faces, max_distance) tuples,
    /// ordered from finest to coarsest.
    pub fn new(levels: Vec<LodLevel>) -> Self {
        Self { levels }
    }

    /// Pick the level to render for a given camera\u2013object distance.
    /// Returns None only if levels is empty.
    pub fn select(&self, dist: f32) -> Option<&LodLevel> {
        for lv in &self.levels {
            if dist <= lv.max_dist {
                return Some(lv);
            }
        }
        self.levels.last()
    }

    /// Convenience: number of triangles in the level that would be chosen.
    pub fn tri_count_at(&self, dist: f32) -> usize {
        self.select(dist).map_or(0, |lv| lv.faces.len())
    }
}

// ============================================================================
//  Terrain LOD chunk  (multi-resolution)
// ============================================================================

/// World-space size of one chunk edge (must match terrain::CHUNK_SIZE).
const CHUNK_SIZE: f32 = 16.0;

/// Resolution tiers: (grid samples per edge, Chebyshev-distance threshold).
/// Chunks closer than the threshold use that resolution.
/// The last tier covers everything beyond.
const LOD_TIERS: [(usize, i32); 3] = [
    (17, 1),  // 17x17  for distance <= 1 chunk
    (9, 2),   // 9x9    for distance <= 2 chunks
    (5, 999), // 5x5    for everything farther
];
const MIN_SKIRT_DEPTH: f32 = 1.0;
const SKIRT_DEPTH_SCALE: f32 = 1.5;

/// A single resolution tier stored inside [LodChunk].
pub struct ChunkTier {
    pub verts: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub faces: Vec<[usize; 3]>,
    pub res: usize,
}

/// Terrain chunk that carries multiple pre-built resolution tiers.
pub struct LodChunk {
    pub cx: i32,
    pub cz: i32,
    pub tiers: Vec<ChunkTier>,
}

impl LodChunk {
    /// Build all LOD tiers for the chunk at grid position (cx, cz).
    pub fn build(hmap: &Heightmap, cx: i32, cz: i32) -> Self {
        let mut tiers = Vec::with_capacity(LOD_TIERS.len());
        for &(res, _) in &LOD_TIERS {
            tiers.push(Self::build_tier(hmap, cx, cz, res));
        }
        Self { cx, cz, tiers }
    }

    /// Select the appropriate tier index for a given Chebyshev distance
    /// (in chunk coordinates) from the camera.
    pub fn tier_index(dist: i32) -> usize {
        for (i, &(_, threshold)) in LOD_TIERS.iter().enumerate() {
            if dist <= threshold {
                return i;
            }
        }
        LOD_TIERS.len() - 1
    }

    /// Convenience: return the tier for a given chunk distance.
    pub fn tier_for_dist(&self, dist: i32) -> &ChunkTier {
        &self.tiers[Self::tier_index(dist)]
    }

    fn build_tier(hmap: &Heightmap, cx: i32, cz: i32, res: usize) -> ChunkTier {
        let origin_x = cx as f32 * CHUNK_SIZE;
        let origin_z = cz as f32 * CHUNK_SIZE;
        let step = CHUNK_SIZE / (res - 1) as f32;

        let mut verts = Vec::with_capacity(res * res);
        for iz in 0..res {
            for ix in 0..res {
                let wx = origin_x + ix as f32 * step;
                let wz = origin_z + iz as f32 * step;
                let wy = hmap.sample(wx, wz);
                verts.push(Vec3::new(wx, wy, wz));
            }
        }

        let quads = res - 1;
        let mut faces = Vec::with_capacity(quads * quads * 2);
        for iz in 0..quads {
            for ix in 0..quads {
                let i00 = iz * res + ix;
                let i10 = i00 + 1;
                let i01 = i00 + res;
                let i11 = i01 + 1;
                faces.push([i00, i01, i10]);
                faces.push([i10, i01, i11]);
            }
        }

        let mut normals = vec![Vec3::ZERO; verts.len()];
        for face in &faces {
            let a = verts[face[0]];
            let b = verts[face[1]];
            let c = verts[face[2]];
            let e1 = b - a;
            let e2 = c - a;
            let fn_ = e1.cross(e2);
            normals[face[0]] = normals[face[0]] + fn_;
            normals[face[1]] = normals[face[1]] + fn_;
            normals[face[2]] = normals[face[2]] + fn_;
        }
        for n in &mut normals {
            *n = n.normalize();
        }

        let skirt_depth = (hmap.amplitude.abs() * SKIRT_DEPTH_SCALE)
            .max(step * 2.0)
            .max(MIN_SKIRT_DEPTH);
        append_skirt(
            &mut verts,
            &mut normals,
            &mut faces,
            &(0..res).collect::<Vec<_>>(),
            skirt_depth,
        );
        append_skirt(
            &mut verts,
            &mut normals,
            &mut faces,
            &((0..res).map(|iz| iz * res + (res - 1)).collect::<Vec<_>>()),
            skirt_depth,
        );
        append_skirt(
            &mut verts,
            &mut normals,
            &mut faces,
            &((0..res).map(|ix| (res - 1) * res + ix).collect::<Vec<_>>()),
            skirt_depth,
        );
        append_skirt(
            &mut verts,
            &mut normals,
            &mut faces,
            &((0..res).map(|iz| iz * res).collect::<Vec<_>>()),
            skirt_depth,
        );

        ChunkTier {
            verts,
            normals,
            faces,
            res,
        }
    }
}

fn append_skirt(
    verts: &mut Vec<Vec3>,
    normals: &mut Vec<Vec3>,
    faces: &mut Vec<[usize; 3]>,
    edge: &[usize],
    depth: f32,
) {
    let mut top = Vec::with_capacity(edge.len());
    let mut bottom = Vec::with_capacity(edge.len());

    for &index in edge {
        let vertex = verts[index];
        let normal = normals[index];
        top.push(verts.len());
        verts.push(vertex);
        normals.push(normal);

        bottom.push(verts.len());
        verts.push(Vec3::new(vertex.x, vertex.y - depth, vertex.z));
        normals.push(normal);
    }

    for i in 0..(edge.len() - 1) {
        let t0 = top[i];
        let t1 = top[i + 1];
        let b0 = bottom[i];
        let b1 = bottom[i + 1];

        // Draw both windings so the skirts still cover cracks from either side.
        faces.push([t0, b0, t1]);
        faces.push([t1, b0, b1]);
        faces.push([t1, b0, t0]);
        faces.push([b1, b0, t1]);
    }
}

/// Terrain manager that uses [LodChunk] for distance-based mesh quality.
pub struct LodTerrain {
    pub heightmap: Heightmap,
    pub chunks: Vec<LodChunk>,
    pub view_radius: i32,
    pub color: crate::renderer::Color,
}

impl LodTerrain {
    pub fn new(heightmap: Heightmap, view_radius: i32, color: crate::renderer::Color) -> Self {
        Self {
            heightmap,
            chunks: Vec::new(),
            view_radius,
            color,
        }
    }

    /// Update loaded chunks around the camera position.
    pub fn update(&mut self, camera_pos: Vec3) {
        let cam_cx = (camera_pos.x / CHUNK_SIZE).floor() as i32;
        let cam_cz = (camera_pos.z / CHUNK_SIZE).floor() as i32;
        let r = self.view_radius;

        self.chunks
            .retain(|c| (c.cx - cam_cx).abs() <= r && (c.cz - cam_cz).abs() <= r);

        for cz in (cam_cz - r)..=(cam_cz + r) {
            for cx in (cam_cx - r)..=(cam_cx + r) {
                if !self.chunks.iter().any(|c| c.cx == cx && c.cz == cz) {
                    self.chunks.push(LodChunk::build(&self.heightmap, cx, cz));
                }
            }
        }
    }

    /// Height at world-space (x, z).
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        self.heightmap.sample(x, z)
    }

    /// Calculate the Chebyshev distance in chunk coords from the camera.
    pub fn chunk_distance(cam_pos: Vec3, cx: i32, cz: i32) -> i32 {
        let cam_cx = (cam_pos.x / CHUNK_SIZE).floor() as i32;
        let cam_cz = (cam_pos.z / CHUNK_SIZE).floor() as i32;
        (cx - cam_cx).abs().max((cz - cam_cz).abs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lod_chunk_tiers_include_skirt_geometry() {
        let chunk = LodChunk::build(&Heightmap::default(), 0, 0);

        for tier in &chunk.tiers {
            let top_vertex_count = tier.res * tier.res;
            let top_face_count = (tier.res - 1) * (tier.res - 1) * 2;
            let skirt_vertex_count = tier.res * 8;
            let skirt_face_count = (tier.res - 1) * 16;

            assert_eq!(tier.verts.len(), top_vertex_count + skirt_vertex_count);
            assert_eq!(tier.normals.len(), top_vertex_count + skirt_vertex_count);
            assert_eq!(tier.faces.len(), top_face_count + skirt_face_count);
        }
    }
}
