pub mod frame;
pub mod frame_graph;
pub mod opengl;
pub mod opengl_shaders;
pub mod software;
pub mod tonemap;
pub mod vulkan;

pub use frame::{
    DrawItem, FogState, LightItem, ParticleBatch, ParticleBillboard, RenderFrame, RenderView,
    RenderViewport, TerrainBatch, TerrainPatchItem, VegetationBatch, WaterBatch,
};
pub use frame_graph::{
    downsample_rgba8, AccessKind, BufferDesc, BufferUsage, ClearAndViewportPass, ClearPass,
    ClosurePass, DebugLinesPass, DrawTexturedNormalMappedPass, DrawTexturedNormalMappedPassParams,
    DrawTexturedPass, DrawTexturedPassParams, FrameGraph, FrameGraphBuilder, FrameGraphError,
    FunctionPass, NoopPass, Pass, PassBuilder, PassContext, PassThumbnail, ResourceDesc,
    ResourceId, ResourceKind, ScenePass, ScreenQuadsPass, TextureDesc, TextureUsage,
    DEFAULT_THUMBNAIL_SIZE,
};
pub use opengl::OpenGLRenderer;
pub use software::SoftwareRenderer;
pub use tonemap::aces_tonemap_rgb;
pub use vulkan::VulkanRenderer;

use crate::assets::AssetStore;
use crate::math::{Mat4, Vec3};

// ── Color ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const BLACK: Self = Self::new(0.0, 0.0, 0.0, 1.0);
    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0, 1.0);
    pub const RED: Self = Self::new(1.0, 0.0, 0.0, 1.0);
    pub const GREEN: Self = Self::new(0.0, 1.0, 0.0, 1.0);
    pub const BLUE: Self = Self::new(0.0, 0.0, 1.0, 1.0);
    pub const YELLOW: Self = Self::new(1.0, 1.0, 0.0, 1.0);
    pub const CYAN: Self = Self::new(0.0, 1.0, 1.0, 1.0);
    pub const MAGENTA: Self = Self::new(1.0, 0.0, 1.0, 1.0);
    pub const TRANSPARENT: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    #[inline]
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
    #[inline]
    pub fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// Pack to 0x00RRGGBB (format expected by GDI DIB with BI_RGB, 32bpp, little-endian).
    #[inline]
    pub fn to_bgra_u32(self) -> u32 {
        let r = (self.r.clamp(0.0, 1.0) * 255.0) as u32;
        let g = (self.g.clamp(0.0, 1.0) * 255.0) as u32;
        let b = (self.b.clamp(0.0, 1.0) * 255.0) as u32;
        (r << 16) | (g << 8) | b
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }
}

// ── Renderer trait ────────────────────────────────────────────────────────────

/// All rendering backends implement this trait.
pub trait Renderer {
    fn width(&self) -> u32;
    fn height(&self) -> u32;

    /// Called when the window is resized.
    fn resize(&mut self, width: u32, height: u32);

    /// Begin a new frame (acquire swap-chain image, etc.).
    fn begin_frame(&mut self);

    /// Clear the colour buffer (and depth for software/3D).
    fn clear(&mut self, color: Color);

    /// Present the rendered frame to the screen.
    fn end_frame(&mut self);

    /// Draw a batch of world-space debug line segments.
    ///
    /// `vp` is the combined view-projection matrix.  Each backend projects
    /// endpoints through `vp` in its own way — software uses its CPU
    /// rasterizer, OpenGL issues a `GL_LINES` draw call.
    ///
    /// This is a no-op for backends that do not support debug visualization.
    fn render_debug_lines(&mut self, lines: &[DebugLine], vp: &Mat4) {
        let _ = (lines, vp);
    }

    /// Draw a batch of screen-space solid rectangles.
    ///
    /// Coordinates use a top-left origin in pixel space. This is intended for
    /// native editor overlays such as hierarchy and inspector panels.
    fn render_screen_quads(&mut self, quads: &[ScreenQuad]) {
        let _ = quads;
    }

    /// Submit a backend-neutral frame description within the current frame.
    ///
    /// Callers are expected to use this between [`Renderer::begin_frame`] and
    /// [`Renderer::end_frame`]. The default path preserves the current
    /// immediate-mode contract: clear, draw debug lines through the primary
    /// view, then draw screen quads. Backends can override this to consume
    /// mesh/material/terrain batches directly as the runtime slice renderer
    /// grows.
    fn submit_frame(&mut self, assets: &AssetStore, frame: &RenderFrame) {
        let _ = assets;
        let clear_color = frame
            .primary_view()
            .map(|view| view.clear_color)
            .unwrap_or(Color::BLACK);
        self.clear(clear_color);
        if let Some(view) = frame.primary_view() {
            let vp = view.projection * view.view;
            self.render_debug_lines(&frame.debug_lines, &vp);
        }
        self.render_screen_quads(&frame.screen_quads);
    }

