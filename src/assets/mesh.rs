use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

// ── Vertex ────────────────────────────────────────────────────────────────────

/// Interleaved GPU vertex: position + normal + UV + tangent.
/// 48 bytes, matches the OpenGL attribute layout in the engine shaders.
///
/// Byte layout (matches shaders):
///   location 0 — pos     [f32; 3]   offset  0
///   location 1 — normal  [f32; 3]   offset 12
///   location 2 — uv      [f32; 2]   offset 24
///   location 3 — tangent [f32; 4]   offset 32
///
/// `tangent.xyz` is the surface tangent in object space (the direction of
/// increasing U on the surface). `tangent.w` is the handedness sign for
/// reconstructing the bitangent in the shader as `cross(normal, tangent.xyz)
/// * tangent.w`. This packing matches the MikkTSpace and glTF 2.0 conventions.
///
/// See `docs/adr/0002-material-pipeline.md` Q4 for the rationale (typed
/// normal-map pipeline; OpenGL Y+ canonical convention; in-house
/// MikkTSpace-equivalent tangent generation).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub tangent: [f32; 4],
}

/// Default tangent value used when a vertex's tangent has not been computed
/// yet. Encodes "+X with right-handed bitangent". Real meshes should overwrite
/// this either via [`MeshData::generate_tangents`] or by setting the field
/// directly when constructing procedural geometry.
pub const DEFAULT_TANGENT: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

// ── MeshData ──────────────────────────────────────────────────────────────────

