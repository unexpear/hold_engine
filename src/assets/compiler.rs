use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::{
    compiled_asset_path_for_logical_name, encode_compiled_asset, encode_compiled_material_payload,
    encode_compiled_mesh_payload, encode_compiled_texture_payload, load_texture, CompiledAssetBlob,
    CompiledAssetDependency, CompiledAssetError, CompiledAssetKind, CompiledAssetMetadata,
    CompiledMaterialPayload, MeshData, ObjError, TexError,
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
        _ => Err(AssetCompilerError::UnsupportedKind(kind)),
    }
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

    let source_modified = fs::metadata(source_path)?.modified()?;
    let compiled_modified = fs::metadata(compiled_path)?.modified()?;
    Ok(source_modified > compiled_modified)
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
        parse_compiled_material_source, resolve_source_asset_path, source_asset_candidates,
        source_asset_is_stale,
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
}
