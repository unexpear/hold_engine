//! Demo: spinning model on procedural terrain with player controller.
//!
//! Usage:
//!   cargo run --example demo           -- load `examples/demo.hsc`
//!   cargo run --example demo -- path/to/model.obj
//!
//! Controls:
//!   WASD       -- move
//!   Mouse      -- look around (hold right-click)
//!   Space      -- jump
//!   Shift      -- sprint (2.5x)
//!   F5         -- toggle first-person / third-person camera
//!   F6         -- reset to the cached scene snapshot
//!   F7         -- reload `examples/demo.hsc` from disk
//!   F8         -- enter/exit play mode from the authored snapshot
//!   Esc        -- quit

use hold::prelude::*;

// -- Components ---------------------------------------------------------------

struct DemoRuntime {
    model_entity: Entity,
    player_entity: Entity,
    cam_entity: Entity,
    player: PlayerController,
}

const DEMO_SCENE_PATH: &str = "examples/demo.hsc";

fn fatal(message: impl std::fmt::Display) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

fn sync_first_person_camera(world: &mut World, cam_entity: Entity, player: &PlayerController) {
    if let Some(t) = world.get_mut::<Transform>(cam_entity) {
        t.position = player.eye_position();
    }

    world.add_component(
        cam_entity,
        CameraMode::FirstPerson {
            yaw: player.yaw,
            pitch: player.pitch,
            move_speed: player.move_speed,
            look_sensitivity: player.look_sensitivity,
        },
    );

    if let Some(camera) = world.get_mut::<Camera>(cam_entity) {
        let forward = Vec3::new(
            player.yaw.sin() * player.pitch.cos(),
            player.pitch.sin(),
            -(player.yaw.cos() * player.pitch.cos()),
        );
        let eye = player.eye_position();
        camera.view = Mat4::look_at(eye, eye + forward, Vec3::Y);
    }
}

fn load_demo_scene_snapshot() -> Result<SceneData, String> {
    load_scene(DEMO_SCENE_PATH)
        .map_err(|err| format!("Error loading '{}': {}", DEMO_SCENE_PATH, err))
}

fn build_demo_runtime(
    world: &mut World,
    scene: &SceneData,
    is_playing: bool,
) -> Result<DemoRuntime, String> {
    let entity_map = spawn_scene(world, &scene)
        .map_err(|err| format!("Error spawning '{}': {}", DEMO_SCENE_PATH, err))?;

    let model_entity = entity_map.get("model").copied().ok_or_else(|| {
        format!(
            "Scene '{}' is missing required entity 'model'.",
            DEMO_SCENE_PATH
        )
    })?;
    let player_entity = entity_map.get("player").copied().ok_or_else(|| {
        format!(
            "Scene '{}' is missing required entity 'player'.",
            DEMO_SCENE_PATH
        )
    })?;
    let cam_entity = entity_map.get("camera").copied().ok_or_else(|| {
        format!(
            "Scene '{}' is missing required entity 'camera'.",
            DEMO_SCENE_PATH
        )
    })?;

    if !world.has_component::<Transform>(player_entity) {
        return Err("Scene camera/player setup requires a Transform on 'player'.".into());
    }
    if !world.has_component::<Transform>(cam_entity) {
        return Err("Scene camera setup requires a Transform on 'camera'.".into());
    }
    if !world.has_component::<Camera>(cam_entity) {
        return Err("Scene camera setup requires a Camera on 'camera'.".into());
    }
    if !world.has_component::<ActiveCamera>(cam_entity) {
        return Err("Scene camera setup requires 'camera' to be the active camera.".into());
    }

    if !is_playing {
        world.remove_component::<Velocity>(model_entity);
    }

    let emitter_entities: Vec<Entity> = world
        .query::<ParticleEmitter>()
        .map(|(entity, _)| entity)
        .collect();
    for emitter_entity in emitter_entities {
        if let Some(emitter) = world.get_mut::<ParticleEmitter>(emitter_entity) {
            emitter.emitting = is_playing;
            emitter.accumulator = 0.0;
        }
    }

    world.add_component(
        cam_entity,
        CameraMode::ThirdPerson {
            target: player_entity,
            distance: 8.0,
            yaw: 0.0,
            pitch: 0.45,
        },
    );

    let player_start = world
        .get::<Transform>(player_entity)
        .map(|transform| transform.position)
        .ok_or_else(|| "Scene player entity is missing its Transform.".to_string())?;

    Ok(DemoRuntime {
        model_entity,
        player_entity,
        cam_entity,
        player: PlayerController::new(player_start),
    })
}

