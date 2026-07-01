//! Typed resource descriptors for the frame graph.
//!
//! The frame graph operates on *transient* resources (textures and buffers
//! whose lifetime spans only one frame) and *imported* resources (the
//! swap-chain image, persistent shadow maps, etc.). Both kinds are described
//! through [`ResourceDesc`]; the graph distinguishes them at the builder
//! level via separate `create_*` and `import_*` methods.
//!
//! Every resource carries a [`ResourceKind`] tag at runtime so edges can be
//! type-checked: a pass that declares a `Texture` write into a `Buffer`
//! reader is rejected at compile time. This is the typed-pin discipline from
//! ADR 0002 Q8 enforced through a runtime tag rather than `PhantomData<T>` —
//! it lets a single graph hold mixed resource types without the API explosion
//! of generics.
//!
//! Resource lifetimes, aliasing, and explicit barrier transitions are out of
//! scope for the v1 frame graph (see ADR 0002 Q8 phasing). The OpenGL backend
//! does not need barrier tracking; the Vulkan backend will reintroduce that
//! layer when it lands.

use crate::assets::TextureFormat;

/// What a texture is being used for. Drives the OpenGL/Vulkan creation flags
/// (sampler vs render target vs storage image) at backend integration time.
///
/// The variants are deliberately a small fixed set rather than a bitflag —
/// most graph passes use exactly one role, and conflating them in v1 makes
/// the type-checking layer harder for no win. A future revision can collapse
/// these into bitflags if multi-role usage becomes common (e.g. read-write
/// storage images for compute passes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureUsage {
    /// Bound as a colour attachment of a render pass.
    ColorAttachment,
    /// Bound as the depth attachment of a render pass.
    DepthAttachment,
    /// Sampled by a fragment shader.
    Sampled,
    /// General storage image (compute or write-from-fragment-shader).
    Storage,
}

/// What a buffer is being used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferUsage {
    /// Indirect-draw / dispatch arguments.
    Indirect,
    /// Uniform block bound via UBO.
    Uniform,
    /// Storage block bound via SSBO.
    Storage,
    /// Vertex or index buffer source.
    Vertex,
    Index,
}

/// Descriptor for a transient or imported texture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureDesc {
    pub name: String,
    pub format: TextureFormat,
    pub width: u32,
    pub height: u32,
    pub usage: TextureUsage,
}

/// Descriptor for a transient or imported buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferDesc {
    pub name: String,
    pub size_bytes: u32,
    pub usage: BufferUsage,
}

/// One of the two resource flavours the frame graph knows about. New
/// flavours (e.g. samplers, acceleration structures) extend this enum
/// rather than parallel-typing the rest of the API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceDesc {
    Texture(TextureDesc),
    Buffer(BufferDesc),
}

impl ResourceDesc {
    /// Compact tag used by edge-type checking. Faster than matching on the
    /// variant and avoids accidental dependency on inner-field equality.
    pub fn kind(&self) -> ResourceKind {
        match self {
            ResourceDesc::Texture(_) => ResourceKind::Texture,
            ResourceDesc::Buffer(_) => ResourceKind::Buffer,
        }
    }

    /// Human-readable name from the underlying descriptor — surfaced in
    /// error messages and (eventually) in the editor's frame inspector.
    pub fn name(&self) -> &str {
        match self {
            ResourceDesc::Texture(d) => &d.name,
            ResourceDesc::Buffer(d) => &d.name,
        }
    }
}

/// Type tag carried by every [`ResourceId`] for fast edge validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Texture,
    Buffer,
}

/// Opaque, typed handle into a frame graph's resource set.
///
/// The `index` is the slot in the graph's resource vector. The `kind` is
/// duplicated here (rather than looked up via the index) so type checks at
/// edge declaration sites don't need a back-pointer to the owning builder.
/// This makes the typical "pass declares a read/write" call cheap and lets
/// the type-mismatch error surface at the moment of declaration, not later.
///
/// Equality is structural on `(index, kind)`; two ids from different graph
/// builders are distinct even if their indices match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResourceId {
    pub(super) index: u32,
    pub(super) kind: ResourceKind,
}

impl ResourceId {
    pub fn kind(&self) -> ResourceKind {
        self.kind
    }
}

/// How a pass touches a resource.
///
/// `Read` and `Write` form the dependency graph: pass B depends on pass A
/// iff A writes (or read-writes) any resource that B reads (or read-writes).
/// `ReadWrite` participates in *both* sides — a pass that ping-pongs a
/// texture reads and writes the same resource — and is treated by the
/// dependency calculator as `Write` for outgoing edges and `Read` for
/// incoming edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    Read,
    Write,
    ReadWrite,
}
