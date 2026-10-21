//! CPU software renderer with a barycentric triangle rasterizer.
//! Blits each frame to the window via Win32 StretchDIBits — no GPU required.

use crate::platform::ffi::*;
use crate::math::{Mat4, Vec3, Vec4};
use super::{Color, Renderer};

pub struct SoftwareRenderer {
    hwnd:   HWND,
    hdc:    HDC,
    width:  u32,
    height: u32,
    pixels: Vec<u32>,   // BGRA, 32bpp, row-major top→bottom
    depth:  Vec<f32>,   // depth buffer (0=near, 1=far)
}

impl SoftwareRenderer {
    pub fn new(hwnd: HWND, width: u32, height: u32) -> Self {
        let hdc = unsafe { GetDC(hwnd) };
        assert!(!hdc.is_null(), "GetDC failed");
        let n = (width * height) as usize;
        Self { hwnd, hdc, width, height, pixels: vec![0; n], depth: vec![f32::INFINITY; n] }
    }

    /// Transform a clip-space Vec4 into screen-space (x, y, depth).
    #[inline]
    fn clip_to_screen(&self, clip: Vec4) -> (f32, f32, f32) {
        let inv_w = 1.0 / clip.w;
        let nx = clip.x * inv_w;
        let ny = clip.y * inv_w;
        let nz = clip.z * inv_w;
        let w = self.width  as f32;
        let h = self.height as f32;
        (
            (nx * 0.5 + 0.5) * w,
            (1.0 - (ny * 0.5 + 0.5)) * h,
            nz,
        )
    }

    // ── Direct pixel access ───────────────────────────────────────────────────

