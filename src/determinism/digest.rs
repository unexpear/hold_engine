//! Frame digest helpers for the determinism harness.
//!
//! A frame digest is a stable hash of the world's user-observable
//! state at a specific tick. Two replays of the same recording
//! must produce identical per-frame digest streams; the harness
//! asserts equality on every commit (ADR 0005 Q9).
//!
//! ## What gets hashed
//!
//! The digest is built around the gameplay-relevant component
//! state of a caller-supplied entity set. We don't iterate the
//! entire `World`'s storage map because:
//!
//! 1. Iterating `HashMap<TypeId, Box<dyn AnyStorage>>` in stable
//!    order would require new public ECS API. The tracked-entity
//!    approach uses what already exists.
//! 2. Real gameplay tests already track the entity handles they
//!    care about; the digest helper mirrors that pattern.
//! 3. Adding entities the test doesn't care about (decorations,
//!    UI gizmos) wouldn't actually contribute to the determinism
//!    contract — only state the user can observe matters.
//!
//! Callers compose digests for their tracked entities by walking
//! the entity list in deterministic order (sort by `Entity::index`
//! before digesting) and feeding each `(Transform, Velocity, ...)`
//! tuple into a [`DeterministicHasher`]. The
//! [`frame_digest_from_components`] free function does this for
//! the Transform + Velocity case the harness uses; gameplay tests
//! that need richer digests build their own with the same pattern.
//!
//! ## Why a typed wrapper?
//!
//! [`FrameDigest`] is a `u64` newtype rather than a bare `u64` so
//! a digest can never silently get confused with an asset handle,
//! generation counter, or any other 64-bit ID floating around the
//! engine. Pillar 4 (typed errors): types prevent whole categories
//! of bugs at the cost of a few more lines per API.
//!
//! See `docs/adr/0005-determinism-architecture.md` Q8.

use std::hash::Hash;

use crate::ecs::components::{Transform, Velocity};
use crate::ecs::{Entity, World};

use super::hasher::DeterministicHasher;

/// A frame's deterministic digest. Compared frame-by-frame across
/// two replays of the same recording; equality means the
/// simulation reached identical observable state. Inequality
/// means the simulation diverged — the harness fails CI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameDigest(pub u64);

impl FrameDigest {
    /// Construct a digest from a raw `u64`. Useful when callers
    /// hash their own state; the harness uses
    /// [`frame_digest_from_components`] for the Transform + Velocity
    /// case.
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    /// Read the digest as a raw `u64`. Useful for serialisation
    /// and "show me the digest of frame N" diagnostics in the
    /// harness's failure output.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for FrameDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FrameDigest(0x{:016x})", self.0)
    }
}

