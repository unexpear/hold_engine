//! Demo: spinning model on procedural terrain with player controller.
//!
//! Usage:
//!   cargo run                         -- procedural tetrahedron (default)
//!   cargo run -- path/to/model.obj    -- load a Wavefront OBJ file
//!
//! Controls:
//!   WASD       -- move
//!   Mouse      -- look around (hold right-click)
//!   Space      -- jump
//!   Shift      -- sprint (2.5x)
//!   F5         -- toggle first-person / third-person camera
//!   Esc        -- quit

use forge::{
    ActiveCamera, Camera, CameraMode, CameraSystem, DirectionalLight,
    Heightmap, LodTerrain, PlayerController, Skybox,
    ecs::System,
    engine::Engine,
    input::{vk, MouseButton},
    math::{Mat4, Quat, Vec3},
    renderer::{Color, Renderer},
    renderer::software::SoftwareRenderer,
    Entity, GlobalTransform, MeshData, Transform, TransformPropagateSystem, World,
};

// -- Components ---------------------------------------------------------------

struct AngularVelocity { y: f32, x: f32 }

// -- Systems ------------------------------------------------------------------

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

// -- Geometry helpers ---------------------------------------------------------

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

fn face_normal(a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let e1 = Vec3::new(b.x - a.x, b.y - a.y, b.z - a.z);
    let e2 = Vec3::new(c.x - a.x, c.y - a.y, c.z - a.z);
    e1.cross(e2).normalize()
}

fn mesh_to_tris(mesh: &MeshData) -> (Vec<Vec3>, Vec<Vec3>, Vec<[usize; 3]>) {
    let verts: Vec<Vec3> = mesh.vertices.iter()
        .map(|v| Vec3::new(v.pos[0], v.pos[1], v.pos[2]))
        .collect();
    let normals: Vec<Vec3> = mesh.vertices.iter()
        .map(|v| Vec3::new(v.normal[0], v.normal[1], v.normal[2]))
        .collect();
    let faces: Vec<[usize; 3]> = mesh.indices.chunks_exact(3)
        .map(|tri| [tri[0] as usize, tri[1] as usize, tri[2] as usize])
        .collect();
    (verts, normals, faces)
}

