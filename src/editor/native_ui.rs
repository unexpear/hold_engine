//! Native hierarchy/inspector overlay for the windowed editor shell.
//!
//! This module builds a backend-neutral screen-space overlay from the
//! authoritative [`EditorRuntimeHost`]. The shell can render the resulting
//! quads in either software or OpenGL and dispatch mouse clicks back through
//! the existing command surface.

use crate::renderer::{Color, ScreenQuad};

use super::{EditorAxis, EditorCommand, EditorPlayState, EditorRuntimeHost, InspectorValue};

const TOOLBAR_H: i32 = 34;
const STATUS_H: i32 = 24;
const PANEL_PAD: i32 = 10;
const PANEL_BORDER: i32 = 1;
const HEADER_H: i32 = 22;
const ROW_H: i32 = 18;
const STATUS_SCALE: i32 = 1;

const UI_BG: Color = Color::new(0.08, 0.09, 0.10, 1.0);
const PANEL_BG: Color = Color::new(0.12, 0.13, 0.16, 1.0);
const PANEL_BORDER_COLOR: Color = Color::new(0.28, 0.31, 0.36, 1.0);
const HEADER_BG: Color = Color::new(0.16, 0.20, 0.24, 1.0);
const ROW_BG: Color = Color::new(0.15, 0.16, 0.19, 1.0);
const ROW_ALT_BG: Color = Color::new(0.13, 0.14, 0.17, 1.0);
const SELECTED_BG: Color = Color::new(0.18, 0.38, 0.24, 1.0);
const SELECTED_COMPONENT_BG: Color = Color::new(0.22, 0.28, 0.36, 1.0);
const BUTTON_BG: Color = Color::new(0.20, 0.22, 0.27, 1.0);
const BUTTON_ACTIVE_BG: Color = Color::new(0.20, 0.35, 0.46, 1.0);
const BUTTON_DISABLED_BG: Color = Color::new(0.16, 0.17, 0.19, 1.0);
const BUTTON_DANGER_BG: Color = Color::new(0.45, 0.21, 0.18, 1.0);
const TEXT_PRIMARY: Color = Color::new(0.92, 0.95, 0.98, 1.0);
const TEXT_MUTED: Color = Color::new(0.65, 0.69, 0.75, 1.0);
const TEXT_ACCENT: Color = Color::new(0.51, 0.90, 0.58, 1.0);
const TEXT_WARNING: Color = Color::new(0.98, 0.78, 0.28, 1.0);

#[derive(Debug, Clone, Default)]
pub struct EditorNativeUi {
    pub quads: Vec<ScreenQuad>,
    pub hit_regions: Vec<EditorUiHitRegion>,
}

impl EditorNativeUi {
    pub fn commands_at(&self, x: i32, y: i32) -> Option<Vec<EditorCommand>> {
        self.hit_regions
            .iter()
            .rev()
            .find(|region| region.contains(x, y))
            .map(|region| region.commands.clone())
    }
}

#[derive(Debug, Clone)]
pub struct EditorUiHitRegion {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub commands: Vec<EditorCommand>,
}

