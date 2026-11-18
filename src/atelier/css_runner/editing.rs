//! Transactional authoring commands, history and geometry previews.

use super::board::corners::{clamp_fillet_radius_m, connected_batch_fillet_max_radius_m};
use super::board::geometry::fit_2d_board;
use super::board::selection::{
    batch_corners_for_points, effective_selected_points, sanitize_2d_subselection,
};
use super::board::types::ToolDraft2d;
use super::state::CssRunnerState;
use clothcraft::authoring::corner_tool::{
    CIRCLE_TENSION, preview_adjust_handle, preview_move_corner,
    preview_move_point_groups_connected, validate_fillet_batch,
};
use clothcraft::authoring::{self, Command};
use clothcraft::fitting::FittingScene;
use clothcraft::garment::GarmentTemplate;
use clothcraft::garment::meshing::MeshingSettings;
use clothcraft::garment::seam::{BoundaryRef, Side};
use glam::DVec2;
use std::sync::Arc;

pub(super) fn default_fabric_id(state: &CssRunnerState) -> String {
    state
        .project
        .template
        .panels
        .get(state.selected_piece_index)
        .map(|p| p.fabric.clone())
        .or_else(|| state.project.template.fabrics.first().map(|f| f.id.clone()))
        .unwrap_or_else(|| "cotton".to_string())
}

pub(super) fn install_authoring_edit(
    state: &mut CssRunnerState,
    edit: authoring::Edit,
    record_old: Option<GarmentTemplate>,
    msg: String,
) {
    if let Some(old) = record_old {
        state.history.record(old);
    }
    state.piece_selected_explicitly = false;
    let next_garment = Arc::new(edit.garment.clone());
    let next_state = if edit.reset_simulation {
        edit.state_after(&state.scene.garment, &state.scene.state)
    } else {
        state.scene.state.clone()
    };

    if let Some(ref sel_id) = edit.selected
        && let Some(idx) = edit.template.panels.iter().position(|p| &p.id == sel_id)
    {
        state.selected_piece_index = idx;
    }
    if let Some(np) = edit.new_point
        && let Some(p) = edit.template.panels.get(state.selected_piece_index)
    {
        state.set_single_2d_point(p.id.clone(), np);
    }

    // Keep the existing framing behavior when pieces enter or leave the
    // board. Editing points on existing pieces keeps the current camera.
    let previous_panels = &state.project.template.panels;
    if previous_panels.len() != edit.template.panels.len()
        || previous_panels
            .iter()
            .zip(&edit.template.panels)
            .any(|(old, new)| old.id != new.id)
    {
        state.board_2d_fit = fit_2d_board(&edit.template);
    }
    state.project.template = edit.template;
    if edit.reset_simulation {
        state.project.state = None;
        state.project.mesh_signature = None;
        state.generation = state.generation.wrapping_add(1);
        state.worker_pending = false;
        state.scene = FittingScene {
            garment: Arc::clone(&next_garment),
            collider: Arc::clone(&state.scene.collider),
            state: next_state.clone(),
            quasistatic: state.scene.quasistatic,
            next_dt: state.scene.next_dt,
        };
        state.initial_scene = FittingScene {
            garment: next_garment,
            collider: Arc::clone(&state.scene.collider),
            state: state.scene.garment.initial_state.clone(),
            quasistatic: state.scene.quasistatic,
            next_dt: state.scene.next_dt,
        };
        if let Some(ref usd) = state.usd_scene {
            let _ = clothcraft::usd_scene::update_panel_viewport_mesh(
                usd,
                &state.scene.garment,
                &state.scene.state,
            );
        }
        state.send_control_state(false);
    }

    sanitize_2d_subselection(state);
    state.status_message = msg;
}

pub fn commit_authoring_command(
    state: &mut CssRunnerState,
    cmd: Command,
    msg: impl Into<String>,
) -> bool {
    state.preview_template = None;
    let before = state.project.template.clone();
    match authoring::apply(&before, cmd) {
        Ok(edit) => {
            install_authoring_edit(state, edit, Some(before), msg.into());
            true
        }
        Err(err) => {
            state.status_message = format!("Edit rejected: {err}");
            false
        }
    }
}

pub fn undo_redo_2d(state: &mut CssRunnerState, redo: bool) -> bool {
    state.preview_template = None;
    match state.history.restore(&state.project.template, redo) {
        Ok(edit) => {
            let label = if redo {
                "Redid 2D edit"
            } else {
                "Undid 2D edit"
            };
            install_authoring_edit(state, edit, None, label.into());
            true
        }
        Err(err) => {
            state.status_message = err;
            false
        }
    }
}

