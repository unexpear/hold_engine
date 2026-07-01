//! Typed, dependency-resolving frame graph for the renderer.
//!
//! See [`docs/adr/0002-material-pipeline.md`] (`Q8`) for the full design
//! rationale. Short version:
//!
//! - A frame is a graph of [`Pass`]es.
//! - Edges between passes are inferred from typed [`ResourceId`] reads and
//!   writes declared via [`PassBuilder`].
//! - The graph is *compiled* into a topologically sorted execution order
//!   (with cycle and type-mismatch detection) before any GPU work runs.
//! - The compiled graph is then *executed*: each pass's
//!   [`Pass::record`] runs in dependency order against a per-pass
//!   [`PassContext`].
//!
//! v1 scope: pure topology + descriptor types. The execute path calls
//! `record` against a stub [`PassContext`]; concrete OpenGL/Vulkan
//! integration arrives in a follow-up commit so this module's design can be
//! reviewed in isolation. The v1 graph also does not yet allocate transient
//! resources or capture per-pass thumbnails for the editor frame inspector;
//! those are tracked under the same ADR for the next M4 slice.

mod capture;
mod compile;
mod pass;
mod passes;
mod resource;

pub use capture::{downsample_rgba8, PassThumbnail, DEFAULT_THUMBNAIL_SIZE};
pub use pass::{ClosurePass, FrameGraphError, NoopPass, Pass, PassBuilder, PassContext};
pub use passes::{
    ClearAndViewportPass, ClearPass, DebugLinesPass, DrawTexturedNormalMappedPass,
    DrawTexturedNormalMappedPassParams, DrawTexturedPass, DrawTexturedPassParams, FunctionPass,
    ScenePass, ScreenQuadsPass,
};
pub use resource::{
    AccessKind, BufferDesc, BufferUsage, ResourceDesc, ResourceId, ResourceKind, TextureDesc,
    TextureUsage,
};

use compile::compile_graph;

/// Mutable, in-progress description of the frame graph. Use
/// [`FrameGraph::builder`] to construct one, register resources and passes
/// against it, then call [`FrameGraphBuilder::compile`] to lock it down.
pub struct FrameGraphBuilder {
    resources: Vec<ResourceDesc>,
    /// Indices into `resources` that came from `import_*` rather than
    /// `create_*`. Imported resources are not allocated by the graph; they
    /// stand in for real GPU objects (e.g. the swap-chain image).
    imported: Vec<u32>,
    passes: Vec<Box<dyn Pass>>,
}

impl Default for FrameGraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameGraphBuilder {
    /// Create an empty builder.
    pub fn new() -> Self {
        Self {
            resources: Vec::new(),
            imported: Vec::new(),
            passes: Vec::new(),
        }
    }

    /// Declare a transient texture. The graph allocates it (in a future
    /// commit; v1 leaves allocation to the backend integration step) and
    /// tracks its lifetime.
    pub fn create_texture(&mut self, desc: TextureDesc) -> ResourceId {
        let index = self.resources.len() as u32;
        self.resources.push(ResourceDesc::Texture(desc));
        ResourceId {
            index,
            kind: ResourceKind::Texture,
        }
    }

    /// Declare a transient buffer.
    pub fn create_buffer(&mut self, desc: BufferDesc) -> ResourceId {
        let index = self.resources.len() as u32;
        self.resources.push(ResourceDesc::Buffer(desc));
        ResourceId {
            index,
            kind: ResourceKind::Buffer,
        }
    }

    /// Import an externally owned texture (e.g. swap-chain image, persistent
    /// shadow atlas). Imported resources have known descriptors but the
    /// graph does not allocate or free them.
    pub fn import_texture(&mut self, desc: TextureDesc) -> ResourceId {
        let id = self.create_texture(desc);
        self.imported.push(id.index);
        id
    }

