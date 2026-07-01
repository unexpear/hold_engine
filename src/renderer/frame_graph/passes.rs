//! Concrete [`super::Pass`] implementations for common operations.
//!
//! These are the building blocks an OpenGL frame graph is assembled from.
//! Each one does exactly one thing — clear, draw a textured mesh, draw a
//! textured + normal-mapped mesh — so the dependency declarations stay
//! readable when several are chained.
//!
//! For passes that don't fit one of these shapes, drop in a
//! [`FunctionPass`] that wraps an arbitrary `record` closure.

use super::pass::{Pass, PassBuilder, PassContext};
use super::resource::ResourceId;
use crate::assets::{GpuMesh, GpuTexture};
use crate::math::{Mat4, Vec3};
use crate::renderer::{Color, Renderer};

/// Clear the colour buffer (and the depth buffer where the renderer wires
/// it). Optionally declares a write to a [`ResourceId`] so the topological
/// sort treats the clear as the producer of the swap-chain image.
///
/// Keep `target = None` for the simplest case — useful when the rest of the
/// graph doesn't model the swap-chain explicitly. Callers wiring a typed
/// graph should pass `Some(target)` so dependent passes can `read` or
/// `read_write` the same id.
pub struct ClearPass {
    pub name: String,
    pub color: Color,
    pub target: Option<ResourceId>,
}

impl ClearPass {
    pub fn new(name: impl Into<String>, color: Color) -> Self {
        Self {
            name: name.into(),
            color,
            target: None,
        }
    }

    /// Attach a target [`ResourceId`] so this clear participates in the
    /// dependency graph as the producer.
    pub fn with_target(mut self, target: ResourceId) -> Self {
        self.target = Some(target);
        self
    }
}

impl Pass for ClearPass {
    fn name(&self) -> &str {
        &self.name
    }

    fn declare(&self, builder: &mut PassBuilder) {
        if let Some(id) = self.target {
            builder.write(id);
        }
    }

    fn record(&self, ctx: &mut PassContext) {
        ctx.renderer.clear(self.color);
    }
}

/// Draw a single textured mesh with Disney metallic-roughness PBR lighting
/// (no normal map).
///
/// The pass owns Copy-able GPU handles and per-frame matrices, lights, and
/// PBR factors, so constructing one at the top of a frame and feeding it
/// into the graph avoids any lifetime entanglement with surrounding scene
/// state. See ADR 0002 Q3 for the shading model.
#[derive(Debug, Clone, Copy)]
pub struct DrawTexturedPassParams {
    pub mesh: GpuMesh,
    pub albedo: GpuTexture,
    pub program: u32,
    pub mvp: Mat4,
    pub model: Mat4,
    pub tint: Color,
    pub light_dir: Vec3,
    pub view_pos: Vec3,
    pub ambient: f32,
    /// Disney metallic factor in `[0, 1]`.
    pub metallic: f32,
    /// Disney roughness factor in `[0, 1]`.
    pub roughness: f32,
}

pub struct DrawTexturedPass {
    pub name: String,
    pub params: DrawTexturedPassParams,
    /// If set, the pass declares a `read_write` against this id so it
    /// chains correctly after a clear and before later overlay passes.
    pub target: Option<ResourceId>,
}

impl DrawTexturedPass {
    pub fn new(name: impl Into<String>, params: DrawTexturedPassParams) -> Self {
        Self {
            name: name.into(),
            params,
            target: None,
        }
    }

    pub fn with_target(mut self, target: ResourceId) -> Self {
        self.target = Some(target);
        self
    }
}

impl Pass for DrawTexturedPass {
    fn name(&self) -> &str {
        &self.name
    }

    fn declare(&self, builder: &mut PassBuilder) {
        if let Some(id) = self.target {
            builder.read_write(id);
        }
    }

    fn record(&self, ctx: &mut PassContext) {
        let p = &self.params;
        ctx.renderer.draw_mesh_textured(
            &p.mesh,
            &p.albedo,
            p.program,
            &p.mvp,
            &p.model,
            p.tint,
            p.light_dir,
            p.view_pos,
            p.ambient,
            p.metallic,
            p.roughness,
        );
    }
}

