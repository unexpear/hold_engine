pub mod software;
pub mod opengl;
pub mod vulkan;

pub use software::SoftwareRenderer;
pub use opengl::OpenGLRenderer;
pub use vulkan::VulkanRenderer;

// ── Color ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const BLACK:   Self = Self::new(0.0, 0.0, 0.0, 1.0);
    pub const WHITE:   Self = Self::new(1.0, 1.0, 1.0, 1.0);
    pub const RED:     Self = Self::new(1.0, 0.0, 0.0, 1.0);
    pub const GREEN:   Self = Self::new(0.0, 1.0, 0.0, 1.0);
    pub const BLUE:    Self = Self::new(0.0, 0.0, 1.0, 1.0);
    pub const YELLOW:  Self = Self::new(1.0, 1.0, 0.0, 1.0);
    pub const CYAN:    Self = Self::new(0.0, 1.0, 1.0, 1.0);
    pub const MAGENTA: Self = Self::new(1.0, 0.0, 1.0, 1.0);
    pub const TRANSPARENT: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    #[inline] pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self { Self { r, g, b, a } }
    #[inline] pub fn rgb(r: f32, g: f32, b: f32) -> Self { Self { r, g, b, a: 1.0 } }

    /// Pack to 0x00RRGGBB (format expected by GDI DIB with BI_RGB, 32bpp, little-endian).
    #[inline] pub fn to_bgra_u32(self) -> u32 {
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
    fn width(&self)  -> u32;
    fn height(&self) -> u32;

    /// Called when the window is resized.
    fn resize(&mut self, width: u32, height: u32);

    /// Begin a new frame (acquire swap-chain image, etc.).
    fn begin_frame(&mut self);

    /// Clear the colour buffer (and depth for software/3D).
    fn clear(&mut self, color: Color);

    /// Present the rendered frame to the screen.
    fn end_frame(&mut self);
}

// ── Backend selector ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Software,
    OpenGL,
    Vulkan,
}

use crate::math::{Mat4, Vec3};

/// Immediate-mode draws the software demo already uses.
/// draw_triangle_3d* takes model-space vertices and an OpenGL-convention MVP.
/// draw_line is screen space: origin top-left, Y down, no depth test.
pub trait Draw3d: Renderer {
    fn draw_triangle_3d(&mut self, v0: Vec3, v1: Vec3, v2: Vec3, color: Color, mvp: &Mat4);

    fn draw_triangle_3d_shaded(
        &mut self,
        v0: Vec3, v1: Vec3, v2: Vec3,
        c0: Color, c1: Color, c2: Color,
        mvp: &Mat4,
    );

    fn draw_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: Color);
}
