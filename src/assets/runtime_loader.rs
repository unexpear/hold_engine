use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use super::wire::{
    write_bool, write_f32, write_option_string, write_string, write_u32, write_u64, ByteReader,
};
use super::{
    decode_compiled_asset, AssetStore, ColorSpace, CompiledAssetBlob, CompiledAssetError,
    CompiledAssetKind, Material, MaterialHandle, MeshData, MeshHandle, NormalMap,
    NormalMapConvention, NormalMapHandle, ShaderHandle, Texture, TextureFormat, TextureHandle,
    Vertex,
};

// ── Per-kind payload versions (M10.2) ─────────────────────────────────────
//
// Each compiled-payload kind carries its own independent version field.
// Before M10.2 a single shared `COMPILED_PAYLOAD_VERSION` counter was
// written by every kind, so changing one kind's layout re-versioned all
// five and a version number alone could not say which kind had changed.
// Splitting the counter means a future mesh change bumps only
// MESH_PAYLOAD_VERSION, leaving texture/material/shader/scene blobs valid,
// and a migration table can be keyed by (kind, version).
//
// All five start at 3: the shared-counter era ended at version 3 (v1 =
// initial mesh pos/normal/uv; v2 = mesh tangent, ADR 0002 Q4; v3 = texture
// ColorSpace tag, ADR 0002 Q3), so every kind's current on-disk shape is
// "version 3". Keeping them all at 3 makes the split byte-for-byte
// wire-preserving (guarded by the golden fixtures). Later milestones bump
// individual kinds: M10.4 takes the texture payload to 4 for normal-map
// convention travel.
//
// See `docs/formats/compiled-asset.md` for the per-kind version policy.
const MESH_PAYLOAD_VERSION: u32 = 3;
/// Texture payload version. Bumped 3 -> 4 in M10.4 to carry a
/// [`NormalMapConvention`] tag after the colour-space field, so a
/// normal map's authored convention travels through the compiled
/// pipeline instead of being assumed at the resolution boundary. v3
/// payloads (no tag) migrate by defaulting to OpenGL — the exact
/// behaviour before M10.4, so existing v3 texture blobs load
/// identically.
const TEXTURE_PAYLOAD_VERSION: u32 = 4;
const MATERIAL_PAYLOAD_VERSION: u32 = 3;
const SHADER_PAYLOAD_VERSION: u32 = 3;
const SCENE_PAYLOAD_VERSION: u32 = 3;

/// The current on-disk payload version for a given asset kind, or
/// `None` for kinds that don't yet have an encoded payload
/// (TerrainLayerSet, VegetationPrototype, Skeleton, AnimationClip,
/// AudioClip — reserved in [`CompiledAssetKind`] but not implemented).
///
/// Used by version-aware staleness (M10.3): a compiled blob whose
/// payload version differs from this is out of date and must be
/// recompiled from source, even if its file mtime is newer. Reading
/// this alongside the envelope version closes both format axes.
pub(crate) fn expected_payload_version(kind: CompiledAssetKind) -> Option<u32> {
    match kind {
        CompiledAssetKind::Mesh => Some(MESH_PAYLOAD_VERSION),
        // Texture and normal maps share the Texture kind + payload.
        CompiledAssetKind::Texture => Some(TEXTURE_PAYLOAD_VERSION),
        CompiledAssetKind::Material => Some(MATERIAL_PAYLOAD_VERSION),
        CompiledAssetKind::Shader => Some(SHADER_PAYLOAD_VERSION),
        CompiledAssetKind::Scene => Some(SCENE_PAYLOAD_VERSION),
        CompiledAssetKind::TerrainLayerSet
        | CompiledAssetKind::VegetationPrototype
        | CompiledAssetKind::Skeleton
        | CompiledAssetKind::AnimationClip
        | CompiledAssetKind::AudioClip => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledMaterialPayload {
    pub albedo_logical_name: String,
    pub normal_logical_name: Option<String>,
    pub specular_gloss_logical_name: Option<String>,
    pub double_sided: bool,
    pub alpha_cutout: bool,
}

#[derive(Debug)]
pub enum CompiledAssetRuntimeError {
    Io(std::io::Error),
    Envelope(CompiledAssetError),
    UnexpectedKind {
        expected: CompiledAssetKind,
        found: CompiledAssetKind,
    },
    MetadataMismatch {
        expected_logical_name: String,
        found_logical_name: String,
    },
}

impl fmt::Display for CompiledAssetRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "compiled asset IO failed: {err}"),
            Self::Envelope(err) => write!(f, "compiled asset decode failed: {err}"),
            Self::UnexpectedKind { expected, found } => {
                write!(
                    f,
                    "compiled asset kind mismatch: expected {:?}, found {:?}",
                    expected, found
                )
            }
            Self::MetadataMismatch {
                expected_logical_name,
                found_logical_name,
            } => write!(
                f,
                "compiled asset logical name mismatch: expected '{expected_logical_name}', found '{found_logical_name}'"
            ),
        }
    }
}

