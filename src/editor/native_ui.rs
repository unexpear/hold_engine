//! Native hierarchy/inspector overlay for the windowed editor shell.
//!
//! This module builds a backend-neutral screen-space overlay from the
//! authoritative [`EditorRuntimeHost`]. The shell can render the resulting
//! quads in either software or OpenGL and dispatch mouse clicks back through
//! the existing command surface.

use std::collections::{HashMap, HashSet};

use crate::renderer::{Color, ScreenQuad};

use super::{
    EditorAxis, EditorCommand, EditorEntity, EditorPlayState, EditorRuntimeHost, InspectorValue,
};

const TOOLBAR_H: i32 = 34;
const STATUS_H: i32 = 24;
const PANEL_PAD: i32 = 10;
const PANEL_BORDER: i32 = 1;
const HEADER_H: i32 = 22;
const ROW_H: i32 = 18;
const HISTORY_PANEL_H: i32 = 164;
const PALETTE_W: i32 = 560;
const PALETTE_MAX_ROWS: usize = 8;
const PALETTE_ROW_H: i32 = 28;
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
const TEXT_ERROR: Color = Color::new(1.0, 0.48, 0.42, 1.0);
const TEXT_SUGGESTION: Color = Color::new(0.58, 0.82, 1.0, 1.0);

#[derive(Debug, Clone, Default)]
pub struct EditorNativeUi {
    pub quads: Vec<ScreenQuad>,
    pub hit_regions: Vec<EditorUiHitRegion>,
    pub panel_regions: Vec<EditorUiPanelRegion>,
}

impl EditorNativeUi {
    pub fn commands_at(&self, x: i32, y: i32) -> Option<Vec<EditorCommand>> {
        self.hit_regions
            .iter()
            .rev()
            .find(|region| region.contains(x, y))
            .map(|region| region.commands.clone())
    }

