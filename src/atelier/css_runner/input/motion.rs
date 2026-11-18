//! Pointer motion for hover, drafts, dragging and camera orbit.

use super::super::board::geometry::{
    board_to_panel_pt, compute_2d_view_box_m, compute_2d_viewport_rect, screen_to_board_m,
    screen_to_board_unclamped_m,
};
use super::super::board::hit_testing::hit_test_2d;
use super::super::board::types::{Drag2d, ToolDraft2d};
use super::super::editing::{
    update_preview_bezier_handle, update_preview_point, update_preview_point_groups,
};
use super::super::state::CssRunnerState;
use cssimpler::renderer::EngineEvent;

pub(in super::super) fn handle_motion(state: &mut CssRunnerState, event: &EngineEvent) -> bool {
    match event {
        EngineEvent::PointerMoved { position, .. } => {
            let (px, py) = (position.x, position.y);
            let dx = px - state.last_pointer_pos.0;
            let dy = py - state.last_pointer_pos.1;
            state.last_pointer_pos = (px, py);
            if state.settings_open || state.import_dialog.is_some() {
                return false;
            }
            if state
                .last_2d_click
                .as_ref()
                .is_some_and(|(_, pos, _)| (px - pos.0).hypot(py - pos.1) > 6.0)
            {
                state.last_2d_click = None;
            }

            if state.draft_2d.is_some() {
                let pointer_down = state.pointer_down;
                let unclamped_b = screen_to_board_unclamped_m(state, px, py);
                let clamped_b = screen_to_board_m(state, px, py);
                let internal_next_local =
                    if let Some(ToolDraft2d::InternalLine { panel_id, .. }) =
                        state.draft_2d.as_ref()
                    {
                        clamped_b.and_then(|b| {
                            state
                                .project
                                .template
                                .panels
                                .iter()
                                .position(|p| &p.id == panel_id)
                                .map(|pidx| board_to_panel_pt(&state.project.template, pidx, b))
                        })
                    } else {
                        None
                    };
                let measurement_hover =
                    if let Some(ToolDraft2d::Measure { panel_id, .. }) = &state.draft_2d {
                        let (_, _, span, _) = compute_2d_view_box_m(state);
                        let tolerance =
                            12. * span / f64::from(compute_2d_viewport_rect(state).w.max(1.));
                        clamped_b
                            .and_then(|board| {
                                super::super::board::hit_testing::measurement_pick(
                                    &state.project.template,
                                    board,
                                    tolerance,
                                    state.selected_piece_index,
                                    Some(panel_id),
                                )
                            })
                            .map(|(_, _, board)| board)
                    } else {
                        None
                    };
                match state.draft_2d.as_mut() {
                    Some(ToolDraft2d::Rect {
                        current_board_m, ..
                    }) if pointer_down => {
                        if let Some(b) = unclamped_b {
                            *current_board_m = b;
                            return true;
                        }
                    }
                    Some(ToolDraft2d::Freehand { samples_board_m }) if pointer_down => {
                        if let Some(b) = unclamped_b
                            && samples_board_m
                                .last()
                                .is_none_or(|last| last.distance(b) >= 0.005)
                        {
                            samples_board_m.push(b);
                            return true;
                        }
                    }
                    Some(ToolDraft2d::Pen { hover_board_m, .. }) => {
                        *hover_board_m = clamped_b;
                        return true;
                    }
                    Some(ToolDraft2d::InternalLine { hover_local_m, .. }) => {
                        *hover_local_m = internal_next_local;
                        return true;
                    }
                    Some(ToolDraft2d::Measure { hover_board_m, .. }) => {
                        *hover_board_m = measurement_hover;
                        return true;
                    }
                    _ => {}
                }
            }

            if state.drag_2d.is_some() {
                let svg_rect = compute_2d_viewport_rect(state);
                let (_vbx, _vby, view_span, _) = compute_2d_view_box_m(state);
                let px_m = view_span / (svg_rect.w.max(1.0) as f64);

                let drag = state.drag_2d.take().unwrap();
                match drag {
                    Drag2d::Marquee {
                        start_board_m,
                        current_board_m,
                        start_pointer,
                        moved,
                        click_point,
                    } => {
                        let current_board_m =
                            screen_to_board_unclamped_m(state, px, py).unwrap_or(current_board_m);
                        let moved =
                            moved || (px - start_pointer.0).hypot(py - start_pointer.1) >= 3.0;
                        state.drag_2d = Some(Drag2d::Marquee {
                            start_board_m,
                            current_board_m,
                            start_pointer,
                            moved,
                            click_point,
                        });
                        return true;
                    }
                    Drag2d::Pan {
                        start_pan,
                        start_pointer,
                    } => {
                        let dpx = (px - start_pointer.0) as f64;
                        let dpy = (py - start_pointer.1) as f64;
                        state.board_2d_pan.0 = start_pan.0 - dpx * px_m;
                        state.board_2d_pan.1 = start_pan.1 - dpy * px_m;
                        state.drag_2d = Some(Drag2d::Pan {
                            start_pan,
                            start_pointer,
                        });
                        return true;
                    }
                    Drag2d::Point {
                        panel_id,
                        point_idx,
                        points,
                        start_board_m,
                        start_pointer,
                        mut moved,
                        mut last_local,
                        mut last_board_m,
                        collapse_on_click,
                    } => {
                        if (px - start_pointer.0).hypot(py - start_pointer.1) >= 2.5 {
                            moved = true;
                        }
                        if let Some(board_m) = screen_to_board_unclamped_m(state, px, py) {
                            last_board_m = board_m;
                            if let Some(panel_idx) = state
                                .project
                                .template
                                .panels
                                .iter()
                                .position(|p| p.id == panel_id)
                            {
                                last_local =
                                    board_to_panel_pt(&state.project.template, panel_idx, board_m);
                                if moved {
                                    if points.len() > 1 {
                                        update_preview_point_groups(
                                            state,
                                            &points,
                                            board_m - start_board_m,
                                        );
                                    } else {
                                        update_preview_point(
                                            state, &panel_id, point_idx, last_local,
                                        );
                                    }
                                }
                            }
                        }
                        state.drag_2d = Some(Drag2d::Point {
                            panel_id,
                            point_idx,
                            points,
                            start_board_m,
                            start_pointer,
                            moved,
                            last_local,
                            last_board_m,
                            collapse_on_click,
                        });
                        return true;
                    }
                    Drag2d::BezierHandle {
                        panel_id,
                        edge_name,
                        handle_index,
                        start_pointer,
                        mut moved,
                        mut last_local,
                    } => {
                        if (px - start_pointer.0).hypot(py - start_pointer.1) >= 2.0 {
                            moved = true;
                        }
                        if let Some(board_m) = screen_to_board_unclamped_m(state, px, py)
                            && let Some(panel_idx) = state
                                .project
                                .template
                                .panels
                                .iter()
                                .position(|p| p.id == panel_id)
                        {
                            last_local =
                                board_to_panel_pt(&state.project.template, panel_idx, board_m);
                            if moved {
                                update_preview_bezier_handle(
                                    state,
                                    &panel_id,
                                    &edge_name,
                                    handle_index,
                                    last_local,
                                );
                            }
                        }
                        state.drag_2d = Some(Drag2d::BezierHandle {
                            panel_id,
                            edge_name,
                            handle_index,
                            start_pointer,
                            moved,
                            last_local,
                        });
                        return true;
                    }
                    Drag2d::CornerRadius {
                        panel_id,
                        edge_name,
                        batch_corners,
                        corner_board_m,
                        bisector,
                        denom,
                        max_radius_m,
                        start_pointer,
                        mut moved,
                    } => {
                        if (px - start_pointer.0).hypot(py - start_pointer.1) >= 2.0 {
                            moved = true;
                        }
                        if moved && let Some(board_m) = screen_to_board_unclamped_m(state, px, py) {
                            let proj = (board_m - corner_board_m).dot(bisector).max(0.0005);
                            let max_clamp = max_radius_m * 0.98;
                            let min_r = 0.001_f64.min(max_clamp);
                            let radius_m = (proj / denom).clamp(min_r, max_clamp.max(min_r));
                            state.corner_radius_mm = radius_m * 1000.0;
                            state.status_message =
                                format!("Corner preview radius: {:.1} mm", state.corner_radius_mm);
                        }
                        state.drag_2d = Some(Drag2d::CornerRadius {
                            panel_id,
                            edge_name,
                            batch_corners,
                            corner_board_m,
                            bisector,
                            denom,
                            max_radius_m,
                            start_pointer,
                            moved,
                        });
                        return true;
                    }
                }
            }

            if state.pointer_down && state.dragging_3d {
                state.camera.yaw -= dx * 0.008;
                state.camera.pitch = (state.camera.pitch + dy * 0.008).clamp(-1.45, 1.45);
                state.sync_usd_camera();
                return true;
            }

            // Passive hover feedback in 2D viewport
            if !state.pointer_down && !state.command_palette_open && !state.solver_menu_open {
                let new_hover = screen_to_board_m(state, px, py)
                    .and_then(|board_m| hit_test_2d(state, board_m));
                if new_hover != state.hovered_2d {
                    state.hovered_2d = new_hover;
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}