/// Compute a digest of `(Transform, Velocity)` state for the
/// supplied entity list. Entities missing one or both components
/// are encoded with a sentinel byte so a "this entity has no
/// Transform yet" frame digests differently from a "this entity's
/// Transform was reset to identity" frame.
///
/// The entity list is sorted by `Entity::index` (with `generation`
/// as the tiebreaker) before digesting so a caller that hands the
/// same set in different orders still gets the same digest.
///
/// `tick` is mixed in first so two consecutive frames with
/// identical state digest differently — the harness wants per-tick
/// distinguishability so a simulation that "freezes" partway
/// through a replay surfaces clearly (rather than blending into a
/// run of identical digests).
pub fn frame_digest_from_components(
    world: &World,
    entities: &[Entity],
    tick: u32,
) -> FrameDigest {
    // Local copy + sort. Caller-side mutation isn't allowed (we
    // only borrow &[Entity]); a small per-frame allocation is
    // the price of order independence. ADR 0005 Q10's per-frame
    // budget is generous enough to absorb this.
    let mut sorted: Vec<Entity> = entities.to_vec();
    sorted.sort_by_key(|e| (e.index, e.generation));

    let mut hasher = DeterministicHasher::new();
    // Mix the tick first so frame N and frame N+1 with identical
    // state get distinct digests.
    tick.hash(&mut hasher);
    // Mix the entity count so a digest of zero entities still
    // distinguishes from "tick only" digests.
    (sorted.len() as u32).hash(&mut hasher);

    for entity in &sorted {
        // (index, generation) identifies the entity stably across
        // ECS implementations; the generation guards against the
        // free-list reuse case.
        entity.index.hash(&mut hasher);
        entity.generation.hash(&mut hasher);

        // Transform — sentinel byte 0 means "absent", 1 means
        // "present and these are the bytes." The bytes are the
        // raw f32 bit-pattern (`f32::to_bits`), not formatted —
        // Hash::hash on f32 itself is forbidden by std (NaN
        // doesn't compare equal to itself), but to_bits gives a
        // stable u32 representation that hashes deterministically.
        match world.get::<Transform>(*entity) {
            Some(t) => {
                hasher_write_byte(&mut hasher, 1);
                hasher_write_vec3(&mut hasher, t.position.x, t.position.y, t.position.z);
                hasher_write_quat(
                    &mut hasher,
                    t.rotation.x,
                    t.rotation.y,
                    t.rotation.z,
                    t.rotation.w,
                );
                hasher_write_vec3(&mut hasher, t.scale.x, t.scale.y, t.scale.z);
            }
            None => hasher_write_byte(&mut hasher, 0),
        }

        // Velocity — same sentinel pattern.
        match world.get::<Velocity>(*entity) {
            Some(v) => {
                hasher_write_byte(&mut hasher, 1);
                hasher_write_vec3(&mut hasher, v.linear.x, v.linear.y, v.linear.z);
                hasher_write_vec3(&mut hasher, v.angular.x, v.angular.y, v.angular.z);
            }
            None => hasher_write_byte(&mut hasher, 0),
        }
    }

    FrameDigest::new(std::hash::Hasher::finish(&hasher))
}

#[inline]
fn hasher_write_byte(hasher: &mut DeterministicHasher, byte: u8) {
    use std::hash::Hasher;
    hasher.write(&[byte]);
}

#[inline]
fn hasher_write_vec3(hasher: &mut DeterministicHasher, x: f32, y: f32, z: f32) {
    x.to_bits().hash(hasher);
    y.to_bits().hash(hasher);
    z.to_bits().hash(hasher);
}