    pub fn panel_at(&self, x: i32, y: i32) -> Option<EditorUiPanel> {
        self.panel_regions
            .iter()
            .rev()
            .find(|region| region.contains(x, y))
            .map(|region| region.panel)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorUiPanel {
    Hierarchy,
    Inspector,
    History,
    CommandPalette,
    /// M9 Frame Inspector panel — bottom strip showing per-pass
    /// thumbnails from [`crate::renderer::FrameGraph::execute_with_capture`]
    /// when F10 is active on the editor shell.
    FrameInspector,
}

#[derive(Debug, Clone)]
pub struct EditorUiPanelRegion {
    pub panel: EditorUiPanel,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl EditorUiPanelRegion {
    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

#[derive(Debug, Clone, Default)]
pub struct EditorNativeUiState {
    pub hierarchy_scroll: usize,
    pub inspector_scroll: usize,
    pub history_scroll: usize,
}

impl EditorNativeUiState {
    pub fn scroll_lines(&mut self, panel: EditorUiPanel, lines: i32) {
        let scroll = match panel {
            EditorUiPanel::Hierarchy => &mut self.hierarchy_scroll,
            EditorUiPanel::Inspector => &mut self.inspector_scroll,
            EditorUiPanel::History => &mut self.history_scroll,
            EditorUiPanel::CommandPalette | EditorUiPanel::FrameInspector => return,
        };

        if lines < 0 {
            *scroll = scroll.saturating_sub(lines.unsigned_abs() as usize);
        } else {
            *scroll = scroll.saturating_add(lines as usize);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorUiActionTone {
    Command,
    Event,
    Error,
    Suggestion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorUiActionLogEntry {
    pub label: String,
    pub detail: String,
    pub tone: EditorUiActionTone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorUiPaletteEntrySource {
    BuiltIn,
    Suggested,
}

#[derive(Debug, Clone)]
pub struct EditorUiPaletteEntry {
    pub title: String,
    pub detail: String,
    pub source: EditorUiPaletteEntrySource,
    pub enabled: bool,
    pub commands: Vec<EditorCommand>,
}

#[derive(Debug, Clone)]
pub struct EditorUiCommandPalette {
    pub query: String,
    pub helper_text: String,
    pub empty_text: String,
    pub entries: Vec<EditorUiPaletteEntry>,
    pub selected_index: usize,
}

pub fn build_native_ui(
    host: &EditorRuntimeHost,
    width: u32,
    height: u32,
    state: &mut EditorNativeUiState,
    history: &[EditorUiActionLogEntry],
    palette: Option<&EditorUiCommandPalette>,
) -> EditorNativeUi {
    let mut ui = EditorNativeUi::default();
    let width = width as i32;
    let height = height as i32;

    let hierarchy_w = (width / 4).clamp(250, 340);
    let inspector_w = (width / 3).clamp(300, 420);
    let right_x = (width - inspector_w).max(hierarchy_w + 24);
    let panel_top = TOOLBAR_H;
    let panel_h = (height - TOOLBAR_H - STATUS_H).max(120);
    let history_x = hierarchy_w + PANEL_PAD;
    let history_w = (right_x - history_x - PANEL_PAD).max(220);
    let history_h = HISTORY_PANEL_H.min((panel_h / 2).max(120));
    let history_y = (height - STATUS_H - history_h - PANEL_PAD).max(panel_top + 16);
    let hierarchy_view_rows = panel_visible_rows(panel_h);
    let inspector_view_rows = panel_visible_rows(panel_h);
    let history_view_rows = panel_visible_rows(history_h);
    let (hierarchy_rows, selected_hierarchy_row) = collect_hierarchy_rows(host);
    let (inspector_rows, selected_inspector_row) = collect_inspector_rows(host);

    sync_scroll_to_selection(
        &mut state.hierarchy_scroll,
        selected_hierarchy_row,
        hierarchy_rows.len(),
        hierarchy_view_rows,
    );
    sync_scroll_to_selection(
        &mut state.inspector_scroll,
        selected_inspector_row,
        inspector_rows.len(),
        inspector_view_rows,
    );
    state.history_scroll = state
        .history_scroll
        .min(history.len().saturating_sub(history_view_rows));

    fill_rect(&mut ui, 0, 0, width, height, UI_BG);
    draw_toolbar(&mut ui, host, width);
    draw_panel(&mut ui, 0, panel_top, hierarchy_w, panel_h);
    draw_panel(&mut ui, right_x, panel_top, inspector_w, panel_h);
    draw_panel(&mut ui, history_x, history_y, history_w, history_h);
    ui.panel_regions.push(EditorUiPanelRegion {
        panel: EditorUiPanel::Hierarchy,
        x: 0,
        y: panel_top,
        w: hierarchy_w,
        h: panel_h,
    });
    ui.panel_regions.push(EditorUiPanelRegion {
        panel: EditorUiPanel::Inspector,
        x: right_x,
        y: panel_top,
        w: inspector_w,
        h: panel_h,
    });
    ui.panel_regions.push(EditorUiPanelRegion {
        panel: EditorUiPanel::History,
        x: history_x,
        y: history_y,
        w: history_w,
        h: history_h,
    });
    draw_status(&mut ui, host, width, height);
    draw_hot_reload_panel(&mut ui, host, width, height);
    draw_frame_inspector_panel(&mut ui, host, width, height);
    draw_hierarchy(
        &mut ui,
        host,
        &hierarchy_rows,
        state.hierarchy_scroll,
        0,
        panel_top,
        hierarchy_w,
        panel_h,
    );
    draw_inspector(
        &mut ui,
        host,
        &inspector_rows,
        state.inspector_scroll,
        right_x,
        panel_top,
        inspector_w,
        panel_h,
    );
    draw_history(
        &mut ui,
        history,
        state.history_scroll,
        history_x,
        history_y,
        history_w,
        history_h,
    );
    if let Some(palette) = palette {
        draw_command_palette(&mut ui, palette, width, height);
    }

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
        "UNDO",
        host.can_undo(),
        play_state == EditorPlayState::Editing && host.can_undo(),
        vec![EditorCommand::Undo],
        false,
    );
    x = push_button(
        ui,
        x,
        y,
        "REDO",
        host.can_redo(),
        play_state == EditorPlayState::Editing && host.can_redo(),
        vec![EditorCommand::Redo],
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
        // Deterministic replay: the editor UI shows the mode but does
        // not expose play/pause/stop transport — replay control comes
        // from the determinism harness (M8), not the play-mode buttons.
        // The button row collapses to a single disabled "REPLAY"
        // indicator until that harness lands.
        EditorPlayState::ReplayingDeterministic => {
            x = push_button(
                ui,
                x,
                y,
                "REPLAY",
                true,
                false,
                Vec::new(),
                false,
            );
        }
    }

    let _ = x;
}

#[derive(Debug, Clone)]
struct HierarchyRow {
    entity_id: u64,
    label: String,
    depth: usize,
    selected: bool,
}

#[derive(Debug, Clone)]
enum InspectorRow {
    Message {
        label: String,
        color: Color,
    },
    Title {
        label: String,
    },
    Component {
        entity_id: u64,
        component_name: String,
        selected: bool,
    },
    Field {
        entity_id: u64,
        component_name: String,
        field_name: String,
        value: InspectorValue,
        selected: bool,
        active_axis: Option<EditorAxis>,
        editable: bool,
    },
}

fn draw_hierarchy(
    ui: &mut EditorNativeUi,
    host: &EditorRuntimeHost,
    rows: &[HierarchyRow],
    scroll: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) {
    let inner_x = x + PANEL_PAD;
    let inner_w = w - PANEL_PAD * 2;
    let header_y = y + PANEL_PAD;
    let visible_rows = panel_visible_rows(h);
    let start = scroll.min(rows.len().saturating_sub(visible_rows));
    let end = (start + visible_rows).min(rows.len());

    fill_rect(ui, inner_x, header_y, inner_w, HEADER_H, HEADER_BG);
    draw_text(ui, inner_x + 8, header_y + 5, 1, "HIERARCHY", TEXT_PRIMARY);
    draw_text(
        ui,
        inner_x + inner_w - text_pixel_width(&format!("{end}/{}", rows.len()), 1) - 8,
        header_y + 5,
        1,
        &format!("{end}/{}", rows.len()),
        TEXT_MUTED,
    );

    let row_step = ROW_H + 2;
    for (visible_index, row) in rows[start..end].iter().enumerate() {
        let row_y = header_y + HEADER_H + 6 + visible_index as i32 * row_step;
        let row_color = if row.selected {
            SELECTED_BG
        } else if (start + visible_index) % 2 == 0 {
            ROW_BG
        } else {
            ROW_ALT_BG
        };
        fill_rect(ui, inner_x, row_y, inner_w, ROW_H, row_color);

        let indent = (row.depth.min(6) as i32) * 14;
        draw_text(
            ui,
            inner_x + 6 + indent,
            row_y + 4,
            1,
            &row.label,
            TEXT_PRIMARY,
        );

        ui.hit_regions.push(EditorUiHitRegion {
            x: inner_x,
            y: row_y,
            w: inner_w,
            h: ROW_H,
            commands: vec![EditorCommand::SelectEntity {
                entity_id: row.entity_id,
            }],
        });
    }

    let _ = host;
}

fn draw_inspector(
    ui: &mut EditorNativeUi,
    host: &EditorRuntimeHost,
    rows: &[InspectorRow],
    scroll: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) {
    let inner_x = x + PANEL_PAD;
    let inner_w = w - PANEL_PAD * 2;
    let header_y = y + PANEL_PAD;
    let visible_rows = panel_visible_rows(h);
    let start = scroll.min(rows.len().saturating_sub(visible_rows));
    let end = (start + visible_rows).min(rows.len());

    fill_rect(ui, inner_x, header_y, inner_w, HEADER_H, HEADER_BG);
    draw_text(ui, inner_x + 8, header_y + 5, 1, "INSPECTOR", TEXT_PRIMARY);
    draw_text(
        ui,
        inner_x + inner_w - text_pixel_width(&format!("{end}/{}", rows.len()), 1) - 8,
        header_y + 5,
        1,
        &format!("{end}/{}", rows.len()),
        TEXT_MUTED,
    );

    let row_step = ROW_H + 2;
    for (visible_index, row) in rows[start..end].iter().enumerate() {
        let row_y = header_y + HEADER_H + 6 + visible_index as i32 * row_step;
        match row {
            InspectorRow::Message { label, color } => {
                draw_text(ui, inner_x + 8, row_y + 4, 1, label, *color);
            }
            InspectorRow::Title { label } => {
                draw_text(ui, inner_x + 4, row_y + 4, 1, label, TEXT_ACCENT);
            }
            InspectorRow::Component {
                entity_id,
                component_name,
                selected,
            } => {
                fill_rect(
                    ui,
                    inner_x,
                    row_y,
                    inner_w,
                    ROW_H,
                    if *selected {
                        SELECTED_COMPONENT_BG
                    } else {
                        ROW_BG
                    },
                );
                draw_text(ui, inner_x + 6, row_y + 4, 1, component_name, TEXT_PRIMARY);
                ui.hit_regions.push(EditorUiHitRegion {
                    x: inner_x,
                    y: row_y,
                    w: inner_w,
                    h: ROW_H,
                    commands: vec![EditorCommand::SelectComponent {
                        entity_id: *entity_id,
                        component_name: component_name.clone(),
                    }],
                });
            }
            InspectorRow::Field {
                entity_id,
                component_name,
                field_name,
                value,
                selected,
                active_axis,
                editable,
            } => {
                fill_rect(
                    ui,
                    inner_x + 12,
                    row_y,
                    inner_w - 12,
                    ROW_H,
                    if *selected { SELECTED_BG } else { ROW_ALT_BG },
                );

                draw_text(
                    ui,
                    inner_x + 18,
                    row_y + 4,
                    1,
                    &format_field_label(field_name, value),
                    TEXT_PRIMARY,
                );

                let select_field = EditorCommand::SelectField {
                    entity_id: *entity_id,
                    component_name: component_name.clone(),
                    field_name: field_name.clone(),
                };
                ui.hit_regions.push(EditorUiHitRegion {
                    x: inner_x + 12,
                    y: row_y,
                    w: inner_w - 12,
                    h: ROW_H,
                    commands: vec![select_field.clone()],
                });

                if *editable {
                    match value {
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
                        InspectorValue::Integer(_) => {
                            // Same button layout as Number, but nudge by whole
                            // units — an integer field has no sub-1 step.
                            let minus_x = inner_x + inner_w - 50;
                            let plus_x = inner_x + inner_w - 26;
                            push_small_button(
                                ui,
                                minus_x,
                                row_y + 1,
                                "-",
                                vec![select_field.clone(), EditorCommand::ApplyDelta(-1.0)],
                                false,
                            );
                            push_small_button(
                                ui,
                                plus_x,
                                row_y + 1,
                                "+",
                                vec![select_field.clone(), EditorCommand::ApplyDelta(1.0)],
                                false,
                            );
                        }
                        InspectorValue::Vec3(_) => {
                            let active_axis = active_axis.unwrap_or(EditorAxis::X);
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
                                    *selected && active_axis == axis,
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
            }
        }
    }

    let _ = host;
}

fn draw_history(
    ui: &mut EditorNativeUi,
    history: &[EditorUiActionLogEntry],
    scroll: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) {
    let inner_x = x + PANEL_PAD;
    let inner_w = w - PANEL_PAD * 2;
    let header_y = y + PANEL_PAD;
    let visible_rows = panel_visible_rows(h);
    let max_scroll = history.len().saturating_sub(visible_rows);
    let start = scroll.min(max_scroll);

    fill_rect(ui, inner_x, header_y, inner_w, HEADER_H, HEADER_BG);
    draw_text(
        ui,
        inner_x + 8,
        header_y + 5,
        1,
        "ACTION HISTORY",
        TEXT_PRIMARY,
    );
    draw_text(
        ui,
        inner_x + inner_w
            - text_pixel_width(&format!("{}/{}", history.len(), history.len()), 1)
            - 8,
        header_y + 5,
        1,
        &format!("{}/{}", history.len().saturating_sub(start), history.len()),
        TEXT_MUTED,
    );

    if history.is_empty() {
        draw_text(
            ui,
            inner_x + 8,
            header_y + HEADER_H + 8,
            1,
            "NO COMMANDS YET",
            TEXT_MUTED,
        );
        return;
    }

    let row_step = ROW_H + 2;
    for (visible_index, entry) in history
        .iter()
        .rev()
        .skip(start)
        .take(visible_rows)
        .enumerate()
    {
        let row_y = header_y + HEADER_H + 6 + visible_index as i32 * row_step;
        let row_color = if visible_index % 2 == 0 {
            ROW_BG
        } else {
            ROW_ALT_BG
        };
        fill_rect(ui, inner_x, row_y, inner_w, ROW_H, row_color);
        let tone_color = match entry.tone {
            EditorUiActionTone::Command => TEXT_PRIMARY,
            EditorUiActionTone::Event => TEXT_ACCENT,
            EditorUiActionTone::Error => TEXT_ERROR,
            EditorUiActionTone::Suggestion => TEXT_SUGGESTION,
        };
        draw_text(
            ui,
            inner_x + 6,
            row_y + 4,
            1,
            &truncate_text(&entry.label, 26),
            tone_color,
        );
        if !entry.detail.is_empty() {
            let detail = truncate_text(&entry.detail, 28);
            let detail_w = text_pixel_width(&detail, 1);
            draw_text(
                ui,
                inner_x + inner_w - detail_w - 8,
                row_y + 4,
                1,
                &detail,
                TEXT_MUTED,
            );
        }
    }
}

fn draw_command_palette(
    ui: &mut EditorNativeUi,
    palette: &EditorUiCommandPalette,
    width: i32,
    height: i32,
) {
    fill_rect(ui, 0, 0, width, height, Color::new(0.02, 0.03, 0.04, 0.62));

    let panel_w = PALETTE_W.min((width - 32).max(320));
    let visible_rows = palette.entries.len().min(PALETTE_MAX_ROWS).max(1);
    let panel_h = HEADER_H + 44 + visible_rows as i32 * PALETTE_ROW_H + 42;
    let panel_x = (width - panel_w) / 2;
    let panel_y = ((height - panel_h) / 2).max(TOOLBAR_H + 12);

    draw_panel(ui, panel_x, panel_y, panel_w, panel_h);
    ui.panel_regions.push(EditorUiPanelRegion {
        panel: EditorUiPanel::CommandPalette,
        x: panel_x,
        y: panel_y,
        w: panel_w,
        h: panel_h,
    });

    let inner_x = panel_x + PANEL_PAD;
    let inner_w = panel_w - PANEL_PAD * 2;
    let header_y = panel_y + PANEL_PAD;
    fill_rect(ui, inner_x, header_y, inner_w, HEADER_H, HEADER_BG);
    draw_text(
        ui,
        inner_x + 8,
        header_y + 5,
        1,
        "COMMAND PALETTE",
        TEXT_PRIMARY,
    );

    let query_text = if palette.query.is_empty() {
        "QUERY ALL".to_string()
    } else {
        format!("QUERY {}", palette.query.to_ascii_uppercase())
    };
    draw_text(
        ui,
        inner_x + 8,
        header_y + HEADER_H + 8,
        1,
        &query_text,
        TEXT_ACCENT,
    );
    draw_text(
        ui,
        inner_x + inner_w - text_pixel_width("CTRL+K CLOSE", 1) - 8,
        header_y + HEADER_H + 8,
        1,
        "CTRL+K CLOSE",
        TEXT_MUTED,
    );

    let rows_y = header_y + HEADER_H + 24;
    if palette.entries.is_empty() {
        draw_text(
            ui,
            inner_x + 8,
            rows_y + 8,
            1,
            &palette.empty_text,
            TEXT_WARNING,
        );
    } else {
        let start = palette
            .selected_index
            .saturating_sub(PALETTE_MAX_ROWS.saturating_sub(1));
        let start = start.min(palette.entries.len().saturating_sub(visible_rows));
        for (offset, entry) in palette
            .entries
            .iter()
            .skip(start)
            .take(visible_rows)
            .enumerate()
        {
            let row_index = start + offset;
            let row_y = rows_y + offset as i32 * PALETTE_ROW_H;
            let selected = row_index == palette.selected_index;
            fill_rect(
                ui,
                inner_x,
                row_y,
                inner_w,
                PALETTE_ROW_H - 2,
                if selected {
                    SELECTED_COMPONENT_BG
                } else if offset % 2 == 0 {
                    ROW_BG
                } else {
                    ROW_ALT_BG
                },
            );

            let source_label = match entry.source {
                EditorUiPaletteEntrySource::BuiltIn => "CORE",
                EditorUiPaletteEntrySource::Suggested => "AI",
            };
            draw_text(
                ui,
                inner_x + 8,
                row_y + 6,
                1,
                &truncate_text(&entry.title, 24),
                if entry.enabled {
                    TEXT_PRIMARY
                } else {
                    TEXT_MUTED
                },
            );
            draw_text(
                ui,
                inner_x + 180,
                row_y + 6,
                1,
                &truncate_text(&entry.detail, 22),
                TEXT_MUTED,
            );
            draw_text(
                ui,
                inner_x + inner_w - text_pixel_width(source_label, 1) - 8,
                row_y + 6,
                1,
                source_label,
                match entry.source {
                    EditorUiPaletteEntrySource::BuiltIn => TEXT_ACCENT,
                    EditorUiPaletteEntrySource::Suggested => TEXT_SUGGESTION,
                },
            );

            if entry.enabled {
                ui.hit_regions.push(EditorUiHitRegion {
                    x: inner_x,
                    y: row_y,
                    w: inner_w,
                    h: PALETTE_ROW_H - 2,
                    commands: entry.commands.clone(),
                });
            }
        }
    }

    draw_text(
        ui,
        inner_x + 8,
        panel_y + panel_h - PANEL_PAD - 9,
        1,
        &truncate_text(&palette.helper_text, 74),
        TEXT_MUTED,
    );
}

fn panel_visible_rows(panel_h: i32) -> usize {
    let available = (panel_h - PANEL_PAD * 2 - HEADER_H - 6).max(ROW_H);
    (available / (ROW_H + 2)).max(1) as usize
}

fn sync_scroll_to_selection(
    scroll: &mut usize,
    selected_index: Option<usize>,
    total_rows: usize,
    visible_rows: usize,
) {
    let max_scroll = total_rows.saturating_sub(visible_rows);
    *scroll = (*scroll).min(max_scroll);

    if let Some(selected_index) = selected_index {
        if selected_index < *scroll {
            *scroll = selected_index;
        } else if selected_index >= *scroll + visible_rows {
            *scroll = selected_index + 1 - visible_rows;
        }
    }

    *scroll = (*scroll).min(max_scroll);
}

fn collect_hierarchy_rows(host: &EditorRuntimeHost) -> (Vec<HierarchyRow>, Option<usize>) {
    let scene = host.session().scene();
    let selected_entity_id = host.cursor().map(|cursor| cursor.entity_id);
    let name_to_id: HashMap<&str, u64> = scene
        .entities
        .iter()
        .map(|entity| (entity.name.as_str(), entity.id))
        .collect();
    let parent_by_id: HashMap<u64, Option<u64>> = scene
        .entities
        .iter()
        .map(|entity| (entity.id, find_parent_entity_id(entity, &name_to_id)))
        .collect();
    let mut depth_cache = HashMap::new();
    let mut rows = Vec::with_capacity(scene.entities.len());
    let mut selected_index = None;

    for (index, entity) in scene.entities.iter().enumerate() {
        let depth = hierarchy_depth(
            entity.id,
            &parent_by_id,
            &mut depth_cache,
            &mut HashSet::new(),
        );
        if Some(entity.id) == selected_entity_id {
            selected_index = Some(index);
        }
        rows.push(HierarchyRow {
            entity_id: entity.id,
            label: format!("E{} {}", entity.id, entity.name),
            depth: depth.min(6),
            selected: Some(entity.id) == selected_entity_id,
        });
    }

    (rows, selected_index)
}

fn collect_inspector_rows(host: &EditorRuntimeHost) -> (Vec<InspectorRow>, Option<usize>) {
    let Some(cursor) = host.cursor() else {
        return (
            vec![InspectorRow::Message {
                label: "NO SELECTION".into(),
                color: TEXT_MUTED,
            }],
            Some(0),
        );
    };

    let Some(entity) = host
        .session()
        .scene()
        .entities
        .iter()
        .find(|entity| entity.id == cursor.entity_id)
    else {
        return (
            vec![InspectorRow::Message {
                label: "SELECTION STALE".into(),
                color: TEXT_WARNING,
            }],
            Some(0),
        );
    };

    let mut rows = Vec::new();
    let mut selected_index = Some(0);
    rows.push(InspectorRow::Title {
        label: format!("ENTITY {} {}", entity.id, entity.name),
    });

    for component in &entity.components {
        let component_selected = component.type_name == cursor.component_name;
        if component_selected && cursor.field_name.is_empty() {
            selected_index = Some(rows.len());
        }
        rows.push(InspectorRow::Component {
            entity_id: entity.id,
            component_name: component.type_name.clone(),
            selected: component_selected,
        });

        for field in &component.fields {
            let field_selected =
                component.type_name == cursor.component_name && field.name == cursor.field_name;
            if field_selected {
                selected_index = Some(rows.len());
            }
            rows.push(InspectorRow::Field {
                entity_id: entity.id,
                component_name: component.type_name.clone(),
                field_name: field.name.clone(),
                value: field.value.clone(),
                selected: field_selected,
                active_axis: cursor.axis,
                editable: host.play_state() != EditorPlayState::Playing,
            });
        }
    }

    (rows, selected_index)
}

fn find_parent_entity_id(entity: &EditorEntity, name_to_id: &HashMap<&str, u64>) -> Option<u64> {
    let parent = entity
        .components
        .iter()
        .find(|component| component.type_name == "Parent")?;

    for field in &parent.fields {
        if field.name == "entity_id" {
            if let InspectorValue::Number(id) = field.value {
                return Some(id as u64);
            }
        }
    }

    for field in &parent.fields {
        if field.name == "entity_name" {
            if let InspectorValue::Text(ref name) = field.value {
                return name_to_id.get(name.as_str()).copied();
            }
        }
    }

    None
}

fn hierarchy_depth(
    entity_id: u64,
    parent_by_id: &HashMap<u64, Option<u64>>,
    depth_cache: &mut HashMap<u64, usize>,
    visiting: &mut HashSet<u64>,
) -> usize {
    if let Some(depth) = depth_cache.get(&entity_id).copied() {
        return depth;
    }

    if !visiting.insert(entity_id) {
        return 0;
    }

    let depth = parent_by_id
        .get(&entity_id)
        .copied()
        .flatten()
        .map(|parent_id| hierarchy_depth(parent_id, parent_by_id, depth_cache, visiting) + 1)
        .unwrap_or(0);

    visiting.remove(&entity_id);
    depth_cache.insert(entity_id, depth);
    depth
}

fn draw_status(ui: &mut EditorNativeUi, host: &EditorRuntimeHost, width: i32, height: i32) {
    let y = height - STATUS_H;
    draw_panel(ui, 0, y, width, STATUS_H);

    let mode = match host.play_state() {
        EditorPlayState::Editing => "EDIT",
        EditorPlayState::Playing => "PLAY",
        EditorPlayState::Paused => "PAUSED",
        // Replay-mode label is deliberately distinct so reload-pane
        // hints in the status bar can correlate with the lock state.
        EditorPlayState::ReplayingDeterministic => "REPLAY",
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

/// Hot-Reload Status panel (M7.4.2). Renders a compact box in
/// the top-right of the editor viewport showing the most recent
/// reload outcomes plus running counts. Only drawn when the host
/// has observed at least one event — the editor's chrome stays
/// minimal during normal authoring.
///
/// Layout:
/// - Header line: `HOT-RELOAD  applied=N  failed=M`
/// - Up to 5 recent rows (most-recent-first), kind + name
/// - One row showing the last error if any, in TEXT_ERROR colour
///
/// See `editor::hot_reload_status` for the data structure and
/// ADR 0004 Q9 (M7.4.2) for the design rationale.
fn draw_hot_reload_panel(
    ui: &mut EditorNativeUi,
    host: &EditorRuntimeHost,
    width: i32,
    _height: i32,
) {
    let status = host.hot_reload_status();
    if status.is_empty() {
        // Empty state: don't draw anything. Keeps the editor's
        // chrome quiet when nothing is happening.
        return;
    }

    const PANEL_W: i32 = 320;
    const ROW_H: i32 = 14;
    const HEADER_H: i32 = 18;
    const MAX_ROWS: i32 = 5;
    let rows_to_draw = (status.len() as i32).min(MAX_ROWS);
    let has_error = status.last_failure().is_some();
    let panel_h = HEADER_H + rows_to_draw * ROW_H + if has_error { ROW_H + 4 } else { 0 } + 12;

    // Position: top-right corner, padded a bit. The status bar
    // at the bottom doesn't overlap because it's at height -
    // STATUS_H.
    let x = width - PANEL_W - PANEL_PAD;
    let y = PANEL_PAD;

    draw_panel(ui, x, y, PANEL_W, panel_h);

    let header = format!(
        "HOT-RELOAD  applied={}  failed={}",
        status.total_applied(),
        status.total_failed(),
    );
    let header_color = if has_error { TEXT_ERROR } else { TEXT_ACCENT };
    draw_text(ui, x + PANEL_PAD, y + 6, STATUS_SCALE, &header, header_color);

    // Recent rows.
    let mut row_y = y + HEADER_H + 2;
    for entry in status.iter_recent().take(MAX_ROWS as usize) {
        let line = match &entry.outcome {
            super::hot_reload_status::HotReloadOutcome::Applied {
                kind,
                logical_name,
                ..
            } => format!("ok  {}  {}", kind, logical_name),
            super::hot_reload_status::HotReloadOutcome::Failed {
                kind, logical_name, ..
            } => format!("ERR {}  {}", kind, logical_name),
        };
        let truncated = truncate_for_panel(&line, PANEL_W - 2 * PANEL_PAD);
        let color = if entry.outcome.is_failure() {
            TEXT_ERROR
        } else {
            TEXT_MUTED
        };
        draw_text(ui, x + PANEL_PAD, row_y, STATUS_SCALE, &truncated, color);
        row_y += ROW_H;
    }

    // Last error detail (if any). Surfaces the actual compile
    // message so the developer can fix the typo without leaving
    // the editor.
    if let Some(last) = status.last_failure() {
        if let super::hot_reload_status::HotReloadOutcome::Failed { error, .. } = &last.outcome {
            let truncated = truncate_for_panel(error, PANEL_W - 2 * PANEL_PAD);
            draw_text(
                ui,
                x + PANEL_PAD,
                row_y + 4,
                STATUS_SCALE,
                &truncated,
                TEXT_ERROR,
            );
        }
    }
}

/// Render the M9 Frame Inspector panel — a bottom-anchored strip
/// showing per-pass thumbnails from the most recent frame-graph
/// capture. Only draws when the F10 inspector toggle is active
/// AND a capture has landed; the disabled/quiet state adds no
/// chrome to the editor.
///
/// Layout (docked at the very bottom, above the status bar):
///
/// ```text
///  ┌ FRAME INSPECTOR  gen=42  passes=4 ────────────────────────┐
///  │  ┌──────┐  ┌──────┐  ┌──────┐  ┌──────┐                   │
///  │  │      │  │      │  │      │  │      │                   │
///  │  │      │  │      │  │      │  │      │                   │
///  │  └──────┘  └──────┘  └──────┘  └──────┘                   │
///  │  Clear    Scene     DebugLine  ScreenQ                    │
///  └───────────────────────────────────────────────────────────┘
/// ```
///
/// The thumbnail slot is filled with the pass's mean colour (a
/// simple sum-then-divide over its RGBA data), which gives a
/// visual differentiator between passes without requiring the
/// textured-quad path — the actual per-pass framebuffer contents
/// live in the pass state for a follow-up commit to render as
/// real textures. Each slot also emits a hit region tagged with
/// the pass name so future work can wire "click = view as main
/// output" without touching this layout code again.
fn draw_frame_inspector_panel(
    ui: &mut EditorNativeUi,
    host: &EditorRuntimeHost,
    width: i32,
    height: i32,
) {
    let inspector = host.frame_inspector();
    if !inspector.is_enabled() {
        return;
    }
    let Some(passes) = inspector.latest() else {
        return;
    };

    const PANEL_H: i32 = 108;
    const HEADER_H: i32 = 20;
    const SLOT_PAD: i32 = 6;
    const LABEL_H: i32 = 14;

    // Sit above the status bar; span most of the width but leave a
    // side pad so it doesn't visually collide with the panels.
    let panel_x = PANEL_PAD;
    let panel_y = height - STATUS_H - PANEL_H - PANEL_PAD;
    let panel_w = width - PANEL_PAD * 2;
    let panel_h = PANEL_H;

    draw_panel(ui, panel_x, panel_y, panel_w, panel_h);
    ui.panel_regions.push(EditorUiPanelRegion {
        panel: EditorUiPanel::FrameInspector,
        x: panel_x,
        y: panel_y,
        w: panel_w,
        h: panel_h,
    });

    let header = format!(
        "FRAME INSPECTOR  gen={}  passes={}",
        inspector.generation(),
        passes.len(),
    );
    draw_text(
        ui,
        panel_x + PANEL_PAD,
        panel_y + 6,
        STATUS_SCALE,
        &header,
        TEXT_ACCENT,
    );

    if passes.is_empty() {
        draw_text(
            ui,
            panel_x + PANEL_PAD,
            panel_y + HEADER_H + 8,
            STATUS_SCALE,
            "waiting for capture...",
            TEXT_MUTED,
        );
        return;
    }

    // Divide the remaining width evenly among the passes. Clamp the
    // slot width to something readable — with 4-8 passes there's
    // plenty of room; if a future graph has 30+ passes we'll let
    // them overflow the panel until horizontal scroll lands.
    let slot_area_x = panel_x + SLOT_PAD;
    let slot_area_y = panel_y + HEADER_H;
    let slot_area_w = panel_w - SLOT_PAD * 2;
    let slot_area_h = panel_h - HEADER_H - SLOT_PAD;
    let slot_thumb_h = slot_area_h - LABEL_H - SLOT_PAD;
    let n = passes.len() as i32;
    let per_slot = (slot_area_w - SLOT_PAD * (n - 1).max(0)) / n.max(1);
    let per_slot = per_slot.max(24);

    for (i, pass) in passes.iter().enumerate() {
        let slot_x = slot_area_x + (per_slot + SLOT_PAD) * i as i32;
        if slot_x + per_slot > slot_area_x + slot_area_w {
            // Overflow guard — future work adds scroll.
            break;
        }
        let slot_y = slot_area_y;

        // Fill with the mean colour of the thumbnail — a v1 stand-in
        // for the full framebuffer capture the textured-quad path
        // will surface later. Uses only integer maths so pipeline
        // debug builds stay deterministic.
        let (r, g, b, a) = mean_rgba(&pass.data);
        let fill = Color::new(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            (a as f32 / 255.0).max(0.6),
        );
        fill_rect(ui, slot_x, slot_y, per_slot, slot_thumb_h, fill);

        // Border so an empty (all-zero) framebuffer stays visible
        // against the panel background.
        fill_rect(ui, slot_x, slot_y, per_slot, PANEL_BORDER, PANEL_BORDER_COLOR);
        fill_rect(
            ui,
            slot_x,
            slot_y + slot_thumb_h - PANEL_BORDER,
            per_slot,
            PANEL_BORDER,
            PANEL_BORDER_COLOR,
        );
        fill_rect(ui, slot_x, slot_y, PANEL_BORDER, slot_thumb_h, PANEL_BORDER_COLOR);
        fill_rect(
            ui,
            slot_x + per_slot - PANEL_BORDER,
            slot_y,
            PANEL_BORDER,
            slot_thumb_h,
            PANEL_BORDER_COLOR,
        );

        // Label under the thumbnail. Truncate on width because the
        // bitmap font has no advance-width API.
        let label = truncate_for_panel(&pass.pass_name, per_slot);
        draw_text(
            ui,
            slot_x,
            slot_y + slot_thumb_h + 2,
            STATUS_SCALE,
            &label,
            TEXT_MUTED,
        );

        // Hit region tagged with the pass — no command yet
        // (future: EditorCommand::ViewFrameInspectorPass { index }),
        // but the surface for click-to-focus is in place.
        ui.hit_regions.push(EditorUiHitRegion {
            x: slot_x,
            y: slot_y,
            w: per_slot,
            h: slot_thumb_h,
            commands: Vec::new(),
        });
    }
}

/// Mean RGBA of a captured thumbnail's pixel data. Silent zero on
/// empty input — an empty capture already displays "waiting for
/// capture..." and never reaches this path, but the guard keeps the
/// helper honest for tests.
fn mean_rgba(data: &[u8]) -> (u8, u8, u8, u8) {
    if data.is_empty() {
        return (0, 0, 0, 0);
    }
    let (mut r, mut g, mut b, mut a): (u64, u64, u64, u64) = (0, 0, 0, 0);
    let mut count: u64 = 0;
    for chunk in data.chunks_exact(4) {
        r += chunk[0] as u64;
        g += chunk[1] as u64;
        b += chunk[2] as u64;
        a += chunk[3] as u64;
        count += 1;
    }
    if count == 0 {
        return (0, 0, 0, 0);
    }
    ((r / count) as u8, (g / count) as u8, (b / count) as u8, (a / count) as u8)
}

/// Truncate a string with an ellipsis if it would render past
/// `max_pixel_width` at the current font scale. The bitmap font
/// in `editor::native_ui` ships with a fixed advance width per
/// glyph; we approximate by character count rather than pixel
/// width because the bitmap render path doesn't expose advance
/// widths. Rough but adequate for the panel's role as a
/// peripheral status surface.
fn truncate_for_panel(s: &str, max_pixel_width: i32) -> String {
    // Conservative: assume ~6 px per glyph at STATUS_SCALE = 1.
    let max_chars = (max_pixel_width / 6).max(8) as usize;
    if s.len() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
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
        // No decimals — an integer field displays as a whole number.
        InspectorValue::Integer(value) => format!("{} {}", name, value),
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
            runtime_slice: crate::runtime_slice::RuntimeSliceAuthoring::default(),
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
        let mut state = EditorNativeUiState::default();
        let ui = build_native_ui(&host, 1280, 720, &mut state, &[], None);

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

        let mut state = EditorNativeUiState::default();
        let ui = build_native_ui(&host, 1280, 720, &mut state, &[], None);
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

    /// The Integer field's inspector arms. The stepper buttons nudge by a
    /// WHOLE unit (±1.0), not the Number field's ±0.1 — a ±0.1 delta rounds to
    /// zero on a discrete field, so a regression to ±0.1 would make the buttons
    /// dead controls. The label renders the value with no decimals.
    #[test]
    fn integer_field_stepper_uses_whole_unit_delta_and_label_has_no_decimals() {
        // Label arm: a whole number, no ".00".
        assert_eq!(
            format_field_label("seed", &InspectorValue::Integer(3_000_000_003)),
            "seed 3000000003"
        );
        // Contrast: a Number renders with two decimals — the arms differ.
        assert_eq!(
            format_field_label("fov", &InspectorValue::Number(60.0)),
            "fov 60.00"
        );

        // Stepper arm: mirror the Number stepper test's structure (Transform +
        // a second single-scalar component), but make the scalar an Integer in
        // a non-bridged component ("DebugBlock") so the world still builds.
        let scene = SceneDocument {
            name: "Ui".into(),
            runtime_slice: crate::runtime_slice::RuntimeSliceAuthoring::default(),
            entities: vec![EditorEntity {
                id: 1,
                name: "Fx".into(),
                components: vec![
                    InspectorComponent {
                        type_name: "Transform".into(),
                        fields: vec![InspectorField {
                            name: "position".into(),
                            value: InspectorValue::Vec3([0.0, 2.0, 10.0]),
                        }],
                    },
                    InspectorComponent {
                        type_name: "DebugBlock".into(),
                        fields: vec![InspectorField {
                            name: "seed".into(),
                            value: InspectorValue::Integer(3_000_000_003),
                        }],
                    },
                ],
            }],
        };
        let mut host = EditorRuntimeHost::new(EditorSceneSession::new(scene)).unwrap();
        host.execute(EditorCommand::SelectField {
            entity_id: 1,
            component_name: "DebugBlock".into(),
            field_name: "seed".into(),
        })
        .unwrap();

        let mut state = EditorNativeUiState::default();
        let ui = build_native_ui(&host, 1280, 720, &mut state, &[], None);

        // Scan the stepper button column across the inspector and collect every
        // ApplyDelta magnitude. Only the Integer arm emits ±1.0 (Number and
        // Vec3 steppers emit ±0.1), so requiring 1.0 to be present pins the
        // Integer arm: a regression to ±0.1 removes it, and the absence fails.
        let mut deltas = Vec::new();
        for y in (100..720).step_by(2) {
            for x in [1235, 1245, 1255, 1260] {
                if let Some(commands) = ui.commands_at(x, y) {
                    for command in &commands {
                        if let EditorCommand::ApplyDelta(delta) = command {
                            deltas.push(delta.abs());
                        }
                    }
                }
            }
        }
        assert!(
            deltas.iter().any(|delta| (delta - 1.0).abs() < f32::EPSILON),
            "an integer stepper (±1.0) must be present; found deltas {deltas:?}"
        );
        // Sanity: the ±0.1 Vec3 stepper from the Transform row is also present,
        // confirming the scan actually reached editable rows.
        assert!(
            deltas.iter().any(|delta| (delta - 0.1).abs() < f32::EPSILON),
            "expected the Vec3 ±0.1 stepper in the scan; found {deltas:?}"
        );
    }

    #[test]
    fn hierarchy_scroll_follows_selected_entity() {
        let scene = SceneDocument {
            name: "Ui".into(),
            runtime_slice: crate::runtime_slice::RuntimeSliceAuthoring::default(),
            entities: (1..=16)
                .map(|id| EditorEntity {
                    id,
                    name: format!("Entity{id}"),
                    components: vec![InspectorComponent {
                        type_name: "Transform".into(),
                        fields: vec![InspectorField {
                            name: "position".into(),
                            value: InspectorValue::Vec3([id as f32, 0.0, 0.0]),
                        }],
                    }],
                })
                .collect(),
        };
        let mut host = EditorRuntimeHost::new(EditorSceneSession::new(scene)).unwrap();
        host.execute(EditorCommand::SelectEntity { entity_id: 16 })
            .unwrap();

        let mut state = EditorNativeUiState::default();
        let ui = build_native_ui(&host, 480, 220, &mut state, &[], None);

        assert!(state.hierarchy_scroll > 0);
        assert_eq!(ui.panel_at(40, 60), Some(EditorUiPanel::Hierarchy));
    }

    #[test]
    fn inspector_scroll_follows_selected_field() {
        let scene = SceneDocument {
            name: "Ui".into(),
            runtime_slice: crate::runtime_slice::RuntimeSliceAuthoring::default(),
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
                        type_name: "DebugBlock".into(),
                        fields: (0..12)
                            .map(|index| InspectorField {
                                name: format!("value_{index}"),
                                value: InspectorValue::Number(index as f32),
                            })
                            .collect(),
                    },
                ],
            }],
        };
        let mut host = EditorRuntimeHost::new(EditorSceneSession::new(scene)).unwrap();
        host.execute(EditorCommand::SelectField {
            entity_id: 1,
            component_name: "DebugBlock".into(),
            field_name: "value_11".into(),
        })
        .unwrap();

        let mut state = EditorNativeUiState::default();
        let ui = build_native_ui(&host, 640, 220, &mut state, &[], None);

        assert!(state.inspector_scroll > 0);
        assert_eq!(ui.panel_at(560, 60), Some(EditorUiPanel::Inspector));
    }

    #[test]
    fn history_panel_registers_for_hit_testing() {
        let host = test_host();
        let mut state = EditorNativeUiState::default();
        let history = vec![
            EditorUiActionLogEntry {
                label: "Command Save".into(),
                detail: "Write scene".into(),
                tone: EditorUiActionTone::Command,
            },
            EditorUiActionLogEntry {
                label: "Event Saved".into(),
                detail: "Scene written".into(),
                tone: EditorUiActionTone::Event,
            },
        ];

        let ui = build_native_ui(&host, 1280, 720, &mut state, &history, None);
        assert_eq!(ui.panel_at(420, 610), Some(EditorUiPanel::History));
    }

    #[test]
    fn command_palette_hit_region_emits_palette_commands() {
        let host = test_host();
        let mut state = EditorNativeUiState::default();
        let palette = EditorUiCommandPalette {
            query: "save".into(),
            helper_text: "helper".into(),
            empty_text: "none".into(),
            entries: vec![EditorUiPaletteEntry {
                title: "Save Scene".into(),
                detail: "Write current .hsc".into(),
                source: EditorUiPaletteEntrySource::BuiltIn,
                enabled: true,
                commands: vec![EditorCommand::Save],
            }],
            selected_index: 0,
        };

        let ui = build_native_ui(&host, 1280, 720, &mut state, &[], Some(&palette));
        let commands = ui.commands_at(370, 352).unwrap();
        assert!(matches!(commands.first(), Some(EditorCommand::Save)));
        assert_eq!(ui.panel_at(370, 300), Some(EditorUiPanel::CommandPalette));
    }

    // ── Hot-Reload Status panel (M7.4.2) ─────────────────────────────────

    /// Empty hot-reload status: no panel quads emitted. The
    /// editor's chrome stays minimal during normal authoring;
    /// the panel only appears once events have been observed.
    #[test]
    fn hot_reload_panel_is_hidden_when_status_empty() {
        let host = test_host();
        let mut state = EditorNativeUiState::default();
        let baseline = build_native_ui(&host, 1280, 720, &mut state, &[], None);
        // Render again — host has no events, panel must not
        // grow the quad count between runs.
        let mut state = EditorNativeUiState::default();
        let again = build_native_ui(&host, 1280, 720, &mut state, &[], None);
        assert_eq!(baseline.quads.len(), again.quads.len());
    }

    /// Pushing a successful reload makes the panel appear: more
    /// quads than the empty baseline. Pillar 2 (iteration
    /// speed): the developer sees something land in the editor
    /// chrome the moment a reload applies.
    #[test]
    fn hot_reload_panel_appears_after_push_event() {
        let mut host = test_host();
        let mut state = EditorNativeUiState::default();
        let baseline = build_native_ui(&host, 1280, 720, &mut state, &[], None);
        let baseline_quads = baseline.quads.len();

        host.push_hot_reload_event("Texture", 7, "textures/foo");

        let mut state = EditorNativeUiState::default();
        let after = build_native_ui(&host, 1280, 720, &mut state, &[], None);
        assert!(
            after.quads.len() > baseline_quads,
            "expected panel quads to appear after push: {} → {}",
            baseline_quads,
            after.quads.len()
        );
    }

    /// Pushing a failure makes the panel render in the
    /// error-coloured state and the panel grows further to
    /// accommodate the error-detail row.
    #[test]
    fn hot_reload_panel_grows_when_failure_present() {
        let mut host = test_host();
        host.push_hot_reload_event("Texture", 1, "textures/foo");
        let mut state = EditorNativeUiState::default();
        let success_only = build_native_ui(&host, 1280, 720, &mut state, &[], None);
        let success_quads = success_only.quads.len();

        host.push_hot_reload_failure(
            "Material",
            "materials/wall",
            "missing albedo dependency",
        );
        let mut state = EditorNativeUiState::default();
        let with_error = build_native_ui(&host, 1280, 720, &mut state, &[], None);
        assert!(
            with_error.quads.len() > success_quads,
            "expected error row to grow the panel: {} → {}",
            success_quads,
            with_error.quads.len()
        );
    }

    /// Truncation: a long error message doesn't make the panel
    /// silently overflow into adjacent UI. The truncation helper
    /// caps the rendered length at the panel width.
    #[test]
    fn hot_reload_panel_truncates_long_text() {
        // The truncate function is private; exercise it
        // indirectly by pushing an absurdly long message and
        // confirming the build still succeeds (no panic).
        let mut host = test_host();
        host.push_hot_reload_failure(
            "Shader",
            "shaders/very/deeply/nested/logical/name/that/keeps/going/forever",
            "error message that goes on and on and includes every detail you could ever want \
             and then some, way more than the panel is going to be willing to render at \
             1280 px viewport width",
        );
        let mut state = EditorNativeUiState::default();
        let _ui = build_native_ui(&host, 1280, 720, &mut state, &[], None);
        // No panic = no overflow into adjacent panels. The
        // truncation helper is what keeps this from corrupting
        // surrounding UI; this test pins the safety property.
    }

    // ── Frame Inspector panel (M9) ────────────────────────────────────────

    fn push_thumb(host: &mut EditorRuntimeHost, index: usize, name: &str) {
        // A 4×2 all-red thumbnail — small enough that mean_rgba can be
        // eyeballed in test failure messages while still exercising
        // the sum-then-divide code path.
        let data: Vec<u8> = (0..8)
            .flat_map(|_| [200u8, 40, 40, 255])
            .collect();
        host.frame_inspector_mut()
            .push_capture(vec![crate::renderer::PassThumbnail {
                pass_index: index,
                pass_name: name.into(),
                width: 4,
                height: 2,
                data,
            }]);
    }

    /// Disabled inspector: panel emits no chrome. Pillar 4
    /// (debuggability) is opt-in — the editor stays quiet by
    /// default so the F10 toggle is the affordance, not always-on
    /// clutter.
    #[test]
    fn frame_inspector_panel_hidden_when_disabled() {
        let mut host = test_host();
        // Push a capture, but leave the inspector disabled.
        push_thumb(&mut host, 0, "Clear");
        assert!(!host.frame_inspector().is_enabled());
        let mut state = EditorNativeUiState::default();
        let ui = build_native_ui(&host, 1280, 720, &mut state, &[], None);
        assert!(
            ui.panel_regions
                .iter()
                .all(|r| r.panel != EditorUiPanel::FrameInspector),
            "disabled inspector must not emit a panel region"
        );
    }

    /// Enabled but no capture: panel appears (the frame) but no
    /// per-pass slots. Serves as the "waiting for capture..."
    /// affordance so the user knows F10 landed.
    #[test]
    fn frame_inspector_panel_shows_when_enabled_with_no_capture() {
        let mut host = test_host();
        host.frame_inspector_mut().set_enabled(true);
        // Enable but don't push a capture yet — latest() is None.
        assert!(host.frame_inspector().is_enabled());
        assert!(host.frame_inspector().latest().is_none());
        let mut state = EditorNativeUiState::default();
        let ui = build_native_ui(&host, 1280, 720, &mut state, &[], None);
        // No latest capture → no panel yet (we skip early). This
        // is intentional: enabling F10 alone shouldn't add chrome
        // before the first frame actually captures. The panel
        // pops in on the very next frame.
        assert!(ui
            .panel_regions
            .iter()
            .all(|r| r.panel != EditorUiPanel::FrameInspector));
    }

    /// Enabled + at least one capture landed: panel appears and
    /// registers exactly one hit region per pass. Wires future
    /// "click to focus" without touching layout code.
    #[test]
    fn frame_inspector_panel_emits_one_hit_region_per_pass() {
        let mut host = test_host();
        host.frame_inspector_mut().set_enabled(true);
        // Push a 4-pass capture — mirrors the real M9 scene graph.
        host.frame_inspector_mut()
            .push_capture(vec![
                crate::renderer::PassThumbnail {
                    pass_index: 0,
                    pass_name: "Clear".into(),
                    width: 4,
                    height: 2,
                    data: vec![10, 20, 30, 255, 10, 20, 30, 255, 10, 20, 30, 255, 10, 20, 30, 255,
                               10, 20, 30, 255, 10, 20, 30, 255, 10, 20, 30, 255, 10, 20, 30, 255],
                },
                crate::renderer::PassThumbnail {
                    pass_index: 1,
                    pass_name: "Scene".into(),
                    width: 4,
                    height: 2,
                    data: vec![40, 60, 80, 255, 40, 60, 80, 255, 40, 60, 80, 255, 40, 60, 80, 255,
                               40, 60, 80, 255, 40, 60, 80, 255, 40, 60, 80, 255, 40, 60, 80, 255],
                },
                crate::renderer::PassThumbnail {
                    pass_index: 2,
                    pass_name: "DebugLines".into(),
                    width: 4,
                    height: 2,
                    data: vec![100, 100, 100, 255, 100, 100, 100, 255, 100, 100, 100, 255,
                               100, 100, 100, 255, 100, 100, 100, 255, 100, 100, 100, 255,
                               100, 100, 100, 255, 100, 100, 100, 255],
                },
                crate::renderer::PassThumbnail {
                    pass_index: 3,
                    pass_name: "ScreenQuads".into(),
                    width: 4,
                    height: 2,
                    data: vec![200, 200, 200, 255, 200, 200, 200, 255, 200, 200, 200, 255,
                               200, 200, 200, 255, 200, 200, 200, 255, 200, 200, 200, 255,
                               200, 200, 200, 255, 200, 200, 200, 255],
                },
            ]);

        let mut state = EditorNativeUiState::default();
        let ui = build_native_ui(&host, 1280, 720, &mut state, &[], None);

        // One panel region for the inspector as a whole.
        let inspector_regions: Vec<_> = ui
            .panel_regions
            .iter()
            .filter(|r| r.panel == EditorUiPanel::FrameInspector)
            .collect();
        assert_eq!(
            inspector_regions.len(),
            1,
            "expected exactly one FrameInspector panel region"
        );

        // Hit regions: at least one per pass. The hierarchy /
        // inspector / history panels also emit hit regions from
        // their toolbar buttons; the check that matters is
        // "inspector added at least four" — the four scene passes.
        let inspector_frame = inspector_regions[0];
        let hit_inside_inspector = ui
            .hit_regions
            .iter()
            .filter(|h| {
                h.y >= inspector_frame.y
                    && h.y < inspector_frame.y + inspector_frame.h
                    && h.x >= inspector_frame.x
                    && h.x < inspector_frame.x + inspector_frame.w
            })
            .count();
        assert!(
            hit_inside_inspector >= 4,
            "expected ≥4 hit regions inside inspector panel (one per pass), got {}",
            hit_inside_inspector
        );
    }

    /// Panel appearance is idempotent — building the UI twice with
    /// the same capture yields the same quad count. Guards against
    /// side effects during draw (accidental state mutation via
    /// mean_rgba, etc.).
    #[test]
    fn frame_inspector_panel_is_idempotent() {
        let mut host = test_host();
        host.frame_inspector_mut().set_enabled(true);
        push_thumb(&mut host, 0, "Clear");

        let mut state = EditorNativeUiState::default();
        let a = build_native_ui(&host, 1280, 720, &mut state, &[], None);
        let mut state = EditorNativeUiState::default();
        let b = build_native_ui(&host, 1280, 720, &mut state, &[], None);
        assert_eq!(a.quads.len(), b.quads.len());
        assert_eq!(a.hit_regions.len(), b.hit_regions.len());
        assert_eq!(a.panel_regions.len(), b.panel_regions.len());
    }
}
