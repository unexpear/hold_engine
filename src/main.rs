//! Demo: spinning tetrahedron with an ECS camera.
//!
//! Camera is now a first-class ECS entity:
//!   • Camera       — fov / near / far / aspect + computed view & proj
//!   • CameraMode   — ThirdPerson orbit around the tetrahedron
//!   • ActiveCamera — marks it as the current render camera
//!   • CameraSystem — runs each frame, recomputes view matrix

use forge::{
    ActiveCamera, Camera, CameraMode, CameraSystem,
    ecs::System,
    engine::Engine,
    input::vk,
    math::{Mat4, Quat, Vec3, Vec4},
    renderer::{Color, Renderer},
    renderer::software::SoftwareRenderer,
    Entity, GlobalTransform, Transform, TransformPropagateSystem, World,
};

// ── Components ────────────────────────────────────────────────────────────────

struct AngularVelocity { y: f32, x: f32 }

// ── Systems ───────────────────────────────────────────────────────────────────

struct SpinSystem;

impl System for SpinSystem {
    fn update(&mut self, world: &mut World, dt: f32) {
        let entities: Vec<Entity> = world
            .query::<Transform>()
            .map(|(e, _)| e)
            .collect();

        for entity in entities {
            let (vy, vx) = match world.get::<AngularVelocity>(entity) {
                Some(v) => (v.y, v.x),
                None    => continue,
            };
            if let Some(t) = world.get_mut::<Transform>(entity) {
                // Accumulate rotation via euler increments each frame.
                let delta = Quat::from_euler_xyz(vx * dt, vy * dt, 0.0);
                t.rotation = (delta * t.rotation).normalize();
            }
        }
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let mut engine = Engine::new("Forge Engine — Camera Demo", 960, 540);
    let aspect = 960.0 / 540.0_f32;

    // ── Tetrahedron entity ─────────────────────────────────────────────────────
    let tetra = engine.world.spawn();
    engine.world.add_component(tetra, Transform {
        position: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale:    Vec3::ONE,
    });
    engine.world.add_component(tetra, GlobalTransform { matrix: Mat4::IDENTITY });
    engine.world.add_component(tetra, AngularVelocity { y: 1.2, x: 0.4 });

    // ── Camera entity — third-person orbiting the tetrahedron ──────────────────
    let cam_entity = engine.world.spawn();
    engine.world.add_component(cam_entity, Camera::perspective(60.0, aspect, 0.1, 1000.0));
    engine.world.add_component(cam_entity, CameraMode::ThirdPerson {
        target:   tetra,
        distance: 3.0,
        yaw:      0.0,
        pitch:    0.3,
    });
    engine.world.add_component(cam_entity, ActiveCamera);
    engine.world.add_component(cam_entity, GlobalTransform { matrix: Mat4::IDENTITY });

    // ── Systems: spin → propagate → camera (order matters) ────────────────────
    engine.schedule.add_system(SpinSystem);
    engine.schedule.add_system(TransformPropagateSystem);
    engine.schedule.add_system(CameraSystem);

    // ── Renderer ───────────────────────────────────────────────────────────────
    let (hwnd, w, h) = (engine.window.hwnd, engine.window.width(), engine.window.height());
    let renderer = SoftwareRenderer::new(hwnd, w, h);

    // Tetrahedron geometry.
    let verts: [Vec3; 4] = [
        Vec3::new( 0.0,   1.0,  0.0),
        Vec3::new(-0.866,-0.5,  0.0),
        Vec3::new( 0.866,-0.5,  0.0),
        Vec3::new( 0.0,  -0.5, -1.0),
    ];
    let faces: [[usize; 3]; 4] = [
        [0,1,2], [0,2,3], [0,3,1], [1,3,2],
    ];
    let face_colors: [Color; 4] = [
        Color::rgb(1.0, 0.2, 0.2),
        Color::rgb(0.2, 1.0, 0.2),
        Color::rgb(0.2, 0.4, 1.0),
        Color::rgb(1.0, 0.9, 0.1),
    ];

    engine.run(renderer, move |world, renderer, input, _dt| {
        if input.key_pressed(vk::ESCAPE) { std::process::exit(0); }

        renderer.clear(Color::rgb(0.05, 0.05, 0.12));

        // Read the view + proj matrices computed by CameraSystem this frame.
        let (view, proj) = world
            .get::<Camera>(cam_entity)
            .map(|c| (c.view, c.proj))
            .unwrap_or((Mat4::IDENTITY, Mat4::IDENTITY));

        let mvp = proj * view; // tetrahedron is at world origin (model = identity)

        for (fi, &[a, b, c]) in faces.iter().enumerate() {
            let col = face_colors[fi];
            renderer.draw_triangle_3d_shaded(
                verts[a], verts[b], verts[c],
                col,
                Color::rgb(col.r * 0.6, col.g * 0.6, col.b * 0.6),
                col,
                &mvp,
            );
        }

        draw_grid(renderer, &mvp);
    });
}

fn draw_grid(r: &mut SoftwareRenderer, mvp: &Mat4) {
    let col = Color::rgb(0.25, 0.25, 0.35);
    let y   = -0.55_f32;
    for i in -5_i32..=5 {
        let fi = i as f32;
        project_line(r, mvp, Vec3::new(fi, y, -5.0), Vec3::new(fi, y,  5.0), col);
        project_line(r, mvp, Vec3::new(-5.0, y, fi), Vec3::new(5.0, y, fi),  col);
    }
}

fn project_line(r: &mut SoftwareRenderer, mvp: &Mat4, a: Vec3, b: Vec3, color: Color) {
    let ca = mvp.transform_point(a);
    let cb = mvp.transform_point(b);
    if ca.w <= 0.0 || cb.w <= 0.0 { return; }
    let w  = r.width()  as f32;
    let h  = r.height() as f32;
    let sx = |v: Vec4| ((v.x / v.w) * 0.5 + 0.5) * w;
    let sy = |v: Vec4| (1.0 - ((v.y / v.w) * 0.5 + 0.5)) * h;
    r.draw_line(sx(ca) as i32, sy(ca) as i32, sx(cb) as i32, sy(cb) as i32, color);
}
