# ADR 0003: Differentiation and competitive posture

## Status
Accepted

## Context
ADRs 0001 and 0002 established the engine's structural foundation: ECS,
multi-backend rendering, project + scene + inspector workflow, and a
Substance-inspired material pipeline. They settled *how* the engine is
built. They did not articulate *what the engine is competing for*, *who
its competitors are*, or *what makes it worth choosing over them*.

This ADR fills that gap. It elevates the project's ambition from "a
working Rust engine" to **the best open-source engine in the world,
competitive with Unreal and Unity on the dimensions that matter most to
indie and professional developers** — and records the principle that
tells us when to slow down and over-engineer something versus when to
ship and move on.

The proving-ground game stays a 3D open-world RPG (ADR 0001). That focus
is what keeps the engine from sprawling into "everything for everyone."
The ambition is broader; the scope discipline is the same.

## Decision

### Q1 — What is the engine actually competing for?
**Decision: developer trust over feature parity.**

We are not going to out-feature Unreal in our first decade. Unreal has
$10⁹+ in cumulative R&D and 30 years of accreted knowledge. Unity has
two decades of indie-tooling momentum. Brute-force feature catch-up is
a losing race for any solo or small-team project, and chasing it
produces shallow imitations rather than sharp tools.

What we *can* compete for is the trust of developers who have been
burned by closed-source engines, royalty surprises, render-pipeline
fragmentation, multi-minute compile cycles, brittle hot reload,
phone-home telemetry, and editors that crash and lose work. Those
developers are looking for an engine that:

- they can actually fork and modify
- doesn't change the rules under them
- doesn't lock them into proprietary formats
- starts up in under a second
- iterates without long pauses
- runs offline forever
- works the same in editor and runtime
- gives them legible, debuggable failure modes
- exists ten years from now

This is the war we're in. We win it by building an engine that doesn't
have the sharp edges that drive developers off Unreal and Unity.

---

### Q2 — Which Unreal/Unity pain points do we explicitly target as differentiators?

**Decision: thirteen pain points across five pillars. Each becomes a
load-bearing design constraint, not a "nice to have."**

#### Pillar 1: Economics and trust
- **No royalties, ever.** Not "below $1M revenue you don't pay" —
  zero, unconditional. (Already in the charter.)
- **No subscriptions.** Not for the engine, not for the editor, not
  for "professional features," not for source access. (Already in the
  charter.)
- **No required online services.** Project files, asset compilation,
  the editor, and the runtime work fully offline. No login,
  no account, no telemetry beacons. (Already in the charter.)
- **MIT licensed and stays MIT licensed.** Forks are encouraged.
  Redistribution is unrestricted. Source ships with every release.

#### Pillar 2: Iteration speed
- **Sub-second editor cold start.** The engine binary is small (zero
  external crates, raw OS bindings) and the editor is a thin shell
  over the engine library. Cold start budget: under one second on a
  development machine. Compare: Unreal 30s+, Unity 10s+.
- **Hot reload that doesn't lie.** Asset hot-reload (textures,
  meshes, shaders, scenes) is the v1 target. Code hot-reload comes
  later, designed deliberately rather than retrofitted. The principle:
  if hot-reload fires and produces a different state than a clean
  reload would, that is a bug, not a quirk. No "did the hot reload
  even apply?" anxiety.
- **Shader compilation is bounded and visible.** Shaders compile at
  asset-import time, not at runtime. First-frame shader stalls — the
  most-hated category of performance bug in modern engines — should be
  structurally impossible.

#### Pillar 3: Editor–runtime unification
- **One codebase. One scene representation. One execution model.**
  The editor is a host over the engine library (ADR 0001 Q1 already
  commits to this). Unity's editor-vs-build "works in editor, breaks
  in build" failure mode is structurally precluded.
- **Play-in-editor uses the same systems as the runtime.** No editor
  fast-paths that diverge from production behaviour.
- **Saving never loses work.** Crash recovery, autosave, and atomic
  scene writes are sacred. A power loss during save must leave either
  the previous scene or the new scene on disk — never a half-written
  one.

#### Pillar 4: Determinism and debuggability
- **Deterministic simulation by default.** Given the same seed and
  the same input stream, the simulation produces identical state
  bit-for-bit across runs. This unlocks reliable replays, networking,
  reproducible bug reports, and deterministic CI tests.