impl EditorUiHitRegion {
    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

pub fn build_native_ui(host: &EditorRuntimeHost, width: u32, height: u32) -> EditorNativeUi {
    let mut ui = EditorNativeUi::default();
    let width = width as i32;
    let height = height as i32;

    let hierarchy_w = (width / 4).clamp(250, 340);
    let inspector_w = (width / 3).clamp(300, 420);
    let right_x = (width - inspector_w).max(hierarchy_w + 24);
    let panel_top = TOOLBAR_H;
    let panel_h = (height - TOOLBAR_H - STATUS_H).max(120);

    fill_rect(&mut ui, 0, 0, width, height, UI_BG);
    draw_toolbar(&mut ui, host, width);
    draw_panel(&mut ui, 0, panel_top, hierarchy_w, panel_h);
    draw_panel(&mut ui, right_x, panel_top, inspector_w, panel_h);
    draw_status(&mut ui, host, width, height);
    draw_hierarchy(&mut ui, host, 0, panel_top, hierarchy_w, panel_h);
    draw_inspector(&mut ui, host, right_x, panel_top, inspector_w, panel_h);

    ui
}

fn draw_toolbar(ui: &mut EditorNativeUi, host: &EditorRuntimeHost, width: i32) {
    draw_panel(ui, 0, 0, width, TOOLBAR_H);

    let play_state = host.play_state();
    let mut x = PANEL_PAD;
    let y = 6;

    x = push_button(
        ui,
        x,
        y,
        "SAVE",
        host.is_dirty(),
        play_state == EditorPlayState::Editing,
        vec![EditorCommand::Save],
        false,
    );
    x = push_button(
        ui,
        x,
        y,
        "RELOAD",
        false,
        play_state == EditorPlayState::Editing,
        vec![EditorCommand::ReloadSource],
        false,
    );
    x = push_button(
        ui,
        x,
        y,
        "PATCH",
        false,
        true,
        vec![EditorCommand::ForcePatch],
        false,
    );
    x = push_button(
        ui,
        x,
        y,
        "REBUILD",
        host.world_requires_rebuild(),
        true,
        vec![EditorCommand::ForceRebuild],
        false,
    );

    match play_state {
        EditorPlayState::Editing => {
            x = push_button(
                ui,
                x,
                y,
                "PLAY",
                false,
                true,
                vec![EditorCommand::EnterPlayMode],
                false,
            );
        }
        EditorPlayState::Playing => {
            x = push_button(
                ui,
                x,
                y,
                "PAUSE",
                true,
                true,
                vec![EditorCommand::PausePlayMode],
                false,
            );
            x = push_button(
                ui,
                x,
                y,
                "STOP",
                true,
                true,
                vec![EditorCommand::StopPlayMode],
                true,
            );
        }
        EditorPlayState::Paused => {
            x = push_button(
                ui,
                x,
                y,
                "RESUME",
                true,
                true,
                vec![EditorCommand::ResumePlayMode],
                false,
            );
            x = push_button(
                ui,
                x,
                y,
                "STOP",
                true,
                true,
                vec![EditorCommand::StopPlayMode],
                true,
            );
        }
    }

    let _ = x;
}

fn draw_hierarchy(
    ui: &mut EditorNativeUi,
    host: &EditorRuntimeHost,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) {
    let inner_x = x + PANEL_PAD;
    let inner_w = w - PANEL_PAD * 2;
    let header_y = y + PANEL_PAD;

    fill_rect(ui, inner_x, header_y, inner_w, HEADER_H, HEADER_BG);
    draw_text(ui, inner_x + 8, header_y + 5, 1, "HIERARCHY", TEXT_PRIMARY);

    let selected_entity_id = host.cursor().map(|cursor| cursor.entity_id);
    let mut row_y = header_y + HEADER_H + 6;
    let max_y = y + h - PANEL_PAD - ROW_H;

    for (index, entity) in host.session().scene().entities.iter().enumerate() {
        if row_y > max_y {
            break;
        }

        let row_color = if Some(entity.id) == selected_entity_id {
            SELECTED_BG
        } else if index % 2 == 0 {
            ROW_BG
        } else {
            ROW_ALT_BG
        };
        fill_rect(ui, inner_x, row_y, inner_w, ROW_H, row_color);

        let label = format!("E{} {}", entity.id, entity.name);
        draw_text(ui, inner_x + 6, row_y + 4, 1, &label, TEXT_PRIMARY);

        ui.hit_regions.push(EditorUiHitRegion {
            x: inner_x,
            y: row_y,
            w: inner_w,
            h: ROW_H,
            commands: vec![EditorCommand::SelectEntity {
                entity_id: entity.id,
            }],
        });

        row_y += ROW_H + 2;
    }
}

fn draw_inspector(
    ui: &mut EditorNativeUi,
    host: &EditorRuntimeHost,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) {
    let inner_x = x + PANEL_PAD;
    let inner_w = w - PANEL_PAD * 2;
    let header_y = y + PANEL_PAD;

    fill_rect(ui, inner_x, header_y, inner_w, HEADER_H, HEADER_BG);
    draw_text(ui, inner_x + 8, header_y + 5, 1, "INSPECTOR", TEXT_PRIMARY);

    let Some(cursor) = host.cursor() else {
        draw_text(
            ui,
            inner_x + 8,
            header_y + HEADER_H + 12,
            1,
            "NO SELECTION",
            TEXT_MUTED,
        );
        return;
    };

    let Some(entity) = host
        .session()
        .scene()
        .entities
        .iter()
        .find(|entity| entity.id == cursor.entity_id)
    else {
        draw_text(
            ui,
            inner_x + 8,
            header_y + HEADER_H + 12,
            1,
            "SELECTION STALE",
            TEXT_WARNING,
        );
        return;
    };

    let mut row_y = header_y + HEADER_H + 6;
    let max_y = y + h - PANEL_PAD - ROW_H;
    let title = format!("ENTITY {} {}", entity.id, entity.name);
    draw_text(ui, inner_x + 4, row_y, 1, &title, TEXT_ACCENT);
    row_y += ROW_H;

    for component in &entity.components {
        if row_y > max_y {
            break;
        }

        let component_selected = component.type_name == cursor.component_name;
        fill_rect(
            ui,
            inner_x,
            row_y,
            inner_w,
            ROW_H,
            if component_selected {
                SELECTED_COMPONENT_BG
            } else {
                ROW_BG
            },
        );
        draw_text(
            ui,
            inner_x + 6,
            row_y + 4,
            1,
            &component.type_name,
            TEXT_PRIMARY,
        );
        ui.hit_regions.push(EditorUiHitRegion {
            x: inner_x,
            y: row_y,
            w: inner_w,
            h: ROW_H,
            commands: vec![EditorCommand::SelectComponent {
                entity_id: entity.id,
                component_name: component.type_name.clone(),
            }],
        });
        row_y += ROW_H + 2;

        for field in &component.fields {
            if row_y > max_y {
                break;
            }

            let field_selected =
                component.type_name == cursor.component_name && field.name == cursor.field_name;
            let row_color = if field_selected {
                SELECTED_BG
            } else {
                ROW_ALT_BG
            };
            fill_rect(ui, inner_x + 12, row_y, inner_w - 12, ROW_H, row_color);

            let text = format_field_label(&field.name, &field.value);
            draw_text(ui, inner_x + 18, row_y + 4, 1, &text, TEXT_PRIMARY);

            let select_field = EditorCommand::SelectField {
                entity_id: entity.id,
                component_name: component.type_name.clone(),
                field_name: field.name.clone(),
            };
            ui.hit_regions.push(EditorUiHitRegion {
                x: inner_x + 12,
                y: row_y,
                w: inner_w - 12,
                h: ROW_H,
                commands: vec![select_field.clone()],
            });

            if host.play_state() != EditorPlayState::Playing {
                match &field.value {
                    InspectorValue::Number(_) => {
                        let minus_x = inner_x + inner_w - 50;
                        let plus_x = inner_x + inner_w - 26;
                        push_small_button(
                            ui,
                            minus_x,
                            row_y + 1,
                            "-",
                            vec![select_field.clone(), EditorCommand::ApplyDelta(-0.1)],
                            false,
                        );
                        push_small_button(
                            ui,
                            plus_x,
                            row_y + 1,
                            "+",
                            vec![select_field.clone(), EditorCommand::ApplyDelta(0.1)],
                            false,
                        );
                    }
                    InspectorValue::Vec3(_) => {
                        let active_axis = cursor.axis.unwrap_or(EditorAxis::X);
                        let axis_base_x = inner_x + inner_w - 94;
                        for (offset, axis) in
                            [(0, EditorAxis::X), (18, EditorAxis::Y), (36, EditorAxis::Z)]
                        {
                            push_small_button(
                                ui,
                                axis_base_x + offset,
                                row_y + 1,
                                match axis {
                                    EditorAxis::X => "X",
                                    EditorAxis::Y => "Y",
                                    EditorAxis::Z => "Z",
                                },
                                vec![select_field.clone(), EditorCommand::SetAxis(axis)],
                                field_selected && active_axis == axis,
                            );
                        }
                        push_small_button(
                            ui,
                            inner_x + inner_w - 50,
                            row_y + 1,
                            "-",
                            vec![
                                select_field.clone(),
                                EditorCommand::SetAxis(active_axis),
                                EditorCommand::ApplyDelta(-0.1),
                            ],
                            false,
                        );
                        push_small_button(
                            ui,
                            inner_x + inner_w - 26,
                            row_y + 1,
                            "+",
                            vec![
                                select_field.clone(),
                                EditorCommand::SetAxis(active_axis),
                                EditorCommand::ApplyDelta(0.1),
                            ],
                            false,
                        );
                    }
                    InspectorValue::Text(_) | InspectorValue::Bool(_) => {}
                }
            }

            row_y += ROW_H + 1;
        }
    }
}

fn draw_status(ui: &mut EditorNativeUi, host: &EditorRuntimeHost, width: i32, height: i32) {
    let y = height - STATUS_H;
    draw_panel(ui, 0, y, width, STATUS_H);

    let mode = match host.play_state() {
        EditorPlayState::Editing => "EDIT",
        EditorPlayState::Playing => "PLAY",
        EditorPlayState::Paused => "PAUSED",
    };
    let dirty = if host.is_dirty() { "DIRTY" } else { "CLEAN" };
    let sync = if host.world_requires_rebuild() {
        "REBUILD"
    } else {
        "SYNC"
    };
    let cursor = host
        .cursor()
        .map(|cursor| {
            format!(
                "{} {} {}",
                cursor.entity_id, cursor.component_name, cursor.field_name
            )
        })
        .unwrap_or_else(|| "NO SELECTION".to_string());
    let status = format!("MODE {}  {}  {}  {}", mode, dirty, sync, cursor);
    draw_text(ui, PANEL_PAD, y + 8, STATUS_SCALE, &status, TEXT_MUTED);
}

fn draw_panel(ui: &mut EditorNativeUi, x: i32, y: i32, w: i32, h: i32) {
    fill_rect(ui, x, y, w, h, PANEL_BG);
    fill_rect(ui, x, y, w, PANEL_BORDER, PANEL_BORDER_COLOR);
    fill_rect(
        ui,
        x,
        y + h - PANEL_BORDER,
        w,
        PANEL_BORDER,
        PANEL_BORDER_COLOR,
    );
    fill_rect(ui, x, y, PANEL_BORDER, h, PANEL_BORDER_COLOR);
    fill_rect(
        ui,
        x + w - PANEL_BORDER,
        y,
        PANEL_BORDER,
        h,
        PANEL_BORDER_COLOR,
    );
}

fn push_button(
    ui: &mut EditorNativeUi,
    x: i32,
    y: i32,
    label: &str,
    active: bool,
    enabled: bool,
    commands: Vec<EditorCommand>,
    danger: bool,
) -> i32 {
    let w = (text_pixel_width(label, 1) + 16).max(52);
    let color = if !enabled {
        BUTTON_DISABLED_BG
    } else if danger {
        BUTTON_DANGER_BG
    } else if active {
        BUTTON_ACTIVE_BG
    } else {
        BUTTON_BG
    };
    fill_rect(ui, x, y, w, 22, color);
    outline_rect(ui, x, y, w, 22, PANEL_BORDER_COLOR);
    draw_text(ui, x + 8, y + 7, 1, label, TEXT_PRIMARY);
    if enabled {
        ui.hit_regions.push(EditorUiHitRegion {
            x,
            y,
            w,
            h: 22,
            commands,
        });
    }
    x + w + 6
}

fn push_small_button(
    ui: &mut EditorNativeUi,
    x: i32,
    y: i32,
    label: &str,
    commands: Vec<EditorCommand>,
    active: bool,
) {
    let color = if active { BUTTON_ACTIVE_BG } else { BUTTON_BG };
    fill_rect(ui, x, y, 16, 16, color);
    outline_rect(ui, x, y, 16, 16, PANEL_BORDER_COLOR);
    draw_text(ui, x + 5, y + 5, 1, label, TEXT_PRIMARY);
    ui.hit_regions.push(EditorUiHitRegion {
        x,
        y,
        w: 16,
        h: 16,
        commands,
    });
}

fn fill_rect(ui: &mut EditorNativeUi, x: i32, y: i32, w: i32, h: i32, color: Color) {
    if w <= 0 || h <= 0 {
        return;
    }
    ui.quads.push(ScreenQuad::new(
        x as f32, y as f32, w as f32, h as f32, color,
    ));
}

fn outline_rect(ui: &mut EditorNativeUi, x: i32, y: i32, w: i32, h: i32, color: Color) {
    fill_rect(ui, x, y, w, 1, color);
    fill_rect(ui, x, y + h - 1, w, 1, color);
    fill_rect(ui, x, y, 1, h, color);
    fill_rect(ui, x + w - 1, y, 1, h, color);
}

fn draw_text(ui: &mut EditorNativeUi, x: i32, y: i32, scale: i32, text: &str, color: Color) {
    let mut cursor_x = x;
    for ch in text.chars() {
        if ch == '\n' {
            cursor_x = x;
            continue;
        }
        draw_glyph(ui, cursor_x, y, scale, ch, color);
        cursor_x += glyph_advance(ch, scale);
    }
}

fn draw_glyph(ui: &mut EditorNativeUi, x: i32, y: i32, scale: i32, ch: char, color: Color) {
    let glyph = glyph_rows(ch.to_ascii_uppercase());
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..5 {
            if bits & (1 << (4 - col)) != 0 {
                fill_rect(
                    ui,
                    x + col * scale,
                    y + row as i32 * scale,
                    scale,
                    scale,
                    color,
                );
            }
        }
    }
}

