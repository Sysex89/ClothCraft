//! Pointer-anchored board zoom and 3D camera zoom.

use super::super::board::geometry::{
    compute_2d_view_box_m, compute_2d_viewport_rect, viewport_3d_rect,
};
use super::super::state::CssRunnerState;
use cssimpler::renderer::EngineEvent;

pub(in super::super) fn handle_scroll(state: &mut CssRunnerState, event: &EngineEvent) -> bool {
    match event {
        EngineEvent::Scroll { delta, .. } => {
            if state.settings_open
                || state.command_palette_open
                || state.solver_menu_open
                || state.import_dialog.is_some()
            {
                return false;
            }
            let dy = match delta {
                cssimpler::renderer::ScrollDelta::Lines { y, .. } => *y * 0.2,
                cssimpler::renderer::ScrollDelta::Pixels { y, .. } => *y * 0.005,
            };
            if !dy.is_finite() || dy == 0.0 {
                return false;
            }
            let (px, py) = state.last_pointer_pos;
            let canvas_2d = compute_2d_viewport_rect(state);
            let rect_3d = viewport_3d_rect(state);
            if canvas_2d.contains(px, py) {
                let (_, _, width, height) = compute_2d_view_box_m(state);
                let offset_x = (f64::from(px - canvas_2d.x) / f64::from(canvas_2d.w) - 0.5) * width;
                let offset_y =
                    (f64::from(py - canvas_2d.y) / f64::from(canvas_2d.h) - 0.5) * height;
                let old_zoom = state.board_2d_zoom.clamp(0.25, 10.0);
                let factor = (1.0 + dy as f64 * 0.15 * state.board_2d_zoom_speed).clamp(0.5, 2.0);
                let new_zoom = (old_zoom * factor).clamp(0.25, 10.0);
                let scale_change = 1.0 - old_zoom / new_zoom;
                state.board_2d_zoom = new_zoom;
                state.board_2d_pan.0 += offset_x * scale_change;
                state.board_2d_pan.1 += offset_y * scale_change;
                return true;
            } else if rect_3d.contains(px, py) {
                state.camera.distance = (state.camera.distance - dy).clamp(0.5, 15.0);
                state.sync_usd_camera();
                return true;
            }
            false
        }
        _ => false,
    }
}
