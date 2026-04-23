use crate::math::Vec3;

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeSliceAuthoring {
    pub terrain: TerrainAuthoring,
    pub vegetation_prototypes: Vec<VegetationPrototypeDef>,
    pub vegetation_paints: Vec<VegetationPaintStroke>,
    pub static_mesh_actors: Vec<StaticMeshActorDef>,
    pub water_volumes: Vec<WaterVolumeDef>,
    pub spawn_points: Vec<SpawnPointDef>,
    pub patrol_routes: Vec<PatrolRouteDef>,
    pub vehicle_spawners: Vec<VehicleSpawnerDef>,
    pub breakable_props: Vec<BreakablePropDef>,
}

impl RuntimeSliceAuthoring {
    pub fn is_empty(&self) -> bool {
        self.terrain == TerrainAuthoring::default()
            && self.vegetation_prototypes.is_empty()
            && self.vegetation_paints.is_empty()
            && self.static_mesh_actors.is_empty()
            && self.water_volumes.is_empty()
            && self.spawn_points.is_empty()
            && self.patrol_routes.is_empty()
            && self.vehicle_spawners.is_empty()
            && self.breakable_props.is_empty()
    }
}

impl Default for RuntimeSliceAuthoring {
    fn default() -> Self {
        Self {
            terrain: TerrainAuthoring::default(),
            vegetation_prototypes: Vec::new(),
            vegetation_paints: Vec::new(),
            static_mesh_actors: Vec::new(),
            water_volumes: Vec::new(),
            spawn_points: Vec::new(),
            patrol_routes: Vec::new(),
            vehicle_spawners: Vec::new(),
            breakable_props: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TerrainAuthoring {
    pub size_meters: Vec3,
    pub chunk_size_meters: u32,
    pub height_scale_meters: f32,
    pub paint_layers: Vec<TerrainPaintLayer>,
}

impl Default for TerrainAuthoring {
    fn default() -> Self {
        Self {
            size_meters: Vec3::new(1024.0, 256.0, 1024.0),
            chunk_size_meters: 64,
            height_scale_meters: 64.0,
            paint_layers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TerrainPaintLayer {
    pub name: String,
    pub material: Option<String>,
    pub tiling: f32,
    pub weightmap_channel: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VegetationPrototypeDef {
    pub name: String,
    pub mesh: Option<String>,
    pub material: Option<String>,
    pub density_per_100m2: f32,
    pub random_yaw: bool,
    pub align_to_normal: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VegetationPaintStroke {
    pub prototype: String,
    pub center: Vec3,
    pub radius: f32,
    pub density: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StaticMeshActorDef {
    pub name: String,
    pub mesh: Option<String>,
    pub material: Option<String>,
    pub position: Vec3,
    pub rotation_euler_deg: Vec3,
    pub scale: Vec3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WaterVolumeDef {
    pub name: String,
    pub material: Option<String>,
    pub center: Vec3,
    pub extents: Vec3,
    pub wave_strength: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnPointKind {
    Generic,
    Player,
    Enemy,
    Vehicle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpawnPointDef {
    pub name: String,
    pub kind: SpawnPointKind,
    pub position: Vec3,
    pub facing_yaw_deg: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatrolRouteDef {
    pub name: String,
    pub points: Vec<Vec3>,
    pub looped: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VehicleSpawnerDef {
    pub name: String,
    pub mesh: Option<String>,
    pub material: Option<String>,
    pub position: Vec3,
    pub facing_yaw_deg: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BreakablePropDef {
    pub name: String,
    pub intact_mesh: Option<String>,
    pub broken_mesh: Option<String>,
    pub material: Option<String>,
    pub position: Vec3,
    pub rotation_euler_deg: Vec3,
    pub scale: Vec3,
    pub hit_points: f32,
    pub break_impulse: f32,
}

#[cfg(test)]
mod tests {
    use crate::math::Vec3;

    use super::{
        BreakablePropDef, RuntimeSliceAuthoring, SpawnPointDef, SpawnPointKind, TerrainPaintLayer,
        VegetationPrototypeDef,
    };

    #[test]
    fn runtime_slice_authoring_defaults_to_empty_content() {
        let slice = RuntimeSliceAuthoring::default();
        assert!(slice.is_empty());
        assert_eq!(slice.terrain.size_meters, Vec3::new(1024.0, 256.0, 1024.0));
        assert_eq!(slice.terrain.chunk_size_meters, 64);
    }

    #[test]
    fn runtime_slice_authoring_tracks_authored_content() {
        let mut slice = RuntimeSliceAuthoring::default();
        slice.terrain.paint_layers.push(TerrainPaintLayer {
            name: "sand".into(),
            material: None,
            tiling: 6.0,
            weightmap_channel: 0,
        });
        slice.vegetation_prototypes.push(VegetationPrototypeDef {
            name: "palm_tree".into(),
            mesh: None,
            material: None,
            density_per_100m2: 2.5,
            random_yaw: true,
            align_to_normal: true,
        });
        slice.spawn_points.push(SpawnPointDef {
            name: "player_start".into(),
            kind: SpawnPointKind::Player,
            position: Vec3::new(32.0, 5.0, 32.0),
            facing_yaw_deg: 180.0,
        });
        slice.breakable_props.push(BreakablePropDef {
            name: "wooden_crate".into(),
            intact_mesh: None,
            broken_mesh: None,
            material: None,
            position: Vec3::new(14.0, 0.0, 20.0),
            rotation_euler_deg: Vec3::ZERO,
            scale: Vec3::ONE,
            hit_points: 40.0,
            break_impulse: 22.0,
        });

        assert!(!slice.is_empty());
        assert_eq!(slice.terrain.paint_layers.len(), 1);
        assert_eq!(slice.vegetation_prototypes[0].name, "palm_tree");
        assert_eq!(slice.spawn_points[0].kind, SpawnPointKind::Player);
        assert_eq!(slice.breakable_props[0].hit_points, 40.0);
    }
}