impl std::error::Error for CompiledAssetRuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Envelope(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for CompiledAssetRuntimeError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<CompiledAssetError> for CompiledAssetRuntimeError {
    fn from(value: CompiledAssetError) -> Self {
        Self::Envelope(value)
    }
}

pub fn compiled_asset_path_for_logical_name(compiled_root: &Path, logical_name: &str) -> PathBuf {
    let mut path = compiled_root.to_path_buf();
    for segment in logical_name.split('/') {
        path.push(segment);
    }
    path.set_extension("hcmp");
    path
}

pub fn encode_compiled_mesh_payload(mesh: &MeshData) -> Result<Vec<u8>, CompiledAssetError> {
    let vertex_count =
        u32::try_from(mesh.vertices.len()).map_err(|_| CompiledAssetError::LengthOverflow)?;
    let index_count =
        u32::try_from(mesh.indices.len()).map_err(|_| CompiledAssetError::LengthOverflow)?;

    // 12 bytes header (version + vertex_count + index_count) +
    // 12 floats per vertex (pos3 + normal3 + uv2 + tangent4) +
    // 4 bytes per index.
    let mut out = Vec::with_capacity(
        12 + mesh.vertices.len() * (12 * std::mem::size_of::<f32>()) + mesh.indices.len() * 4,
    );
    write_u32(&mut out, MESH_PAYLOAD_VERSION);
    write_u32(&mut out, vertex_count);
    write_u32(&mut out, index_count);
    for vertex in &mesh.vertices {
        for value in vertex.pos {
            write_f32(&mut out, value);
        }
        for value in vertex.normal {
            write_f32(&mut out, value);
        }
        for value in vertex.uv {
            write_f32(&mut out, value);
        }
        for value in vertex.tangent {
            write_f32(&mut out, value);
        }
    }
    for &index in &mesh.indices {
        write_u32(&mut out, index);
    }
    Ok(out)
}

pub fn decode_compiled_mesh_payload(bytes: &[u8]) -> Result<MeshData, CompiledAssetError> {
    let mut cursor = ByteReader::new(bytes);
    let version = cursor.read_u32()?;
    match version {
        3 => decode_mesh_v3(&mut cursor),
        _ => Err(CompiledAssetError::UnsupportedVersion(version)),
    }
}

fn decode_mesh_v3(cursor: &mut ByteReader) -> Result<MeshData, CompiledAssetError> {
    let vertex_count = cursor.read_u32()? as usize;
    let index_count = cursor.read_u32()? as usize;
    // Guard both untrusted counts against the remaining buffer before
    // reserving. Each vertex is exactly VERTEX_SIZE_BYTES and each index
    // exactly 4 bytes on the wire, so the two products bound the total.
    // Saturating arithmetic makes an overflowing count trip the guard
    // rather than wrap.
    let needed = (vertex_count as u64)
        .saturating_mul(VERTEX_SIZE_BYTES as u64)
        .saturating_add((index_count as u64).saturating_mul(4));
    cursor.ensure_bytes_available(needed)?;
    let mut vertices = Vec::with_capacity(vertex_count);
    let mut indices = Vec::with_capacity(index_count);

    for _ in 0..vertex_count {
        vertices.push(Vertex {
            pos: [cursor.read_f32()?, cursor.read_f32()?, cursor.read_f32()?],
            normal: [cursor.read_f32()?, cursor.read_f32()?, cursor.read_f32()?],
            uv: [cursor.read_f32()?, cursor.read_f32()?],
            tangent: [
                cursor.read_f32()?,
                cursor.read_f32()?,
                cursor.read_f32()?,
                cursor.read_f32()?,
            ],
        });
    }

    for _ in 0..index_count {
        indices.push(cursor.read_u32()?);
    }

    cursor.expect_end()?;
    Ok(MeshData::from_raw(vertices, indices))
}

/// On-wire size of one encoded vertex: 12 little-endian f32 values
/// (pos3 + normal3 + uv2 + tangent4). Used to bound `vertex_count`
/// against the buffer before allocating.
const VERTEX_SIZE_BYTES: usize = 12 * 4;

pub fn encode_compiled_texture_payload(texture: &Texture) -> Result<Vec<u8>, CompiledAssetError> {
    // Colour textures carry the OpenGL convention tag (irrelevant for
    // colour data, but the field is shared with the normal-map path).
    encode_compiled_texture_payload_with_convention(texture, NormalMapConvention::OpenGl)
}

/// Encode a texture payload (v4) with an explicit normal-map
/// convention. Colour textures use [`encode_compiled_texture_payload`]
/// (OpenGL); the convention is meaningful only when the payload is later
/// resolved as a normal map (see [`decode_compiled_normal_map_payload`]).
/// The stored pixels are the RAW source bytes; any DirectX→OpenGL flip
/// happens on load, not here — so the convention tag describes the
/// stored data's authored orientation.
pub fn encode_compiled_texture_payload_with_convention(
    texture: &Texture,
    convention: NormalMapConvention,
) -> Result<Vec<u8>, CompiledAssetError> {
    // 32-byte header (v4 adds a 4-byte convention tag over v3) plus pixels.
    let mut out = Vec::with_capacity(texture.data.len() + 32);
    write_u32(&mut out, TEXTURE_PAYLOAD_VERSION);
    write_u32(&mut out, texture.width);
    write_u32(&mut out, texture.height);
    write_u32(&mut out, encode_texture_format(texture.format));
    write_u32(&mut out, encode_color_space(texture.color_space));
    write_u32(&mut out, encode_normal_map_convention(convention));
    write_u64(
        &mut out,
        u64::try_from(texture.data.len()).map_err(|_| CompiledAssetError::LengthOverflow)?,
    );
    out.extend_from_slice(&texture.data);
    Ok(out)
}

/// The raw fields of a decoded texture payload, before it is projected
/// into either a colour [`Texture`] (convention dropped) or a
/// [`NormalMap`] (convention applied). Kept internal so both public
/// decoders share one body parser per version and cannot drift.
struct DecodedTexturePayload {
    width: u32,
    height: u32,
    format: TextureFormat,
    color_space: ColorSpace,
    /// Authored convention of the stored (raw) pixels. v3 payloads have
    /// no tag and default to OpenGL.
    convention: NormalMapConvention,
    data: Vec<u8>,
}

pub fn decode_compiled_texture_payload(bytes: &[u8]) -> Result<Texture, CompiledAssetError> {
    let decoded = decode_texture_payload_raw(bytes)?;
    // Colour path: the convention tag is irrelevant and dropped.
    Ok(Texture {
        width: decoded.width,
        height: decoded.height,
        format: decoded.format,
        color_space: decoded.color_space,
        data: decoded.data,
    })
}

/// Resolve a compiled texture payload as a [`NormalMap`], applying its
/// authored convention (M10.4). A DirectX-convention payload has its
/// green channel flipped on load so the stored GPU bytes are canonical
/// OpenGL Y+, and `source_convention` records the original — fixing the
/// pre-M10.4 bug where every normal map was assumed OpenGL and the
/// convention never round-tripped. v3 payloads default to OpenGL, so
/// existing blobs resolve exactly as before.
pub fn decode_compiled_normal_map_payload(
    bytes: &[u8],
) -> Result<NormalMap, CompiledAssetError> {
    let decoded = decode_texture_payload_raw(bytes)?;
    let normal_map = match decoded.convention {
        NormalMapConvention::OpenGl => {
            NormalMap::new_opengl(decoded.width, decoded.height, decoded.format, decoded.data)
        }
        NormalMapConvention::DirectX => {
            NormalMap::from_directx(decoded.width, decoded.height, decoded.format, decoded.data)
        }
    };
    Ok(normal_map)
}

fn decode_texture_payload_raw(bytes: &[u8]) -> Result<DecodedTexturePayload, CompiledAssetError> {
    let mut cursor = ByteReader::new(bytes);
    let version = cursor.read_u32()?;
    match version {
        3 => decode_texture_v3(&mut cursor),
        4 => decode_texture_v4(&mut cursor),
        _ => Err(CompiledAssetError::UnsupportedVersion(version)),
    }
}

/// v3 texture body: no convention tag. Migrates by defaulting the
/// convention to OpenGL — the exact pre-M10.4 behaviour.
fn decode_texture_v3(cursor: &mut ByteReader) -> Result<DecodedTexturePayload, CompiledAssetError> {
    let width = cursor.read_u32()?;
    let height = cursor.read_u32()?;
    let format = decode_texture_format(cursor.read_u32()?)?;
    let color_space = decode_color_space(cursor.read_u32()?)?;
    let data_len = cursor.read_u64()? as usize;
    let data = cursor.read_vec(data_len)?;
    cursor.expect_end()?;
    Ok(DecodedTexturePayload {
        width,
        height,
        format,
        color_space,
        convention: NormalMapConvention::OpenGl,
        data,
    })
}

/// v4 texture body: adds the normal-map convention tag after the
/// colour-space field.
fn decode_texture_v4(cursor: &mut ByteReader) -> Result<DecodedTexturePayload, CompiledAssetError> {
    let width = cursor.read_u32()?;
    let height = cursor.read_u32()?;
    let format = decode_texture_format(cursor.read_u32()?)?;
    let color_space = decode_color_space(cursor.read_u32()?)?;
    let convention = decode_normal_map_convention(cursor.read_u32()?)?;
    let data_len = cursor.read_u64()? as usize;
    let data = cursor.read_vec(data_len)?;
    cursor.expect_end()?;
    Ok(DecodedTexturePayload {
        width,
        height,
        format,
        color_space,
        convention,
        data,
    })
}

pub fn encode_compiled_material_payload(
    payload: &CompiledMaterialPayload,
) -> Result<Vec<u8>, CompiledAssetError> {
    let mut out = Vec::with_capacity(128);
    write_u32(&mut out, MATERIAL_PAYLOAD_VERSION);
    write_string(&mut out, &payload.albedo_logical_name)?;
    write_option_string(&mut out, payload.normal_logical_name.as_deref())?;
    write_option_string(&mut out, payload.specular_gloss_logical_name.as_deref())?;
    write_bool(&mut out, payload.double_sided);
    write_bool(&mut out, payload.alpha_cutout);
    Ok(out)
}

pub fn decode_compiled_material_payload(
    bytes: &[u8],
) -> Result<CompiledMaterialPayload, CompiledAssetError> {
    let mut cursor = ByteReader::new(bytes);
    let version = cursor.read_u32()?;
    match version {
        3 => decode_material_v3(&mut cursor),
        _ => Err(CompiledAssetError::UnsupportedVersion(version)),
    }
}

fn decode_material_v3(
    cursor: &mut ByteReader,
) -> Result<CompiledMaterialPayload, CompiledAssetError> {
    let payload = CompiledMaterialPayload {
        albedo_logical_name: cursor.read_string()?,
        normal_logical_name: cursor.read_option_string()?,
        specular_gloss_logical_name: cursor.read_option_string()?,
        double_sided: cursor.read_bool()?,
        alpha_cutout: cursor.read_bool()?,
    };
    cursor.expect_end()?;
    Ok(payload)
}

/// On-disk payload for a [`crate::assets::Shader`]. Two GLSL
/// strings (vertex + fragment) plus the asset's user-facing name.
/// See `compile_shader_asset` for the source-file format and
/// ADR 0004 Q9 (M7.4.1) for the hot-reload rationale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledShaderPayload {
    pub name: String,
    pub vertex_source: String,
    pub fragment_source: String,
}

