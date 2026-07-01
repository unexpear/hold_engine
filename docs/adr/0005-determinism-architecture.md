# ADR 0005: Determinism architecture

## Status
Accepted

## Context
ADR 0003 names "Determinism and debuggability" as Pillar 4 — a
load-bearing differentiation pillar:

> Deterministic simulation by default. Given the same seed and the
> same input stream, the simulation produces identical state
> bit-for-bit across runs. This unlocks reliable replays,
> networking, reproducible bug reports, and deterministic CI tests.

The replay-lock placeholder already exists
(`EditorPlayState::ReplayingDeterministic`, ADR 0004 Q7) and it
correctly gates hot-reload during replay. But there is no recorder,
no replayer, and no harness that verifies the simulation is
actually deterministic. Without that harness, the promise is
decorative — the same shape that ADR 0004 Q15 called out for
hot-reload.

This ADR captures the determinism architecture before the code
lands. The discipline named in ADR 0003 says decisions hard to
reverse get spent on; the determinism contract is *exactly* that
— once we ship it, every system in the engine has to honor it
forever, and breaking it later silently breaks every replay,
networked game, and CI determinism test in flight.

The proving ground stays the 3D open-world RPG. A determinism
failure in that scenario looks like: a developer captures a 60-second
playthrough recording, sends it to a teammate to reproduce a bug,
and the bug doesn't reproduce because the simulation diverges on
frame 412 (a HashMap iteration order, or a `f32::sin` LUT
populated lazily, or a `dt` jittering by a microsecond). That
failure mode has to be impossible. That's the bar.

## Decision

### Q1 — What does "deterministic simulation" mean for v1?
**Decision: same engine binary + same recording → bit-for-bit
identical World state at every frame boundary.**

The promise scopes deliberately:

**In scope:**
1. Same simulation results from the same `(seed, input_stream)`
   when run on the same engine binary, on the same OS / CPU
   architecture (Windows x86_64 v1, ADR 0001).
2. Frame-boundary World state — the snapshot a system would see
   at `schedule.run()` start, after the previous frame's mutations
   committed.
3. ECS event queue ordering. `World::events` carries a per-frame
   ordered log; that order is part of the contract.

**Out of scope (by design):**
1. **Cross-binary determinism.** A binary built with a different
   Rust compiler version, different opt-level, or different LLVM
   version may produce different floating-point rounding. The
   harness asserts within a binary, not across binaries.
2. **Cross-architecture determinism.** Even within the same
   binary, x86_64 ↔ ARM may diverge on subnormals, FMA fusion, and
   transcendentals. v1 is Windows x86_64 only (ADR 0001), so this
   is moot today; documented here so a future port doesn't
   accidentally widen the contract.
3. **Renderer output.** The renderer is downstream of the
   simulation. ADR 0004 Q2 already excludes "transient backend
   identifiers" from the hot-reload promise; the determinism
   promise inherits that exclusion. Two replays produce the same
   World state, but GPU resource IDs may differ.
4. **GPU readback timing.** Async GPU work (compute shaders,
   buffer readbacks) may complete on different frames between
   runs. The simulation must not depend on GPU completion
   timing — if a system needs a GPU readback, it's an explicit
   stall, not a "is this ready yet" poll.

This narrowed scope is what's actually testable and actually
defensible. "Bit-for-bit identical state across all platforms,
all toolchains, all hardware" is undeliverable; "same binary
same inputs same state" is.

The promise is enforced via the determinism harness defined in
**Q9** below. Without that test, the promise is decorative.

---

### Q2 — Sources of non-determinism we must eliminate
**Decision: enumerate them now, build code around them, gate the
harness on every one.**

A simulation is deterministic iff every input is deterministic. The
known sources of non-determinism in this codebase or its language
runtime:

| Source | Risk | Mitigation |
|--------|------|------------|
| Wall-clock `dt` (`Instant::now()`) | High — varies every frame | The recorder captures the actual `dt` per frame. Replay feeds the recorded `dt`, not a fresh measurement. |
| `HashMap` iteration order (Rust default) | Medium — randomized hasher | Gameplay code never iterates `HashMap` for state-affecting decisions. Component storages are `Vec`-backed (insertion order). World does iterate `HashMap<TypeId, Box<dyn AnyStorage>>` but only for storage lookup, not iteration. |
| Random number generators | High — every system using one | A seeded LCG owned by `World`, advanced explicitly. `thread_rng` and `rand::random` are forbidden in gameplay code (CI grep gate). |
| Threading | High — ordering is non-deterministic | The schedule is currently sequential (schedule.rs line 71-74). Any future parallel-system runner must produce results equivalent to the sequential order — proven by the harness. |
| File-system iteration order | Low — gameplay rarely touches FS | Asset compilation happens at import, not at runtime. Hot-reload is gated to frame boundaries (ADR 0004 Q4) and replay locks reloads entirely (ADR 0004 Q7). |
| Floating-point fast-math | Medium — compiler-controlled | Rust release builds default to strict IEEE 754 (no `-ffast-math` equivalent). We never set `RUSTFLAGS=-Cfast-math`. |
| Transcendental functions | Low within a binary, high across binaries | `f32::sin`/`cos`/`exp`/etc. are determinstic within a binary; the v1 promise scopes to "same binary." Documented in Q1. |
| `Instant`/`SystemTime` reads in gameplay | High | Banned in gameplay code paths. The schedule's `dt` is the only time source. CI grep gate. |
| External API non-determinism (network, audio, etc.) | High | Banned for v1. No networked simulation; audio is fire-and-forget event-driven (no readback into simulation). |