pub(super) fn round_corners_batch(
    state: &mut CssRunnerState,
    corners: &[(String, String)],
    max_radius_m: f64,
) -> bool {
    let Some((panel_id, edge_name)) = corners.first() else {
        return false;
    };
    let Some(eff_r) = clamp_fillet_radius_m(state.corner_radius_mm, max_radius_m) else {
        state.status_message = "Incident boundaries too short to round".into();
        return false;
    };
    let tension = if state.corner_mode_radius {
        CIRCLE_TENSION
    } else {
        state.corner_tension
    };
    let total_corners = validate_fillet_batch(&state.project.template, corners, eff_r, tension)
        .map_or(corners.len(), |v| v.corners.len());

    if corners.len() == 1 {
        let linked_extra = total_corners.saturating_sub(1);
        let msg = if linked_extra > 0 {
            format!(
                "Rounded corner on {edge_name} (+{linked_extra} linked) (R={:.1} mm, T={tension:.2})",
                eff_r * 1000.0
            )
        } else {
            format!(
                "Rounded corner on {edge_name} (R={:.1} mm, T={tension:.2})",
                eff_r * 1000.0
            )
        };
        let ok = commit_authoring_command(
            state,
            Command::RoundCornerShape {
                panel: panel_id.clone(),
                edge: edge_name.clone(),
                radius_m: eff_r,
                tension,
                rule: state.corner_rule,
            },
            msg,
        );
        if ok {
            state.clear_2d_subselection();
            // Select the newly created fillet curve edge for immediate Bézier inspection
            if let Some(p) = state
                .project
                .template
                .panels
                .iter()
                .find(|p| &p.id == panel_id)
                && let Some(ref poly) = p.polygon
                && let Some(idx) = poly.edges.iter().position(|e| e == edge_name)
            {
                let fillet_idx = (idx + poly.edges.len() - 1) % poly.edges.len();
                state.selected_2d_edge = Some((panel_id.clone(), poly.edges[fillet_idx].clone()));
            }
        }
        return ok;
    }

    let ok = commit_authoring_command(
        state,
        Command::RoundCornersShape {
            corners: corners.to_vec(),
            radius_m: eff_r,
            tension,
            rule: state.corner_rule,
        },
        format!(
            "Rounded {} selected corners ({total_corners} total w/ linked) (R={:.1} mm, T={tension:.2})",
            corners.len(),
            eff_r * 1000.0
        ),
    );
    if ok {
        state.clear_2d_subselection();
    }
    ok
}

pub(super) fn round_corner_by_name(
    state: &mut CssRunnerState,
    panel_id: &str,
    edge_name: &str,
    max_radius_m: f64,
) -> bool {
    round_corners_batch(
        state,
        &[(panel_id.to_owned(), edge_name.to_owned())],
        max_radius_m,
    )
}

pub(super) fn round_selected_corner(state: &mut CssRunnerState) -> bool {
    let sel_pts = effective_selected_points(state);
    let corners = if !sel_pts.is_empty() {
        batch_corners_for_points(&state.project.template, &sel_pts)
    } else if let Some((ref pid, ref ename)) = state.selected_2d_edge {
        vec![(pid.clone(), ename.clone())]
    } else {
        Vec::new()
    };

    if corners.is_empty() {
        state.status_message = "Select one or more corner points first to round".into();
        return false;
    }
    match connected_batch_fillet_max_radius_m(&state.project.template, &corners) {
        Ok(val) if val.max_radius_m > 1e-4 => {
            round_corners_batch(state, &corners, val.max_radius_m)
        }
        Ok(_) => {
            state.status_message = "Incident boundaries too short to round".into();
            false
        }
        Err(err) => {
            state.status_message = format!("Cannot round corner: {err}");
            false
        }
    }
}

pub(super) fn toggle_selected_curve(state: &mut CssRunnerState, curved: bool) -> bool {
    let sel_pts = effective_selected_points(state);
    let points = if !sel_pts.is_empty() {
        sel_pts
    } else if let Some((ref pid, ref ename)) = state.selected_2d_edge {
        state
            .project
            .template
            .panels
            .iter()
            .find(|p| &p.id == pid)
            .and_then(|p| p.polygon.as_ref())
            .and_then(|poly| poly.edges.iter().position(|e| e == ename))
            .map_or_else(Vec::new, |idx| vec![(pid.clone(), idx)])
    } else {
        Vec::new()
    };
    if points.is_empty() {
        state.status_message = "Select a point or boundary to toggle Bézier curves".into();
        return false;
    }
    let count = points.len();
    let msg = if count == 1 {
        if curved {
            "Converted incident boundaries to cubic Bézier".to_string()
        } else {
            "Straightened incident boundaries".to_string()
        }
    } else if curved {
        format!("Converted incident boundaries on {count} selected corners to cubic Bézier")
    } else {
        format!("Straightened incident boundaries on {count} selected corners")
    };
    commit_authoring_command(state, Command::CurvePoints { points, curved }, msg)
}

pub(super) fn update_preview_point(
    state: &mut CssRunnerState,
    panel_id: &str,
    point_idx: usize,
    local_pos: DVec2,
) {
    let mut draft = state.project.template.clone();
    if preview_move_corner(
        &state.project.template,
        &mut draft,
        panel_id,
        point_idx,
        local_pos,
    )
    .is_ok()
    {
        state.preview_template = Some(draft);
    }
}