fn glyph_advance(ch: char, scale: i32) -> i32 {
    if ch == ' ' {
        4 * scale
    } else {
        6 * scale
    }
}

fn text_pixel_width(text: &str, scale: i32) -> i32 {
    text.chars().map(|ch| glyph_advance(ch, scale)).sum()
}

fn format_field_label(name: &str, value: &InspectorValue) -> String {
    match value {
        InspectorValue::Number(number) => format!("{} {:.2}", name, number),
        InspectorValue::Text(text) => format!("{} {}", name, truncate_text(text, 16)),
        InspectorValue::Vec3(value) => {
            format!("{} {:.2} {:.2} {:.2}", name, value[0], value[1], value[2])
        }
        InspectorValue::Bool(value) => {
            format!("{} {}", name, if *value { "TRUE" } else { "FALSE" })
        }
    }
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        text.chars().take(max_chars).collect::<String>() + "..."
    }
}

fn glyph_rows(ch: char) -> [u8; 7] {
    match ch {
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0F, 0x10, 0x10, 0x10, 0x10, 0x10, 0x0F],
        'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0F, 0x10, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x1F],
        'J' => [0x01, 0x01, 0x01, 0x01, 0x11, 0x11, 0x0E],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x14, 0x04, 0x04, 0x04, 0x1F],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1E, 0x01, 0x01, 0x0E, 0x01, 0x01, 0x1E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x10, 0x1E, 0x01, 0x01, 0x1E],
        '6' => [0x0E, 0x10, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x01, 0x0E],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x06],
        ':' => [0x00, 0x06, 0x06, 0x00, 0x06, 0x06, 0x00],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        '/' => [0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F],
        '*' => [0x00, 0x11, 0x0A, 0x1F, 0x0A, 0x11, 0x00],
        '=' => [0x00, 0x1F, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '[' => [0x0E, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0E],
        ']' => [0x0E, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0E],
        '?' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
        ' ' => [0x00; 7],
        _ => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{
        EditorEntity, EditorSceneSession, InspectorComponent, InspectorField, SceneDocument,
    };

    fn test_host() -> EditorRuntimeHost {
        let scene = SceneDocument {
            name: "Ui".into(),
            entities: vec![EditorEntity {
                id: 1,
                name: "Camera".into(),
                components: vec![
                    InspectorComponent {
                        type_name: "Transform".into(),
                        fields: vec![InspectorField {
                            name: "position".into(),
                            value: InspectorValue::Vec3([0.0, 2.0, 10.0]),
                        }],
                    },
                    InspectorComponent {
                        type_name: "Camera".into(),
                        fields: vec![InspectorField {
                            name: "fov_deg".into(),
                            value: InspectorValue::Number(60.0),
                        }],
                    },
                ],
            }],
        };

        EditorRuntimeHost::new(EditorSceneSession::new(scene)).unwrap()
    }

    #[test]
    fn hierarchy_row_click_maps_to_select_entity_command() {
        let host = test_host();
        let ui = build_native_ui(&host, 1280, 720);

        let commands = ui.commands_at(24, 74).unwrap();
        assert!(matches!(
            commands.first(),
            Some(EditorCommand::SelectEntity { entity_id: 1 })
        ));
    }

    #[test]
    fn inspector_stepper_click_emits_select_and_delta_commands() {
        let mut host = test_host();
        host.execute(EditorCommand::SelectField {
            entity_id: 1,
            component_name: "Camera".into(),
            field_name: "fov_deg".into(),
        })
        .unwrap();

        let ui = build_native_ui(&host, 1280, 720);
        let commands = ui.commands_at(1245, 122).unwrap();
        assert!(commands.len() >= 2);
        assert!(matches!(
            commands[0],
            EditorCommand::SelectField { entity_id: 1, .. }
        ));
        assert!(commands
            .iter()
            .any(|command| matches!(command, EditorCommand::ApplyDelta(_))));
    }
}
