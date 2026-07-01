use std::fmt;

use super::wire::{write_string, write_u32, write_u64, ByteReader};

pub const COMPILED_ASSET_MAGIC: [u8; 8] = *b"HOLDCMP1";
/// Current compiled-asset envelope version the encoder writes.
///
/// Version history:
/// - 1: magic + version + kind + metadata + dependencies + payload.
/// - 2: adds a trailing little-endian u32 CRC-32 (IEEE) footer over
///   every preceding byte (M10.3), so on-disk corruption is
///   distinguishable from truncation or a version mismatch. The
///   decoder still reads v1 (migration): a v1 blob has no footer and
///   simply skips the integrity check.
///
/// The 8-byte magic is deliberately NOT bumped across envelope
/// versions — it stays `HOLDCMP1` and the u32 version field is the sole
/// source of truth. This means an old reader encountering a newer
/// envelope fails with `UnsupportedVersion` (actionable) rather than
/// `InvalidMagic` (looks like a wrong/corrupt file). See
/// `docs/formats/compiled-asset.md`.
pub const COMPILED_ASSET_VERSION: u32 = 2;

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
    /// The stored CRC-32 footer (envelope v2+) did not match the CRC
    /// computed over the decoded bytes — the blob is corrupt on disk.
    /// Distinct from every structural error above: the layout parsed
    /// cleanly but the integrity check failed, so the right response is
    /// "regenerate from source", not "the format is wrong". Carries
    /// both values so the corruption is reproducible from the error.
    ChecksumMismatch { stored: u32, computed: u32 },
    /// A texture payload's format tag was not a known [`TextureFormat`].
    /// A payload-body error (lives here rather than in
    /// `CompiledAssetRuntimeError` so all payload decoders share one
    /// error type — M10.5). Carries the raw tag.
    InvalidTextureFormat(u32),
    /// A texture payload's colour-space tag was not a known
    /// [`ColorSpace`]. Carries the raw tag.
    InvalidColorSpace(u32),
    /// A texture payload's normal-map convention tag (v4+) was not a
    /// known [`NormalMapConvention`]. Carries the raw tag.
    InvalidNormalMapConvention(u32),
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
            Self::ChecksumMismatch { stored, computed } => write!(
                f,
                "compiled asset checksum mismatch: stored {stored:#010x}, computed {computed:#010x}"
            ),
            Self::InvalidTextureFormat(raw) => write!(f, "invalid compiled texture format {raw}"),
            Self::InvalidColorSpace(raw) => write!(f, "invalid compiled colour-space tag {raw}"),
            Self::InvalidNormalMapConvention(raw) => {
                write!(f, "invalid compiled normal-map convention tag {raw}")
            }
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
    // Envelope v2 (M10.3): append a CRC-32 footer over every preceding
    // byte (magic + version + body + payload). Detects on-disk bit-rot
    // that a structural parse would miss. Must be computed last, over
    // the fully-assembled buffer.
    let checksum = crc32(&out);
    write_u32(&mut out, checksum);
    Ok(out)
}

/// CRC-32 (IEEE 802.3 / zlib): reversed polynomial `0xEDB88320`, init
/// `0xFFFFFFFF`, final inversion. Bitwise (no lookup table) — the
/// integrity check runs at asset-load / staleness-check time, never in
/// the render hot path, so table-driven speed isn't warranted and the
/// table-free form is trivially auditable. Zero external crates, per
/// the charter. Reference check value: `crc32(b"123456789") ==
/// 0xCBF43926`.
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            // Branchless: mask is all-ones when the low bit is set.
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

