This folder contains the browser-side Morphic playground for the `hold_engine`
repo.

It is intentionally build-free: open `index.html` in a browser, or serve this
folder from any static file server, and the ES module demo will boot without a
Node or TypeScript toolchain.

If you want the browser editor to talk to the Rust parser/session and save back
to a real `.hsc` file, run:

- `cargo run --example browser_backend`

Then open the printed local URL instead of opening `index.html` directly.

Browser codec regression check:

- `node --test morphic-web-prototype/src/SceneCodec.test.mjs morphic-web-prototype/src/AssetBrowser.test.mjs`

What is included here:

- Click-to-select morphs with a live inspector
- Right-click context actions to add, duplicate, or delete children
- Authored scene save/download plus undo/redo in the browser workspace
- A trusted local scripting console with `registry`, `tree`, `renderer`, and
  `MorphicObject` helpers
- A shared `.hsc` scene bridge so the browser tree can export to, and import
  from, the Rust scene format used by `src/scene.rs`
- A shared editor command/event protocol mirror in `src/EditorProtocol.js`
  that matches the Rust host protocol in `src/editor/protocol.rs`
- A tiny Rust bridge host in `../examples/editor_bridge.rs` that can execute
  those protocol messages against a live `EditorRuntimeHost`
- A tiny HTTP backend in `../examples/browser_backend.rs` that serves this
  folder, validates `.hsc` source through the Rust bridge, and saves it back to
  disk
- A read-only backend asset browser that can preview project bitmaps such as the
  example `.bmp` textures under `examples/source/`, and backend-transcoded `.tga`
  textures when present

This branch keeps the Rust engine source untouched while giving the repo a
working browser prototype in the same tracked history.
