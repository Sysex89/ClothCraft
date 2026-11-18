//! ClothCraft drag handles; cssimpler owns native text editing.
use super::super::{
    actions::{RunnerAction, RunnerUiOp, apply_runner_ui_op},
    state::CssRunnerState,
    ui::number::NumberField,
};
use super::cursor_wrap::CursorWrap;
use cssimpler::renderer::{ButtonState, EngineEvent, KeyIdentity, PointerButton};

#[derive(Default)]
pub(in super::super) struct NumberDragState {
    pub pointer_y: f32,
    pub shift: bool,
    pub control: bool,
    pub active: Option<NumberDrag>,
}
pub(in super::super) struct NumberDrag {
    pub field: NumberField,
    pub last_y: f32,
    pub initial_value: f64,
    pub initial_mode_radius: bool,
    pub initial_preview_active: bool,
    pub value: f64,
    pub remainder: f64,
    pub cursor: Option<CursorWrap>,
}

fn move_value(
    drag: &mut NumberDrag,
    dy: f64,
    shift: bool,
    control: bool,
) -> Option<(NumberField, f64)> {
    drag.remainder += dy;
    let steps = (drag.remainder / 4.0).trunc();
    if steps == 0.0 {
        return None;
    }
    drag.remainder -= steps * 4.0;
    let (min, max, step, _) = drag.field.spec();
    let multiplier = if shift { 10.0 } else { 1.0 } * if control { 0.1 } else { 1.0 };
    drag.value = (drag.value + steps * step * multiplier).clamp(min, max);
    // Keep precision from fine drags when modifiers are released. Never round
    // to the normal step, which would discard Ctrl's smaller increments.
    Some((drag.field.clone(), drag.value))
}

/// Apply a drag edit. Placement edits join the action queue so they stay
/// ordered with typed values and with Enter/Escape; see `placement`.
fn apply_number_edit(state: &mut CssRunnerState, field: NumberField, value: f64) {
    let op = RunnerUiOp::SetNumber(field, value);
    if matches!(op, RunnerUiOp::SetNumber(NumberField::Placement { .. }, _)) {
        state.actions.push(RunnerAction::Ui(op));
    } else {
        apply_runner_ui_op(state, op);
    }
}

pub(in super::super) fn handle_number_event(
    state: &mut CssRunnerState,
    event: &EngineEvent,
) -> bool {
    use std::sync::atomic::Ordering;
    let mut drag = state.number_drag.lock().unwrap();
    let mut edit = None;
    let mut cancelled_preview = None;
    let mut consumed = false;
    match event {
        EngineEvent::PointerMoved {
            position,
            modifiers,
        } => {
            drag.pointer_y = position.y;
            drag.shift = modifiers.shift;
            drag.control = modifiers.control;
            if drag.active.is_some() {
                // Value changes are applied once per displayed frame in
                // tick_number_drag; this event only records the latest pointer.
                state.last_pointer_pos = (position.x, position.y);
                consumed = true;
            }
        }
        EngineEvent::ModifiersChanged(modifiers) => {
            drag.shift = modifiers.shift;
            drag.control = modifiers.control;
        }
        EngineEvent::PointerButton {
            button: PointerButton::Primary,
            state: button,
            modifiers,
        } => {
            drag.shift = modifiers.shift;
            drag.control = modifiers.control;
            if *button == ButtonState::Pressed {
                state.numeric_input_active.store(false, Ordering::Relaxed);
            }
            if *button == ButtonState::Released
                && let Some(mut active) = drag.active.take()
            {
                // Flush motion since the last displayed frame.
                if let Some(cursor) = &mut active.cursor {
                    if let Some(dy) = cursor.sample() {
                        edit = move_value(&mut active, dy, modifiers.shift, modifiers.control);
                    }
                } else {
                    let dy = (active.last_y - drag.pointer_y) as f64;
                    active.last_y = drag.pointer_y;
                    edit = move_value(&mut active, dy, modifiers.shift, modifiers.control);
                }
                consumed = true;
                state.pointer_down = false;
            }
        }

        EngineEvent::Key(key)
            if key.state == ButtonState::Pressed
                && key.logical_key == KeyIdentity::Named("Escape".into()) =>
        {
            if let Some(active) = drag.active.take() {
                state.pointer_down = false;
                cancelled_preview =
                    Some((active.initial_mode_radius, active.initial_preview_active));
                edit = Some((active.field, active.initial_value));
                consumed = true;
            }
            if state.numeric_input_active.swap(false, Ordering::Relaxed) {
                state.settings_open = false;
                // Escape leaves the field without applying a typed placement.
                // Queued after the keystrokes it discards.
                state
                    .actions
                    .push(RunnerAction::Ui(RunnerUiOp::CancelPlacement));
                consumed = false;
            }
        }
        EngineEvent::Key(_) | EngineEvent::Scroll { .. } if drag.active.is_some() => {
            consumed = true
        }
        EngineEvent::FocusChanged(false) | EngineEvent::Suspended => {
            if drag.active.take().is_some() {
                state.pointer_down = false;
            }
            state.numeric_input_active.store(false, Ordering::Relaxed);
        }
        _ => {}
    }
    drop(drag);
    if let Some((field, value)) = edit {
        apply_number_edit(state, field, value);
    }
    if let Some((mode, preview)) = cancelled_preview {
        state.corner_mode_radius = mode;
        state.corner_preview_active = preview;
    }
    consumed
}

pub(in super::super) fn tick_number_drag(state: &mut CssRunnerState) -> bool {
    let mut drag = state.number_drag.lock().unwrap();
    let (shift, control) = (drag.shift, drag.control);
    let pointer_y = drag.pointer_y;
    let edit = drag.active.as_mut().and_then(|active| {
        if let Some(cursor) = active.cursor.as_mut() {
            let dy = cursor.sample()?;
            move_value(active, dy, shift, control)
        } else {
            let dy = (active.last_y - pointer_y) as f64;
            active.last_y = pointer_y;
            move_value(active, dy, shift, control)
        }
    });
    drop(drag);
    if let Some((field, value)) = edit {
        apply_number_edit(state, field, value);
        true
    } else {
        false
    }
}
