# Hold Engine

This branch combines the original Rust engine with a standalone browser-side
Morphic playground.

Main pieces:

- `src/` contains the Rust engine crate
- `examples/demo.rs` is the native example entrypoint
- `morphic-web-prototype/` is a no-build browser demo that runs as ES modules

Quick starts:

- Native engine: `cargo check`
- Native demo: `cargo run --example demo`
- Browser prototype: open `morphic-web-prototype/index.html`

The goal of this branch is to keep one tracked place for both the low-level
engine work and the fast browser-side interaction prototype.
