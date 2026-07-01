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
    /// GLSL shader pair (vertex + fragment) compiled from a single
    /// `.hshd` source file with `#shader vertex` / `#shader fragment`
    /// section markers. See `compile_shader_asset` and ADR 0004 Q9
    /// (M7.4.1 — shader hot-reload).
    Shader = 9,
    /// Scene document compiled from a `.hsc` source file. The
    /// payload carries the canonical `.hscene` text serialization of
    /// the scene's [`crate::editor::SceneDocument`] — round-tripping
    /// through this textual form lets the dispatcher's hot-reload
    /// pipeline ride the same compile-validate seam every other
    /// asset kind uses, while keeping the on-disk authored format
    /// human-editable. See ADR 0004 Q14 + Q14a (M7.4.1 — scene
    /// hot-reload with entity-preserving diff).
    Scene = 10,
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
            9 => Ok(Self::Shader),
            10 => Ok(Self::Scene),
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
    /// A declared count or length field would require more bytes than
    /// remain in the buffer. Distinct from [`Self::UnexpectedEof`]
    /// (which fires when a read actually runs off the end): this fires
    /// *before* a speculative `Vec::with_capacity(untrusted_count)`, so
    /// a malformed header cannot request a multi-gigabyte allocation
    /// from a handful of on-disk bytes. Carries the declared element
    /// count and the bytes actually remaining so the failure is
    /// reproducible from the error alone (ADR 0003 Pillar 4).
    DeclaredLengthExceedsBuffer { declared: u64, remaining: u64 },
    /// Bytes remained after the last declared field was consumed. The
    /// compiled format is exact-length by contract — any additive
    /// extension must bump the version — so trailing bytes signal
    /// corruption or a version/layout mismatch, never a
    /// forward-compatible append. See `docs/formats/compiled-asset.md`.
    TrailingBytes { extra: u64 },
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
            Self::DeclaredLengthExceedsBuffer { declared, remaining } => write!(
                f,
                "compiled asset declares {declared} elements but only {remaining} bytes remain"
            ),
            Self::TrailingBytes { extra } => write!(
                f,
                "compiled asset has {extra} unexpected trailing byte(s) after the final field"
            ),
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
    // Guard the untrusted count before reserving. Each dependency is at
    // least DEPENDENCY_MIN_BYTES on the wire (kind tag + two zero-length
    // strings), so a count implying more bytes than remain is malformed
    // and is rejected before `Vec::with_capacity` can over-allocate.
    cursor.ensure_count_bound(dependency_count, DEPENDENCY_MIN_BYTES)?;
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
    // The envelope is exact-length: nothing legitimately follows the
    // payload. Reject trailing bytes so corruption / version drift
    // surfaces as a typed error instead of silent acceptance.
    cursor.expect_end()?;
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