pub(super) fn update_preview_point_groups(
    state: &mut CssRunnerState,
    points: &[(String, usize)],
    board_delta: DVec2,
) {
    let mut draft = state.project.template.clone();
    if preview_move_point_groups_connected(&state.project.template, &mut draft, points, board_delta)
        .is_ok()
    {
        state.preview_template = Some(draft);
    }
}

pub(super) fn update_preview_bezier_handle(
    state: &mut CssRunnerState,
    panel_id: &str,
    edge_name: &str,
    handle_index: usize,
    position: DVec2,
) {
    let mut draft = state.project.template.clone();
    if preview_adjust_handle(
        &state.project.template,
        &mut draft,
        panel_id,
        edge_name,
        handle_index,
        position,
        state.corner_rule,
    )
    .is_ok()
    {
        state.preview_template = Some(draft);
    }
}

pub(super) fn finish_pen_draft(state: &mut CssRunnerState) -> bool {
    let Some(ToolDraft2d::Pen { points_board_m, .. }) = state.draft_2d.take() else {
        return false;
    };
    if points_board_m.len() < 3 {
        state.status_message = "Pen tool requires at least 3 vertices".into();
        return true;
    }
    let count = points_board_m.len();
    let fabric = default_fabric_id(state);
    commit_authoring_command(
        state,
        Command::Polygon {
            points: points_board_m,
            tangents: vec![],
            meshing: MeshingSettings::default(),
            fabric,
            replace: false,
        },
        format!("Created {count}-point polygon pattern"),
    );
    true
}

pub(super) fn finish_internal_line_draft(state: &mut CssRunnerState) -> bool {
    let Some(ToolDraft2d::InternalLine {
        panel_id,
        points_local_m,
        ..
    }) = state.draft_2d.take()
    else {
        return false;
    };
    if points_local_m.len() < 2 {
        state.status_message = "Internal Line requires at least 2 points".into();
        return true;
    }
    let count = points_local_m.len();
    commit_authoring_command(
        state,
        Command::InternalLine {
            panel: panel_id.clone(),
            points: points_local_m,
            tangents: vec![DVec2::ZERO; count],
        },
        format!("Added {count}-point internal line on {panel_id}"),
    );
    true
}

/// Same relative tolerance the seam compiler uses before it demands `allow_ease`
/// (`garment::seam`), so the UI never offers an un-eased seam the compiler rejects.
pub fn sew_lengths_equal(a_len_m: f64, b_len_m: f64) -> bool {
    (a_len_m - b_len_m).abs() <= 1e-8 * a_len_m.max(b_len_m)
}

pub(super) fn sew_confirm_prompt(
    a: &(String, String),
    b: &(String, String),
    a_len_m: f64,
    b_len_m: f64,
) -> String {
    let lengths = format!(
        "Sew {}.{} ({:.1} mm) ↔ {}.{} ({:.1} mm)",
        a.0,
        a.1,
        a_len_m * 1e3,
        b.0,
        b.1,
        b_len_m * 1e3
    );
    if sew_lengths_equal(a_len_m, b_len_m) {
        format!("{lengths}: press Enter or Sew to confirm, Escape to cancel")
    } else {
        format!(
            "{lengths}: lengths differ by {:.1} mm; choose Sew with easing or Escape to cancel",
            (a_len_m - b_len_m).abs() * 1e3
        )
    }
}

/// Commit the pending seam. Without `ease`, unequal lengths are refused and the
/// draft is kept so the user can still choose easing or cancel.
pub fn confirm_pending_sew(state: &mut CssRunnerState, ease: bool) -> bool {
    let Some(ToolDraft2d::SewConfirm {
        a,
        b,
        reverse,
        a_len_m,
        b_len_m,
    }) = state.draft_2d.clone()
    else {
        return false;
    };
    let equal = sew_lengths_equal(a_len_m, b_len_m);
    if !ease && !equal {
        state.status_message = format!(
            "Lengths differ by {:.1} mm: choose Sew with easing to accept the mismatch, or Escape to cancel",
            (a_len_m - b_len_m).abs() * 1e3
        );
        return false;
    }
    state.draft_2d = None;
    let message = if equal {
        format!("Sewed {}.{} ↔ {}.{}", a.0, a.1, b.0, b.1)
    } else {
        format!(
            "Sewed {}.{} ↔ {}.{} with {:.1} mm easing",
            a.0,
            a.1,
            b.0,
            b.1,
            (a_len_m - b_len_m).abs() * 1e3
        )
    };
    commit_authoring_command(
        state,
        Command::Sew {
            a: BoundaryRef {
                reversed: false,
                panel: a.0,
                side: Side::Right,
                edge: Some(a.1),
                range: [0.0, 1.0],
            },
            b: BoundaryRef {
                reversed: false,
                panel: b.0,
                side: Side::Left,
                edge: Some(b.1),
                range: [0.0, 1.0],
            },
            reverse,
            ease: ease && !equal,
        },
        message,
    )
}