/// Draw a textured + normal-mapped mesh. Same shape as
/// [`DrawTexturedPass`] but with a `normal_map` handle and the
/// normal-mapping shader pair from `opengl_shaders::TEXTURED_NORMAL_*`.
///
/// Exists as its own type rather than an `Option<GpuTexture>` on
/// [`DrawTexturedPassParams`] because the typed-pin discipline (ADR 0002 Q4)
/// extends here too — a normal-mapped pass *requires* a normal map at the
/// type level; a flat textured pass *cannot accidentally consume* one.
#[derive(Debug, Clone, Copy)]
pub struct DrawTexturedNormalMappedPassParams {
    pub mesh: GpuMesh,
    pub albedo: GpuTexture,
    pub normal_map: GpuTexture,
    pub program: u32,
    pub mvp: Mat4,
    pub model: Mat4,
    pub tint: Color,
    pub light_dir: Vec3,
    pub view_pos: Vec3,
    pub ambient: f32,
    /// Disney metallic factor in `[0, 1]`.
    pub metallic: f32,
    /// Disney roughness factor in `[0, 1]`.
    pub roughness: f32,
}

pub struct DrawTexturedNormalMappedPass {
    pub name: String,
    pub params: DrawTexturedNormalMappedPassParams,
    pub target: Option<ResourceId>,
}

impl DrawTexturedNormalMappedPass {
    pub fn new(name: impl Into<String>, params: DrawTexturedNormalMappedPassParams) -> Self {
        Self {
            name: name.into(),
            params,
            target: None,
        }
    }

    pub fn with_target(mut self, target: ResourceId) -> Self {
        self.target = Some(target);
        self
    }
}

impl Pass for DrawTexturedNormalMappedPass {
    fn name(&self) -> &str {
        &self.name
    }

    fn declare(&self, builder: &mut PassBuilder) {
        if let Some(id) = self.target {
            builder.read_write(id);
        }
    }

    fn record(&self, ctx: &mut PassContext) {
        let p = &self.params;
        ctx.renderer.draw_mesh_textured_normal_mapped(
            &p.mesh,
            &p.albedo,
            &p.normal_map,
            p.program,
            &p.mvp,
            &p.model,
            p.tint,
            p.light_dir,
            p.view_pos,
            p.ambient,
            p.metallic,
            p.roughness,
        );
    }
}

/// Pass that wraps an arbitrary `record` closure. Useful for one-off passes
/// that don't fit a more specific shape (debug overlays, tooling, custom
/// shader effects under development) and still benefit from being threaded
/// through the dependency graph.
///
/// `declare` is also a closure so the pass can express its resource
/// accesses inline. Both closures are stored as `Box<dyn Fn>` rather than
/// `FnOnce`, mirroring [`super::ClosurePass`] — `record` runs once per
/// frame, so closures must be safe to call repeatedly.
pub struct FunctionPass {
    pub name: String,
    declare_fn: Box<dyn Fn(&mut PassBuilder)>,
    record_fn: Box<dyn Fn(&mut PassContext)>,
}

impl FunctionPass {
    pub fn new(
        name: impl Into<String>,
        declare_fn: impl Fn(&mut PassBuilder) + 'static,
        record_fn: impl Fn(&mut PassContext) + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            declare_fn: Box::new(declare_fn),
            record_fn: Box::new(record_fn),
        }
    }
}

impl Pass for FunctionPass {
    fn name(&self) -> &str {
        &self.name
    }

    fn declare(&self, builder: &mut PassBuilder) {
        (self.declare_fn)(builder);
    }

    fn record(&self, ctx: &mut PassContext) {
        (self.record_fn)(ctx);
    }
}

// ─── Real-scene passes (M9) ────────────────────────────────────────────────
//
// These four passes decompose the editor's per-frame render into stages
// visible to the frame inspector. They pull view/batches from
// `ctx.frame` and lean on narrower public methods on `OpenGLRenderer`
// so each stage's output is a natural thumbnail: viewport cleared,
// scene 3D drawn, debug overlays laid on, screen UI on top.
//
// A future decomposition can split `ScenePass` further (terrain vs
// meshes vs particles) — each such split lands as an additional
// `Pass` here without touching the shell's graph construction shape.
//
// See ADR 0002 Q8 (frame inspector) and ADR 0003 Pillar 4
// (debuggability).