/// CPU-side mesh: vertex buffer + u32 index buffer ready to upload to the GPU.
///
/// Create via [`MeshData::from_obj`] or [`MeshData::from_raw`], then call
/// `renderer.upload_mesh(&mesh)` to get a drawable [`GpuMesh`].
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl MeshData {
    /// Load a Wavefront OBJ file.
    ///
    /// # What is supported
    /// * All face formats: `v`, `v/vt`, `v//vn`, `v/vt/vn`
    /// * Quads and n-gons (fan-triangulated automatically)
    /// * Negative (relative) OBJ indices
    /// * Multiple objects / groups in a single file (merged into one mesh)
    /// * Smooth normals generated when the file has no `vn` entries
    /// * V-coordinate flipped for OpenGL (OBJ origin = top-left, GL = bottom-left)
    ///
    /// # Errors
    /// Returns an [`ObjError`] with the line number and a human-readable message.
    ///
    /// # Example
    /// ```no_run
    /// use hold::MeshData;
    ///
    /// # fn main() -> Result<(), hold::ObjError> {
    /// let mesh = MeshData::from_obj("assets/models/sword.obj")?;
    /// println!("{} vertices, {} triangles", mesh.vertex_count(), mesh.triangle_count());
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_obj<P: AsRef<Path>>(path: P) -> Result<Self, ObjError> {
        load_obj(path.as_ref())
    }

    /// Build a `MeshData` directly from pre-built arrays (procedural geometry).
    pub fn from_raw(vertices: Vec<Vertex>, indices: Vec<u32>) -> Self {
        Self { vertices, indices }
    }

    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Recompute per-vertex tangents in place. Call this after mutating
    /// positions, normals, UVs, or topology if the mesh wasn't loaded via the
    /// OBJ path (which already runs tangent generation on import).
    ///
    /// See [`generate_tangents`] for the algorithm and the MikkTSpace-upgrade
    /// note tracked under ADR 0002 Q4.
    pub fn generate_tangents(&mut self) {
        generate_tangents(&mut self.vertices, &self.indices);
    }

    // ── Procedural primitives ─────────────────────────────────────────────────

    /// Unit cube centred at the origin (half-extents 0.5 on each axis).
    /// Each face has its own 4 vertices so normals are hard/flat per face.
    /// Produces 24 vertices and 36 indices.
    ///
    /// Tangents are exact per-face: each tangent points in the world-space
    /// direction of increasing U on that face (right-handed wrt the face
    /// normal and the bitangent).
    pub fn cube() -> Self {
        // (corners[4], normal, tangent)
        // - corners are in CCW order viewed from outside.
        // - tangent is the +U direction on this face (xyz only; w handedness
        //   added below).
        type FaceDef = ([([f32; 3], [f32; 2]); 4], [f32; 3], [f32; 3]);
        let faces: [FaceDef; 6] = [
            // +Z face: U runs along +X
            (
                [
                    ([-0.5, -0.5, 0.5], [0.0, 0.0]),
                    ([0.5, -0.5, 0.5], [1.0, 0.0]),
                    ([0.5, 0.5, 0.5], [1.0, 1.0]),
                    ([-0.5, 0.5, 0.5], [0.0, 1.0]),
                ],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 0.0],
            ),
            // -Z face: U runs along -X
            (
                [
                    ([0.5, -0.5, -0.5], [0.0, 0.0]),
                    ([-0.5, -0.5, -0.5], [1.0, 0.0]),
                    ([-0.5, 0.5, -0.5], [1.0, 1.0]),
                    ([0.5, 0.5, -0.5], [0.0, 1.0]),
                ],
                [0.0, 0.0, -1.0],
                [-1.0, 0.0, 0.0],
            ),
            // +X face: U runs along -Z
            (
                [
                    ([0.5, -0.5, 0.5], [0.0, 0.0]),
                    ([0.5, -0.5, -0.5], [1.0, 0.0]),
                    ([0.5, 0.5, -0.5], [1.0, 1.0]),
                    ([0.5, 0.5, 0.5], [0.0, 1.0]),
                ],
                [1.0, 0.0, 0.0],
                [0.0, 0.0, -1.0],
            ),
            // -X face: U runs along +Z
            (
                [
                    ([-0.5, -0.5, -0.5], [0.0, 0.0]),
                    ([-0.5, -0.5, 0.5], [1.0, 0.0]),
                    ([-0.5, 0.5, 0.5], [1.0, 1.0]),
                    ([-0.5, 0.5, -0.5], [0.0, 1.0]),
                ],
                [-1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
            ),
            // +Y face: U runs along +X
            (
                [
                    ([-0.5, 0.5, 0.5], [0.0, 0.0]),
                    ([0.5, 0.5, 0.5], [1.0, 0.0]),
                    ([0.5, 0.5, -0.5], [1.0, 1.0]),
                    ([-0.5, 0.5, -0.5], [0.0, 1.0]),
                ],
                [0.0, 1.0, 0.0],
                [1.0, 0.0, 0.0],
            ),
            // -Y face: U runs along +X
            (
                [
                    ([-0.5, -0.5, -0.5], [0.0, 0.0]),
                    ([0.5, -0.5, -0.5], [1.0, 0.0]),
                    ([0.5, -0.5, 0.5], [1.0, 1.0]),
                    ([-0.5, -0.5, 0.5], [0.0, 1.0]),
                ],
                [0.0, -1.0, 0.0],
                [1.0, 0.0, 0.0],
            ),
        ];

        let mut vertices = Vec::with_capacity(24);
        let mut indices = Vec::with_capacity(36);

        for (corners, normal, tangent_xyz) in &faces {
            let base = vertices.len() as u32;
            for &(pos, uv) in corners {
                vertices.push(Vertex {
                    pos,
                    normal: *normal,
                    uv,
                    tangent: [tangent_xyz[0], tangent_xyz[1], tangent_xyz[2], 1.0],
                });
            }
            // Two triangles per quad (CCW): 0-1-2 and 0-2-3.
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }

        Self { vertices, indices }
    }

    /// Flat XZ-plane grid centred at the origin with unit total size.
    ///
    /// `cols` × `rows` quads, each subdivided into two CCW triangles.
    /// Normals all point up (+Y). Minimum 1×1 quad.
    pub fn plane(cols: u32, rows: u32) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let nx = (cols + 1) as usize;
        let nz = (rows + 1) as usize;

        let mut vertices = Vec::with_capacity(nx * nz);
        let mut indices = Vec::with_capacity((cols * rows * 6) as usize);

        for zi in 0..nz {
            for xi in 0..nx {
                let u = xi as f32 / cols as f32;
                let v = zi as f32 / rows as f32;
                vertices.push(Vertex {
                    pos: [u - 0.5, 0.0, v - 0.5],
                    normal: [0.0, 1.0, 0.0],
                    uv: [u, 1.0 - v],
                    // Plane lies in XZ; U increases with +X so tangent = +X.
                    tangent: [1.0, 0.0, 0.0, 1.0],
                });
            }
        }

        for zi in 0..rows as usize {
            for xi in 0..cols as usize {
                let tl = (zi * nx + xi) as u32;
                let tr = tl + 1;
                let bl = tl + nx as u32;
                let br = bl + 1;
                // CCW triangles facing up: (tl,bl,br) and (tl,br,tr).
                indices.extend_from_slice(&[tl, bl, br, tl, br, tr]);
            }
        }

        Self { vertices, indices }
    }

    /// UV sphere of radius 1 centred at the origin.
    ///
    /// `stacks` horizontal bands (latitude), `slices` vertical divisions
    /// (longitude). Minimums are 2 stacks and 3 slices.
    /// Poles collapse to single triangles, middle quads to two triangles.
    pub fn sphere(stacks: u32, slices: u32) -> Self {
        use std::f32::consts::PI;

        let stacks = stacks.max(2);
        let slices = slices.max(3);
        let row = slices + 1; // vertices per latitude ring (seam duplicated)

        let mut vertices = Vec::with_capacity(((stacks + 1) * row) as usize);
        let mut indices = Vec::with_capacity((stacks * slices * 6) as usize);

        for si in 0..=stacks {
            let phi = PI * si as f32 / stacks as f32; // 0 (top) → π (bottom)
            let y = phi.cos();
            let r = phi.sin();

            for li in 0..=slices {
                let theta = 2.0 * PI * li as f32 / slices as f32;
                let x = r * theta.cos();
                let z = r * theta.sin();
                let u = li as f32 / slices as f32;
                let v = si as f32 / stacks as f32;

                // Tangent = ∂P/∂theta normalized. For a unit sphere with
                // P(θ,φ) = (sinφ cosθ, cosφ, sinφ sinθ), the partial wrt θ
                // is (-sinφ sinθ, 0, sinφ cosθ); after normalizing the
                // sinφ falls out, leaving (-sinθ, 0, cosθ). At the poles
                // (sinφ=0) this is degenerate; we keep the +X direction
                // there since the pole vertices collapse to a single point
                // and tangent ambiguity has no visible effect.
                let tangent_xyz = if r.abs() > 1e-6 {
                    [-theta.sin(), 0.0, theta.cos()]
                } else {
                    [1.0, 0.0, 0.0]
                };

                vertices.push(Vertex {
                    pos: [x, y, z],
                    normal: [x, y, z], // unit sphere: pos == normal
                    uv: [u, 1.0 - v],
                    tangent: [tangent_xyz[0], tangent_xyz[1], tangent_xyz[2], 1.0],
                });
            }
        }

        for si in 0..stacks {
            for li in 0..slices {
                let tl = si * row + li;
                let tr = tl + 1;
                let bl = tl + row;
                let br = bl + 1;

                // At top pole (si==0): tl==tr, emit only bottom triangle.
                if si != 0 {
                    indices.extend_from_slice(&[tl, tr, br]);
                }
                // At bottom pole (si==stacks-1): bl==br, emit only top triangle.
                if si != stacks - 1 {
                    indices.extend_from_slice(&[tl, br, bl]);
                }
            }
        }

        Self { vertices, indices }
    }
}

