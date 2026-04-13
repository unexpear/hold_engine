//! Shared editor bridge protocol for browser/native command and event transport.
//!
//! The protocol is intentionally tiny and dependency-free:
//! - one message per line,
//! - `|`-separated tokens,
//! - backslash escapes for reserved characters.
//!
//! This gives the Rust host and the browser prototype a single serializable
//! command/event surface before any IPC layer exists.

use super::{
    EditorAxis, EditorCommand, EditorCommandError, EditorEvent, EditorInspectorCursor,
    EditorPlayState, InspectorValue,
};

#[derive(Debug)]
pub enum EditorProtocolError {
    InvalidMessage { message: String },
    InvalidToken { token: String, message: String },
    Command(EditorCommandError),
}

impl std::fmt::Display for EditorProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMessage { message } => write!(f, "invalid protocol message: {message}"),
            Self::InvalidToken { token, message } => {
                write!(f, "invalid protocol token '{token}': {message}")
            }
            Self::Command(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for EditorProtocolError {}

impl From<EditorCommandError> for EditorProtocolError {
    fn from(value: EditorCommandError) -> Self {
        Self::Command(value)
    }
}

pub fn encode_command(command: &EditorCommand) -> String {
    match command {
        EditorCommand::SelectNextEntity => "cmd|SelectNextEntity".into(),
        EditorCommand::SelectPrevEntity => "cmd|SelectPrevEntity".into(),
        EditorCommand::SelectEntity { entity_id } => format!("cmd|SelectEntity|{entity_id}"),
        EditorCommand::SelectNextComponent => "cmd|SelectNextComponent".into(),
        EditorCommand::SelectPrevComponent => "cmd|SelectPrevComponent".into(),
        EditorCommand::SelectComponent {
            entity_id,
            component_name,
        } => format!(
            "cmd|SelectComponent|{}|{}",
            entity_id,
            escape_token(component_name)
        ),
        EditorCommand::SelectNextField => "cmd|SelectNextField".into(),
        EditorCommand::SelectPrevField => "cmd|SelectPrevField".into(),
        EditorCommand::SelectField {
            entity_id,
            component_name,
            field_name,
        } => format!(
            "cmd|SelectField|{}|{}|{}",
            entity_id,
            escape_token(component_name),
            escape_token(field_name)
        ),
        EditorCommand::SetAxis(axis) => format!("cmd|SetAxis|{}", encode_axis(*axis)),
        EditorCommand::ApplyDelta(delta) => format!("cmd|ApplyDelta|{delta}"),
        EditorCommand::EnterPlayMode => "cmd|EnterPlayMode".into(),
        EditorCommand::PausePlayMode => "cmd|PausePlayMode".into(),
        EditorCommand::ResumePlayMode => "cmd|ResumePlayMode".into(),
        EditorCommand::StopPlayMode => "cmd|StopPlayMode".into(),
        EditorCommand::Save => "cmd|Save".into(),
        EditorCommand::ReloadSource => "cmd|ReloadSource".into(),
        EditorCommand::ForcePatch => "cmd|ForcePatch".into(),
        EditorCommand::ForceRebuild => "cmd|ForceRebuild".into(),
    }
}

pub fn decode_command(message: &str) -> Result<EditorCommand, EditorProtocolError> {
    let tokens = split_tokens(message);
    let head = tokens
        .first()
        .ok_or_else(|| invalid_message("empty message"))?;
    if head != "cmd" {
        return Err(invalid_message("message must start with 'cmd'"));
    }
    let kind = tokens
        .get(1)
        .ok_or_else(|| invalid_message("missing command kind"))?;

    match kind.as_str() {
        "SelectNextEntity" => {
            expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::SelectNextEntity)
        }
        "SelectPrevEntity" => {
            expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::SelectPrevEntity)
        }
        "SelectEntity" => {
            expect_len(tokens.as_slice(), 3)?;
            Ok(EditorCommand::SelectEntity {
                entity_id: parse_u64(&tokens[2])?,
            })
        }
        "SelectNextComponent" => {
            expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::SelectNextComponent)
        }
        "SelectPrevComponent" => {
            expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::SelectPrevComponent)
        }
        "SelectComponent" => {
            expect_len(tokens.as_slice(), 4)?;
            Ok(EditorCommand::SelectComponent {
                entity_id: parse_u64(&tokens[2])?,
                component_name: tokens[3].clone(),
            })
        }
        "SelectNextField" => {
            expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::SelectNextField)
        }
        "SelectPrevField" => {
            expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::SelectPrevField)
        }
        "SelectField" => {
            expect_len(tokens.as_slice(), 5)?;
            Ok(EditorCommand::SelectField {
                entity_id: parse_u64(&tokens[2])?,
                component_name: tokens[3].clone(),
                field_name: tokens[4].clone(),
            })
        }
        "SetAxis" => {
            expect_len(tokens.as_slice(), 3)?;
            Ok(EditorCommand::SetAxis(parse_axis(&tokens[2])?))
        }
        "ApplyDelta" => {
            expect_len(tokens.as_slice(), 3)?;
            Ok(EditorCommand::ApplyDelta(parse_f32(&tokens[2])?))
        }
        "EnterPlayMode" => expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::EnterPlayMode),
        "PausePlayMode" => expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::PausePlayMode),
        "ResumePlayMode" => expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::ResumePlayMode),
        "StopPlayMode" => expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::StopPlayMode),
        "Save" => expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::Save),
        "ReloadSource" => expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::ReloadSource),
        "ForcePatch" => expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::ForcePatch),
        "ForceRebuild" => expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::ForceRebuild),
        _ => Err(invalid_token(kind, "unknown command kind")),
    }
}