pub fn decode_compiled_asset(bytes: &[u8]) -> Result<CompiledAssetBlob, CompiledAssetError> {
    let mut cursor = ByteReader::new(bytes);
    if cursor.read_array::<8>()? != COMPILED_ASSET_MAGIC {
        return Err(CompiledAssetError::InvalidMagic);
    }

    // Version-dispatch seam (M10.2/M10.3). The encoder always writes
    // COMPILED_ASSET_VERSION; the decoder supports an explicit set of
    // versions, so an older on-disk envelope is READ (migrated) rather
    // than rejected. v1 and v2 share the same body layout; v2 adds a
    // trailing CRC-32 footer. The catch-all keeps unknown/future
    // versions a typed error, never a silent misparse.
    let version = cursor.read_u32()?;
    match version {
        1 => {
            // v1: body, then exact end (no footer).
            let blob = read_envelope_body(&mut cursor)?;
            cursor.expect_end()?;
            Ok(blob)
        }
        2 => {
            // v2: body, then a u32 CRC-32 footer, then exact end. The
            // footer covers every byte before it, so verify against the
            // original slice rather than re-serialising.
            let blob = read_envelope_body(&mut cursor)?;
            let stored = cursor.read_u32()?;
            cursor.expect_end()?;
            // `bytes.len() >= 4` is guaranteed: read_u32 above succeeded,
            // so at least the 4 footer bytes exist.
            let computed = crc32(&bytes[..bytes.len() - 4]);
            if computed != stored {
                return Err(CompiledAssetError::ChecksumMismatch { stored, computed });
            }
            Ok(blob)
        }
        _ => Err(CompiledAssetError::UnsupportedVersion(version)),
    }
}

