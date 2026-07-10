use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::runtime_loader::expected_payload_version;
use super::{
    compiled_asset_path_for_logical_name, decode_compiled_asset, encode_compiled_asset,
    encode_compiled_material_payload, encode_compiled_mesh_payload, encode_compiled_scene_payload,
    encode_compiled_shader_payload, encode_compiled_texture_payload, load_texture,
    CompiledAssetBlob, CompiledAssetDependency, CompiledAssetError, CompiledAssetKind,
    CompiledAssetMetadata, CompiledMaterialPayload, CompiledScenePayload, CompiledShaderPayload,
    MeshData, ObjError, TexError, COMPILED_ASSET_MAGIC, COMPILED_ASSET_VERSION,
};

const COMPILER_TAG: &str = "hold_assetc@runtime-slice-0.1";

#[derive(Debug)]
pub enum AssetCompilerError {
    Io(io::Error),
    Obj(ObjError),
    Texture(TexError),
    Compiled(CompiledAssetError),
    UnsupportedKind(CompiledAssetKind),
    InvalidMaterialLine { line: usize, message: String },
    MissingMaterialField(&'static str),
    /// `.hshd` shader source failed to parse — missing section,
    /// unknown directive, or empty body. The string carries enough
    /// context for the editor pane to display alongside the
    /// dispatcher's `CompileFailed` event.
    InvalidShaderSource(String),
    /// `.hsc` scene source failed to parse. Carries the line number
    /// and message from `crate::scene::SceneError` so the editor's
    /// hot-reload status panel can surface "scene line 42: unknown
    /// directive" rather than a generic compile failure. See ADR
    /// 0004 Q14 (M7.4.1 — scene hot-reload).
    InvalidSceneSource { line: usize, message: String },
}

impl fmt::Display for AssetCompilerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::Obj(err) => write!(f, "mesh import failed: {err}"),
            Self::Texture(err) => write!(f, "texture import failed: {err}"),
            Self::Compiled(err) => write!(f, "compiled asset encode failed: {err}"),
            Self::UnsupportedKind(kind) => write!(f, "unsupported compiler kind {:?}", kind),
            Self::InvalidMaterialLine { line, message } => {
                write!(f, "invalid material source on line {line}: {message}")
            }
            Self::MissingMaterialField(field) => {
                write!(f, "material source missing required field `{field}`")
            }
            Self::InvalidShaderSource(message) => {
                write!(f, "invalid shader source: {message}")
            }
            Self::InvalidSceneSource { line, message } => {
                write!(f, "invalid scene source on line {line}: {message}")
            }
        }
    }
}

impl std::error::Error for AssetCompilerError {}

impl From<io::Error> for AssetCompilerError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<ObjError> for AssetCompilerError {
    fn from(value: ObjError) -> Self {
        Self::Obj(value)
    }
}

impl From<TexError> for AssetCompilerError {
    fn from(value: TexError) -> Self {
        Self::Texture(value)
    }
}

impl From<CompiledAssetError> for AssetCompilerError {
    fn from(value: CompiledAssetError) -> Self {
        Self::Compiled(value)
    }
}

pub fn parse_compiled_material_source(
    source: &str,
) -> Result<CompiledMaterialPayload, AssetCompilerError> {
    let mut albedo_logical_name = None;
    let mut normal_logical_name = None;
    let mut specular_gloss_logical_name = None;
    let mut double_sided = false;
    let mut alpha_cutout = false;

    for (index, raw_line) in source.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.split_whitespace();
        let key = parts.next().unwrap_or_default();
        let value = parts
            .next()
            .ok_or_else(|| AssetCompilerError::InvalidMaterialLine {
                line: line_number,
                message: "expected a value after the key".to_string(),
            })?;
        if parts.next().is_some() {
            return Err(AssetCompilerError::InvalidMaterialLine {
                line: line_number,
                message: "expected exactly one value per line".to_string(),
            });
        }

        match key {
            "albedo" | "albedo_logical_name" => {
                albedo_logical_name = Some(value.to_string());
            }
            "normal" | "normal_logical_name" => {
                normal_logical_name = parse_optional_logical_name(value);
            }
            "specular_gloss" | "specular_gloss_logical_name" => {
                specular_gloss_logical_name = parse_optional_logical_name(value);
            }
            "double_sided" => {
                double_sided = parse_bool_token(line_number, value)?;
            }
            "alpha_cutout" => {
                alpha_cutout = parse_bool_token(line_number, value)?;
            }
            other => {
                return Err(AssetCompilerError::InvalidMaterialLine {
                    line: line_number,
                    message: format!("unknown material key `{other}`"),
                });
            }
        }
    }

    Ok(CompiledMaterialPayload {
        albedo_logical_name: albedo_logical_name
            .ok_or(AssetCompilerError::MissingMaterialField("albedo"))?,
        normal_logical_name,
        specular_gloss_logical_name,
        double_sided,
        alpha_cutout,
    })
}

