# ADR 0001: Engine/Editor foundation decisions

## Status
Accepted

## Context
The project goal is a zero-dependency, Windows-only Rust engine + editor platform for 3D open-world RPG development. We need immediate decisions for architecture sequencing.

## Decision

### Q1 — Lock engine as a library now, or refactor later?
**Decision: lock as a library now.**

**Reasoning**
- Prevents editor/runtime coupling from becoming accidental technical debt.
- Forces stable API seams early: world update, rendering submission, asset IO, scene serialization.
- Makes testing and eventual tooling automation easier.

**Immediate consequences**
- Engine exposes library APIs for lifecycle/update/render hooks.
- Front-ends (game launcher/editor) are hosts over the same core library.

---

### Q2 — Editor + engine same process or separate (IPC)?
**Decision: same process first, with a transport boundary abstraction for later IPC split.**

**Reasoning**
- Faster iteration for early tooling (inspector, scene graph, gizmos).
- Lower complexity while core data model is still evolving.
- Boundary abstraction keeps the door open for out-of-process editor later.

**Immediate consequences**
- Define editor-facing service traits instead of direct global access.
- Keep command/event interfaces serializable so IPC can be added later.

---

### Q3 — First milestone: playable scene or editable scene?
**Decision: editable scene first.**

**Reasoning**
- The differentiator is ecosystem/tooling, not just runtime loop.
- Validates the critical requirement: `entity -> component -> field -> value` editing.
- Produces reusable workflow foundation for all future gameplay work.

**Immediate consequences**
- Milestone M1 focuses on project + scene + inspector roundtrip.
- Gameplay richness is deferred until editing flow is stable.

## Milestone decomposition

### M1 — Editable Scene Vertical Slice
- project file (`.hproj`) load/save
- scene document load/save
- entity/component hierarchy panel model
- inspector get/set by dynamic path
- apply changes back to runtime world

### M2 — Play-in-Editor Loop
- start/stop simulation
- deterministic world reset from scene snapshot
- hot-reload modified component values into paused world

### M3 — Rendering & World Scale Foundations
- backend parity checks (software/OpenGL/Vulkan feature matrix)
- streaming-ready chunk/LOD interfaces
- asset dependency graph + import pipeline contracts

## Guardrails
- No external crates.
- Windows-only runtime path.
- Typed ECS remains source of truth; editor stores editable projections, not divergent runtime state.

## Related
- Project charter: `docs/PROJECT_CHARTER.md`
