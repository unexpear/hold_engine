# Compiled-asset binary format

Status: **normative** (M10). This document is the source of truth for
the on-disk `.hcmp` compiled-asset format. The golden-byte tests in
`src/assets/compiled.rs` and `src/assets/runtime_loader.rs` assert the
exact bytes described here; a change to either must move together.

Related: ADR 0003 Pillars 1 (economics/trust: open, evolvable formats)
and 5 (footprint/source quality: documented binary formats). Reader and
writer live in `src/assets/compiled.rs` (envelope) and
`src/assets/runtime_loader.rs` (payloads).

## Conventions

- **Endianness:** every multi-byte integer and every `f32` is
  **little-endian**.
- **Lengths are byte counts**, not element or character counts, unless a
  field is explicitly a count (e.g. `dependency_count`, `vertex_count`).
- **Strings** are UTF-8, framed as a `u32` byte-length prefix followed by
  exactly that many raw bytes. No NUL terminator.
- **Optional strings** are a single `u8` present-flag (`0` or `1`)
  followed by a framed string when the flag is `1`.
- **Booleans** are a single `u8`: `0` = false, non-zero = true (the
  encoder always writes `0`/`1`).
- The format is **exact-length**: after the last declared field, no
  bytes may remain. Additive extensions are made by bumping a version,
  never by appending to an existing version (see *Trailing bytes*).

## Two version axes

The format carries **two independent versions**:

1. The **envelope version** (`COMPILED_ASSET_VERSION`) — the wrapper
   framing (magic, metadata, dependencies, payload container, integrity
   footer). Current: **2**.
2. A **per-kind payload version** — the inner bytes for a specific asset
   kind. Each kind (mesh, texture, material, shader, scene) has its own
   version so changing one kind's layout does not re-version the others.
   Current: mesh **3**, texture **4**, material **3**, shader **3**,
   scene **3**.

A migration table is therefore keyed by *(kind, payload version)* for
payloads and by *(envelope version)* for the wrapper.

## Magic vs. version (source-of-truth rule)

The 8-byte magic is `HOLDCMP1` (ASCII). It is **frozen across envelope
versions** — it is NOT bumped to `HOLDCMP2`, `HOLDCMP3`, … The `u32`
version field immediately after the magic is the **sole source of truth**
for the envelope version.

Rationale: if the magic were bumped per version, an old reader hitting a
newer file would fail with `InvalidMagic` — indistinguishable from "this
isn't a compiled asset at all." Freezing the magic and advancing only the
version field means an old reader fails with `UnsupportedVersion(n)`,
which is actionable ("rebuild your assets / upgrade the engine").

## Envelope

### Version 2 (current)

| Field | Type | Notes |
|-------|------|-------|
| `magic` | `[u8; 8]` | `HOLDCMP1` |
| `version` | `u32` | `2` |
| `kind` | `u32` | asset-kind tag (see *Kind tags*) |
| `logical_name` | string | e.g. `materials/foliage/palm` |
| `source_path` | string | authored source path |
| `compiler_tag` | string | compiler identity/version |
| `runtime_path` | string | intended runtime path |
| `dependency_count` | `u32` | number of dependency records |
| `dependencies[]` | record × count | each: `kind: u32`, `logical_name: string`, `source_path: string` |
| `payload_len` | `u64` | payload byte count (note: **u64**, unlike the `u32` string/count prefixes) |
| `payload` | `[u8; payload_len]` | opaque; interpreted per kind |
| `checksum` | `u32` | CRC-32 (IEEE) over **every preceding byte** |

The minimal v2 envelope (Mesh kind, empty strings, no dependencies,
empty payload) is **48 bytes**: 44 body bytes + a 4-byte footer.

### Version 1 (legacy, still decodable)

Identical to v2 **without** the trailing `checksum` field. The decoder
still reads v1 (migration): a v1 blob simply skips the integrity check.
The encoder no longer writes v1. The minimal v1 envelope is 44 bytes.

### Integrity checksum (v2+)

`checksum` is a standard IEEE 802.3 / zlib CRC-32: reversed polynomial
`0xEDB88320`, initial value `0xFFFFFFFF`, final inversion. Reference
value: `crc32("123456789") == 0xCBF43926`. It covers `magic || version ||
body || payload` — i.e. all bytes before the footer. A mismatch yields a
typed `ChecksumMismatch { stored, computed }`: the layout parsed cleanly
but the bytes are corrupt on disk, so the correct response is "regenerate
from source", not "the format is wrong".

### Kind tags

`1` Mesh, `2` Material, `3` Texture, `4` TerrainLayerSet,
`5` VegetationPrototype, `6` Skeleton, `7` AnimationClip, `8` AudioClip,
`9` Shader, `10` Scene. Tags 4–8 are reserved (no payload implemented
yet). An unknown tag is a typed `InvalidKind(raw)`.

## Payloads