pub fn compile_mesh_asset(
    source_path: &Path,
    logical_name: &str,
) -> Result<CompiledAssetBlob, AssetCompilerError> {
    let mesh = MeshData::from_obj(source_path)?;
    let payload = encode_compiled_mesh_payload(&mesh)?;
    Ok(compose_blob(
        CompiledAssetKind::Mesh,
        logical_name,
        source_path,
        payload,
        Vec::new(),
    )?)
}

pub fn compile_asset_from_source(
    kind: CompiledAssetKind,
    source_path: &Path,
    logical_name: &str,
) -> Result<CompiledAssetBlob, AssetCompilerError> {
    match kind {
        CompiledAssetKind::Mesh => compile_mesh_asset(source_path, logical_name),
        CompiledAssetKind::Material => compile_material_asset(source_path, logical_name),
        CompiledAssetKind::Texture => compile_texture_asset(source_path, logical_name),
        CompiledAssetKind::Shader => compile_shader_asset(source_path, logical_name),
        CompiledAssetKind::Scene => compile_scene_asset(source_path, logical_name),
        _ => Err(AssetCompilerError::UnsupportedKind(kind)),
    }
}

/// Compile a `.hsc` scene source file into a
/// [`crate::assets::CompiledAssetBlob`] of kind
/// [`CompiledAssetKind::Scene`].
///
/// ## Pipeline
///
/// 1. Read the `.hsc` text from disk.
/// 2. Parse via `crate::scene::parse_scene` → `SceneData`.
/// 3. Convert to `crate::editor::SceneDocument::from_scene_data`.
/// 4. Re-serialize through `to_hscene_string` to produce the
///    canonical text payload.
///
/// The compile step is therefore a *parse-validate-canonicalize*
/// pass: it rejects malformed `.hsc` files at compile time so the
/// dispatcher's runtime decode path can assume the payload is
/// well-formed (no need for a second error surface there).
///
/// The hot-reload contract — runtime entity preservation across
/// reloads — is enforced downstream in
/// `EditorSceneSession::reload_scene_document`. This function only
/// produces the validated payload; it does not touch the runtime
/// `World`.
///
/// See ADR 0004 Q14 + Q14a (M7.4.1).
pub fn compile_scene_asset(
    source_path: &Path,
    logical_name: &str,
) -> Result<CompiledAssetBlob, AssetCompilerError> {
    let source = fs::read_to_string(source_path)?;
    // crate::scene::parse_scene returns SceneError on malformed
    // input. We surface its line+message in InvalidSceneSource so
    // the editor pane can route the failure to the same panel that
    // shows compile errors for materials, shaders, etc.
    let scene = crate::scene::parse_scene(&source).map_err(|err| {
        AssetCompilerError::InvalidSceneSource {
            line: err.line,
            message: err.msg,
        }
    })?;
    let document = crate::editor::SceneDocument::from_scene_data(&scene);
    let hscene_text = document.to_hscene_string();
    let payload = CompiledScenePayload {
        name: logical_name.to_string(),
        hscene_text,
    };
    let encoded = encode_compiled_scene_payload(&payload)?;
    Ok(compose_blob(
        CompiledAssetKind::Scene,
        logical_name,
        source_path,
        encoded,
        Vec::new(),
    )?)
}

