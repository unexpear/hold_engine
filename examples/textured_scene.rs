//! OpenGL-only textured sample scene.
//!
//! Proves a minimal asset-store path for:
//! - `MeshData` handles
//! - `Texture` handles
//! - `Material` handles referencing a shared shader
//! - OpenGL mesh upload + textured draw calls
//!
//! Usage:
//!   cargo run --example textured_scene
//!   cargo run --example textured_scene -- --self-test-seconds 1
//!
//! Controls:
//!   Esc        -- quit
//!   F5         -- toggle orbit pause

use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

use hold::assets::{Material, Shader};
use hold::input::vk;
use hold::renderer::{opengl_shaders, OpenGLRenderer};
use hold::{
    AppExit, AssetStore, Color, Engine, GpuMesh, GpuTexture, Handle, InputState, Mat4, MeshData,
    Renderer, Texture, TextureFormat, Vec3, World,
};

const WINDOW_TITLE: &str = "Hold Engine - Textured Scene";
const FLOOR_TINT: Color = Color::new(0.95, 0.98, 1.0, 1.0);
const CRATE_TINT: Color = Color::new(1.0, 0.98, 0.94, 1.0);
const CLEAR_COLOR: Color = Color::new(0.08, 0.10, 0.12, 1.0);

struct Args {
    self_test_seconds: Option<f32>,
}

impl Args {
    fn parse() -> Self {
        let mut args = std::env::args().skip(1);
        let mut self_test_seconds = None;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--self-test" => self_test_seconds = Some(1.0),
                "--self-test-seconds" => {
                    if let Some(raw) = args.next() {
                        if let Ok(seconds) = raw.parse::<f32>() {
                            self_test_seconds = Some(seconds.max(0.0));
                        }
                    }
                }
                _ => {}
            }
        }

        Self { self_test_seconds }
    }
}

struct TexturedScene {
    assets: AssetStore,
    floor_mesh: Handle<MeshData>,
    cube_mesh: Handle<MeshData>,
    floor_material: Handle<Material>,
    cube_material: Handle<Material>,
    gpu: Option<GpuScene>,
    orbit_angle: f32,
    orbit_paused: bool,
    self_test_remaining: Option<f32>,
}

struct GpuScene {
    meshes: HashMap<Handle<MeshData>, GpuMesh>,
    programs: HashMap<Handle<Shader>, u32>,
    textures: HashMap<Handle<Texture>, GpuTexture>,
}

impl GpuScene {
    fn free(self, renderer: &mut OpenGLRenderer) {
        for (_, mesh) in self.meshes {
            renderer.free_mesh(mesh);
        }
        for (_, texture) in self.textures {
            renderer.free_texture(texture);
        }
        for (_, program) in self.programs {
            renderer.delete_program(program);
        }
    }
}

impl TexturedScene {
    fn new(self_test_remaining: Option<f32>) -> Self {
        let mut assets = AssetStore::new();

        let shader = assets.insert(Shader {
            name: "textured-lit".into(),
            vertex_source: opengl_shaders::TEXTURED_VERT.into(),
            fragment_source: opengl_shaders::TEXTURED_FRAG.into(),
        });

        let floor_texture = assets.insert(build_floor_texture());
        let crate_texture = assets.insert(build_crate_texture());

        let floor_material = assets.insert(Material::new(shader, floor_texture));
        let cube_material = assets.insert(Material::new(shader, crate_texture));

        let mut floor_mesh = MeshData::plane(1, 1);
        for vertex in &mut floor_mesh.vertices {
            vertex.uv[0] *= 6.0;
            vertex.uv[1] *= 6.0;
        }

        let floor_mesh = assets.insert(floor_mesh);
        let cube_mesh = assets.insert(MeshData::cube());

        Self {
            assets,
            floor_mesh,
            cube_mesh,
            floor_material,
            cube_material,
            gpu: None,
            orbit_angle: 0.0,
            orbit_paused: false,
            self_test_remaining,
        }
    }

    fn ensure_gpu(&mut self, renderer: &mut OpenGLRenderer) {
        if self.gpu.is_some() {
            return;
        }

        let mut meshes = HashMap::new();
        let mut programs = HashMap::new();
        let mut textures = HashMap::new();

        for mesh_handle in [self.floor_mesh, self.cube_mesh] {
            let mesh = self
                .assets
                .get(mesh_handle)
                .expect("mesh handle should resolve");
            meshes.insert(mesh_handle, renderer.upload_mesh(mesh));
        }

        let material_handles = [self.floor_material, self.cube_material];
        for material_handle in material_handles {
            let material = self
                .assets
                .get(material_handle)
                .expect("material handle should resolve");

            programs.entry(material.shader).or_insert_with(|| {
                let shader = self
                    .assets
                    .get(material.shader)
                    .expect("shader handle should resolve");
                renderer.compile_shader_program(&shader.vertex_source, &shader.fragment_source)
            });

            textures.entry(material.albedo).or_insert_with(|| {
                let texture = self
                    .assets
                    .get(material.albedo)
                    .expect("texture handle should resolve");
                renderer.upload_texture(texture)
            });
        }

        self.gpu = Some(GpuScene {
            meshes,
            programs,
            textures,
        });
    }

    fn shutdown(&mut self, renderer: &mut OpenGLRenderer) {
        if let Some(gpu) = self.gpu.take() {
            gpu.free(renderer);
        }
    }