// -- Main ---------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let obj_path = args.get(1);

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

    // -- Model entity (spinning landmark) -----
    let model_entity = engine.world.spawn();
    engine.world.add_component(model_entity, Transform {
        position: Vec3::new(0.0, 8.0, 0.0),
        rotation: Quat::IDENTITY,
        scale:    Vec3::ONE,
    });
    engine.world.add_component(model_entity, GlobalTransform { matrix: Mat4::IDENTITY });
    engine.world.add_component(model_entity, AngularVelocity { y: 1.2, x: 0.4 });

    // -- Sun ----------------------------------
    let sun = engine.world.spawn();
    engine.world.add_component(sun, DirectionalLight::sun(
        Vec3::new(0.4, 0.8, 0.3),
        Color::WHITE,
    ));

    // -- Player entity (third-person target) --
    let player_entity = engine.world.spawn();
    engine.world.add_component(player_entity, Transform::from_position(Vec3::new(0.0, 20.0, 0.0)));
    engine.world.add_component(player_entity, GlobalTransform { matrix: Mat4::IDENTITY });

    // -- Camera entity ------------------------
    let cam_entity = engine.world.spawn();
    engine.world.add_component(cam_entity, Camera::perspective(60.0, aspect, 0.1, 1000.0));
    engine.world.add_component(cam_entity, CameraMode::first_person());
    engine.world.add_component(cam_entity, ActiveCamera);
    engine.world.add_component(cam_entity, Transform::from_position(Vec3::new(0.0, 20.0, 0.0)));
    engine.world.add_component(cam_entity, GlobalTransform { matrix: Mat4::IDENTITY });

    // -- Systems ------------------------------
    engine.schedule.add_system(SpinSystem);
    engine.schedule.add_system(TransformPropagateSystem);
    engine.schedule.add_system(CameraSystem);

    // -- Renderer -----------------------------
    let (hwnd, w, h) = (engine.window.hwnd, engine.window.width(), engine.window.height());
    let renderer = SoftwareRenderer::new(hwnd, w, h);

    // Prepare model geometry.
    let (obj_verts, obj_normals, obj_faces, face_colors_builtin);

    match &loaded_mesh {
        Some(mesh) => {
            let (v, n, f) = mesh_to_tris(mesh);
            obj_verts   = v;
            obj_normals = n;
            obj_faces   = f;
            face_colors_builtin = vec![];
        }
        None => {
            let (v, f, c) = builtin_tetrahedron();
            let mut normals = vec![Vec3::ZERO; v.len()];
            for face in &f {
                let n = face_normal(v[face[0]], v[face[1]], v[face[2]]);
                normals[face[0]] = normals[face[0]] + Vec3::new(n.x, n.y, n.z);
                normals[face[1]] = normals[face[1]] + Vec3::new(n.x, n.y, n.z);
                normals[face[2]] = normals[face[2]] + Vec3::new(n.x, n.y, n.z);
            }
            for n in &mut normals {
                *n = n.normalize();
            }
            obj_verts   = v;
            obj_normals = normals;
            obj_faces   = f;
            face_colors_builtin = c;
        }
    };

    let obj_color = Color::rgb(0.7, 0.75, 0.85);

    // -- Terrain ------------------------------
    let mut terrain = LodTerrain::new(
        Heightmap::default(),
        3,
        Color::rgb(0.3, 0.55, 0.25),
    );

    // -- Player controller --------------------
    let mut player = PlayerController::new(Vec3::new(0.0, 20.0, 0.0));

    // -- Skybox --------------------------------
    let skybox = Skybox::day(Vec3::new(0.4, 0.8, 0.3));


    engine.run(renderer, move |world, renderer, input, dt| {
        if input.key_pressed(vk::ESCAPE) { std::process::exit(0); }

        // -- Toggle camera mode with F5 --
        if input.key_pressed(vk::F5) {
            let mode = world.get::<CameraMode>(cam_entity).cloned();
            if let Some(current) = mode {
                let new_mode = match current {
                    CameraMode::ThirdPerson { .. } => {
                        CameraMode::first_person()
                    }
                    CameraMode::FirstPerson { .. } => {
                        CameraMode::ThirdPerson {
                            target:   player_entity,
                            distance: 8.0,
                            yaw:      player.yaw,
                            pitch:    0.5,
                        }
                    }
                };
                world.add_component(cam_entity, new_mode);
            }
        }

        // -- Player input (active in all camera modes) --
        {
            let (dx, dy) = if input.mouse_button(MouseButton::Right) {
                input.mouse_delta()
            } else {
                (0, 0)
            };

            let mut move_input = Vec3::ZERO;
            if input.key(vk::W) { move_input.z += 1.0; }
            if input.key(vk::S) { move_input.z -= 1.0; }
            if input.key(vk::D) { move_input.x += 1.0; }
            if input.key(vk::A) { move_input.x -= 1.0; }

            player.update(
                dt,
                move_input,
                (dx as f32, dy as f32),
                input.key_pressed(vk::SPACE),
                input.key(vk::SHIFT),
                &terrain,
            );

            // Sync player entity transform (for third-person target).
            if let Some(t) = world.get_mut::<Transform>(player_entity) {
                t.position = player.position;
            }
        }

        // -- Sync camera to player in first-person --
        {
            let mode = world.get::<CameraMode>(cam_entity).cloned();
            if let Some(CameraMode::FirstPerson { .. }) = mode {
                if let Some(t) = world.get_mut::<Transform>(cam_entity) {
                    t.position = player.eye_position();
                }
                world.add_component(cam_entity, CameraMode::FirstPerson {
                    yaw: player.yaw,
                    pitch: player.pitch,
                    move_speed: player.move_speed,
                    look_sensitivity: player.look_sensitivity,
                });
            }
        }

        // -- Update terrain chunks around the player --
        terrain.update(player.position);

        renderer.clear(Color::BLACK);

        let (view, proj) = world
            .get::<Camera>(cam_entity)
            .map(|c| (c.view, c.proj))
            .unwrap_or((Mat4::IDENTITY, Mat4::IDENTITY));

        // -- Draw skybox (before geometry, writes only colour, no depth) --
        skybox.draw(renderer, &view, &proj);

        let mvp = proj * view;

        let (light_dir, light_color, ambient) = world
            .query::<DirectionalLight>()
            .map(|(_, l)| (l.direction, l.color, l.ambient))
            .next()
            .unwrap_or((Vec3::new(0.0, 1.0, 0.0), Color::WHITE, 0.15));

        // -- Draw terrain (LOD: pick resolution tier per chunk) --
        let terrain_col = terrain.color;
        for chunk in &terrain.chunks {
            let dist = LodTerrain::chunk_distance(player.position, chunk.cx, chunk.cz);
            let tier = chunk.tier_for_dist(dist);
            for face in &tier.faces {
                let &[a, b, c] = face;
                renderer.draw_triangle_3d_lit(
                    tier.verts[a], tier.verts[b], tier.verts[c],
                    tier.normals[a], tier.normals[b], tier.normals[c],
                    terrain_col,
                    light_dir,
                    light_color,
                    ambient,
                    &mvp,
                );
            }
        }

        // -- Draw model --
        for (fi, face) in obj_faces.iter().enumerate() {
            let &[a, b, c] = face;
            let col = if face_colors_builtin.is_empty() { obj_color } else { face_colors_builtin[fi] };
            renderer.draw_triangle_3d_lit(
                obj_verts[a], obj_verts[b], obj_verts[c],
                obj_normals[a], obj_normals[b], obj_normals[c],
                col,
                light_dir,
                light_color,
                ambient,
                &mvp,
            );
        }
    });
}
