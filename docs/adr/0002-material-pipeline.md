# ADR 0002: Material pipeline & content authoring inspiration

## Status
Accepted

## Context
The engine has a working software renderer, an OpenGL backend with mesh/texture
upload helpers, and a Vulkan stub. The asset system has handle types and an OBJ
loader, but no GPU-side material model beyond per-vertex normals and a single
texture sample. Before the renderer abstraction grows further, we need an
explicit decision on what a "material" is in this engine and how content is
authored against it.

The motivating question: what design language do we borrow from? The two
mature reference points are Substance Designer (procedural material authoring
as a typed node graph) and Substance Painter (per-mesh layered composition
driven by baked mesh maps). They are complementary, not alternatives.

This ADR locks the high-level material-pipeline direction. Specific shader
math, file formats, and editor UI layout are deliberately out of scope here
and will be settled in follow-on ADRs as each phase begins.

## Decision

### Q1 — What inspirational model do we follow for material authoring?
**Decision: Substance Designer + Substance Painter, applied as engine
patterns rather than feature-for-feature copies.**

**Reasoning**
- Both tools are battle-tested across the industry; their pin-typing,
  layering, and procedural-mask discipline are widely understood by content
  creators.
- Designer's typed-graph discipline solves the "is this pixel a color or a
  normal?" class of bugs at compile time, not at render time.
- Painter's mesh-map substrate (curvature, AO, world-normal, position,
  thickness) is what makes procedural materials look mesh-aware instead of
  generic. Without it, "smart" materials cannot exist.
- The two tools stack cleanly: Designer authors a single material; Painter
  composes multiple materials onto a mesh; the engine consumes both.

**What we explicitly do not borrow**
- In-engine 3D painting (brush, projection, stamp). Authoring stays in DCC
  tools.
- Iray / path-traced preview mode.
- The full Substance node libraries (~300+ nodes for Designer, ~100+ for
  Painter). We grow node sets only when an actual material in the engine
  requires the node.
- The 4-pane SD layout. We steal "always-live preview", not the chrome.
- Visual scripting of gameplay logic. ECS systems remain code, not graphs.

---

### Q2 — What is a "material" in this engine?
**Decision: a layered stack, not a flat texture set.**

A `Material` is an ordered list of `Layer`s. Each layer carries:
- five PBR channels: `base_color`, `normal`, `roughness`, `metallic`, `height`
- a `mask` (grayscale) controlling where the layer applies
- a `blend_mode` per channel (color channels lerp; normals UDN-blend; heights
  max-blend)

The composited result drives a single shading pass. v1 caps the stack at
8 layers per material as a budget guardrail; revisit if content pressure
demands more.

**Reasoning**
- Matches Painter's mental model and how authored content arrives.
- Channel-aware blending (the fact that two normals do not lerp) is forced
  to be explicit in the data model rather than implicit in shader code.
- A flat texture set is a degenerate 1-layer material — no special case
  needed.

---

### Q3 — Shading model and color-space discipline
**Decision: Disney metallic-roughness PBR, ACES tonemap, strict sRGB-vs-linear
discipline at asset import.**

- `base_color`: sRGB texture, decoded to linear at sample time.
- `normal`, `roughness`, `metallic`, `height`, AO, curvature, masks: linear
  textures, no gamma curve.
- Tonemapping is applied uniformly across all backends so the editor
  viewport matches runtime output.

**Reasoning**
- Disney metallic-roughness is the universal interop format. Every DCC tool
  exports it; every PBR texture pack assumes it. Inventing our own model
  would isolate the engine from existing content.
- Color-space mistakes are the single most common "why does it look wrong"
  failure mode after normal-map orientation. Encode the convention in the
  asset header so the loader applies the right gamma deterministically.

---

### Q4 — Normal map handling
**Decision: normal maps are a distinct typed asset, not a `Texture`.**

- New asset type `NormalMap` separate from `Texture`. A graph or material
  pin typed `Color` will not accept a `NormalMap`, and vice versa.
- Engine canonical convention: **OpenGL Y+** (green channel points up).
  DirectX-style normal maps (Y-) are flipped at import time. Convention
  is recorded in the asset header and surfaced in the inspector.
- Tangent space stored on the vertex as `Vec4` (xyz = tangent, w = bitangent
  handedness sign), interpolated, normalized in the fragment shader.
- Tangent generation uses an in-house implementation matching the
  MikkTSpace reference algorithm. **No external crate dependency.**
  The MikkTSpace algorithm is openly published and self-contained at
  ~1k lines; reimplementing it preserves the charter's zero-dependency
  posture.

**Reasoning**
- The typed-pin distinction eliminates the most common normal-map bug
  (sampling normal data through a sRGB-decoding color pipeline) at the
  data-model level.
- MikkTSpace is the de facto industry standard for tangent generation;
  any DCC export can be assumed to assume MikkTSpace-compatible
  tangents. Hand-rolled tangent algorithms are the #1 source of
  normal-mapping seams.

---

### Q5 — Mesh maps as first-class assets
**Decision: every imported mesh produces a fixed set of baked mesh maps,
cached alongside the compiled mesh.**

The v1 set:
- `MeshMapAO` — ambient occlusion
- `MeshMapCurvature` — surface curvature

