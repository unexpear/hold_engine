pub mod compiled;
pub mod compiler;
pub mod handle;
pub mod invalidation;
pub mod mesh;
pub mod mesh_map;
pub mod pool;
pub mod runtime_loader;
pub mod store;
pub mod texture;
pub mod types;
pub(crate) mod wire;

pub use compiled::{
    decode_compiled_asset, encode_compiled_asset, CompiledAssetBlob, CompiledAssetDependency,
    CompiledAssetError, CompiledAssetKind, CompiledAssetMetadata, COMPILED_ASSET_MAGIC,
    COMPILED_ASSET_VERSION,
};
pub use compiler::{
    compile_asset_from_source, compile_material_asset, compile_mesh_asset, compile_scene_asset,
    compile_shader_asset, compile_texture_asset, parse_compiled_material_source,
    parse_shader_source,
    resolve_source_asset_path, source_asset_candidates, source_asset_is_stale,
    write_compiled_asset, AssetCompilerError,
};
pub use handle::Handle;
pub use invalidation::{AssetInvalidated, AssetKind, InvalidationBus, SubscriberId};
pub use mesh::{MeshData, ObjError, Vertex};
pub use mesh_map::{compute_ao_per_vertex, compute_curvature_per_vertex};
pub use pool::AssetPool;
pub use runtime_loader::{
    compiled_asset_path_for_logical_name, decode_compiled_material_payload,
    decode_compiled_mesh_payload, decode_compiled_normal_map_payload, decode_compiled_scene_payload,
    decode_compiled_shader_payload, decode_compiled_texture_payload,
    encode_compiled_material_payload, encode_compiled_mesh_payload, encode_compiled_scene_payload,
    encode_compiled_shader_payload, encode_compiled_texture_payload,
    encode_compiled_texture_payload_with_convention, CompiledAssetRuntimeError,
    CompiledAssetRuntimeLoader, CompiledMaterialPayload, CompiledScenePayload, CompiledShaderPayload,
};
pub use store::AssetStore;
pub use texture::{decode_bmp, decode_tga, load_bmp, load_texture, load_tga, TexError};
pub use types::{
    ColorSpace, GpuMesh, GpuTexture, Material, MaterialHandle, MeshHandle, MeshMapAO,
    MeshMapAOHandle, MeshMapCurvature, MeshMapCurvatureHandle, MeshMapError, NormalMap,
    NormalMapConvention, NormalMapHandle, Shader, ShaderHandle, Texture, TextureFormat,
    TextureHandle, DEFAULT_METALLIC, DEFAULT_ROUGHNESS,
};
