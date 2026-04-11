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

use hold::{
    encode_editor_event_protocol, EditorEvent, EditorInspectorCursor, EditorRuntimeHost,
};

const DEFAULT_SCENE: &str = "examples/demo.hsc";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scene_path = parse_scene_arg();
    let mut host = EditorRuntimeHost::open_hsc(&scene_path)?;

    eprintln!(
        "[editor-bridge] loaded '{}' ({} entities)",
        host.session().scene().name,
        host.session().scene().entities.len()
    );

    emit_startup_snapshot(&host)?;

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        let message = line.trim();
        if message.is_empty() {
            continue;
        }

        match host.execute_protocol_message(message) {
            Ok(events) => {
                for event in events {
                    println!("{event}");
                }
            }
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

fn emit_startup_snapshot(host: &EditorRuntimeHost) -> io::Result<()> {
    println!("{}", encode_editor_event_protocol(&EditorEvent::WorldRebuilt));
    if let Some(cursor) = host.cursor() {
        emit_selection(cursor)?;
    }
    io::stdout().flush()
}

fn emit_selection(cursor: EditorInspectorCursor) -> io::Result<()> {
    println!(
        "{}",
        encode_editor_event_protocol(&EditorEvent::SelectionChanged(cursor))
    );
    Ok(())
}
