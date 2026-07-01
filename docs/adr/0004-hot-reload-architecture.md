# ADR 0004: Hot-reload architecture

## Status
Accepted

## Context
ADR 0003 elevated "iteration speed" to a load-bearing differentiation
pillar (Pillar 2): sub-second editor cold start, hot reload that
doesn't lie, shaders compiled at import. The first two pieces are
already partly delivered (zero-deps + small binary footprint give the
cold-start win for free); hot reload is the piece that still doesn't
exist.

This ADR captures the architecture before any code lands. The
discipline named in ADR 0003 says decisions that are hard to reverse
get spent on; hot-reload is exactly that — built wrong, the wrong
shape cements iteration pain into every dev cycle going forward.

The proving ground stays the 3D open-world RPG. Hot-reload's promise
is concrete: a developer saves a texture in Krita, alt-tabs to the
running game, and the change is visible by the next frame — without
restarting, without manual triggers, without editor freezes. That's
the bar.

This document is the second draft. The first draft was reviewed
against the existing codebase by an independent reviewer who found
four load-bearing issues (infrastructure the original draft assumed
was in place but wasn't) plus seven specifications the original draft
had left implicit. Every finding has been folded in as a concrete
decision below; no scope was cut.

## Decision

### Q1 — What does "hot reload" actually mean for v1?
**Decision: asset hot-reload only. Code hot-reload deliberately
deferred to a future ADR.**

Asset hot-reload covers textures, normal maps, shaders, scenes,
materials, and meshes. A change to any source file (`*.bmp`, `*.tga`,
`*.obj`, `*.glsl`, `*.hsc`, `*.material`, ...) is detected, the
asset is recompiled and replaced in the live runtime, and any GPU /
editor caches that referenced it are invalidated.

Code hot-reload (swapping out compiled Rust game logic without a
restart) requires a stable C ABI between engine and game crates,
`LoadLibrary`-based reload, and ECS state preservation across the
swap. It is a 6+ month structural change. It is also the smaller
pain point in practice — DCC asset edits are 80%+ of the iteration
cycles a content-heavy game generates.

**Consequences**
- Code edits still require recompile + relaunch in v1.
- The asset-reload architecture is designed so that a future
  code-reload ADR can extend it without rewriting the foundation:
  the same invalidation bus carries "code-side cache invalidated"
  events when that work lands.

---

### Q2 — What is the engine's hot-reload promise?
**Decision: hot-reloaded *user-observable state* must equal
clean-reload *user-observable state*.**

User-observable state is defined as:
1. The contents of `AssetStore` for every asset kind (Texture,
   NormalMap, Material, MeshData, Shader, MeshMapAO,
   MeshMapCurvature, ...).
2. The pixels rendered to the framebuffer for any fixed scene +
   camera + lighting configuration, modulo `ε` tolerance to
   absorb float non-associativity in the GPU pipeline.

Excluded from the promise (by design):
- Transient backend identifiers — GL texture IDs, VAO names,
  program IDs. The OpenGL driver hands out fresh integers on each
  upload; these will differ across reload. Tests must not depend
  on them.
- Per-frame-cache memory addresses, allocator state, and any
  lazy-evicted GPU resource that hasn't yet been re-uploaded.

This narrowed scope is what's actually testable and actually
defensible. The original "bit-for-bit equivalent" framing was
unimplementable in any backend that hands out resource IDs
non-deterministically.

The promise is enforced via the equivalence harness defined in
**Q14** below — without that test, the promise is decorative.

---

### Q3 — What is the high-level architecture?
**Decision: four cooperating components on three threads.**

```
[Source files]
      │
      │ Win32 ReadDirectoryChangesW
      ▼
 [FileWatcher]   thread per watched root
      │
      │ std::sync::mpsc
      ▼
 [CompileQueue]  worker thread
      │
      │ CompileResult { handle, payload, generation }
      ▼
 [ReloadDispatcher]   main thread, frame boundary
      │
      │ AssetInvalidated events
      ▼
 [InvalidationBus]   subscribers: GPU caches, editor inspector,
                     ECS world events
```

| Thread | Owns | Communicates via |
|--------|------|------------------|
| Main / frame | `AssetStore`, `OpenGLRenderer`, simulation tick | mpsc receiver |
| FileWatcher | Win32 directory `HANDLE`, change buffer | mpsc sender |
| CompileQueue worker | Compiler state, generation counters, source-path index | mpsc sender to main; mpsc receiver from watcher |

Three threads in v1. The compile worker can grow into a thread pool
when asset count justifies it (M8+).

---

### Q4 — How do the components interact in detail?

#### FileWatcher
- Win32 `ReadDirectoryChangesW`, one thread per watched root.
- **Path canonicalization at the boundary.** Every emitted path is
  canonicalized (absolute path, normalized separators, lower-cased
  on Windows) before leaving the watcher thread, so the
  CompileQueue's reverse index lookup is always exact.
- **Coalesces events within a 50ms window per path.** Editors save
  by writing temp + rename, producing Modified+Created+Deleted
  bursts; the watcher emits the final state, not the intermediate
  noise. The 50ms value is a v1 starting point; **the M7.1
  milestone instruments raw event logs to calibrate it
  empirically against actual DCC tools** — Photoshop, Krita,
  Substance Painter, Blender, VS Code each have different save
  patterns. The constant is a `WATCH_COALESCE_WINDOW_MS` named
  config so the calibration only changes one line.
- **Buffer overflow handling.** `ReadDirectoryChangesW` can drop
  events under high event rate. The watcher detects buffer
  overflow (Win32 returns `ERROR_NOTIFY_ENUM_DIR` or sets the
  notification's `Action` to indicate overflow) and triggers a
  **periodic full-scan reconciliation pass**: walk the watched
  tree, compare every file's `mtime` against the last scan, emit
  synthetic Modified events for any file newer than its
  last-known-state. Reconciliation runs every 30s in normal
  operation (cheap; the OS file metadata is cached) and on demand
  after any overflow. This makes "asset stale forever" a bug we
  catch instead of a silent failure mode.
- **Filter by extension.** Only assets the engine recognises reach
  the CompileQueue; `.tmp`, IDE swap files, `.git`, etc. never
  enter the pipeline.
- Failure mode: log and retry on Win32 errors; never panics. A
  "stop" sentinel drains the channel for clean shutdown.

#### CompileQueue
- **Builds its own reverse index** at startup:
  `HashMap<PathBuf, Vec<LogicalName>>`. The asset compiler ships
  per-asset dependency lists in `CompiledAssetMetadata.dependencies`
  (see `src/assets/compiled.rs`) — those are *forward* lists. The
  CompileQueue inverts them at registration time and updates
  incrementally as assets are added/removed. The original ADR
  draft claimed the compiler "already produced" this inversion;
  it does not — the inversion is the queue's job.
- On `FileChange`, looks up affected logical names. For each,
  schedules a compile with an incremented per-asset generation
  counter.
- **Per-logical-name bounded queue.** Only the latest pending
  compile per logical name is queued; if a third change arrives
  while the second is queued and the first is in-flight, the
  third replaces the second. The generation counter mechanism
  already implies this; it's now explicit.
- Compile runs `compile_asset_from_source`, producing a `Vec<u8>`
  `.hcmp` payload + metadata. Result returned over mpsc as either
  `CompileResult::Ok { logical_name, kind, payload, generation }`
  or `CompileResult::Err { logical_name, error, generation }`.
- **Last-wins via generation**: the dispatcher checks
  `generation == latest_for(logical_name)` before applying. Stale
  results discarded.

#### ReloadDispatcher
- Lives on the main thread. **Drains the result channel inside a
  new `Engine::on_reload_dispatch()` hook**, called between
  `end_frame()` and the next `begin_frame()`. The original draft
  said "frame-start hook"; the codebase audit (`src/engine.rs`)
  found no such hook exists — it must be added. This is part of
  the M7.0 pre-flight (see Q10).
- Resolves the logical name to its existing `Handle<T>` via the
  `CompiledAssetRuntimeLoader`'s cache. Decodes the new payload
  into the strong type.
- Calls `AssetStore::replace<T>(handle, value) -> Option<T>` —
  the new method to be added in M7.0 (see Q10). Replaces the
  asset under the existing handle and returns the previous value
  for cleanup. The `AssetPool<T>` is already a generational
  sparse-vec; this is a five-line addition (verified against
  `src/assets/store.rs` + `src/assets/pool.rs`).
- **Atomicity**: replace only happens at frame boundaries. The
  simulation tick sees a clean step transition, never a mid-frame
  swap. This is the property that preserves determinism per Q7.
- After replace, emits `AssetInvalidated { kind, handle_index }`
  onto the InvalidationBus (Q11 below) and via `World::events`
  (Q12 below).

#### InvalidationBus
- Synchronous broadcast: subscribers register
  `Box<dyn Fn(&AssetInvalidated)>` callbacks at engine
  initialisation, with a `SubscriberId` returned for explicit
  deregistration.
- Drained once per frame after the dispatcher applies reloads.
- Subscriber list is **enumerated explicitly in Q11** rather than
  left to "grows as needed" — the codebase audit found caches
  the original draft missed.

---

### Q5 — Eager or lazy GPU re-upload?
**Decision: lazy. Drop on invalidate; re-upload on first use after
the swap.**

Eager re-upload guarantees no first-use flicker but pays upload cost
for every asset that's currently off-screen. Lazy re-upload trades a
1-frame flicker on first use for zero cost on off-screen assets;
at 60 fps the flicker is invisible.

Eager becomes a future opt-in (`AssetStore::reupload_eagerly`) when
profile data shows it matters. Lazy is the right v1 default.

---

### Q6 — Identity preservation across reload
**Decision: a `Handle<T>` survives reload. Never reissue.**

Every consumer of `Handle<T>` — components in the ECS, materials
referencing textures, scene serialization, the editor inspector —
keeps working without rewiring. The only state that changes is the
asset *behind* the handle.

This is enforced at the type-system level by exposing only
`AssetStore::replace<T>(handle, value)` as the reload entry point;
there is no `remove + re-insert` path that would issue a new handle.
A future commit can extend with a `try_replace` for cases where the
caller wants to detect a stale handle, but the default flow preserves
identity unconditionally.

---

### Q7 — Determinism and replay
**Decision: reloads are *inputs*, not noise. Live determinism is
defined relative to the input stream including reload events.
Replay mode locks reloads entirely.**

Pillar 4 (determinism) requires that given the same seed and input
stream, the simulation produces identical state across runs. A
reload event is part of that input stream — like a keystroke or
mouse click. Two live runs that observe a reload of asset X on
different frames are not "non-deterministic"; they're observing
different input streams. The runs would diverge from a fresh
re-execution because the reload-frame timing is part of what the
deterministic seed has to absorb.

For deterministic replay (the tested CI property), reloads are
forbidden during a replay session. The replay reads back a
recorded input stream — including the timing of any reloads that
happened during the original run — and the engine must reproduce
the original state bit-for-bit.

This decision requires a new editor session state:
`EditorPlayState::ReplayingDeterministic`. The codebase audit
(`src/editor/session.rs:21`) confirmed only `Editing`/`Playing`/
`Paused` exist today. **The new variant is part of the M7.0
pre-flight.** Without it, Q7's lock is unenforceable.

When the editor is in `ReplayingDeterministic`, the
`ReloadDispatcher` buffers compile results without applying them.
On exit from replay, the buffer is either drained (if the user
chose "exit replay and apply pending reloads") or discarded
(if the user chose "exit replay and stay on the recorded
state").

The CI determinism harness compares per-tick state across runs of
the same recorded input stream. Reload-induced state changes are
part of the recorded stream and therefore part of the verified
property — non-determinism in reload application would surface
as a test failure, not as silent drift.

---

### Q8 — What are the explicit non-goals for v1?
**Decision: defer the following with named owners.**

| Non-goal | Owner |
|----------|-------|
| Code hot-reload (Rust crates, fn-pointer fixup) | Future ADR; M11+ |
| Diff-based partial reloads (mip levels, mesh sections) | M7.5+ optimization |
| Compile thread pool (single-threaded worker for v1) | M8+ |
| Cross-process editor + IPC reload transport | M8+ |
| Networked hot-reload (game on console, edit on PC) | M9+ |
| Asset version migration (always full recompile in v1) | Captured by M10 |
| Code-reload-aware ECS state preservation | M11+ |
| Eager GPU re-upload | Future opt-in when profiling demands |

---

### Q9 — Phasing
**Decision: six milestones. M7.0 is new pre-flight; M7.5 is the
equivalence harness. Each milestone produces a demoable artifact.**

| Week | Milestone | Demoable result |
|------|-----------|-----------------|
| **M7.0** | Pre-flight infrastructure (Q10) | `AssetStore::replace<T>` lands; `Engine::on_reload_dispatch` hook lands; `EditorPlayState::ReplayingDeterministic` variant lands; per-cache invalidation strategy table lands as code (Q11). All four pieces have unit tests. No reload yet — this is the runway. |
| **M7.1** | FileWatcher + change-event pipeline | Editor logs `FileChange { ... }` for every asset save. M7.1 deliverable also includes a 24-hour raw-event capture across the project's asset edits, which sets the M7.1.1 calibration constant for the coalescing window. |
| **M7.2** | CompileQueue worker + reverse index + result channel | Editor logs successful + failed compile results with per-asset generation. |
| **M7.3** | ReloadDispatcher + InvalidationBus + handle-keyed cache eviction + texture hot-reload e2e | Save a texture in an external editor → spinning crate in `textured_scene` updates within one frame. |
| **M7.4** | Scenes + materials + shaders + meshes; editor protocol codec for `AssetInvalidated` (Q13); "Hot-Reload Status" editor pane; deterministic-replay lock; `World::events` integration (Q12) | Every asset kind reloads; editor shows reload state and recent errors; replay mode locks reloads; user-game code observes reload events. |
| **M7.5** | Equivalence harness (Q14) | CI test asserts hot-reloaded state ≡ clean-reload state for a sample project on every commit. |

Six weeks instead of four. The added time is honest engineering — the
agent-found infrastructure prerequisites and the equivalence harness
are both load-bearing.

---

### Q10 — Pre-flight infrastructure (M7.0)
**Decision: four pieces of engine infrastructure must land before
M7.1 can start.**

The codebase audit found that the original ADR assumed all four
existed. They do not. Implementing them is M7.0; M7.1 cannot start
until M7.0 ships.

**1. `AssetStore::replace<T>(handle, value) -> Option<T>`**

A new public method on `src/assets/store.rs::AssetStore`. The
existing `AssetPool<T>` is a generational sparse-vec with `insert`,
`get`, `get_mut`, and `remove`. `replace` is implemented as:

```rust
pub fn replace<T: 'static>(
    &mut self,
    handle: Handle<T>,
    value: T,
) -> Option<T> {
    let pool = self.get_pool_mut::<T>();
    pool.replace(handle, value)
}
```

The pool method's contract: if the handle's generation matches the
slot's current generation, replace the value and return the old
one without bumping generation. If the generation has gone stale,
return `None` and leave the slot untouched. **Identity is
preserved unconditionally** — the handle the caller passed in
remains valid after the call.

Tests:
- Replace returns the old value
- Handle remains resolvable to the new value
- Generation does NOT change
- Replace on a stale handle returns `None`

**2. Frame-start hook in `Engine::run`**

Add a new method `Engine::on_reload_dispatch<F>(f: F)` where `F:
FnMut(&mut AssetStore, &OpenGLRenderer)` is called between
`end_frame()` and the next iteration's `begin_frame()`. The
ReloadDispatcher registers a closure that drains the compile
result channel and applies replacements.

Alternative considered: dispatch inside `submit_frame()`. Rejected
because it couples the dispatcher to the renderer and makes the
replay-mode lock harder to enforce (replay mode shouldn't even
run the dispatcher).

The hook is a single new field on `Engine` plus 3 lines in the
loop. Tests cover: hook fires once per frame, hook can mutate
AssetStore, hook errors don't crash the loop.

**3. `EditorPlayState::ReplayingDeterministic`**

Extend `src/editor/session.rs::EditorPlayState` from
`{ Editing, Playing, Paused }` to
`{ Editing, Playing, Paused, ReplayingDeterministic }`. The session
exposes `play_state_locks_reloads(&self) -> bool`, returning
`true` for `ReplayingDeterministic` and `false` for the other
three. The dispatcher checks this before draining its channel.

The new variant is required for Q7's lock to be enforceable. The
deterministic-replay engine that *uses* this state lands in M8;
M7.0 only adds the variant, not the engine.

**4. Per-cache invalidation strategy table (Q11)**

Enumerated in Q11 below. Lands as code in M7.0 so M7.3 can wire
the InvalidationBus subscribers to actual eviction code.

---

### Q11 — Per-cache invalidation strategies
**Decision: every cache the renderer maintains has an explicit
invalidation strategy. Some are handle-keyed (direct eviction);
others are not (custom logic).**

The original draft assumed all caches were handle-keyed and "evict
by handle" was a one-line pattern. The codebase audit found this is
false for several caches.

| Cache | File:line | Keyed by | On reload of … | Eviction strategy |
|-------|-----------|----------|----------------|-------------------|
| `frame_texture_cache` | `opengl.rs:633` | `TextureHandle` | the matching `Texture` | Direct: `cache.remove(&handle)` |
| `frame_normal_map_cache` | `opengl.rs` (added in M4) | `NormalMapHandle` | the matching `NormalMap` | Direct |
| `frame_mesh_cache` | `opengl.rs` | `MeshHandle` | the matching `MeshData` | Direct; also `gl.delete_vao` for the dropped GpuMesh |
| `frame_program_cache` | `opengl.rs` | `ShaderHandle` | the matching `Shader` | Direct; also `gl.delete_program` for the dropped program ID + cascade to `uniform_cache` |
| `uniform_cache` | `opengl.rs:666` | GL program ID (`u32`) | a shader (cascade from `frame_program_cache`) | Indirect: keyed by program ID, evicted when its program is freed |
| `terrain_patch_mesh_cache` | `opengl.rs:675` | `TerrainPatchMeshKey` | terrain heightmap or terrain material | Custom: full-clear on any heightmap-bearing or terrain-material reload (rare event; full-clear is fine for v1) |
| `unit_plane_mesh` | `opengl.rs` | `Option<GpuMesh>` (singleton) | n/a (procedural) | Never invalidated — no asset behind it |

The `InvalidationBus` subscriber set is wired in M7.0 to call the
strategies above. New caches added later must declare their
strategy in this table or fail review.

---

### Q12 — Game code reload notifications
**Decision: game code observes reloads via `World::events`,
matching the existing event-bus pattern.**

The user's per-frame game closure already receives `&mut World`,
which exposes an event bus (`World::events`). The
`AssetInvalidated` event is emitted onto that bus by the
ReloadDispatcher (in addition to the engine-internal
InvalidationBus that handles GPU caches and the editor inspector).

Game code that wants to react reads the event:

```rust
engine.run(renderer, move |world, renderer, input, dt| {
    for event in world.events.iter::<AssetInvalidated>() {
        match event.kind {
            AssetKind::Scene => /* re-resolve entity refs */,
            _ => {}
        }
    }
    // ... rest of game tick
});
```

Adopting the existing pattern means no new API surface — the engine
keeps one event bus for game-observable events, the renderer keeps
one for internal cache invalidation.

---

### Q13 — Editor protocol codec for reload events
**Decision: `AssetInvalidated` and `CompileFailed` ride on the
existing line-based editor protocol with a fixed payload shape.
Per-event payload bound below 1 KB.**

The existing `src/editor/protocol.rs` is a line-oriented `|`-delimited
encoder. New event kinds:

```text
ASSET_INVALIDATED|<kind>|<handle_index>|<logical_name>
COMPILE_FAILED|<kind>|<logical_name>|<error_message>
```

`error_message` is a quoted string with `|` and `\` escaped. Total
encoded length is bounded by `logical_name` length + a typical 200-
character error message — well under 1 KB. This stays inside the
50ms coalescing window's bandwidth budget even in worst-case bursts.

Larger payloads (asset preview thumbnails for the inspector) flow
on a separate request-response channel, not the broadcast event
stream. The editor inspector requests previews on demand; bursts of
reloads don't generate burst thumbnails.

---

### Q14 — Scene-reload entity preservation
**Decision: when a scene asset reloads, the runtime World preserves
existing entities, reapplies authored components from the new
snapshot, deletes entities removed from the authored snapshot, and
adds entities new in the authored snapshot.**

The `EditorSceneSession::last_world_map` (`src/editor/session.rs:38`)
already maintains the mapping from authored entity IDs to runtime
entity IDs. On scene reload:

1. Decode the new scene payload into a `SceneDocument`.
2. Diff against the previous authored snapshot:
   - **Modified authored entities**: reapply the new component values
     to the corresponding runtime entity (preserving runtime-only
     components — physics velocity, AI state, etc.).
   - **Added authored entities**: spawn new runtime entities, extend
     the mapping.
   - **Removed authored entities**: despawn matching runtime entities,
     contract the mapping.
3. Emit `AssetInvalidated { kind: Scene }` so game code can re-resolve
   entity refs using the (mostly stable) mapping.

The "preserve runtime entities" choice is the right default because
runtime gameplay state often outlives any single authored snapshot
(active dialogue trees, in-flight projectiles, NPC pathfinding
state). A "reset to authored" button in the editor explicitly
discards runtime state and rebuilds from scratch — that's the
escape hatch when preservation produces wrong behaviour.

The `EditorSessionRebuildRequired` error already exists for cases
where the diff is too complex (added/removed components on a type
the world hadn't seen yet) — those fall back to a full rebuild. M7.4
extends the diff logic; the fallback path is the safety net.

---

### Q14a — Refinement: hardware-portable diff algorithm
**Decision: the scene-reload diff runs in O(n_old + n_new) time and
O(n_old + n_new) transient memory, with no SIMD/AVX requirements,
no parallelism, and no allocations on the steady-state hot path
when the entity set is unchanged.**

This refinement is folded in alongside the original Q14 because
the project charter constrains v1 to run well across the full
i3-equivalent through i9-equivalent CPU spectrum. An O(n²) diff
that's invisible on a 12-core i9 is a 200ms editor stall on an i3
with a 50K-entity open-world streamed scene. Decisions about the
diff algorithm are hard to reverse later (every editor workflow
becomes implicitly tuned to whatever performance shape it has on
day one), so they get specified up front.

**Algorithm**

```text
diff_scene(authored_old: &SceneDocument,
           authored_new: &SceneDocument,
           last_world_map: &[(u64, Entity)])
        -> SceneDiff

  // 1. Build hash sets from the two authored snapshots. O(n_old + n_new).
  let old_ids: HashSet<u64> = authored_old.entities.iter().map(|e| e.id).collect();
  let new_ids: HashSet<u64> = authored_new.entities.iter().map(|e| e.id).collect();

  // 2. Three-way partition. Each membership check is O(1) amortized.
  let to_remove: Vec<u64> = old_ids.difference(&new_ids).copied().collect();
  let to_add:    Vec<u64> = new_ids.difference(&old_ids).copied().collect();
  let to_patch:  Vec<u64> = new_ids.intersection(&old_ids).copied().collect();

  // 3. Index the runtime map so the apply phase can look up entities
  //    by editor id in O(1). Built lazily; skipped if to_remove +
  //    to_patch are both empty (i.e. only adds — no map lookups needed).
  let runtime_lookup: Option<HashMap<u64, Entity>> = if to_remove.is_empty()
                                                    && to_patch.is_empty() {
      None
  } else {
      Some(last_world_map.iter().copied().collect())
  };
```

The lazy `runtime_lookup` is the cache-locality concession to the
common case: scenes that authored a single transform tweak shouldn't
pay for hashing 50K entries. For small scenes the linear-scan path
in `EditorSceneSession::patch_world` already wins on cache locality;
the diff path only steps up to a HashMap when adds/removes/patches
demand random-access lookups.

**Complexity bounds**

| Quantity              | Bound                       | Reason                                |
|-----------------------|-----------------------------|---------------------------------------|
| Time (diff phase)     | `O(n_old + n_new)`          | Two HashSet builds + one partition.   |
| Memory (transient)    | `O(n_old + n_new)`          | Two HashSets + (sometimes) HashMap.   |
| Allocations (no-op)   | `0` after the no-change fast path detects equality. | `authored_old == authored_new` short-circuits. |
| Parallelism required  | None.                       | Single-threaded, runs at frame boundary. |
| SIMD required         | None.                       | Scalar code only.                     |

The "0 allocations on no-op" property matters because the file
watcher emits Modified events on saves that don't actually change
the scene's authored content (editor "save all" sweeps, IDE
on-blur re-saves). The dispatcher should detect content equality
on the decoded `SceneDocument` and short-circuit before the diff
phase even runs — not by hashing the document (more allocation),
but by reusing the byte-identical compiled-payload comparison the
generation counter already does.

**Hardware-portability guarantees**

- **i3-class CPU (4 cores, no AVX-512)**: a 50K-entity scene's
  diff completes in single-digit milliseconds; HashMap rehashing
  is the dominant cost, and `HashMap` (in-house) uses the standard
  AHash-style scalar mixing that doesn't require SIMD.
- **Memory pressure**: peak transient is ~72 bytes per authored
  entity (two `HashSet<u64>` plus one `HashMap<u64, Entity>`).
  At 100K entities — well past the realistic upper bound for a
  hand-authored scene — that's 7 MB of one-frame allocation.
  Below the threshold where any low-end machine notices.
- **No high-end-only assumption**: the diff doesn't pre-allocate
  multi-megabyte scratch buffers, doesn't memoize across reloads
  (each diff is independent), doesn't depend on cache sizes that
  vary across CPU tiers. The same code path runs identically on
  i3 and i9 — the i9 just finishes sooner.

**Why over-engineer the diff at all instead of just rebuilding?**

Full rebuild is O(n) too, but its constant factor is dominated by
component-system allocation, which is 10–100× the cost of a hash
lookup. On an i3 with a streamed open-world scene, full-rebuild
would frame-drop visibly on every save. The diff path's job is to
make small edits feel instant on the slowest target hardware, not
just acceptable on the fastest. That's the Pillar 2 promise.

The full-rebuild path stays as the explicit safety-net fallback
(unsupported components, world-shape divergence, or a "reset to
authored" button the developer presses on purpose). It's not the
hot path; it's the escape hatch.

---

### Q15 — Equivalence harness as load-bearing CI test
**Decision: a property test verifies hot-reloaded state ≡
clean-reload state on every commit. Without this test, the Q2
promise is decorative.**

```text
Property test: hot_reload_equivalence
  Given:  a sample project P with N assets and M materials
  Setup:  load P; render a fixed scene + camera + lighting
          configuration; capture rendered framebuffer as F_clean.
  When:   re-load P from disk again; randomly perturb K asset
          source files (no semantic changes — re-save bit-equal
          contents); let the hot-reload pipeline drain.
  Then:   re-render the same scene; capture F_hot.
          Assert pixel_difference(F_clean, F_hot) < ε per
          channel for every pixel.
          Also assert AssetStore content matches per-asset.
```

The harness lives in M7.5 with its own deliverable. It runs in CI on
every commit. The sample project starts as a tiny scene (cube, floor,
two materials) and expands as M7 progresses — by M7.4 it covers all
six asset kinds.

This test is the only enforcement of Q2. ADR 0003 Pillar 4 says
backend parity is sacred; this is the test that makes Pillar 2's
"hot reload that doesn't lie" sacred too.

---

## Guardrails

- **Hot-reloaded user-observable state ≡ clean-reload state.** This
  is the Q2 promise; every implementation choice is evaluated
  against it. The Q14 equivalence harness is the test that keeps
  this load-bearing.
- **Editor never freezes.** All file-system, compile, and upload
  work happens off the main frame loop. Sync points are at frame
  boundaries only. CI tests assert that the editor's frame loop
  time budget never spikes during a reload burst.
- **Zero external crates** holds. Win32 `ReadDirectoryChangesW`
  directly; `std::sync::mpsc` from std; in-house everything else.
- **Identity preservation.** Reload never reissues a `Handle<T>`.
  Tests pin this property at the `AssetStore::replace` boundary.
- **Determinism preservation.** Reloads happen only at frame
  boundaries; replay mode locks reloads entirely. Q14 harness pins
  this in CI.
- **Failure surfaces, not silences.** Compile errors, watcher
  errors, GPU upload errors → typed errors on the InvalidationBus
  → editor pane. The previous asset stays live. Never crash.
- **Pre-flight infrastructure (Q10) lands first.** No file-watching
  code merges before M7.0 ships `AssetStore::replace`, the frame
  hook, the replay state, and the per-cache invalidation table.
- **Reverse index built explicitly, not borrowed.** The CompileQueue
  owns its `HashMap<PathBuf, Vec<LogicalName>>`. The asset
  compiler's per-asset dependency lists are inputs, not the index
  itself.
- **Hardware-portable performance budget.** The hot-reload
  pipeline runs identically across the i3-equivalent through
  i9-equivalent CPU spectrum the engine targets. No SIMD/AVX
  dependencies, no cache-size assumptions, no parallelism that
  scales above 4 cores, no allocation patterns that depend on
  high-end memory bandwidth. Diff/dispatch algorithms have
  complexity bounds documented at the decision site (see Q14a),
  and CI runs the equivalence harness on a constrained-CPU
  configuration so a regression on the slow target lands as a
  test failure, not a runtime stutter on a developer machine no
  one tests on.

## Open questions

- **The 50ms coalescing window** is a v1 starting point. M7.1's
  raw-event capture sets the calibrated value. If real DCC tools
  cluster around different timings (Photoshop ≠ Substance Painter
  ≠ Blender ≠ VS Code), the constant may grow into a
  per-extension table. Tracked as M7.1.1.
- **Watcher recursive descent depth.** v1 watches a single root
  (the project's `source/` directory) recursively. Multi-root
  watching extends the design without changing it; tracked as M9
  if content layouts demand it.
- **Compile-error UI surface for chained dependencies.** When a
  shader fails to compile and three materials depend on it, the
  editor shows three "compile failed" entries that all blame the
  shader. The right UX is "show the root cause once, list the
  dependents" — tracked as M7.4 polish.
- **Generalising the InvalidationBus to support code reload.** The
  bus shape (synchronous broadcast, typed event, subscriber list)
  generalises in principle to "code-side cache invalidated" events
  for the future code-reload ADR. The future ADR verifies this and
  may extend the bus with new event kinds. v1 doesn't pre-build
  for it.

## Related
- Project charter: `docs/PROJECT_CHARTER.md`
- Foundation ADR: `docs/adr/0001-engine-editor-foundation.md`
- Material pipeline ADR: `docs/adr/0002-material-pipeline.md`
- Differentiation and competitive posture:
  `docs/adr/0003-differentiation-and-competitive-posture.md`

## Revision history
- **Draft 1** (initial): Q1–Q9, three-thread model, four-week
  phasing.
- **Draft 3** (M7.4.1 Scene-reload prep): Added Q14a
  (hardware-portable diff algorithm with O(n) complexity bound,
  zero-allocation no-op fast path, no SIMD/AVX dependencies)
  in response to the i3-through-i9 portability constraint
  added to the project charter. Added the "hardware-portable
  performance budget" guardrail enumerating what the diff
  algorithm and pipeline are forbidden from depending on. No
  prior decision was reversed; Q14's "preserve runtime entities"
  framing remains the headline rule. Q14a specifies the
  algorithm shape that satisfies it across the target hardware
  spectrum.
- **Draft 2**: Independent codebase review found
  four load-bearing infrastructure prerequisites the original
  draft assumed were in place but weren't — those become the new
  Q10 (M7.0 pre-flight). Added Q11 (per-cache invalidation
  strategies enumerated against actual code), Q12 (`World::events`
  integration), Q13 (editor protocol codec), Q14 (scene-reload
  entity preservation), Q15 (equivalence harness CI test).
  Tightened Q2 (narrowed promise scope to user-observable state),
  Q4 (path canonicalization, bounded queue, buffer overflow
  reconciliation, calibrated coalescing window), Q7 (reloads as
  inputs framing). Phasing grew from four weeks to six (M7.0 +
  M7.5). No scope cut.
