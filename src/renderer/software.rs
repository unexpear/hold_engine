//! CPU software renderer with a barycentric triangle rasterizer.
//! Blits each frame to the window via Win32 StretchDIBits -- no GPU required.

use super::{Color, Renderer, ScreenQuad};
use crate::math::{Mat4, Vec3, Vec4};
use crate::platform::ffi::*;

pub struct SoftwareRenderer {
    hwnd: HWND,
    hdc: HDC,
    width: u32,
    height: u32,
    pixels: Vec<u32>,
    depth: Vec<f32>,
}

impl SoftwareRenderer {
    /// # Safety
    /// `hwnd` must be a valid, live Win32 window handle that outlives the
    /// returned renderer. Passing an invalid handle is undefined behaviour in
    /// the Win32 calls this makes (`GetDC`, `StretchDIBits`, …).
    pub unsafe fn new(hwnd: HWND, width: u32, height: u32) -> Self {
        let hdc = unsafe { GetDC(hwnd) };
        assert!(!hdc.is_null(), "GetDC failed");
        let n = (width * height) as usize;
        Self {
            hwnd,
            hdc,
            width,
            height,
            pixels: vec![0; n],
            depth: vec![f32::INFINITY; n],
        }
    }

    #[inline]
    fn clip_to_screen(&self, clip: Vec4) -> (f32, f32, f32) {
        let inv_w = 1.0 / clip.w;
        let nx = clip.x * inv_w;
        let ny = clip.y * inv_w;
        let nz = clip.z * inv_w;
        let w = self.width as f32;
        let h = self.height as f32;
        ((nx * 0.5 + 0.5) * w, (1.0 - (ny * 0.5 + 0.5)) * h, nz)
    }