/// Compile a `.hshd` shader source file into a
/// [`crate::assets::CompiledAssetBlob`] of kind
/// [`CompiledAssetKind::Shader`].
///
/// ## Source format
///
/// A `.hshd` file contains both vertex and fragment GLSL in one
/// document, delimited by lines starting with the marker
/// `#shader vertex` or `#shader fragment`. Anything before the
/// first marker is treated as a header comment and discarded.
/// Markers are case-sensitive and must occupy their own line; the
/// rest of the line is ignored so authors can leave a trailing
/// comment.
///
/// Example:
///
/// ```text
/// // textured-pbr.hshd
/// #shader vertex
/// #version 330 core
/// layout(location = 0) in vec3 a_pos;
/// // … vertex source …
///
/// #shader fragment
/// #version 330 core
/// out vec4 frag_color;
/// // … fragment source …
/// ```
///
/// Both sections must be present and non-empty; either missing
/// produces [`AssetCompilerError::InvalidShaderSource`]. The
/// per-section text is captured verbatim so `#version` directives,
/// uniform declarations, and `#extension` pragmas pass through
/// untouched.
///
/// See ADR 0004 Q9 (M7.4.1).
pub fn compile_shader_asset(
    source_path: &Path,
    logical_name: &str,
) -> Result<CompiledAssetBlob, AssetCompilerError> {
    let source = fs::read_to_string(source_path)?;
    let payload = parse_shader_source(&source, logical_name)?;
    let encoded = encode_compiled_shader_payload(&payload)?;
    Ok(compose_blob(
        CompiledAssetKind::Shader,
        logical_name,
        source_path,
        encoded,
        Vec::new(),
    )?)
}

/// Parse a `.hshd` source string into a [`CompiledShaderPayload`].
/// Public for testing — the production path goes through
/// [`compile_shader_asset`] which reads from disk first.
pub fn parse_shader_source(
    source: &str,
    logical_name: &str,
) -> Result<CompiledShaderPayload, AssetCompilerError> {
    #[derive(PartialEq, Eq)]
    enum Section {
        None,
        Vertex,
        Fragment,
    }
    let mut current = Section::None;
    let mut vertex = String::new();
    let mut fragment = String::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("#shader") {
            // Marker line. The first whitespace-separated token
            // after `#shader` names the section. Rest of the
            // line ignored so authors can leave a trailing
            // comment.
            let next = rest.split_whitespace().next();
            current = match next {
                Some("vertex") => Section::Vertex,
                Some("fragment") => Section::Fragment,
                Some(other) => {
                    return Err(AssetCompilerError::InvalidShaderSource(format!(
                        "unknown shader section {:?} (expected `vertex` or `fragment`)",
                        other
                    )));
                }
                None => {
                    return Err(AssetCompilerError::InvalidShaderSource(
                        "`#shader` directive missing section name".into(),
                    ));
                }
            };
            continue;
        }
        match current {
            Section::Vertex => {
                vertex.push_str(line);
                vertex.push('\n');
            }
            Section::Fragment => {
                fragment.push_str(line);
                fragment.push('\n');
            }
            Section::None => {
                // Pre-marker preamble — silently ignored. Lets
                // authors put a license header or doc comment at
                // the top of the file.
            }
        }
    }

    if vertex.trim().is_empty() {
        return Err(AssetCompilerError::InvalidShaderSource(
            "shader source missing vertex section".into(),
        ));
    }
    if fragment.trim().is_empty() {
        return Err(AssetCompilerError::InvalidShaderSource(
            "shader source missing fragment section".into(),
        ));
    }

    Ok(CompiledShaderPayload {
        name: logical_name.to_string(),
        vertex_source: vertex,
        fragment_source: fragment,
    })
}