pub fn encode_compiled_shader_payload(
    payload: &CompiledShaderPayload,
) -> Result<Vec<u8>, CompiledAssetError> {
    let mut out = Vec::with_capacity(
        16 + payload.name.len() + payload.vertex_source.len() + payload.fragment_source.len(),
    );
    write_u32(&mut out, SHADER_PAYLOAD_VERSION);
    write_string(&mut out, &payload.name)?;
    write_string(&mut out, &payload.vertex_source)?;
    write_string(&mut out, &payload.fragment_source)?;
    Ok(out)
}

pub fn decode_compiled_shader_payload(
    bytes: &[u8],
) -> Result<CompiledShaderPayload, CompiledAssetError> {
    let mut cursor = ByteReader::new(bytes);
    let version = cursor.read_u32()?;
    match version {
        3 => decode_shader_v3(&mut cursor),
        _ => Err(CompiledAssetError::UnsupportedVersion(version)),
    }
}

fn decode_shader_v3(cursor: &mut ByteReader) -> Result<CompiledShaderPayload, CompiledAssetError> {
    let payload = CompiledShaderPayload {
        name: cursor.read_string()?,
        vertex_source: cursor.read_string()?,
        fragment_source: cursor.read_string()?,
    };
    cursor.expect_end()?;
    Ok(payload)
}

/// On-disk payload for a compiled scene asset. Carries the canonical
/// `.hscene` text serialization of a [`crate::editor::SceneDocument`]
/// so the dispatcher can decode → SceneDocument and hand it to the
/// editor's runtime reload path.
///
/// Why a text payload instead of binary? The `.hscene` format is the
/// only one that round-trips a `SceneDocument` exactly today (see
/// `SceneDocument::to_hscene_string` / `from_hscene_str`). Going
/// through the same serialization both authoring and runtime use
/// avoids a separate binary scene format that could drift out of
/// sync with the text path. The runtime cost is one parse on
/// reload, which dominates anything else the diff path does — and
/// the diff path itself runs on the editor's main thread at frame
/// boundaries, not in the hot render loop.
///
/// See ADR 0004 Q14 + Q14a (M7.4.1 — scene hot-reload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledScenePayload {
    pub name: String,
    /// Canonical `.hscene` text. UTF-8. Guaranteed parseable by
    /// `SceneDocument::from_hscene_str` (the compile step would have
    /// rejected the source `.hsc` if the resulting document couldn't
    /// re-serialize).
    pub hscene_text: String,
}

pub fn encode_compiled_scene_payload(
    payload: &CompiledScenePayload,
) -> Result<Vec<u8>, CompiledAssetError> {
    let mut out = Vec::with_capacity(16 + payload.name.len() + payload.hscene_text.len());
    write_u32(&mut out, SCENE_PAYLOAD_VERSION);
    write_string(&mut out, &payload.name)?;
    write_string(&mut out, &payload.hscene_text)?;
    Ok(out)
}

pub fn decode_compiled_scene_payload(
    bytes: &[u8],
) -> Result<CompiledScenePayload, CompiledAssetError> {
    let mut cursor = ByteReader::new(bytes);
    let version = cursor.read_u32()?;
    match version {
        3 => decode_scene_v3(&mut cursor),
        _ => Err(CompiledAssetError::UnsupportedVersion(version)),
    }
}

fn decode_scene_v3(cursor: &mut ByteReader) -> Result<CompiledScenePayload, CompiledAssetError> {
    let payload = CompiledScenePayload {
        name: cursor.read_string()?,
        hscene_text: cursor.read_string()?,
    };
    cursor.expect_end()?;
    Ok(payload)
}

