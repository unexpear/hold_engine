use super::mesh::MeshData;
use crate::assets::Handle;

pub type MeshHandle = Handle<MeshData>;
pub type TextureHandle = Handle<Texture>;
pub type NormalMapHandle = Handle<NormalMap>;
pub type ShaderHandle = Handle<Shader>;
pub type MaterialHandle = Handle<Material>;
pub type MeshMapAOHandle = Handle<MeshMapAO>;
pub type MeshMapCurvatureHandle = Handle<MeshMapCurvature>;

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

/// Colour-space tag carried on every [`Texture`] asset.
///
/// PBR shading runs in linear space; sRGB-encoded inputs (most authored
/// colour textures) must be decoded to linear before they are sampled.
/// Tagging the asset rather than guessing from filename, format, or
/// shader binding ensures the decode happens deterministically at
/// upload time and never gets accidentally double-applied.
///
/// See ADR 0002 Q3 for the discipline. The rule of thumb:
/// - `Srgb`: base colour, albedo, paintings, UI sprites, anything
///   authored visually.
/// - `Linear`: roughness, metallic, height, AO, masks, look-up tables,
///   anything sampled as numerical data rather than displayed colour.
///
/// [`crate::assets::NormalMap`] does not carry this tag — normal maps
/// are unconditionally linear at the type level (ADR 0002 Q4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    /// Texture data is sRGB-encoded; the GPU upload path applies the
    /// sRGB→linear decode at sample time via an internal format like
    /// `GL_SRGB8_ALPHA8`. Default for new [`Texture`] assets because the
    /// most common content (authored colour) is sRGB.
    Srgb,
    /// Texture data is already in linear space. The GPU upload path uses
    /// a non-sRGB internal format (`GL_RGBA8`) so values pass through
    /// unmodified. Required for any data-bearing map (roughness,
    /// metallic, masks, ...).
    Linear,
}

impl Default for ColorSpace {
    fn default() -> Self {
        Self::Srgb
    }
}

/// 2-D texture asset carrying raw pixel data plus a colour-space tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
    /// Whether `data` is sRGB-encoded or already linear. See
    /// [`ColorSpace`] and ADR 0002 Q3 for the decode rules.
    pub color_space: ColorSpace,
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
// NormalMap
// ---------------------------------------------------------------------------

/// Tangent-space convention for a normal map's green (Y) channel.
///
/// Different tools export with different sign conventions:
/// - **OpenGL** convention: green channel up. The decoded normal points in
///   the +Y direction in tangent space when green is bright. This is the
///   engine's canonical convention (see ADR 0002 Q4).
/// - **DirectX** convention: green channel down. Used by 3ds Max defaults,
///   Substance Painter's "DirectX" export preset, and some game engines.
///
/// Imported assets that use the DirectX convention are flipped to OpenGL at
/// upload time. The convention is stored on the asset (rather than implicitly
/// assumed) so the editor inspector can surface it loudly — confused
/// conventions are the most common cause of "lighting looks wrong" bugs in
/// normal-mapped content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalMapConvention {
    /// Green up (+Y). The engine's canonical convention. Required at the
    /// shader sampling boundary; non-canonical inputs are converted at
    /// import time.
    OpenGl,
    /// Green down (-Y). Flipped to [`NormalMapConvention::OpenGl`] at
    /// upload time before the GPU sees the data.
    DirectX,
}

impl Default for NormalMapConvention {
    fn default() -> Self {
        Self::OpenGl
    }
}

/// Tangent-space normal map asset.
///
/// Distinct from [`Texture`] at the type level: the [`Material`] type accepts
/// only a [`NormalMapHandle`] in its normal slot, so it is impossible to
/// accidentally bind a colour texture as a normal map (or vice versa). This is
/// the typed-pin discipline from ADR 0002 Q4 applied at the data-model layer.
///
/// The pixel layout uses three or four channels:
/// - RGB encodes `(R, G, B) = ((nx + 1) * 0.5, (ny + 1) * 0.5, (nz + 1) * 0.5)`
///   in linear space (no sRGB encoding curve).
/// - The optional alpha channel is unused by the shader sampler and is
///   preserved only for round-trip fidelity.
///
/// Pixels are always stored in the engine's canonical OpenGL Y+ convention; if
/// the source asset was DirectX, the upload path applies the green-channel flip
/// before the bytes reach the GPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalMap {
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
    /// The convention the source data was authored in. The engine keeps this
    /// for round-trip fidelity and inspector display; the on-GPU bytes are
    /// always normalised to OpenGL Y+.
    pub source_convention: NormalMapConvention,
    /// Raw pixel bytes in the layout described by `format`, already
    /// normalised to OpenGL Y+.
    pub data: Vec<u8>,
}

