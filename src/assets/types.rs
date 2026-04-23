use super::mesh::MeshData;
use crate::assets::Handle;

pub type MeshHandle = Handle<MeshData>;
pub type TextureHandle = Handle<Texture>;
pub type ShaderHandle = Handle<Shader>;
pub type MaterialHandle = Handle<Material>;

/// GPU-side mesh handle returned by the OpenGL renderer after uploading a
/// [MeshData](super::MeshData).
///
/// Stores the OpenGL buffer IDs needed to draw and clean up the mesh.
/// Created via [OpenGLRenderer::upload_mesh()](crate::renderer::opengl::OpenGLRenderer::upload_mesh)
/// and freed via [OpenGLRenderer::free_mesh()](crate::renderer::opengl::OpenGLRenderer::free_mesh).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuMesh {
    /// Vertex Array Object encapsulates the vertex attribute state.
    pub vao: u32,
    /// Vertex Buffer Object.
    pub vbo: u32,
    /// Element Buffer Object (index buffer).
    pub ebo: u32,
    /// Number of indices to pass to glDrawElements.
    pub index_count: u32,
}

// ---------------------------------------------------------------------------
// Texture
// ---------------------------------------------------------------------------

/// Pixel format descriptor for a texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFormat {
    Rgba8,
    Rgb8,
    R8,
}

/// 2-D texture asset carrying raw pixel data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
    /// Raw pixel bytes in the layout described by `format`.
    pub data: Vec<u8>,
}

/// GPU-side texture handle returned by the OpenGL renderer after uploading a
/// [`Texture`].
///
/// Stores the OpenGL texture ID. Created via
/// [`OpenGLRenderer::upload_texture()`](crate::renderer::opengl::OpenGLRenderer::upload_texture)
/// and freed via
/// [`OpenGLRenderer::free_texture()`](crate::renderer::opengl::OpenGLRenderer::free_texture).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuTexture {
    /// OpenGL texture object name.
    pub id: u32,
    pub width: u32,
    pub height: u32,
}

// ---------------------------------------------------------------------------
// Shader
// ---------------------------------------------------------------------------

/// GLSL / SPIR-V shader source pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shader {
    pub name: String,
    pub vertex_source: String,
    pub fragment_source: String,
}

// ---------------------------------------------------------------------------
// Material
// ---------------------------------------------------------------------------

/// Surface material referencing a shader and one or more texture maps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Material {
    pub shader: ShaderHandle,
    pub albedo: TextureHandle,
    pub normal: Option<TextureHandle>,
    pub specular_gloss: Option<TextureHandle>,
    pub double_sided: bool,
    pub alpha_cutout: bool,
}

impl Material {
    pub fn new(shader: ShaderHandle, albedo: TextureHandle) -> Self {
        Self {
            shader,
            albedo,
            normal: None,
            specular_gloss: None,
            double_sided: false,
            alpha_cutout: false,
        }
    }

    pub fn with_normal(mut self, normal: TextureHandle) -> Self {
        self.normal = Some(normal);
        self
    }

    pub fn with_specular_gloss(mut self, specular_gloss: TextureHandle) -> Self {
        self.specular_gloss = Some(specular_gloss);
        self
    }

    pub fn with_double_sided(mut self, double_sided: bool) -> Self {
        self.double_sided = double_sided;
        self
    }

    pub fn with_alpha_cutout(mut self, alpha_cutout: bool) -> Self {
        self.alpha_cutout = alpha_cutout;
        self
    }
}
