use std::fmt;

pub const COMPILED_ASSET_MAGIC: [u8; 8] = *b"HOLDCMP1";
pub const COMPILED_ASSET_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CompiledAssetKind {
    Mesh = 1,
    Material = 2,
    Texture = 3,
    TerrainLayerSet = 4,
    VegetationPrototype = 5,
    Skeleton = 6,
    AnimationClip = 7,
    AudioClip = 8,
}

impl CompiledAssetKind {
    pub fn from_raw(raw: u32) -> Result<Self, CompiledAssetError> {
        match raw {
            1 => Ok(Self::Mesh),
            2 => Ok(Self::Material),
            3 => Ok(Self::Texture),
            4 => Ok(Self::TerrainLayerSet),
            5 => Ok(Self::VegetationPrototype),
            6 => Ok(Self::Skeleton),
            7 => Ok(Self::AnimationClip),
            8 => Ok(Self::AudioClip),
            _ => Err(CompiledAssetError::InvalidKind(raw)),
        }
    }

    pub const fn to_raw(self) -> u32 {
        self as u32
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledAssetDependency {
    pub kind: CompiledAssetKind,
    pub logical_name: String,
    pub source_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledAssetMetadata {
    pub kind: CompiledAssetKind,
    pub logical_name: String,
    pub source_path: String,
    pub compiler_tag: String,
    pub runtime_path: String,
    pub dependencies: Vec<CompiledAssetDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledAssetBlob {
    pub metadata: CompiledAssetMetadata,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompiledAssetError {
    InvalidMagic,
    UnsupportedVersion(u32),
    InvalidKind(u32),
    UnexpectedEof,
    Utf8,
    LengthOverflow,
}

impl fmt::Display for CompiledAssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => f.write_str("invalid compiled asset magic"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported compiled asset version {version}")
            }
            Self::InvalidKind(kind) => write!(f, "invalid compiled asset kind {kind}"),
            Self::UnexpectedEof => f.write_str("compiled asset payload truncated"),
            Self::Utf8 => f.write_str("compiled asset metadata is not valid UTF-8"),
            Self::LengthOverflow => f.write_str("compiled asset string or payload is too large"),
        }
    }
}

impl std::error::Error for CompiledAssetError {}

pub fn encode_compiled_asset(asset: &CompiledAssetBlob) -> Result<Vec<u8>, CompiledAssetError> {
    let mut out = Vec::with_capacity(asset.payload.len() + 128);
    out.extend_from_slice(&COMPILED_ASSET_MAGIC);
    write_u32(&mut out, COMPILED_ASSET_VERSION);
    write_u32(&mut out, asset.metadata.kind.to_raw());
    write_string(&mut out, &asset.metadata.logical_name)?;
    write_string(&mut out, &asset.metadata.source_path)?;
    write_string(&mut out, &asset.metadata.compiler_tag)?;
    write_string(&mut out, &asset.metadata.runtime_path)?;
    write_u32(&mut out, asset.metadata.dependencies.len() as u32);
    for dependency in &asset.metadata.dependencies {
        write_u32(&mut out, dependency.kind.to_raw());
        write_string(&mut out, &dependency.logical_name)?;
        write_string(&mut out, &dependency.source_path)?;
    }
    write_u64(&mut out, asset.payload.len() as u64);
    out.extend_from_slice(&asset.payload);
    Ok(out)
}

pub fn decode_compiled_asset(bytes: &[u8]) -> Result<CompiledAssetBlob, CompiledAssetError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.read_array::<8>()? != COMPILED_ASSET_MAGIC {
        return Err(CompiledAssetError::InvalidMagic);
    }

    let version = cursor.read_u32()?;
    if version != COMPILED_ASSET_VERSION {
        return Err(CompiledAssetError::UnsupportedVersion(version));
    }

    let kind = CompiledAssetKind::from_raw(cursor.read_u32()?)?;
    let logical_name = cursor.read_string()?;
    let source_path = cursor.read_string()?;
    let compiler_tag = cursor.read_string()?;
    let runtime_path = cursor.read_string()?;
    let dependency_count = cursor.read_u32()? as usize;
    let mut dependencies = Vec::with_capacity(dependency_count);
    for _ in 0..dependency_count {
        dependencies.push(CompiledAssetDependency {
            kind: CompiledAssetKind::from_raw(cursor.read_u32()?)?,
            logical_name: cursor.read_string()?,
            source_path: cursor.read_string()?,
        });
    }

    let payload_len = cursor.read_u64()? as usize;
    let payload = cursor.read_vec(payload_len)?;
    Ok(CompiledAssetBlob {
        metadata: CompiledAssetMetadata {
            kind,
            logical_name,
            source_path,
            compiler_tag,
            runtime_path,
            dependencies,
        },
        payload,
    })
}

fn write_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_string(out: &mut Vec<u8>, value: &str) -> Result<(), CompiledAssetError> {
    let len = u32::try_from(value.len()).map_err(|_| CompiledAssetError::LengthOverflow)?;
    write_u32(out, len);
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn read_u32(&mut self) -> Result<u32, CompiledAssetError> {
        Ok(u32::from_le_bytes(self.read_array::<4>()?))
    }

    fn read_u64(&mut self) -> Result<u64, CompiledAssetError> {
        Ok(u64::from_le_bytes(self.read_array::<8>()?))
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

    fn read_string(&mut self) -> Result<String, CompiledAssetError> {
        let len = self.read_u32()? as usize;
        let bytes = self.read_vec(len)?;
        String::from_utf8(bytes).map_err(|_| CompiledAssetError::Utf8)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_compiled_asset, encode_compiled_asset, CompiledAssetBlob, CompiledAssetDependency,
        CompiledAssetError, CompiledAssetKind, CompiledAssetMetadata,
    };

    #[test]
    fn compiled_asset_roundtrip_preserves_metadata_and_payload() {
        let asset = CompiledAssetBlob {
            metadata: CompiledAssetMetadata {
                kind: CompiledAssetKind::Material,
                logical_name: "materials/foliage/palm_frond".into(),
                source_path: "assets/source/foliage/palm_frond.material".into(),
                compiler_tag: "material-compiler@2026.04".into(),
                runtime_path: "compiled/foliage/palm_frond.hcmp".into(),
                dependencies: vec![
                    CompiledAssetDependency {
                        kind: CompiledAssetKind::Texture,
                        logical_name: "textures/palm_albedo".into(),
                        source_path: "assets/source/foliage/palm_albedo.tga".into(),
                    },
                    CompiledAssetDependency {
                        kind: CompiledAssetKind::Texture,
                        logical_name: "textures/palm_normal".into(),
                        source_path: "assets/source/foliage/palm_normal.tga".into(),
                    },
                ],
            },
            payload: vec![1, 3, 3, 7, 9, 11],
        };

        let encoded = encode_compiled_asset(&asset).unwrap();
        let decoded = decode_compiled_asset(&encoded).unwrap();
        assert_eq!(decoded, asset);
    }

    #[test]
    fn compiled_asset_rejects_invalid_magic() {
        let err = decode_compiled_asset(b"not-a-valid-asset").unwrap_err();
        assert_eq!(err, CompiledAssetError::InvalidMagic);
    }

    #[test]
    fn compiled_asset_rejects_truncated_payload() {
        let asset = CompiledAssetBlob {
            metadata: CompiledAssetMetadata {
                kind: CompiledAssetKind::Mesh,
                logical_name: "meshes/crate".into(),
                source_path: "assets/source/crate.obj".into(),
                compiler_tag: "mesh-compiler@test".into(),
                runtime_path: "compiled/crate.hcmp".into(),
                dependencies: vec![],
            },
            payload: vec![5, 4, 3, 2, 1],
        };

        let mut encoded = encode_compiled_asset(&asset).unwrap();
        encoded.pop();

        let err = decode_compiled_asset(&encoded).unwrap_err();
        assert_eq!(err, CompiledAssetError::UnexpectedEof);
    }
}
