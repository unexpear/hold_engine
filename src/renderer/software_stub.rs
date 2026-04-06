//! Non-Windows software renderer shim.
//!
//! The full Win32 blit-backed software renderer is only available on Windows.
//! This stub keeps the crate buildable and testable on other platforms.

use std::ffi::c_void;

use crate::math::{Mat4, Vec3};

use super::{Color, Renderer};

pub struct SoftwareRenderer {
    width: u32,
    height: u32,
    _window: *mut c_void,
}

impl SoftwareRenderer {
    pub fn new(hwnd: *mut c_void, width: u32, height: u32) -> Self {
        Self { width, height, _window: hwnd }
    }

    #[inline]
    pub fn draw_pixel(&mut self, _x: i32, _y: i32, _color: Color) {}

    pub fn draw_line(&mut self, _x0: i32, _y0: i32, _x1: i32, _y1: i32, _color: Color) {}

    pub fn draw_rect(&mut self, _x: i32, _y: i32, _w: i32, _h: i32, _color: Color) {}

    pub fn fill_rect(&mut self, _x: i32, _y: i32, _w: i32, _h: i32, _color: Color) {}

    pub fn fill_triangle_2d(
        &mut self,
        _v0: (f32, f32),
        _v1: (f32, f32),
        _v2: (f32, f32),
        _color: Color,
    ) {
    }

    pub fn draw_triangle_3d(
        &mut self,
        _v0: Vec3,
        _v1: Vec3,
        _v2: Vec3,
        _color: Color,
        _mvp: &Mat4,
    ) {
    }

    pub fn draw_triangle_3d_lit(
        &mut self,
        _v0: Vec3,
        _v1: Vec3,
        _v2: Vec3,
        _n0: Vec3,
        _n1: Vec3,
        _n2: Vec3,
        _base_color: Color,
        _light_dir: Vec3,
        _light_color: Color,
        _ambient: f32,
        _mvp: &Mat4,
    ) {
    }
}

impl Renderer for SoftwareRenderer {
    fn width(&self) -> u32 { self.width }
    fn height(&self) -> u32 { self.height }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    fn begin_frame(&mut self) {}
    fn clear(&mut self, _color: Color) {}
    fn end_frame(&mut self) {}
}