- **Frame inspector and pass thumbnails as first-class debug
  features.** The frame graph already produces these (ADR 0002 Q8). A
  developer should never have to "wonder what the renderer did" — they
  see every pass's output. Compare: RenderDoc is a separate tool with
  setup overhead; this is built in.
- **Errors are typed, named, and carry context.** No `String`-typed
  error returns. No silent fallbacks. The compiler is a design partner.

#### Pillar 5: Footprint and source quality
- **Engine binary stays under ten megabytes.** Zero external crates is
  not a stylistic choice; it is a discipline that produces small
  binaries, fast cold start, and complete auditability of every byte.
- **Open formats end-to-end.** Compiled assets are versioned, documented
  binary formats. Scene files are human-readable. There is no
  proprietary container the developer cannot inspect or migrate from.
- **Documentation is part of the artifact.** ADRs accompany every
  meaningful decision. Public APIs carry intent comments, not just
  mechanics. Code that references a non-obvious decision cites the
  ADR.

---

### Q3 — What does "tastefully over-engineered" mean in this codebase?

**Decision: we pay over-engineering tax in five specific places, and
nowhere else.**

The phrase carries a real distinction. It is not a license to abstract
everything; it is a license to spend extra effort where the engineering
quality compounds.

We over-engineer:

1. **Type safety at API boundaries.** A normal map cannot be a colour
   texture (ADR 0002 Q4). A frame graph resource handle carries its
   kind tag (ADR 0002 Q8). Two passes that conflict on a write are
   rejected at compile-graph time. The type system prevents whole
   categories of bugs at the cost of a few more lines per API.

2. **Error types.** Every error variant is named, carries the data
   needed to reproduce the failure, and has a `Display` message that
   tells the developer what to do. No `String` errors. No
   `anyhow::Error` veil.

3. **Tests for invariants, not just happy paths.** Tests prove
   properties: "tangents are orthonormal to normals," "the topological
   sort is deterministic," "DirectX-convention normal maps end up in
   OpenGL convention after construction." Each test names what
   invariant it locks down.

4. **Documentation that survives the original author.** Every public
   type has a doc comment explaining intent. ADRs record decisions
   with context. Code comments cite ADRs when the decision is
   non-obvious. A new contributor reading any file should be able to
   understand the *why*, not just the *what*.

5. **Long-horizon design.** When a decision will be hard to reverse,
   we spend the time up front to get the shape right. Renderer trait
   abstraction. Asset format versioning. Frame graph resource
   typing. We accept slower week-one velocity in exchange for not
   rewriting in year three.

We do **not** over-engineer:

- **Premature abstraction.** We do not build a generic `dyn Renderer`
  trait if only one backend uses the trait surface. Backends grow
  the abstraction by being added, not by being imagined. ADR 0002 Q8's
  PassContext is OpenGL-bound deliberately; the abstraction will
  emerge from the second backend's actual requirements.
- **Configurability that nobody asked for.** Defaults that are good
  enough for 95% of cases beat knobs that take 5% of users a week to
  understand.
- **Architecture diagrams divorced from code.** ADRs cite specific
  files and types. There is no separate architecture document that
  drifts from reality.
- **Premature performance optimisation.** We design data layouts for
  cache-friendliness, but we do not micro-tune until profiling
  demands it.

The principle, named:

> **Pay tax where the failure modes are bad. Spend velocity where
> they are not.**

A type-system bug that ships in a public API costs every downstream
user; a missing utility helper costs the developer who needed it for
ten minutes. The first deserves over-engineering tax; the second does
not.

---

### Q4 — What are the explicit non-goals?

**Decision: we publicly refuse the following directions, so the
roadmap doesn't drift into them under pressure.**

- **Visual scripting for gameplay logic.** Unreal Blueprints and
  Unity Visual Scripting (Bolt) have been tried; both scale poorly
  past ~50 nodes. Gameplay code stays in Rust. Designers compose via
  data (ECS components, scene files, parameter exposure on materials),
  not via node graphs.
- **A marketplace.** Marketplaces produce asset rot, version-skew
  bugs, and incentives that conflict with the developer-trust pillar.
  The community can host its own asset libraries; the engine project
  does not run one.
