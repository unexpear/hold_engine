//! Determinism harness — the load-bearing CI test for ADR 0005.
//!
//! ADR 0003 Pillar 4 names "Deterministic simulation by default"
//! as a load-bearing differentiation pillar:
//!
//! > Given the same seed and the same input stream, the simulation
//! > produces identical state bit-for-bit across runs.
//!
//! Without a CI test that proves this property holds, every future
//! change is a free chance to silently break it. With this test,
//! breakage lands on a red CI build before it lands on developers
//! and replays diverge in the field.
//!
//! ## What this harness exercises
//!
//! 1. **Records** an input stream against a small synthetic
//!    simulation (a single moving entity driven by keyboard
//!    keys).
//! 2. **Replays** the recording twice into two separate fresh
//!    `World` instances.
//! 3. **Asserts** the per-frame digest streams are bit-identical
//!    between the two replays.
//!
//! Step 2's "two replays" pattern catches every source of
//! non-determinism enumerated in ADR 0005 Q2: if a system smuggled
//! in `Instant::now()`, the two replays would disagree on the
//! first frame that read the clock; if a HashMap iteration order
//! affected gameplay, the two replays would diverge on the first
//! frame that depended on it.
//!
//! ## Public surface
//!
//! [`record_and_replay_frame_digests`] is the function shape every
//! determinism CI test follows: build a recording with one
//! callback, replay it with another, get back the per-frame
//! digest stream. The callback shape lets a caller plug in any
//! gameplay logic — the harness doesn't bake in a single sample
//! scene.
//!
//! See `docs/adr/0005-determinism-architecture.md` Q9 for the
//! design and Q10 for the hardware-portability bounds (the
//! harness runs on i3-class CPUs in single-digit milliseconds
//! per frame).

use crate::ecs::{Entity, World};
use crate::input::InputState;

use super::digest::{frame_digest_from_components, FrameDigest};
use super::recorder::InputRecorder;
use super::recording::InputRecording;
use super::replayer::InputReplayer;

/// Drive a simulation through `frame_count` frames while
/// recording the per-frame `(InputState, dt)` pairs the
/// `step_fn` consumes. Returns both the recording and the
/// digest stream observed during recording.
///
/// The `step_fn` is called once per frame with `(world, input,
/// dt)`, in that order. It's expected to:
///
/// 1. Read `input` to drive gameplay logic.
/// 2. Mutate `world` to reflect the frame's effects.
///
/// `tracked_entities_fn` returns the entity list the digest
/// helper should hash on this frame. It's a closure so the
/// caller can spawn entities mid-recording and have them
/// included in subsequent digests; an alternative shape that
/// asks for a fixed entity list up front would force every test
/// to pre-spawn its full set.
///
/// `inputs_fn` returns the synthesised `InputState` for each
/// frame. In a real-engine integration the OS would deliver
/// inputs via the window system; in the harness we synthesise
/// them deterministically so the test is hermetic.
///
/// `dt` is fixed across frames at 1/60 s — a reasonable default
/// for the harness's small synthetic scenes. Real recordings
/// captured against the live editor carry the wall-clock `dt`
/// per ADR 0005 Q7.
pub fn record_with_inputs<S, T, I>(
    initial_seed: u64,
    frame_count: usize,
    dt: f32,
    mut inputs_fn: I,
    mut step_fn: S,
    mut tracked_entities_fn: T,
) -> (InputRecording, Vec<FrameDigest>)
where
    S: FnMut(&mut World, &InputState, f32),
    T: FnMut(&World) -> Vec<Entity>,
    I: FnMut(usize) -> InputState,
{
    let mut world = World::new();
    let mut recorder = InputRecorder::new(initial_seed);
    let mut digests = Vec::with_capacity(frame_count);

    for frame_index in 0..frame_count {
        // Synthesise the frame's input. In a real run this comes
        // from the OS via Window::poll_events; in the harness it
        // comes from a closure so the test stays hermetic.
        let input = inputs_fn(frame_index);
        // Capture before stepping — the recorder snapshots what
        // the simulation is *about* to consume (ADR 0005 Q5).
        recorder.tick(&input, dt);
        // Run the gameplay logic.
        step_fn(&mut world, &input, dt);
        // Advance the world's tick counter so the digest's
        // per-tick distinguishability holds.
        world.advance_tick();
        // Compute the post-step digest.
        let entities = tracked_entities_fn(&world);
        digests.push(frame_digest_from_components(
            &world,
            &entities,
            world.tick(),
        ));
    }

    (recorder.into_recording(), digests)
}

