use std::collections::HashMap;
use std::f32::consts::TAU;

use crate::assets::{MaterialHandle, MeshHandle};
use crate::math::{Mat4, Vec3};
use crate::renderer::{
    Color, DebugLine, FogState, LightItem, RenderFrame, RenderView, RenderViewport, TerrainBatch,
    TerrainPatchItem, VegetationBatch, WaterBatch,
};
use crate::scene::SceneData;

#[derive(Debug, Clone, Default)]
pub struct RuntimeSliceBindings {
    mesh_handles: HashMap<String, MeshHandle>,
    material_handles: HashMap<String, MaterialHandle>,
}

impl RuntimeSliceBindings {
    pub fn bind_mesh(&mut self, logical_name: impl Into<String>, handle: MeshHandle) {
        self.mesh_handles.insert(logical_name.into(), handle);
    }

    pub fn bind_material(&mut self, logical_name: impl Into<String>, handle: MaterialHandle) {
        self.material_handles.insert(logical_name.into(), handle);
    }

    pub fn resolve_mesh(&self, logical_name: &str) -> Option<MeshHandle> {
        self.mesh_handles.get(logical_name).copied()
    }

    pub fn resolve_material(&self, logical_name: &str) -> Option<MaterialHandle> {
        self.material_handles.get(logical_name).copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuntimeSliceViewConfig {
    pub viewport: RenderViewport,
    pub clear_color: Color,
    pub view: Mat4,
    pub projection: Mat4,
    pub camera_position: Vec3,
    pub light: LightItem,
    pub fog: Option<FogState>,
    pub draw_authoring_helpers: bool,
}

impl Default for RuntimeSliceViewConfig {
    fn default() -> Self {
        Self {
            viewport: RenderViewport::default(),
            clear_color: Color::new(0.42, 0.68, 0.96, 1.0),
            view: Mat4::IDENTITY,
            projection: Mat4::IDENTITY,
            camera_position: Vec3::ZERO,
            light: LightItem::default(),
            fog: None,
            draw_authoring_helpers: true,
        }
    }
}

pub fn build_runtime_slice_frame(
    scene: &SceneData,
    bindings: &RuntimeSliceBindings,
    config: RuntimeSliceViewConfig,
) -> RenderFrame {
    let mut frame = RenderFrame::default();
    let mut view = RenderView {
        name: "runtime-slice".into(),
        viewport: config.viewport,
        clear_color: config.clear_color,
        view: config.view,
        projection: config.projection,
        camera_position: config.camera_position,
        fog: config.fog,
        ..RenderView::default()
    };
    view.lights.push(config.light);

    append_runtime_slice_to_view(scene, bindings, &mut view);
    if config.draw_authoring_helpers {
        append_runtime_slice_debug_lines(scene, &mut frame.debug_lines);
    }

    frame.views.push(view);
    frame
}

pub fn append_runtime_slice_to_view(
    scene: &SceneData,
    bindings: &RuntimeSliceBindings,
    view: &mut RenderView,
) {
    append_runtime_slice_terrain(scene, bindings, view);
    append_runtime_slice_static_meshes(scene, bindings, view);
    append_runtime_slice_vegetation(scene, bindings, view);
    append_runtime_slice_water(scene, bindings, view);
}

fn append_runtime_slice_terrain(
    scene: &SceneData,
    bindings: &RuntimeSliceBindings,
    view: &mut RenderView,
) {
    let terrain = &scene.runtime_slice.terrain;
    if terrain.size_meters.x <= 0.0 || terrain.size_meters.z <= 0.0 {
        return;
    }

    let chunk_size = terrain.chunk_size_meters.max(1) as f32;
    let chunk_count_x = (terrain.size_meters.x / chunk_size).ceil().max(1.0) as u32;
    let chunk_count_z = (terrain.size_meters.z / chunk_size).ceil().max(1.0) as u32;
    let material = terrain
        .paint_layers
        .iter()
        .find_map(|layer| layer.material.as_deref())
        .and_then(|name| bindings.resolve_material(name))
        .unwrap_or_else(MaterialHandle::null);

    let mut patches = Vec::with_capacity((chunk_count_x * chunk_count_z) as usize);
    for cz in 0..chunk_count_z {
        for cx in 0..chunk_count_x {
            let origin = Vec3::new(cx as f32 * chunk_size, 0.0, cz as f32 * chunk_size);
            let center = origin + Vec3::new(chunk_size * 0.5, 0.0, chunk_size * 0.5);
            let lod_level = terrain_lod_level(view.camera_position, center, chunk_size);
            patches.push(TerrainPatchItem {
                origin,
                size: chunk_size,
                height_scale: terrain.height_scale_meters,
                lod_level,
            });
        }
    }

    if !patches.is_empty() {
        view.terrain_batches
            .push(TerrainBatch { material, patches });
    }
}

fn append_runtime_slice_static_meshes(
    scene: &SceneData,
    bindings: &RuntimeSliceBindings,
    view: &mut RenderView,
) {
    for actor in &scene.runtime_slice.static_mesh_actors {
        let (Some(mesh_name), Some(material_name)) =
            (actor.mesh.as_deref(), actor.material.as_deref())
        else {
            continue;
        };
        let (Some(mesh), Some(material)) = (
            bindings.resolve_mesh(mesh_name),
            bindings.resolve_material(material_name),
        ) else {
            continue;
        };
        view.draw_items.push(crate::renderer::DrawItem::new(
            mesh,
            material,
            transform_from_euler(actor.position, actor.rotation_euler_deg, actor.scale),
        ));
    }

    for spawner in &scene.runtime_slice.vehicle_spawners {
        let (Some(mesh_name), Some(material_name)) =
            (spawner.mesh.as_deref(), spawner.material.as_deref())
        else {
            continue;
        };
        let (Some(mesh), Some(material)) = (
            bindings.resolve_mesh(mesh_name),
            bindings.resolve_material(material_name),
        ) else {
            continue;
        };
        view.draw_items.push(crate::renderer::DrawItem::new(
            mesh,
            material,
            Mat4::translate(spawner.position) * Mat4::rotate_y(spawner.facing_yaw_deg.to_radians()),
        ));
    }

    for prop in &scene.runtime_slice.breakable_props {
        let (Some(mesh_name), Some(material_name)) =
            (prop.intact_mesh.as_deref(), prop.material.as_deref())
        else {
            continue;
        };
        let (Some(mesh), Some(material)) = (
            bindings.resolve_mesh(mesh_name),
            bindings.resolve_material(material_name),
        ) else {
            continue;
        };
        view.draw_items.push(crate::renderer::DrawItem::new(
            mesh,
            material,
            transform_from_euler(prop.position, prop.rotation_euler_deg, prop.scale),
        ));
    }
}

fn append_runtime_slice_vegetation(
    scene: &SceneData,
    bindings: &RuntimeSliceBindings,
    view: &mut RenderView,
) {
    for prototype in &scene.runtime_slice.vegetation_prototypes {
        let (Some(mesh_name), Some(material_name)) =
            (prototype.mesh.as_deref(), prototype.material.as_deref())
        else {
            continue;
        };
        let (Some(mesh), Some(material)) = (
            bindings.resolve_mesh(mesh_name),
            bindings.resolve_material(material_name),
        ) else {
            continue;
        };

        let mut instances = Vec::new();
        for stroke in scene
            .runtime_slice
            .vegetation_paints
            .iter()
            .filter(|stroke| stroke.prototype == prototype.name)
        {
            instances.extend(build_vegetation_instances(prototype, stroke));
        }

        if !instances.is_empty() {
            view.vegetation_batches.push(VegetationBatch {
                mesh,
                material,
                instances,
                wind_strength: 0.2,
            });
        }
    }
}

fn append_runtime_slice_water(
    scene: &SceneData,
    bindings: &RuntimeSliceBindings,
    view: &mut RenderView,
) {
    for volume in &scene.runtime_slice.water_volumes {
        view.water_batches.push(WaterBatch {
            mesh: None,
            material: volume
                .material
                .as_deref()
                .and_then(|name| bindings.resolve_material(name)),
            model: Mat4::translate(volume.center)
                * Mat4::scale(Vec3::new(
                    volume.extents.x.max(1.0),
                    volume.extents.y.max(1.0),
                    volume.extents.z.max(1.0),
                )),
            wave_strength: volume.wave_strength,
            flow_direction: Vec3::new(0.6, 0.0, 1.0).normalize(),
            fog_color: Color::new(0.0, 0.18, 0.24, 1.0),
        });
    }
}

fn append_runtime_slice_debug_lines(scene: &SceneData, lines: &mut Vec<DebugLine>) {
    for route in &scene.runtime_slice.patrol_routes {
        for segment in route.points.windows(2) {
            lines.push(DebugLine::new(
                segment[0] + Vec3::new(0.0, 0.2, 0.0),
                segment[1] + Vec3::new(0.0, 0.2, 0.0),
                Color::YELLOW,
            ));
        }
        if route.looped && route.points.len() > 2 {
            lines.push(DebugLine::new(
                route.points[route.points.len() - 1] + Vec3::new(0.0, 0.2, 0.0),
                route.points[0] + Vec3::new(0.0, 0.2, 0.0),
                Color::YELLOW,
            ));
        }
    }

    for spawn in &scene.runtime_slice.spawn_points {
        let pos = spawn.position;
        lines.push(DebugLine::new(
            pos + Vec3::new(-1.0, 0.0, 0.0),
            pos + Vec3::new(1.0, 0.0, 0.0),
            Color::GREEN,
        ));
        lines.push(DebugLine::new(
            pos + Vec3::new(0.0, 0.0, -1.0),
            pos + Vec3::new(0.0, 0.0, 1.0),
            Color::GREEN,
        ));
        let yaw = spawn.facing_yaw_deg.to_radians();
        let facing = Vec3::new(yaw.sin(), 0.0, yaw.cos()) * 2.0;
        lines.push(DebugLine::new(pos, pos + facing, Color::CYAN));
    }
}

fn transform_from_euler(position: Vec3, rotation_euler_deg: Vec3, scale: Vec3) -> Mat4 {
    Mat4::translate(position)
        * Mat4::rotate_y(rotation_euler_deg.y.to_radians())
        * Mat4::rotate_x(rotation_euler_deg.x.to_radians())
        * Mat4::rotate_z(rotation_euler_deg.z.to_radians())
        * Mat4::scale(scale)
}

fn terrain_lod_level(camera_position: Vec3, patch_center: Vec3, chunk_size: f32) -> u8 {
    let chebyshev = (patch_center.x - camera_position.x)
        .abs()
        .max((patch_center.z - camera_position.z).abs())
        / chunk_size.max(1.0);
    if chebyshev <= 1.5 {
        0
    } else if chebyshev <= 3.0 {
        1
    } else {
        2
    }
}

fn build_vegetation_instances(
    prototype: &crate::runtime_slice::VegetationPrototypeDef,
    stroke: &crate::runtime_slice::VegetationPaintStroke,
) -> Vec<Mat4> {
    let area_units = std::f32::consts::PI * stroke.radius * stroke.radius / 100.0;
    let count = (area_units * prototype.density_per_100m2 * stroke.density)
        .round()
        .max(0.0) as usize;
    let count = count.max(1);

    let mut instances = Vec::with_capacity(count);
    for index in 0..count {
        let radial_t = ((index as f32) + 0.5) / count as f32;
        let radius = stroke.radius * radial_t.sqrt();
        let jitter = hash_unit(&stroke.prototype, index as u32);
        let angle = (index as f32 * 2.399_963_1 + jitter * TAU).rem_euclid(TAU);
        let position = stroke.center + Vec3::new(angle.cos() * radius, 0.0, angle.sin() * radius);
        let scale = 0.85 + hash_unit(&prototype.name, index as u32 + 41) * 0.35;
        let yaw = if prototype.random_yaw {
            hash_unit(&prototype.name, index as u32 + 97) * TAU
        } else {
            0.0
        };
        instances
            .push(Mat4::translate(position) * Mat4::rotate_y(yaw) * Mat4::scale_uniform(scale));
    }
    instances
}

fn hash_unit(label: &str, salt: u32) -> f32 {
    let mut hash = 2166136261u32 ^ salt;
    for &byte in label.as_bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(16777619);
    }
    (hash as f32) / (u32::MAX as f32)
}

#[cfg(test)]
mod tests {
    use crate::assets::{MaterialHandle, MeshHandle};
    use crate::renderer::RenderViewport;
    use crate::runtime_slice::{
        BreakablePropDef, PatrolRouteDef, RuntimeSliceAuthoring, SpawnPointDef, SpawnPointKind,
        StaticMeshActorDef, TerrainAuthoring, TerrainPaintLayer, VegetationPaintStroke,
        VegetationPrototypeDef, VehicleSpawnerDef, WaterVolumeDef,
    };

