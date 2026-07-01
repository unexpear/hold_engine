//! Fixed-seed deterministic hasher for frame digests.
//!
//! Rust's standard library `Hasher` implementations
//! (`DefaultHasher`, `SipHasher`) randomize their seed at process
//! start as a HashDoS mitigation. That's the right default for
//! `HashMap`, but it makes any digest computed through them
//! non-deterministic across runs — exactly the property the
//! determinism harness needs.
//!
//! This module ships an in-house FNV-1a 64-bit hasher with a
//! fixed seed. Zero external crates per the project charter; ~50
//! lines of code. FNV-1a is chosen because it's:
//!
//! 1. **Deterministic by construction.** No internal randomness;
//!    the same byte sequence always produces the same hash.
//! 2. **Fast, scalar, branch-light.** Single-cycle multiply-xor
//!    per byte. No SIMD/AVX — runs identically on i3 and i9 (ADR
//!    0005 Q10 hardware-portability bound).
//! 3. **Trivially auditable.** ~10 lines of math, fully covered
//!    by published reference vectors (Pillar 5: every byte of
//!    the engine is auditable).
//!
//! FNV-1a is **not** cryptographic. Don't use it for HashDoS
//! mitigation, content-addressed storage, or anything an
//! adversary controls. For frame digests in CI tests, where
//! collisions are tested against published reference outputs,
//! it's the right choice.
//!
//! See `docs/adr/0005-determinism-architecture.md` Q8 for the
//! frame-digest algorithm that uses this hasher.

use std::hash::Hasher;

/// FNV-1a 64-bit prime constant. From the FNV reference
/// (`http://www.isthe.com/chongo/tech/comp/fnv/`).
const FNV_64_PRIME: u64 = 0x100000001b3;
/// FNV-1a 64-bit offset basis. The fixed seed that makes the
/// hasher deterministic across runs.
const FNV_64_OFFSET_BASIS: u64 = 0xcbf29ce484222325;

/// In-house FNV-1a 64-bit hasher. Implements [`std::hash::Hasher`]
/// so types with a `Hash` impl can feed bytes through it via the
/// usual `T::hash(&value, &mut hasher)` API.
///
/// The seed is fixed at [`FNV_64_OFFSET_BASIS`]. Two hashers
/// constructed at different points in the same process — or in
/// different processes, on different machines — produce the same
/// output for the same input bytes.
///
/// Cost: one multiply + one xor per input byte. ~1ns per byte on
/// modern x86_64. The frame-digest path hashes O(world_size)
/// bytes and runs once per frame in the harness (not on the
/// gameplay-hot path), so even a busy world digests in single
/// microseconds (ADR 0005 Q10).
#[derive(Debug, Clone)]
pub struct DeterministicHasher {
    state: u64,
}

impl DeterministicHasher {
    /// Construct a fresh hasher with the FNV-1a offset basis as
    /// its seed.
    pub fn new() -> Self {
        Self {
            state: FNV_64_OFFSET_BASIS,
        }
    }

    /// Construct a hasher with a custom seed. Useful for the
    /// frame-digest helper that mixes a per-frame nonce into the
    /// hash so two consecutive identical-state frames produce
    /// distinct digests (which is what tests want — frame N's
    /// digest is "frame N's state at tick N," not "this state
    /// hash, whenever").
    pub fn with_seed(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Mix a single byte into the hash state. Inlined because
    /// the per-byte mixing is the inner loop of every hash
    /// computation; the overhead of a non-inlined call would
    /// dominate the work.
    #[inline]
    fn mix_byte(&mut self, byte: u8) {
        self.state ^= byte as u64;
        self.state = self.state.wrapping_mul(FNV_64_PRIME);
    }
}

impl Default for DeterministicHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Hasher for DeterministicHasher {
    /// Final hash value. Doesn't reset the hasher state — calling
    /// `finish` twice in a row returns the same value, as the
    /// `Hasher` contract requires.
    fn finish(&self) -> u64 {
        self.state
    }

