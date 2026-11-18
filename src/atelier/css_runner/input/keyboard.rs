//! Keyboard shortcuts and cancellation.

use super::super::board::interaction::set_active_2d_tool;
use super::super::board::selection::effective_selected_points;
use super::super::board::types::{Drag2d, ToolDraft2d};
use super::super::editing::{
    commit_authoring_command, finish_internal_line_draft, finish_pen_draft, round_selected_corner,
    undo_redo_2d,
};
use super::super::state::CssRunnerState;
use clothcraft::authoring::Command;
use cssimpler::renderer::EngineEvent;

pub(in super::super) fn handle_keyboard(state: &mut CssRunnerState, event: &EngineEvent) -> bool {
    match event {
        EngineEvent::Key(k) => {
            if k.state == cssimpler::renderer::ButtonState::Pressed {
                if state.settings_open {
                    if matches!(&k.logical_key, cssimpler::renderer::KeyIdentity::Named(name) if name == "Escape")
                    {
                        state.settings_open = false;
                        return true;
                    }
                    return false;
                }
                let ctrl = k.modifiers.control || k.modifiers.super_key;
                if let cssimpler::renderer::KeyIdentity::Character(ref ch) = k.logical_key {
                    if ch == " " {
                        state.playing = !state.playing;
                        state.send_control_state(false);
                        return true;
                    }
                    if ch == "k" && ctrl {
                        state.command_palette_open = !state.command_palette_open;
                        return true;
                    }
                    if ch == "s" && ctrl {
                        state.save_project();
                        return true;
                    }
                    if (ch == "z" || ch == "Z") && ctrl {
                        if k.modifiers.shift {
                            undo_redo_2d(state, true);
                        } else {
                            undo_redo_2d(state, false);
                        }
                        return true;
                    }
                    if (ch == "y" || ch == "Y") && ctrl {
                        undo_redo_2d(state, true);
                        return true;
                    }
                    if !ctrl {
                        if ch == "z" || ch == "Z" || ch == "a" || ch == "A" {
                            set_active_2d_tool(state, "Edit");
                            return true;
                        }
                        if ch == "s" || ch == "S" {
                            set_active_2d_tool(state, "Rect");
                            return true;
                        }
                        if ch == "h" || ch == "H" {
                            set_active_2d_tool(state, "Pen");
                            return true;
                        }
                        if ch == "f" || ch == "F" {
                            set_active_2d_tool(state, "Freehand");
                            return true;
                        }
                        if ch == "b" || ch == "B" {
                            set_active_2d_tool(state, "Sew");
                            return true;
                        }
                        if ch == "x" || ch == "X" {
                            set_active_2d_tool(state, "Split");
                            return true;
                        }
                        if ch == "g" || ch == "G" {
                            set_active_2d_tool(state, "Internal Line");
                            return true;
                        }
                        if ch == "m" || ch == "M" {
                            set_active_2d_tool(state, "Measure");
                            return true;
                        }
                        if ch == "t" || ch == "T" {
                            set_active_2d_tool(state, "Trim");
                            return true;
                        }
                        if ch == "v" || ch == "V" {
                            set_active_2d_tool(state, "Corner Select");
                            return true;
                        }
                        if ch == "c" || ch == "C" {
                            set_active_2d_tool(state, "Corner");
                            return true;
                        }
                    }
                    if ch == "]" {
                        let count = state.project.template.panels.len().max(1);
                        state.selected_piece_index = (state.selected_piece_index + 1) % count;
                        state.clear_2d_subselection();
                        state.piece_selected_explicitly = true;
                        if let Some(p) = state
                            .project
                            .template
                            .panels
                            .get(state.selected_piece_index)
                        {
                            state.status_message = format!("Selected piece: {} ({})", p.name, p.id);
                        }
                        return true;
                    }
                    if ch == "[" {
                        let count = state.project.template.panels.len().max(1);
                        state.selected_piece_index =
                            (state.selected_piece_index + count - 1) % count;
                        state.clear_2d_subselection();
                        state.piece_selected_explicitly = true;
                        if let Some(p) = state
                            .project
                            .template
                            .panels
                            .get(state.selected_piece_index)
                        {
                            state.status_message = format!("Selected piece: {} ({})", p.name, p.id);
                        }
                        return true;
                    }
                    if ch == "+" || ch == "=" {
                        state.board_2d_zoom = (state.board_2d_zoom * 1.25).clamp(0.25, 10.0);
                        return true;
                    }
                    if ch == "-" || ch == "_" {
                        state.board_2d_zoom = (state.board_2d_zoom / 1.25).clamp(0.25, 10.0);
                        return true;
                    }
                    if ch == "0" {
                        state.frame_all_2d();
                        return true;
                    }
                }
                if let cssimpler::renderer::KeyIdentity::Named(ref name) = k.logical_key {
                    if name == "Escape" {
                        state.last_2d_click = None;
                        if matches!(state.drag_2d, Some(Drag2d::Marquee { .. })) {
                            state.drag_2d = None;
                            state.pointer_down = false;
                            state.status_message = "Cancelled rectangle selection".into();
                            return true;
                        }
                        if state.command_palette_open || state.solver_menu_open {
                            state.command_palette_open = false;
                            state.solver_menu_open = false;
                            return true;
                        }
                        if state.draft_2d.is_some() {
                            state.draft_2d = None;
                            state.status_message = "Cancelled 2D tool draft".into();
                            return true;
                        }
                        if state.drag_2d.is_some()
                            || !effective_selected_points(state).is_empty()
                            || state.selected_2d_edge.is_some()
                            || state.corner_preview_active
                        {
                            state.drag_2d = None;
                            state.preview_template = None;
                            state.clear_2d_subselection();
                            return true;
                        }
                        if state.piece_selected_explicitly {
                            state.piece_selected_explicitly = false;
                            return true;
                        }
                    }
                    if name == "Delete" || name == "Backspace" {
                        let sel_pts = effective_selected_points(state);
                        if sel_pts.len() > 1 {
                            let count = sel_pts.len();
                            if commit_authoring_command(
                                state,
                                Command::RemovePoints { points: sel_pts },
                                format!("Removed {count} selected corners"),
                            ) {
                                state.clear_2d_subselection();
                            }
                            return true;
                        } else if let Some((panel, point)) = sel_pts.into_iter().next() {
                            if commit_authoring_command(
                                state,
                                Command::RemovePoint { panel, point },
                                format!("Removed point #{point}"),
                            ) {
                                state.clear_2d_subselection();
                            }
                            return true;
                        } else if state.piece_selected_explicitly
                            && state.draft_2d.is_none()
                            && state.drag_2d.is_none()
                            && let Some(p) = state
                                .project
                                .template
                                .panels
                                .get(state.selected_piece_index)
                        {
                            let (panel, name) = (p.id.clone(), p.name.clone());
                            commit_authoring_command(
                                state,
                                Command::RemovePanel { panel },
                                format!("Deleted piece {name}"),
                            );
                            return true;
                        }
                    }
                    if name == "Enter" {
                        if matches!(state.draft_2d, Some(ToolDraft2d::Pen { .. })) {
                            return finish_pen_draft(state);
                        }
                        if matches!(state.draft_2d, Some(ToolDraft2d::InternalLine { .. })) {
                            return finish_internal_line_draft(state);
                        }
                        if !effective_selected_points(state).is_empty()
                            || state.selected_2d_edge.is_some()
                        {
                            round_selected_corner(state);
                            return true;
                        }
                    }
                }
            }
            false
        }
        _ => false,
    }
}
