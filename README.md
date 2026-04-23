# Hold Engine

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