/// Clears the framebuffer and sets the viewport rectangle from
/// `ctx.frame.primary_view()`. If the frame has no primary view, the
/// pass is a no-op — the editor renders "empty scene" without
/// crashing.
///
/// Always the first scene pass; every downstream scene pass writes
/// after it, so wiring a shared "backbuffer" `ResourceId` here gives
/// the topological sort a stable production edge.
pub struct ClearAndViewportPass {
    pub name: String,
    pub target: Option<ResourceId>,
}

impl ClearAndViewportPass {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            target: None,
        }
    }

    pub fn with_target(mut self, target: ResourceId) -> Self {
        self.target = Some(target);
        self
    }
}

impl Pass for ClearAndViewportPass {
    fn name(&self) -> &str {
        &self.name
    }

    fn declare(&self, builder: &mut PassBuilder) {
        if let Some(id) = self.target {
            builder.write(id);
        }
    }

    fn record(&self, ctx: &mut PassContext) {
        if let Some(view) = ctx.frame.primary_view() {
            ctx.renderer.clear_and_setup_view(view);
        }
    }
}

/// Draws the 3D scene for the primary view: terrain, meshes,
/// vegetation, water, and particle batches. Encapsulates the existing
/// [`crate::renderer::opengl::OpenGLRenderer::draw_submitted_view`]
/// call so the M9 inspector shows a single "3D scene" thumbnail
/// after the clear — future work splits this into finer
/// terrain/mesh/water/particle passes once each has its own
/// resource-typed dependency story.
pub struct ScenePass {
    pub name: String,
    pub target: Option<ResourceId>,
}

impl ScenePass {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            target: None,
        }
    }

    pub fn with_target(mut self, target: ResourceId) -> Self {
        self.target = Some(target);
        self
    }
}

impl Pass for ScenePass {
    fn name(&self) -> &str {
        &self.name
    }

    fn declare(&self, builder: &mut PassBuilder) {
        if let Some(id) = self.target {
            builder.read_write(id);
        }
    }

    fn record(&self, ctx: &mut PassContext) {
        if let Some(view) = ctx.frame.primary_view() {
            ctx.renderer.draw_submitted_view(ctx.assets, view);
        }
    }
}

/// Draws the frame's debug lines against the primary view's VP.
/// Runs after the scene so gizmos land on top; if the frame has no
/// primary view, the pass is a no-op.
pub struct DebugLinesPass {
    pub name: String,
    pub target: Option<ResourceId>,
}

impl DebugLinesPass {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            target: None,
        }
    }

    pub fn with_target(mut self, target: ResourceId) -> Self {
        self.target = Some(target);
        self
    }
}

impl Pass for DebugLinesPass {
    fn name(&self) -> &str {
        &self.name
    }

    fn declare(&self, builder: &mut PassBuilder) {
        if let Some(id) = self.target {
            builder.read_write(id);
        }
    }

    fn record(&self, ctx: &mut PassContext) {
        if let Some(view) = ctx.frame.primary_view() {
            let vp = view.projection * view.view;
            ctx.renderer.render_debug_lines(&ctx.frame.debug_lines, &vp);
        }
    }
}

/// Draws the frame's screen-space quads. Always runs last of the
/// scene passes so editor chrome lands on top of both the 3D scene
/// and any debug overlays.
pub struct ScreenQuadsPass {
    pub name: String,
    pub target: Option<ResourceId>,
}

impl ScreenQuadsPass {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            target: None,
        }
    }

    pub fn with_target(mut self, target: ResourceId) -> Self {
        self.target = Some(target);
        self
    }
}

impl Pass for ScreenQuadsPass {
    fn name(&self) -> &str {
        &self.name
    }

    fn declare(&self, builder: &mut PassBuilder) {
        if let Some(id) = self.target {
            builder.read_write(id);
        }
    }

    fn record(&self, ctx: &mut PassContext) {
        ctx.renderer.render_screen_quads(&ctx.frame.screen_quads);
    }
}
