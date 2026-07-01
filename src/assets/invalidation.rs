//! Asset invalidation events and broadcast bus.
//!
//! The `ReloadDispatcher` (M7.3) emits an [`AssetInvalidated`] event
//! through an [`InvalidationBus`] every time an asset is hot-reloaded.
//! Subscribers — the renderer's GPU caches, the editor inspector, the
//! ECS world's event bus, future caches added in later milestones —
//! react however the per-cache invalidation strategy demands (drop a
//! cached entry, refresh a preview, re-resolve an entity reference,
//! etc.).
//!
//! This module ships in M7.0 (pre-flight infrastructure) so the
//! integration seam exists before the file watcher and compile queue
//! land in M7.1–M7.2. M7.3 wires real subscribers; for now only the
//! event types and broadcast machinery are present, fully tested in
//! isolation.
//!
//! See `docs/adr/0004-hot-reload-architecture.md` Q10 + Q11.

use std::fmt;

/// Kinds of asset that can be hot-reloaded. Mirrors the typed-pin
/// asset hierarchy in `assets/types.rs`. Subscribers filter by kind
/// to avoid downcasting on every event.
///
/// New asset types added later (textures with mip chains, audio,
/// animations, ...) extend this enum. Match sites in subscriber code
/// will fail to compile until they handle the new kind, which is
/// exactly the discipline we want — no silent "this kind doesn't
/// have an eviction strategy" gaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetKind {
    Texture,
    NormalMap,
    Mesh,
    Shader,
    Material,
    Scene,
    MeshMapAO,
    MeshMapCurvature,
}

impl AssetKind {
    /// Stable string label for protocol encoding and editor display.
    /// The editor protocol codec (ADR 0004 Q13) uses these.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Texture => "Texture",
            Self::NormalMap => "NormalMap",
            Self::Mesh => "Mesh",
            Self::Shader => "Shader",
            Self::Material => "Material",
            Self::Scene => "Scene",
            Self::MeshMapAO => "MeshMapAO",
            Self::MeshMapCurvature => "MeshMapCurvature",
        }
    }
}

impl fmt::Display for AssetKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Emitted by the ReloadDispatcher after an asset has been replaced
/// in the live AssetStore.
///
/// `handle_index` is the asset pool's slot index (the same `index`
/// field on `Handle<T>`). Subscribers that key their caches by
/// handle (e.g. `frame_texture_cache: HashMap<TextureHandle, _>`)
/// reconstruct the handle from `(kind, handle_index)` and call
/// `cache.remove(&handle)`. Subscribers that key by something else
/// (`uniform_cache` keyed by GL program ID, `terrain_patch_mesh_cache`
/// keyed by `TerrainPatchMeshKey`) implement custom logic — see the
/// per-cache strategy table in ADR 0004 Q11.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetInvalidated {
    pub kind: AssetKind,
    pub handle_index: u32,
    /// Project-relative logical name of the asset, e.g.
    /// `textures/crate_albedo`. Useful for editor inspector display
    /// and error messages; not required for cache eviction.
    pub logical_name: String,
}

/// Opaque ID returned by [`InvalidationBus::subscribe`]. Pass it back
/// to [`InvalidationBus::unsubscribe`] to deregister.
///
/// IDs are not reused after unsubscribe; the bus's monotonic counter
/// guarantees every ID returned during the bus's lifetime is unique.
/// This makes "use after unsubscribe" a deterministically rejectable
/// case rather than a silent re-attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriberId(u64);