Every payload begins with a `u32` payload version, then kind-specific
fields. Normal maps reuse the **Texture** kind and payload.

### Mesh — version 3

| Field | Type |
|-------|------|
| `version` | `u32` = 3 |
| `vertex_count` | `u32` |
| `index_count` | `u32` |
| `vertices[]` | 12 × `f32` each: `pos[3]`, `normal[3]`, `uv[2]`, `tangent[4]` (48 bytes/vertex) |
| `indices[]` | `u32` each |

Payload-version history (shared-counter era, pre-M10.2): v1 = pos/normal/uv;
v2 = added tangent (ADR 0002 Q4); v3 = texture colour-space tag. Mesh's
current shape is the v2 layout carried at counter value 3.

### Texture — versions 3 and 4

Version 4 (current):

| Field | Type | Notes |
|-------|------|-------|
| `version` | `u32` = 4 |
| `width` | `u32` | |
| `height` | `u32` | |
| `format` | `u32` | `1` Rgba8, `2` Rgb8, `3` R8 |
| `color_space` | `u32` | `1` Srgb, `2` Linear (ADR 0002 Q3) |
| `convention` | `u32` | normal-map convention: `1` OpenGL, `2` DirectX (M10.4) |
| `data_len` | `u64` | pixel byte count |
| `data` | `[u8; data_len]` | raw pixels in `format` layout |

Version 3 is identical **without** the `convention` field. The stored
pixels are the **raw source bytes**. The `convention` tag describes their
authored orientation; it is meaningful only when the payload is resolved
as a normal map:

- As a colour texture (`decode_compiled_texture_payload`): the convention
  is ignored.
- As a normal map (`decode_compiled_normal_map_payload`): an OpenGL
  payload loads unflipped; a DirectX payload has its green channel flipped
  to the engine's canonical OpenGL Y+ on load, and the source convention
  is recorded on the `NormalMap`.

**v3 migration:** a v3 texture payload has no `convention` field and
defaults to OpenGL — byte-for-byte the pre-M10.4 behaviour, so existing v3
blobs resolve identically.

### Material — version 3

| Field | Type |
|-------|------|
| `version` | `u32` = 3 |
| `albedo_logical_name` | string |
| `normal_logical_name` | optional string |
| `specular_gloss_logical_name` | optional string |
| `double_sided` | `u8` bool |
| `alpha_cutout` | `u8` bool |

Material references its textures by **logical name**, not by handle — the
format is decoupled from runtime store layout.

### Shader — version 3

| Field | Type |
|-------|------|
| `version` | `u32` = 3 |
| `name` | string |
| `vertex_source` | string (GLSL) |
| `fragment_source` | string (GLSL) |

### Scene — version 3

| Field | Type |
|-------|------|
| `version` | `u32` = 3 |
| `name` | string |
| `hscene_text` | string (canonical `.hscene` serialization) |

## Robustness guarantees

- **Bounded allocation.** Every untrusted count/length is checked against
  the remaining buffer before any `Vec::with_capacity`
  (`DeclaredLengthExceedsBuffer { declared, remaining }`). A crafted
  header cannot request a large speculative allocation from a few on-disk
  bytes.
- **Trailing bytes.** Decoding enforces exact length: bytes after the last
  declared field are a typed `TrailingBytes { extra }`, never silently
  ignored. This is why additive extensions require a version bump — an old
  strict reader must reject a file that appended new data.
- **Typed errors.** No `String` errors, no silent fallbacks. Structural
  problems (`InvalidMagic`, `UnsupportedVersion`, `InvalidKind`,
  `UnexpectedEof`, `Utf8`, enum-tag errors), integrity problems
  (`ChecksumMismatch`), and loader problems (`UnexpectedKind`,
  `MetadataMismatch`, IO) are distinct.

## Versioning policy

- **Bumping.** A change to the envelope framing bumps
  `COMPILED_ASSET_VERSION`; a change to one kind's payload bumps only that
  kind's payload version. Add a new decoder arm for the new version and
  keep the old arm for migration; the frozen-version and golden tests must
  be updated in the same commit.
- **Migration on read.** Older versions remain decodable so a shipped
  runtime can read an older on-disk asset pack. New reader + old bytes is
  supported; old reader + new bytes fails loudly (`UnsupportedVersion`).
- **Staleness vs. migration.** These are complementary, not redundant:
  - `source_asset_is_stale` treats a blob whose envelope or payload
    version is behind current — or that fails to decode (corruption /
    failed checksum) — as **stale**, so the editor/build **regenerates it
    from source**. This is the primary path when source is available.
  - Migration (decoding an older version) is the **fallback** for when
    source is *not* available (a shipped runtime reading an old pack). It
    is always exercised by its unit tests even when the editor prefers
    recompilation.

## Downgrade / pinned-version emission

Not supported. The encoder always writes the current version. There is no
API to target an older version. Add one only if a shipped older runtime
must consume newly-compiled assets — not a current requirement.
