# Hold Engine

A from-scratch Rust game engine and editor platform. Long-term ambition:
**the best open-source engine in the world** — competitive with Unreal
and Unity on the dimensions that matter most to indie and professional
developers.

We don't try to out-feature Unreal in a decade. We compete for the trust
of developers who have been burned by closed source, royalty-after-revenue
clauses, render-pipeline fragmentation, brittle hot reload, multi-minute
recompiles, phone-home telemetry, and editors that crash and lose work.

Five differentiating pillars (full statement: `docs/adr/0003-differentiation-and-competitive-posture.md`):

1. **Economics and trust** — no royalties, no subscriptions, no required
   online services, MIT forever.
2. **Iteration speed** — sub-second editor cold start, hot reload that
   doesn't lie, shaders compiled at import.
3. **Editor–runtime unification** — one codebase, one scene
   representation, "works in editor / breaks in build" structurally
   precluded.
4. **Determinism and debuggability** — deterministic simulation by
   default, built-in frame inspector with per-pass thumbnails, typed
   errors.
5. **Footprint and source quality** — engine binary under ten megabytes,
   zero external crates, open formats end-to-end.

This branch combines the original Rust engine with a standalone browser-side
Morphic playground.

Main pieces:

- `src/` contains the Rust engine crate
- `examples/demo.rs` is the native example entrypoint
- `examples/editor_shell.rs` is the native Play-in-Editor shell
- `examples/editor_bridge.rs` is the line-based bridge host for shared editor commands/events
- `morphic-web-prototype/` is a no-build browser demo that runs as ES modules

Quick starts:

- Native engine: `cargo check`
- Full Windows validation: `powershell -ExecutionPolicy Bypass -File scripts/windows_validate.ps1 -RunTests`
- Native demo: `cargo run --example demo`
- Native editor shell: `cargo run --example editor_shell`
- Textured OpenGL sample: `cargo run --example textured_scene`
- Timed shell smoke test: `cargo run --example editor_shell -- --backend vulkan --self-test`
- Backend capability snapshot: `cargo run --example editor_shell -- --backend-info`
- Native editor bridge: `cargo run --example editor_bridge`
- Browser backend: `cargo run --example browser_backend`
- Browser backend with explicit asset root: `cargo run --example browser_backend -- --assets-root examples/source`
- Browser prototype: open `morphic-web-prototype/index.html`

Shared editor bridge groundwork:

- Rust command/event protocol: `src/editor/protocol.rs`
- Browser mirror protocol: `morphic-web-prototype/src/EditorProtocol.js`
- Bridge host example: `examples/editor_bridge.rs`
- HTTP browser backend example: `examples/browser_backend.rs`
- Browser backend asset browser/bitmap preview: `GET /api/browser` plus `GET /api/project-file?path=...`

The goal of this branch is to keep one tracked place for both the low-level
engine work and the fast browser-side interaction prototype.

Vision:

- free to use forever
- no royalties
- no subscriptions
- no required online services
- no dependence on paid infrastructure for core workflows

The long-term aim is an indie-friendly engine that remains usable offline,
stays free of financial lock-in, and does not break if hosted services go
away.

Licensing status:

- this repository is distributed under the MIT License
- commercial and non-commercial use are allowed under that license
- the project goal remains no royalties, no subscriptions, and no required online services for core workflows

Current backend snapshot:

- `software`: ready for the native editor shell, including debug lines and screen-space overlays
- `opengl`: experimental editor shell backend with overlay support plus mesh/texture upload helpers
- `vulkan`: experimental hybrid shell backend; swapchain/present are Vulkan, while current editor overlays are CPU-rasterized

Hot-reload (live editor):

- `examples/editor_shell.rs` automatically wires the hot-reload pipeline
  (`FileWatcher` → `CompileQueue` → `ReloadDispatcher`) on startup against the
  scene's source root. Save a `.hsc` and the change converges through the
  entity-preserving diff in `EditorSceneSession::reload_scene_document`,
  preserving runtime gameplay state across the reload.
- The Hot-Reload Status panel (top-right of the editor) renders applied/failed
  events; the action log carries a per-tick narrative summary.
- Replay-mode locks reloads automatically (ADR 0004 Q7); reloads buffer until
  replay ends.
- Pipeline disables silently when the scene was opened without a source root
  on disk (e.g. synthetic test scenes); the panel just stays empty.
- Architecture: `docs/adr/0004-hot-reload-architecture.md` (Q3, Q10, Q14).

Determinism — record / replay (live editor):

- `examples/editor_shell.rs` wires the `src/determinism/` infrastructure to
  F-keys: **F11** toggles input recording, **F12** toggles replay against
  the most recently completed recording.
- During replay, the editor's `play_state` switches to
  `ReplayingDeterministic`, which automatically locks hot-reload via the
  existing dispatcher gate — replay never diverges from the recorded state
  because of a stray reload.
- F-key handlers always read live OS input (process_input carve-out), so F12
  to cancel replay always works regardless of what the recording captured.
- The codec in `src/determinism/recording.rs` is byte-stable so a recording
  saved to `.holdrec` reloads and replays identically across processes,
  machines, and dev sessions (within the same engine binary).
- The load-bearing CI test (`record_and_replay_frame_digests`) records a
  small simulation, replays it twice from fresh state, and asserts per-frame
  state digest streams match — the same shape as the hot-reload equivalence
  harness, scoped to determinism.
- Architecture: `docs/adr/0005-determinism-architecture.md`.

Roadmap pointers:

- Foundation decisions: `docs/adr/0001-engine-editor-foundation.md` (M1–M3: editable scene, play-in-editor, world-scale foundations)
- Material pipeline (Substance Designer + Painter inspired): `docs/adr/0002-material-pipeline.md` (M4–M6: typed frame graph, normal maps + mesh maps, layered materials)
- **Differentiation and competitive posture**: `docs/adr/0003-differentiation-and-competitive-posture.md` (the five pillars; pain-point targeting; M7–M12 sequence)
- **Hot-reload architecture**: `docs/adr/0004-hot-reload-architecture.md` (M7: file watcher + compile queue + reload dispatcher + invalidation bus; "hot-reloaded state ≡ clean-reload state" as a load-bearing promise)
- Project charter: `docs/PROJECT_CHARTER.md`

Binary format specs:

- Compiled-asset (`.hcmp`) envelope + payloads: `docs/formats/compiled-asset.md` (M10: versioned envelope, per-kind payload versions, CRC-32 integrity footer, migration-on-read, field-by-field byte layout pinned by golden tests)