    /// Mix a byte slice into the hash state. The standard library
    /// only requires this method; per-type `Hash` impls call it
    /// (sometimes via `write_u32`, `write_u64`, etc., which the
    /// default impls route back through `write`).
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.mix_byte(byte);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::Hash;

    /// Empty input produces the FNV-1a offset basis. Reference
    /// vector from the canonical FNV spec — pinning this catches
    /// accidental constant changes.
    #[test]
    fn empty_input_yields_fnv_offset_basis() {
        let hasher = DeterministicHasher::new();
        assert_eq!(hasher.finish(), FNV_64_OFFSET_BASIS);
    }

    /// FNV-1a reference vector: hashing the single byte `'a'`
    /// (0x61) produces a known, published value. From the FNV
    /// reference test cases:
    ///   FNV1a64("a") = 0xaf63dc4c8601ec8c
    #[test]
    fn fnv_1a_reference_vector_single_byte_a() {
        let mut hasher = DeterministicHasher::new();
        hasher.write(b"a");
        assert_eq!(hasher.finish(), 0xaf63dc4c8601ec8c);
    }

    /// FNV-1a reference vector: hashing the string "foobar":
    ///   FNV1a64("foobar") = 0x85944171f73967e8
    #[test]
    fn fnv_1a_reference_vector_foobar() {
        let mut hasher = DeterministicHasher::new();
        hasher.write(b"foobar");
        assert_eq!(hasher.finish(), 0x85944171f73967e8);
    }

    /// FNV-1a is not commutative: hashing "ab" and "ba" produces
    /// different values. Tests pin this property because a
    /// "deterministic" hasher that ignored input order would
    /// silently produce identical digests for permuted state.
    #[test]
    fn fnv_1a_is_order_sensitive() {
        let mut h_ab = DeterministicHasher::new();
        h_ab.write(b"ab");
        let mut h_ba = DeterministicHasher::new();
        h_ba.write(b"ba");
        assert_ne!(h_ab.finish(), h_ba.finish());
    }

    /// Two hashers fed the same bytes produce the same output —
    /// across constructions, across "process restarts" (we can't
    /// actually restart in a unit test, but the fixed seed makes
    /// this property bit-stable by construction).
    #[test]
    fn two_independent_hashers_match_on_identical_input() {
        let payload = b"the quick brown fox jumps over the lazy dog";
        let mut a = DeterministicHasher::new();
        a.write(payload);
        let mut b = DeterministicHasher::new();
        b.write(payload);
        assert_eq!(a.finish(), b.finish());
    }

    /// `finish` is read-only: calling it twice doesn't mutate the
    /// hasher. The `Hasher` trait requires this.
    #[test]
    fn finish_is_idempotent() {
        let mut h = DeterministicHasher::new();
        h.write(b"some data");
        let first = h.finish();
        let second = h.finish();
        assert_eq!(first, second);
    }

    /// A hasher with a custom seed produces different output than
    /// the default. Used by the frame-digest path to mix a
    /// per-frame nonce.
    #[test]
    fn custom_seed_changes_output() {
        let mut a = DeterministicHasher::new();
        a.write(b"payload");
        let mut b = DeterministicHasher::with_seed(0x1234_5678_9ABC_DEF0);
        b.write(b"payload");
        assert_ne!(a.finish(), b.finish());
    }

    /// Standard `Hash` trait integration: types that derive Hash
    /// flow through `DeterministicHasher` correctly. Uses the
    /// stdlib's `write_u32`/`write_u64` defaults, which route
    /// back to our `write`. Tests pin "this works the same way
    /// as a normal Hasher" — important for digest helpers that
    /// hash structured types.
    #[test]
    fn integrates_with_standard_hash_trait() {
        #[derive(Hash)]
        struct Triple(u32, i64, bool);

        let mut a = DeterministicHasher::new();
        Triple(1, -2, true).hash(&mut a);
        let mut b = DeterministicHasher::new();
        Triple(1, -2, true).hash(&mut b);
        assert_eq!(a.finish(), b.finish());

        let mut c = DeterministicHasher::new();
        Triple(1, -2, false).hash(&mut c);
        assert_ne!(a.finish(), c.finish());
    }
}
