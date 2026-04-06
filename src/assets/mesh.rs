use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

// ── Vertex ────────────────────────────────────────────────────────────────────

/// Interleaved GPU vertex: position + normal + UV.
/// 32 bytes, matches the OpenGL attribute layout in the engine shaders.
///
/// Byte layout (matches shaders):
///   location 0 — pos    [f32; 3]   offset  0
///   location 1 — normal [f32; 3]   offset 12
///   location 2 — uv     [f32; 2]   offset 24
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Vertex {
    pub pos:    [f32; 3],
    pub normal: [f32; 3],
    pub uv:     [f32; 2],
}

// ── MeshData ──────────────────────────────────────────────────────────────────

/// CPU-side mesh: vertex buffer + u32 index buffer ready to upload to the GPU.
///
/// Create via [`MeshData::from_obj`] or [`MeshData::from_raw`], then call
/// `renderer.upload_mesh(&mesh)` to get a drawable [`GpuMesh`].
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices:  Vec<u32>,
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
    /// # fn demo() -> Result<(), Box<dyn std::error::Error>> {
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

    pub fn vertex_count(&self)   -> usize { self.vertices.len() }
    pub fn triangle_count(&self) -> usize { self.indices.len() / 3 }
}

// ── ObjError ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ObjError {
    Io(std::io::Error),
    ParseFloat { line: usize, value: String },
    ParseInt   { line: usize, value: String },
    InvalidFace { line: usize },
    EmptyMesh,
}

impl fmt::Display for ObjError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObjError::Io(e)                      => write!(f, "IO error: {e}"),
            ObjError::ParseFloat { line, value } => write!(f, "line {line}: expected float, got {value:?}"),
            ObjError::ParseInt   { line, value } => write!(f, "line {line}: expected integer, got {value:?}"),
            ObjError::InvalidFace { line }       => write!(f, "line {line}: malformed face"),
            ObjError::EmptyMesh                  => write!(f, "OBJ contains no geometry"),
        }
    }
}

impl std::error::Error for ObjError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let ObjError::Io(e) = self { Some(e) } else { None }
    }
}

// ── Parser ────────────────────────────────────────────────────────────────────

fn load_obj(path: &Path) -> Result<MeshData, ObjError> {
    let file   = File::open(path).map_err(ObjError::Io)?;
    let reader = BufReader::new(file);

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut uvs:       Vec<[f32; 2]> = Vec::new();
    let mut normals:   Vec<[f32; 3]> = Vec::new();

    // Unified vertex deduplication.
    // Key = (pos_idx, uv_idx, norm_idx) all 1-based; 0 means absent.
    let mut vertex_map: HashMap<(u32, u32, u32), u32> = HashMap::new();
    let mut vertices:   Vec<Vertex> = Vec::new();
    let mut indices:    Vec<u32>    = Vec::new();
    let mut has_normals = false;

    for (ln, line_result) in reader.lines().enumerate() {
        let line_num = ln + 1;
        let raw      = line_result.map_err(ObjError::Io)?;
        let line     = raw.trim();

        if line.starts_with('#') || line.is_empty() { continue; }

        let (keyword, rest) = match line.split_once(char::is_whitespace) {
            Some((k, r)) => (k, r.trim()),
            None         => (line, ""),
        };

        match keyword {
            "v" => {
                positions.push(parse_vec3(rest, line_num)?);
            }
            "vt" => {
                let mut it = rest.split_whitespace();
                let u = parse_f32(it.next(), line_num)?;
                let v = it.next()
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
                    .map(|s| parse_face_vert(s, line_num,
                                             positions.len(),
                                             uvs.len(),
                                             normals.len()))
                    .collect::<Result<_, _>>()?;

                // Fan-triangulate: triangle fan from corner 0.
                for i in 1..parsed.len() - 1 {
                    for &(pi, ui, ni) in &[parsed[0], parsed[i], parsed[i + 1]] {
                        let key = (pi as u32, ui as u32, ni as u32);
                        let dense_idx = *vertex_map.entry(key).or_insert_with(|| {
                            let pos    = positions.get(pi.wrapping_sub(1)).copied().unwrap_or([0.0; 3]);
                            let uv     = uvs.get(ui.wrapping_sub(1)).copied().unwrap_or([0.0; 2]);
                            let normal = normals.get(ni.wrapping_sub(1)).copied().unwrap_or([0.0, 1.0, 0.0]);
                            let idx    = vertices.len() as u32;
                            vertices.push(Vertex { pos, normal, uv });
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
    s.parse::<f32>().map_err(|_| ObjError::ParseFloat { line, value: s.to_owned() })
}

/// Parse one face vertex token: `v`, `v/vt`, `v//vn`, or `v/vt/vn`.
/// Returns 1-based indices; 0 means the component was absent.
fn parse_face_vert(
    token:  &str,
    line:   usize,
    n_pos:  usize,
    n_uv:   usize,
    n_norm: usize,
) -> Result<(usize, usize, usize), ObjError> {
    let mut parts = token.split('/');

    let pi = resolve_index(parts.next(), line, n_pos)?;
    let ui = match parts.next() {
        Some(s) if !s.is_empty() => resolve_index(Some(s), line, n_uv)?,
        _                        => 0,
    };
    let ni = match parts.next() {
        Some(s) if !s.is_empty() => resolve_index(Some(s), line, n_norm)?,
        _                        => 0,
    };

    Ok((pi, ui, ni))
}

/// Converts a possibly-negative OBJ index to a 1-based positive index.
fn resolve_index(s: Option<&str>, line: usize, count: usize) -> Result<usize, ObjError> {
    let s = s.ok_or(ObjError::InvalidFace { line })?;
    let i: i32 = s.parse()
        .map_err(|_| ObjError::ParseInt { line, value: s.to_owned() })?;
    if i > 0 {
        Ok(i as usize)
    } else if i < 0 {
        let abs = (-i) as usize;
        if abs > count { return Err(ObjError::InvalidFace { line }); }
        Ok(count + 1 - abs)
    } else {
        Err(ObjError::InvalidFace { line })
    }
}

// ── Normal generation ─────────────────────────────────────────────────────────

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

        let e1 = [p1[0]-p0[0], p1[1]-p0[1], p1[2]-p0[2]];
        let e2 = [p2[0]-p0[0], p2[1]-p0[1], p2[2]-p0[2]];

        // Cross product — magnitude proportional to triangle area (angle-weighted).
        let nx = e1[1]*e2[2] - e1[2]*e2[1];
        let ny = e1[2]*e2[0] - e1[0]*e2[2];
        let nz = e1[0]*e2[1] - e1[1]*e2[0];

        for &i in &[i0, i1, i2] {
            vertices[i].normal[0] += nx;
            vertices[i].normal[1] += ny;
            vertices[i].normal[2] += nz;
        }
    }

    for v in vertices.iter_mut() {
        let [nx, ny, nz] = v.normal;
        let len = (nx*nx + ny*ny + nz*nz).sqrt();
        v.normal = if len > 1e-10 {
            [nx/len, ny/len, nz/len]
        } else {
            [0.0, 1.0, 0.0]
        };
    }
}