Deferred to v2:
- `MeshMapWorldNormal`, `MeshMapPosition`, `MeshMapThickness`

**Reasoning**
- Mesh maps are the substrate procedural masks read from. Without them,
  Painter-style "smart" behavior (edge wear, dirt-in-cavities) cannot exist.
- AO and curvature are the two highest-value maps for typical game content.
  The remaining three are useful but can be added when a specific material
  needs them.
- Bake-once-at-import (rather than real-time) keeps the runtime cost zero
  and avoids needing a baking pipeline in the hot path.

---

### Q6 — Procedural masks
**Decision: masks are tiny typed expressions over mesh maps, evaluated at
material-build time and baked into a mask texture.**

- A mask expression has `Grayscale` output and reads from any mesh map plus
  constants.
- Masks compose: `remap(curvature, 0.5, 0.9) * (1 - threshold(ao, 0.3))`.
- Evaluation happens once per material per mesh, not per frame. The
  resulting texture is cached.

**Reasoning**
- Per-frame procedural mask evaluation has no upside for static authored
  content; bake-time evaluation matches Painter's actual model.
- Keeping masks as expressions (not full graphs) bounds complexity and
  keeps the serialized form small and human-readable.

---

### Q7 — Smart materials
**Decision: smart materials are serializable layer-stack presets with
exposed parameters that resolve against the target mesh's baked maps.**

- A smart material file is a layer stack + parameter declarations + default
  parameter values.
- "Drop on a mesh" resolves the masks against that mesh's mesh maps,
  producing a concrete `Material` instance.
- The editor's material browser becomes a library of these.

**Reasoning**
- This is Painter's content distribution model in serialized form. It is
  what makes a material library useful across projects rather than
  per-mesh bespoke.
- Exposed parameters give content creators a stable API surface independent
  of the layer-stack internals.

---

### Q8 — Frame graph as a typed node graph
**Decision: the renderer's `RenderFrame` evolves into a typed frame graph
with per-pass thumbnails surfaced to the editor.**

- Render passes are nodes. Their inputs and outputs are typed resource
  handles (`Texture(desc)`, `Buffer(desc)`).
- The frame graph compiles to a topologically ordered execution plan,
  validates type compatibility across edges, and allocates transient
  resources.
- When the editor is attached, each pass output is captured as a thumbnail
  and streamed to a frame inspector panel. Click a thumbnail to set it as
  the main viewport output (Substance Designer's "view a node" gesture).

**Reasoning**
- The current flat `RenderFrame` vec and never-called `submit_frame` method
  are a known design smell from the ADR-0001-era audits. The Substance-style
  frame graph solves the smell as a side effect.
- Per-pass thumbnails turn the editor into a render debugger by default,
  which is high-leverage during the long backend-parity work ahead.

## Phased rollout

These extend the existing M1–M3 milestone sequence in ADR 0001.

### M4 — Material pipeline foundation
- Frame graph + typed pins + per-pass thumbnails
- Vertex tangents (in-house MikkTSpace-equivalent)
- `NormalMap` as a typed asset distinct from `Texture`
- OpenGL shader path with normal mapping
- One reference textured + normal-mapped material visible in the editor
  frame inspector
- Estimated: 4–6 weeks

### M5 — PBR + color-space discipline
- Disney metallic-roughness BRDF in the OpenGL shader path
- Asset-header color-space tagging; loader applies sRGB decode where flagged
- ACES tonemap applied uniformly across all backends
- Mesh map baking on import (AO, curvature)
- Estimated: 4–6 weeks

### M6 — Layered materials & smart materials
- `Material` becomes a layer stack at runtime
- Channel-aware blending in the fragment shader (lerp colors, UDN-blend
  normals, max-blend heights)
- Procedural mask expressions evaluated at material-build time
- Smart-material file format (serializable layer stack + exposed parameters)
- Editor material browser
- Estimated: 6–8 weeks

Total phased arc: roughly 14–20 weeks for a coherent Substance-inspired
material pipeline. Each phase is independently shippable and unblocks the
next.

## Guardrails
- **Zero external crates.** Every algorithm (MikkTSpace, ACES, BRDF
  evaluation, Sobel filter for height-to-normal, etc.) is implemented
  in-house from published references.
- Typed pins are enforced at graph-compile time, not at runtime.
- Color-space convention is recorded in the asset header, not inferred.
- Backend feature parity: any new pass added to the frame graph must have
  a software-renderer implementation, even if degraded. The software
  backend stays the parity baseline.
- Layered material stack capped at 8 layers in v1; revisit only under
  content pressure.
- Mesh map baking happens at import, not runtime.

## Out of scope for this ADR
- In-engine 3D painting tools.
- Specific shader math (will be settled in M5 ADR).
- Smart-material file format details (will be settled in M6 ADR).
- Editor UI layout for the material browser and frame inspector.
- Compressed texture formats (BC5, BC7). Uncompressed for v1; revisit later.
- Triplanar projection at runtime. Add when terrain or hand-modeled rocks
  demand it.
- Anchor points / cross-layer mask references. v2.

## Related
- Project charter: `docs/PROJECT_CHARTER.md`
- Foundation ADR: `docs/adr/0001-engine-editor-foundation.md`
