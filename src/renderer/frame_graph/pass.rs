//! [`Pass`] trait, builder API, and the per-pass execution context.
//!
//! A pass is the unit of work in the frame graph. It declares which
//! resources it reads, writes, and creates; the graph computes the
//! dependency edges from those declarations, topologically sorts the passes,
//! and calls [`Pass::record`] in order.
//!
//! See ADR 0002 Q8 for the design rationale.

use super::resource::{AccessKind, ResourceId, ResourceKind};
use crate::assets::AssetStore;
use crate::renderer::opengl::OpenGLRenderer;
use crate::renderer::RenderFrame;

/// Errors a pass can raise from [`Pass::declare`] (via the builder) or that
/// the frame graph can raise during compile.
///
/// All variants carry enough context to be useful in messages without
/// requiring the caller to consult the graph's internal state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameGraphError {
    /// A pass declared a read/write of a [`ResourceId`] that does not exist
    /// in the owning graph (e.g. an id from a different builder, or an id
    /// fabricated outside `create_*` / `import_*`).
    UnknownResource { resource_index: u32 },

    /// A pass tried to use a resource at an [`AccessKind`] incompatible with
    /// the resource's [`ResourceKind`]. Currently this only triggers via
    /// API misuse (the typed accessors prevent the usual cross-kind
    /// mistake), but the variant exists so downstream backends can extend
    /// the typing without breaking the error type.
    TypeMismatch {
        resource: ResourceId,
        expected: ResourceKind,
        found: ResourceKind,
    },

    /// The compiled graph contains a cycle (resource X is written by pass A
    /// and read by pass B which is also written by pass A's reader chain).
    /// Carries the list of pass indices in the offending cycle, in the
    /// order discovered.
    Cycle { passes: Vec<usize> },

    /// Two passes write to the same resource without an explicit
    /// `read_write` declaration making the ping-pong intent visible. The v1
    /// graph treats this as ambiguous rather than picking an order.
    ConflictingWriters {
        resource: ResourceId,
        writers: Vec<usize>,
    },
}

impl std::fmt::Display for FrameGraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownResource { resource_index } => write!(
                f,
                "frame graph: pass referenced unknown resource index {resource_index}"
            ),
            Self::TypeMismatch {
                resource,
                expected,
                found,
            } => write!(
                f,
                "frame graph: resource {:?} is a {:?}, used as {:?}",
                resource, found, expected
            ),
            Self::Cycle { passes } => write!(
                f,
                "frame graph: cycle detected through passes {:?}",
                passes
            ),
            Self::ConflictingWriters { resource, writers } => write!(
                f,
                "frame graph: resource {:?} is written by multiple passes {:?} \
                 without an explicit read_write chain",
                resource, writers
            ),
        }
    }
}

impl std::error::Error for FrameGraphError {}

/// Trait every render pass implements.
///
/// The two methods are deliberately split:
/// - [`Pass::declare`] runs at *compile* time, once. It is given the builder
///   and must register the pass's resource accesses. It does not draw.
/// - [`Pass::record`] runs at *execute* time, once per frame. It receives a
///   [`PassContext`] and emits draw calls.
///
/// Splitting them lets the graph compute edges and execution order *before*
/// any GPU work happens, which is the precondition for all the things that
/// make a frame graph useful: dependency tracking, transient resource
/// allocation, and per-pass thumbnail capture.
pub trait Pass {
    /// Stable, human-readable name for the pass — used in dependency error
    /// messages and (eventually) in the editor frame inspector.
    fn name(&self) -> &str;

    /// Register this pass's resource accesses. Called once during
    /// `FrameGraph::compile`. Implementations call `builder.read`,
    /// `builder.write`, etc. on the [`PassBuilder`].
    fn declare(&self, builder: &mut PassBuilder);

    /// Emit GPU work for this pass. Called once per frame after the graph
    /// has been compiled, in topological order. The current v1
    /// [`PassContext`] is a placeholder — concrete backend plumbing arrives
    /// when the OpenGL backend is wired into the graph (ADR 0002 Q8 phasing).
    fn record(&self, ctx: &mut PassContext);
}

/// Information collected by a [`Pass`] during [`Pass::declare`].
///
/// The builder is consumed by the graph compiler after `declare` returns; it
/// does not survive into `record`. Errors raised here are reported through
/// `FrameGraph::compile`, not as panics.
pub struct PassBuilder {
    /// Index of the pass currently declaring its accesses. Held so the
    /// editor frame inspector and future backend integration can correlate
    /// builder errors with the originating pass; v1 doesn't read it during
    /// compile, so it carries `dead_code`-allow.
    #[allow(dead_code)]
    pub(super) pass_index: usize,
    pub(super) resource_count: u32,
    pub(super) accesses: Vec<ResourceAccess>,
    pub(super) errors: Vec<FrameGraphError>,
}

