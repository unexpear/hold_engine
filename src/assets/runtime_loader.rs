use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use super::{
    decode_compiled_asset, AssetStore, ColorSpace, CompiledAssetBlob, CompiledAssetError,
    CompiledAssetKind, Material, MaterialHandle, MeshData, MeshHandle, NormalMap, NormalMapHandle,
    ShaderHandle, Texture, TextureFormat, TextureHandle, Vertex,
};

/// On-disk payload version for compiled assets. Bumped whenever any encoded
/// payload (mesh / texture / material) changes shape. Old payloads are rejected
/// rather than migrated; the asset compiler regenerates them.
///
/// Version history:
/// - 1: initial format. Vertex = pos(3) + normal(3) + uv(2).
/// - 2: vertex layout extended with tangent(4). See ADR 0002 Q4.
/// - 3: texture payload tagged with `ColorSpace`. See ADR 0002 Q3.
const COMPILED_PAYLOAD_VERSION: u32 = 3;

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
    InvalidTextureFormat(u32),
    InvalidColorSpace(u32),
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
            Self::InvalidTextureFormat(raw) => {
                write!(f, "invalid compiled texture format {raw}")
            }
            Self::InvalidColorSpace(raw) => {
                write!(f, "invalid compiled colour-space tag {raw}")
            }
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
    write_u32(&mut out, COMPILED_PAYLOAD_VERSION);
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
    let mut cursor = PayloadCursor::new(bytes);
    let version = cursor.read_u32()?;
    if version != COMPILED_PAYLOAD_VERSION {
        return Err(CompiledAssetError::UnsupportedVersion(version));
    }

    let vertex_count = cursor.read_u32()? as usize;
    let index_count = cursor.read_u32()? as usize;
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

    Ok(MeshData::from_raw(vertices, indices))
}

pub fn encode_compiled_texture_payload(texture: &Texture) -> Result<Vec<u8>, CompiledAssetError> {
    // 28-byte header (v3 header is 4 bigger than v2 for the colour-space tag)
    // plus pixel data.
    let mut out = Vec::with_capacity(texture.data.len() + 28);
    write_u32(&mut out, COMPILED_PAYLOAD_VERSION);
    write_u32(&mut out, texture.width);
    write_u32(&mut out, texture.height);
    write_u32(&mut out, encode_texture_format(texture.format));
    write_u32(&mut out, encode_color_space(texture.color_space));
    write_u64(
        &mut out,
        u64::try_from(texture.data.len()).map_err(|_| CompiledAssetError::LengthOverflow)?,
    );
    out.extend_from_slice(&texture.data);
    Ok(out)
}

pub fn decode_compiled_texture_payload(bytes: &[u8]) -> Result<Texture, CompiledAssetRuntimeError> {
    let mut cursor = PayloadCursor::new(bytes);
    let version = cursor.read_u32()?;
    if version != COMPILED_PAYLOAD_VERSION {
        return Err(CompiledAssetError::UnsupportedVersion(version).into());
    }

    let width = cursor.read_u32()?;
    let height = cursor.read_u32()?;
    let format = decode_texture_format(cursor.read_u32()?)?;
    let color_space = decode_color_space(cursor.read_u32()?)?;
    let data_len = cursor.read_u64()? as usize;
    let data = cursor.read_vec(data_len)?;

    Ok(Texture {
        width,
        height,
        format,
        color_space,
        data,
    })
}

pub fn encode_compiled_material_payload(
    payload: &CompiledMaterialPayload,
) -> Result<Vec<u8>, CompiledAssetError> {
    let mut out = Vec::with_capacity(128);
    write_u32(&mut out, COMPILED_PAYLOAD_VERSION);
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
    let mut cursor = PayloadCursor::new(bytes);
    let version = cursor.read_u32()?;
    if version != COMPILED_PAYLOAD_VERSION {
        return Err(CompiledAssetError::UnsupportedVersion(version));
    }

    Ok(CompiledMaterialPayload {
        albedo_logical_name: cursor.read_string()?,
        normal_logical_name: cursor.read_option_string()?,
        specular_gloss_logical_name: cursor.read_option_string()?,
        double_sided: cursor.read_bool()?,
        alpha_cutout: cursor.read_bool()?,
    })
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
    write_u32(&mut out, COMPILED_PAYLOAD_VERSION);
    write_string(&mut out, &payload.name)?;
    write_string(&mut out, &payload.vertex_source)?;
    write_string(&mut out, &payload.fragment_source)?;
    Ok(out)
}

pub fn decode_compiled_shader_payload(
    bytes: &[u8],
) -> Result<CompiledShaderPayload, CompiledAssetError> {
    let mut cursor = PayloadCursor::new(bytes);
    let version = cursor.read_u32()?;
    if version != COMPILED_PAYLOAD_VERSION {
        return Err(CompiledAssetError::UnsupportedVersion(version));
    }

    Ok(CompiledShaderPayload {
        name: cursor.read_string()?,
        vertex_source: cursor.read_string()?,
        fragment_source: cursor.read_string()?,
    })
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
    write_u32(&mut out, COMPILED_PAYLOAD_VERSION);
    write_string(&mut out, &payload.name)?;
    write_string(&mut out, &payload.hscene_text)?;
    Ok(out)
}

