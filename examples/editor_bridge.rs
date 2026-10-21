//! Minimal line-based bridge host for the shared editor protocol.
//!
//! Usage:
//!   cargo run --example editor_bridge
//!   cargo run --example editor_bridge -- --scene examples/demo.hsc
//!
//! Protocol contract:
//! - stdin: one encoded `EditorCommand` message per line
//! - stdout: zero or more encoded `EditorEvent` messages per command
//! - stderr: human-readable diagnostics only

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use hold::EditorBridgeServer;

const DEFAULT_SCENE: &str = "examples/demo.hsc";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scene_path = parse_scene_arg();
    let mut server = EditorBridgeServer::open_hsc(&scene_path)?;

    eprintln!(
        "[editor-bridge] loaded '{}' ({} entities)",
        server.host().session().scene().name,
        server.host().session().scene().entities.len()
    );

    emit_messages(&server.startup_messages())?;

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        let message = line.trim();
        if message.is_empty() {
            continue;
        }

        match server.execute_message(message) {
            Ok(messages) => emit_messages(&messages)?,
            Err(err) => {
                eprintln!("[editor-bridge] {err}");
            }
        }
    }

    Ok(())
}

fn parse_scene_arg() -> PathBuf {
    let mut args = std::env::args().skip(1).peekable();
    let mut scene = PathBuf::from(DEFAULT_SCENE);

    while let Some(arg) = args.next() {
        if arg == "--scene" {
            if let Some(path) = args.next() {
                scene = PathBuf::from(path);
            }
        }
    }

    scene
}

fn emit_messages(messages: &[String]) -> io::Result<()> {
    for message in messages {
        println!("{message}");
    }
    io::stdout().flush()
}