/// One access entry produced by a pass's `declare`. Materialised into a
/// dependency edge during compile.
#[derive(Debug, Clone, Copy)]
pub(super) struct ResourceAccess {
    pub resource: ResourceId,
    pub access: AccessKind,
}

impl PassBuilder {
    pub(super) fn new(pass_index: usize, resource_count: u32) -> Self {
        Self {
            pass_index,
            resource_count,
            accesses: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Declare that this pass reads from `id`. The graph will mark this
    /// pass as depending on whatever pass last wrote to `id`.
    pub fn read(&mut self, id: ResourceId) {
        if !self.validate(id) {
            return;
        }
        self.accesses.push(ResourceAccess {
            resource: id,
            access: AccessKind::Read,
        });
    }

    /// Declare that this pass writes to `id`. The graph will treat the
    /// resource as produced by this pass; downstream readers depend on it.
    pub fn write(&mut self, id: ResourceId) {
        if !self.validate(id) {
            return;
        }
        self.accesses.push(ResourceAccess {
            resource: id,
            access: AccessKind::Write,
        });
    }

    /// Declare that this pass both reads and writes `id` in place
    /// (ping-pong, blit-into-self, in-place blur). Acts as a `Read` for
    /// incoming dependency edges and a `Write` for outgoing ones.
    pub fn read_write(&mut self, id: ResourceId) {
        if !self.validate(id) {
            return;
        }
        self.accesses.push(ResourceAccess {
            resource: id,
            access: AccessKind::ReadWrite,
        });
    }

    fn validate(&mut self, id: ResourceId) -> bool {
        if id.index >= self.resource_count {
            self.errors.push(FrameGraphError::UnknownResource {
                resource_index: id.index,
            });
            return false;
        }
        true
    }
}

/// Per-pass execution context.
///
/// Carries the OpenGL renderer reference, the asset store, and the
/// current [`RenderFrame`] the pass needs to record GPU work, plus
/// metadata (pass name and index) for debugging and the editor frame
/// inspector.
///
/// The frame graph is bound to the OpenGL backend at this layer
/// deliberately. Per the M4 phasing in ADR 0002 Q8, OpenGL is the only
/// "working-minimal" backend; binding here avoids a generic abstraction
/// designed against a single concrete user. When the Vulkan backend lands
/// it will get its own parallel `frame_graph_vulkan` module with the same
/// topology core but a Vulkan-specific `PassContext`.
///
/// The `frame` field is the scene description the editor and runtime
/// build once per frame (M9: real scene render through the frame
/// graph). Passes read views and draw batches from it rather than
/// caching per-frame data inside themselves — the graph is
/// constructed once at editor startup and drives whatever `frame`
/// the shell hands to `execute` / `execute_with_capture`.
pub struct PassContext<'a> {
    pub pass_name: &'a str,
    pub pass_index: usize,
    pub renderer: &'a mut OpenGLRenderer,
    pub assets: &'a AssetStore,
    pub frame: &'a RenderFrame,
}

impl<'a> PassContext<'a> {
    pub(super) fn new(
        pass_name: &'a str,
        pass_index: usize,
        renderer: &'a mut OpenGLRenderer,
        assets: &'a AssetStore,
        frame: &'a RenderFrame,
    ) -> Self {
        Self {
            pass_name,
            pass_index,
            renderer,
            assets,
            frame,
        }
    }
}

/// Convenience [`Pass`] used by tests and as a placeholder during graph
/// construction. Records a name and declares zero accesses.
pub struct NoopPass {
    pub name: String,
}

impl NoopPass {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl Pass for NoopPass {
    fn name(&self) -> &str {
        &self.name
    }
    fn declare(&self, _builder: &mut PassBuilder) {}
    fn record(&self, _ctx: &mut PassContext) {}
}

/// A `Pass` implementation that wraps a closure for `declare` and a closure
/// for `record`. Useful in tests and for tiny passes that don't need their
/// own struct. Both closures are `Box<dyn Fn>` so they can be moved into the
/// graph and called later without `&mut self` reentrancy.
///
/// The closures are not constrained to `Send + Sync`. The frame graph
/// executes passes sequentially on the calling thread; demanding thread
/// safety here would gratuitously rule out the common case of passes that
/// share a non-`Sync` cell with surrounding test or game-loop code (e.g. an
/// `Rc<RefCell<_>>` capturing a counter or trace log).
pub struct ClosurePass {
    pub name: String,
    declare_fn: Box<dyn Fn(&mut PassBuilder)>,
    record_fn: Box<dyn Fn(&mut PassContext)>,
}

impl ClosurePass {
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

impl Pass for ClosurePass {
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

