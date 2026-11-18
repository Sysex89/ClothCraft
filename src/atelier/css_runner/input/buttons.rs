//! Pointer presses/releases for board tools and camera capture.

use super::super::board::corners::{clamp_fillet_radius_m, connected_batch_fillet_max_radius_m};
use super::super::board::geometry::{
    board_to_panel_pt, compute_2d_view_box_m, compute_2d_viewport_rect, dist_point_to_segment,
    panel_raw_points, panel_to_board_pt, screen_to_board_m, screen_to_board_unclamped_m,
    viewport_3d_rect,
};
use super::super::board::hit_testing::{hit_test_2d, measurement_pick};
use super::super::board::selection::{batch_corners_for_points, effective_selected_points};
use super::super::board::types::{Drag2d, Hover2dTarget, ToolDraft2d};
use super::super::editing::{
    commit_authoring_command, default_fabric_id, finish_internal_line_draft, finish_pen_draft,
    round_corners_batch,
};
use super::super::state::CssRunnerState;
use clothcraft::authoring::corner_tool::{
    CIRCLE_TENSION, connected_corners, connected_edges, validate_fillet_batch,
};
use clothcraft::authoring::{self, Command};
use clothcraft::garment::meshing::MeshingSettings;
use clothcraft::garment::polygon::PatternMeasurement;
use clothcraft::garment::seam::{BoundaryRef, Side};
use clothcraft::garment::shapes::Primitive;
use cssimpler::renderer::EngineEvent;

