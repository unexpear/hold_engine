//! Shared editor bridge protocol for browser/native command and event transport.
//!
//! The protocol is intentionally tiny and dependency-free:
//! - one message per line,
//! - `|`-separated tokens,
//! - backslash escapes for reserved characters.
//!
//! This gives the Rust host and the browser prototype a single serializable
//! command/event surface before any IPC layer exists.

use crate::math::Vec3;

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
        EditorCommand::Undo => "cmd|Undo".into(),
        EditorCommand::Redo => "cmd|Redo".into(),
        EditorCommand::ReloadSource => "cmd|ReloadSource".into(),
        EditorCommand::ForcePatch => "cmd|ForcePatch".into(),
        EditorCommand::ForceRebuild => "cmd|ForceRebuild".into(),
        EditorCommand::RecoverAutosave => "cmd|RecoverAutosave".into(),
        EditorCommand::DiscardAutosave => "cmd|DiscardAutosave".into(),
        EditorCommand::SetTerrainSettings {
            size_meters,
            chunk_size_meters,
            height_scale_meters,
        } => format!(
            "cmd|SetTerrainSettings|{}|{}|{}|{}|{}",
            size_meters.x, size_meters.y, size_meters.z, chunk_size_meters, height_scale_meters
        ),
        EditorCommand::UpsertTerrainPaintLayer {
            name,
            material,
            tiling,
            weightmap_channel,
        } => format!(
            "cmd|UpsertTerrainPaintLayer|{}|{}|{}|{}",
            escape_token(name),
            escape_token(material.as_deref().unwrap_or_default()),
            tiling,
            weightmap_channel
        ),
        EditorCommand::RemoveTerrainPaintLayer { name } => {
            format!("cmd|RemoveTerrainPaintLayer|{}", escape_token(name))
        }
        EditorCommand::PaintVegetation {
            prototype,
            center,
            radius,
            density,
        } => format!(
            "cmd|PaintVegetation|{}|{}|{}|{}|{}|{}",
            escape_token(prototype),
            center.x,
            center.y,
            center.z,
            radius,
            density
        ),
        EditorCommand::EraseVegetation {
            prototype,
            center,
            radius,
        } => format!(
            "cmd|EraseVegetation|{}|{}|{}|{}|{}",
            escape_token(prototype.as_deref().unwrap_or_default()),
            center.x,
            center.y,
            center.z,
            radius
        ),
        EditorCommand::UpsertPatrolRoute {
            name,
            points,
            looped,
        } => {
            let mut tokens = vec![
                "cmd".to_string(),
                "UpsertPatrolRoute".to_string(),
                escape_token(name),
                encode_bool(*looped),
                points.len().to_string(),
            ];
            append_vec3_tokens(&mut tokens, points);
            tokens.join("|")
        }
        EditorCommand::RemovePatrolRoute { name } => {
            format!("cmd|RemovePatrolRoute|{}", escape_token(name))
        }
        EditorCommand::UpsertVehicleSpawner {
            name,
            mesh,
            material,
            position,
            facing_yaw_deg,
        } => format!(
            "cmd|UpsertVehicleSpawner|{}|{}|{}|{}|{}|{}|{}",
            escape_token(name),
            escape_token(mesh.as_deref().unwrap_or_default()),
            escape_token(material.as_deref().unwrap_or_default()),
            position.x,
            position.y,
            position.z,
            facing_yaw_deg
        ),
        EditorCommand::RemoveVehicleSpawner { name } => {
            format!("cmd|RemoveVehicleSpawner|{}", escape_token(name))
        }
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
        "Undo" => expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::Undo),
        "Redo" => expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::Redo),
        "ReloadSource" => expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::ReloadSource),
        "ForcePatch" => expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::ForcePatch),
        "ForceRebuild" => expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::ForceRebuild),
        "RecoverAutosave" => {
            expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::RecoverAutosave)
        }
        "DiscardAutosave" => {
            expect_len(tokens.as_slice(), 2).map(|_| EditorCommand::DiscardAutosave)
        }
        "SetTerrainSettings" => {
            expect_len(tokens.as_slice(), 7)?;
            Ok(EditorCommand::SetTerrainSettings {
                size_meters: parse_vec3(&tokens[2], &tokens[3], &tokens[4])?,
                chunk_size_meters: parse_u32(&tokens[5])?,
                height_scale_meters: parse_f32(&tokens[6])?,
            })
        }
        "UpsertTerrainPaintLayer" => {
            expect_len(tokens.as_slice(), 6)?;
            Ok(EditorCommand::UpsertTerrainPaintLayer {
                name: tokens[2].clone(),
                material: parse_optional_token(&tokens[3]),
                tiling: parse_f32(&tokens[4])?,
                weightmap_channel: parse_u8(&tokens[5])?,
            })
        }
        "RemoveTerrainPaintLayer" => {
            expect_len(tokens.as_slice(), 3)?;
            Ok(EditorCommand::RemoveTerrainPaintLayer {
                name: tokens[2].clone(),
            })
        }
        "PaintVegetation" => {
            expect_len(tokens.as_slice(), 8)?;
            Ok(EditorCommand::PaintVegetation {
                prototype: tokens[2].clone(),
                center: parse_vec3(&tokens[3], &tokens[4], &tokens[5])?,
                radius: parse_f32(&tokens[6])?,
                density: parse_f32(&tokens[7])?,
            })
        }
        "EraseVegetation" => {
            expect_len(tokens.as_slice(), 7)?;
            Ok(EditorCommand::EraseVegetation {
                prototype: parse_optional_token(&tokens[2]),
                center: parse_vec3(&tokens[3], &tokens[4], &tokens[5])?,
                radius: parse_f32(&tokens[6])?,
            })
        }
        "UpsertPatrolRoute" => {
            expect_min_len(tokens.as_slice(), 5)?;
            let point_count = parse_u32(&tokens[4])? as usize;
            let expected_len = 5 + point_count * 3;
            expect_len(tokens.as_slice(), expected_len)?;
            let mut points = Vec::with_capacity(point_count);
            let mut index = 5;
            for _ in 0..point_count {
                points.push(parse_vec3(
                    &tokens[index],
                    &tokens[index + 1],
                    &tokens[index + 2],
                )?);
                index += 3;
            }
            Ok(EditorCommand::UpsertPatrolRoute {
                name: tokens[2].clone(),
                looped: parse_bool(&tokens[3])?,
                points,
            })
        }
        "RemovePatrolRoute" => {
            expect_len(tokens.as_slice(), 3)?;
            Ok(EditorCommand::RemovePatrolRoute {
                name: tokens[2].clone(),
            })
        }
        "UpsertVehicleSpawner" => {
            expect_len(tokens.as_slice(), 9)?;
            Ok(EditorCommand::UpsertVehicleSpawner {
                name: tokens[2].clone(),
                mesh: parse_optional_token(&tokens[3]),
                material: parse_optional_token(&tokens[4]),
                position: parse_vec3(&tokens[5], &tokens[6], &tokens[7])?,
                facing_yaw_deg: parse_f32(&tokens[8])?,
            })
        }
        "RemoveVehicleSpawner" => {
            expect_len(tokens.as_slice(), 3)?;
            Ok(EditorCommand::RemoveVehicleSpawner {
                name: tokens[2].clone(),
            })
        }
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
        EditorEvent::UndoApplied => "evt|UndoApplied".into(),
        EditorEvent::RedoApplied => "evt|RedoApplied".into(),
        EditorEvent::Reloaded => "evt|Reloaded".into(),
        EditorEvent::AssetInvalidated {
            kind,
            handle_index,
            logical_name,
        } => format!(
            "evt|AssetInvalidated|{}|{}|{}",
            escape_token(kind),
            handle_index,
            escape_token(logical_name),
        ),
        EditorEvent::CompileFailed {
            kind,
            logical_name,
            error,
        } => format!(
            "evt|CompileFailed|{}|{}|{}",
            escape_token(kind),
            escape_token(logical_name),
            escape_token(error),
        ),
        EditorEvent::RuntimeSliceUpdated { path } => {
            format!("evt|RuntimeSliceUpdated|{}", escape_token(path))
        }
        EditorEvent::AutosaveRecovered { path } => {
            format!("evt|AutosaveRecovered|{}", escape_token(path))
        }
        EditorEvent::AutosaveDiscarded => "evt|AutosaveDiscarded".into(),
        EditorEvent::FrameCaptured { generation, passes } => {
            // Serialize as generation|count|(index|name|w|h)*. Pixel
            // data intentionally does not travel over the line
            // protocol — bridge clients that want thumbnails fetch
            // through a future dedicated channel (ADR 0002 Q8's frame
            // inspector, M9 follow-up).
            let mut out = format!("evt|FrameCaptured|{}|{}", generation, passes.len());
            for p in passes {
                out.push('|');
                out.push_str(&p.index.to_string());
                out.push('|');
                out.push_str(&escape_token(&p.name));
                out.push('|');
                out.push_str(&p.width.to_string());
                out.push('|');
                out.push_str(&p.height.to_string());
            }
            out
        }
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
        "UndoApplied" => expect_len(tokens.as_slice(), 2).map(|_| EditorEvent::UndoApplied),
        "RedoApplied" => expect_len(tokens.as_slice(), 2).map(|_| EditorEvent::RedoApplied),
        "Reloaded" => expect_len(tokens.as_slice(), 2).map(|_| EditorEvent::Reloaded),
        "RuntimeSliceUpdated" => {
            expect_len(tokens.as_slice(), 3)?;
            Ok(EditorEvent::RuntimeSliceUpdated {
                path: tokens[2].clone(),
            })
        }
        "AutosaveRecovered" => {
            expect_len(tokens.as_slice(), 3)?;
            Ok(EditorEvent::AutosaveRecovered {
                path: tokens[2].clone(),
            })
        }
        "AutosaveDiscarded" => {
            expect_len(tokens.as_slice(), 2).map(|_| EditorEvent::AutosaveDiscarded)
        }
        "AssetInvalidated" => {
            expect_len(tokens.as_slice(), 5)?;
            Ok(EditorEvent::AssetInvalidated {
                kind: tokens[2].clone(),
                handle_index: parse_u32(&tokens[3])?,
                logical_name: tokens[4].clone(),
            })
        }
        "CompileFailed" => {
            expect_len(tokens.as_slice(), 5)?;
            Ok(EditorEvent::CompileFailed {
                kind: tokens[2].clone(),
                logical_name: tokens[3].clone(),
                error: tokens[4].clone(),
            })
        }
        "FrameCaptured" => {
            // evt|FrameCaptured|gen|count|(index|name|w|h)*
            if tokens.len() < 4 {
                return Err(invalid_message(
                    "FrameCaptured requires at least gen and count",
                ));
            }
            let generation = parse_u64(&tokens[2])?;
            let count = parse_usize(&tokens[3])?;
            let expected = 4 + count * 4;
            expect_len(tokens.as_slice(), expected)?;
            let mut passes = Vec::with_capacity(count);
            for i in 0..count {
                let base = 4 + i * 4;
                passes.push(super::FrameInspectorPass {
                    index: parse_u32(&tokens[base])?,
                    name: tokens[base + 1].clone(),
                    width: parse_u32(&tokens[base + 2])?,
                    height: parse_u32(&tokens[base + 3])?,
                });
            }
            Ok(EditorEvent::FrameCaptured { generation, passes })
        }
        _ => Err(invalid_token(kind, "unknown event kind")),
    }
}

