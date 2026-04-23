use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use super::{
    decode_compiled_asset, AssetStore, CompiledAssetBlob, CompiledAssetError, CompiledAssetKind,
    Material, MaterialHandle, MeshData, MeshHandle, ShaderHandle, Texture, TextureFormat,
    TextureHandle, Vertex,
};

const COMPILED_PAYLOAD_VERSION: u32 = 1;

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

    let mut out = Vec::with_capacity(
        12 + mesh.vertices.len() * (8 * std::mem::size_of::<f32>()) + mesh.indices.len() * 4,
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
        });
    }

    for _ in 0..index_count {
        indices.push(cursor.read_u32()?);
    }

    Ok(MeshData::from_raw(vertices, indices))
}

pub fn encode_compiled_texture_payload(texture: &Texture) -> Result<Vec<u8>, CompiledAssetError> {
    let mut out = Vec::with_capacity(texture.data.len() + 24);
    write_u32(&mut out, COMPILED_PAYLOAD_VERSION);
    write_u32(&mut out, texture.width);
    write_u32(&mut out, texture.height);
    write_u32(&mut out, encode_texture_format(texture.format));
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
    let data_len = cursor.read_u64()? as usize;
    let data = cursor.read_vec(data_len)?;

    Ok(Texture {
        width,
        height,
        format,
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

pub struct CompiledAssetRuntimeLoader {
    compiled_root: PathBuf,
    mesh_handles: HashMap<String, MeshHandle>,
    texture_handles: HashMap<String, TextureHandle>,
    material_handles: HashMap<String, MaterialHandle>,
}

impl CompiledAssetRuntimeLoader {
    pub fn new(compiled_root: impl Into<PathBuf>) -> Self {
        Self {
            compiled_root: compiled_root.into(),
            mesh_handles: HashMap::new(),
            texture_handles: HashMap::new(),
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
            Some(name) => Some(self.ensure_texture(assets, name)?),
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
            material = material.with_normal(handle);
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
        encode_compiled_texture_payload, CompiledAssetRuntimeLoader, CompiledMaterialPayload,
    };
    use crate::assets::{
        encode_compiled_asset, CompiledAssetBlob, CompiledAssetDependency, CompiledAssetKind,
        CompiledAssetMetadata, MeshData, ShaderHandle, Texture, TextureFormat,
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