pub fn source_asset_candidates(
    source_root: &Path,
    logical_name: &str,
    kind: CompiledAssetKind,
) -> Vec<PathBuf> {
    let base = logical_name_to_source_base(source_root, logical_name);
    match kind {
        CompiledAssetKind::Mesh => vec![base.with_extension("obj")],
        CompiledAssetKind::Material => vec![base.with_extension("hmat")],
        CompiledAssetKind::Texture => vec![base.with_extension("bmp"), base.with_extension("tga")],
        CompiledAssetKind::Shader => vec![base.with_extension("hshd")],
        CompiledAssetKind::Scene => vec![base.with_extension("hsc")],
        _ => Vec::new(),
    }
}

pub fn resolve_source_asset_path(
    source_root: &Path,
    logical_name: &str,
    kind: CompiledAssetKind,
) -> Option<PathBuf> {
    source_asset_candidates(source_root, logical_name, kind)
        .into_iter()
        .find(|path| path.exists())
}

pub fn source_asset_is_stale(
    compiled_root: &Path,
    source_path: &Path,
    logical_name: &str,
) -> Result<bool, AssetCompilerError> {
    let compiled_path = compiled_asset_path_for_logical_name(compiled_root, logical_name);
    if !compiled_path.exists() {
        return Ok(true);
    }

    // Mtime staleness: the source has been edited since the last compile.
    let source_modified = fs::metadata(source_path)?.modified()?;
    let compiled_modified = fs::metadata(&compiled_path)?.modified()?;
    if source_modified > compiled_modified {
        return Ok(true);
    }

    // Format staleness (M10.3): a compiled blob from an older envelope
    // or payload version — or one that no longer decodes (corruption,
    // failed checksum) — must be regenerated even when its mtime is
    // newer than the source. Without this, bumping a format version
    // leaves stale blobs that pass the mtime check and only explode at
    // runtime decode. Reading the blob here costs one file read on the
    // not-mtime-stale path; staleness runs at build/import time, never
    // in the render loop, so that cost is acceptable.
    let compiled_bytes = fs::read(&compiled_path)?;
    Ok(!compiled_blob_is_current(&compiled_bytes))
}

/// Whether an on-disk compiled blob matches the CURRENT format on both
/// version axes. Returns `false` (⇒ stale ⇒ recompile) when the
/// envelope version is old, when the blob no longer decodes (structural
/// corruption or a failed CRC), or when the payload version is old.
fn compiled_blob_is_current(bytes: &[u8]) -> bool {
    // Envelope version at its fixed offset (after the 8-byte magic).
    // A v1 blob is still *decodable* via migration, so decode success
    // alone doesn't mean "current" — check the written version directly.
    if bytes.len() < 12 || bytes[..8] != COMPILED_ASSET_MAGIC {
        return false;
    }
    let envelope_version = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if envelope_version != COMPILED_ASSET_VERSION {
        return false;
    }

    // Full decode validates structure + the v2 CRC footer; any error
    // means the blob can't be trusted and should be regenerated.
    let blob = match decode_compiled_asset(bytes) {
        Ok(blob) => blob,
        Err(_) => return false,
    };

    // Payload version (first 4 bytes of the payload), compared against
    // the current per-kind version. Kinds without an encoded payload
    // (None) are considered current once the envelope check passes.
    match expected_payload_version(blob.metadata.kind) {
        Some(expected) => {
            blob.payload.len() >= 4
                && u32::from_le_bytes([
                    blob.payload[0],
                    blob.payload[1],
                    blob.payload[2],
                    blob.payload[3],
                ]) == expected
        }
        None => true,
    }
}

pub fn compile_texture_asset(
    source_path: &Path,
    logical_name: &str,
) -> Result<CompiledAssetBlob, AssetCompilerError> {
    let texture = load_texture(source_path)?;
    let payload = encode_compiled_texture_payload(&texture)?;
    Ok(compose_blob(
        CompiledAssetKind::Texture,
        logical_name,
        source_path,
        payload,
        Vec::new(),
    )?)
}