fn rebuild_demo_world(scene: &SceneData, is_playing: bool) -> Result<(World, DemoRuntime), String> {
    let mut world = World::new();
    let runtime = build_demo_runtime(&mut world, scene, is_playing)?;
    Ok((world, runtime))
}

fn new_demo_terrain() -> LodTerrain {
    LodTerrain::new(Heightmap::default(), 3, Color::rgb(0.3, 0.55, 0.25))
}

// -- Geometry helpers ---------------------------------------------------------

fn builtin_tetrahedron() -> (Vec<Vec3>, Vec<[usize; 3]>, Vec<Color>) {
    let verts = vec![
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(-0.866, -0.5, 0.0),
        Vec3::new(0.866, -0.5, 0.0),
        Vec3::new(0.0, -0.5, -1.0),
    ];
    let faces = vec![[0, 1, 2], [0, 2, 3], [0, 3, 1], [1, 3, 2]];
    let colors = vec![
        Color::rgb(1.0, 0.2, 0.2),
        Color::rgb(0.2, 1.0, 0.2),
        Color::rgb(0.2, 0.4, 1.0),
        Color::rgb(1.0, 0.9, 0.1),
    ];
    (verts, faces, colors)
}

fn compute_vertex_normals(verts: &[Vec3], faces: &[[usize; 3]]) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; verts.len()];

    for face in faces {
        let n = face_normal(verts[face[0]], verts[face[1]], verts[face[2]]);
        normals[face[0]] = normals[face[0]] + Vec3::new(n.x, n.y, n.z);
        normals[face[1]] = normals[face[1]] + Vec3::new(n.x, n.y, n.z);
        normals[face[2]] = normals[face[2]] + Vec3::new(n.x, n.y, n.z);
    }

    for n in &mut normals {
        *n = n.normalize();
    }

    normals
}

fn builtin_player_marker() -> (Vec<Vec3>, Vec<Vec3>, Vec<[usize; 3]>) {
    let verts = vec![
        Vec3::new(0.0, 1.1, 0.0),
        Vec3::new(0.0, 0.3, 0.45),
        Vec3::new(0.35, 0.3, 0.0),
        Vec3::new(0.0, 0.3, -0.35),
        Vec3::new(-0.35, 0.3, 0.0),
        Vec3::new(0.0, -0.9, 0.0),
    ];
    let faces = vec![
        [0, 1, 2],
        [0, 2, 3],
        [0, 3, 4],
        [0, 4, 1],
        [5, 2, 1],
        [5, 3, 2],
        [5, 4, 3],
        [5, 1, 4],
    ];
    let normals = compute_vertex_normals(&verts, &faces);
    (verts, normals, faces)
}

fn face_normal(a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let e1 = Vec3::new(b.x - a.x, b.y - a.y, b.z - a.z);
    let e2 = Vec3::new(c.x - a.x, c.y - a.y, c.z - a.z);
    e1.cross(e2).normalize()
}

fn mesh_to_tris(mesh: &MeshData) -> (Vec<Vec3>, Vec<Vec3>, Vec<[usize; 3]>) {
    let verts: Vec<Vec3> = mesh
        .vertices
        .iter()
        .map(|v| Vec3::new(v.pos[0], v.pos[1], v.pos[2]))
        .collect();
    let normals: Vec<Vec3> = mesh
        .vertices
        .iter()
        .map(|v| Vec3::new(v.normal[0], v.normal[1], v.normal[2]))
        .collect();
    let faces: Vec<[usize; 3]> = mesh
        .indices
        .chunks_exact(3)
        .map(|tri| [tri[0] as usize, tri[1] as usize, tri[2] as usize])
        .collect();
    (verts, normals, faces)
}

