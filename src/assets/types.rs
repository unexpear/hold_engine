use crate::assets::Handle;

/// GPU-side mesh handle returned by the OpenGL renderer after uploading a
/// [MeshData](super::MeshData).
///
/// Stores the OpenGL buffer IDs needed to draw and clean up the mesh.
/// Created via [OpenGLRenderer::upload_mesh()](crate::renderer::opengl::OpenGLRenderer::upload_mesh)
/// and freed via [OpenGLRenderer::free_mesh()](crate::renderer::opengl::OpenGLRenderer::free_mesh).
pub struct GpuMesh {
    /// Vertex Array Object — encapsulates the vertex attribute state.
    pub vao:         u32,
    /// Vertex Buffer Object.
    pub vbo:         u32,
    /// Element Buffer Object (index buffer).
    pub ebo:         u32,
    /// Number of indices to pass to glDrawElements.
    pub index_count: u32,
}

// ---------------------------------------------------------------------------
// Texture
// ---------------------------------------------------------------------------

/// Pixel format descriptor for a [\Texture\].
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
    /// Raw pixel bytes in the layout described by \ormat\.
    pub data:   Vec<u8>,
}

/// GPU-side texture handle returned by the OpenGL renderer after uploading a
/// [`Texture`].
///
/// Stores the OpenGL texture ID.  Created via
/// [`OpenGLRenderer::upload_texture()`](crate::renderer::opengl::OpenGLRenderer::upload_texture)
/// and freed via
/// [`OpenGLRenderer::free_texture()`](crate::renderer::opengl::OpenGLRenderer::free_texture).
pub struct GpuTexture {
    /// OpenGL texture object name.
    pub id: u32,
    pub width:  u32,
    pub height: u32,
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