    use super::*;

    fn sample_scene() -> SceneData {
        SceneData {
            name: "Island".into(),
            runtime_slice: RuntimeSliceAuthoring {
                terrain: TerrainAuthoring {
                    size_meters: Vec3::new(1024.0, 256.0, 1024.0),
                    chunk_size_meters: 64,
                    height_scale_meters: 72.0,
                    paint_layers: vec![TerrainPaintLayer {
                        name: "sand".into(),
                        material: Some("materials/sand".into()),
                        tiling: 6.0,
                        weightmap_channel: 0,
                    }],
                },
                vegetation_prototypes: vec![VegetationPrototypeDef {
                    name: "palm".into(),
                    mesh: Some("meshes/palm".into()),
                    material: Some("materials/palm".into()),
                    density_per_100m2: 2.5,
                    random_yaw: true,
                    align_to_normal: true,
                }],
                vegetation_paints: vec![VegetationPaintStroke {
                    prototype: "palm".into(),
                    center: Vec3::new(48.0, 0.0, 80.0),
                    radius: 12.0,
                    density: 1.5,
                }],
                static_mesh_actors: vec![StaticMeshActorDef {
                    name: "rock".into(),
                    mesh: Some("meshes/rock".into()),
                    material: Some("materials/rock".into()),
                    position: Vec3::new(10.0, 0.0, 12.0),
                    rotation_euler_deg: Vec3::new(0.0, 30.0, 0.0),
                    scale: Vec3::new(2.0, 2.0, 2.0),
                }],
                water_volumes: vec![WaterVolumeDef {
                    name: "ocean".into(),
                    material: Some("materials/ocean".into()),
                    center: Vec3::new(512.0, 0.0, 512.0),
                    extents: Vec3::new(1024.0, 6.0, 1024.0),
                    wave_strength: 0.35,
                }],
                spawn_points: vec![SpawnPointDef {
                    name: "player_start".into(),
                    kind: SpawnPointKind::Player,
                    position: Vec3::new(32.0, 4.0, 32.0),
                    facing_yaw_deg: 180.0,
                }],
                patrol_routes: vec![PatrolRouteDef {
                    name: "route_a".into(),
                    points: vec![Vec3::ZERO, Vec3::new(10.0, 0.0, 10.0)],
                    looped: false,
                }],
                vehicle_spawners: vec![VehicleSpawnerDef {
                    name: "jeep".into(),
                    mesh: Some("meshes/jeep".into()),
                    material: Some("materials/jeep".into()),
                    position: Vec3::new(40.0, 0.0, 40.0),
                    facing_yaw_deg: 90.0,
                }],
                breakable_props: vec![BreakablePropDef {
                    name: "crate".into(),
                    intact_mesh: Some("meshes/crate".into()),
                    broken_mesh: Some("meshes/crate_broken".into()),
                    material: Some("materials/crate".into()),
                    position: Vec3::new(14.0, 0.0, 20.0),
                    rotation_euler_deg: Vec3::ZERO,
                    scale: Vec3::ONE,
                    hit_points: 40.0,
                    break_impulse: 22.0,
                }],
            },
            entities: Vec::new(),
        }
    }