// ── ObjError ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ObjError {
    Io(std::io::Error),
    ParseFloat { line: usize, value: String },
    ParseInt { line: usize, value: String },
    InvalidFace { line: usize },
    EmptyMesh,
}

impl fmt::Display for ObjError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObjError::Io(e) => write!(f, "IO error: {e}"),
            ObjError::ParseFloat { line, value } => {
                write!(f, "line {line}: expected float, got {value:?}")
            }
            ObjError::ParseInt { line, value } => {
                write!(f, "line {line}: expected integer, got {value:?}")
            }
            ObjError::InvalidFace { line } => write!(f, "line {line}: malformed face"),
            ObjError::EmptyMesh => write!(f, "OBJ contains no geometry"),
        }
    }
}

impl std::error::Error for ObjError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let ObjError::Io(e) = self {
            Some(e)
        } else {
            None
        }
    }
}

// ── Parser ────────────────────────────────────────────────────────────────────

fn load_obj(path: &Path) -> Result<MeshData, ObjError> {
    let file = File::open(path).map_err(ObjError::Io)?;
    let reader = BufReader::new(file);

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();

    // Unified vertex deduplication.
    // Key = (pos_idx, uv_idx, norm_idx) all 1-based; 0 means absent.
    let mut vertex_map: HashMap<(u32, u32, u32), u32> = HashMap::new();
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut has_normals = false;

    for (ln, line_result) in reader.lines().enumerate() {
        let line_num = ln + 1;
        let raw = line_result.map_err(ObjError::Io)?;
        let line = raw.trim();

        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        let (keyword, rest) = match line.split_once(char::is_whitespace) {
            Some((k, r)) => (k, r.trim()),
            None => (line, ""),
        };

        match keyword {
            "v" => {
                positions.push(parse_vec3(rest, line_num)?);
            }
            "vt" => {
                let mut it = rest.split_whitespace();
                let u = parse_f32(it.next(), line_num)?;
                let v = it
                    .next()
                    .map(|s| parse_f32_val(s, line_num))
                    .transpose()?
                    .unwrap_or(0.0);
                uvs.push([u, 1.0 - v]); // flip V for OpenGL
            }
            "vn" => {
                normals.push(parse_vec3(rest, line_num)?);
                has_normals = true;
            }
            "f" => {
                let corners: Vec<&str> = rest.split_whitespace().collect();
                if corners.len() < 3 {
                    return Err(ObjError::InvalidFace { line: line_num });
                }

                // Parse each corner into 1-based (pos, uv, norm) indices.
                let parsed: Vec<(usize, usize, usize)> = corners
                    .iter()
                    .map(|s| {
                        parse_face_vert(s, line_num, positions.len(), uvs.len(), normals.len())
                    })
                    .collect::<Result<_, _>>()?;

                // Fan-triangulate: triangle fan from corner 0.
                for i in 1..parsed.len() - 1 {
                    for &(pi, ui, ni) in &[parsed[0], parsed[i], parsed[i + 1]] {
                        let key = (pi as u32, ui as u32, ni as u32);
                        let dense_idx = *vertex_map.entry(key).or_insert_with(|| {
                            let pos = positions
                                .get(pi.wrapping_sub(1))
                                .copied()
                                .unwrap_or([0.0; 3]);
                            let uv = uvs.get(ui.wrapping_sub(1)).copied().unwrap_or([0.0; 2]);
                            let normal = normals
                                .get(ni.wrapping_sub(1))
                                .copied()
                                .unwrap_or([0.0, 1.0, 0.0]);
                            let idx = vertices.len() as u32;
                            vertices.push(Vertex {
                                pos,
                                normal,
                                uv,
                                tangent: DEFAULT_TANGENT,
                            });
                            idx
                        });
                        indices.push(dense_idx);
                    }
                }
            }
            // mtllib, usemtl, s, g, o, l — silently ignored
            _ => {}
        }
    }

    if vertices.is_empty() {
        return Err(ObjError::EmptyMesh);
    }

    if !has_normals {
        generate_smooth_normals(&mut vertices, &indices);
    }

    // Always generate tangents for OBJ-loaded meshes. OBJ has no tangent
    // channel, so without this every imported mesh would carry the default
    // +X tangent and normal mapping would be visibly wrong on most surfaces.
    generate_tangents(&mut vertices, &indices);

    Ok(MeshData { vertices, indices })
}