/// Replay a recording into a fresh `World`, capturing the
/// per-frame digest stream. The `step_fn` and
/// `tracked_entities_fn` callbacks must match the ones used
/// during recording — the harness's contract is "same gameplay
/// logic, same inputs, same digests."
///
/// Returns the digest stream observed during this replay. Two
/// replays of the same recording with the same callbacks must
/// produce identical digest streams; that's the determinism
/// promise the harness pins.
pub fn replay_into_digests<S, T>(
    recording: &InputRecording,
    mut step_fn: S,
    mut tracked_entities_fn: T,
) -> Vec<FrameDigest>
where
    S: FnMut(&mut World, &InputState, f32),
    T: FnMut(&World) -> Vec<Entity>,
{
    let mut world = World::new();
    let mut input_state = InputState::new();
    let mut replayer = InputReplayer::new(recording);
    let mut digests = Vec::with_capacity(recording.len());

    while let Some(dt) = replayer.tick_into(&mut input_state) {
        step_fn(&mut world, &input_state, dt);
        world.advance_tick();
        let entities = tracked_entities_fn(&world);
        digests.push(frame_digest_from_components(
            &world,
            &entities,
            world.tick(),
        ));
    }

    digests
}

/// Record a simulation, replay it twice from fresh state, and
/// assert the two replays produce identical digest streams. This
/// is the load-bearing CI test for ADR 0005 Q1 (same recording →
/// same simulation).
///
/// Returns the recording + the matched digest stream so callers
/// can use it for further analysis (e.g. an assertion that the
/// recording exercised at least N distinct states, not just
/// "every frame was identical to frame 0").
///
/// Panics with a descriptive message naming the frame index of
/// the first divergence if the two replays disagree. Panic in
/// the harness is the right behavior — divergence is a hard
/// failure that demands a developer's attention.
pub fn record_and_replay_frame_digests<S, T, I>(
    initial_seed: u64,
    frame_count: usize,
    dt: f32,
    mut inputs_fn: I,
    mut step_fn: S,
    mut tracked_entities_fn: T,
) -> (InputRecording, Vec<FrameDigest>)
where
    S: FnMut(&mut World, &InputState, f32),
    T: FnMut(&World) -> Vec<Entity>,
    I: FnMut(usize) -> InputState,
{
    // Phase 1: record while running the simulation, capture the
    // baseline digest stream.
    let (recording, baseline_digests) = record_with_inputs(
        initial_seed,
        frame_count,
        dt,
        |i| inputs_fn(i),
        |w, i, d| step_fn(w, i, d),
        |w| tracked_entities_fn(w),
    );

    // Phase 2: replay into a fresh World, capture a second
    // digest stream.
    let replay_digests = replay_into_digests(
        &recording,
        |w, i, d| step_fn(w, i, d),
        |w| tracked_entities_fn(w),
    );

    // Phase 3: assert the two streams match frame-by-frame. The
    // first divergence wins the panic message — past that, the
    // simulation has already gone off the rails and later frames
    // wouldn't help triage.
    assert_eq!(
        baseline_digests.len(),
        replay_digests.len(),
        "harness: digest stream length mismatch (record={} replay={})",
        baseline_digests.len(),
        replay_digests.len()
    );
    for (i, (a, b)) in baseline_digests.iter().zip(replay_digests.iter()).enumerate() {
        assert_eq!(
            a, b,
            "harness: simulation diverged at frame {i} \
             (record={a}, replay={b}). \
             ADR 0005 Q1 contract violated — see Q2 for sources of non-determinism to audit."
        );
    }

    // Phase 4: replay a *second* time. This catches non-determinism
    // that's invariant under one record + one replay but not
    // across two replays (e.g. a system that depends on its own
    // internal HashMap iteration order — frame 1 of replay 1
    // could match frame 1 of recording, but frame 1 of replay 2
    // could disagree with both).
    let second_replay_digests = replay_into_digests(
        &recording,
        |w, i, d| step_fn(w, i, d),
        |w| tracked_entities_fn(w),
    );
    for (i, (a, b)) in baseline_digests
        .iter()
        .zip(second_replay_digests.iter())
        .enumerate()
    {
        assert_eq!(
            a, b,
            "harness: replay #2 diverged from recording at frame {i} \
             (record={a}, replay2={b})."
        );
    }

    (recording, baseline_digests)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::components::{Transform, Velocity};
    use crate::input::vk;
    // (Vec3 was used by an early sketch of this module; the
    // current tests synthesize Transform/Velocity via the
    // ::IDENTITY / ::ZERO constants and never name Vec3 directly.)

    /// A minimal physics-like system: integrates Velocity into
    /// Transform.position. Fixed dt feeds in from outside; no
    /// internal time source. Used in the harness tests below.
    fn integrate_velocity(world: &mut World, _input: &InputState, dt: f32, e: Entity) {
        let new_pos = world
            .get::<Transform>(e)
            .zip(world.get::<Velocity>(e))
            .map(|(t, v)| t.position + v.linear * dt);
        if let Some(p) = new_pos {
            if let Some(t) = world.get_mut::<Transform>(e) {
                t.position = p;
            }
        }
    }

    /// Apply a tiny gameplay rule: pressing W adds 1.0 to
    /// Velocity.linear.x for the entity. No clock reads, no
    /// HashMap iteration — pure deterministic state machine.
    fn apply_input(world: &mut World, input: &InputState, e: Entity) {
        if input.key_pressed(vk::W) {
            if let Some(v) = world.get_mut::<Velocity>(e) {
                v.linear.x += 1.0;
            }
        }
    }

    /// Round-trip: a recording driven through the harness
    /// produces matching digest streams across record + two
    /// replays. This is the load-bearing test for ADR 0005 Q1.
    ///
    /// Uses `Cell<Option<Entity>>` to share the spawned-entity
    /// handle between the step and tracked-entities closures.
    /// `Option<Entity>` is `Copy`, so `Cell::get/set` is a plain
    /// load/store — no `RefCell` runtime check overhead.
    #[test]
    fn determinism_harness_round_trip_matches_digests() {
        use std::cell::Cell;

        let entity_handle = Cell::new(None::<Entity>);
        let last_seen = Cell::new(None::<Entity>);

        let (_recording, digests) = record_and_replay_frame_digests(
            42, // initial_seed
            10, // frame_count
            1.0 / 60.0,
            |frame_index| {
                // Press W on frames 0, 3, 6 to give the entity a
                // distinct trajectory. The harness wraps each
                // frame's input as a synthetic InputState built
                // from scratch.
                let mut input = InputState::new();
                if matches!(frame_index, 0 | 3 | 6) {
                    input.test_press_key(vk::W);
                }
                input
            },
            |world, input, dt| {
                // Spawn-on-first-call. After spawn, the same
                // handle is reused.
                let e = match entity_handle.get() {
                    Some(e) if world.is_alive(e) => e,
                    _ => {
                        let e = world.spawn();
                        world.add_component(e, Transform::IDENTITY);
                        world.add_component(e, Velocity::ZERO);
                        entity_handle.set(Some(e));
                        e
                    }
                };
                last_seen.set(Some(e));
                apply_input(world, input, e);
                integrate_velocity(world, input, dt, e);
            },
            |_world| last_seen.get().into_iter().collect(),
        );

        assert_eq!(digests.len(), 10);
        // Frames must not all be identical — that would mean the
        // simulation didn't actually do anything. The W presses
        // should produce a strictly increasing position.x.
        let unique_count = {
            let mut sorted: Vec<u64> = digests.iter().map(|d| d.as_u64()).collect();
            sorted.sort_unstable();
            sorted.dedup();
            sorted.len()
        };
        assert!(
            unique_count >= 5,
            "harness sanity: expected many distinct digests, got {unique_count}"
        );
    }

    /// Empty recording (zero frames) is trivially deterministic:
    /// both replays produce empty digest streams. Edge case
    /// — the harness must not panic on zero frames.
    #[test]
    fn determinism_harness_handles_zero_frame_recording() {
        let mut spawned: Option<Entity> = None;
        let (recording, digests) = record_and_replay_frame_digests(
            0,
            0,
            1.0 / 60.0,
            |_| InputState::new(),
            |world, _input, _dt| {
                if spawned.is_none() {
                    spawned = Some(world.spawn());
                }
            },
            |_| Vec::new(),
        );
        assert!(recording.is_empty());
        assert!(digests.is_empty());
    }

    /// Recording survives encode → decode → replay. The codec
    /// is part of the determinism contract: a recording dropped
    /// to disk and reloaded must replay identically. Exercises
    /// the full pipeline end-to-end.
    ///
    /// The test uses `Cell<Option<Entity>>` (rather than `mut`
    /// locals) to share the spawned-entity handle between the
    /// `step` and `track` closures — both want to read the same
    /// `Option<Entity>` and the borrow checker rejects the
    /// FnMut + Fn pair on a plain `mut` local. `Cell` is the
    /// right tool because `Option<Entity>` is `Copy`, so we
    /// don't pay the `RefCell` runtime borrow check overhead.
    #[test]
    fn determinism_harness_survives_codec_round_trip() {
        use crate::determinism::{decode_input_recording, encode_input_recording};
        use std::cell::Cell;

        // Phase 1: record + capture baseline digests.
        let spawn_handle = Cell::new(None::<Entity>);
        let last_handle = Cell::new(None::<Entity>);
        let (recording, baseline) = record_with_inputs(
            7,
            5,
            1.0 / 60.0,
            |frame_index| {
                let mut input = InputState::new();
                if matches!(frame_index, 0 | 2) {
                    input.test_press_key(vk::W);
                }
                input
            },
            |world, input, dt| {
                let e = match spawn_handle.get() {
                    Some(e) if world.is_alive(e) => e,
                    _ => {
                        let e = world.spawn();
                        world.add_component(e, Transform::IDENTITY);
                        world.add_component(e, Velocity::ZERO);
                        spawn_handle.set(Some(e));
                        e
                    }
                };
                last_handle.set(Some(e));
                apply_input(world, input, e);
                integrate_velocity(world, input, dt, e);
            },
            |_world| last_handle.get().into_iter().collect::<Vec<_>>(),
        );

        // Phase 2: encode + decode through the codec.
        let bytes = encode_input_recording(&recording);
        let decoded = decode_input_recording(&bytes).expect("codec round-trip");
        assert_eq!(decoded, recording);

        // Phase 3: replay the *decoded* recording, fresh World.
        // Fresh Cells so the spawn happens on the new world.
        let spawn_handle2 = Cell::new(None::<Entity>);
        let last_handle2 = Cell::new(None::<Entity>);
        let replay_digests = replay_into_digests(
            &decoded,
            |world, input, dt| {
                let e = match spawn_handle2.get() {
                    Some(e) if world.is_alive(e) => e,
                    _ => {
                        let e = world.spawn();
                        world.add_component(e, Transform::IDENTITY);
                        world.add_component(e, Velocity::ZERO);
                        spawn_handle2.set(Some(e));
                        e
                    }
                };
                last_handle2.set(Some(e));
                apply_input(world, input, e);
                integrate_velocity(world, input, dt, e);
            },
            |_world| last_handle2.get().into_iter().collect::<Vec<_>>(),
        );

        // Phase 4: digest streams match.
        assert_eq!(replay_digests, baseline);
    }

    /// Sanity check: a recording that exercises distinct W
    /// presses produces a digest stream with multiple distinct
    /// values. This catches the case where a future regression
    /// makes the digest position-blind — the harness would
    /// silently report "all frames matched" because every digest
    /// would be the same.
    ///
    /// The expected behaviour: W pressed on frames 0/3/6 ramps
    /// Velocity.linear.x → position.x grows non-linearly →
    /// digests differ across frames.
    #[test]
    fn harness_digest_actually_reflects_simulation_state() {
        use std::cell::Cell;

        let entity_handle = Cell::new(None::<Entity>);
        let last_seen = Cell::new(None::<Entity>);

        let (_recording, digests) = record_with_inputs(
            42,
            10,
            1.0 / 60.0,
            |frame_index| {
                let mut input = InputState::new();
                if matches!(frame_index, 0 | 3 | 6) {
                    input.test_press_key(vk::W);
                }
                input
            },
            |world, input, dt| {
                let e = match entity_handle.get() {
                    Some(e) if world.is_alive(e) => e,
                    _ => {
                        let e = world.spawn();
                        world.add_component(e, Transform::IDENTITY);
                        world.add_component(e, Velocity::ZERO);
                        entity_handle.set(Some(e));
                        e
                    }
                };
                last_seen.set(Some(e));
                apply_input(world, input, e);
                integrate_velocity(world, input, dt, e);
            },
            |_world| last_seen.get().into_iter().collect(),
        );

        assert_eq!(digests.len(), 10);
        // Distinct digests prove the digest helper actually
        // observed state changes across frames. Otherwise a
        // position-blind digest would silently let determinism
        // failures slip through.
        let unique = {
            let mut s: Vec<u64> = digests.iter().map(|d| d.as_u64()).collect();
            s.sort_unstable();
            s.dedup();
            s.len()
        };
        assert!(
            unique >= 5,
            "expected the digest to track per-frame state changes; got only {unique} unique values"
        );
    }
}