    /// Import an externally owned buffer.
    pub fn import_buffer(&mut self, desc: BufferDesc) -> ResourceId {
        let id = self.create_buffer(desc);
        self.imported.push(id.index);
        id
    }

    /// Add a pass. Passes execute in dependency order computed from their
    /// resource accesses, not in the order they were added.
    pub fn add_pass(&mut self, pass: Box<dyn Pass>) {
        self.passes.push(pass);
    }

    /// Lock the builder into a compiled graph. Validates dependency
    /// topology, type compatibility, and writer uniqueness; surfaces the
    /// first failure as a [`FrameGraphError`].
    pub fn compile(self) -> Result<FrameGraph, FrameGraphError> {
        let resource_count = self.resources.len() as u32;
        let pass_names: Vec<String> = self.passes.iter().map(|p| p.name().to_string()).collect();

        // Each pass declares its accesses against a fresh PassBuilder.
        // We collect any builder errors, return the first if any are
        // present, then feed the access tables into the compile algorithm.
        let mut all_accesses: Vec<Vec<pass::ResourceAccess>> = Vec::with_capacity(self.passes.len());
        for (i, p) in self.passes.iter().enumerate() {
            let mut b = PassBuilder::new(i, resource_count);
            p.declare(&mut b);
            if let Some(err) = b.errors.into_iter().next() {
                return Err(err);
            }
            all_accesses.push(b.accesses);
        }

        let output = compile_graph(all_accesses, &pass_names, resource_count)?;

        Ok(FrameGraph {
            resources: self.resources,
            imported: self.imported,
            passes: self.passes,
            execution_order: output.execution_order,
        })
    }
}

/// A compiled frame graph ready to execute.
pub struct FrameGraph {
    resources: Vec<ResourceDesc>,
    imported: Vec<u32>,
    passes: Vec<Box<dyn Pass>>,
    execution_order: Vec<usize>,
}

impl FrameGraph {
    /// Start a new builder.
    pub fn builder() -> FrameGraphBuilder {
        FrameGraphBuilder::new()
    }

    /// All resource descriptors registered in the graph (transient and
    /// imported, in declaration order).
    pub fn resources(&self) -> &[ResourceDesc] {
        &self.resources
    }

    /// Indices (into [`Self::resources`]) of resources that were imported
    /// rather than created. Useful for backends that need to skip
    /// allocation for these.
    pub fn imported_resource_indices(&self) -> &[u32] {
        &self.imported
    }

    /// Number of passes in the graph.
    pub fn pass_count(&self) -> usize {
        self.passes.len()
    }

    /// The execution order computed at compile time. Each entry is an
    /// index into the graph's pass list. Tie-breaks on declaration order.
    pub fn execution_order(&self) -> &[usize] {
        &self.execution_order
    }

    /// Run the graph: invoke each pass's [`Pass::record`] in execution
    /// order, threading the OpenGL renderer, asset store, and the
    /// current [`RenderFrame`] through the per-pass [`PassContext`].
    ///
    /// Borrows all three for the duration of the call. Passes that need
    /// to retain references beyond `record` should clone the relevant
    /// handles into their own struct fields rather than stashing the
    /// borrows.
    ///
    /// The `frame` is the M9 wire-up: the graph is built once at editor
    /// startup and driven by whatever frame the shell hands it each
    /// tick, so scene passes read views/batches from `ctx.frame`
    /// rather than caching per-frame state inside pass structs.
    pub fn execute(
        &self,
        renderer: &mut crate::renderer::opengl::OpenGLRenderer,
        assets: &crate::assets::AssetStore,
        frame: &crate::renderer::RenderFrame,
    ) {
        for &pass_idx in &self.execution_order {
            let p = &self.passes[pass_idx];
            let name: String = p.name().to_string();
            let mut ctx = PassContext::new(&name, pass_idx, renderer, assets, frame);
            p.record(&mut ctx);
        }
    }