pub struct CompiledAssetRuntimeLoader {
    compiled_root: PathBuf,
    mesh_handles: HashMap<String, MeshHandle>,
    texture_handles: HashMap<String, TextureHandle>,
    /// Normal-map handles are cached separately from texture handles even
    /// though both originate from the same compiled-texture payload type.
    /// A logical name resolved as a normal map produces a [`NormalMap`] asset
    /// (with convention metadata); the same logical name resolved as a colour
    /// texture would produce a distinct [`Texture`] asset. This lets a single
    /// source `.png` flow into either typed pin without the runtime forgetting
    /// the distinction. See ADR 0002 Q4.
    normal_map_handles: HashMap<String, NormalMapHandle>,
    material_handles: HashMap<String, MaterialHandle>,
}

impl CompiledAssetRuntimeLoader {
    pub fn new(compiled_root: impl Into<PathBuf>) -> Self {
        Self {
            compiled_root: compiled_root.into(),
            mesh_handles: HashMap::new(),
            texture_handles: HashMap::new(),
            normal_map_handles: HashMap::new(),
            material_handles: HashMap::new(),
        }
    }

    pub fn compiled_root(&self) -> &Path {
        &self.compiled_root
    }

    pub fn ensure_mesh(
        &mut self,
        assets: &mut AssetStore,
        logical_name: &str,
    ) -> Result<MeshHandle, CompiledAssetRuntimeError> {
        if let Some(handle) = self.mesh_handles.get(logical_name) {
            return Ok(*handle);
        }

        let blob = self.load_blob(logical_name, CompiledAssetKind::Mesh)?;
        let mesh = decode_compiled_mesh_payload(&blob.payload)?;
        let handle = assets.insert(mesh);
        self.mesh_handles.insert(logical_name.to_string(), handle);
        Ok(handle)
    }

    pub fn ensure_texture(
        &mut self,
        assets: &mut AssetStore,
        logical_name: &str,
    ) -> Result<TextureHandle, CompiledAssetRuntimeError> {
        if let Some(handle) = self.texture_handles.get(logical_name) {
            return Ok(*handle);
        }

        let blob = self.load_blob(logical_name, CompiledAssetKind::Texture)?;
        let texture = decode_compiled_texture_payload(&blob.payload)?;
        let handle = assets.insert(texture);
        self.texture_handles
            .insert(logical_name.to_string(), handle);
        Ok(handle)
    }

    /// Resolve a logical name as a [`NormalMap`] asset. Reads the same
    /// compiled-texture payload (kind [`CompiledAssetKind::Texture`]) that
    /// [`Self::ensure_texture`] does, but resolves it through
    /// [`decode_compiled_normal_map_payload`] so the payload's authored
    /// [`NormalMapConvention`] (M10.4) is honoured: a DirectX-convention
    /// payload is flipped to canonical OpenGL Y+ on load, and the source
    /// convention round-trips onto the [`NormalMap`]. A v3 payload (no
    /// convention tag) defaults to OpenGL — the pre-M10.4 behaviour — so
    /// existing compiled normal maps resolve unchanged. See ADR 0002 Q4.
    ///
    /// Follow-up: the compiler still emits OpenGL-convention texture
    /// payloads by default (a raw image carries no convention metadata);
    /// wiring a per-normal-map convention declaration from material
    /// sources into `compile_*` so DirectX sources compile as DirectX is
    /// tracked separately. The format and this loader now carry and honour
    /// the convention end to end.
    pub fn ensure_normal_map(
        &mut self,
        assets: &mut AssetStore,
        logical_name: &str,
    ) -> Result<NormalMapHandle, CompiledAssetRuntimeError> {
        if let Some(handle) = self.normal_map_handles.get(logical_name) {
            return Ok(*handle);
        }

        let blob = self.load_blob(logical_name, CompiledAssetKind::Texture)?;
        let normal_map = decode_compiled_normal_map_payload(&blob.payload)?;
        let handle = assets.insert(normal_map);
        self.normal_map_handles
            .insert(logical_name.to_string(), handle);
        Ok(handle)
    }

    pub fn ensure_material(
        &mut self,
        assets: &mut AssetStore,
        logical_name: &str,
        shader: ShaderHandle,
    ) -> Result<MaterialHandle, CompiledAssetRuntimeError> {
        if let Some(handle) = self.material_handles.get(logical_name) {
            return Ok(*handle);
        }

        let blob = self.load_blob(logical_name, CompiledAssetKind::Material)?;
        let payload = decode_compiled_material_payload(&blob.payload)?;
        let albedo = self.ensure_texture(assets, &payload.albedo_logical_name)?;
        let normal = match payload.normal_logical_name.as_deref() {
            Some(name) => Some(self.ensure_normal_map(assets, name)?),
            None => None,
        };
        let specular_gloss = match payload.specular_gloss_logical_name.as_deref() {
            Some(name) => Some(self.ensure_texture(assets, name)?),
            None => None,
        };

        let mut material = Material::new(shader, albedo)
            .with_double_sided(payload.double_sided)
            .with_alpha_cutout(payload.alpha_cutout);
        if let Some(handle) = normal {
            material = material.with_normal_map(handle);
        }
        if let Some(handle) = specular_gloss {
            material = material.with_specular_gloss(handle);
        }

        let handle = assets.insert(material);
        self.material_handles
            .insert(logical_name.to_string(), handle);
        Ok(handle)
    }

    fn load_blob(
        &self,
        logical_name: &str,
        expected_kind: CompiledAssetKind,
    ) -> Result<CompiledAssetBlob, CompiledAssetRuntimeError> {
        let path = compiled_asset_path_for_logical_name(&self.compiled_root, logical_name);
        let bytes = fs::read(path)?;
        let blob = decode_compiled_asset(&bytes)?;
        if blob.metadata.kind != expected_kind {
            return Err(CompiledAssetRuntimeError::UnexpectedKind {
                expected: expected_kind,
                found: blob.metadata.kind,
            });
        }
        if blob.metadata.logical_name != logical_name {
            return Err(CompiledAssetRuntimeError::MetadataMismatch {
                expected_logical_name: logical_name.to_string(),
                found_logical_name: blob.metadata.logical_name,
            });
        }
        Ok(blob)
    }
}

fn encode_texture_format(format: TextureFormat) -> u32 {
    match format {
        TextureFormat::Rgba8 => 1,
        TextureFormat::Rgb8 => 2,
        TextureFormat::R8 => 3,
    }
}

fn decode_texture_format(raw: u32) -> Result<TextureFormat, CompiledAssetError> {
    match raw {
        1 => Ok(TextureFormat::Rgba8),
        2 => Ok(TextureFormat::Rgb8),
        3 => Ok(TextureFormat::R8),
        _ => Err(CompiledAssetError::InvalidTextureFormat(raw)),
    }
}

fn encode_color_space(space: ColorSpace) -> u32 {
    match space {
        ColorSpace::Srgb => 1,
        ColorSpace::Linear => 2,
    }
}