    #[inline]
    pub fn draw_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 { return; }
        self.pixels[y as usize * self.width as usize + x as usize] = color.to_bgra_u32();
    }

    // ── 2D primitives ─────────────────────────────────────────────────────────

    /// Bresenham line.
    pub fn draw_line(&mut self, mut x0: i32, mut y0: i32, x1: i32, y1: i32, color: Color) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.draw_pixel(x0, y0, color);
            if x0 == x1 && y0 == y1 { break; }
            let e2 = 2 * err;
            if e2 >= dy { err += dy; x0 += sx; }
            if e2 <= dx { err += dx; y0 += sy; }
        }
    }

    pub fn draw_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Color) {
        self.draw_line(x,     y,     x+w-1, y,     color);
        self.draw_line(x+w-1, y,     x+w-1, y+h-1, color);
        self.draw_line(x+w-1, y+h-1, x,     y+h-1, color);
        self.draw_line(x,     y+h-1, x,     y,     color);
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Color) {
        let x0 = x.max(0) as u32;
        let y0 = y.max(0) as u32;
        let x1 = (x + w).min(self.width  as i32).max(0) as u32;
        let y1 = (y + h).min(self.height as i32).max(0) as u32;
        let packed = color.to_bgra_u32();
        for row in y0..y1 {
            let base = row as usize * self.width as usize;
            for col in x0..x1 {
                self.pixels[base + col as usize] = packed;
            }
        }
    }

    // ── 2D filled triangle (screen-space, no depth) ───────────────────────────

    pub fn fill_triangle_2d(
        &mut self,
        v0: (f32, f32), v1: (f32, f32), v2: (f32, f32),
        color: Color,
    ) {
        let min_x = v0.0.min(v1.0).min(v2.0).max(0.0) as i32;
        let max_x = (v0.0.max(v1.0).max(v2.0) as i32 + 1).min(self.width as i32 - 1);
        let min_y = v0.1.min(v1.1).min(v2.1).max(0.0) as i32;
        let max_y = (v0.1.max(v1.1).max(v2.1) as i32 + 1).min(self.height as i32 - 1);
        let packed = color.to_bgra_u32();
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let p = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = edge(v1, v2, p);
                let w1 = edge(v2, v0, p);
                let w2 = edge(v0, v1, p);
                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    self.pixels[y as usize * self.width as usize + x as usize] = packed;
                }
            }
        }
    }

    // ── 3D filled triangle (with depth buffer) ────────────────────────────────

    /// Transform `v0/v1/v2` by `mvp`, rasterize with depth test.
    /// Vertices are in model space; mvp = projection * view * model.
    pub fn draw_triangle_3d(
        &mut self,
        v0: Vec3, v1: Vec3, v2: Vec3,
        color: Color,
        mvp: &Mat4,
    ) {
        let c0 = mvp.transform_point(v0);
        let c1 = mvp.transform_point(v1);
        let c2 = mvp.transform_point(v2);

        // Clip triangles where any vertex is behind the near plane.
        if c0.w <= 0.0 || c1.w <= 0.0 || c2.w <= 0.0 { return; }

        // Perspective divide → NDC (-1..1 in X/Y, -1..1 in Z for OpenGL convention).
        let n0 = ndc(c0);
        let n1 = ndc(c1);
        let n2 = ndc(c2);

        // Coarse frustum cull (all three vertices outside the same plane).
        if n0.x < -1.0 && n1.x < -1.0 && n2.x < -1.0 { return; }
        if n0.x >  1.0 && n1.x >  1.0 && n2.x >  1.0 { return; }
        if n0.y < -1.0 && n1.y < -1.0 && n2.y < -1.0 { return; }
        if n0.y >  1.0 && n1.y >  1.0 && n2.y >  1.0 { return; }

        // Viewport transform → screen space.
        let s0 = self.clip_to_screen(c0);
        let s1 = self.clip_to_screen(c1);
        let s2 = self.clip_to_screen(c2);

        self.rasterize(s0, s1, s2, color);
    }

    /// Gouraud-shaded 3D triangle (one colour per vertex, interpolated).
    pub fn draw_triangle_3d_shaded(
        &mut self,
        v0: Vec3, v1: Vec3, v2: Vec3,
        c0: Color, c1: Color, c2: Color,
        mvp: &Mat4,
    ) {
        let clip0 = mvp.transform_point(v0);
        let clip1 = mvp.transform_point(v1);
        let clip2 = mvp.transform_point(v2);
        if clip0.w <= 0.0 || clip1.w <= 0.0 || clip2.w <= 0.0 { return; }

        let s0 = self.clip_to_screen(clip0);
        let s1 = self.clip_to_screen(clip1);
        let s2 = self.clip_to_screen(clip2);

        let min_x = s0.0.min(s1.0).min(s2.0).max(0.0) as i32;
        let max_x = (s0.0.max(s1.0).max(s2.0) as i32 + 1).min(self.width  as i32 - 1);
        let min_y = s0.1.min(s1.1).min(s2.1).max(0.0) as i32;
        let max_y = (s0.1.max(s1.1).max(s2.1) as i32 + 1).min(self.height as i32 - 1);

        let area = edge2d((s0.0, s0.1), (s1.0, s1.1), (s2.0, s2.1));
        if area <= 0.0 { return; }

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let p = (x as f32 + 0.5, y as f32 + 0.5);
                let b0 = edge2d((s1.0,s1.1), (s2.0,s2.1), p);
                let b1 = edge2d((s2.0,s2.1), (s0.0,s0.1), p);
                let b2 = edge2d((s0.0,s0.1), (s1.0,s1.1), p);
                if b0 < 0.0 || b1 < 0.0 || b2 < 0.0 { continue; }
                let (u, v, w_b) = (b0/area, b1/area, b2/area);
                let depth = u*s0.2 + v*s1.2 + w_b*s2.2;
                let idx = y as usize * self.width as usize + x as usize;
                if depth < self.depth[idx] {
                    self.depth[idx] = depth;
                    let color = Color {
                        r: u*c0.r + v*c1.r + w_b*c2.r,
                        g: u*c0.g + v*c1.g + w_b*c2.g,
                        b: u*c0.b + v*c1.b + w_b*c2.b,
                        a: 1.0,
                    };
                    self.pixels[idx] = color.to_bgra_u32();
                }
            }
        }
    }

    // ── Internal rasterizer ───────────────────────────────────────────────────

    fn rasterize(
        &mut self,
        s0: (f32, f32, f32),
        s1: (f32, f32, f32),
        s2: (f32, f32, f32),
        color: Color,
    ) {
        let min_x = s0.0.min(s1.0).min(s2.0).max(0.0) as i32;
        let max_x = (s0.0.max(s1.0).max(s2.0) as i32 + 1).min(self.width  as i32 - 1);
        let min_y = s0.1.min(s1.1).min(s2.1).max(0.0) as i32;
        let max_y = (s0.1.max(s1.1).max(s2.1) as i32 + 1).min(self.height as i32 - 1);

        let area = edge2d((s0.0, s0.1), (s1.0, s1.1), (s2.0, s2.1));
        if area <= 0.0 { return; } // back-face or degenerate

        let packed = color.to_bgra_u32();

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let p = (x as f32 + 0.5, y as f32 + 0.5);
                let b0 = edge2d((s1.0,s1.1), (s2.0,s2.1), p);
                let b1 = edge2d((s2.0,s2.1), (s0.0,s0.1), p);
                let b2 = edge2d((s0.0,s0.1), (s1.0,s1.1), p);
                if b0 < 0.0 || b1 < 0.0 || b2 < 0.0 { continue; }
                let (u, v, w) = (b0/area, b1/area, b2/area);
                let depth = u*s0.2 + v*s1.2 + w*s2.2;
                let idx = y as usize * self.width as usize + x as usize;
                if depth < self.depth[idx] {
                    self.depth[idx] = depth;
                    self.pixels[idx] = packed;
                }
            }
        }
    }
}