fn append_value_tokens(tokens: &mut Vec<String>, value: &InspectorValue) {
    match value {
        InspectorValue::Number(number) => {
            tokens.push("number".into());
            tokens.push(number.to_string());
        }
        InspectorValue::Integer(value) => {
            tokens.push("integer".into());
            tokens.push(value.to_string());
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

fn append_vec3_tokens(tokens: &mut Vec<String>, values: &[Vec3]) {
    for value in values {
        tokens.push(value.x.to_string());
        tokens.push(value.y.to_string());
        tokens.push(value.z.to_string());
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
        "integer" => {
            expect_len(tokens, 2)?;
            Ok(InspectorValue::Integer(parse_i64(&tokens[1])?))
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
        // Deterministic replay surfaces over the editor protocol so
        // remote clients (browser editor, IDE plugin) can show the
        // mode lock visually. See ADR 0004 Q7.
        EditorPlayState::ReplayingDeterministic => "ReplayingDeterministic".into(),
    }
}

fn encode_bool(value: bool) -> String {
    if value {
        "true".into()
    } else {
        "false".into()
    }
}

fn parse_play_state(token: &str) -> Result<EditorPlayState, EditorProtocolError> {
    match token {
        "Editing" => Ok(EditorPlayState::Editing),
        "Playing" => Ok(EditorPlayState::Playing),
        "Paused" => Ok(EditorPlayState::Paused),
        "ReplayingDeterministic" => Ok(EditorPlayState::ReplayingDeterministic),
        _ => Err(invalid_token(
            token,
            "expected Editing, Playing, Paused, or ReplayingDeterministic",
        )),
    }
}

fn parse_u64(token: &str) -> Result<u64, EditorProtocolError> {
    token
        .parse::<u64>()
        .map_err(|_| invalid_token(token, "expected unsigned integer"))
}

fn parse_i64(token: &str) -> Result<i64, EditorProtocolError> {
    token
        .parse::<i64>()
        .map_err(|_| invalid_token(token, "expected integer"))
}

fn parse_u32(token: &str) -> Result<u32, EditorProtocolError> {
    token
        .parse::<u32>()
        .map_err(|_| invalid_token(token, "expected unsigned integer"))
}

fn parse_usize(token: &str) -> Result<usize, EditorProtocolError> {
    token
        .parse::<usize>()
        .map_err(|_| invalid_token(token, "expected non-negative count"))
}

fn parse_u8(token: &str) -> Result<u8, EditorProtocolError> {
    token
        .parse::<u8>()
        .map_err(|_| invalid_token(token, "expected byte value"))
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

fn parse_bool(token: &str) -> Result<bool, EditorProtocolError> {
    match token {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(invalid_token(token, "expected true or false")),
    }
}

fn parse_optional_token(token: &str) -> Option<String> {
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

fn parse_vec3(x: &str, y: &str, z: &str) -> Result<Vec3, EditorProtocolError> {
    Ok(Vec3::new(parse_f32(x)?, parse_f32(y)?, parse_f32(z)?))
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

fn expect_min_len(tokens: &[String], expected: usize) -> Result<(), EditorProtocolError> {
    if tokens.len() >= expected {
        Ok(())
    } else {
        Err(invalid_message(format!(
            "expected at least {expected} tokens, got {}",
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
            EditorCommand::Undo,
            EditorCommand::Redo,
            EditorCommand::ReloadSource,
            EditorCommand::ForceRebuild,
            EditorCommand::RecoverAutosave,
            EditorCommand::DiscardAutosave,
            EditorCommand::SetTerrainSettings {
                size_meters: Vec3::new(1024.0, 320.0, 1024.0),
                chunk_size_meters: 64,
                height_scale_meters: 72.0,
            },
            EditorCommand::UpsertTerrainPaintLayer {
                name: "sand".into(),
                material: Some("materials/terrain/sand".into()),
                tiling: 6.0,
                weightmap_channel: 1,
            },
            EditorCommand::PaintVegetation {
                prototype: "palm_tree".into(),
                center: Vec3::new(32.0, 4.0, 48.0),
                radius: 12.0,
                density: 2.5,
            },
            EditorCommand::UpsertPatrolRoute {
                name: "beach_patrol".into(),
                points: vec![Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0)],
                looped: true,
            },
            EditorCommand::UpsertVehicleSpawner {
                name: "jeep_spawn".into(),
                mesh: Some("meshes/vehicles/jeep".into()),
                material: Some("materials/vehicles/jeep".into()),
                position: Vec3::new(15.0, 1.0, 22.0),
                facing_yaw_deg: 90.0,
            },
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
                (
                    EditorCommand::SetTerrainSettings {
                        size_meters: size_a,
                        chunk_size_meters: chunk_a,
                        height_scale_meters: height_a,
                    },
                    EditorCommand::SetTerrainSettings {
                        size_meters: size_b,
                        chunk_size_meters: chunk_b,
                        height_scale_meters: height_b,
                    },
                ) => {
                    assert_eq!(size_a, size_b);
                    assert_eq!(chunk_a, chunk_b);
                    assert!((height_a - height_b).abs() < 0.0001);
                }
                (
                    EditorCommand::UpsertTerrainPaintLayer {
                        name: name_a,
                        material: material_a,
                        tiling: tiling_a,
                        weightmap_channel: channel_a,
                    },
                    EditorCommand::UpsertTerrainPaintLayer {
                        name: name_b,
                        material: material_b,
                        tiling: tiling_b,
                        weightmap_channel: channel_b,
                    },
                ) => {
                    assert_eq!(name_a, name_b);
                    assert_eq!(material_a, material_b);
                    assert!((tiling_a - tiling_b).abs() < 0.0001);
                    assert_eq!(channel_a, channel_b);
                }
                (
                    EditorCommand::PaintVegetation {
                        prototype: prototype_a,
                        center: center_a,
                        radius: radius_a,
                        density: density_a,
                    },
                    EditorCommand::PaintVegetation {
                        prototype: prototype_b,
                        center: center_b,
                        radius: radius_b,
                        density: density_b,
                    },
                ) => {
                    assert_eq!(prototype_a, prototype_b);
                    assert_eq!(center_a, center_b);
                    assert!((radius_a - radius_b).abs() < 0.0001);
                    assert!((density_a - density_b).abs() < 0.0001);
                }
                (
                    EditorCommand::UpsertPatrolRoute {
                        name: name_a,
                        points: points_a,
                        looped: looped_a,
                    },
                    EditorCommand::UpsertPatrolRoute {
                        name: name_b,
                        points: points_b,
                        looped: looped_b,
                    },
                ) => {
                    assert_eq!(name_a, name_b);
                    assert_eq!(points_a, points_b);
                    assert_eq!(looped_a, looped_b);
                }
                (
                    EditorCommand::UpsertVehicleSpawner {
                        name: name_a,
                        mesh: mesh_a,
                        material: material_a,
                        position: position_a,
                        facing_yaw_deg: yaw_a,
                    },
                    EditorCommand::UpsertVehicleSpawner {
                        name: name_b,
                        mesh: mesh_b,
                        material: material_b,
                        position: position_b,
                        facing_yaw_deg: yaw_b,
                    },
                ) => {
                    assert_eq!(name_a, name_b);
                    assert_eq!(mesh_a, mesh_b);
                    assert_eq!(material_a, material_b);
                    assert_eq!(position_a, position_b);
                    assert!((yaw_a - yaw_b).abs() < 0.0001);
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
            EditorEvent::RuntimeSliceUpdated {
                path: "terrain.paint_layers.sand".into(),
            },
            EditorEvent::UndoApplied,
            EditorEvent::RedoApplied,
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
                (
                    EditorEvent::RuntimeSliceUpdated { path: a },
                    EditorEvent::RuntimeSliceUpdated { path: b },
                ) => assert_eq!(a, b),
                (EditorEvent::UndoApplied, EditorEvent::UndoApplied) => {}
                (EditorEvent::RedoApplied, EditorEvent::RedoApplied) => {}
                _ => panic!("roundtrip mismatch"),
            }
        }
    }

    /// A `FieldEdited` event carrying an integer field value survives the
    /// wire codec exactly, including a value past f32's 2^24 exact-integer
    /// ceiling. The editor streams seed edits to remote clients through this
    /// path, so an f32 hop here would reintroduce the determinism hazard the
    /// `Integer` variant exists to remove.
    #[test]
    fn field_edited_integer_survives_f32_hazard() {
        let hazard: i64 = 0xDEAD_BEEF; // 3_735_928_559
        assert_ne!(
            (hazard as f32) as i64,
            hazard,
            "value must be an f32 hazard or this test proves nothing"
        );

        let event = EditorEvent::FieldEdited {
            cursor: EditorInspectorCursor {
                entity_id: 3,
                component_name: "ParticleEmitter".into(),
                field_name: "seed".into(),
                axis: None,
            },
            new_value: InspectorValue::Integer(hazard),
        };
        let decoded = decode_event(&encode_event(&event)).unwrap();
        match decoded {
            EditorEvent::FieldEdited { new_value, .. } => {
                assert_eq!(new_value, InspectorValue::Integer(hazard));
            }
            other => panic!("expected FieldEdited, got {other:?}"),
        }
    }

    /// A malformed integer literal in a `FieldEdited` value is a typed decode
    /// error, not a silently-swallowed zero or a truncated value — matching the
    /// module's negative-test convention for other decode kinds.
    #[test]
    fn field_edited_with_malformed_integer_is_rejected() {
        let good = encode_event(&EditorEvent::FieldEdited {
            cursor: EditorInspectorCursor {
                entity_id: 3,
                component_name: "ParticleEmitter".into(),
                field_name: "seed".into(),
                axis: None,
            },
            new_value: InspectorValue::Integer(42),
        });
        // Control: the well-formed message decodes.
        assert!(decode_event(&good).is_ok());
        // A fractional literal in an integer field must be REJECTED, not
        // truncated (the silent-truncation bug this variant exists to kill).
        assert!(decode_event(&good.replace("integer|42", "integer|2.5")).is_err());
        // Garbage likewise, rather than a swallowed 0.
        assert!(decode_event(&good.replace("integer|42", "integer|xyz")).is_err());
    }

    #[test]
    fn decode_tolerates_utf8_bom_prefix() {
        let command = decode_command("\u{feff}cmd|SelectNextEntity").unwrap();
        assert!(matches!(command, EditorCommand::SelectNextEntity));

        let event = decode_event("\u{feff}evt|WorldRebuilt").unwrap();
        assert!(matches!(event, EditorEvent::WorldRebuilt));
    }

    /// `AssetInvalidated` round-trips through the codec with an
    /// awkward logical name (containing `|`, backslash, newline)
    /// that exercises the escape path. ADR 0004 Q13 is the
    /// load-bearing reference: remote clients consume this codec
    /// to refresh their previews on hot-reload.
    #[test]
    fn asset_invalidated_event_roundtrips() {
        let original = EditorEvent::AssetInvalidated {
            kind: "Texture".into(),
            handle_index: 42,
            logical_name: "textures/awkward | name\\with\nweirdness".into(),
        };
        let encoded = encode_event(&original);
        let decoded = decode_event(&encoded).unwrap();
        match decoded {
            EditorEvent::AssetInvalidated {
                kind,
                handle_index,
                logical_name,
            } => {
                assert_eq!(kind, "Texture");
                assert_eq!(handle_index, 42);
                assert_eq!(logical_name, "textures/awkward | name\\with\nweirdness");
            }
            other => panic!("expected AssetInvalidated, got {:?}", other),
        }
    }

    /// `CompileFailed` round-trips with an error message that
    /// includes protocol-reserved characters. The message comes
    /// straight from the underlying `AssetCompilerError` Display
    /// impl and can contain anything; the codec's escape layer
    /// must keep it intact.
    #[test]
    fn compile_failed_event_roundtrips() {
        let original = EditorEvent::CompileFailed {
            kind: "Material".into(),
            logical_name: "materials/wall".into(),
            error: "missing field `albedo` | line 12".into(),
        };
        let encoded = encode_event(&original);
        let decoded = decode_event(&encoded).unwrap();
        match decoded {
            EditorEvent::CompileFailed {
                kind,
                logical_name,
                error,
            } => {
                assert_eq!(kind, "Material");
                assert_eq!(logical_name, "materials/wall");
                assert_eq!(error, "missing field `albedo` | line 12");
            }
            other => panic!("expected CompileFailed, got {:?}", other),
        }
    }

    /// The M11 recovery events round-trip over the bridge, including a path
    /// containing protocol-reserved characters. A codec mismatch here would
    /// silently break recovery for a remote editor client.
    #[test]
    fn autosave_recovery_events_roundtrip() {
        let original = EditorEvent::AutosaveRecovered {
            path: "C:\\proj\\.hold/recovery/awkward | name.hsc.autosave".into(),
        };
        match decode_event(&encode_event(&original)).unwrap() {
            EditorEvent::AutosaveRecovered { path } => {
                assert_eq!(path, "C:\\proj\\.hold/recovery/awkward | name.hsc.autosave");
            }
            other => panic!("expected AutosaveRecovered, got {other:?}"),
        }

        match decode_event(&encode_event(&EditorEvent::AutosaveDiscarded)).unwrap() {
            EditorEvent::AutosaveDiscarded => {}
            other => panic!("expected AutosaveDiscarded, got {other:?}"),
        }
    }

    /// Decoder rejects malformed AssetInvalidated messages
    /// loudly: missing tokens, non-numeric handle_index, etc.
    /// Pillar 4 (debuggability): a remote client that ships an
    /// out-of-version protocol message gets a typed error, not
    /// silent acceptance of garbage.
    #[test]
    fn asset_invalidated_decode_rejects_malformed_input() {
        // Missing logical_name token.
        assert!(decode_event("evt|AssetInvalidated|Texture|7").is_err());
        // Non-numeric handle_index.
        assert!(decode_event("evt|AssetInvalidated|Texture|seven|name").is_err());
    }

    /// `FrameCaptured` round-trips a realistic 4-pass capture — the
    /// exact shape the M9 scene graph produces. Preserves pass
    /// ordering, generation counter, and dimensions.
    #[test]
    fn frame_captured_event_roundtrips() {
        let original = EditorEvent::FrameCaptured {
            generation: 42,
            passes: vec![
                super::super::FrameInspectorPass {
                    index: 0,
                    name: "Clear".into(),
                    width: 128,
                    height: 72,
                },
                super::super::FrameInspectorPass {
                    index: 1,
                    name: "Scene".into(),
                    width: 128,
                    height: 72,
                },
                super::super::FrameInspectorPass {
                    index: 2,
                    name: "DebugLines".into(),
                    width: 128,
                    height: 72,
                },
                super::super::FrameInspectorPass {
                    index: 3,
                    name: "ScreenQuads".into(),
                    width: 128,
                    height: 72,
                },
            ],
        };
        let encoded = encode_event(&original);
        let decoded = decode_event(&encoded).unwrap();
        match decoded {
            EditorEvent::FrameCaptured { generation, passes } => {
                assert_eq!(generation, 42);
                assert_eq!(passes.len(), 4);
                assert_eq!(passes[0].name, "Clear");
                assert_eq!(passes[1].name, "Scene");
                assert_eq!(passes[2].name, "DebugLines");
                assert_eq!(passes[3].name, "ScreenQuads");
                assert_eq!(passes[3].index, 3);
                assert_eq!(passes[3].width, 128);
                assert_eq!(passes[3].height, 72);
            }
            other => panic!("expected FrameCaptured, got {:?}", other),
        }
    }

    /// `FrameCaptured` with zero passes is a legitimate wire value —
    /// the graph compiled with no passes (edge case) still emits an
    /// event so bridge clients can update their generation counter.
    #[test]
    fn frame_captured_zero_passes_roundtrips() {
        let original = EditorEvent::FrameCaptured {
            generation: 7,
            passes: Vec::new(),
        };
        let encoded = encode_event(&original);
        let decoded = decode_event(&encoded).unwrap();
        match decoded {
            EditorEvent::FrameCaptured { generation, passes } => {
                assert_eq!(generation, 7);
                assert!(passes.is_empty());
            }
            other => panic!("expected FrameCaptured, got {:?}", other),
        }
    }

    /// Pass names may contain the pipe delimiter (highly unlikely for
    /// pass names but the codec must handle it). Escape layer keeps
    /// the boundary honest.
    #[test]
    fn frame_captured_pass_names_survive_escape_chars() {
        let original = EditorEvent::FrameCaptured {
            generation: 1,
            passes: vec![super::super::FrameInspectorPass {
                index: 0,
                name: "Weird|Name\\With\nCharacters".into(),
                width: 128,
                height: 72,
            }],
        };
        let encoded = encode_event(&original);
        let decoded = decode_event(&encoded).unwrap();
        match decoded {
            EditorEvent::FrameCaptured { passes, .. } => {
                assert_eq!(passes[0].name, "Weird|Name\\With\nCharacters");
            }
            other => panic!("expected FrameCaptured, got {:?}", other),
        }
    }

    /// Decoder rejects a `FrameCaptured` where the count doesn't
    /// match the number of pass records that follow — surfaces a
    /// bug in the encoder rather than silently truncating the tail.
    #[test]
    fn frame_captured_decode_rejects_count_mismatch() {
        // Claims 2 passes but only supplies one.
        assert!(decode_event("evt|FrameCaptured|1|2|0|Clear|128|72").is_err());
        // Claims 0 passes but supplies trailing tokens.
        assert!(decode_event("evt|FrameCaptured|1|0|0|Clear|128|72").is_err());
    }
}