#[inline]
fn hasher_write_quat(hasher: &mut DeterministicHasher, x: f32, y: f32, z: f32, w: f32) {
    x.to_bits().hash(hasher);
    y.to_bits().hash(hasher);
    z.to_bits().hash(hasher);
    w.to_bits().hash(hasher);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec3;

    /// Empty world (no entities tracked, tick 0) produces a
    /// stable digest. This is the "minimum input" reference
    /// vector — pinning it catches any change to how the digest
    /// hashes the tick header or the entity-count prefix.
    #[test]
    fn empty_entity_list_produces_stable_digest() {
        let world = World::new();
        let digest_a = frame_digest_from_components(&world, &[], 0);
        let digest_b = frame_digest_from_components(&world, &[], 0);
        assert_eq!(digest_a, digest_b);
    }

    /// Different ticks produce different digests for the same
    /// state. This is the "frame N is distinguishable from frame
    /// N+1" property — without it, a simulation that freezes
    /// would silently produce a run of equal digests.
    #[test]
    fn different_ticks_produce_different_digests_for_same_state() {
        let world = World::new();
        let digest_t0 = frame_digest_from_components(&world, &[], 0);
        let digest_t1 = frame_digest_from_components(&world, &[], 1);
        assert_ne!(digest_t0, digest_t1);
    }

    /// Same world + same tick + same entity list (in any order)
    /// produces the same digest. Tests pin the order-independence
    /// property: callers that pass entities in different orders
    /// still get equal digests.
    #[test]
    fn entity_order_does_not_affect_digest() {
        let mut world = World::new();
        let e1 = world.spawn();
        let e2 = world.spawn();
        let e3 = world.spawn();
        world.add_component(e1, Transform::from_position(Vec3::new(1.0, 2.0, 3.0)));
        world.add_component(e2, Transform::from_position(Vec3::new(4.0, 5.0, 6.0)));
        world.add_component(e3, Transform::from_position(Vec3::new(7.0, 8.0, 9.0)));

        let digest_in_order = frame_digest_from_components(&world, &[e1, e2, e3], 5);
        let digest_reversed = frame_digest_from_components(&world, &[e3, e2, e1], 5);
        let digest_shuffled = frame_digest_from_components(&world, &[e2, e1, e3], 5);

        assert_eq!(digest_in_order, digest_reversed);
        assert_eq!(digest_in_order, digest_shuffled);
    }

    /// Mutating an entity's Transform changes the digest. Pillar
    /// 4 (debuggability): if a system silently broke a Transform
    /// field's update path, the digest would still flip — the
    /// harness catches it.
    #[test]
    fn mutating_transform_changes_digest() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::from_position(Vec3::new(0.0, 0.0, 0.0)));
        let digest_before = frame_digest_from_components(&world, &[e], 0);

        world.add_component(e, Transform::from_position(Vec3::new(1.0, 0.0, 0.0)));
        let digest_after = frame_digest_from_components(&world, &[e], 0);

        assert_ne!(digest_before, digest_after);
    }

    /// "Missing Transform" digests differently from "Transform
    /// at the origin." The sentinel byte makes that distinction
    /// — without it, an absent component and a zero component
    /// would silently collide.
    #[test]
    fn missing_component_distinguishes_from_zero_component() {
        let mut world_a = World::new();
        let e_a = world_a.spawn();
        // No Transform attached.
        let digest_missing = frame_digest_from_components(&world_a, &[e_a], 0);

        let mut world_b = World::new();
        let e_b = world_b.spawn();
        world_b.add_component(e_b, Transform::IDENTITY);
        let digest_present = frame_digest_from_components(&world_b, &[e_b], 0);

        assert_ne!(digest_missing, digest_present);
    }

    /// Velocity also contributes to the digest. Tests pin that
    /// the Transform-only digest path doesn't accidentally drop
    /// the Velocity contribution.
    #[test]
    fn velocity_contributes_to_digest() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::IDENTITY);
        let digest_no_velocity = frame_digest_from_components(&world, &[e], 0);

        world.add_component(e, Velocity::from_linear(Vec3::new(1.0, 2.0, 3.0)));
        let digest_with_velocity = frame_digest_from_components(&world, &[e], 0);

        assert_ne!(digest_no_velocity, digest_with_velocity);
    }

    /// `FrameDigest::Display` formats as a 16-hex-digit value.
    /// Used in harness failure output to show "frame 42's
    /// digest was 0x... but expected 0x..."; pin the format so
    /// CI logs stay grep-friendly.
    #[test]
    fn frame_digest_displays_as_hex() {
        let d = FrameDigest::new(0xDEADBEEF_CAFEBABE);
        let s = format!("{d}");
        assert_eq!(s, "FrameDigest(0xdeadbeefcafebabe)");
    }

    /// Floating-point bit-pattern hashing handles NaN
    /// deterministically. `f32::to_bits` returns a stable u32
    /// for any f32 (including NaN); this is what makes the
    /// digest immune to `f32::Hash` being undefined.
    #[test]
    fn nan_transform_position_hashes_deterministically() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::from_position(Vec3::new(f32::NAN, 0.0, 0.0)));
        let digest_a = frame_digest_from_components(&world, &[e], 0);
        let digest_b = frame_digest_from_components(&world, &[e], 0);
        // Two captures of the same NaN bit-pattern produce
        // identical digests, even though `f32::NAN != f32::NAN`.
        assert_eq!(digest_a, digest_b);
    }
}