pub fn encode_event(event: &EditorEvent) -> String {
    match event {
        EditorEvent::SelectionChanged(cursor) => format!(
            "evt|SelectionChanged|{}|{}|{}|{}",
            cursor.entity_id,
            escape_token(&cursor.component_name),
            escape_token(&cursor.field_name),
            cursor.axis.map(encode_axis).unwrap_or_default()
        ),
        EditorEvent::FieldEdited { cursor, new_value } => {
            let mut tokens = vec![
                "evt".to_string(),
                "FieldEdited".to_string(),
                cursor.entity_id.to_string(),
                escape_token(&cursor.component_name),
                escape_token(&cursor.field_name),
                cursor.axis.map(encode_axis).unwrap_or_default(),
            ];
            append_value_tokens(&mut tokens, new_value);
            tokens.join("|")
        }
        EditorEvent::PlayModeChanged(state) => {
            format!("evt|PlayModeChanged|{}", encode_play_state(*state))
        }
        EditorEvent::PatchApplied => "evt|PatchApplied".into(),
        EditorEvent::PatchRequiresRebuild { reason } => {
            format!("evt|PatchRequiresRebuild|{}", escape_token(reason))
        }
        EditorEvent::WorldRebuilt => "evt|WorldRebuilt".into(),
        EditorEvent::Saved => "evt|Saved".into(),
        EditorEvent::Reloaded => "evt|Reloaded".into(),
    }
}

pub fn decode_event(message: &str) -> Result<EditorEvent, EditorProtocolError> {
    let tokens = split_tokens(message);
    let head = tokens
        .first()
        .ok_or_else(|| invalid_message("empty message"))?;
    if head != "evt" {
        return Err(invalid_message("message must start with 'evt'"));
    }
    let kind = tokens
        .get(1)
        .ok_or_else(|| invalid_message("missing event kind"))?;

    match kind.as_str() {
        "SelectionChanged" => {
            expect_len(tokens.as_slice(), 6)?;
            Ok(EditorEvent::SelectionChanged(EditorInspectorCursor {
                entity_id: parse_u64(&tokens[2])?,
                component_name: tokens[3].clone(),
                field_name: tokens[4].clone(),
                axis: parse_optional_axis(&tokens[5])?,
            }))
        }
        "FieldEdited" => {
            if tokens.len() < 8 {
                return Err(invalid_message("FieldEdited requires at least 8 tokens"));
            }
            Ok(EditorEvent::FieldEdited {
                cursor: EditorInspectorCursor {
                    entity_id: parse_u64(&tokens[2])?,
                    component_name: tokens[3].clone(),
                    field_name: tokens[4].clone(),
                    axis: parse_optional_axis(&tokens[5])?,
                },
                new_value: parse_value_tokens(&tokens[6..])?,
            })
        }
        "PlayModeChanged" => {
            expect_len(tokens.as_slice(), 3)?;
            Ok(EditorEvent::PlayModeChanged(parse_play_state(&tokens[2])?))
        }
        "PatchApplied" => expect_len(tokens.as_slice(), 2).map(|_| EditorEvent::PatchApplied),
        "PatchRequiresRebuild" => {
            expect_len(tokens.as_slice(), 3)?;
            Ok(EditorEvent::PatchRequiresRebuild {
                reason: tokens[2].clone(),
            })
        }
        "WorldRebuilt" => expect_len(tokens.as_slice(), 2).map(|_| EditorEvent::WorldRebuilt),
        "Saved" => expect_len(tokens.as_slice(), 2).map(|_| EditorEvent::Saved),
        "Reloaded" => expect_len(tokens.as_slice(), 2).map(|_| EditorEvent::Reloaded),
        _ => Err(invalid_token(kind, "unknown event kind")),
    }
}