fn decode_color_space(raw: u32) -> Result<ColorSpace, CompiledAssetError> {
    match raw {
        1 => Ok(ColorSpace::Srgb),
        2 => Ok(ColorSpace::Linear),
        _ => Err(CompiledAssetError::InvalidColorSpace(raw)),
    }
}

fn encode_normal_map_convention(convention: NormalMapConvention) -> u32 {
    match convention {
        NormalMapConvention::OpenGl => 1,
        NormalMapConvention::DirectX => 2,
    }
}

fn decode_normal_map_convention(raw: u32) -> Result<NormalMapConvention, CompiledAssetError> {
    match raw {
        1 => Ok(NormalMapConvention::OpenGl),
        2 => Ok(NormalMapConvention::DirectX),
        _ => Err(CompiledAssetError::InvalidNormalMapConvention(raw)),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        compiled_asset_path_for_logical_name, decode_compiled_material_payload,
        decode_compiled_mesh_payload, decode_compiled_normal_map_payload,
        decode_compiled_scene_payload, decode_compiled_shader_payload,
        decode_compiled_texture_payload, encode_compiled_material_payload,
        encode_compiled_mesh_payload, encode_compiled_scene_payload, encode_compiled_shader_payload,
        encode_compiled_texture_payload, encode_compiled_texture_payload_with_convention,
        CompiledAssetRuntimeError, CompiledAssetRuntimeLoader, CompiledMaterialPayload,
        CompiledScenePayload, CompiledShaderPayload,
    };
    use crate::assets::{
        encode_compiled_asset, ColorSpace, CompiledAssetBlob, CompiledAssetDependency,
        CompiledAssetError, CompiledAssetKind, CompiledAssetMetadata, MeshData, NormalMapConvention,
        ShaderHandle, Texture, TextureFormat, Vertex,
    };

    fn unique_test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "hold_runtime_loader_{}_{}_{}",
            label,
            std::process::id(),
            nonce
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_blob(root: &Path, blob: &CompiledAssetBlob) {
        let path = compiled_asset_path_for_logical_name(root, &blob.metadata.logical_name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, encode_compiled_asset(blob).unwrap()).unwrap();
    }

    /// Linear-space textures must round-trip through the compiled-asset
    /// envelope with their `ColorSpace` tag intact. Without this, sRGB
    /// would be the silent default and every roughness / mask map would
    /// get sRGB-decoded at sample time — a textbook PBR bug. See ADR
    /// 0002 Q3.
    #[test]
    fn texture_payload_roundtrips_linear_color_space() {
        let texture = Texture {
            width: 2,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Linear,
            data: vec![64, 64, 64, 255, 200, 200, 200, 255],
        };
        let roundtrip =
            decode_compiled_texture_payload(&encode_compiled_texture_payload(&texture).unwrap())
                .unwrap();
        assert_eq!(roundtrip, texture);
        assert_eq!(roundtrip.color_space, ColorSpace::Linear);
    }

    /// An invalid `ColorSpace` tag in the binary stream surfaces as a
    /// typed `InvalidColorSpace` error rather than silently defaulting
    /// to one of the variants. Production loaders rely on this to flag
    /// corrupted or future-version assets loudly.
    #[test]
    fn texture_payload_rejects_unknown_color_space_tag() {
        // Build a valid v3 texture, then tamper with the colour-space
        // byte (it sits at offset 16: u32 version, u32 width, u32
        // height, u32 format, u32 color_space).
        let texture = Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![0, 0, 0, 255],
        };
        let mut bytes = encode_compiled_texture_payload(&texture).unwrap();
        // Overwrite the color_space field (4 bytes at offset 16: u32
        // version, width, height, format, color_space) with an invalid
        // tag (99). The convention field follows at offset 20 in v4.
        bytes[16] = 99;
        bytes[17] = 0;
        bytes[18] = 0;
        bytes[19] = 0;
        match decode_compiled_texture_payload(&bytes) {
            Err(CompiledAssetError::InvalidColorSpace(99)) => {}
            other => panic!("expected InvalidColorSpace(99), got {:?}", other),
        }
    }

    #[test]
    fn payload_roundtrips_preserve_mesh_texture_and_material_content() {
        let mesh = MeshData::cube();
        let mesh_roundtrip =
            decode_compiled_mesh_payload(&encode_compiled_mesh_payload(&mesh).unwrap()).unwrap();
        assert_eq!(mesh.vertex_count(), mesh_roundtrip.vertex_count());
        assert_eq!(mesh.triangle_count(), mesh_roundtrip.triangle_count());

        let texture = Texture {
            width: 2,
            height: 2,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
        };
        let texture_roundtrip =
            decode_compiled_texture_payload(&encode_compiled_texture_payload(&texture).unwrap())
                .unwrap();
        assert_eq!(texture_roundtrip, texture);

        let payload = CompiledMaterialPayload {
            albedo_logical_name: "textures/crate_albedo".into(),
            normal_logical_name: Some("textures/crate_normal".into()),
            specular_gloss_logical_name: None,
            double_sided: false,
            alpha_cutout: true,
        };
        let payload_roundtrip =
            decode_compiled_material_payload(&encode_compiled_material_payload(&payload).unwrap())
                .unwrap();
        assert_eq!(payload_roundtrip, payload);
    }

    #[test]
    fn runtime_loader_recursively_loads_material_dependencies() {
        let root = unique_test_root("recursive");

        write_blob(
            &root,
            &CompiledAssetBlob {
                metadata: CompiledAssetMetadata {
                    kind: CompiledAssetKind::Texture,
                    logical_name: "textures/crate_albedo".into(),
                    source_path: "source/crate_albedo.tga".into(),
                    compiler_tag: "texture-compiler@test".into(),
                    runtime_path: "compiled/textures/crate_albedo.hcmp".into(),
                    dependencies: Vec::new(),
                },
                payload: encode_compiled_texture_payload(&Texture {
                    width: 1,
                    height: 1,
                    format: TextureFormat::Rgba8,
                    color_space: ColorSpace::Srgb,
                    data: vec![255, 255, 255, 255],
                })
                .unwrap(),
            },
        );

        write_blob(
            &root,
            &CompiledAssetBlob {
                metadata: CompiledAssetMetadata {
                    kind: CompiledAssetKind::Material,
                    logical_name: "materials/crate".into(),
                    source_path: "source/crate.material".into(),
                    compiler_tag: "material-compiler@test".into(),
                    runtime_path: "compiled/materials/crate.hcmp".into(),
                    dependencies: vec![CompiledAssetDependency {
                        kind: CompiledAssetKind::Texture,
                        logical_name: "textures/crate_albedo".into(),
                        source_path: "source/crate_albedo.tga".into(),
                    }],
                },
                payload: encode_compiled_material_payload(&CompiledMaterialPayload {
                    albedo_logical_name: "textures/crate_albedo".into(),
                    normal_logical_name: None,
                    specular_gloss_logical_name: None,
                    double_sided: false,
                    alpha_cutout: false,
                })
                .unwrap(),
            },
        );

        write_blob(
            &root,
            &CompiledAssetBlob {
                metadata: CompiledAssetMetadata {
                    kind: CompiledAssetKind::Mesh,
                    logical_name: "meshes/crate".into(),
                    source_path: "source/crate.obj".into(),
                    compiler_tag: "mesh-compiler@test".into(),
                    runtime_path: "compiled/meshes/crate.hcmp".into(),
                    dependencies: Vec::new(),
                },
                payload: encode_compiled_mesh_payload(&MeshData::cube()).unwrap(),
            },
        );

        let mut assets = crate::assets::AssetStore::new();
        let mut loader = CompiledAssetRuntimeLoader::new(&root);
        let material = loader
            .ensure_material(&mut assets, "materials/crate", ShaderHandle::null())
            .unwrap();
        let mesh = loader.ensure_mesh(&mut assets, "meshes/crate").unwrap();

        assert!(assets.get(material).is_some());
        assert!(assets.get(mesh).is_some());
        let material_asset = assets.get(material).unwrap();
        assert!(assets.get(material_asset.albedo).is_some());

        fs::remove_dir_all(root).ok();
    }

    // ── M10 format-stability: frozen payload version ──────────────────────

    /// Each per-kind payload version is a wire contract. Freeze all
    /// five so a bump is a deliberate, reviewed change. They start
    /// equal (3) at the M10.2 split; later milestones move individual
    /// kinds (M10.4 -> texture 4), and this test must be updated in the
    /// same commit as the bump.
    #[test]
    fn payload_versions_are_frozen() {
        assert_eq!(super::MESH_PAYLOAD_VERSION, 3);
        assert_eq!(super::TEXTURE_PAYLOAD_VERSION, 4); // bumped in M10.4
        assert_eq!(super::MATERIAL_PAYLOAD_VERSION, 3);
        assert_eq!(super::SHADER_PAYLOAD_VERSION, 3);
        assert_eq!(super::SCENE_PAYLOAD_VERSION, 3);
    }

    // ── M10 format-stability: golden-byte fixtures per payload ────────────
    //
    // Exact on-disk bytes for a known input, hand-derived from the
    // documented layout. A wire-layout change fails these immediately,
    // where a roundtrip test would not.

    #[test]
    fn mesh_payload_golden_single_vertex_no_indices() {
        // One vertex: pos = [1, 0, 0], everything else 0. No indices.
        let mesh = MeshData::from_raw(
            vec![Vertex {
                pos: [1.0, 0.0, 0.0],
                normal: [0.0, 0.0, 0.0],
                uv: [0.0, 0.0],
                tangent: [0.0, 0.0, 0.0, 0.0],
            }],
            vec![],
        );
        let mut expected: Vec<u8> = Vec::new();
        expected.extend_from_slice(&3u32.to_le_bytes()); // version
        expected.extend_from_slice(&1u32.to_le_bytes()); // vertex_count
        expected.extend_from_slice(&0u32.to_le_bytes()); // index_count
        expected.extend_from_slice(&1.0f32.to_le_bytes()); // pos.x = 1.0 -> 00 00 80 3F
        expected.extend_from_slice(&[0u8; 44]); // remaining 11 f32 = 0
        assert_eq!(expected.len(), 60);
        // pos.x is the only non-zero float; confirm the 1.0 bit pattern.
        assert_eq!(&expected[12..16], &[0x00, 0x00, 0x80, 0x3F]);
        assert_eq!(encode_compiled_mesh_payload(&mesh).unwrap(), expected);
        let decoded = decode_compiled_mesh_payload(&expected).unwrap();
        assert_eq!(decoded.vertex_count(), 1);
    }

    #[test]
    fn texture_payload_golden_v4_1x1_rgba8_srgb() {
        let texture = Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![10, 20, 30, 40],
        };
        #[rustfmt::skip]
        let expected: Vec<u8> = vec![
            0x04, 0x00, 0x00, 0x00, // version = 4
            0x01, 0x00, 0x00, 0x00, // width = 1
            0x01, 0x00, 0x00, 0x00, // height = 1
            0x01, 0x00, 0x00, 0x00, // format = Rgba8 (1)
            0x01, 0x00, 0x00, 0x00, // color_space = Srgb (1)
            0x01, 0x00, 0x00, 0x00, // normal-map convention = OpenGl (1)
            0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // data_len (u64) = 4
            0x0A, 0x14, 0x1E, 0x28, // data
        ];
        assert_eq!(encode_compiled_texture_payload(&texture).unwrap(), expected);
        assert_eq!(decode_compiled_texture_payload(&expected).unwrap(), texture);
    }

    /// Reference payload migration (M10.4): a hand-authored v3 texture
    /// payload — no convention tag — resolves as a normal map with the
    /// OpenGL convention and its pixels UNCHANGED, exactly matching the
    /// pre-M10.4 behaviour. Proves bumping the texture payload to v4 did
    /// not orphan existing v3 blobs.
    #[test]
    fn texture_payload_v3_migrates_to_opengl_normal_map() {
        #[rustfmt::skip]
        let v3_bytes: Vec<u8> = vec![
            0x03, 0x00, 0x00, 0x00, // version = 3 (old, no convention tag)
            0x01, 0x00, 0x00, 0x00, // width = 1
            0x01, 0x00, 0x00, 0x00, // height = 1
            0x01, 0x00, 0x00, 0x00, // format = Rgba8 (1)
            0x02, 0x00, 0x00, 0x00, // color_space = Linear (2)
            0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // data_len (u64) = 4
            0x0A, 0xF5, 0x1E, 0xFF, // one RGBA pixel, green = 245
        ];
        // As a colour texture: pixels intact.
        let tex = decode_compiled_texture_payload(&v3_bytes).unwrap();
        assert_eq!(tex.data, vec![0x0A, 0xF5, 0x1E, 0xFF]);
        // As a normal map: OpenGL default, green NOT flipped.
        let nm = decode_compiled_normal_map_payload(&v3_bytes).unwrap();
        assert_eq!(nm.source_convention, NormalMapConvention::OpenGl);
        assert_eq!(nm.data, vec![0x0A, 0xF5, 0x1E, 0xFF]);
    }

    /// v4 lets a normal map's authored convention travel. A DirectX
    /// payload is flipped to canonical OpenGL Y+ on load (green channel
    /// inverted) and records DirectX as its source convention — the fix
    /// for the pre-M10.4 bug where every normal map was assumed OpenGL.
    /// An OpenGL payload loads unflipped. Both are exercised through the
    /// real encoder, not hand-authored bytes.
    #[test]
    fn normal_map_payload_v4_applies_convention_on_load() {
        // One RGBA pixel with green = 245; raw source bytes are stored.
        let texture = Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Linear,
            data: vec![0x0A, 0xF5, 0x1E, 0xFF],
        };

        // OpenGL: no flip, green stays 245.
        let og_bytes = encode_compiled_texture_payload_with_convention(
            &texture,
            NormalMapConvention::OpenGl,
        )
        .unwrap();
        let og = decode_compiled_normal_map_payload(&og_bytes).unwrap();
        assert_eq!(og.source_convention, NormalMapConvention::OpenGl);
        assert_eq!(og.data[1], 0xF5);

        // DirectX: green flipped to 255 - 245 = 10, convention recorded.
        let dx_bytes = encode_compiled_texture_payload_with_convention(
            &texture,
            NormalMapConvention::DirectX,
        )
        .unwrap();
        let dx = decode_compiled_normal_map_payload(&dx_bytes).unwrap();
        assert_eq!(dx.source_convention, NormalMapConvention::DirectX);
        assert_eq!(dx.data[1], 0x0A, "green channel must be flipped on load");
        // Red / blue / alpha untouched.
        assert_eq!(dx.data[0], 0x0A);
        assert_eq!(dx.data[2], 0x1E);
        assert_eq!(dx.data[3], 0xFF);
    }

    /// An unknown convention tag surfaces a typed error rather than
    /// silently defaulting.
    #[test]
    fn texture_payload_v4_rejects_unknown_convention_tag() {
        let texture = Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![0, 0, 0, 255],
        };
        let mut bytes = encode_compiled_texture_payload(&texture).unwrap();
        // convention tag is the 6th u32 (offset 20..24).
        bytes[20..24].copy_from_slice(&99u32.to_le_bytes());
        match decode_compiled_normal_map_payload(&bytes) {
            Err(CompiledAssetError::InvalidNormalMapConvention(99)) => {}
            other => panic!("expected InvalidNormalMapConvention(99), got {other:?}"),
        }
    }

    #[test]
    fn material_payload_golden_albedo_normal_some_spec_none() {
        let payload = CompiledMaterialPayload {
            albedo_logical_name: "a".into(),
            normal_logical_name: Some("n".into()),
            specular_gloss_logical_name: None,
            double_sided: false,
            alpha_cutout: true,
        };
        #[rustfmt::skip]
        let expected: Vec<u8> = vec![
            0x03, 0x00, 0x00, 0x00, // version = 3
            0x01, 0x00, 0x00, 0x00, b'a', // albedo = "a"
            0x01, 0x01, 0x00, 0x00, 0x00, b'n', // normal = Some("n") (present flag + string)
            0x00, // specular_gloss = None
            0x00, // double_sided = false
            0x01, // alpha_cutout = true
        ];
        assert_eq!(encode_compiled_material_payload(&payload).unwrap(), expected);
        assert_eq!(decode_compiled_material_payload(&expected).unwrap(), payload);
    }

    #[test]
    fn shader_payload_golden_and_roundtrip() {
        let payload = CompiledShaderPayload {
            name: "s".into(),
            vertex_source: "v".into(),
            fragment_source: "f".into(),
        };
        #[rustfmt::skip]
        let expected: Vec<u8> = vec![
            0x03, 0x00, 0x00, 0x00, // version = 3
            0x01, 0x00, 0x00, 0x00, b's', // name = "s"
            0x01, 0x00, 0x00, 0x00, b'v', // vertex_source = "v"
            0x01, 0x00, 0x00, 0x00, b'f', // fragment_source = "f"
        ];
        assert_eq!(encode_compiled_shader_payload(&payload).unwrap(), expected);
        assert_eq!(decode_compiled_shader_payload(&expected).unwrap(), payload);
    }

    #[test]
    fn scene_payload_golden_and_roundtrip() {
        let payload = CompiledScenePayload {
            name: "sc".into(),
            hscene_text: "x".into(),
        };
        #[rustfmt::skip]
        let expected: Vec<u8> = vec![
            0x03, 0x00, 0x00, 0x00, // version = 3
            0x02, 0x00, 0x00, 0x00, b's', b'c', // name = "sc"
            0x01, 0x00, 0x00, 0x00, b'x', // hscene_text = "x"
        ];
        assert_eq!(encode_compiled_scene_payload(&payload).unwrap(), expected);
        assert_eq!(decode_compiled_scene_payload(&expected).unwrap(), payload);
    }

    // ── M10 hardening: trailing-byte + oversized-count rejection ──────────

    #[test]
    fn texture_payload_rejects_trailing_bytes() {
        let texture = Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![0, 0, 0, 255],
        };
        let mut bytes = encode_compiled_texture_payload(&texture).unwrap();
        bytes.push(0xFF);
        match decode_compiled_texture_payload(&bytes) {
            Err(CompiledAssetError::TrailingBytes { extra }) => assert_eq!(extra, 1),
            other => panic!("expected TrailingBytes, got {other:?}"),
        }
    }

    #[test]
    fn shader_payload_rejects_trailing_bytes() {
        let payload = CompiledShaderPayload {
            name: "s".into(),
            vertex_source: "v".into(),
            fragment_source: "f".into(),
        };
        let mut bytes = encode_compiled_shader_payload(&payload).unwrap();
        bytes.push(0x00);
        assert_eq!(
            decode_compiled_shader_payload(&bytes).unwrap_err(),
            CompiledAssetError::TrailingBytes { extra: 1 }
        );
    }

    #[test]
    fn mesh_payload_rejects_oversized_vertex_count_without_allocating() {
        // version=3, vertex_count=u32::MAX, index_count=0, no vertex data.
        // Decode must reject via DeclaredLengthExceedsBuffer before
        // Vec::with_capacity tries to reserve ~206 GB of vertices.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        // MeshData doesn't implement Debug, so match on the error side
        // without formatting the (never-produced) Ok value.
        match decode_compiled_mesh_payload(&bytes) {
            Err(CompiledAssetError::DeclaredLengthExceedsBuffer { .. }) => {}
            Err(other) => panic!("expected DeclaredLengthExceedsBuffer, got {other:?}"),
            Ok(_) => panic!("expected DeclaredLengthExceedsBuffer, got Ok(mesh)"),
        }
    }

    #[test]
    fn shader_and_scene_payloads_reject_truncation() {
        let shader = encode_compiled_shader_payload(&CompiledShaderPayload {
            name: "s".into(),
            vertex_source: "v".into(),
            fragment_source: "f".into(),
        })
        .unwrap();
        assert_eq!(
            decode_compiled_shader_payload(&shader[..shader.len() - 1]).unwrap_err(),
            CompiledAssetError::UnexpectedEof
        );

        let scene = encode_compiled_scene_payload(&CompiledScenePayload {
            name: "sc".into(),
            hscene_text: "x".into(),
        })
        .unwrap();
        assert_eq!(
            decode_compiled_scene_payload(&scene[..scene.len() - 1]).unwrap_err(),
            CompiledAssetError::UnexpectedEof
        );
    }

    #[test]
    fn payloads_reject_unsupported_version() {
        // Patch the version field (first 4 bytes) of a valid shader
        // payload to an unsupported value.
        let mut bytes = encode_compiled_shader_payload(&CompiledShaderPayload {
            name: "s".into(),
            vertex_source: "v".into(),
            fragment_source: "f".into(),
        })
        .unwrap();
        bytes[0..4].copy_from_slice(&99u32.to_le_bytes());
        assert_eq!(
            decode_compiled_shader_payload(&bytes).unwrap_err(),
            CompiledAssetError::UnsupportedVersion(99)
        );
    }

    // ── Loader structural guards (M10 review coverage) ────────────────────

    /// `load_blob` rejects a blob whose kind differs from the kind the
    /// caller asked for — the guard against a mislabeled asset. A
    /// Texture blob filed where a mesh was requested must surface
    /// UnexpectedKind, not decode as garbage.
    #[test]
    fn ensure_mesh_rejects_wrong_kind_blob() {
        let root = unique_test_root("wrong_kind");
        // Write a *Texture* blob at the "meshes/crate" logical path.
        write_blob(
            &root,
            &CompiledAssetBlob {
                metadata: CompiledAssetMetadata {
                    kind: CompiledAssetKind::Texture,
                    logical_name: "meshes/crate".into(),
                    source_path: "source/crate.tga".into(),
                    compiler_tag: "test".into(),
                    runtime_path: "compiled/meshes/crate.hcmp".into(),
                    dependencies: Vec::new(),
                },
                payload: encode_compiled_texture_payload(&Texture {
                    width: 1,
                    height: 1,
                    format: TextureFormat::Rgba8,
                    color_space: ColorSpace::Srgb,
                    data: vec![0, 0, 0, 255],
                })
                .unwrap(),
            },
        );
        let mut assets = crate::assets::AssetStore::new();
        let mut loader = CompiledAssetRuntimeLoader::new(&root);
        match loader.ensure_mesh(&mut assets, "meshes/crate") {
            Err(CompiledAssetRuntimeError::UnexpectedKind { expected, found }) => {
                assert_eq!(expected, CompiledAssetKind::Mesh);
                assert_eq!(found, CompiledAssetKind::Texture);
            }
            other => panic!("expected UnexpectedKind, got {other:?}"),
        }
        fs::remove_dir_all(root).ok();
    }

    /// `load_blob` rejects a blob whose stored logical name differs from
    /// the one requested — the guard against a misfiled asset (right
    /// kind, wrong identity). Written to the requested path but stamped
    /// with a different logical name.
    #[test]
    fn ensure_mesh_rejects_logical_name_mismatch() {
        let root = unique_test_root("name_mismatch");
        // Compute the on-disk path for "meshes/crate" but stamp the
        // blob's metadata with a *different* logical name.
        let path = compiled_asset_path_for_logical_name(&root, "meshes/crate");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let blob = CompiledAssetBlob {
            metadata: CompiledAssetMetadata {
                kind: CompiledAssetKind::Mesh,
                logical_name: "meshes/other".into(), // != requested
                source_path: "source/other.obj".into(),
                compiler_tag: "test".into(),
                runtime_path: "compiled/meshes/other.hcmp".into(),
                dependencies: Vec::new(),
            },
            payload: encode_compiled_mesh_payload(&MeshData::cube()).unwrap(),
        };
        fs::write(&path, encode_compiled_asset(&blob).unwrap()).unwrap();

        let mut assets = crate::assets::AssetStore::new();
        let mut loader = CompiledAssetRuntimeLoader::new(&root);
        match loader.ensure_mesh(&mut assets, "meshes/crate") {
            Err(CompiledAssetRuntimeError::MetadataMismatch {
                expected_logical_name,
                found_logical_name,
            }) => {
                assert_eq!(expected_logical_name, "meshes/crate");
                assert_eq!(found_logical_name, "meshes/other");
            }
            other => panic!("expected MetadataMismatch, got {other:?}"),
        }
        fs::remove_dir_all(root).ok();
    }

    /// End-to-end (M10.4): a normal map's convention survives a full
    /// compile-payload -> envelope -> disk -> ensure_normal_map round
    /// trip. A DirectX blob resolves flipped with DirectX recorded; an
    /// OpenGL blob resolves unflipped. This is the disk path the
    /// dispatcher's in-memory path must match.
    #[test]
    fn ensure_normal_map_full_disk_roundtrip_honours_convention() {
        let root = unique_test_root("nm_roundtrip");
        let write_nm = |logical: &str, convention: NormalMapConvention| {
            let path = compiled_asset_path_for_logical_name(&root, logical);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let payload = encode_compiled_texture_payload_with_convention(
                &Texture {
                    width: 1,
                    height: 1,
                    format: TextureFormat::Rgba8,
                    color_space: ColorSpace::Linear,
                    data: vec![10, 200, 30, 255], // green = 200
                },
                convention,
            )
            .unwrap();
            let blob = CompiledAssetBlob {
                metadata: CompiledAssetMetadata {
                    kind: CompiledAssetKind::Texture,
                    logical_name: logical.into(),
                    source_path: "source/n.tga".into(),
                    compiler_tag: "test".into(),
                    runtime_path: "compiled/n.hcmp".into(),
                    dependencies: Vec::new(),
                },
                payload,
            };
            fs::write(&path, encode_compiled_asset(&blob).unwrap()).unwrap();
        };

        write_nm("normals/og", NormalMapConvention::OpenGl);
        write_nm("normals/dx", NormalMapConvention::DirectX);

        let mut assets = crate::assets::AssetStore::new();
        let mut loader = CompiledAssetRuntimeLoader::new(&root);

        let og_handle = loader.ensure_normal_map(&mut assets, "normals/og").unwrap();
        let og = assets.get(og_handle).unwrap();
        assert_eq!(og.source_convention, NormalMapConvention::OpenGl);
        assert_eq!(og.data[1], 200, "OpenGL green unflipped");

        let dx_handle = loader.ensure_normal_map(&mut assets, "normals/dx").unwrap();
        let dx = assets.get(dx_handle).unwrap();
        assert_eq!(dx.source_convention, NormalMapConvention::DirectX);
        assert_eq!(dx.data[1], 55, "DirectX green flipped (255 - 200)");

        fs::remove_dir_all(root).ok();
    }
}