impl NormalMap {
    /// Construct a normal map directly from already-normalised OpenGL Y+ data.
    /// Use [`NormalMap::from_directx`] for source data with the DirectX
    /// convention; the green channel is flipped during construction so the
    /// stored bytes are always in the canonical convention.
    pub fn new_opengl(width: u32, height: u32, format: TextureFormat, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            format,
            source_convention: NormalMapConvention::OpenGl,
            data,
        }
    }

    /// Construct a normal map from DirectX-convention data (green down). The
    /// green channel is flipped during construction so the stored bytes are
    /// in the engine's canonical OpenGL Y+ convention. The original
    /// convention is recorded in `source_convention` for inspector display.
    pub fn from_directx(width: u32, height: u32, format: TextureFormat, mut data: Vec<u8>) -> Self {
        flip_green_channel_in_place(&mut data, format);
        Self {
            width,
            height,
            format,
            source_convention: NormalMapConvention::DirectX,
            data,
        }
    }
}

/// Flip the green channel (`g' = 255 - g`) for every pixel in `data`. Used to
/// convert DirectX-convention normal maps to the engine's OpenGL Y+ canonical
/// convention at construction time, so the GPU never sees the wrong sign.
///
/// The R8 format has no green channel; this is a no-op for it. R8 normal maps
/// are unusual but legal (they can encode 1-channel height-derived normals at
/// runtime), so the function tolerates them rather than panicking.
fn flip_green_channel_in_place(data: &mut [u8], format: TextureFormat) {
    let (bytes_per_pixel, green_offset) = match format {
        TextureFormat::Rgba8 => (4, 1),
        TextureFormat::Rgb8 => (3, 1),
        TextureFormat::R8 => return,
    };
    let mut i = green_offset;
    while i < data.len() {
        data[i] = 255 - data[i];
        i += bytes_per_pixel;
    }
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

/// Surface material referencing a shader and one or more typed asset maps,
/// plus PBR factors that drive the Disney metallic-roughness shading model.
///
/// The `normal` slot is typed as [`NormalMapHandle`], not [`TextureHandle`],
/// per ADR 0002 Q4: a colour texture cannot be bound here at the type level,
/// and a normal map cannot be bound to the colour slots.
///
/// `metallic` and `roughness` carry the two PBR knobs every modern shading
/// model takes as input. v1 stores them as scalar factors only; future work
/// adds a `metallic_roughness` texture map (glTF 2.0 convention: metallic
/// in B, roughness in G of a single texture). See ADR 0002 Q3 for the full
/// shading-model rationale.
#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub shader: ShaderHandle,
    pub albedo: TextureHandle,
    pub normal: Option<NormalMapHandle>,
    pub specular_gloss: Option<TextureHandle>,
    pub double_sided: bool,
    pub alpha_cutout: bool,
    /// Metallic factor in `[0, 1]`. `0` = dielectric (plastic, wood,
    /// fabric), `1` = pure metal (gold, copper, iron). Default `0.0`
    /// because most authored game content is dielectric and the right
    /// failure mode for "I forgot to set it" is "looks like plastic"
    /// rather than "looks like blown-out chrome."
    pub metallic: f32,
    /// Surface roughness factor in `[0, 1]`. `0` = perfect mirror,
    /// `1` = fully diffuse. Default `0.5` is a neutral mid-rough
    /// surface that reads cleanly under typical lighting without
    /// being either glassy or chalky.
    pub roughness: f32,
}

/// Default metallic factor for materials that don't set one explicitly.
/// Dielectric (0.0) — see the [`Material::metallic`] field doc.
pub const DEFAULT_METALLIC: f32 = 0.0;

