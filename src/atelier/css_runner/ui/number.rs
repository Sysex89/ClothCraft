//! Shared native numeric editor for adjustable workspace values.
use super::super::{
    actions::{RunnerAction, RunnerUiOp},
    placement::{PlacementAxis, placement_value},
    state::CssRunnerState,
};
use cssimpler::{core::Node, ui};
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberField {
    ZoomSpeed,
    CornerRadius,
    CornerTension,
    Placement(PlacementAxis),
}

pub(in super::super) fn number_field(state: &CssRunnerState, id: &str, field: NumberField) -> Node {
    let value = match field {
        NumberField::ZoomSpeed => state.board_2d_zoom_speed,
        NumberField::CornerRadius => state.corner_radius_mm,
        NumberField::CornerTension => state.corner_tension,
        NumberField::Placement(axis) => placement_value(state, axis),
    };
    let (_, _, _, title) = field.spec();
    let drag = state.number_drag.clone();
    let native_drag = state.native_number_drag;
    let initial_mode_radius = state.corner_mode_radius;
    let initial_preview_active = state.corner_preview_active;
    let handle_id = format!("{id}-drag");
    let actions = state.actions.clone();
    let active = state.numeric_input_active.clone();
    let displayed = state
        .numeric_text
        .lock()
        .unwrap()
        .as_ref()
        .filter(|(key, _)| key == id && active.load(Ordering::Relaxed))
        .map(|(_, text)| text.clone())
        .unwrap_or_else(|| format_number(value));
    let focus_text = state.numeric_text.clone();
    let input_text = state.numeric_text.clone();
    let focus_id = id.to_owned();
    let input_id = id.to_owned();
    let initial_text = displayed.clone();
    ui! {
        <div class="cad-number-control">
            <input type="text" id={id} class="cad-number-input" value={displayed}
                title={title}
                onmousedown_shared={move || {
                    active.store(true, Ordering::Relaxed);
                    *focus_text.lock().unwrap() = Some((focus_id.clone(), initial_text.clone()));
                }}
                oninput_shared={move |text: &str| {
                    // Retain partial numeric strings (e.g. "2.") while native
                    // text editing owns focus. Parsing must not remove the dot.
                    *input_text.lock().unwrap() = Some((input_id.clone(), text.to_owned()));
                    if let Ok(value) = text.parse::<f64>() && value.is_finite() {
                        actions.push(RunnerAction::Ui(RunnerUiOp::SetNumber(field, value)));
                    }
                }} />
            <button id={handle_id} class="cad-number-drag"
                title="Drag up/down; Shift: 10×; Ctrl: 0.1×; Escape: cancel"
                onmousedown_shared={move || {
                    let mut drag = drag.lock().unwrap();
                    drag.active = Some(super::super::input::number::NumberDrag {
                        field, last_y: drag.pointer_y, initial_value: value, value, remainder: 0.0,
                        initial_mode_radius, initial_preview_active,
                        cursor: if native_drag { super::super::input::cursor_wrap::CursorWrap::new() } else { None },
                    });
                }}>
                <svg viewBox="0 0 24 24" class="icon" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M12 4v16M8 8l4-4 4 4M8 16l4 4 4-4" />
                </svg>
            </button>
        </div>
    }
}

impl NumberField {
    pub(in super::super) fn spec(self) -> (f64, f64, f64, &'static str) {
        match self {
            Self::ZoomSpeed => (0.25, 4.0, 0.25, "2D zoom speed (×)"),
            Self::CornerRadius => (1.0, 250.0, 1.0, "Corner radius (mm)"),
            Self::CornerTension => (0.05, 1.5, 0.05, "Corner tension"),
            Self::Placement(axis) if axis.is_rotation() => {
                (-180.0, 180.0, 1.0, "Piece rotation (degrees)")
            }
            Self::Placement(_) => (-300.0, 300.0, 0.5, "Piece position (cm)"),
        }
    }
}

fn format_number(value: f64) -> String {
    format!("{value:.4}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}