fn append_value_tokens(tokens: &mut Vec<String>, value: &InspectorValue) {
    match value {
        InspectorValue::Number(number) => {
            tokens.push("number".into());
            tokens.push(number.to_string());
        }
        InspectorValue::Text(text) => {
            tokens.push("text".into());
            tokens.push(escape_token(text));
        }
        InspectorValue::Vec3(value) => {
            tokens.push("vec3".into());
            tokens.push(value[0].to_string());
            tokens.push(value[1].to_string());
            tokens.push(value[2].to_string());
        }
        InspectorValue::Bool(value) => {
            tokens.push("bool".into());
            tokens.push(value.to_string());
        }
    }
}

fn parse_value_tokens(tokens: &[String]) -> Result<InspectorValue, EditorProtocolError> {
    let kind = tokens
        .first()
        .ok_or_else(|| invalid_message("missing value kind"))?;
    match kind.as_str() {
        "number" => {
            expect_len(tokens, 2)?;
            Ok(InspectorValue::Number(parse_f32(&tokens[1])?))
        }
        "text" => {
            expect_len(tokens, 2)?;
            Ok(InspectorValue::Text(tokens[1].clone()))
        }
        "vec3" => {
            expect_len(tokens, 4)?;
            Ok(InspectorValue::Vec3([
                parse_f64(&tokens[1])? as f32,
                parse_f64(&tokens[2])? as f32,
                parse_f64(&tokens[3])? as f32,
            ]))
        }
        "bool" => {
            expect_len(tokens, 2)?;
            match tokens[1].as_str() {
                "true" => Ok(InspectorValue::Bool(true)),
                "false" => Ok(InspectorValue::Bool(false)),
                _ => Err(invalid_token(&tokens[1], "expected true or false")),
            }
        }
        _ => Err(invalid_token(kind, "unknown value kind")),
    }
}

fn split_tokens(message: &str) -> Vec<String> {
    message
        .trim_start_matches('\u{feff}')
        .split('|')
        .map(unescape_token)
        .collect()
}