pub fn decode_compiled_scene_payload(
    bytes: &[u8],
) -> Result<CompiledScenePayload, CompiledAssetError> {
    let mut cursor = PayloadCursor::new(bytes);
    let version = cursor.read_u32()?;
    if version != COMPILED_PAYLOAD_VERSION {
        return Err(CompiledAssetError::UnsupportedVersion(version));
    }

    Ok(CompiledScenePayload {
        name: cursor.read_string()?,
        hscene_text: cursor.read_string()?,
    })
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
    /// compiled-texture payload that [`Self::ensure_texture`] does, but inserts
    /// the result as a typed [`NormalMap`] (defaulting to OpenGL Y+
    /// convention) so it can be bound only to a [`Material`]'s normal slot.
    /// See ADR 0002 Q4.
    ///
    /// **TODO (M5):** the compiled-asset envelope should grow a dedicated
    /// `CompiledAssetKind::NormalMap` so the convention can travel through the
    /// pipeline rather than being assumed at the resolution boundary. Until
    /// then, source data is assumed to already be in the canonical convention.
    pub fn ensure_normal_map(
        &mut self,
        assets: &mut AssetStore,
        logical_name: &str,
    ) -> Result<NormalMapHandle, CompiledAssetRuntimeError> {
        if let Some(handle) = self.normal_map_handles.get(logical_name) {
            return Ok(*handle);
        }

        let blob = self.load_blob(logical_name, CompiledAssetKind::Texture)?;
        let texture = decode_compiled_texture_payload(&blob.payload)?;
        let normal_map =
            NormalMap::new_opengl(texture.width, texture.height, texture.format, texture.data);
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

fn decode_texture_format(raw: u32) -> Result<TextureFormat, CompiledAssetRuntimeError> {
    match raw {
        1 => Ok(TextureFormat::Rgba8),
        2 => Ok(TextureFormat::Rgb8),
        3 => Ok(TextureFormat::R8),
        _ => Err(CompiledAssetRuntimeError::InvalidTextureFormat(raw)),
    }
}

fn encode_color_space(space: ColorSpace) -> u32 {
    match space {
        ColorSpace::Srgb => 1,
        ColorSpace::Linear => 2,
    }
}

fn decode_color_space(raw: u32) -> Result<ColorSpace, CompiledAssetRuntimeError> {
    match raw {
        1 => Ok(ColorSpace::Srgb),
        2 => Ok(ColorSpace::Linear),
        _ => Err(CompiledAssetRuntimeError::InvalidColorSpace(raw)),
    }
}

fn write_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_bool(out: &mut Vec<u8>, value: bool) {
    out.push(if value { 1 } else { 0 });
}

fn write_string(out: &mut Vec<u8>, value: &str) -> Result<(), CompiledAssetError> {
    let len = u32::try_from(value.len()).map_err(|_| CompiledAssetError::LengthOverflow)?;
    write_u32(out, len);
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn write_option_string(out: &mut Vec<u8>, value: Option<&str>) -> Result<(), CompiledAssetError> {
    write_bool(out, value.is_some());
    if let Some(value) = value {
        write_string(out, value)?;
    }
    Ok(())
}

struct PayloadCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> PayloadCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn read_u32(&mut self) -> Result<u32, CompiledAssetError> {
        Ok(u32::from_le_bytes(self.read_array::<4>()?))
    }

    fn read_u64(&mut self) -> Result<u64, CompiledAssetError> {
        Ok(u64::from_le_bytes(self.read_array::<8>()?))
    }

    fn read_f32(&mut self) -> Result<f32, CompiledAssetError> {
        Ok(f32::from_le_bytes(self.read_array::<4>()?))
    }

    fn read_bool(&mut self) -> Result<bool, CompiledAssetError> {
        Ok(self.read_array::<1>()?[0] != 0)
    }

    fn read_string(&mut self) -> Result<String, CompiledAssetError> {
        let len = self.read_u32()? as usize;
        let bytes = self.read_vec(len)?;
        String::from_utf8(bytes).map_err(|_| CompiledAssetError::Utf8)
    }

    fn read_option_string(&mut self) -> Result<Option<String>, CompiledAssetError> {
        if self.read_bool()? {
            Ok(Some(self.read_string()?))
        } else {
            Ok(None)
        }
    }

    fn read_vec(&mut self, len: usize) -> Result<Vec<u8>, CompiledAssetError> {
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

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], CompiledAssetError> {
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
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        compiled_asset_path_for_logical_name, decode_compiled_material_payload,
        decode_compiled_mesh_payload, decode_compiled_texture_payload,
        encode_compiled_material_payload, encode_compiled_mesh_payload,
        encode_compiled_texture_payload, CompiledAssetRuntimeError, CompiledAssetRuntimeLoader,
        CompiledMaterialPayload,
    };
    use crate::assets::{
        encode_compiled_asset, ColorSpace, CompiledAssetBlob, CompiledAssetDependency,
        CompiledAssetKind, CompiledAssetMetadata, MeshData, ShaderHandle, Texture, TextureFormat,
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
        // Overwrite the color_space field (4 bytes at offset 16) with
        // an invalid tag (99).
        bytes[16] = 99;
        bytes[17] = 0;
        bytes[18] = 0;
        bytes[19] = 0;
        match decode_compiled_texture_payload(&bytes) {
            Err(CompiledAssetRuntimeError::InvalidColorSpace(99)) => {}
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
}