// ── Parse helpers ─────────────────────────────────────────────────────────────

fn parse_vec3(rest: &str, line: usize) -> Result<[f32; 3], ObjError> {
    let mut it = rest.split_whitespace();
    let x = parse_f32(it.next(), line)?;
    let y = parse_f32(it.next(), line)?;
    let z = parse_f32(it.next(), line)?;
    Ok([x, y, z])
}

fn parse_f32(s: Option<&str>, line: usize) -> Result<f32, ObjError> {
    let s = s.ok_or(ObjError::InvalidFace { line })?;
    parse_f32_val(s, line)
}

fn parse_f32_val(s: &str, line: usize) -> Result<f32, ObjError> {
    s.parse::<f32>().map_err(|_| ObjError::ParseFloat {
        line,
        value: s.to_owned(),
    })
}

/// Parse one face vertex token: `v`, `v/vt`, `v//vn`, or `v/vt/vn`.
/// Returns 1-based indices; 0 means the component was absent.
fn parse_face_vert(
    token: &str,
    line: usize,
    n_pos: usize,
    n_uv: usize,
    n_norm: usize,
) -> Result<(usize, usize, usize), ObjError> {
    let mut parts = token.split('/');

    let pi = resolve_index(parts.next(), line, n_pos)?;
    let ui = match parts.next() {
        Some(s) if !s.is_empty() => resolve_index(Some(s), line, n_uv)?,
        _ => 0,
    };
    let ni = match parts.next() {
        Some(s) if !s.is_empty() => resolve_index(Some(s), line, n_norm)?,
        _ => 0,
    };

    Ok((pi, ui, ni))
}