/// Minimum on-wire size of one dependency record: a u32 kind tag plus
/// two u32-length-prefixed strings at their zero-length minimum
/// (4 + 4 + 4). Used to bound `dependency_count` against the remaining
/// buffer before allocating.
const DEPENDENCY_MIN_BYTES: usize = 12;

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

    /// Bytes not yet consumed.
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    /// Reject a declared element count that could not possibly fit in
    /// the remaining buffer, given each element's minimum wire size.
    /// Called before `Vec::with_capacity` to close the allocation-DoS
    /// surface (a u32 count times a small element size can request
    /// gigabytes from a few bytes of input).
    fn ensure_count_bound(
        &self,
        count: usize,
        min_elem_size: usize,
    ) -> Result<(), CompiledAssetError> {
        let needed = (count as u64).saturating_mul(min_elem_size as u64);
        let remaining = self.remaining() as u64;
        if needed > remaining {
            return Err(CompiledAssetError::DeclaredLengthExceedsBuffer {
                declared: count as u64,
                remaining,
            });
        }
        Ok(())
    }

    /// Assert every byte has been consumed. Call at the end of a decode
    /// to enforce the exact-length contract.
    fn expect_end(&self) -> Result<(), CompiledAssetError> {
        let extra = self.remaining() as u64;
        if extra != 0 {
            return Err(CompiledAssetError::TrailingBytes { extra });
        }
        Ok(())
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

    // ── M10 format-stability: frozen constants ────────────────────────────

    /// The magic and version are a wire contract. If either changes,
    /// every previously-compiled `.hcmp` on disk becomes unreadable —
    /// this test forces that to be a deliberate, reviewed edit rather
    /// than an accidental one. See docs/formats/compiled-asset.md.
    #[test]
    fn envelope_magic_and_version_are_frozen() {
        assert_eq!(&super::COMPILED_ASSET_MAGIC, b"HOLDCMP1");
        assert_eq!(super::COMPILED_ASSET_VERSION, 1);
    }

    // ── M10 format-stability: golden-byte fixtures ────────────────────────
    //
    // These assert the EXACT on-disk bytes for a known input. Unlike a
    // roundtrip test (encode -> decode -> equal), which passes even if
    // encoder and decoder drift together, a golden fixture fails the
    // moment the wire layout changes — which is exactly the guarantee
    // M10 is about. The expected vectors are derived by hand from the
    // documented field layout, NOT captured from current output, so a
    // latent layout bug would show up here as a mismatch instead of
    // being silently frozen.

    /// The minimal envelope: Mesh kind, all-empty metadata strings, no
    /// dependencies, empty payload. Exactly 44 bytes.
    #[test]
    fn envelope_golden_minimal_44_bytes() {
        let asset = CompiledAssetBlob {
            metadata: CompiledAssetMetadata {
                kind: CompiledAssetKind::Mesh,
                logical_name: String::new(),
                source_path: String::new(),
                compiler_tag: String::new(),
                runtime_path: String::new(),
                dependencies: vec![],
            },
            payload: vec![],
        };
        #[rustfmt::skip]
        let expected: Vec<u8> = vec![
            b'H', b'O', b'L', b'D', b'C', b'M', b'P', b'1', // magic
            0x01, 0x00, 0x00, 0x00, // version = 1
            0x01, 0x00, 0x00, 0x00, // kind = Mesh (1)
            0x00, 0x00, 0x00, 0x00, // logical_name len = 0
            0x00, 0x00, 0x00, 0x00, // source_path len = 0
            0x00, 0x00, 0x00, 0x00, // compiler_tag len = 0
            0x00, 0x00, 0x00, 0x00, // runtime_path len = 0
            0x00, 0x00, 0x00, 0x00, // dependency_count = 0
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // payload_len (u64) = 0
        ];
        assert_eq!(expected.len(), 44);
        assert_eq!(encode_compiled_asset(&asset).unwrap(), expected);
        // Round-trips too.
        assert_eq!(decode_compiled_asset(&expected).unwrap(), asset);
    }

    /// A richer envelope exercising a non-empty logical name, one
    /// dependency, and a two-byte payload — pins the string framing,
    /// dependency-record framing, and the asymmetric u64 payload length.
    #[test]
    fn envelope_golden_with_metadata_and_dependency() {
        let asset = CompiledAssetBlob {
            metadata: CompiledAssetMetadata {
                kind: CompiledAssetKind::Texture,
                logical_name: "tex".into(),
                source_path: String::new(),
                compiler_tag: String::new(),
                runtime_path: String::new(),
                dependencies: vec![CompiledAssetDependency {
                    kind: CompiledAssetKind::Mesh,
                    logical_name: "m".into(),
                    source_path: String::new(),
                }],
            },
            payload: vec![0xAB, 0xCD],
        };
        #[rustfmt::skip]
        let expected: Vec<u8> = vec![
            b'H', b'O', b'L', b'D', b'C', b'M', b'P', b'1', // magic
            0x01, 0x00, 0x00, 0x00, // version = 1
            0x03, 0x00, 0x00, 0x00, // kind = Texture (3)
            0x03, 0x00, 0x00, 0x00, b't', b'e', b'x', // logical_name = "tex"
            0x00, 0x00, 0x00, 0x00, // source_path len = 0
            0x00, 0x00, 0x00, 0x00, // compiler_tag len = 0
            0x00, 0x00, 0x00, 0x00, // runtime_path len = 0
            0x01, 0x00, 0x00, 0x00, // dependency_count = 1
            0x01, 0x00, 0x00, 0x00, // dep[0].kind = Mesh (1)
            0x01, 0x00, 0x00, 0x00, b'm', // dep[0].logical_name = "m"
            0x00, 0x00, 0x00, 0x00, // dep[0].source_path len = 0
            0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // payload_len (u64) = 2
            0xAB, 0xCD, // payload
        ];
        assert_eq!(encode_compiled_asset(&asset).unwrap(), expected);
        assert_eq!(decode_compiled_asset(&expected).unwrap(), asset);
    }

    // ── M10 hardening: negative / typed-error coverage ────────────────────

    /// Build the 44-byte minimal Mesh envelope so negative tests can
    /// patch individual fields.
    fn minimal_envelope_bytes() -> Vec<u8> {
        let asset = CompiledAssetBlob {
            metadata: CompiledAssetMetadata {
                kind: CompiledAssetKind::Mesh,
                logical_name: String::new(),
                source_path: String::new(),
                compiler_tag: String::new(),
                runtime_path: String::new(),
                dependencies: vec![],
            },
            payload: vec![],
        };
        encode_compiled_asset(&asset).unwrap()
    }

    #[test]
    fn envelope_rejects_unsupported_version() {
        let mut bytes = minimal_envelope_bytes();
        bytes[8..12].copy_from_slice(&2u32.to_le_bytes()); // version field
        assert_eq!(
            decode_compiled_asset(&bytes).unwrap_err(),
            CompiledAssetError::UnsupportedVersion(2)
        );
    }

    #[test]
    fn envelope_rejects_invalid_top_level_kind() {
        let mut bytes = minimal_envelope_bytes();
        bytes[12..16].copy_from_slice(&99u32.to_le_bytes()); // kind field
        assert_eq!(
            decode_compiled_asset(&bytes).unwrap_err(),
            CompiledAssetError::InvalidKind(99)
        );
    }

    #[test]
    fn envelope_rejects_invalid_dependency_kind() {
        // Envelope with one dependency whose kind tag is invalid.
        let asset = CompiledAssetBlob {
            metadata: CompiledAssetMetadata {
                kind: CompiledAssetKind::Material,
                logical_name: String::new(),
                source_path: String::new(),
                compiler_tag: String::new(),
                runtime_path: String::new(),
                dependencies: vec![CompiledAssetDependency {
                    kind: CompiledAssetKind::Texture,
                    logical_name: String::new(),
                    source_path: String::new(),
                }],
            },
            payload: vec![],
        };
        let mut bytes = encode_compiled_asset(&asset).unwrap();
        // dep[0].kind sits right after dependency_count. Offsets:
        // magic8 + ver4 + kind4 + 4 empty strings (16) + dep_count4 = 36.
        bytes[36..40].copy_from_slice(&77u32.to_le_bytes());
        assert_eq!(
            decode_compiled_asset(&bytes).unwrap_err(),
            CompiledAssetError::InvalidKind(77)
        );
    }

    #[test]
    fn envelope_rejects_invalid_utf8_string() {
        // magic + version + kind + logical_name(len=1, byte 0xFF).
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"HOLDCMP1");
        bytes.extend_from_slice(&1u32.to_le_bytes()); // version
        bytes.extend_from_slice(&1u32.to_le_bytes()); // kind = Mesh
        bytes.extend_from_slice(&1u32.to_le_bytes()); // logical_name len = 1
        bytes.push(0xFF); // invalid UTF-8
        assert_eq!(
            decode_compiled_asset(&bytes).unwrap_err(),
            CompiledAssetError::Utf8
        );
    }

    #[test]
    fn envelope_rejects_trailing_bytes() {
        let mut bytes = minimal_envelope_bytes();
        bytes.push(0x00); // one byte past the declared payload
        assert_eq!(
            decode_compiled_asset(&bytes).unwrap_err(),
            CompiledAssetError::TrailingBytes { extra: 1 }
        );
    }

    #[test]
    fn envelope_rejects_oversized_dependency_count_without_allocating() {
        // Patch dependency_count (offset 32..36 in the minimal envelope)
        // to u32::MAX. Decode must reject via DeclaredLengthExceedsBuffer
        // BEFORE Vec::with_capacity would try to reserve ~51 GB.
        let mut bytes = minimal_envelope_bytes();
        bytes[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
        match decode_compiled_asset(&bytes).unwrap_err() {
            CompiledAssetError::DeclaredLengthExceedsBuffer { declared, .. } => {
                assert_eq!(declared, u32::MAX as u64);
            }
            other => panic!("expected DeclaredLengthExceedsBuffer, got {other:?}"),
        }
    }
}