/// Synchronous broadcast of [`AssetInvalidated`] events to registered
/// subscribers.
///
/// Subscribers register a `Box<dyn FnMut(&AssetInvalidated)>` callback
/// and receive a [`SubscriberId`] for explicit deregistration. Events
/// are emitted via [`Self::emit`], which calls every active subscriber
/// in registration order.
///
/// Threading: the bus is not `Send` or `Sync` — callbacks may be
/// `!Send` (e.g. a closure capturing `Rc<RefCell<...>>`). The
/// ReloadDispatcher (M7.3) lives on the main thread and emits events
/// at frame boundaries, so single-threaded broadcast is the right
/// shape for v1. A future M8+ thread-pool dispatcher can introduce a
/// separate `ConcurrentInvalidationBus` if the need arises.
///
/// Failure modes:
/// - Subscriber callbacks that panic poison the bus for that emit
///   call. Surrounding emits continue normally; the panicked
///   subscriber stays registered (we don't auto-deregister on panic
///   because that creates ordering surprises). Callbacks should be
///   defensive and surface their own errors.
/// - `unsubscribe` on an unknown or already-unsubscribed ID returns
///   `false` rather than panicking. Idempotent.
pub struct InvalidationBus {
    next_id: u64,
    subscribers: Vec<(SubscriberId, Box<dyn FnMut(&AssetInvalidated)>)>,
}

impl Default for InvalidationBus {
    fn default() -> Self {
        Self::new()
    }
}

impl InvalidationBus {
    /// Construct an empty bus.
    pub fn new() -> Self {
        Self {
            next_id: 0,
            subscribers: Vec::new(),
        }
    }

    /// Register a callback. Returns the [`SubscriberId`] used to later
    /// deregister.
    pub fn subscribe<F>(&mut self, f: F) -> SubscriberId
    where
        F: FnMut(&AssetInvalidated) + 'static,
    {
        let id = SubscriberId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.subscribers.push((id, Box::new(f)));
        id
    }

    /// Deregister the callback identified by `id`. Returns `true` if a
    /// subscriber was removed, `false` if `id` was unknown or already
    /// removed (idempotent).
    pub fn unsubscribe(&mut self, id: SubscriberId) -> bool {
        let before = self.subscribers.len();
        self.subscribers.retain(|(sid, _)| *sid != id);
        before != self.subscribers.len()
    }

    /// Broadcast `event` to every registered subscriber, in
    /// registration order. The borrow of the event is shared across
    /// callbacks; each callback gets `&AssetInvalidated`.
    pub fn emit(&mut self, event: &AssetInvalidated) {
        for (_, callback) in &mut self.subscribers {
            callback(event);
        }
    }

    /// Number of currently registered subscribers. Useful for tests
    /// and editor diagnostics ("3 caches listening for reloads").
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn sample_event(kind: AssetKind, idx: u32) -> AssetInvalidated {
        AssetInvalidated {
            kind,
            handle_index: idx,
            logical_name: format!("logical/asset_{}", idx),
        }
    }

    /// A fresh bus has zero subscribers and `emit` is a no-op.
    /// The hot-reload pipeline must pay zero cost when no caches
    /// have wired up listeners (e.g. headless tests, server runs).
    #[test]
    fn fresh_bus_has_no_subscribers_and_emit_is_noop() {
        let mut bus = InvalidationBus::new();
        assert_eq!(bus.subscriber_count(), 0);
        bus.emit(&sample_event(AssetKind::Texture, 0));
        // Survived without panicking.
    }

    /// Subscribe stores the callback and returns a unique ID. Each
    /// subscribe gets a fresh ID; IDs are not recycled after
    /// unsubscribe (verified in `subscriber_ids_are_never_recycled`).
    #[test]
    fn subscribe_registers_callback() {
        let mut bus = InvalidationBus::new();
        let calls = Rc::new(RefCell::new(0u32));
        let calls_for_cb = Rc::clone(&calls);

        let id = bus.subscribe(move |_| *calls_for_cb.borrow_mut() += 1);
        assert_eq!(bus.subscriber_count(), 1);

        bus.emit(&sample_event(AssetKind::Texture, 0));
        bus.emit(&sample_event(AssetKind::Texture, 1));
        assert_eq!(*calls.borrow(), 2);
        // ID is unused beyond identification — just verify the type.
        let _ = id;
    }