pub fn compile_material_asset(
    source_path: &Path,
    logical_name: &str,
) -> Result<CompiledAssetBlob, AssetCompilerError> {
    let source = fs::read_to_string(source_path)?;
    let payload = parse_compiled_material_source(&source)?;
    let encoded_payload = encode_compiled_material_payload(&payload)?;
    let mut dependencies = vec![texture_dependency(&payload.albedo_logical_name)];
    if let Some(name) = payload.normal_logical_name.as_deref() {
        dependencies.push(texture_dependency(name));
    }
    if let Some(name) = payload.specular_gloss_logical_name.as_deref() {
        dependencies.push(texture_dependency(name));
    }

    Ok(compose_blob(
        CompiledAssetKind::Material,
        logical_name,
        source_path,
        encoded_payload,
        dependencies,
    )?)
}

/// Write a compiled asset to the compiled tree.
///
/// # Why this is deliberately NOT an atomic write (M11 decision record)
///
/// M11 routed every *authored* writer (`.hsc`, `.hscene`, `.hproj`) through
/// [`crate::platform::atomic_write`], because authored work is irreplaceable:
/// a torn write destroys something the user cannot get back.
///
/// Compiled assets are the opposite: **derived, regenerable data**. M10 gave
/// the envelope a CRC-32 integrity footer and made `source_asset_is_stale`
/// version- and checksum-aware, so a torn `.hcmp` fails its CRC, reads as
/// *stale*, and is transparently regenerated from the authored source. The
/// worst case is one recompile — never data loss.
///
/// Making this atomic would therefore buy nothing, while costing a real
/// `FlushFileBuffers` per asset across a full build of hundreds or thousands
/// of files. That is a measurable build-time cost for zero authored-data
/// benefit, so the plain `fs::write` stays on purpose.
///
/// This is a decision, not an oversight. If compiled assets ever become
/// non-regenerable (e.g. a shipped asset pack with no sources), revisit it.
pub fn write_compiled_asset(
    compiled_root: &Path,
    blob: &CompiledAssetBlob,
) -> Result<PathBuf, AssetCompilerError> {
    let path = compiled_asset_path_for_logical_name(compiled_root, &blob.metadata.logical_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let encoded = encode_compiled_asset(blob)?;
    fs::write(&path, encoded)?;
    Ok(path)
}

fn compose_blob(
    kind: CompiledAssetKind,
    logical_name: &str,
    source_path: &Path,
    payload: Vec<u8>,
    dependencies: Vec<CompiledAssetDependency>,
) -> Result<CompiledAssetBlob, AssetCompilerError> {
    Ok(CompiledAssetBlob {
        metadata: CompiledAssetMetadata {
            kind,
            logical_name: logical_name.to_string(),
            source_path: source_path.to_string_lossy().into_owned(),
            compiler_tag: COMPILER_TAG.to_string(),
            runtime_path: compiled_runtime_path(logical_name),
            dependencies,
        },
        payload,
    })
}

fn compiled_runtime_path(logical_name: &str) -> String {
    format!("compiled/{logical_name}.hcmp")
}

fn logical_name_to_source_base(source_root: &Path, logical_name: &str) -> PathBuf {
    let mut path = source_root.to_path_buf();
    for segment in logical_name.split('/') {
        path.push(segment);
    }
    path
}

fn texture_dependency(logical_name: &str) -> CompiledAssetDependency {
    CompiledAssetDependency {
        kind: CompiledAssetKind::Texture,
        logical_name: logical_name.to_string(),
        source_path: String::new(),
    }
}

fn parse_optional_logical_name(token: &str) -> Option<String> {
    if token == "-" || token.eq_ignore_ascii_case("none") {
        None
    } else {
        Some(token.to_string())
    }
}

fn parse_bool_token(line: usize, token: &str) -> Result<bool, AssetCompilerError> {
    if token.eq_ignore_ascii_case("true") || token.eq_ignore_ascii_case("yes") || token == "1" {
        Ok(true)
    } else if token.eq_ignore_ascii_case("false")
        || token.eq_ignore_ascii_case("no")
        || token == "0"
    {
        Ok(false)
    } else {
        Err(AssetCompilerError::InvalidMaterialLine {
            line,
            message: format!("expected boolean, found `{token}`"),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{
        compile_shader_asset, parse_compiled_material_source, parse_shader_source,
        resolve_source_asset_path, source_asset_candidates, source_asset_is_stale,
        AssetCompilerError,
    };
    use crate::assets::{compiled_asset_path_for_logical_name, encode_compiled_asset};
    use crate::assets::{
        encode_compiled_mesh_payload, CompiledAssetBlob, CompiledAssetKind, CompiledAssetMetadata,
        MeshData,
    };

    fn unique_test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "hold_asset_compiler_{}_{}_{}",
            label,
            std::process::id(),
            nonce
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn parse_material_source_supports_optional_maps_and_flags() {
        let source = "\
# test material
albedo island/foliage/palm_albedo
normal island/foliage/palm_normal
specular_gloss none
double_sided true
alpha_cutout 1
";

        let payload = parse_compiled_material_source(source).unwrap();
        assert_eq!(payload.albedo_logical_name, "island/foliage/palm_albedo");
        assert_eq!(
            payload.normal_logical_name.as_deref(),
            Some("island/foliage/palm_normal")
        );
        assert_eq!(payload.specular_gloss_logical_name, None);
        assert!(payload.double_sided);
        assert!(payload.alpha_cutout);
    }

    #[test]
    fn parse_material_source_requires_albedo() {
        let err = parse_compiled_material_source("double_sided false\n").unwrap_err();
        assert!(format!("{err}").contains("missing required field `albedo`"));
    }

    #[test]
    fn resolve_source_asset_path_checks_expected_extensions() {
        let root = unique_test_root("resolve");
        let textures_dir = root.join("textures");
        fs::create_dir_all(&textures_dir).unwrap();
        fs::write(textures_dir.join("sand.tga"), b"tga").unwrap();

        let candidates =
            source_asset_candidates(&root, "textures/sand", CompiledAssetKind::Texture);
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            resolve_source_asset_path(&root, "textures/sand", CompiledAssetKind::Texture).unwrap(),
            textures_dir.join("sand.tga")
        );

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn source_asset_is_stale_detects_missing_or_outdated_compiled_assets() {
        let root = unique_test_root("stale");
        let source_root = root.join("source");
        let compiled_root = root.join("compiled");
        fs::create_dir_all(source_root.join("meshes")).unwrap();
        fs::create_dir_all(compiled_root.join("meshes")).unwrap();

        let source_path = source_root.join("meshes").join("crate.obj");
        fs::write(
            &source_path,
            "o crate\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
        )
        .unwrap();

        assert!(source_asset_is_stale(&compiled_root, &source_path, "meshes/crate").unwrap());

        let compiled_path = compiled_asset_path_for_logical_name(&compiled_root, "meshes/crate");
        let blob = CompiledAssetBlob {
            metadata: CompiledAssetMetadata {
                kind: CompiledAssetKind::Mesh,
                logical_name: "meshes/crate".into(),
                source_path: source_path.display().to_string(),
                compiler_tag: "test".into(),
                runtime_path: "compiled/meshes/crate.hcmp".into(),
                dependencies: Vec::new(),
            },
            payload: encode_compiled_mesh_payload(&MeshData::cube()).unwrap(),
        };
        fs::write(&compiled_path, encode_compiled_asset(&blob).unwrap()).unwrap();
        assert!(!source_asset_is_stale(&compiled_root, &source_path, "meshes/crate").unwrap());

        thread::sleep(Duration::from_millis(10));
        fs::write(
            &source_path,
            "o crate\nv 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nf 1 2 3\n",
        )
        .unwrap();
        assert!(source_asset_is_stale(&compiled_root, &source_path, "meshes/crate").unwrap());

        fs::remove_dir_all(root).ok();
    }

    /// Version-aware staleness (M10.3): a compiled blob written from an
    /// OLDER envelope version is stale even when its file mtime is newer
    /// than the source. Without this, bumping the format leaves blobs
    /// that pass the mtime check and blow up at runtime decode.
    #[test]
    fn source_asset_is_stale_detects_old_format_version() {
        let root = unique_test_root("stale_format");
        let source_root = root.join("source");
        let compiled_root = root.join("compiled");
        fs::create_dir_all(source_root.join("meshes")).unwrap();
        fs::create_dir_all(compiled_root.join("meshes")).unwrap();
        let source_path = source_root.join("meshes").join("crate.obj");
        fs::write(&source_path, "o crate\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();

        let compiled_path = compiled_asset_path_for_logical_name(&compiled_root, "meshes/crate");
        let blob = CompiledAssetBlob {
            metadata: CompiledAssetMetadata {
                kind: CompiledAssetKind::Mesh,
                logical_name: "meshes/crate".into(),
                source_path: source_path.display().to_string(),
                compiler_tag: "test".into(),
                runtime_path: "compiled/meshes/crate.hcmp".into(),
                dependencies: Vec::new(),
            },
            payload: encode_compiled_mesh_payload(&MeshData::cube()).unwrap(),
        };

        // A current (v2) blob written after the source is fresh.
        fs::write(&compiled_path, encode_compiled_asset(&blob).unwrap()).unwrap();
        assert!(!source_asset_is_stale(&compiled_root, &source_path, "meshes/crate").unwrap());

        // Downgrade the on-disk blob to a v1 envelope (patch the version
        // field to 1, drop the v2 CRC footer) and give it the newest
        // mtime. mtime says fresh, but the envelope version is behind the
        // current v2 -> stale.
        let mut v1_bytes = encode_compiled_asset(&blob).unwrap();
        v1_bytes.truncate(v1_bytes.len() - 4); // remove CRC footer
        v1_bytes[8..12].copy_from_slice(&1u32.to_le_bytes()); // version -> 1
        thread::sleep(Duration::from_millis(10));
        fs::write(&compiled_path, &v1_bytes).unwrap();
        assert!(source_asset_is_stale(&compiled_root, &source_path, "meshes/crate").unwrap());

        fs::remove_dir_all(root).ok();
    }

    /// A corrupt compiled blob (failed CRC) reads as stale so it gets
    /// regenerated from source rather than failing at runtime decode.
    #[test]
    fn source_asset_is_stale_treats_corruption_as_stale() {
        let root = unique_test_root("stale_corrupt");
        let source_root = root.join("source");
        let compiled_root = root.join("compiled");
        fs::create_dir_all(source_root.join("meshes")).unwrap();
        fs::create_dir_all(compiled_root.join("meshes")).unwrap();
        let source_path = source_root.join("meshes").join("crate.obj");
        fs::write(&source_path, "o crate\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();

        let compiled_path = compiled_asset_path_for_logical_name(&compiled_root, "meshes/crate");
        let blob = CompiledAssetBlob {
            metadata: CompiledAssetMetadata {
                kind: CompiledAssetKind::Mesh,
                logical_name: "meshes/crate".into(),
                source_path: source_path.display().to_string(),
                compiler_tag: "test".into(),
                runtime_path: "compiled/meshes/crate.hcmp".into(),
                dependencies: Vec::new(),
            },
            payload: encode_compiled_mesh_payload(&MeshData::cube()).unwrap(),
        };

        // Flip a payload byte so the structure stays valid but the CRC
        // fails, then give it the newest mtime.
        let mut corrupt = encode_compiled_asset(&blob).unwrap();
        let idx = corrupt.len() - 8; // inside the mesh payload, before footer
        corrupt[idx] ^= 0xFF;
        fs::write(&compiled_path, &corrupt).unwrap();
        assert!(source_asset_is_stale(&compiled_root, &source_path, "meshes/crate").unwrap());

        fs::remove_dir_all(root).ok();
    }

    // ── parse_shader_source tests ─────────────────────────────────────────

    /// Happy path: a `.hshd` file with both sections produces a
    /// payload whose vertex_source and fragment_source are the
    /// verbatim section bodies. The marker lines themselves do NOT
    /// appear in the output.
    #[test]
    fn parse_shader_source_extracts_both_sections_verbatim() {
        let src = "// preamble comment\n\
                   #shader vertex\n\
                   #version 330 core\n\
                   layout(location = 0) in vec3 a_pos;\n\
                   void main() { gl_Position = vec4(a_pos, 1.0); }\n\
                   \n\
                   #shader fragment\n\
                   #version 330 core\n\
                   out vec4 frag_color;\n\
                   void main() { frag_color = vec4(1.0); }\n";
        let payload = parse_shader_source(src, "shaders/test").unwrap();
        assert_eq!(payload.name, "shaders/test");
        assert!(payload.vertex_source.contains("#version 330"));
        assert!(payload.vertex_source.contains("a_pos"));
        assert!(!payload.vertex_source.contains("#shader"));
        assert!(payload.fragment_source.contains("frag_color"));
        assert!(!payload.fragment_source.contains("#shader"));
        // Pre-marker preamble silently ignored.
        assert!(!payload.vertex_source.contains("preamble comment"));
        assert!(!payload.fragment_source.contains("preamble comment"));
    }

    /// Missing fragment section surfaces as a typed error so the
    /// editor pane can display "shader source missing fragment
    /// section" without the developer having to read GLSL output.
    #[test]
    fn parse_shader_source_rejects_missing_fragment() {
        let src = "#shader vertex\n#version 330 core\nvoid main() {}\n";
        match parse_shader_source(src, "shaders/test") {
            Err(AssetCompilerError::InvalidShaderSource(msg)) => {
                assert!(
                    msg.contains("fragment"),
                    "error names the missing section: {}",
                    msg
                );
            }
            other => panic!("expected InvalidShaderSource, got {:?}", other),
        }
    }

    /// Empty vertex section is rejected the same way. Pillar 4
    /// (debuggability): a developer who left the vertex body
    /// empty by accident gets a typed error pointing at the right
    /// section, not a confusing GLSL compile error from the GPU
    /// driver three layers downstream.
    #[test]
    fn parse_shader_source_rejects_empty_vertex_section() {
        let src = "#shader vertex\n\n#shader fragment\n#version 330 core\nvoid main() {}\n";
        match parse_shader_source(src, "shaders/test") {
            Err(AssetCompilerError::InvalidShaderSource(msg)) => {
                assert!(msg.contains("vertex"));
            }
            other => panic!("expected InvalidShaderSource, got {:?}", other),
        }
    }

    /// Unknown section name (e.g. typo `vertx` instead of
    /// `vertex`) surfaces with the offending token in the error
    /// message so the developer can fix the typo from the message
    /// alone.
    #[test]
    fn parse_shader_source_rejects_unknown_section_name() {
        let src = "#shader vertx\n#version 330 core\nvoid main() {}\n";
        match parse_shader_source(src, "shaders/test") {
            Err(AssetCompilerError::InvalidShaderSource(msg)) => {
                assert!(
                    msg.contains("vertx"),
                    "error names the offending token: {}",
                    msg
                );
            }
            other => panic!("expected InvalidShaderSource, got {:?}", other),
        }
    }

    /// `compile_shader_asset` reads the file from disk, parses,
    /// and produces a `CompiledAssetBlob` of kind Shader with the
    /// encoded payload. End-to-end happy path.
    #[test]
    fn compile_shader_asset_round_trips_through_disk() {
        let root = unique_test_root("shader_compile");
        let source_path = root.join("test.hshd");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            &source_path,
            "#shader vertex\n#version 330 core\nvoid main() {}\n\
             #shader fragment\n#version 330 core\nvoid main() {}\n",
        )
        .unwrap();

        let blob = compile_shader_asset(&source_path, "shaders/test").unwrap();
        assert_eq!(blob.metadata.kind, CompiledAssetKind::Shader);
        assert_eq!(blob.metadata.logical_name, "shaders/test");

        // Decode the payload back and verify content.
        let decoded = crate::assets::decode_compiled_shader_payload(&blob.payload).unwrap();
        assert_eq!(decoded.name, "shaders/test");
        assert!(decoded.vertex_source.contains("void main()"));
        assert!(decoded.fragment_source.contains("void main()"));

        fs::remove_dir_all(root).ok();
    }
}