- **Per-install or per-revenue monetisation of any kind.** Even
  hypothetically. The project's economic model is donations,
  enterprise support contracts, or nothing — not a future "pro tier."
- **Mobile and console targets in the v1 generation.** Locked behind
  a future ADR if and when desktop parity is solid. The Windows-first
  posture (ADR 0001) is what makes the small team possible.
- **Photogrammetry and capture workflows.** Not in scope. Other tools
  do this well; we integrate via standard file formats.
- **Replacing the developer's IDE.** The engine ships an editor for
  scenes, materials, and assets — not a code editor. Code lives in
  whatever the developer's IDE of choice already is.
- **AAA-graphics features ahead of correctness.** Virtual shadow
  maps, hardware ray tracing, and Nanite-equivalents are planned but
  late in the sequence. They sit on top of a determinism, frame-graph,
  and asset-pipeline foundation that has to be solid first.

---

### Q5 — How do we sequence work toward the elevated ambition?

**Decision: keep the existing milestone numbering, but reorder the
post-M6 work to lead with the differentiating pillars.**

The current sequence (ADR 0001 + 0002):

- M1: Editable Scene Vertical Slice ✅
- M2: Play-in-Editor Loop (in progress)
- M3: Rendering & World Scale Foundations
- M4: Material pipeline foundation ✅
- M5: PBR + colour-space discipline
- M6: Layered materials & smart materials

The reordered post-M6 sequence, prioritising differentiators:

- **M7: Hot-reload spike.** Asset hot-reload (textures, meshes,
  shaders, scenes) end-to-end. Establishes the discipline that hot
  reload state matches clean-reload state. (Pillar 2.)
- **M8: Determinism harness.** Deterministic simulation tick;
  reproducible-replay test infrastructure; CI that catches
  non-deterministic regressions. (Pillar 4.)
- **M9: Editor frame-inspector UI.** Carry per-pass thumbnails
  (already produced by `FrameGraph::execute_with_capture`) over the
  editor protocol; render them in the editor viewport panel.
  (Pillars 3 + 4.)
- **M10: Asset format hardening.** Versioned compiled-asset envelope
  with migration support; documented binary formats; format-stability
  guarantees. (Pillars 1 + 5.)
- **M11: Save format and crash safety.** Atomic writes, autosave,
  recovery. Power-loss tests in CI. (Pillar 3.)
- **M12: Cross-platform abstraction extraction.** Pull `src/platform`
  behind a trait so non-Windows targets become possible. The
  Windows-only commitment in ADR 0001 gets revisited at this
  milestone, not before. (Pillar 1.)

Each of these is a multi-week project that produces a concrete,
demoable, tested artifact. None of them are "infrastructure
investment" in the bad sense — every one is felt by the developer using
the engine on day one of that milestone shipping.

## Guardrails

- **Differentiation pillars are load-bearing.** A feature that
  conflicts with one of the five pillars requires an ADR explaining
  why the conflict is necessary. Breaking a pillar without that ADR
  is a bug.
- **The Windows-only commitment (ADR 0001) is intact** until M12
  reopens it. Do not start non-Windows work before that.
- **Zero external crates** holds. Every algorithm is implemented in
  house. The `crossterm` and `ratatui` deps from the deleted Sandhold
  experiment have already been pruned from the workspace.
- **Every commit names the pillar it advances**, in either the
  message body or a referenced ADR. Drift is caught at review.
- **The proving-ground game (3D open-world RPG)** stays the lens for
  prioritisation. Features that are useful in the abstract but
  irrelevant to that game are non-priorities even when they're
  technically interesting.

## Open questions

- **Documentation surfaces.** This ADR sets ambition; should it have a
  matching public-facing rendered version (engine homepage, README)
  that surfaces the pillars to potential users? Likely yes; tracked
  alongside M9.
- **Comparison cadence.** When we claim "X seconds faster than
  Unreal" we need a reproducible benchmark. The benchmarks themselves
  are an artifact and should land before any marketing claim does.
- **Community governance.** When the project gathers contributors,
  the decision authority and ADR-review process need formal shape.
  Premature today; tracked as a future ADR rather than left implicit.

## Related
- Project charter: `docs/PROJECT_CHARTER.md`
- Foundation ADR: `docs/adr/0001-engine-editor-foundation.md`
- Material pipeline ADR: `docs/adr/0002-material-pipeline.md`
