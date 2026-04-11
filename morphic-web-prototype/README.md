This folder contains the browser-side Morphic playground for the `hold_engine`
repo.

It is intentionally build-free: open `index.html` in a browser, or serve this
folder from any static file server, and the ES module demo will boot without a
Node or TypeScript toolchain.

What is included here:

- Click-to-select morphs with a live inspector
- Right-click context actions to add, duplicate, or delete children
- A trusted local scripting console with `registry`, `tree`, `renderer`, and
  `MorphicObject` helpers
- A shared `.hsc` scene bridge so the browser tree can export to, and import
  from, the Rust scene format used by `src/scene.rs`
- A shared editor command/event protocol mirror in `src/EditorProtocol.js`
  that matches the Rust host protocol in `src/editor/protocol.rs`
- A tiny Rust bridge host in `../examples/editor_bridge.rs` that can execute
  those protocol messages against a live `EditorRuntimeHost`

This branch keeps the Rust engine source untouched while giving the repo a
working browser prototype in the same tracked history.