Each row is a load-bearing rule. The harness proves the simulation
is deterministic by replaying recordings; if a new system smuggles
in an `Instant::now()`, the harness diverges and CI fails.

---

### Q3 — What is the high-level architecture?
**Decision: three components, all in `src/determinism/`.**

```
[InputState per frame]    [dt per frame]    [seed at start]
        │                       │                  │
        └──────┬────────────────┴──────────────────┘
               ▼
        InputRecorder ──── on each tick, captures the frame's
               │           (input snapshot, dt) pair into a
               │           growing InputRecording.
               ▼
        InputRecording
               │
               ▼
        InputReplayer ──── feeds (input snapshot, dt) back
               │           into the simulation in order.
               ▼
        DeterminismHarness ── records once, replays, hashes
                             frame digests, asserts equality.
```

Each component is single-responsibility:
- **`InputRecording`** — pure data, serializable. The artifact a
  developer captures, ships in a bug report, or commits to CI.
- **`InputRecorder`** — observes the engine's per-frame
  `InputState` + `dt` and appends to an `InputRecording`.
- **`InputReplayer`** — given an `InputRecording`, returns
  the next `(InputState, dt)` pair for each engine tick.
- **`DeterminismHarness`** — the load-bearing CI test. Drives
  the engine through a recording twice with fresh state, asserts
  frame-by-frame world digest equality.

All four ship in v1. The harness is the test that makes the
promise enforceable; without it, the rest is plumbing.

---

### Q4 — `InputRecording` data layout
**Decision: a flat `Vec<RecordedFrame>` plus a single seed +
metadata header. Binary-serializable, deterministic to encode.**

```rust
pub struct InputRecording {
    /// Stable header; rejected on decode if it doesn't match.
    pub format_version: u32,
    /// Initial RNG seed at frame 0. The simulation's `World` is
    /// seeded with this value before tick 0 runs.
    pub initial_seed: u64,
    /// Per-frame inputs. `frames[i]` is the state delivered to
    /// the engine at frame `i`. Length == replay length.
    pub frames: Vec<RecordedFrame>,
}

pub struct RecordedFrame {
    /// dt the engine saw this frame. Replay feeds this back so
    /// physics integrate identically.
    pub dt: f32,
    /// InputState as a snapshot at frame start. Using owned data
    /// (not borrowed) so the recording is self-contained.
    pub input: RecordedInput,
}

pub struct RecordedInput {
    /// Sorted to make encode/decode deterministic. HashSets do
    /// not iterate in insertion order; the recorder sorts before
    /// snapshotting so two recordings made from the same engine
    /// state are byte-equal.
    pub keys_down: Vec<u32>,
    pub keys_just_pressed: Vec<u32>,
    pub keys_just_released: Vec<u32>,
    pub mouse_pos: (i32, i32),
    pub mouse_delta: (i32, i32),
    pub mouse_buttons: Vec<u8>, // MouseButton encoded
    pub mouse_just_pressed: Vec<u8>,
    pub mouse_just_released: Vec<u8>,
    pub mouse_wheel: f32,
}
```

Why `Vec<u32>` instead of `HashSet<u32>` in `RecordedInput`? Two
reasons: (1) determinism — sorted vecs encode byte-identically
across runs; (2) serialization — we want a stable on-disk layout.
The reconstructed `InputState` rebuilds the HashSets internally
from the sorted vecs.

The recording format version is the breaking-change gate: if the
recorded layout ever changes (e.g. gamepad axes added), version
bumps and old recordings are rejected at decode rather than
silently misinterpreted.

---

### Q5 — How does input recording work?
**Decision: a single `InputRecorder::tick(&InputState, dt)`
called once per frame from the engine's run loop, before
`schedule.run`.**