/// Default roughness factor for materials that don't set one explicitly.
/// Mid-rough (0.5) — see the [`Material::roughness`] field doc.
pub const DEFAULT_ROUGHNESS: f32 = 0.5;

impl Material {
    pub fn new(shader: ShaderHandle, albedo: TextureHandle) -> Self {
        Self {
            shader,
            albedo,
            normal: None,
            specular_gloss: None,
            double_sided: false,
            alpha_cutout: false,
            metallic: DEFAULT_METALLIC,
            roughness: DEFAULT_ROUGHNESS,
        }
    }

    /// Attach a normal map to this material. Takes a [`NormalMapHandle`]
    /// rather than a [`TextureHandle`] — see the type-level rationale on
    /// [`Material`] and ADR 0002 Q4.
    pub fn with_normal_map(mut self, normal: NormalMapHandle) -> Self {
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

    /// Set the metallic factor. Clamped to `[0, 1]` because values
    /// outside the range produce non-physical lighting and the Disney
    /// BRDF math assumes the input is bounded.
    pub fn with_metallic(mut self, metallic: f32) -> Self {
        self.metallic = metallic.clamp(0.0, 1.0);
        self
    }

    /// Set the roughness factor. Clamped to `[0, 1]` for the same
    /// reason as [`Self::with_metallic`].
    pub fn with_roughness(mut self, roughness: f32) -> Self {
        self.roughness = roughness.clamp(0.0, 1.0);
        self
    }
}

// ---------------------------------------------------------------------------
// Mesh maps (procedural-mask substrate)
// ---------------------------------------------------------------------------

/// Per-vertex ambient occlusion in `[0, 1]`. `1.0` = fully visible to the
/// hemisphere, `0.0` = fully occluded.
///
/// Mesh maps are the *substrate* layered-material masks read from
/// (Painter-style; see ADR 0002 Q5). v1 stores them per-vertex; the GPU
/// rasteriser interpolates between vertices the same way it interpolates
/// vertex colours, which is visually equivalent to texel-domain baking
/// for low-poly content at much less import-time cost. The data structure
/// is positioned so a future texture-domain bake (M6+) extends rather
/// than replaces it.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshMapAO {
    /// One AO value per vertex of the source mesh, ordered to match
    /// [`MeshData::vertices`]. Outside `[0, 1]` is rejected by
    /// `MeshMapAO::new` rather than silently clamped, so callers that
    /// produce out-of-range values learn about it loudly.
    pub values: Vec<f32>,
}

/// Per-vertex curvature scalar. The convention here is a roughness-style
/// `[0, 1]` measurement: `0.0` = perfectly flat surface (vertex normal
/// agrees with every adjacent face normal), `1.0` = sharp ridge or edge
/// (vertex normal disagrees maximally with neighbours).
///
/// This is a *proxy* for true differential curvature; it tracks how much
/// the surface bends locally without the full discrete-mean-curvature
/// machinery. Sufficient for procedural masks (edge-wear, dirt-in-cavities
/// patterns the layered material system uses, ADR 0002 Q5+Q6); a future
/// commit can swap in a more rigorous estimator without changing the
/// asset shape.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshMapCurvature {
    /// One curvature value per vertex of the source mesh, ordered to
    /// match [`MeshData::vertices`]. Bounded to `[0, 1]` by construction.
    pub values: Vec<f32>,
}

/// Error returned when a mesh-map constructor receives data that
/// violates the asset's invariants (length mismatch with the source
/// mesh, NaN values, out-of-range values).
#[derive(Debug, Clone, PartialEq)]
pub enum MeshMapError {
    /// `values.len()` did not match the source mesh's vertex count.
    LengthMismatch { expected: usize, found: usize },
    /// At least one value was outside `[0, 1]` or was non-finite.
    OutOfRange { index: usize, value: f32 },
}

impl std::fmt::Display for MeshMapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LengthMismatch { expected, found } => write!(
                f,
                "mesh map length {found} does not match mesh vertex count {expected}"
            ),
            Self::OutOfRange { index, value } => write!(
                f,
                "mesh map value at index {index} is {value}, expected finite in [0, 1]"
            ),
        }
    }
}