/// Converts a possibly-negative OBJ index to a 1-based positive index.
fn resolve_index(s: Option<&str>, line: usize, count: usize) -> Result<usize, ObjError> {
    let s = s.ok_or(ObjError::InvalidFace { line })?;
    let i: i32 = s.parse().map_err(|_| ObjError::ParseInt {
        line,
        value: s.to_owned(),
    })?;
    if i > 0 {
        Ok(i as usize)
    } else if i < 0 {
        let abs = (-i) as usize;
        if abs > count {
            return Err(ObjError::InvalidFace { line });
        }
        Ok(count + 1 - abs)
    } else {
        Err(ObjError::InvalidFace { line })
    }
}

// ── Normal generation ─────────────────────────────────────────────────────────

// ── Tangent generation ───────────────────────────────────────────────────────

/// Compute per-vertex tangents in place from positions, normals, UVs, and
/// indices. Each vertex's tangent ends up orthonormal to its normal and aligned
/// with the surface +U direction, with `tangent.w` carrying the bitangent
/// handedness sign so the fragment shader can reconstruct the bitangent as
/// `cross(normal, tangent.xyz) * tangent.w`.
///
/// Algorithm: Lengyel-style per-triangle tangent and bitangent accumulation,
/// followed by Gram-Schmidt orthonormalization against each vertex normal and a
/// handedness check via the dot of the original (un-orthonormalized) bitangent
/// with `cross(normal, ortho_tangent)`. This is the same skeleton MikkTSpace
/// uses; what MikkTSpace adds is per-corner (rather than per-vertex) tangent
/// resolution and a deterministic angle-weighted accumulation that matches the
/// reference implementation bit-for-bit across DCC tools.
///
/// **TODO (ADR 0002 Q4):** replace this Lengyel-style implementation with the
/// in-house MikkTSpace-equivalent before any normal-mapped content is authored
/// against the engine. This v1 implementation is acceptable for procedural
/// geometry and simple OBJ test meshes; it will produce visible seams on
/// content baked against the MikkTSpace convention.
pub fn generate_tangents(vertices: &mut [Vertex], indices: &[u32]) {
    let n = vertices.len();
    let mut tan_acc = vec![[0.0f32; 3]; n];
    let mut bit_acc = vec![[0.0f32; 3]; n];

    for tri in indices.chunks_exact(3) {
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;

        let p0 = vertices[i0].pos;
        let p1 = vertices[i1].pos;
        let p2 = vertices[i2].pos;

        let uv0 = vertices[i0].uv;
        let uv1 = vertices[i1].uv;
        let uv2 = vertices[i2].uv;

        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];

        let du1 = uv1[0] - uv0[0];
        let dv1 = uv1[1] - uv0[1];
        let du2 = uv2[0] - uv0[0];
        let dv2 = uv2[1] - uv0[1];

        let det = du1 * dv2 - du2 * dv1;
        // Degenerate UVs (zero-area in UV space) — leave the accumulator
        // alone for these triangles. They'll fall back to the per-vertex
        // default during the orthonormalization pass below.
        if det.abs() < 1e-10 {
            continue;
        }
        let inv_det = 1.0 / det;

        let tx = (dv2 * e1[0] - dv1 * e2[0]) * inv_det;
        let ty = (dv2 * e1[1] - dv1 * e2[1]) * inv_det;
        let tz = (dv2 * e1[2] - dv1 * e2[2]) * inv_det;

        let bx = (-du2 * e1[0] + du1 * e2[0]) * inv_det;
        let by = (-du2 * e1[1] + du1 * e2[1]) * inv_det;
        let bz = (-du2 * e1[2] + du1 * e2[2]) * inv_det;

        for &i in &[i0, i1, i2] {
            tan_acc[i][0] += tx;
            tan_acc[i][1] += ty;
            tan_acc[i][2] += tz;
            bit_acc[i][0] += bx;
            bit_acc[i][1] += by;
            bit_acc[i][2] += bz;
        }
    }

    for i in 0..n {
        let n_xyz = vertices[i].normal;
        let t = tan_acc[i];
        let b = bit_acc[i];

        // Gram-Schmidt: orthogonalize accumulated tangent against the normal.
        let n_dot_t = n_xyz[0] * t[0] + n_xyz[1] * t[1] + n_xyz[2] * t[2];
        let mut ot = [
            t[0] - n_xyz[0] * n_dot_t,
            t[1] - n_xyz[1] * n_dot_t,
            t[2] - n_xyz[2] * n_dot_t,
        ];

        let len = (ot[0] * ot[0] + ot[1] * ot[1] + ot[2] * ot[2]).sqrt();
        if len > 1e-6 {
            ot[0] /= len;
            ot[1] /= len;
            ot[2] /= len;
        } else {
            // No usable accumulated tangent — fall back to a vector
            // perpendicular to the normal. Pick the world axis least aligned
            // with the normal to keep the fallback well-conditioned.
            let abs_n = [n_xyz[0].abs(), n_xyz[1].abs(), n_xyz[2].abs()];
            let axis: [f32; 3] = if abs_n[0] <= abs_n[1] && abs_n[0] <= abs_n[2] {
                [1.0, 0.0, 0.0]
            } else if abs_n[1] <= abs_n[2] {
                [0.0, 1.0, 0.0]
            } else {
                [0.0, 0.0, 1.0]
            };
            // Cross axis with normal, then normalize.
            let mut o = [
                axis[1] * n_xyz[2] - axis[2] * n_xyz[1],
                axis[2] * n_xyz[0] - axis[0] * n_xyz[2],
                axis[0] * n_xyz[1] - axis[1] * n_xyz[0],
            ];
            let l = (o[0] * o[0] + o[1] * o[1] + o[2] * o[2]).sqrt();
            if l > 1e-6 {
                o[0] /= l;
                o[1] /= l;
                o[2] /= l;
                ot = o;
            } else {
                // Truly degenerate; use a fixed default so the math downstream
                // doesn't see NaN.
                ot = [1.0, 0.0, 0.0];
            }
        }

        // Handedness: compare cross(normal, ot) against the accumulated
        // bitangent. If they disagree (mirrored UVs), w = -1.
        let nxt = [
            n_xyz[1] * ot[2] - n_xyz[2] * ot[1],
            n_xyz[2] * ot[0] - n_xyz[0] * ot[2],
            n_xyz[0] * ot[1] - n_xyz[1] * ot[0],
        ];
        let dot = nxt[0] * b[0] + nxt[1] * b[1] + nxt[2] * b[2];
        let w = if dot < 0.0 { -1.0 } else { 1.0 };

        vertices[i].tangent = [ot[0], ot[1], ot[2], w];
    }
}

