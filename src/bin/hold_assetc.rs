use std::env;
use std::path::PathBuf;
use std::process;

use hold::assets::{
    compile_material_asset, compile_mesh_asset, compile_texture_asset, write_compiled_asset,
    CompiledAssetBlob,
};

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let kind = match args.next() {
        Some(value) if value == "--help" || value == "-h" => return Err(usage()),
        Some(value) => value,
        None => return Err(usage()),
    };
    let source_path = PathBuf::from(args.next().ok_or_else(usage)?);
    let compiled_root = PathBuf::from(args.next().ok_or_else(usage)?);
    let logical_name = args.next().ok_or_else(usage)?;
    if args.next().is_some() {
        return Err(usage());
    }

    let blob = compile_by_kind(&kind, &source_path, &logical_name)?;
    let path = write_compiled_asset(&compiled_root, &blob).map_err(|err| err.to_string())?;
    println!(
        "[hold_assetc] wrote {:?} asset `{}` to {}",
        blob.metadata.kind,
        blob.metadata.logical_name,
        path.display()
    );
    Ok(())
}

fn compile_by_kind(
    kind: &str,
    source_path: &PathBuf,
    logical_name: &str,
) -> Result<CompiledAssetBlob, String> {
    match kind {
        "mesh" => compile_mesh_asset(source_path, logical_name).map_err(|err| err.to_string()),
        "texture" => {
            compile_texture_asset(source_path, logical_name).map_err(|err| err.to_string())
        }
        "material" => {
            compile_material_asset(source_path, logical_name).map_err(|err| err.to_string())
        }
        _ => Err(format!("unsupported asset kind `{kind}`\n\n{}", usage())),
    }
}

fn usage() -> String {
    "usage: hold_assetc <mesh|texture|material> <source> <compiled_root> <logical_name>\n\
\n\
material source format:\n\
  albedo <texture logical name>\n\
  normal <texture logical name|none>\n\
  specular_gloss <texture logical name|none>\n\
  double_sided <true|false>\n\
  alpha_cutout <true|false>"
        .to_string()
}