    fn sample_bindings() -> RuntimeSliceBindings {
        let mut bindings = RuntimeSliceBindings::default();
        bindings.bind_material("materials/sand", MaterialHandle::null());
        bindings.bind_material("materials/palm", MaterialHandle::null());
        bindings.bind_material("materials/rock", MaterialHandle::null());
        bindings.bind_material("materials/ocean", MaterialHandle::null());
        bindings.bind_material("materials/jeep", MaterialHandle::null());
        bindings.bind_material("materials/crate", MaterialHandle::null());
        bindings.bind_mesh("meshes/palm", MeshHandle::null());
        bindings.bind_mesh("meshes/rock", MeshHandle::null());
        bindings.bind_mesh("meshes/jeep", MeshHandle::null());
        bindings.bind_mesh("meshes/crate", MeshHandle::null());
        bindings
    }

    #[test]
    fn build_runtime_slice_frame_emits_expected_slice_batches() {
        let frame = build_runtime_slice_frame(
            &sample_scene(),
            &sample_bindings(),
            RuntimeSliceViewConfig {
                viewport: RenderViewport {
                    x: 0,
                    y: 0,
                    width: 1280,
                    height: 720,
                },
                camera_position: Vec3::new(96.0, 16.0, 96.0),
                ..RuntimeSliceViewConfig::default()
            },
        );

        assert_eq!(frame.views.len(), 1);
        let view = frame.primary_view().unwrap();
        assert_eq!(view.draw_items.len(), 3);
        assert_eq!(view.terrain_batches.len(), 1);
        assert_eq!(view.terrain_batches[0].patches.len(), 256);
        assert_eq!(view.terrain_batches[0].patches[0].lod_level, 0);
        assert_eq!(view.vegetation_batches.len(), 1);
        assert!(!view.vegetation_batches[0].instances.is_empty());
        assert_eq!(view.water_batches.len(), 1);
        assert_eq!(frame.debug_lines.len(), 4);
    }

    #[test]
    fn unresolved_bindings_skip_handle_backed_draws_but_keep_authored_helpers() {
        let frame = build_runtime_slice_frame(
            &sample_scene(),
            &RuntimeSliceBindings::default(),
            RuntimeSliceViewConfig::default(),
        );

        let view = frame.primary_view().unwrap();
        assert!(view.draw_items.is_empty());
        assert!(view.vegetation_batches.is_empty());
        assert_eq!(view.water_batches.len(), 1);
        assert_eq!(view.terrain_batches.len(), 1);
        assert!(!frame.debug_lines.is_empty());
    }
}