// -- Main ---------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let obj_path = args.get(1);

    let loaded_mesh = obj_path.map(|p| match MeshData::from_obj(p) {
        Ok(m) => {
            println!(
                "Loaded {} ({} vertices, {} triangles)",
                p,
                m.vertex_count(),
                m.triangle_count()
            );
            m
        }
        Err(e) => fatal(format!("Error loading {}: {}", p, e)),
    });

    let mut engine = Engine::new("Hold Engine - Demo", 960, 540);
    let mut is_playing = false;
    let mut scene_snapshot = load_demo_scene_snapshot().unwrap_or_else(|err| fatal(err));
    let (initial_world, mut runtime) =
        rebuild_demo_world(&scene_snapshot, is_playing).unwrap_or_else(|err| fatal(err));
    engine.world = initial_world;
    println!(
        "Preview mode loaded from '{}'. Press F8 to enter play mode.",
        DEMO_SCENE_PATH
    );

    // -- Systems ------------------------------
    engine.schedule.add_system(ParticleLifetimeSystem);
    engine.schedule.add_system(ParticleEmitSystem);
    engine.schedule.add_system(PhysicsSystem::default());
    engine.schedule.add_system(TransformPropagateSystem);
    engine.schedule.add_system(CameraSystem);

    // -- Renderer -----------------------------
    let (hwnd, w, h) = (
        engine.window.hwnd,
        engine.window.width(),
        engine.window.height(),
    );
    // SAFETY: `hwnd` is the live window handle just created by `Engine::new`.
    let renderer = unsafe { SoftwareRenderer::new(hwnd, w, h) };

    // Prepare model geometry.
    let (obj_verts, obj_normals, obj_faces, face_colors_builtin);

    match &loaded_mesh {
        Some(mesh) => {
            let (v, n, f) = mesh_to_tris(mesh);
            obj_verts = v;
            obj_normals = n;
            obj_faces = f;
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
            obj_verts = v;
            obj_normals = normals;
            obj_faces = f;
            face_colors_builtin = c;
        }
    };

    let obj_color = Color::rgb(0.7, 0.75, 0.85);
    let (player_verts, player_normals, player_faces) = builtin_player_marker();
    let player_color = Color::rgb(1.0, 0.42, 0.18);

    // -- Terrain ------------------------------
    let mut terrain = new_demo_terrain();

    // -- Skybox -------------------------------
    let skybox = Skybox::day(Vec3::new(0.4, 0.8, 0.3));

    engine.run(renderer, move |world, renderer, input, dt| {
        if input.key_pressed(vk::ESCAPE) {
            std::process::exit(0);
        }

        if input.key_pressed(vk::F6) {
            let (next_world, next_runtime) =
                rebuild_demo_world(&scene_snapshot, is_playing).unwrap_or_else(|err| fatal(err));
            *world = next_world;
            runtime = next_runtime;
            terrain = new_demo_terrain();
            if is_playing {
                println!("Reset play mode to the cached scene snapshot.");
            } else {
                println!("Restored preview mode to the cached scene snapshot.");
            }
            return;
        }

        if input.key_pressed(vk::F7) {
            let next_scene_snapshot = match load_demo_scene_snapshot() {
                Ok(scene) => scene,
                Err(err) => {
                    eprintln!("{err}");
                    return;
                }
            };
            let (next_world, next_runtime) =
                match rebuild_demo_world(&next_scene_snapshot, is_playing) {
                    Ok(next_state) => next_state,
                    Err(err) => {
                        eprintln!("{err}");
                        return;
                    }
                };
            *world = next_world;
            runtime = next_runtime;
            scene_snapshot = next_scene_snapshot;
            terrain = new_demo_terrain();
            if is_playing {
                println!(
                    "Reloaded '{}' from disk and restarted play mode.",
                    DEMO_SCENE_PATH
                );
            } else {
                println!(
                    "Reloaded '{}' from disk into preview mode.",
                    DEMO_SCENE_PATH
                );
            }
            return;
        }

        if input.key_pressed(vk::F8) {
            is_playing = !is_playing;
            let (next_world, next_runtime) =
                rebuild_demo_world(&scene_snapshot, is_playing).unwrap_or_else(|err| fatal(err));
            *world = next_world;
            runtime = next_runtime;
            terrain = new_demo_terrain();
            if is_playing {
                println!("Entered play mode from the authored snapshot.");
            } else {
                println!("Stopped play mode and restored the authored snapshot.");
            }
            return;
        }

        // -- Toggle camera mode with F5 --
        if input.key_pressed(vk::F5) && is_playing {
            let mode = world.get::<CameraMode>(runtime.cam_entity).cloned();
            if let Some(current) = mode {
                let new_mode = match current {
                    CameraMode::ThirdPerson { .. } => CameraMode::first_person(),
                    CameraMode::FirstPerson { .. } => CameraMode::ThirdPerson {
                        target: runtime.player_entity,
                        distance: 8.0,
                        yaw: runtime.player.yaw,
                        pitch: 0.5,
                    },
                };
                world.add_component(runtime.cam_entity, new_mode);
            }
        }

        // -- Player input (active in all camera modes) --
        if is_playing {
            let (dx, dy) = if input.mouse_button(MouseButton::Right) {
                input.mouse_delta()
            } else {
                (0, 0)
            };

            let mut move_input = Vec3::ZERO;
            if input.key(vk::W) {
                move_input.z += 1.0;
            }
            if input.key(vk::S) {
                move_input.z -= 1.0;
            }
            if input.key(vk::D) {
                move_input.x += 1.0;
            }
            if input.key(vk::A) {
                move_input.x -= 1.0;
            }

            runtime.player.update(
                dt,
                move_input,
                (dx as f32, dy as f32),
                input.key_pressed(vk::SPACE),
                input.key(vk::SHIFT),
                &terrain,
            );

            // Sync player entity transform (for third-person target).
            if let Some(t) = world.get_mut::<Transform>(runtime.player_entity) {
                t.position = runtime.player.position;
                t.rotation = Quat::from_euler_xyz(0.0, runtime.player.yaw, 0.0);
            }
        }

        // -- Sync camera to player in first-person --
        if is_playing {
            let mode = world.get::<CameraMode>(runtime.cam_entity).cloned();
            if let Some(CameraMode::FirstPerson { .. }) = mode {
                sync_first_person_camera(world, runtime.cam_entity, &runtime.player);
            }
        }

        // -- Update terrain chunks around the player --
        terrain.update(runtime.player.position);

        if let Some(camera) = world.get_mut::<Camera>(runtime.cam_entity) {
            let height = renderer.height().max(1) as f32;
            camera.set_aspect(renderer.width() as f32 / height);
        }

        renderer.clear(Color::BLACK);

        let (view, proj) = world
            .get::<Camera>(runtime.cam_entity)
            .map(|c| (c.view, c.proj))
            .unwrap_or((Mat4::IDENTITY, Mat4::IDENTITY));

        // -- Draw skybox (before geometry, writes only colour, no depth) --
        skybox.draw(renderer, &view, &proj);

        let view_proj = proj * view;

        let (light_dir, light_color, ambient) = world
            .query::<DirectionalLight>()
            .map(|(_, l)| (l.direction, l.color, l.ambient))
            .next()
            .unwrap_or((Vec3::new(0.0, 1.0, 0.0), Color::WHITE, 0.15));
        let show_player_marker = matches!(
            world.get::<CameraMode>(runtime.cam_entity),
            Some(CameraMode::ThirdPerson { .. })
        );

        // -- Draw terrain (LOD: pick resolution tier per chunk) --
        let terrain_col = terrain.color;
        for chunk in &terrain.chunks {
            let dist = LodTerrain::chunk_distance(runtime.player.position, chunk.cx, chunk.cz);
            let tier = chunk.tier_for_dist(dist);
            for face in &tier.faces {
                let &[a, b, c] = face;
                renderer.draw_triangle_3d_lit(
                    tier.verts[a],
                    tier.verts[b],
                    tier.verts[c],
                    tier.normals[a],
                    tier.normals[b],
                    tier.normals[c],
                    terrain_col,
                    light_dir,
                    light_color,
                    ambient,
                    &view_proj,
                );
            }
        }

        // -- Draw model --
        let model_mvp = world
            .get::<GlobalTransform>(runtime.model_entity)
            .map(|transform| view_proj * transform.matrix)
            .unwrap_or(view_proj);
        for (fi, face) in obj_faces.iter().enumerate() {
            let &[a, b, c] = face;
            let col = if face_colors_builtin.is_empty() {
                obj_color
            } else {
                face_colors_builtin[fi]
            };
            renderer.draw_triangle_3d_lit(
                obj_verts[a],
                obj_verts[b],
                obj_verts[c],
                obj_normals[a],
                obj_normals[b],
                obj_normals[c],
                col,
                light_dir,
                light_color,
                ambient,
                &model_mvp,
            );
        }

        if show_player_marker {
            let player_mvp = world
                .get::<GlobalTransform>(runtime.player_entity)
                .map(|transform| view_proj * transform.matrix)
                .unwrap_or(view_proj);
            for face in &player_faces {
                let &[a, b, c] = face;
                renderer.draw_triangle_3d_lit(
                    player_verts[a],
                    player_verts[b],
                    player_verts[c],
                    player_normals[a],
                    player_normals[b],
                    player_normals[c],
                    player_color,
                    light_dir,
                    light_color,
                    ambient,
                    &player_mvp,
                );
            }
        }

        let particle_lines = generate_particle_debug_lines(world);
        renderer.render_debug_lines(&particle_lines, &view_proj);
    });
}