    #[inline]
    pub fn draw_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        self.pixels[y as usize * self.width as usize + x as usize] = color.to_bgra_u32();
    }

    pub fn draw_line(&mut self, mut x0: i32, mut y0: i32, x1: i32, y1: i32, color: Color) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.draw_pixel(x0, y0, color);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    pub fn draw_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Color) {
        self.draw_line(x, y, x + w - 1, y, color);
        self.draw_line(x + w - 1, y, x + w - 1, y + h - 1, color);
        self.draw_line(x + w - 1, y + h - 1, x, y + h - 1, color);
        self.draw_line(x, y + h - 1, x, y, color);
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Color) {
        let x0 = x.max(0) as u32;
        let y0 = y.max(0) as u32;
        let x1 = (x + w).min(self.width as i32).max(0) as u32;
        let y1 = (y + h).min(self.height as i32).max(0) as u32;
        let packed = color.to_bgra_u32();
        for row in y0..y1 {
            let base = row as usize * self.width as usize;
            for col in x0..x1 {
                self.pixels[base + col as usize] = packed;
            }
        }
    }

    pub fn fill_triangle_2d(
        &mut self,
        v0: (f32, f32),
        v1: (f32, f32),
        v2: (f32, f32),
        color: Color,
    ) {
        let Some((min_x, max_x, min_y, max_y)) =
            triangle_bounds(self.width, self.height, v0, v1, v2)
        else {
            return;
        };
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

    pub fn draw_triangle_3d(&mut self, v0: Vec3, v1: Vec3, v2: Vec3, color: Color, mvp: &Mat4) {
        let c0 = mvp.transform_point(v0);
        let c1 = mvp.transform_point(v1);
        let c2 = mvp.transform_point(v2);
        if c0.w <= 0.0 || c1.w <= 0.0 || c2.w <= 0.0 {
            return;
        }
        let n0 = ndc(c0);
        let n1 = ndc(c1);
        let n2 = ndc(c2);
        if n0.x < -1.0 && n1.x < -1.0 && n2.x < -1.0 {
            return;
        }
        if n0.x > 1.0 && n1.x > 1.0 && n2.x > 1.0 {
            return;
        }
        if n0.y < -1.0 && n1.y < -1.0 && n2.y < -1.0 {
            return;
        }
        if n0.y > 1.0 && n1.y > 1.0 && n2.y > 1.0 {
            return;
        }
        let s0 = self.clip_to_screen(c0);
        let s1 = self.clip_to_screen(c1);
        let s2 = self.clip_to_screen(c2);
        self.rasterize(s0, s1, s2, color);
    }

    pub fn draw_triangle_3d_shaded(
        &mut self,
        v0: Vec3,
        v1: Vec3,
        v2: Vec3,
        c0: Color,
        c1: Color,
        c2: Color,
        mvp: &Mat4,
    ) {
        let clip0 = mvp.transform_point(v0);
        let clip1 = mvp.transform_point(v1);
        let clip2 = mvp.transform_point(v2);
        if clip0.w <= 0.0 || clip1.w <= 0.0 || clip2.w <= 0.0 {
            return;
        }
        let s0 = self.clip_to_screen(clip0);
        let s1 = self.clip_to_screen(clip1);
        let s2 = self.clip_to_screen(clip2);
        let Some((min_x, max_x, min_y, max_y)) = triangle_bounds(
            self.width,
            self.height,
            (s0.0, s0.1),
            (s1.0, s1.1),
            (s2.0, s2.1),
        ) else {
            return;
        };
        let area = edge2d((s0.0, s0.1), (s1.0, s1.1), (s2.0, s2.1));
        if area <= 0.0 {
            return;
        }
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let p = (x as f32 + 0.5, y as f32 + 0.5);
                let b0 = edge2d((s1.0, s1.1), (s2.0, s2.1), p);
                let b1 = edge2d((s2.0, s2.1), (s0.0, s0.1), p);
                let b2 = edge2d((s0.0, s0.1), (s1.0, s1.1), p);
                if b0 < 0.0 || b1 < 0.0 || b2 < 0.0 {
                    continue;
                }
                let (u, v, w_b) = (b0 / area, b1 / area, b2 / area);
                let depth = u * s0.2 + v * s1.2 + w_b * s2.2;
                let idx = y as usize * self.width as usize + x as usize;
                if depth < self.depth[idx] {
                    self.depth[idx] = depth;
                    let color = Color {
                        r: u * c0.r + v * c1.r + w_b * c2.r,
                        g: u * c0.g + v * c1.g + w_b * c2.g,
                        b: u * c0.b + v * c1.b + w_b * c2.b,
                        a: 1.0,
                    };
                    self.pixels[idx] = color.to_bgra_u32();
                }
            }
        }
    }

    /// Lit 3D triangle with per-pixel normal interpolation and directional light.
    pub fn draw_triangle_3d_lit(
        &mut self,
        v0: Vec3,
        v1: Vec3,
        v2: Vec3,
        n0: Vec3,
        n1: Vec3,
        n2: Vec3,
        base_color: Color,
        light_dir: Vec3,
        light_color: Color,
        ambient: f32,
        mvp: &Mat4,
    ) {
        let clip0 = mvp.transform_point(v0);
        let clip1 = mvp.transform_point(v1);
        let clip2 = mvp.transform_point(v2);
        if clip0.w <= 0.0 || clip1.w <= 0.0 || clip2.w <= 0.0 {
            return;
        }
        let s0 = self.clip_to_screen(clip0);
        let s1 = self.clip_to_screen(clip1);
        let s2 = self.clip_to_screen(clip2);
        let Some((min_x, max_x, min_y, max_y)) = triangle_bounds(
            self.width,
            self.height,
            (s0.0, s0.1),
            (s1.0, s1.1),
            (s2.0, s2.1),
        ) else {
            return;
        };
        let area = edge2d((s0.0, s0.1), (s1.0, s1.1), (s2.0, s2.1));
        if area <= 0.0 {
            return;
        }
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let p = (x as f32 + 0.5, y as f32 + 0.5);
                let b0 = edge2d((s1.0, s1.1), (s2.0, s2.1), p);
                let b1 = edge2d((s2.0, s2.1), (s0.0, s0.1), p);
                let b2 = edge2d((s0.0, s0.1), (s1.0, s1.1), p);
                if b0 < 0.0 || b1 < 0.0 || b2 < 0.0 {
                    continue;
                }
                let (u, v, w_b) = (b0 / area, b1 / area, b2 / area);
                let depth = u * s0.2 + v * s1.2 + w_b * s2.2;
                let idx = y as usize * self.width as usize + x as usize;
                if depth < self.depth[idx] {
                    self.depth[idx] = depth;
                    let nx = u * n0.x + v * n1.x + w_b * n2.x;
                    let ny = u * n0.y + v * n1.y + w_b * n2.y;
                    let nz = u * n0.z + v * n1.z + w_b * n2.z;
                    let len = (nx * nx + ny * ny + nz * nz).sqrt();
                    let (nx, ny, nz) = if len > 1e-8 {
                        (nx / len, ny / len, nz / len)
                    } else {
                        (0.0, 1.0, 0.0)
                    };
                    let n_dot_l = (nx * light_dir.x + ny * light_dir.y + nz * light_dir.z).max(0.0);
                    let total = (ambient + (1.0 - ambient) * n_dot_l).min(1.0);
                    let color = Color {
                        r: (base_color.r * light_color.r * total).min(1.0),
                        g: (base_color.g * light_color.g * total).min(1.0),
                        b: (base_color.b * light_color.b * total).min(1.0),
                        a: 1.0,
                    };
                    self.pixels[idx] = color.to_bgra_u32();
                }
            }
        }
    }

    /// Project a world-space 3-D line segment and draw it.
    ///
    /// Lines that straddle the near plane (w ≤ 0 at either endpoint) are
    /// silently skipped — a full near-clip implementation is not needed for
    /// editor debug visualization.
    pub fn draw_line_3d(&mut self, start: Vec3, end: Vec3, color: Color, vp: &Mat4) {
        let cs = vp.transform_point(start);
        let ce = vp.transform_point(end);
        if cs.w <= 0.0 || ce.w <= 0.0 {
            return;
        }
        let (sx, sy, _) = self.clip_to_screen(cs);
        let (ex, ey, _) = self.clip_to_screen(ce);
        self.draw_line(sx as i32, sy as i32, ex as i32, ey as i32, color);
    }

    fn rasterize(
        &mut self,
        s0: (f32, f32, f32),
        s1: (f32, f32, f32),
        s2: (f32, f32, f32),
        color: Color,
    ) {
        let Some((min_x, max_x, min_y, max_y)) = triangle_bounds(
            self.width,
            self.height,
            (s0.0, s0.1),
            (s1.0, s1.1),
            (s2.0, s2.1),
        ) else {
            return;
        };
        let area = edge2d((s0.0, s0.1), (s1.0, s1.1), (s2.0, s2.1));
        if area <= 0.0 {
            return;
        }
        let packed = color.to_bgra_u32();
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let p = (x as f32 + 0.5, y as f32 + 0.5);
                let b0 = edge2d((s1.0, s1.1), (s2.0, s2.1), p);
                let b1 = edge2d((s2.0, s2.1), (s0.0, s0.1), p);
                let b2 = edge2d((s0.0, s0.1), (s1.0, s1.1), p);
                if b0 < 0.0 || b1 < 0.0 || b2 < 0.0 {
                    continue;
                }
                let (u, v, w) = (b0 / area, b1 / area, b2 / area);
                let depth = u * s0.2 + v * s1.2 + w * s2.2;
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
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        let n = (width * height) as usize;
        self.pixels.resize(n, 0);
        self.depth.resize(n, f32::INFINITY);
    }

    fn begin_frame(&mut self) {}

    fn clear(&mut self, color: Color) {
        let packed = color.to_bgra_u32();
        self.pixels
            .iter_mut()
            .zip(self.depth.iter_mut())
            .for_each(|(p, d)| {
                *p = packed;
                *d = f32::INFINITY;
            });
    }

    fn render_debug_lines(&mut self, lines: &[super::DebugLine], vp: &Mat4) {
        for line in lines {
            self.draw_line_3d(line.start, line.end, line.color, vp);
        }
    }

    fn render_screen_quads(&mut self, quads: &[ScreenQuad]) {
        for quad in quads {
            self.fill_rect(
                quad.x.round() as i32,
                quad.y.round() as i32,
                quad.w.round().max(0.0) as i32,
                quad.h.round().max(0.0) as i32,
                quad.color,
            );
        }
    }

    fn end_frame(&mut self) {
        unsafe {
            let hdc = self.hdc;
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as DWORD,
                    biWidth: self.width as LONG,
                    biHeight: -(self.height as LONG),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                },
                bmiColors: [RGBQUAD {
                    rgbBlue: 0,
                    rgbGreen: 0,
                    rgbRed: 0,
                    rgbReserved: 0,
                }],
            };
            StretchDIBits(
                hdc,
                0,
                0,
                self.width as i32,
                self.height as i32,
                0,
                0,
                self.width as i32,
                self.height as i32,
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
        unsafe {
            ReleaseDC(self.hwnd, self.hdc);
        }
    }
}