The recorder doesn't intrude on the rest of the engine. Callers
that opt into recording wrap the existing run loop with a
recorder reference; callers that don't pay zero cost.

```rust
let mut recorder = hold::determinism::InputRecorder::new(initial_seed);
engine.run_app_recording(renderer, app, &mut recorder);
let recording = recorder.into_recording();
```

The integration shim is shipped as
[`Engine::run_app_recording`](`crate::engine::Engine::run_app_recording`).
It captures before delegating to `App::update`, so the recorder
sees the exact `(InputState, dt)` pair the simulation is about to
consume. Callers that don't want recording use the existing
`Engine::run_app` — zero overhead.

Why "once per frame, before schedule.run"? Because that's exactly
where `schedule.run` consumes its inputs. Recording at any other
point would capture a state the simulation didn't see.

---

### Q6 — How does input replay work?
**Decision: `InputReplayer` mutates the engine-supplied
`InputState` in place each tick, then the engine's normal run
loop sees the replayed state as if it had come from the OS.**

```rust
let mut replayer = InputReplayer::new(recording);
while let Some(frame) = replayer.next_frame() {
    apply_recorded_input(&frame.input, &mut engine.input);
    engine.tick_one(frame.dt);
}
```

`apply_recorded_input` overwrites the relevant `InputState`
fields. The engine's `Window::poll_events` is bypassed during
replay because it would otherwise inject real OS events on top
of the replayed ones.

The replayer is finite: when the recording is exhausted, the
replay ends. No "loop the recording" mode in v1 — replay length
equals recording length, exact.

---

### Q7 — Fixed timestep vs replayed `dt`?
**Decision: replay uses the recorded `dt` per frame. Fixed
timestep is a separate, additive concern (not v1).**

A common design choice in deterministic engines is to mandate a
fixed simulation timestep so floating-point integration is
identical regardless of the wall-clock cadence. We don't take
that path for v1 because:

1. It would force gameplay code to either accumulate sub-frame
   error or run the simulation multiple times per render. Both
   add complexity that isn't required by the determinism
   promise.
2. Replaying the recorded `dt` gives the same identity property
   (same recording → same simulation) without changing the
   gameplay-facing API.
3. A future fixed-timestep mode can layer on top: the recorder
   captures the fixed-step `dt` instead of the wall-clock one,
   and replay still works.

The cost: the recorder's `dt` capture must be exactly the value
the engine fed to `schedule.run`. That's a single read at a
known point in the run loop — easy to get right, easy to
test.

---

### Q8 — Frame digest for the harness
**Decision: a stable hash of the world's user-observable state,
computed by walking component storages in `TypeId`-stable order.**

The harness compares two replays by computing a per-frame digest
and asserting equality. The digest captures "what a system would
see," not transient internal state:

```rust
pub fn frame_digest(world: &World) -> u64 {
    let mut hasher = DeterministicHasher::new();
    // 1. Tick counter
    world.tick().hash(&mut hasher);
    // 2. Live entity ids (sorted)
    let mut entities: Vec<Entity> = world.entities().collect();
    entities.sort();
    entities.hash(&mut hasher);
    // 3. Component storages, walked in stable type order.
    //    The storage's Hash impl serialises its data in
    //    insertion order — Vec-backed, deterministic.
    for (type_id, storage) in world.storages_in_stable_order() {
        type_id.hash(&mut hasher);
        storage.deterministic_hash(&mut hasher);
    }
    // 4. Event queue contents (serialised in arrival order).
    world.events().deterministic_hash(&mut hasher);
    hasher.finish()
}
```

Two design choices worth calling out:

**Why a custom `DeterministicHasher`?** Rust's default `Hasher`
implementations randomize their seed at process start. Using
`std::collections::hash_map::DefaultHasher` would make the digest
*itself* non-deterministic. We use a fixed-seed FNV-1a or
similar — in-house, ~30 lines, no external crate.

**Why not just `std::Debug` and string-compare?** Two reasons:
- Performance: O(n) bytes to format for digest comparison vs
  O(n) bytes to hash. The hash wins by an order of magnitude
  on a busy world.
- Stability: `Debug` impls aren't part of the public contract
  and might change with refactors. A typed digest pinned by
  tests is a stable contract.

The component-storage `deterministic_hash` requires every
gameplay-relevant component type to be hashable in a stable way.
Built-ins implement it directly; new components opt in via
trait. The CI gate is "every storage type has a
`DeterministicHash` impl, no exceptions."

---

### Q9 — The harness as load-bearing CI test
**Decision: a property test verifies replay produces identical
frame digests on every commit. Without this, Q1 is decorative.**