impl Renderer for SoftwareRenderer {
    fn width(&self)  -> u32 { self.width }
    fn height(&self) -> u32 { self.height }

    fn resize(&mut self, width: u32, height: u32) {
        self.width  = width;
        self.height = height;
        let n = (width * height) as usize;
        self.pixels.resize(n, 0);
        self.depth.resize(n, f32::INFINITY);
    }

    fn begin_frame(&mut self) {}

    fn clear(&mut self, color: Color) {
        let packed = color.to_bgra_u32();
        self.pixels.iter_mut().zip(self.depth.iter_mut()).for_each(|(p, d)| {
            *p = packed;
            *d = f32::INFINITY;
        });
    }

    fn end_frame(&mut self) {
        // Blit pixel buffer to the window via StretchDIBits.
        unsafe {
            let hdc = self.hdc;

            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize:          std::mem::size_of::<BITMAPINFOHEADER>() as DWORD,
                    biWidth:         self.width  as LONG,
                    biHeight:        -(self.height as LONG), // negative = top-down
                    biPlanes:        1,
                    biBitCount:      32,
                    biCompression:   BI_RGB,
                    biSizeImage:     0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed:       0,
                    biClrImportant:  0,
                },
                bmiColors: [RGBQUAD { rgbBlue: 0, rgbGreen: 0, rgbRed: 0, rgbReserved: 0 }],
            };

            StretchDIBits(
                hdc,
                0, 0, self.width as i32, self.height as i32,
                0, 0, self.width as i32, self.height as i32,
                self.pixels.as_ptr() as *const _,
                &bmi,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
        }
    }
}

impl Drop for SoftwareRenderer {
    fn drop(&mut self) {
        unsafe { ReleaseDC(self.hwnd, self.hdc); }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// 2D edge function (sign = winding side).
#[inline]
fn edge(a: (f32,f32), b: (f32,f32), p: (f32,f32)) -> f32 {
    (p.0 - a.0) * (b.1 - a.1) - (p.1 - a.1) * (b.0 - a.0)
}

/// Edge function for (x,y,z) tuples — ignores z.
#[inline]
fn edge2d(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> f32 {
    edge(a, b, p)
}

/// Perspective divide a clip-space Vec4 → NDC Vec3.
#[inline]
fn ndc(v: Vec4) -> Vec3 {
    Vec3::new(v.x / v.w, v.y / v.w, v.z / v.w)
}

impl super::Draw3d for SoftwareRenderer {
    fn draw_triangle_3d(&mut self, v0: Vec3, v1: Vec3, v2: Vec3, color: Color, mvp: &Mat4) {
        SoftwareRenderer::draw_triangle_3d(self, v0, v1, v2, color, mvp);
    }

    fn draw_triangle_3d_shaded(
        &mut self,
        v0: Vec3, v1: Vec3, v2: Vec3,
        c0: Color, c1: Color, c2: Color,
        mvp: &Mat4,
    ) {
        SoftwareRenderer::draw_triangle_3d_shaded(self, v0, v1, v2, c0, c1, c2, mvp);
    }

    fn draw_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: Color) {
        SoftwareRenderer::draw_line(self, x0, y0, x1, y1, color);
    }
}