fn escape_token(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '|' => out.push_str("\\p"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out
}

fn unescape_token(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('p') => out.push('|'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn encode_axis(axis: EditorAxis) -> String {
    match axis {
        EditorAxis::X => "X".into(),
        EditorAxis::Y => "Y".into(),
        EditorAxis::Z => "Z".into(),
    }
}

fn parse_axis(token: &str) -> Result<EditorAxis, EditorProtocolError> {
    match token {
        "X" => Ok(EditorAxis::X),
        "Y" => Ok(EditorAxis::Y),
        "Z" => Ok(EditorAxis::Z),
        _ => Err(invalid_token(token, "expected X, Y, or Z")),
    }
}

fn parse_optional_axis(token: &str) -> Result<Option<EditorAxis>, EditorProtocolError> {
    if token.is_empty() {
        Ok(None)
    } else {
        Ok(Some(parse_axis(token)?))
    }
}

fn encode_play_state(state: EditorPlayState) -> String {
    match state {
        EditorPlayState::Editing => "Editing".into(),
        EditorPlayState::Playing => "Playing".into(),
        EditorPlayState::Paused => "Paused".into(),
    }
}

fn parse_play_state(token: &str) -> Result<EditorPlayState, EditorProtocolError> {
    match token {
        "Editing" => Ok(EditorPlayState::Editing),
        "Playing" => Ok(EditorPlayState::Playing),
        "Paused" => Ok(EditorPlayState::Paused),
        _ => Err(invalid_token(token, "expected Editing, Playing, or Paused")),
    }
}

fn parse_u64(token: &str) -> Result<u64, EditorProtocolError> {
    token
        .parse::<u64>()
        .map_err(|_| invalid_token(token, "expected unsigned integer"))
}

fn parse_f32(token: &str) -> Result<f32, EditorProtocolError> {
    token
        .parse::<f32>()
        .map_err(|_| invalid_token(token, "expected number"))
}

fn parse_f64(token: &str) -> Result<f64, EditorProtocolError> {
    token
        .parse::<f64>()
        .map_err(|_| invalid_token(token, "expected number"))
}

fn invalid_message(message: impl Into<String>) -> EditorProtocolError {
    EditorProtocolError::InvalidMessage {
        message: message.into(),
    }
}

fn invalid_token(token: impl Into<String>, message: impl Into<String>) -> EditorProtocolError {
    EditorProtocolError::InvalidToken {
        token: token.into(),
        message: message.into(),
    }
}

fn expect_len(tokens: &[String], expected: usize) -> Result<(), EditorProtocolError> {
    if tokens.len() == expected {
        Ok(())
    } else {
        Err(invalid_message(format!(
            "expected {expected} tokens, got {}",
            tokens.len()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_roundtrip_handles_paramless_and_param_commands() {
        let commands = vec![
            EditorCommand::SelectNextEntity,
            EditorCommand::SelectEntity { entity_id: 7 },
            EditorCommand::SelectComponent {
                entity_id: 9,
                component_name: "Directional|Light".into(),
            },
            EditorCommand::SelectField {
                entity_id: 11,
                component_name: "Transform".into(),
                field_name: "position".into(),
            },
            EditorCommand::SetAxis(EditorAxis::Z),
            EditorCommand::ApplyDelta(-1.25),
            EditorCommand::ReloadSource,
            EditorCommand::ForceRebuild,
        ];

        for command in commands {
            let encoded = encode_command(&command);
            let decoded = decode_command(&encoded).unwrap();
            assert_eq!(
                std::mem::discriminant(&decoded),
                std::mem::discriminant(&command)
            );
            match (command, decoded) {
                (
                    EditorCommand::SelectEntity { entity_id: a },
                    EditorCommand::SelectEntity { entity_id: b },
                ) => assert_eq!(a, b),
                (
                    EditorCommand::SelectComponent {
                        entity_id: entity_a,
                        component_name: component_a,
                    },
                    EditorCommand::SelectComponent {
                        entity_id: entity_b,
                        component_name: component_b,
                    },
                ) => {
                    assert_eq!(entity_a, entity_b);
                    assert_eq!(component_a, component_b);
                }
                (
                    EditorCommand::SelectField {
                        entity_id: entity_a,
                        component_name: component_a,
                        field_name: field_a,
                    },
                    EditorCommand::SelectField {
                        entity_id: entity_b,
                        component_name: component_b,
                        field_name: field_b,
                    },
                ) => {
                    assert_eq!(entity_a, entity_b);
                    assert_eq!(component_a, component_b);
                    assert_eq!(field_a, field_b);
                }
                (EditorCommand::SetAxis(a), EditorCommand::SetAxis(b)) => assert_eq!(a, b),
                (EditorCommand::ApplyDelta(a), EditorCommand::ApplyDelta(b)) => {
                    assert!((a - b).abs() < 0.0001)
                }
                _ => {}
            }
        }
    }

    #[test]
    fn event_roundtrip_preserves_tokens_and_escapes() {
        let events = vec![
            EditorEvent::SelectionChanged(EditorInspectorCursor {
                entity_id: 7,
                component_name: "Transform".into(),
                field_name: "position".into(),
                axis: Some(EditorAxis::Y),
            }),
            EditorEvent::FieldEdited {
                cursor: EditorInspectorCursor {
                    entity_id: 9,
                    component_name: "Label|Block".into(),
                    field_name: "text".into(),
                    axis: None,
                },
                new_value: InspectorValue::Text("line 1\nline 2 | ok".into()),
            },
            EditorEvent::PatchRequiresRebuild {
                reason: "parent | missing".into(),
            },
        ];

        for event in events {
            let encoded = encode_event(&event);
            let decoded = decode_event(&encoded).unwrap();
            match (event, decoded) {
                (EditorEvent::SelectionChanged(a), EditorEvent::SelectionChanged(b)) => {
                    assert_eq!(a, b)
                }
                (
                    EditorEvent::FieldEdited {
                        cursor: cursor_a,
                        new_value: value_a,
                    },
                    EditorEvent::FieldEdited {
                        cursor: cursor_b,
                        new_value: value_b,
                    },
                ) => {
                    assert_eq!(cursor_a, cursor_b);
                    assert_eq!(value_a, value_b);
                }
                (
                    EditorEvent::PatchRequiresRebuild { reason: a },
                    EditorEvent::PatchRequiresRebuild { reason: b },
                ) => assert_eq!(a, b),
                _ => panic!("roundtrip mismatch"),
            }
        }
    }

    #[test]
    fn decode_tolerates_utf8_bom_prefix() {
        let command = decode_command("\u{feff}cmd|SelectNextEntity").unwrap();
        assert!(matches!(command, EditorCommand::SelectNextEntity));

        let event = decode_event("\u{feff}evt|WorldRebuilt").unwrap();
        assert!(matches!(event, EditorEvent::WorldRebuilt));
    }
}