/// Accumulate weighted face normals per vertex, then normalise.
/// Gives smooth shading where vertices are shared across faces.
fn generate_smooth_normals(vertices: &mut [Vertex], indices: &[u32]) {
    for v in vertices.iter_mut() {
        v.normal = [0.0, 0.0, 0.0];
    }

    for tri in indices.chunks_exact(3) {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        let p0 = vertices[i0].pos;
        let p1 = vertices[i1].pos;
        let p2 = vertices[i2].pos;

        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];

        // Cross product — magnitude proportional to triangle area (angle-weighted).
        let nx = e1[1] * e2[2] - e1[2] * e2[1];
        let ny = e1[2] * e2[0] - e1[0] * e2[2];
        let nz = e1[0] * e2[1] - e1[1] * e2[0];

        for &i in &[i0, i1, i2] {
            vertices[i].normal[0] += nx;
            vertices[i].normal[1] += ny;
            vertices[i].normal[2] += nz;
        }
    }

    for v in vertices.iter_mut() {
        let [nx, ny, nz] = v.normal;
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        v.normal = if len > 1e-10 {
            [nx / len, ny / len, nz / len]
        } else {
            [0.0, 1.0, 0.0]
        };
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    fn len3(v: [f32; 3]) -> f32 {
        dot3(v, v).sqrt()
    }

    fn approx(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() < eps
    }

    /// Vertex byte size must be 48 bytes (3+3+2+4 = 12 f32). Anything else
    /// would silently mis-stride the OpenGL VAO setup.
    #[test]
    fn vertex_layout_is_48_bytes() {
        assert_eq!(std::mem::size_of::<Vertex>(), 48);
    }

    /// The cube ships with hand-authored tangents; every vertex's tangent
    /// must be a unit vector orthogonal to its normal, with handedness +1.
    #[test]
    fn cube_tangents_are_orthonormal_to_normals() {
        let mesh = MeshData::cube();
        for v in &mesh.vertices {
            let t_xyz = [v.tangent[0], v.tangent[1], v.tangent[2]];
            assert!(
                approx(len3(t_xyz), 1.0, 1e-5),
                "cube tangent {:?} is not unit length",
                t_xyz
            );
            assert!(
                approx(dot3(t_xyz, v.normal), 0.0, 1e-5),
                "cube tangent {:?} is not orthogonal to normal {:?}",
                t_xyz,
                v.normal
            );
            assert_eq!(
                v.tangent[3], 1.0,
                "cube tangents should all be right-handed (w = 1)"
            );
        }
    }

    /// The plane lies in XZ with U increasing along +X, so every tangent
    /// must be exactly (1, 0, 0, 1).
    #[test]
    fn plane_tangents_match_u_axis() {
        let mesh = MeshData::plane(4, 4);
        for v in &mesh.vertices {
            assert_eq!(v.tangent, [1.0, 0.0, 0.0, 1.0], "plane tangent drift");
        }
    }

    /// Sphere tangents must be orthogonal to the normal at every non-pole
    /// vertex and unit length everywhere. Poles are allowed to fall back
    /// to the (1,0,0) default since the parameterisation is degenerate.
    #[test]
    fn sphere_tangents_are_orthonormal_to_normals() {
        let mesh = MeshData::sphere(8, 12);
        for v in &mesh.vertices {
            let t_xyz = [v.tangent[0], v.tangent[1], v.tangent[2]];
            assert!(
                approx(len3(t_xyz), 1.0, 1e-5),
                "sphere tangent {:?} is not unit length",
                t_xyz
            );
            // Pole vertices have normals near ±Y; tangent there is the
            // fallback (1,0,0), which is orthogonal to ±Y too — so the
            // dot-product check holds for every vertex on a unit sphere.
            assert!(
                approx(dot3(t_xyz, v.normal), 0.0, 1e-4),
                "sphere tangent {:?} not orthogonal to normal {:?}",
                t_xyz,
                v.normal
            );
        }
    }

    /// `generate_tangents` over a single triangle in the XY plane with UVs
    /// aligned to world axes must produce tangent = +X, bitangent sign +1.
    #[test]
    fn generate_tangents_canonical_triangle() {
        let mut verts = vec![
            Vertex {
                pos: [0.0, 0.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [0.0, 0.0],
                tangent: DEFAULT_TANGENT,
            },
            Vertex {
                pos: [1.0, 0.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [1.0, 0.0],
                tangent: DEFAULT_TANGENT,
            },
            Vertex {
                pos: [0.0, 1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [0.0, 1.0],
                tangent: DEFAULT_TANGENT,
            },
        ];
        let indices = vec![0u32, 1, 2];
        generate_tangents(&mut verts, &indices);
        for v in &verts {
            assert!(approx(v.tangent[0], 1.0, 1e-5), "tangent x = {}", v.tangent[0]);
            assert!(approx(v.tangent[1], 0.0, 1e-5), "tangent y = {}", v.tangent[1]);
            assert!(approx(v.tangent[2], 0.0, 1e-5), "tangent z = {}", v.tangent[2]);
            assert_eq!(v.tangent[3], 1.0, "handedness should be +1 for canonical UVs");
        }
    }

    /// Mirroring the V axis flips the bitangent direction; `generate_tangents`
    /// must report this via the handedness sign w = -1.
    #[test]
    fn generate_tangents_detects_mirrored_uvs() {
        let mut verts = vec![
            Vertex {
                pos: [0.0, 0.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [0.0, 0.0],
                tangent: DEFAULT_TANGENT,
            },
            Vertex {
                pos: [1.0, 0.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [1.0, 0.0],
                tangent: DEFAULT_TANGENT,
            },
            Vertex {
                pos: [0.0, 1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [0.0, -1.0], // V mirrored
                tangent: DEFAULT_TANGENT,
            },
        ];
        let indices = vec![0u32, 1, 2];
        generate_tangents(&mut verts, &indices);
        for v in &verts {
            assert_eq!(
                v.tangent[3], -1.0,
                "mirrored-V triangle must report handedness -1"
            );
        }
    }
}
