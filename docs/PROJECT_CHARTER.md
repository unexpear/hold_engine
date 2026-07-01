# Project Charter: Engine + Editor Ecosystem

## What we are building
A from-scratch Rust game engine and editor platform.

**Long-term ambition:** the best open-source engine in the world —
competitive with Unreal and Unity on the dimensions that matter most to
indie and professional developers. We do not race them on feature count;
we win developer trust on the dimensions where they hurt their users.

**Initial proving ground:** 3D open-world RPGs. The proving-ground game
keeps scope honest; the ambition keeps the architecture honest.

The artifact is both runtime and tooling:
- engine core (ECS, rendering, input, math)
- multi-backend rendering (software CPU + OpenGL + Vulkan)
- asset system
- world/scene handling
- editor (projects, files, inspector, scene editing)

## Why this exists
We are intentionally owning the full stack (OS to gameplay) to maximize
long-term control, debuggability, and architectural clarity.

The goal is not a one-off game runtime. The goal is a reusable ecosystem
that future projects can build on, that survives the original author
leaving, and that earns the trust of developers who have been burned by
royalty-after-revenue clauses, render-pipeline fragmentation, brittle
hot reload, and editors that crash and lose work.

## Differentiating pillars
See `docs/adr/0003-differentiation-and-competitive-posture.md` for the
detail. Five fronts we explicitly compete on:

1. **Economics and trust.** No royalties. No subscriptions. No required
   online services. MIT licensed and stays MIT licensed.
2. **Iteration speed.** Sub-second editor cold start. Hot reload that
   doesn't lie. Shaders compiled at import, never at runtime.
3. **Editor–runtime unification.** One codebase, one scene
   representation, one execution model. "Works in editor, breaks in
   build" is structurally precluded.
4. **Determinism and debuggability.** Deterministic simulation by
   default. Frame inspector and per-pass thumbnails as first-class
   debug features. Typed errors with context.
5. **Footprint and source quality.** Engine binary under ten megabytes.
   Open formats end-to-end. Documentation is part of the artifact.

## Product principles
- free to use forever
- no royalties
- no subscriptions
- no required online services
- no dependence on paid infrastructure for core workflows
- offline-first core workflows for development and runtime

## Licensing
- the engine core is distributed under the MIT License
- commercial and non-commercial use are allowed under that license
- the project should preserve the no-royalty, no-subscription, and no-required-online-services posture for core workflows


## Hard constraints
### Technical
- Rust
- Windows-only runtime
- zero external crates
- raw Win32 and raw graphics APIs

### Architecture
- ECS-first (not OOP-first)
- data-driven design
- modular systems
- engine remains a library (hosted by editor/launcher)

### Rendering
- software renderer (CPU)
- OpenGL via manual WGL setup and function loading
- Vulkan via raw FFI

### Material pipeline & content authoring
- Substance Designer + Substance Painter as the design-language reference
- typed pins at the data-model level (a normal map is not a color)
- layered materials with channel-aware blending
- mesh maps (AO, curvature, ...) baked at import as the substrate for
  procedural masks
- normal maps as a distinct typed asset; engine canonical convention is
  OpenGL Y+; in-house MikkTSpace-equivalent tangent generation
- Disney metallic-roughness PBR; ACES tonemap; strict sRGB-vs-linear
  discipline at asset import
- frame graph as a typed node graph with per-pass thumbnails surfaced to
  the editor (Substance Designer's "view a node" gesture)
- detailed direction and phased rollout: `docs/adr/0002-material-pipeline.md`

### Hot-reload (Pillar 2: iteration speed)
- asset hot-reload end-to-end: textures, normal maps, shaders, scenes,
  materials, meshes
- load-bearing promise: hot-reloaded state ≡ clean-reload state, bit
  for bit
- file watcher → compile queue (worker thread) → reload dispatcher
  (frame boundary) → invalidation bus
- `Handle<T>` survives reload; identity preserved unconditionally
- determinism preserved: reloads only at frame boundaries; replay mode
  locks reloads entirely
- code hot-reload deliberately deferred to a future ADR
- detailed architecture: `docs/adr/0004-hot-reload-architecture.md`

### Scope
- 3D open world is primary
- 2D support may exist as secondary
- scalable world features (LOD/streaming) are planned, with staged rollout

### Editor/tooling
- Unity/Godot-style workflow target
- project system, file/assets browser, scene editing, inspector
- reusable for multiple projects (not hard-coded to one game)

### Data access requirement
Support dynamic editor access and mutation by path:

`entity -> component -> field -> value`

while preserving typed ECS performance internally.

## Development posture
- Build from scratch.
- Use iterative milestones with explicit decisions.
- Favor harsh auditing/review over optimistic assumptions.

## Engineering principle: tastefully over-engineered
> Pay tax where the failure modes are bad. Spend velocity where they are not.

We over-engineer five places:
1. **Type safety at API boundaries** — the type system as design tool;
   invalid states become uncompilable.
2. **Error types** — named, contextual, typed; no `String` errors.
3. **Tests for invariants** — tests prove properties, not just paths.
4. **Documentation that survives the author** — ADRs for non-obvious
   decisions; intent comments on every public type.
5. **Long-horizon design** — decisions that are hard to reverse get
   spent on; decisions that are easy to revisit get shipped.

We do **not** over-engineer:
- Premature abstraction (one user is not a trait surface).
- Configurability that nobody asked for.
- Architecture diagrams divorced from code.
- Performance micro-tuning ahead of profiling.

See ADR 0003 for the full statement.

## Explicit non-goals
The roadmap actively refuses these so it doesn't drift into them under
pressure (see ADR 0003 for rationale):

- Visual scripting for gameplay logic.
- A marketplace.
- Per-install or per-revenue monetisation, ever.
- Mobile and console targets in the v1 generation.
- Photogrammetry and capture workflows.
- Replacing the developer's IDE.
- AAA-graphics features ahead of correctness.

## Core tension (decision compass)
Every major decision should be evaluated against:

- control vs speed
- flexibility vs simplicity
- editor investment vs runtime investment
- the five differentiating pillars (do we strengthen one or weaken one?)

## Bottom line
This repository is not "just an engine." It is a bid to be the best
open-source engine in the world, competitive with Unreal and Unity on
the dimensions that matter most to indie and professional developers,
built from first principles, owned end-to-end, free forever.