impl std::error::Error for MeshMapError {}

impl MeshMapAO {
    /// Construct from a `Vec<f32>`, validating that the length matches the
    /// source mesh's vertex count and every value is finite and inside
    /// `[0, 1]`. The validation pass is the price the type system charges
    /// for the rest of the engine being able to assume the invariants
    /// hold at sample sites.
    pub fn new(values: Vec<f32>, mesh_vertex_count: usize) -> Result<Self, MeshMapError> {
        validate_unit_range(&values, mesh_vertex_count)?;
        Ok(Self { values })
    }
}

impl MeshMapCurvature {
    /// Construct from a `Vec<f32>` with the same validation as
    /// [`MeshMapAO::new`]. See that method's doc for the rationale.
    pub fn new(values: Vec<f32>, mesh_vertex_count: usize) -> Result<Self, MeshMapError> {
        validate_unit_range(&values, mesh_vertex_count)?;
        Ok(Self { values })
    }
}

fn validate_unit_range(values: &[f32], expected_len: usize) -> Result<(), MeshMapError> {
    if values.len() != expected_len {
        return Err(MeshMapError::LengthMismatch {
            expected: expected_len,
            found: values.len(),
        });
    }
    for (i, &v) in values.iter().enumerate() {
        if !v.is_finite() || !(0.0..=1.0).contains(&v) {
            return Err(MeshMapError::OutOfRange { index: i, value: v });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetStore;

    /// The default convention for a freshly constructed [`NormalMap`] must be
    /// OpenGL Y+ — the engine's canonical convention. Anything else would
    /// break the type-level guarantee that the GPU only sees normalised
    /// data. See ADR 0002 Q4.
    #[test]
    fn normal_map_default_convention_is_opengl() {
        let nm = NormalMap::new_opengl(2, 2, TextureFormat::Rgb8, vec![128; 12]);
        assert_eq!(nm.source_convention, NormalMapConvention::OpenGl);
        assert_eq!(NormalMapConvention::default(), NormalMapConvention::OpenGl);
    }

    /// Constructing from a DirectX-convention buffer must flip the green
    /// channel during construction, so the stored bytes are already in the
    /// canonical convention. The original convention is preserved on the
    /// asset for inspector display only.
    #[test]
    fn normal_map_from_directx_flips_green_channel() {
        // 2 RGBA pixels: red=10/g=20/b=30/a=40 and red=200/g=100/b=50/a=255.
        let src = vec![10, 20, 30, 40, 200, 100, 50, 255];
        let nm = NormalMap::from_directx(2, 1, TextureFormat::Rgba8, src);
        // Green channel must now be 255 - original.
        assert_eq!(nm.data[1], 255 - 20);
        assert_eq!(nm.data[5], 255 - 100);
        // Other channels unchanged.
        assert_eq!(nm.data[0], 10);
        assert_eq!(nm.data[2], 30);
        assert_eq!(nm.data[3], 40);
        assert_eq!(nm.data[4], 200);
        assert_eq!(nm.data[6], 50);
        assert_eq!(nm.data[7], 255);
        // Convention metadata records the source for inspector display.
        assert_eq!(nm.source_convention, NormalMapConvention::DirectX);
    }

    /// R8 normal maps have no green channel; the flip is a no-op rather than
    /// a panic. R8 inputs are unusual but legal (e.g. height-derived normal
    /// maps before reconstruction), so the path must tolerate them.
    #[test]
    fn normal_map_from_directx_r8_is_noop() {
        let src = vec![10, 20, 30, 40];
        let original = src.clone();
        let nm = NormalMap::from_directx(2, 2, TextureFormat::R8, src);
        assert_eq!(nm.data, original);
    }

    /// `ColorSpace::default()` is sRGB. Most authored content arriving
    /// through asset import is sRGB, so the default deliberately favours
    /// the common case rather than the safer linear default — explicit
    /// `ColorSpace::Linear` tagging is required for data-bearing maps.
    /// See ADR 0002 Q3.
    #[test]
    fn color_space_default_is_srgb() {
        assert_eq!(ColorSpace::default(), ColorSpace::Srgb);
    }

    /// `ColorSpace::Srgb` and `ColorSpace::Linear` compare unequal — the
    /// type system must keep them distinct so the GPU upload path
    /// dispatches correctly. Tests the simplest possible safety net.
    #[test]
    fn color_space_variants_are_distinct() {
        assert_ne!(ColorSpace::Srgb, ColorSpace::Linear);
    }

    /// `MeshMapAO::new` accepts in-range values matching the mesh's
    /// vertex count. This is the happy path; the rest of the constructor
    /// tests lock down the failure modes.
    #[test]
    fn mesh_map_ao_accepts_valid_values() {
        let map = MeshMapAO::new(vec![0.0, 0.5, 1.0], 3).unwrap();
        assert_eq!(map.values, vec![0.0, 0.5, 1.0]);
    }

    /// Length mismatch surfaces as a typed `LengthMismatch` error rather
    /// than silently truncating or padding. Procedural-mask sample sites
    /// rely on the length matching the mesh's vertex count exactly.
    #[test]
    fn mesh_map_ao_rejects_length_mismatch() {
        match MeshMapAO::new(vec![0.5, 0.5], 3) {
            Err(MeshMapError::LengthMismatch { expected, found }) => {
                assert_eq!(expected, 3);
                assert_eq!(found, 2);
            }
            other => panic!("expected LengthMismatch, got {:?}", other),
        }
    }

    /// Out-of-range values surface with the offending index. NaN, ±∞,
    /// values < 0 and values > 1 are all rejected. Sample sites can
    /// then assume `[0, 1]` without per-sample clamping.
    #[test]
    fn mesh_map_ao_rejects_out_of_range_values() {
        let cases: Vec<(Vec<f32>, usize)> = vec![
            (vec![0.5, -0.1, 0.5], 1),
            (vec![0.5, 1.5, 0.5], 1),
            (vec![f32::NAN, 0.5, 0.5], 0),
            (vec![0.5, 0.5, f32::INFINITY], 2),
        ];
        for (values, bad_index) in cases {
            let display = values.clone();
            match MeshMapAO::new(values, 3) {
                Err(MeshMapError::OutOfRange { index, .. }) => {
                    assert_eq!(index, bad_index, "for input {:?}", display);
                }
                other => panic!("expected OutOfRange, got {:?} for {:?}", other, display),
            }
        }
    }

    /// `MeshMapCurvature::new` shares the validator with `MeshMapAO::new`.
    /// One smoke test confirms the type uses the same gate.
    #[test]
    fn mesh_map_curvature_shares_validator() {
        assert!(MeshMapCurvature::new(vec![0.0, 0.7], 2).is_ok());
        assert!(MeshMapCurvature::new(vec![0.0, 1.5], 2).is_err());
    }

    /// Verifies the typed-pin discipline at the asset-store layer: the same
    /// logical name can produce both a [`Texture`] and a [`NormalMap`] handle,
    /// and the two handles refer to *different* assets in the store. A handle
    /// of one type cannot be silently substituted for the other — the
    /// `AssetStore::get` call would fall through to the wrong pool. See
    /// ADR 0002 Q4.
    #[test]
    fn texture_and_normal_map_handles_are_distinct_at_the_store() {
        let mut assets = AssetStore::new();
        let tex = assets.insert(Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgb8,
            color_space: ColorSpace::Srgb,
            data: vec![255, 128, 64],
        });
        let nm = assets.insert(NormalMap::new_opengl(
            1,
            1,
            TextureFormat::Rgb8,
            vec![128, 128, 255],
        ));

        // Handles are typed; the store keeps them in distinct pools.
        // The fact that the call below compiles is itself part of the test:
        // attempting to call `assets.get::<Texture>(nm)` would not type-check.
        assert!(assets.get(tex).is_some());
        assert!(assets.get(nm).is_some());

        let tex_view = assets.get(tex).unwrap();
        assert_eq!(tex_view.data, vec![255, 128, 64]);
        let nm_view = assets.get(nm).unwrap();
        assert_eq!(nm_view.data, vec![128, 128, 255]);
        assert_eq!(nm_view.source_convention, NormalMapConvention::OpenGl);
    }
}