    fn draw_material_mesh(
        &self,
        renderer: &mut OpenGLRenderer,
        mesh_handle: Handle<MeshData>,
        material_handle: Handle<Material>,
        model: Mat4,
        view_proj: Mat4,
        view_pos: Vec3,
        light_dir: Vec3,
        ambient: f32,
        tint: Color,
    ) {
        let gpu = self.gpu.as_ref().expect("GPU scene should be initialized");
        let material = self
            .assets
            .get(material_handle)
            .expect("material handle should resolve");
        let mesh = gpu
            .meshes
            .get(&mesh_handle)
            .expect("mesh should have been uploaded");
        let texture = gpu
            .textures
            .get(&material.albedo)
            .expect("texture should have been uploaded");
        let program = *gpu
            .programs
            .get(&material.shader)
            .expect("shader should have been compiled");
        let mvp = view_proj * model;

        renderer.draw_mesh_textured(
            mesh, texture, program, &mvp, &model, tint, light_dir, view_pos, ambient,
        );
    }

    fn update(
        &mut self,
        world: &mut World,
        renderer: &mut OpenGLRenderer,
        input: &InputState,
        dt: f32,
    ) {
        self.ensure_gpu(renderer);

        if input.key_pressed(vk::ESCAPE) {
            self.shutdown(renderer);
            world.events.emit(AppExit);
            return;
        }

        if input.key_pressed(vk::F5) {
            self.orbit_paused = !self.orbit_paused;
        }

        if !self.orbit_paused {
            self.orbit_angle = (self.orbit_angle + dt * 0.45).rem_euclid(TAU);
        }

        if let Some(remaining) = &mut self.self_test_remaining {
            *remaining -= dt;
            if *remaining <= 0.0 {
                self.shutdown(renderer);
                world.events.emit(AppExit);
                return;
            }
        }

        renderer.clear(CLEAR_COLOR);

        let aspect = renderer.width().max(1) as f32 / renderer.height().max(1) as f32;
        let orbit_radius = 5.8;
        let eye = Vec3::new(
            self.orbit_angle.cos() * orbit_radius,
            2.6 + (self.orbit_angle * 0.7).sin() * 0.25,
            self.orbit_angle.sin() * orbit_radius,
        );
        let view = Mat4::look_at(eye, Vec3::new(0.0, 0.65, 0.0), Vec3::Y);
        let proj = Mat4::perspective_gl(60.0_f32.to_radians(), aspect, 0.1, 100.0);
        let view_proj = proj * view;
        let light_dir = Vec3::new(-0.35, 0.85, 0.25).normalize();
        let ambient = 0.24;

        let floor_model =
            Mat4::translate(Vec3::new(0.0, -0.02, 0.0)) * Mat4::scale(Vec3::new(8.0, 1.0, 8.0));
        self.draw_material_mesh(
            renderer,
            self.floor_mesh,
            self.floor_material,
            floor_model,
            view_proj,
            eye,
            light_dir,
            ambient,
            FLOOR_TINT,
        );

        let cube_spin = self.orbit_angle * 1.6;
        let cube_model = Mat4::translate(Vec3::new(0.0, 0.75, 0.0))
            * Mat4::rotate_y(cube_spin)
            * Mat4::rotate_x(PI * 0.08)
            * Mat4::scale_uniform(1.4);
        self.draw_material_mesh(
            renderer,
            self.cube_mesh,
            self.cube_material,
            cube_model,
            view_proj,
            eye,
            light_dir,
            ambient,
            CRATE_TINT,
        );
    }
}

fn build_floor_texture() -> Texture {
    let size = 64u32;
    let cell = 8u32;
    let mut data = Vec::with_capacity((size * size * 4) as usize);

    for y in 0..size {
        for x in 0..size {
            let tile_x = x / cell;
            let tile_y = y / cell;
            let on_border = x % cell == 0 || y % cell == 0;
            let diagonal = (x + y) % (cell * 2) == 0;

            let color = if on_border {
                [22, 28, 32, 255]
            } else if diagonal {
                [48, 88, 92, 255]
            } else if (tile_x + tile_y) % 2 == 0 {
                [36, 52, 58, 255]
            } else {
                [28, 40, 46, 255]
            };
            data.extend_from_slice(&color);
        }
    }

    Texture {
        width: size,
        height: size,
        format: TextureFormat::Rgba8,
        data,
    }
}

fn build_crate_texture() -> Texture {
    let size = 64u32;
    let panel = 16u32;
    let mut data = Vec::with_capacity((size * size * 4) as usize);

    for y in 0..size {
        for x in 0..size {
            let local_x = x % panel;
            let local_y = y % panel;
            let outer_border =
                local_x < 2 || local_y < 2 || local_x >= panel - 2 || local_y >= panel - 2;
            let inner_border =
                local_x == 4 || local_y == 4 || local_x == panel - 5 || local_y == panel - 5;
            let checker = ((x / panel) + (y / panel)) % 2 == 0;

            let color = if outer_border {
                [56, 34, 22, 255]
            } else if inner_border {
                [214, 176, 120, 255]
            } else if checker {
                [152, 98, 58, 255]
            } else {
                [120, 76, 44, 255]
            };
            data.extend_from_slice(&color);
        }
    }

    Texture {
        width: size,
        height: size,
        format: TextureFormat::Rgba8,
        data,
    }
}

fn main() {
    let args = Args::parse();
    let engine = Engine::new(WINDOW_TITLE, 1280, 720);
    let renderer = OpenGLRenderer::new(
        engine.window.hwnd,
        engine.window.width(),
        engine.window.height(),
    );
    let mut scene = TexturedScene::new(args.self_test_seconds);

    println!("[textured] material + texture handle sample ready");
    println!("[textured] Esc=quit  F5=toggle orbit");
    if let Some(seconds) = args.self_test_seconds {
        println!(
            "[textured] self-test mode armed; automatic shutdown after {:.2}s",
            seconds
        );
    }

    engine.run(renderer, move |world, renderer, input, dt| {
        scene.update(world, renderer, input, dt);
    });
}
