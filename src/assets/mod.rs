pub mod compiled;
pub mod compiler;
pub mod handle;
pub mod mesh;
pub mod pool;
pub mod runtime_loader;
pub mod store;
pub mod texture;
pub mod types;

pub use compiled::{
    decode_compiled_asset, encode_compiled_asset, CompiledAssetBlob, CompiledAssetDependency,
    CompiledAssetError, CompiledAssetKind, CompiledAssetMetadata, COMPILED_ASSET_MAGIC,
    COMPILED_ASSET_VERSION,
};
pub use compiler::{
    compile_asset_from_source, compile_material_asset, compile_mesh_asset, compile_texture_asset,
    parse_compiled_material_source, resolve_source_asset_path, source_asset_candidates,
    source_asset_is_stale, write_compiled_asset, AssetCompilerError,
};
pub use handle::Handle;
pub use mesh::{MeshData, ObjError, Vertex};
pub use pool::AssetPool;
pub use runtime_loader::{
    compiled_asset_path_for_logical_name, decode_compiled_material_payload,
    decode_compiled_mesh_payload, decode_compiled_texture_payload,
    encode_compiled_material_payload, encode_compiled_mesh_payload,
    encode_compiled_texture_payload, CompiledAssetRuntimeError, CompiledAssetRuntimeLoader,
    CompiledMaterialPayload,
};
pub use store::AssetStore;
pub use texture::{decode_bmp, decode_tga, load_bmp, load_texture, load_tga, TexError};
pub use types::{
    GpuMesh, GpuTexture, Material, MaterialHandle, MeshHandle, Shader, ShaderHandle, Texture,
    TextureFormat, TextureHandle,
};