    /// Render a frame through the M9 frame graph and capture a
    /// [`PassThumbnail`] per pass for the editor's frame inspector
    /// (ADR 0002 Q8, ADR 0003 Pillar 4).
    ///
    /// Backends that don't implement the frame graph inherit this
    /// default, which just falls back to [`Self::submit_frame`] and
    /// returns an empty capture. Callers can distinguish "capture not
    /// available" from "capture produced zero passes" by inspecting
    /// the returned vector's length against the graph's expected pass
    /// count (four, currently — clear/scene/debug-lines/screen-quads).
    fn submit_frame_capturing(
        &mut self,
        assets: &AssetStore,
        frame: &RenderFrame,
        thumb_w: u32,
        thumb_h: u32,
    ) -> Vec<PassThumbnail> {
        let _ = (thumb_w, thumb_h);
        self.submit_frame(assets, frame);
        Vec::new()
    }
}

// ── Debug draw primitive ──────────────────────────────────────────────────────

/// A single 3-D line segment for editor debug visualization.
///
/// Passed to [`Renderer::render_debug_lines`] in batches.  Both endpoints are
/// in world space; the renderer projects them through the supplied VP matrix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DebugLine {
    pub start: Vec3,
    pub end: Vec3,
    pub color: Color,
}

impl DebugLine {
    #[inline]
    pub fn new(start: Vec3, end: Vec3, color: Color) -> Self {
        Self { start, end, color }
    }
}

/// A single screen-space filled rectangle.
///
/// Coordinates are expressed in pixels using a top-left origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenQuad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: Color,
}

impl ScreenQuad {
    #[inline]
    pub fn new(x: f32, y: f32, w: f32, h: f32, color: Color) -> Self {
        Self { x, y, w, h, color }
    }
}

// ── Backend selector ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Software,
    OpenGL,
    Vulkan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendMaturity {
    Ready,
    Experimental,
    FoundationOnly,
}

impl std::fmt::Display for BackendMaturity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ready => f.write_str("ready"),
            Self::Experimental => f.write_str("experimental"),
            Self::FoundationOnly => f.write_str("foundation-only"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendCapabilities {
    pub clear_and_present: bool,
    pub resize: bool,
    pub debug_lines: bool,
    pub screen_quads: bool,
    pub mesh_upload: bool,
    pub textured_mesh: bool,
    pub native_editor_shell_ready: bool,
    pub maturity: BackendMaturity,
    pub summary: &'static str,
    pub gaps: &'static str,
}

impl BackendCapabilities {
    pub const fn editor_overlay_ready(self) -> bool {
        self.debug_lines && self.screen_quads
    }
}

impl BackendKind {
    pub const ALL: [Self; 3] = [Self::Software, Self::OpenGL, Self::Vulkan];

    pub const fn cli_name(self) -> &'static str {
        match self {
            Self::Software => "software",
            Self::OpenGL => "opengl",
            Self::Vulkan => "vulkan",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Software => "Software",
            Self::OpenGL => "OpenGL",
            Self::Vulkan => "Vulkan",
        }
    }

    pub const fn capabilities(self) -> BackendCapabilities {
        match self {
            Self::Software => BackendCapabilities {
                clear_and_present: true,
                resize: true,
                debug_lines: true,
                screen_quads: true,
                mesh_upload: false,
                textured_mesh: false,
                native_editor_shell_ready: true,
                maturity: BackendMaturity::Ready,
                summary: "Reference backend for the native editor shell and debug tooling.",
                gaps: "No GPU mesh upload or textured mesh path yet.",
            },
            Self::OpenGL => BackendCapabilities {
                clear_and_present: true,
                resize: true,
                debug_lines: true,
                screen_quads: true,
                mesh_upload: true,
                textured_mesh: true,
                native_editor_shell_ready: true,
                maturity: BackendMaturity::Experimental,
                summary: "GPU path with editor overlays plus mesh and texture upload helpers.",
                gaps: "Needs broader parity validation and runtime hardening across driver setups.",
            },
            Self::Vulkan => BackendCapabilities {
                clear_and_present: true,
                resize: true,
                debug_lines: true,
                screen_quads: true,
                mesh_upload: false,
                textured_mesh: false,
                native_editor_shell_ready: true,
                maturity: BackendMaturity::Experimental,
                summary: "Hybrid editor path: Vulkan swapchain/present with CPU-rasterized debug and UI overlays.",
                gaps: "Still missing native Vulkan mesh upload, textured mesh rendering, and a pure-GPU overlay path.",
            },
        }
    }
}

impl std::fmt::Display for BackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.cli_name())
    }
}

#[cfg(test)]
mod tests {
    use super::{BackendKind, BackendMaturity};

    #[test]
    fn backend_catalog_stays_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for backend in BackendKind::ALL {
            assert!(seen.insert(backend.cli_name()));
        }
        assert_eq!(seen.len(), BackendKind::ALL.len());
    }

    #[test]
    fn editor_shell_ready_backends_have_overlay_support() {
        for backend in BackendKind::ALL {
            let capabilities = backend.capabilities();
            if capabilities.native_editor_shell_ready {
                assert!(capabilities.editor_overlay_ready());
            }
        }
    }

    #[test]
    fn vulkan_is_editor_ready_but_still_missing_mesh_features() {
        let capabilities = BackendKind::Vulkan.capabilities();
        assert_eq!(capabilities.maturity, BackendMaturity::Experimental);
        assert!(capabilities.native_editor_shell_ready);
        assert!(capabilities.debug_lines);
        assert!(capabilities.screen_quads);
        assert!(!capabilities.mesh_upload);
        assert!(!capabilities.textured_mesh);
    }
}