    /// Like [`Self::execute`] but also reads back the framebuffer after
    /// every pass and packages the result as a [`PassThumbnail`]. Returns
    /// the captures in execution order.
    ///
    /// Implements ADR 0002 Q8's editor frame inspector hook: the
    /// "Substance Designer view-a-node" gesture. The capture goes through
    /// `glReadPixels`, which is synchronous and stalls the GPU pipeline,
    /// so this path is intended for editor / debug capture only — production
    /// rendering should stay on [`Self::execute`].
    ///
    /// `thumb_w`, `thumb_h` set the thumbnail dimensions; pass
    /// [`DEFAULT_THUMBNAIL_SIZE`] for a sensible default. The captured
    /// framebuffer is downsampled on the CPU using box-averaging
    /// (see [`downsample_rgba8`]).
    pub fn execute_with_capture(
        &self,
        renderer: &mut crate::renderer::opengl::OpenGLRenderer,
        assets: &crate::assets::AssetStore,
        frame: &crate::renderer::RenderFrame,
        thumb_w: u32,
        thumb_h: u32,
    ) -> Vec<PassThumbnail> {
        let mut thumbs = Vec::with_capacity(self.execution_order.len());
        for &pass_idx in &self.execution_order {
            let p = &self.passes[pass_idx];
            let name: String = p.name().to_string();

            // Run the pass.
            {
                let mut ctx = PassContext::new(&name, pass_idx, renderer, assets, frame);
                p.record(&mut ctx);
            }

            // Synchronously read the framebuffer. The borrow of `renderer`
            // from PassContext is released by the scope above; we can
            // freely re-borrow here.
            // Pull dimensions through the Renderer trait so we don't depend
            // on private fields of OpenGLRenderer.
            let fb_w = <crate::renderer::opengl::OpenGLRenderer as crate::renderer::Renderer>::width(renderer).max(1);
            let fb_h = <crate::renderer::opengl::OpenGLRenderer as crate::renderer::Renderer>::height(renderer).max(1);
            let full = renderer.read_framebuffer_rgba8();
            let data = downsample_rgba8(&full, fb_w, fb_h, thumb_w, thumb_h);
            thumbs.push(PassThumbnail {
                pass_index: pass_idx,
                pass_name: name,
                width: thumb_w,
                height: thumb_h,
                data,
            });
        }
        thumbs
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::TextureFormat;

    fn tex(name: &str) -> TextureDesc {
        TextureDesc {
            name: name.into(),
            format: TextureFormat::Rgba8,
            width: 64,
            height: 64,
            usage: TextureUsage::ColorAttachment,
        }
    }

    /// An empty graph compiles cleanly. Topology-only check; the real
    /// `execute(renderer, assets)` path is exercised by the OpenGL
    /// integration test (see textured_scene example) since it requires a
    /// live HWND-bound renderer that can't be constructed in unit tests.
    #[test]
    fn empty_graph_compiles_cleanly() {
        let g = FrameGraph::builder().compile().unwrap();
        assert_eq!(g.pass_count(), 0);
        assert_eq!(g.execution_order().len(), 0);
    }

    /// Single pass with no dependencies sits at execution order [0].
    /// We don't drive `execute` here — that path needs a real renderer;
    /// see the `textured_scene` example for end-to-end verification.
    #[test]
    fn single_pass_lands_at_index_zero() {
        let mut b = FrameGraph::builder();
        b.add_pass(Box::new(NoopPass::new("lonely")));
        let g = b.compile().unwrap();
        assert_eq!(g.execution_order(), &[0]);
        assert_eq!(g.pass_count(), 1);
    }

    /// Three passes in a linear chain (A writes X → B reads X / writes Y →
    /// C reads Y) must execute in that order even when added out of order.
    #[test]
    fn linear_chain_orders_by_dependency() {
        let mut b = FrameGraph::builder();
        let x = b.create_texture(tex("x"));
        let y = b.create_texture(tex("y"));
        // Add C before B before A — declaration order should not be the
        // execution order; the dependency chain is the source of truth.
        b.add_pass(Box::new(ClosurePass::new(
            "C",
            {
                let y = y;
                move |pb| pb.read(y)
            },
            |_| {},
        )));
        b.add_pass(Box::new(ClosurePass::new(
            "B",
            {
                let x = x;
                let y = y;
                move |pb| {
                    pb.read(x);
                    pb.write(y);
                }
            },
            |_| {},
        )));
        b.add_pass(Box::new(ClosurePass::new(
            "A",
            {
                let x = x;
                move |pb| pb.write(x)
            },
            |_| {},
        )));
        let g = b.compile().unwrap();
        let names: Vec<&str> = g
            .execution_order()
            .iter()
            .map(|&i| g.passes[i].name())
            .collect();
        assert_eq!(names, vec!["A", "B", "C"]);
    }

    /// Diamond: A writes X, B and C both read X, D reads outputs of B and C.
    /// A must come first; D must come last; B and C come in declaration
    /// order (deterministic tie-break).
    #[test]
    fn diamond_dependency_orders_deterministically() {
        let mut b = FrameGraph::builder();
        let x = b.create_texture(tex("x"));
        let bx = b.create_texture(tex("bx"));
        let cx = b.create_texture(tex("cx"));
        b.add_pass(Box::new(ClosurePass::new(
            "A",
            {
                let x = x;
                move |pb| pb.write(x)
            },
            |_| {},
        )));
        b.add_pass(Box::new(ClosurePass::new(
            "B",
            {
                let x = x;
                let bx = bx;
                move |pb| {
                    pb.read(x);
                    pb.write(bx);
                }
            },
            |_| {},
        )));
        b.add_pass(Box::new(ClosurePass::new(
            "C",
            {
                let x = x;
                let cx = cx;
                move |pb| {
                    pb.read(x);
                    pb.write(cx);
                }
            },
            |_| {},
        )));
        b.add_pass(Box::new(ClosurePass::new(
            "D",
            {
                let bx = bx;
                let cx = cx;
                move |pb| {
                    pb.read(bx);
                    pb.read(cx);
                }
            },
            |_| {},
        )));
        let g = b.compile().unwrap();
        let names: Vec<&str> = g
            .execution_order()
            .iter()
            .map(|&i| g.passes[i].name())
            .collect();
        assert_eq!(names[0], "A");
        assert_eq!(names[3], "D");
        // B and C: deterministic tie-break = declaration order.
        assert_eq!(names[1], "B");
        assert_eq!(names[2], "C");
    }

    /// A cycle (A writes X / reads Y, B writes Y / reads X) must surface as
    /// a typed `Cycle` error rather than infinite-looping or panicking.
    #[test]
    fn cycle_is_detected() {
        let mut b = FrameGraph::builder();
        let x = b.create_texture(tex("x"));
        let y = b.create_texture(tex("y"));
        b.add_pass(Box::new(ClosurePass::new(
            "A",
            {
                let x = x;
                let y = y;
                move |pb| {
                    pb.write(x);
                    pb.read(y);
                }
            },
            |_| {},
        )));
        b.add_pass(Box::new(ClosurePass::new(
            "B",
            {
                let x = x;
                let y = y;
                move |pb| {
                    pb.read(x);
                    pb.write(y);
                }
            },
            |_| {},
        )));
        match b.compile() {
            Err(FrameGraphError::Cycle { passes }) => {
                assert_eq!(passes, vec![0, 1]);
            }
            Err(other) => panic!("expected Cycle, got {:?}", other),
            Ok(_) => panic!("expected Cycle, got Ok(FrameGraph)"),
        }
    }

    /// Two passes both write to the same resource without an explicit
    /// read-write chain → `ConflictingWriters` error rather than picking
    /// an arbitrary order.
    #[test]
    fn conflicting_writers_is_rejected() {
        let mut b = FrameGraph::builder();
        let x = b.create_texture(tex("x"));
        b.add_pass(Box::new(ClosurePass::new(
            "A",
            {
                let x = x;
                move |pb| pb.write(x)
            },
            |_| {},
        )));
        b.add_pass(Box::new(ClosurePass::new(
            "B",
            {
                let x = x;
                move |pb| pb.write(x)
            },
            |_| {},
        )));
        match b.compile() {
            Err(FrameGraphError::ConflictingWriters { writers, .. }) => {
                assert_eq!(writers, vec![0, 1]);
            }
            Err(other) => panic!("expected ConflictingWriters, got {:?}", other),
            Ok(_) => panic!("expected ConflictingWriters, got Ok(FrameGraph)"),
        }
    }

    /// Read-write usage on the same resource by a single pass is *not* a
    /// conflict (it's the canonical in-place blit / ping-pong shape) and
    /// must compile cleanly.
    #[test]
    fn read_write_on_single_pass_is_fine() {
        let mut b = FrameGraph::builder();
        let x = b.create_texture(tex("x"));
        b.add_pass(Box::new(ClosurePass::new(
            "blur",
            {
                let x = x;
                move |pb| pb.read_write(x)
            },
            |_| {},
        )));
        let g = b.compile().unwrap();
        assert_eq!(g.execution_order(), &[0]);
    }

    /// A resource declared but never read or written is allowed. Useful for
    /// passes that don't yet exist while a graph is being built up.
    #[test]
    fn dangling_resource_is_allowed() {
        let mut b = FrameGraph::builder();
        let _x = b.create_texture(tex("x"));
        b.add_pass(Box::new(NoopPass::new("noop")));
        let g = b.compile().unwrap();
        assert_eq!(g.resources().len(), 1);
        assert_eq!(g.execution_order(), &[0]);
    }

    /// Imported resources are visible via `imported_resource_indices` so
    /// backends that need to skip transient-allocation logic for them can
    /// distinguish at execute time.
    #[test]
    fn imported_resources_are_tracked() {
        let mut b = FrameGraph::builder();
        let _t = b.create_texture(tex("transient"));
        let _i = b.import_texture(tex("imported"));
        let g = b.compile().unwrap();
        assert_eq!(g.imported_resource_indices(), &[1]);
    }

    /// Reverse-declared producer-consumer pair sorts producer-first via
    /// topology, even though it was added second. The `record` side of
    /// this property is exercised by the textured_scene example (which
    /// can't run as a unit test because it requires a live HWND).
    #[test]
    fn topology_sorts_producer_before_consumer_independent_of_declaration_order() {
        let mut b = FrameGraph::builder();
        let x = b.create_texture(tex("x"));
        b.add_pass(Box::new(ClosurePass::new(
            "B",
            move |pb| pb.read(x),
            |_| {},
        )));
        b.add_pass(Box::new(ClosurePass::new(
            "A",
            move |pb| pb.write(x),
            |_| {},
        )));
        let g = b.compile().unwrap();
        let names: Vec<&str> = g
            .execution_order()
            .iter()
            .map(|&i| g.passes[i].name())
            .collect();
        assert_eq!(names, vec!["A", "B"]);
    }

    /// `read` against a fabricated id (one outside the graph's resource
    /// set) surfaces as `UnknownResource` rather than corrupting state.
    #[test]
    fn unknown_resource_id_is_rejected() {
        let mut b = FrameGraph::builder();
        b.add_pass(Box::new(ClosurePass::new(
            "rogue",
            move |pb| {
                let bogus = ResourceId {
                    index: 999,
                    kind: ResourceKind::Texture,
                };
                pb.read(bogus);
            },
            |_| {},
        )));
        match b.compile() {
            Err(FrameGraphError::UnknownResource { resource_index }) => {
                assert_eq!(resource_index, 999);
            }
            Err(other) => panic!("expected UnknownResource, got {:?}", other),
            Ok(_) => panic!("expected UnknownResource, got Ok(FrameGraph)"),
        }
    }
}
