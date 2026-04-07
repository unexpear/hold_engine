# Project Charter: Engine + Editor Ecosystem

## What we are building
A from-scratch Rust game engine and editor platform to create and run **3D open-world RPGs**.

This includes both runtime and tooling:
- engine core (ECS, rendering, input, math)
- multi-backend rendering (software CPU + OpenGL + Vulkan)
- asset system
- world/scene handling
- editor (projects, files, inspector, scene editing)

## Why this exists
We are intentionally owning the full stack (OS to gameplay) to maximize long-term control, debuggability, and architectural clarity.

The goal is not a one-off game runtime. The goal is a reusable ecosystem that future projects can build on.

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

## Core tension (decision compass)
Every major decision should be evaluated against:

- control vs speed
- flexibility vs simplicity
- editor investment vs runtime investment

## Bottom line
This repository is not "just an engine." It is a full game development ecosystem, built from first principles.