fn triangle_bounds(
    width: u32,
    height: u32,
    v0: (f32, f32),
    v1: (f32, f32),
    v2: (f32, f32),
) -> Option<(i32, i32, i32, i32)> {
    if width == 0 || height == 0 {
        return None;
    }

    let coords = [v0, v1, v2];
    if coords.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
        return None;
    }

    let min_x = coords
        .iter()
        .map(|(x, _)| *x)
        .fold(f32::INFINITY, f32::min)
        .floor();
    let max_x = coords
        .iter()
        .map(|(x, _)| *x)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil();
    let min_y = coords
        .iter()
        .map(|(_, y)| *y)
        .fold(f32::INFINITY, f32::min)
        .floor();
    let max_y = coords
        .iter()
        .map(|(_, y)| *y)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil();

    let screen_max_x = width as f32 - 1.0;
    let screen_max_y = height as f32 - 1.0;
    if max_x < 0.0 || min_x > screen_max_x || max_y < 0.0 || min_y > screen_max_y {
        return None;
    }

    Some((
        min_x.max(0.0) as i32,
        max_x.min(screen_max_x) as i32,
        min_y.max(0.0) as i32,
        max_y.min(screen_max_y) as i32,
    ))
}

#[inline]
fn edge(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> f32 {
    (p.0 - a.0) * (b.1 - a.1) - (p.1 - a.1) * (b.0 - a.0)
}

#[inline]
fn edge2d(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> f32 {
    edge(a, b, p)
}

#[inline]
fn ndc(v: Vec4) -> Vec3 {
    Vec3::new(v.x / v.w, v.y / v.w, v.z / v.w)
}

#[cfg(test)]
mod tests {
    use super::triangle_bounds;

    #[test]
    fn triangle_bounds_clamps_huge_coordinates() {
        let bounds = triangle_bounds(960, 540, (f32::MAX, 10.0), (100.0, 40.0), (120.0, 60.0));

        assert_eq!(bounds, Some((100, 959, 10, 60)));
    }

    #[test]
    fn triangle_bounds_rejects_non_finite_points() {
        let bounds = triangle_bounds(960, 540, (f32::INFINITY, 0.0), (0.0, 1.0), (2.0, 3.0));

        assert!(bounds.is_none());
    }
}