```text
Property test: simulation_is_deterministic_under_replay
  Given:  a sample scene S, an InputRecording R captured against
          S at seed K, and a frame budget N == R.frames.len().
  When:   load S; replay R; capture per-frame digest stream D_a.
          Then load S again (fresh World); replay R; capture D_b.
  Then:   assert D_a == D_b, frame-by-frame.
```

The harness lives in `src/determinism/harness.rs` and runs in CI
on every commit. The sample scene starts as a tiny scene with a
single moving entity (sphere on a velocity, gravity-damped) and
expands as systems are integrated.

This test is the only enforcement of Q1. ADR 0003 Pillar 4 says
the determinism promise is a competitive differentiator; this is
the test that makes it sacred. Mirrors the shape of ADR 0004
Q15's equivalence harness for hot-reload.

---

### Q10 — Hardware-portable performance budget
**Decision: the recorder/replayer/harness run identically across
the i3-equivalent through i9-equivalent CPU spectrum the engine
targets. No SIMD/AVX dependencies. No parallelism. Memory
footprint scales linearly with recording length.**

This refinement is here because the project charter mandates
i3-to-i9 portability and because the determinism path runs on
*every* frame in record mode — a slow recorder would tax every
playtest, not just CI runs.

| Concern | Bound |
|---------|-------|
| Recorder per-frame cost | < 10μs on i3-class CPU for typical input states (~50 keys, 10 mouse events). Single allocation per frame at the `Vec::push`; sort is on tiny per-frame slices. |
| Recording memory footprint | ~80 bytes per frame at typical input density. 60s @ 60Hz = ~290 KB. Below any low-end RAM constraint. |
| Replayer per-frame cost | < 5μs (read from `Vec`, copy out). |
| Harness frame-digest cost | O(world size). Acceptable in CI; not on gameplay-hot path. |
| Parallelism | None required. Single-threaded recorder, single-threaded replayer, single-threaded harness. |
| SIMD | None required. Scalar code throughout. |

Rationale: the recorder runs every frame the developer plays the
game; making it cheap matters. The replayer runs only during
replay (rare). The harness runs in CI (one-shot). All three are
held to "fast on i3" because the alternative — premature
SIMD/AVX-512 / parallelism — would cement i9-only assumptions
and break the portability contract.

---

## Guardrails

- **Same binary, same inputs, same state.** This is the Q1
  promise; every implementation choice is evaluated against it.
  The Q9 harness is the test that keeps this load-bearing.
- **No `Instant::now` / `SystemTime::now` in gameplay code.**
  Time enters the simulation only through `dt`. CI grep gate
  enforces.
- **No `thread_rng` / `rand::random` in gameplay code.** RNG
  enters the simulation only through `World`-owned seeded LCG.
  CI grep gate enforces.
- **Recording format versioned.** A breaking change to
  `RecordedFrame` bumps `format_version`; old recordings refuse
  to decode rather than silently misinterpret.
- **Frame digest pinned in CI.** Every component storage has a
  `DeterministicHash` impl. New components without one fail to
  compile (trait bound on storage registration).
- **Replay locks hot-reload (ADR 0004 Q7).** Already enforced;
  this ADR re-asserts it.
- **No external crates.** In-house deterministic hasher, in-house
  recording codec.

## Open questions

- **Network determinism.** v1 has no networked simulation. A
  future networking ADR will define what "deterministic across
  hosts" means and how it interacts with `dt` capture (likely:
  fixed timestep, lockstep input exchange).
- **Floating-point cross-toolchain.** The v1 promise scopes to
  "same binary." A future "deterministic across toolchains" mode
  would require either soft-float (slow) or carefully audited
  IEEE-754 strictness flags. Tracked as M-future.
- **Transcendentals.** `f32::sin`/`cos`/`exp` are deterministic
  within a binary on x86_64. A cross-architecture port would need
  in-house tables. Tracked as M-future.
- **Recording compression.** v1 stores raw `Vec<RecordedFrame>`.
  A 5-minute recording at 60Hz is ~1.4 MB raw — fine for v1, but
  a future compression pass (RLE on identical consecutive frames
  + delta encoding) could shrink that 10×. Tracked as M-future.

## Related
- Project charter: `docs/PROJECT_CHARTER.md`
- Foundation ADR: `docs/adr/0001-engine-editor-foundation.md`
- Differentiation: `docs/adr/0003-differentiation-and-competitive-posture.md`
- Hot-reload (replay-lock counterpart): `docs/adr/0004-hot-reload-architecture.md`

## Revision history
- **Draft 1** (initial): Q1–Q10. Captures the determinism
  contract, the recording layout, the harness shape, and the
  hardware-portability bounds. Ships before any code lands so
  the recorder/replayer/harness implementation has an audited
  design to anchor against — the same discipline ADR 0004 used
  for hot-reload.