/// Read the shared envelope body — everything after the magic and
/// version fields, up to and including the payload, but NOT the
/// version-specific terminal handling (v1 ends here; v2 has a CRC
/// footer). Split out so both version arms of [`decode_compiled_asset`]
/// share one body parser and cannot drift apart.
fn read_envelope_body(cursor: &mut ByteReader) -> Result<CompiledAssetBlob, CompiledAssetError> {
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

    /// The magic and current-write version are a wire contract. A
    /// change here means the encoder starts writing a new format — this
    /// test forces that to be a deliberate, reviewed edit. The magic
    /// stays frozen across envelope versions (see the version-field
    /// source-of-truth note on COMPILED_ASSET_VERSION).
    #[test]
    fn envelope_magic_and_version_are_frozen() {
        assert_eq!(&super::COMPILED_ASSET_MAGIC, b"HOLDCMP1");
        assert_eq!(super::COMPILED_ASSET_VERSION, 2);
    }

    /// The CRC-32 implementation is a standard IEEE CRC-32; pin it
    /// against the canonical reference check value so a subtle edit to
    /// the polynomial / init / inversion is caught independently of the
    /// golden fixtures (which use this same function to derive the
    /// footer).
    #[test]
    fn crc32_matches_ieee_reference_vector() {
        assert_eq!(super::crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(super::crc32(b""), 0x0000_0000);
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

    /// The minimal v2 envelope: Mesh kind, all-empty metadata strings,
    /// no dependencies, empty payload — 44 body bytes + a 4-byte CRC-32
    /// footer = 48 bytes. The footer value (0x087132DE) is the CRC of
    /// the 44 preceding bytes; the `crc32_matches_ieee_reference_vector`
    /// test independently proves the CRC function is a correct IEEE
    /// CRC-32, so this fixture pins the exact wire output end to end.
    #[test]
    fn envelope_v2_golden_minimal_48_bytes() {
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
            0x02, 0x00, 0x00, 0x00, // version = 2
            0x01, 0x00, 0x00, 0x00, // kind = Mesh (1)
            0x00, 0x00, 0x00, 0x00, // logical_name len = 0
            0x00, 0x00, 0x00, 0x00, // source_path len = 0
            0x00, 0x00, 0x00, 0x00, // compiler_tag len = 0
            0x00, 0x00, 0x00, 0x00, // runtime_path len = 0
            0x00, 0x00, 0x00, 0x00, // dependency_count = 0
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // payload_len (u64) = 0
            0xDE, 0x32, 0x71, 0x08, // CRC-32 footer (LE) over the 44 bytes above
        ];
        assert_eq!(expected.len(), 48);
        assert_eq!(encode_compiled_asset(&asset).unwrap(), expected);
        assert_eq!(decode_compiled_asset(&expected).unwrap(), asset);
    }

    /// A richer v2 envelope exercising a non-empty logical name, one
    /// dependency, and a two-byte payload — pins the string framing,
    /// dependency-record framing, the asymmetric u64 payload length, and
    /// the CRC footer over all of it.
    #[test]
    fn envelope_v2_golden_with_metadata_and_dependency() {
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
            0x02, 0x00, 0x00, 0x00, // version = 2
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
            0x55, 0xD9, 0xD2, 0xC6, // CRC-32 footer (LE)
        ];
        assert_eq!(encode_compiled_asset(&asset).unwrap(), expected);
        assert_eq!(decode_compiled_asset(&expected).unwrap(), asset);
    }

    /// Reference migration (M10.3): a hand-authored v1 envelope — the
    /// pre-checksum format, version = 1, NO footer — still decodes
    /// correctly through the v1 arm and yields the same in-memory blob a
    /// v2 encode would. This is the load-bearing proof that bumping the
    /// current format to v2 did not orphan already-compiled v1 assets on
    /// disk. The bytes are the exact old format, authored by hand.
    #[test]
    fn envelope_v1_still_decodes_reference_migration() {
        #[rustfmt::skip]
        let v1_bytes: Vec<u8> = vec![
            b'H', b'O', b'L', b'D', b'C', b'M', b'P', b'1', // magic
            0x01, 0x00, 0x00, 0x00, // version = 1 (old format)
            0x0A, 0x00, 0x00, 0x00, // kind = Scene (10)
            0x02, 0x00, 0x00, 0x00, b's', b'0', // logical_name = "s0"
            0x00, 0x00, 0x00, 0x00, // source_path len = 0
            0x00, 0x00, 0x00, 0x00, // compiler_tag len = 0
            0x00, 0x00, 0x00, 0x00, // runtime_path len = 0
            0x00, 0x00, 0x00, 0x00, // dependency_count = 0
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // payload_len (u64) = 1
            0x2A, // payload = [42]
            // NO checksum footer — this is v1.
        ];
        let decoded = decode_compiled_asset(&v1_bytes).unwrap();
        let expected = CompiledAssetBlob {
            metadata: CompiledAssetMetadata {
                kind: CompiledAssetKind::Scene,
                logical_name: "s0".into(),
                source_path: String::new(),
                compiler_tag: String::new(),
                runtime_path: String::new(),
                dependencies: vec![],
            },
            payload: vec![42],
        };
        assert_eq!(decoded, expected);
        // And the same logical asset, re-encoded as current (v2), still
        // round-trips — so a migrate-on-read then re-save is lossless.
        assert_eq!(
            decode_compiled_asset(&encode_compiled_asset(&expected).unwrap()).unwrap(),
            expected
        );
    }

    /// A v2 blob whose payload byte is flipped after encoding decodes
    /// structurally (the layout is intact) but fails the CRC check —
    /// the integrity footer's whole reason to exist. The error is
    /// ChecksumMismatch, distinct from any structural error, so callers
    /// (e.g. version-aware staleness) can treat it as "regenerate".
    #[test]
    fn envelope_v2_detects_single_byte_corruption() {
        let asset = CompiledAssetBlob {
            metadata: CompiledAssetMetadata {
                kind: CompiledAssetKind::Texture,
                logical_name: "t".into(),
                source_path: String::new(),
                compiler_tag: String::new(),
                runtime_path: String::new(),
                dependencies: vec![],
            },
            payload: vec![0x11, 0x22, 0x33, 0x44],
        };
        let mut bytes = encode_compiled_asset(&asset).unwrap();
        // Flip a payload byte (well before the 4-byte footer). Structure
        // stays valid; only the CRC catches it.
        let corrupt_idx = bytes.len() - 5;
        bytes[corrupt_idx] ^= 0xFF;
        match decode_compiled_asset(&bytes).unwrap_err() {
            CompiledAssetError::ChecksumMismatch { .. } => {}
            other => panic!("expected ChecksumMismatch, got {other:?}"),
        }
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
        // 1 and 2 are both valid now; use a version the decoder has no
        // arm for. The version match is checked before the body/footer,
        // so no checksum interaction.
        let mut bytes = minimal_envelope_bytes();
        bytes[8..12].copy_from_slice(&99u32.to_le_bytes()); // version field
        assert_eq!(
            decode_compiled_asset(&bytes).unwrap_err(),
            CompiledAssetError::UnsupportedVersion(99)
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