    /// Multiple subscribers see every event in registration order.
    /// Order matters because some subscribers depend on others having
    /// updated (e.g. uniform_cache cascade depends on
    /// frame_program_cache having dropped first).
    #[test]
    fn multiple_subscribers_see_every_event_in_order() {
        let mut bus = InvalidationBus::new();
        let trace: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

        let trace_a = Rc::clone(&trace);
        bus.subscribe(move |ev| {
            trace_a
                .borrow_mut()
                .push(format!("A:{}/{}", ev.kind, ev.handle_index));
        });
        let trace_b = Rc::clone(&trace);
        bus.subscribe(move |ev| {
            trace_b
                .borrow_mut()
                .push(format!("B:{}/{}", ev.kind, ev.handle_index));
        });

        bus.emit(&sample_event(AssetKind::Texture, 7));
        bus.emit(&sample_event(AssetKind::Mesh, 3));

        assert_eq!(
            *trace.borrow(),
            vec![
                "A:Texture/7".to_string(),
                "B:Texture/7".to_string(),
                "A:Mesh/3".to_string(),
                "B:Mesh/3".to_string(),
            ]
        );
    }

    /// Unsubscribe by ID removes exactly that subscriber. Other
    /// subscribers continue to receive events. This is the property
    /// the editor inspector relies on — a panel close removes its
    /// listener without affecting GPU caches.
    #[test]
    fn unsubscribe_removes_only_the_named_subscriber() {
        let mut bus = InvalidationBus::new();
        let kept = Rc::new(RefCell::new(0u32));
        let removed = Rc::new(RefCell::new(0u32));

        let kept_for_cb = Rc::clone(&kept);
        bus.subscribe(move |_| *kept_for_cb.borrow_mut() += 1);
        let removed_for_cb = Rc::clone(&removed);
        let removed_id = bus.subscribe(move |_| *removed_for_cb.borrow_mut() += 1);

        bus.emit(&sample_event(AssetKind::Texture, 0));
        assert_eq!(*kept.borrow(), 1);
        assert_eq!(*removed.borrow(), 1);

        let was_present = bus.unsubscribe(removed_id);
        assert!(was_present);
        assert_eq!(bus.subscriber_count(), 1);

        bus.emit(&sample_event(AssetKind::Texture, 1));
        assert_eq!(*kept.borrow(), 2);
        assert_eq!(*removed.borrow(), 1); // not invoked after unsubscribe
    }

    /// Unsubscribe with an unknown ID returns false rather than
    /// panicking. Calling unsubscribe twice on the same ID is
    /// idempotent — the second call returns false.
    #[test]
    fn unsubscribe_unknown_id_is_idempotent() {
        let mut bus = InvalidationBus::new();
        let id = bus.subscribe(|_| {});
        assert!(bus.unsubscribe(id));
        assert!(!bus.unsubscribe(id)); // second call: nothing to remove
        // A fabricated unrelated ID is also rejected.
        let bogus = SubscriberId(9_999);
        assert!(!bus.unsubscribe(bogus));
    }

    /// Subscriber IDs are never recycled. After `unsubscribe + subscribe`,
    /// the new subscriber gets a fresh ID, not the recycled one. This
    /// makes "stale ID after unsubscribe" a deterministically rejectable
    /// state and prevents accidental re-attachment to a different
    /// callback.
    #[test]
    fn subscriber_ids_are_never_recycled() {
        let mut bus = InvalidationBus::new();
        let first = bus.subscribe(|_| {});
        bus.unsubscribe(first);
        let second = bus.subscribe(|_| {});
        assert_ne!(first, second);
    }

    /// AssetKind round-trips through its string label. The editor
    /// protocol codec (ADR 0004 Q13) depends on this.
    #[test]
    fn asset_kind_str_labels_are_distinct() {
        let kinds = [
            AssetKind::Texture,
            AssetKind::NormalMap,
            AssetKind::Mesh,
            AssetKind::Shader,
            AssetKind::Material,
            AssetKind::Scene,
            AssetKind::MeshMapAO,
            AssetKind::MeshMapCurvature,
        ];
        let mut labels: Vec<&str> = kinds.iter().map(|k| k.as_str()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), kinds.len(), "AssetKind labels must be unique");
    }
}
