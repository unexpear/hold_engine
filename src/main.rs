//! Demo: spinning model with an ECS camera.
//!
//! Usage:
//!   cargo run                         — procedural tetrahedron (default)
//!   cargo run -- path/to/model.obj    — load a Wavefront OBJ file

use forge::{
    ActiveCamera, Camera, CameraMode, CameraSystem,
    ecs::System,
    engine::Engine,
    input::vk,
    math::{Mat4, Quat, Vec3, Vec4},
    renderer::{Color, Renderer},
    renderer::software::SoftwareRenderer,
    Entity, GlobalTransform, MeshData, Transform, TransformPropagateSystem, World,
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
                let delta = Quat::from_euler_xyz(vx * dt, vy * dt, 0.0);
                t.rotation = (delta * t.rotation).normalize();
            }
        }
    }
}

// ── Geometry helpers ──────────────────────────────────────────────────────────

/// Built-in tetrahedron — no file needed.
fn builtin_tetrahedron() -> (Vec<Vec3>, Vec<[usize; 3]>, Vec<Color>) {
    let verts = vec![
        Vec3::new( 0.0,   1.0,  0.0),
        Vec3::new(-0.866,-0.5,  0.0),
        Vec3::new( 0.866,-0.5,  0.0),
        Vec3::new( 0.0,  -0.5, -1.0),
    ];
    let faces = vec![[0,1,2], [0,2,3], [0,3,1], [1,3,2]];
    let colors = vec![
        Color::rgb(1.0, 0.2, 0.2),
        Color::rgb(0.2, 1.0, 0.2),
        Color::rgb(0.2, 0.4, 1.0),
        Color::rgb(1.0, 0.9, 0.1),
    ];
    (verts, faces, colors)
}

/// Convert a MeshData (from OBJ) into software-renderer-friendly arrays.
fn mesh_to_tris(mesh: &MeshData) -> (Vec<Vec3>, Vec<[usize; 3]>) {
    let verts: Vec<Vec3> = mesh.vertices.iter()
        .map(|v| Vec3::new(v.pos[0], v.pos[1], v.pos[2]))
        .collect();
    let faces: Vec<[usize; 3]> = mesh.indices.chunks_exact(3)
        .map(|tri| [tri[0] as usize, tri[1] as usize, tri[2] as usize])
        .collect();
    (verts, faces)
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let obj_path = args.get(1);

    // Try loading the OBJ if a path was given.
    let loaded_mesh = obj_path.map(|p| {
        match MeshData::from_obj(p) {
            Ok(m) => {
                println!("Loaded {} ({} vertices, {} triangles)",
                    p, m.vertex_count(), m.triangle_count());
                m
            }
            Err(e) => {
                eprintln!("Error loading {}: {}", p, e);
                std::process::exit(1);
            }
        }
    });

    let mut engine = Engine::new("Forge Engine — Demo", 960, 540);
    let aspect = 960.0 / 540.0_f32;

    // ── Model entity ──────────────────────────────────────────────────────────
    let model_entity = engine.world.spawn();
    engine.world.add_component(model_entity, Transform {
        position: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale:    Vec3::ONE,
    });
    engine.world.add_component(model_entity, GlobalTransform { matrix: Mat4::IDENTITY });
    engine.world.add_component(model_entity, AngularVelocity { y: 1.2, x: 0.4 });

    // ── Camera entity ─────────────────────────────────────────────────────────
    let cam_entity = engine.world.spawn();
    engine.world.add_component(cam_entity, Camera::perspective(60.0, aspect, 0.1, 1000.0));
    engine.world.add_component(cam_entity, CameraMode::ThirdPerson {
        target:   model_entity,
        distance: 3.0,
        yaw:      0.0,
        pitch:    0.3,
    });
    engine.world.add_component(cam_entity, ActiveCamera);
    engine.world.add_component(cam_entity, GlobalTransform { matrix: Mat4::IDENTITY });

    // ── Systems ───────────────────────────────────────────────────────────────
    engine.schedule.add_system(SpinSystem);
    engine.schedule.add_system(TransformPropagateSystem);
    engine.schedule.add_system(CameraSystem);

    // ── Renderer ──────────────────────────────────────────────────────────────
    let (hwnd, w, h) = (engine.window.hwnd, engine.window.width(), engine.window.height());
    let renderer = SoftwareRenderer::new(hwnd, w, h);

    // Prepare geometry for software renderer.
    let (obj_verts, obj_faces) = match &loaded_mesh {
        Some(mesh) => mesh_to_tris(mesh),
        None => {
            let (v, f, _) = builtin_tetrahedron();
            (v, f)
        }
    };
    let face_colors_builtin;
    let face_colors: &[Color] = match &loaded_mesh {
        None => {
            let (_, _, c) = builtin_tetrahedron();
            face_colors_builtin = c;
            &face_colors_builtin
        }
        Some(_) => &[], // OBJ models use a single colour below
    };
    let obj_color = Color::rgb(0.7, 0.75, 0.85);

    engine.run(renderer, move |world, renderer, input, _dt| {
        if input.key_pressed(vk::ESCAPE) { std::process::exit(0); }

        renderer.clear(Color::rgb(0.05, 0.05, 0.12));

        let (view, proj) = world
            .get::<Camera>(cam_entity)
            .map(|c| (c.view, c.proj))
            .unwrap_or((Mat4::IDENTITY, Mat4::IDENTITY));

        let mvp = proj * view;

        for (fi, face) in obj_faces.iter().enumerate() {
            let &[a, b, c] = face;
            let col = if face_colors.is_empty() { obj_color } else { face_colors[fi] };
            renderer.draw_triangle_3d_shaded(
                obj_verts[a], obj_verts[b], obj_verts[c],
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
