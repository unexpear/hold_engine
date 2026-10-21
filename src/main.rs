//! Demo: spinning Gouraud-shaded tetrahedron.
//! Software rasterizer by default. Set FORGE_BACKEND=vulkan (or vk) for the same scene.
//!
//! Shows:
//!  • Engine + ECS setup
//!  • Transform and Velocity components
//!  • SpinSystem (runs via world.update each frame)
//!  • Software renderer with 3D perspective + depth buffer
//!  • Input: Escape to quit, WASD/arrows to tumble the camera

use forge::{
    ecs::{System, World},
    engine::Engine,
    input::{vk, InputState},
    math::{deg_to_rad, Mat4, Vec3},
    renderer::{software::SoftwareRenderer, vulkan::VulkanRenderer, Color, Draw3d},
    Entity,
};

// ── Components ────────────────────────────────────────────────────────────────

struct Transform {
    rotation_y: f32,
    rotation_x: f32,
}

struct AngularVelocity {
    y: f32, // radians/sec
    x: f32,
}

// ── Systems ───────────────────────────────────────────────────────────────────

struct SpinSystem;

impl System for SpinSystem {
    fn update(&mut self, world: &mut World, dt: f32) {
        // Collect entity IDs first to avoid borrow conflict.
        let entities: Vec<Entity> = world
            .query::<Transform>()
            .map(|(entity, _)| entity)
            .collect();

        for entity in entities {
            // Read velocity
            let (vy, vx) = {
                let vel = world.get::<AngularVelocity>(entity);
                match vel {
                    Some(v) => (v.y, v.x),
                    None    => continue,
                }
            };
            // Update transform
            if let Some(t) = world.get_mut::<Transform>(entity) {
                t.rotation_y += vy * dt;
                t.rotation_x += vx * dt;
            }
        }
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let vulkan = match std::env::var("FORGE_BACKEND") {
        Ok(value) => {
            let value = value.to_ascii_lowercase();
            value == "vulkan" || value == "vk"
        }
        Err(_) => false,
    };
    let title = if vulkan {
        "Forge Engine — Vulkan Demo"
    } else {
        "Forge Engine — Software Demo"
    };
    let mut engine = Engine::new(title, 960, 540);

    // Spawn the tetrahedron entity.
    let tetra = engine.world.spawn();
    engine.world.add_component(tetra, Transform { rotation_y: 0.0, rotation_x: 0.2 });
    engine.world.add_component(tetra, AngularVelocity { y: 1.2, x: 0.4 });

    // Register the spin system so it runs automatically each frame.
    engine.schedule.add_system(SpinSystem);

    let (hwnd, hinstance, w, h) = (
        engine.window.hwnd,
        engine.window.hinstance,
        engine.window.width(),
        engine.window.height(),
    );
    if vulkan {
        let renderer = VulkanRenderer::new(hwnd, hinstance, w, h);
        engine.run(renderer, move |world, renderer, input, dt| {
            demo_frame(tetra, world, renderer, input, dt);
        });
    } else {
        let renderer = SoftwareRenderer::new(hwnd, w, h);
        engine.run(renderer, move |world, renderer, input, dt| {
            demo_frame(tetra, world, renderer, input, dt);
        });
    }

}

fn demo_frame<R: Draw3d>(tetra: Entity, world: &mut World, renderer: &mut R, input: &InputState, _dt: f32) {
    // Camera: sits back a bit, looks at origin.
    let eye    = Vec3::new(0.0, 0.5, 3.0);
    let center = Vec3::ZERO;
    let up     = Vec3::Y;

    // Tetrahedron geometry (unit tetrahedron centred at origin).
    let verts: [Vec3; 4] = [
        Vec3::new( 0.0,  1.0,  0.0),   // apex
        Vec3::new(-0.866, -0.5,  0.0), // left-front
        Vec3::new( 0.866, -0.5,  0.0), // right-front
        Vec3::new( 0.0,  -0.5, -1.0),  // back
    ];
    // Faces (indices), wound counter-clockwise from outside.
    let faces: [[usize; 3]; 4] = [
        [0, 1, 2], // bottom (facing down, front)
        [0, 2, 3], // right
        [0, 3, 1], // left
        [1, 3, 2], // base
    ];
    let face_colors: [Color; 4] = [
        Color::rgb(1.0, 0.2, 0.2),
        Color::rgb(0.2, 1.0, 0.2),
        Color::rgb(0.2, 0.4, 1.0),
        Color::rgb(1.0, 0.9, 0.1),
    ];

        // Quit on Escape.
        if input.key_pressed(vk::ESCAPE) { std::process::exit(0); }

        // Clear to a dark background.
        renderer.clear(Color::rgb(0.05, 0.05, 0.12));

        // Read current transform (if the entity still exists).
        let (ry, rx) = world
            .get::<Transform>(tetra)
            .map(|transform| (transform.rotation_y, transform.rotation_x))
            .unwrap_or((0.0, 0.0));

        // Build MVP.
        let aspect = renderer.width() as f32 / renderer.height() as f32;
        let proj   = Mat4::perspective_gl(deg_to_rad(60.0), aspect, 0.1, 100.0);
        let view   = Mat4::look_at(eye, center, up);
        let model  = Mat4::rotate_y(ry) * Mat4::rotate_x(rx);
        let mvp    = proj * view * model;

        // Draw each face as a shaded triangle.
        for (face_idx, &[a, b, c]) in faces.iter().enumerate() {
            let col = face_colors[face_idx];
            // Simple lighting: slightly darker shade for variation.
            let shaded = Color::rgb(
                col.r * 0.85,
                col.g * 0.85,
                col.b * 0.85,
            );
            renderer.draw_triangle_3d_shaded(
                verts[a], verts[b], verts[c],
                col, shaded, col,
                &mvp,
            );
        }

        // Draw a simple floor grid for spatial reference.
        draw_grid(renderer, &mvp);
}


fn draw_grid<R: Draw3d>(r: &mut R, mvp: &Mat4) {
    let grid_color = Color::rgb(0.25, 0.25, 0.35);
    let y = -0.55_f32;
    let range = 5;
    for i in -range..=range {
        let fi = i as f32;
        // Lines along Z.
        project_line(r, mvp, Vec3::new(fi, y, -range as f32), Vec3::new(fi, y, range as f32), grid_color);
        // Lines along X.
        project_line(r, mvp, Vec3::new(-range as f32, y, fi), Vec3::new(range as f32, y, fi), grid_color);
    }
}

fn project_line<R: Draw3d>(r: &mut R, mvp: &Mat4, a: Vec3, b: Vec3, color: Color) {
    let ca = mvp.transform_point(a);
    let cb = mvp.transform_point(b);
    if ca.w <= 0.0 || cb.w <= 0.0 { return; }

    let w = r.width() as f32;
    let h = r.height() as f32;

    let sx = |v: forge::math::Vec4| ((v.x / v.w) * 0.5 + 0.5) * w;
    let sy = |v: forge::math::Vec4| (1.0 - ((v.y / v.w) * 0.5 + 0.5)) * h;

    r.draw_line(
        sx(ca) as i32, sy(ca) as i32,
        sx(cb) as i32, sy(cb) as i32,
        color,
    );
}
