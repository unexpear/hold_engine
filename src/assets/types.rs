use crate::assets::Handle;

/// Placeholder GPU mesh asset.
///
/// Actual backend-specific buffer handles (VAO/VBO ids for OpenGL,
/// `VkBuffer` for Vulkan, etc.) will live in `_gpu_data` until the
/// respective backend is wired up.
pub struct GpuMesh {
    pub vertex_count: u32,
    pub index_count:  u32,
    /// Raw, backend-specific GPU data.  Placeholder until backends populate it.
    pub _gpu_data:    Vec<u8>,
}

// ---------------------------------------------------------------------------
// Texture
// ---------------------------------------------------------------------------

/// Pixel format descriptor for a [`Texture`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFormat {
    Rgba8,
    Rgb8,
    R8,
}

/// 2-D texture asset carrying raw pixel data.
pub struct Texture {
    pub width:  u32,
    pub height: u32,
    pub format: TextureFormat,
    /// Raw pixel bytes in the layout described by `format`.
    pub data:   Vec<u8>,
}

// ---------------------------------------------------------------------------
// Shader
// ---------------------------------------------------------------------------

/// GLSL / SPIR-V shader source pair.
pub struct Shader {
    pub name:            String,
    pub vertex_source:   String,
    pub fragment_source: String,
}

// ---------------------------------------------------------------------------
// Material
// ---------------------------------------------------------------------------

/// Surface material referencing a shader and one or more texture maps.
pub struct Material {
    pub shader: Handle<Shader>,
    pub albedo: Handle<Texture>,
    // Additional texture maps (normal, roughness, …) will be added here.
}
