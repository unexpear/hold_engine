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
- Native demo: `cargo run --example demo`
- Native editor shell: `cargo run --example editor_shell`
- Native editor bridge: `cargo run --example editor_bridge`
- Browser prototype: open `morphic-web-prototype/index.html`

Shared editor bridge groundwork:

- Rust command/event protocol: `src/editor/protocol.rs`
- Browser mirror protocol: `morphic-web-prototype/src/EditorProtocol.js`
- Bridge host example: `examples/editor_bridge.rs`

The goal of this branch is to keep one tracked place for both the low-level
engine work and the fast browser-side interaction prototype.
