use crate::assets::{MaterialHandle, MeshHandle};
use crate::math::{Mat4, Vec3};

use super::{Color, DebugLine, ScreenQuad};

#[derive(Debug, Clone, PartialEq)]
pub struct RenderFrame {
    pub views: Vec<RenderView>,
    pub debug_lines: Vec<DebugLine>,
    pub screen_quads: Vec<ScreenQuad>,
}

impl Default for RenderFrame {
    fn default() -> Self {
        Self {
            views: Vec::new(),
            debug_lines: Vec::new(),
            screen_quads: Vec::new(),
        }
    }
}

impl RenderFrame {
    pub fn primary_view(&self) -> Option<&RenderView> {
        self.views.first()
    }

    pub fn primary_view_mut(&mut self) -> Option<&mut RenderView> {
        self.views.first_mut()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderView {
    pub name: String,
    pub viewport: RenderViewport,
    pub clear_color: Color,
    pub view: Mat4,
    pub projection: Mat4,
    pub camera_position: Vec3,
    pub fog: Option<FogState>,
    pub draw_items: Vec<DrawItem>,
    pub lights: Vec<LightItem>,
    pub terrain_batches: Vec<TerrainBatch>,
    pub vegetation_batches: Vec<VegetationBatch>,
    pub particle_batches: Vec<ParticleBatch>,
    pub water_batches: Vec<WaterBatch>,
}

impl Default for RenderView {
    fn default() -> Self {
        Self {
            name: "main".into(),
            viewport: RenderViewport::default(),
            clear_color: Color::BLACK,
            view: Mat4::IDENTITY,
            projection: Mat4::IDENTITY,
            camera_position: Vec3::ZERO,
            fog: None,
            draw_items: Vec::new(),
            lights: Vec::new(),
            terrain_batches: Vec::new(),
            vegetation_batches: Vec::new(),
            particle_batches: Vec::new(),
            water_batches: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderViewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Default for RenderViewport {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FogState {
    pub color: Color,
    pub density: f32,
    pub start_distance: f32,
    pub end_distance: f32,
}

impl Default for FogState {
    fn default() -> Self {
        Self {
            color: Color::BLACK,
            density: 0.0,
            start_distance: 0.0,
            end_distance: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawItem {
    pub mesh: MeshHandle,
    pub material: MaterialHandle,
    pub model: Mat4,
    pub cast_shadows: bool,
    pub receive_shadows: bool,
    pub alpha_tested: bool,
}

impl DrawItem {
    pub fn new(mesh: MeshHandle, material: MaterialHandle, model: Mat4) -> Self {
        Self {
            mesh,
            material,
            model,
            cast_shadows: true,
            receive_shadows: true,
            alpha_tested: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightItem {
    pub direction: Vec3,
    pub color: Color,
    pub intensity: f32,
    pub ambient: f32,
}

impl Default for LightItem {
    fn default() -> Self {
        Self {
            direction: Vec3::new(0.3, -1.0, 0.2).normalize(),
            color: Color::WHITE,
            intensity: 1.0,
            ambient: 0.15,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TerrainBatch {
    pub material: MaterialHandle,
    pub patches: Vec<TerrainPatchItem>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainPatchItem {
    pub origin: Vec3,
    pub size: f32,
    pub height_scale: f32,
    pub lod_level: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VegetationBatch {
    pub mesh: MeshHandle,
    pub material: MaterialHandle,
    pub instances: Vec<Mat4>,
    pub wind_strength: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParticleBatch {
    pub material: Option<MaterialHandle>,
    pub billboards: Vec<ParticleBillboard>,
    pub soft_edges: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleBillboard {
    pub position: Vec3,
    pub size: f32,
    pub color: Color,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterBatch {
    pub mesh: Option<MeshHandle>,
    pub material: Option<MaterialHandle>,
    pub model: Mat4,
    pub wave_strength: f32,
    pub flow_direction: Vec3,
    pub fog_color: Color,
}

impl Default for WaterBatch {
    fn default() -> Self {
        Self {
            mesh: None,
            material: None,
            model: Mat4::IDENTITY,
            wave_strength: 0.0,
            flow_direction: Vec3::X,
            fog_color: Color::new(0.0, 0.15, 0.2, 1.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::assets::{MaterialHandle, MeshHandle};
    use crate::math::{Mat4, Vec3};

    use super::{DrawItem, RenderFrame, RenderView, TerrainPatchItem, WaterBatch};

    #[test]
    fn render_frame_exposes_primary_view() {
        let mut frame = RenderFrame::default();
        assert!(frame.primary_view().is_none());

        frame.views.push(RenderView::default());
        frame.views.push(RenderView {
            name: "shadow".into(),
            ..RenderView::default()
        });

        assert_eq!(frame.primary_view().unwrap().name, "main");
        frame.primary_view_mut().unwrap().name = "game".into();
        assert_eq!(frame.primary_view().unwrap().name, "game");
    }

    #[test]
    fn draw_item_defaults_match_runtime_slice_expectations() {
        let item = DrawItem::new(MeshHandle::null(), MaterialHandle::null(), Mat4::IDENTITY);
        assert!(item.cast_shadows);
        assert!(item.receive_shadows);
        assert!(!item.alpha_tested);
        assert!(item.mesh.is_null());
        assert!(item.material.is_null());
    }

    #[test]
    fn water_and_terrain_batches_keep_world_space_values() {
        let water = WaterBatch {
            wave_strength: 0.35,
            flow_direction: Vec3::new(0.5, 0.0, 1.0),
            ..WaterBatch::default()
        };
        let patch = TerrainPatchItem {
            origin: Vec3::new(256.0, 0.0, 512.0),
            size: 64.0,
            height_scale: 28.0,
            lod_level: 2,
        };

        assert!((water.wave_strength - 0.35).abs() < 1e-5);
        assert_eq!(water.flow_direction, Vec3::new(0.5, 0.0, 1.0));
        assert_eq!(patch.origin, Vec3::new(256.0, 0.0, 512.0));
        assert_eq!(patch.lod_level, 2);
    }
}