pub(in super::super) fn handle_buttons(state: &mut CssRunnerState, event: &EngineEvent) -> bool {
    match event {
        EngineEvent::PointerButton {
            button:
                cssimpler::renderer::PointerButton::Secondary
                | cssimpler::renderer::PointerButton::Middle,
            state: btn_state,
            ..
        } => {
            if state.settings_open || state.command_palette_open || state.solver_menu_open {
                return false;
            }
            let canvas_2d = compute_2d_viewport_rect(state);
            let (px, py) = state.last_pointer_pos;
            if *btn_state == cssimpler::renderer::ButtonState::Pressed && canvas_2d.contains(px, py)
            {
                state.pointer_down = true;
                state.drag_2d = Some(Drag2d::Pan {
                    start_pan: state.board_2d_pan,
                    start_pointer: (px, py),
                });
                return true;
            }
            if *btn_state == cssimpler::renderer::ButtonState::Released
                && matches!(state.drag_2d, Some(Drag2d::Pan { .. }))
            {
                state.pointer_down = false;
                state.drag_2d = None;
                return true;
            }
            false
        }
        EngineEvent::PointerButton {
            button: cssimpler::renderer::PointerButton::Primary,
            state: btn_state,
            modifiers,
        } => {
            if state.settings_open || state.command_palette_open || state.solver_menu_open {
                return false;
            }
            if modifiers.shift || modifiers.control || modifiers.super_key {
                state.last_2d_click = None;
            }
            let multi_mod = modifiers.shift || modifiers.control || modifiers.super_key;
            let was_down = state.pointer_down;
            state.pointer_down = *btn_state == cssimpler::renderer::ButtonState::Pressed;
            let (px, py) = state.last_pointer_pos;
            let canvas_2d = compute_2d_viewport_rect(state);
            let rect_3d = viewport_3d_rect(state);

            if state.pointer_down && !was_down {
                if canvas_2d.contains(px, py) {
                    if let Some(board_m) = screen_to_board_m(state, px, py) {
                        let (_vbx, _vby, view_span, _) = compute_2d_view_box_m(state);
                        let px_m = view_span / (canvas_2d.w.max(1.0) as f64);

                        match state.active_2d_tool.as_str() {
                            "Rect" => {
                                state.draft_2d = Some(ToolDraft2d::Rect {
                                    start_board_m: board_m,
                                    current_board_m: board_m,
                                });
                                state.status_message =
                                    "Rect: drag to size new rectangle pattern".into();
                                return true;
                            }
                            "Freehand" => {
                                state.draft_2d = Some(ToolDraft2d::Freehand {
                                    samples_board_m: vec![board_m],
                                });
                                state.status_message =
                                    "Freehand: drag to sketch contour, release to create".into();
                                return true;
                            }
                            "Pen" => {
                                let should_close = matches!(
                                    &state.draft_2d,
                                    Some(ToolDraft2d::Pen { points_board_m, .. })
                                        if points_board_m.len() >= 3
                                            && points_board_m[0].distance(board_m) <= 12.0 * px_m
                                );
                                if should_close {
                                    return finish_pen_draft(state);
                                }
                                let count = match &mut state.draft_2d {
                                    Some(ToolDraft2d::Pen {
                                        points_board_m,
                                        hover_board_m,
                                    }) => {
                                        points_board_m.push(board_m);
                                        *hover_board_m = Some(board_m);
                                        points_board_m.len()
                                    }
                                    _ => {
                                        state.draft_2d = Some(ToolDraft2d::Pen {
                                            points_board_m: vec![board_m],
                                            hover_board_m: Some(board_m),
                                        });
                                        1
                                    }
                                };
                                state.status_message = format!(
                                    "Pen: {count} vertex{} — Enter or click start point to close",
                                    if count == 1 { "" } else { "es" }
                                );
                                return true;
                            }
                            "Internal Line" => {
                                let existing_panel = match &state.draft_2d {
                                    Some(ToolDraft2d::InternalLine { panel_id, .. }) => {
                                        Some(panel_id.clone())
                                    }
                                    _ => None,
                                };
                                let panel_idx = existing_panel
                                    .as_deref()
                                    .and_then(|pid| {
                                        state
                                            .project
                                            .template
                                            .panels
                                            .iter()
                                            .position(|p| p.id == pid)
                                    })
                                    .or_else(|| match hit_test_2d(state, board_m) {
                                        Some(Hover2dTarget::Panel(i))
                                        | Some(Hover2dTarget::Point { panel_idx: i, .. })
                                        | Some(Hover2dTarget::Edge { panel_idx: i, .. }) => Some(i),
                                        _ => Some(state.selected_piece_index),
                                    })
                                    .filter(|&i| i < state.project.template.panels.len());
                                if let Some(pidx) = panel_idx {
                                    let pid = state.project.template.panels[pidx].id.clone();
                                    state.selected_piece_index = pidx;
                                    let local =
                                        board_to_panel_pt(&state.project.template, pidx, board_m);
                                    let close_double = matches!(
                                        &state.draft_2d,
                                        Some(ToolDraft2d::InternalLine {
                                            panel_id,
                                            points_local_m,
                                            ..
                                        }) if panel_id == &pid
                                            && points_local_m.len() >= 2
                                            && points_local_m
                                                .last()
                                                .is_some_and(|p| p.distance(local) <= 6.0 * px_m)
                                    );
                                    if close_double {
                                        return finish_internal_line_draft(state);
                                    }
                                    let count = match &mut state.draft_2d {
                                        Some(ToolDraft2d::InternalLine {
                                            panel_id,
                                            points_local_m,
                                            hover_local_m,
                                        }) if panel_id == &pid => {
                                            points_local_m.push(local);
                                            *hover_local_m = Some(local);
                                            points_local_m.len()
                                        }
                                        _ => {
                                            state.draft_2d = Some(ToolDraft2d::InternalLine {
                                                panel_id: pid.clone(),
                                                points_local_m: vec![local],
                                                hover_local_m: Some(local),
                                            });
                                            1
                                        }
                                    };
                                    state.status_message = format!(
                                        "Internal Line on {pid}: {count} point{} (Enter to finish)",
                                        if count == 1 { "" } else { "s" }
                                    );
                                    return true;
                                }
                            }
                            "Sew" => {
                                let hit = hit_test_2d(state, board_m);
                                state.hovered_2d = hit.clone();
                                if let Some(Hover2dTarget::Edge {
                                    panel_idx,
                                    edge_idx,
                                }) = hit
                                    && let Some(p) = state.project.template.panels.get(panel_idx)
                                    && let Some(ref poly) = p.polygon
                                    && let Some(ename) = poly.edges.get(edge_idx)
                                {
                                    let pid = p.id.clone();
                                    let edge_name = ename.clone();
                                    let first = match &state.draft_2d {
                                        Some(ToolDraft2d::Sew {
                                            first_edge: Some((panel, edge)),
                                        }) => {
                                            // Consecutive contour edges traverse their shared
                                            // corner in opposite roles (end of one, start of the other).
                                            let n = poly.edges.len();
                                            let reverse = panel == &pid
                                                && (edge == &poly.edges[(edge_idx + 1) % n]
                                                    || edge == &poly.edges[(edge_idx + n - 1) % n]);
                                            Some((panel.clone(), edge.clone(), reverse))
                                        }
                                        _ => None,
                                    };
                                    state.selected_piece_index = panel_idx;
                                    state.clear_2d_subselection();
                                    state.selected_2d_edge = Some((pid.clone(), edge_name.clone()));
                                    state.accordion_open[1] = true;
                                    state.open_accordion_idx = 1;
                                    if let Some((first_pid, first_edge, reverse)) = first {
                                        if first_pid == pid && first_edge == edge_name {
                                            state.status_message =
                                                "Pick a second different boundary edge to sew"
                                                    .into();
                                            return true;
                                        }
                                        state.draft_2d = None;
                                        commit_authoring_command(
                                            state,
                                            Command::Sew {
                                                a: BoundaryRef {
                                                    reversed: false,
                                                    panel: first_pid.clone(),
                                                    side: Side::Right,
                                                    edge: Some(first_edge.clone()),
                                                    range: [0.0, 1.0],
                                                },
                                                b: BoundaryRef {
                                                    reversed: false,
                                                    panel: pid.clone(),
                                                    side: Side::Left,
                                                    edge: Some(edge_name.clone()),
                                                    range: [0.0, 1.0],
                                                },
                                                reverse,
                                                ease: true,
                                            },
                                            format!(
                                                "Sewed {first_pid}.{first_edge} ↔ {pid}.{edge_name}"
                                            ),
                                        );
                                        return true;
                                    } else {
                                        state.draft_2d = Some(ToolDraft2d::Sew {
                                            first_edge: Some((pid.clone(), edge_name.clone())),
                                        });
                                        state.status_message = format!(
                                            "Sew: selected {pid}.{edge_name} — click second edge"
                                        );
                                        return true;
                                    }
                                }
                            }
                            "Measure" => {
                                let first = match &state.draft_2d {
                                    Some(ToolDraft2d::Measure {
                                        panel_id, first, ..
                                    }) => Some((panel_id.clone(), first.clone())),
                                    _ => None,
                                };
                                let Some((idx, anchor, _)) = measurement_pick(
                                    &state.project.template,
                                    board_m,
                                    12. * px_m,
                                    state.selected_piece_index,
                                    first.as_ref().map(|(id, _)| id.as_str()),
                                ) else {
                                    state.status_message =
                                        "Pick a measurement point on the same pattern piece".into();
                                    return true;
                                };
                                let panel = &state.project.template.panels[idx];
                                let pid = panel.id.clone();
                                if let Some((_, a)) = first {
                                    let poly = panel.polygon.as_ref().unwrap();
                                    let mid = (1..)
                                        .map(|n| format!("measure_{n}"))
                                        .find(|id| poly.measurements.iter().all(|m| &m.id != id))
                                        .unwrap();
                                    let measurement = PatternMeasurement {
                                        id: mid.clone(),
                                        label: mid,
                                        a,
                                        b: anchor,
                                    };
                                    match poly.measurement_length(&measurement) {
                                        Ok(length) if length > 0. => {
                                            if commit_authoring_command(
                                                state,
                                                Command::SetMeasurement {
                                                    panel: pid,
                                                    measurement,
                                                },
                                                format!("Measured {:.1} mm", length * 1000.),
                                            ) {
                                                state.draft_2d = None;
                                            }
                                        }
                                        _ => {
                                            state.status_message =
                                                "Pick two distinct material points".into()
                                        }
                                    }
                                } else {
                                    state.draft_2d = Some(ToolDraft2d::Measure {
                                        panel_id: pid,
                                        first: anchor,
                                        hover_board_m: None,
                                    });
                                    state.status_message =
                                        "Pick the second point on this pattern piece".into();
                                }
                                return true;
                            }
                            "Trim" => {
                                let hit = hit_test_2d(state, board_m);
                                state.hovered_2d = hit.clone();
                                match hit {
                                    Some(Hover2dTarget::Point {
                                        panel_idx,
                                        point_idx,
                                    }) => {
                                        if let Some(p) =
                                            state.project.template.panels.get(panel_idx)
                                        {
                                            let pid = p.id.clone();
                                            let pname = p.name.clone();
                                            commit_authoring_command(
                                                state,
                                                Command::RemovePoint {
                                                    panel: pid,
                                                    point: point_idx,
                                                },
                                                format!("Removed corner #{point_idx} on {pname}"),
                                            );
                                            return true;
                                        }
                                    }
                                    Some(Hover2dTarget::Edge {
                                        panel_idx,
                                        edge_idx,
                                    }) => {
                                        if let Some(p) =
                                            state.project.template.panels.get(panel_idx)
                                            && let Some(ref poly) = p.polygon
                                            && let Some(ename) = poly.edges.get(edge_idx)
                                        {
                                            let pid = p.id.clone();
                                            let edge_name = ename.clone();
                                            if let Some(seam_id) = state
                                                .project
                                                .template
                                                .seams
                                                .iter()
                                                .find(|s| {
                                                    s.references().any(|r| {
                                                        r.panel == pid
                                                            && r.edge.as_deref()
                                                                == Some(edge_name.as_str())
                                                    })
                                                })
                                                .map(|s| s.id.clone())
                                            {
                                                commit_authoring_command(
                                                    state,
                                                    Command::RemoveSeam { seam: seam_id },
                                                    format!("Removed seam on {pid}.{edge_name}"),
                                                );
                                                return true;
                                            }
                                            if poly.curves.contains_key(&edge_name) {
                                                commit_authoring_command(
                                                    state,
                                                    Command::CurvePoint {
                                                        panel: pid,
                                                        anchor: edge_idx,
                                                        curved: false,
                                                    },
                                                    format!(
                                                        "Straightened curved boundary {edge_name}"
                                                    ),
                                                );
                                                return true;
                                            }
                                            state.status_message = format!(
                                                "Trim: {edge_name} has no seam or curve to trim"
                                            );
                                            return true;
                                        }
                                    }
                                    _ => {
                                        // Check internal lines near click
                                        let hit_r = 10.0 * px_m;
                                        let mut found_line: Option<(String, String)> = None;
                                        for (pidx, p) in
                                            state.project.template.panels.iter().enumerate()
                                        {
                                            if let Some(ref poly) = p.polygon {
                                                let local = board_to_panel_pt(
                                                    &state.project.template,
                                                    pidx,
                                                    board_m,
                                                );
                                                for line in &poly.lines {
                                                    if line.points.windows(2).any(|w| {
                                                        dist_point_to_segment(local, w[0], w[1])
                                                            <= hit_r
                                                    }) {
                                                        found_line =
                                                            Some((p.id.clone(), line.id.clone()));
                                                        break;
                                                    }
                                                }
                                            }
                                            if found_line.is_some() {
                                                break;
                                            }
                                        }
                                        if let Some((pid, line_id)) = found_line {
                                            commit_authoring_command(
                                                state,
                                                Command::RemoveLine {
                                                    panel: pid,
                                                    line: line_id.clone(),
                                                },
                                                format!("Removed internal line {line_id}"),
                                            );
                                            return true;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }

                        let hit = hit_test_2d(state, board_m);
                        state.hovered_2d = hit.clone();
                        let now = std::time::Instant::now();
                        let double_click =
                            state
                                .last_2d_click
                                .take()
                                .is_some_and(|(time, pos, target)| {
                                    now.duration_since(time).as_millis() <= 400
                                        && (px - pos.0).hypot(py - pos.1) <= 6.0
                                        && hit.as_ref() == Some(&target)
                                })
                                && state.active_2d_tool == "Edit"
                                && !multi_mod;
                        if state.active_2d_tool == "Edit" && !multi_mod && !double_click {
                            state.last_2d_click = hit.clone().map(|target| (now, (px, py), target));
                        }
                        if state.active_2d_tool == "Edit" && modifiers.shift {
                            let click_point = match hit {
                                Some(Hover2dTarget::Point {
                                    panel_idx,
                                    point_idx,
                                }) => Some((
                                    state.project.template.panels[panel_idx].id.clone(),
                                    point_idx,
                                )),
                                _ => None,
                            };
                            state.drag_2d = Some(Drag2d::Marquee {
                                start_board_m: board_m,
                                current_board_m: board_m,
                                start_pointer: (px, py),
                                moved: false,
                                click_point,
                            });
                            return true;
                        }
                        match hit {
                            Some(Hover2dTarget::CornerPreviewHandle {
                                panel_idx,
                                edge_name,
                            }) => {
                                let tension = if state.corner_mode_radius {
                                    CIRCLE_TENSION
                                } else {
                                    state.corner_tension
                                };
                                let sel_pts = effective_selected_points(state);
                                let hovered_info = {
                                    let tmpl = state.active_template();
                                    tmpl.panels.get(panel_idx).and_then(|p| {
                                        let poly = p.polygon.as_ref()?;
                                        let pt_idx =
                                            poly.edges.iter().position(|e| e == &edge_name)?;
                                        Some((p.id.clone(), pt_idx))
                                    })
                                };
                                if let Some((pid, pt_idx)) = hovered_info {
                                    let in_multi = sel_pts.len() > 1
                                        && sel_pts.contains(&(pid.clone(), pt_idx));
                                    let batch_corners = if in_multi {
                                        batch_corners_for_points(state.active_template(), &sel_pts)
                                    } else {
                                        vec![(pid.clone(), edge_name.clone())]
                                    };
                                    if let Some((prev, max_r)) = {
                                        let tmpl = state.active_template();
                                        connected_batch_fillet_max_radius_m(tmpl, &batch_corners)
                                            .ok()
                                            .and_then(|base_val| {
                                                let eff_r = clamp_fillet_radius_m(
                                                    state.corner_radius_mm,
                                                    base_val.max_radius_m,
                                                )?;
                                                let val = validate_fillet_batch(
                                                    tmpl,
                                                    &batch_corners,
                                                    eff_r,
                                                    tension,
                                                )
                                                .ok()?;
                                                let prev = val
                                                    .corners
                                                    .iter()
                                                    .find(|c| c.panel == pid && c.point == pt_idx)
                                                    .map(|c| c.preview.clone())
                                                    .unwrap_or(val.primary);
                                                Some((prev, val.max_radius_m))
                                            })
                                    } {
                                        let half_turn = prev.turn_radians.abs() * 0.5;
                                        let denom = (half_turn.tan() * half_turn.sin()).max(1e-4);
                                        let corner_board_m = panel_to_board_pt(
                                            &state.project.template,
                                            panel_idx,
                                            prev.corner,
                                        );
                                        let bisector = authoring::rotate_2d(
                                            prev.bisector,
                                            state.project.template.panels[panel_idx]
                                                .layout_rotation_degrees,
                                        );
                                        state.selected_piece_index = panel_idx;
                                        if !in_multi {
                                            state.set_single_2d_point(
                                                pid.clone(),
                                                prev.corner_index,
                                            );
                                        }
                                        state.accordion_open[1] = true;
                                        state.open_accordion_idx = 1;
                                        state.drag_2d = Some(Drag2d::CornerRadius {
                                            panel_id: pid,
                                            edge_name,
                                            batch_corners,
                                            corner_board_m,
                                            bisector,
                                            denom,
                                            max_radius_m: max_r,
                                            start_pointer: (px, py),
                                            moved: false,
                                        });
                                        return true;
                                    }
                                }
                            }
                            Some(Hover2dTarget::BezierHandle {
                                panel_idx,
                                edge_idx,
                                handle_index,
                            }) => {
                                if let Some(p) = state.project.template.panels.get(panel_idx)
                                    && let Some(ref poly) = p.polygon
                                    && let Some(edge_name) = poly.edges.get(edge_idx)
                                {
                                    let pid = p.id.clone();
                                    let ename = edge_name.clone();
                                    let local = board_to_panel_pt(
                                        &state.project.template,
                                        panel_idx,
                                        board_m,
                                    );
                                    let partners: Vec<String> =
                                        connected_edges(&state.project.template, &pid, &ename)
                                            .into_iter()
                                            .filter(|ce| ce.panel != pid)
                                            .map(|ce| ce.panel)
                                            .collect();
                                    state.selected_piece_index = panel_idx;
                                    state.clear_2d_subselection();
                                    state.selected_2d_edge = Some((pid.clone(), ename.clone()));
                                    state.accordion_open[1] = true;
                                    state.open_accordion_idx = 1;
                                    state.drag_2d = Some(Drag2d::BezierHandle {
                                        panel_id: pid,
                                        edge_name: ename.clone(),
                                        handle_index,
                                        start_pointer: (px, py),
                                        moved: false,
                                        last_local: local,
                                    });
                                    state.status_message = if partners.is_empty() {
                                        format!("Editing Bézier handle {handle_index} on {ename}")
                                    } else {
                                        format!(
                                            "Editing Bézier handle {handle_index} on {ename} (linked: {})",
                                            partners.join(", ")
                                        )
                                    };
                                    return true;
                                }
                            }
                            Some(Hover2dTarget::Point {
                                panel_idx,
                                point_idx,
                            }) => {
                                let point_info =
                                    state.project.template.panels.get(panel_idx).map(|p| {
                                        let pid = p.id.clone();
                                        let pname = p.name.clone();
                                        let local = board_to_panel_pt(
                                            &state.project.template,
                                            panel_idx,
                                            board_m,
                                        );
                                        let edge_name = p
                                            .polygon
                                            .as_ref()
                                            .and_then(|poly| poly.edges.get(point_idx))
                                            .cloned();
                                        let rot_deg = p.layout_rotation_degrees;
                                        let partners: Vec<String> = connected_corners(
                                            &state.project.template,
                                            &pid,
                                            point_idx,
                                        )
                                        .into_iter()
                                        .filter(|c| c.panel != pid)
                                        .map(|c| c.panel)
                                        .collect();
                                        (pid, pname, local, edge_name, rot_deg, partners)
                                    });
                                if let Some((pid, pname, local, edge_name, rot_deg, partners)) =
                                    point_info
                                {
                                    let link_suffix = if partners.is_empty() {
                                        String::new()
                                    } else {
                                        format!(" (linked: {})", partners.join(", "))
                                    };
                                    let sel_pts = effective_selected_points(state);

                                    if multi_mod {
                                        state.selected_piece_index = panel_idx;
                                        state.toggle_2d_point(pid.clone(), point_idx);
                                        state.accordion_open[1] = true;
                                        state.open_accordion_idx = 1;
                                        let new_pts = effective_selected_points(state);
                                        let count = new_pts.len();
                                        if state.active_2d_tool == "Corner"
                                            || state.active_2d_tool == "Corner Select"
                                            || !new_pts.contains(&(pid.clone(), point_idx))
                                        {
                                            state.status_message = if count == 0 {
                                                format!("Cleared corner selection on {pname}")
                                            } else {
                                                format!(
                                                    "Selected {count} corners ({pname} #{point_idx}{link_suffix})"
                                                )
                                            };
                                            return true;
                                        }
                                        state.drag_2d = Some(Drag2d::Point {
                                            panel_id: pid,
                                            point_idx,
                                            points: new_pts,
                                            start_board_m: board_m,
                                            start_pointer: (px, py),
                                            moved: false,
                                            last_local: local,
                                            last_board_m: board_m,
                                            collapse_on_click: None,
                                        });
                                        state.status_message = format!(
                                            "Selected {count} corners ({pname} #{point_idx}{link_suffix})"
                                        );
                                        return true;
                                    }

                                    let already_in_multi = sel_pts.len() > 1
                                        && sel_pts.contains(&(pid.clone(), point_idx));
                                    state.selected_piece_index = panel_idx;
                                    if !already_in_multi {
                                        state.set_single_2d_point(pid.clone(), point_idx);
                                    }
                                    state.accordion_open[1] = true;
                                    state.open_accordion_idx = 1;
                                    let active_pts = if already_in_multi {
                                        sel_pts
                                    } else {
                                        vec![(pid.clone(), point_idx)]
                                    };

                                    if state.active_2d_tool == "Corner Select" {
                                        state.set_single_2d_point(pid.clone(), point_idx);
                                        state.status_message = format!(
                                            "Selected corner #{point_idx} on {pname}{link_suffix}"
                                        );
                                        return true;
                                    } else if state.active_2d_tool == "Corner" {
                                        let batch_corners = if already_in_multi {
                                            batch_corners_for_points(
                                                &state.project.template,
                                                &active_pts,
                                            )
                                        } else if let Some(ref ename) = edge_name {
                                            vec![(pid.clone(), ename.clone())]
                                        } else {
                                            Vec::new()
                                        };
                                        if let Some(ename) = edge_name
                                            && !batch_corners.is_empty()
                                        {
                                            let tension = if state.corner_mode_radius {
                                                CIRCLE_TENSION
                                            } else {
                                                state.corner_tension
                                            };
                                            match connected_batch_fillet_max_radius_m(
                                                &state.project.template,
                                                &batch_corners,
                                            ) {
                                                Ok(base_val) => {
                                                    let Some(eff_r) = clamp_fillet_radius_m(
                                                        state.corner_radius_mm,
                                                        base_val.max_radius_m,
                                                    ) else {
                                                        state.status_message =
                                                            "Incident boundaries too short to round"
                                                                .into();
                                                        return true;
                                                    };
                                                    match validate_fillet_batch(
                                                        &state.project.template,
                                                        &batch_corners,
                                                        eff_r,
                                                        tension,
                                                    ) {
                                                        Ok(val) => {
                                                            let prev = val
                                                                .corners
                                                                .iter()
                                                                .find(|c| {
                                                                    c.panel == pid
                                                                        && c.point == point_idx
                                                                })
                                                                .map(|c| c.preview.clone())
                                                                .unwrap_or(val.primary);
                                                            let half_turn =
                                                                prev.turn_radians.abs() * 0.5;
                                                            let denom = (half_turn.tan()
                                                                * half_turn.sin())
                                                            .max(1e-4);
                                                            let corner_board_m = panel_to_board_pt(
                                                                &state.project.template,
                                                                panel_idx,
                                                                prev.corner,
                                                            );
                                                            let bisector = authoring::rotate_2d(
                                                                prev.bisector,
                                                                rot_deg,
                                                            );
                                                            state.drag_2d =
                                                                Some(Drag2d::CornerRadius {
                                                                    panel_id: pid,
                                                                    edge_name: ename,
                                                                    batch_corners,
                                                                    corner_board_m,
                                                                    bisector,
                                                                    denom,
                                                                    max_radius_m: val.max_radius_m,
                                                                    start_pointer: (px, py),
                                                                    moved: false,
                                                                });
                                                            return true;
                                                        }
                                                        Err(err) => {
                                                            state.status_message = err;
                                                            return true;
                                                        }
                                                    }
                                                }
                                                Err(err) => {
                                                    state.status_message = err;
                                                    return true;
                                                }
                                            }
                                        }
                                    } else {
                                        let collapse_on_click =
                                            already_in_multi.then(|| (pid.clone(), point_idx));
                                        let status_msg = if already_in_multi {
                                            format!(
                                                "Dragging {} selected corners{link_suffix}",
                                                active_pts.len()
                                            )
                                        } else {
                                            format!(
                                                "Selected corner #{point_idx} on {pname}{link_suffix}"
                                            )
                                        };
                                        state.drag_2d = Some(Drag2d::Point {
                                            panel_id: pid,
                                            point_idx,
                                            points: active_pts,
                                            start_board_m: board_m,
                                            start_pointer: (px, py),
                                            moved: false,
                                            last_local: local,
                                            last_board_m: board_m,
                                            collapse_on_click,
                                        });
                                        state.status_message = status_msg;
                                        return true;
                                    }
                                }
                            }
                            Some(Hover2dTarget::Edge {
                                panel_idx,
                                edge_idx,
                            }) => {
                                if let Some(p) = state.project.template.panels.get(panel_idx)
                                    && let Some(ref poly) = p.polygon
                                    && let Some(edge_name) = poly.edges.get(edge_idx)
                                {
                                    let pid = p.id.clone();
                                    let pname = p.name.clone();
                                    let ename = edge_name.clone();
                                    let sampled_opt =
                                        if state.active_2d_tool == "Split" || double_click {
                                            poly.edge_points(edge_idx).ok()
                                        } else {
                                            None
                                        };
                                    let partners: Vec<String> =
                                        connected_edges(&state.project.template, &pid, &ename)
                                            .into_iter()
                                            .filter(|ce| ce.panel != pid)
                                            .map(|ce| ce.panel)
                                            .collect();
                                    state.selected_piece_index = panel_idx;
                                    state.clear_2d_subselection();
                                    state.selected_2d_edge = Some((pid.clone(), ename.clone()));
                                    state.accordion_open[1] = true;
                                    state.open_accordion_idx = 1;
                                    if state.active_2d_tool == "Split" || double_click {
                                        if let Some(sampled) = sampled_opt {
                                            let local = board_to_panel_pt(
                                                &state.project.template,
                                                panel_idx,
                                                board_m,
                                            );
                                            let mut acc_s = 0.0_f64;
                                            let mut best_s = 0.0_f64;
                                            let mut best_d = f64::INFINITY;
                                            for pair in sampled.windows(2) {
                                                let seg = pair[1] - pair[0];
                                                let seg_l = seg.length();
                                                let t = if seg_l > 1e-12 {
                                                    ((local - pair[0]).dot(seg) / (seg_l * seg_l))
                                                        .clamp(0.0, 1.0)
                                                } else {
                                                    0.0
                                                };
                                                let proj = pair[0] + seg * t;
                                                let d = local.distance(proj);
                                                if d < best_d {
                                                    best_d = d;
                                                    best_s = acc_s + seg_l * t;
                                                }
                                                acc_s += seg_l;
                                            }
                                            if acc_s > 0.002 {
                                                let at_m = best_s.clamp(0.001, acc_s - 0.001);
                                                commit_authoring_command(
                                                    state,
                                                    Command::SplitBoundary {
                                                        panel: pid,
                                                        edge: ename.clone(),
                                                        at_m,
                                                    },
                                                    format!("Split boundary {ename}"),
                                                );
                                            }
                                        }
                                        return true;
                                    }
                                    state.status_message = if partners.is_empty() {
                                        format!("Selected edge {ename} on {pname}")
                                    } else {
                                        format!(
                                            "Selected edge {ename} on {pname} (linked: {})",
                                            partners.join(", ")
                                        )
                                    };
                                    return true;
                                }
                            }
                            Some(Hover2dTarget::Panel(panel_idx)) => {
                                state.selected_piece_index = panel_idx;
                                if !multi_mod {
                                    state.clear_2d_subselection();
                                    state.piece_selected_explicitly = true;
                                }
                                state.drag_2d = Some(Drag2d::Pan {
                                    start_pan: state.board_2d_pan,
                                    start_pointer: (px, py),
                                });
                                if let Some(p) = state.project.template.panels.get(panel_idx) {
                                    state.status_message =
                                        format!("Selected piece: {} ({})", p.name, p.id);
                                }
                                return true;
                            }
                            None => {
                                if !multi_mod {
                                    state.clear_2d_subselection();
                                }
                                state.drag_2d = Some(Drag2d::Pan {
                                    start_pan: state.board_2d_pan,
                                    start_pointer: (px, py),
                                });
                                return true;
                            }
                        }
                    }
                } else if rect_3d.contains(px, py) {
                    state.dragging_3d = true;
                }
            } else if !state.pointer_down {
                state.dragging_3d = false;
                if matches!(state.draft_2d, Some(ToolDraft2d::Rect { .. }))
                    && let Some(ToolDraft2d::Rect {
                        start_board_m,
                        current_board_m,
                    }) = state.draft_2d.take()
                {
                    let final_b =
                        screen_to_board_unclamped_m(state, px, py).unwrap_or(current_board_m);
                    let lo = start_board_m.min(final_b);
                    let hi = start_board_m.max(final_b);
                    let size = hi - lo;
                    if size.x >= 0.03 && size.y >= 0.03 {
                        let fabric = default_fabric_id(state);
                        commit_authoring_command(
                            state,
                            Command::Primitive {
                                kind: Primitive::Rectangle,
                                size,
                                sweep_degrees: 360.0,
                                origin: lo,
                                meshing: MeshingSettings::default(),
                                fabric,
                                replace: false,
                            },
                            format!(
                                "Created rectangle {:.0}×{:.0} mm",
                                size.x * 1000.0,
                                size.y * 1000.0
                            ),
                        );
                    } else {
                        state.status_message =
                            "Drag at least 30×30 mm to create a rectangle".into();
                    }
                    return true;
                }
                if matches!(state.draft_2d, Some(ToolDraft2d::Freehand { .. }))
                    && let Some(ToolDraft2d::Freehand { samples_board_m }) = state.draft_2d.take()
                {
                    if samples_board_m.len() >= 3 {
                        let fabric = default_fabric_id(state);
                        commit_authoring_command(
                            state,
                            Command::Freehand {
                                samples: samples_board_m,
                                tolerance_m: 0.008,
                                meshing: MeshingSettings::default(),
                                fabric,
                            },
                            "Created freehand pattern piece",
                        );
                    } else {
                        state.status_message =
                            "Draw a closed freehand loop to create a pattern piece".into();
                    }
                    return true;
                }
                if let Some(drag) = state.drag_2d.take() {
                    match drag {
                        Drag2d::Marquee {
                            start_board_m,
                            current_board_m,
                            start_pointer,
                            moved,
                            click_point,
                        } => {
                            let moved =
                                moved || (px - start_pointer.0).hypot(py - start_pointer.1) >= 3.0;
                            if moved {
                                let end = screen_to_board_unclamped_m(state, px, py)
                                    .unwrap_or(current_board_m);
                                let lo = start_board_m.min(end);
                                let hi = start_board_m.max(end);
                                let mut points = effective_selected_points(state);
                                for (idx, panel) in state.project.template.panels.iter().enumerate()
                                {
                                    for (point_idx, local) in
                                        panel_raw_points(panel).into_iter().enumerate()
                                    {
                                        let point =
                                            panel_to_board_pt(&state.project.template, idx, local);
                                        if point.cmpge(lo).all() && point.cmple(hi).all() {
                                            let key = (panel.id.clone(), point_idx);
                                            if !points.contains(&key) {
                                                points.push(key);
                                            }
                                        }
                                    }
                                }
                                state.selected_2d_points = points;
                                state.sync_selected_point();
                                state.selected_2d_edge = None;
                                state.corner_preview_active = false;
                                state.status_message =
                                    format!("Selected {} points", state.selected_2d_points.len());
                            } else if let Some((panel, point)) = click_point {
                                state.toggle_2d_point(panel, point);
                            }
                            return true;
                        }
                        Drag2d::Point {
                            panel_id,
                            point_idx,
                            points,
                            start_board_m,
                            moved,
                            last_local,
                            last_board_m,
                            collapse_on_click,
                            ..
                        } => {
                            state.preview_template = None;
                            if !moved {
                                if let Some((cpid, cidx)) = collapse_on_click {
                                    state.set_single_2d_point(cpid, cidx);
                                }
                            } else if points.len() > 1 {
                                let board_delta = screen_to_board_unclamped_m(state, px, py)
                                    .unwrap_or(last_board_m)
                                    - start_board_m;
                                let count = points.len();
                                commit_authoring_command(
                                    state,
                                    Command::MovePointGroupsConnected {
                                        points,
                                        board_delta,
                                    },
                                    format!(
                                        "Moved {count} corners ({:.1}, {:.1}) mm",
                                        board_delta.x * 1000.0,
                                        board_delta.y * 1000.0
                                    ),
                                );
                            } else {
                                let local = screen_to_board_unclamped_m(state, px, py)
                                    .and_then(|board_m| {
                                        state
                                            .project
                                            .template
                                            .panels
                                            .iter()
                                            .position(|p| p.id == panel_id)
                                            .map(|panel_idx| {
                                                board_to_panel_pt(
                                                    &state.project.template,
                                                    panel_idx,
                                                    board_m,
                                                )
                                            })
                                    })
                                    .unwrap_or(last_local);
                                commit_authoring_command(
                                    state,
                                    Command::Anchor {
                                        panel: panel_id,
                                        anchor: point_idx,
                                        position: local,
                                    },
                                    format!(
                                        "Moved point #{point_idx} to ({:.1}, {:.1}) mm",
                                        local.x * 1000.0,
                                        local.y * 1000.0
                                    ),
                                );
                            }
                            return true;
                        }
                        Drag2d::BezierHandle {
                            panel_id,
                            edge_name,
                            handle_index,
                            moved,
                            last_local,
                            ..
                        } => {
                            state.preview_template = None;
                            if moved {
                                commit_authoring_command(
                                    state,
                                    Command::AdjustCornerHandle {
                                        panel: panel_id,
                                        edge: edge_name.clone(),
                                        handle_index,
                                        position: last_local,
                                        rule: state.corner_rule,
                                    },
                                    format!("Adjusted Bézier handle on {edge_name}"),
                                );
                            }
                            return true;
                        }
                        Drag2d::CornerRadius {
                            panel_id,
                            edge_name,
                            batch_corners,
                            max_radius_m,
                            ..
                        } => {
                            state.preview_template = None;
                            let fallback = [(panel_id, edge_name)];
                            let corners = if batch_corners.is_empty() {
                                &fallback[..]
                            } else {
                                batch_corners.as_slice()
                            };
                            round_corners_batch(state, corners, max_radius_m);
                            return true;
                        }
                        Drag2d::Pan { .. } => {
                            return true;
                        }
                    }
                }
                if state.draft_2d.is_some()
                    || (was_down && (canvas_2d.contains(px, py) || rect_3d.contains(px, py)))
                {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}
