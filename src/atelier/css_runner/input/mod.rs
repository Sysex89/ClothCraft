//! Event routing and focus/resize handling.

use super::board::types::Drag2d;
use super::state::CssRunnerState;
use cssimpler::renderer::EngineEvent;

pub(super) mod buttons;
pub(super) mod cursor_wrap;
pub(super) mod keyboard;
pub(super) mod motion;
pub(super) mod number;
pub(super) mod scroll;

/// Route one engine event and coalesce pointer-driven repaints.
///
/// Raw pointer events are still applied in arrival order. Motion and wheel
/// handling only marks the runner dirty; the next displayed frame repaints the
/// latest state once. Press, release, keyboard, text and focus events stay
/// immediate.
pub fn handle_runner_event(state: &mut CssRunnerState, event: &EngineEvent) -> bool {
    state.frame_drag.feed(event);
    let handled = route_runner_event(state, event);
    if handled
        && matches!(
            event,
            EngineEvent::PointerMoved { .. } | EngineEvent::Scroll { .. }
        )
    {
        state.input_dirty = true;
        false
    } else {
        handled
    }
}

fn route_runner_event(state: &mut CssRunnerState, event: &EngineEvent) -> bool {
    use std::sync::atomic::Ordering;
    let text_editing = state.numeric_input_active.load(Ordering::Relaxed);
    // A press ends the pending Arrange edit. The commit is queued behind any
    // keystrokes still waiting, so they are included; it may run after the
    // press changed the selection, which the commit keeps.
    if (text_editing || state.placement_draft.is_some())
        && matches!(
            event,
            EngineEvent::PointerButton {
                state: cssimpler::renderer::ButtonState::Pressed,
                ..
            }
        )
    {
        state.actions.push(super::actions::RunnerAction::Ui(
            super::actions::RunnerUiOp::CommitPlacement,
        ));
    }
    if number::handle_number_event(state, event) {
        return true;
    }
    // Native numeric text inputs own keyboard events until the next pointer press.
    if matches!(
        event,
        EngineEvent::PointerButton {
            state: cssimpler::renderer::ButtonState::Pressed,
            ..
        }
    ) {
        state.mesh_input_focus = false;
    }
    if let EngineEvent::Key(k) = event
        && k.logical_key == cssimpler::renderer::KeyIdentity::Named("Tab".into())
        && state.mesh_draft.open
    {
        state.mesh_input_focus = true;
    }
    if state.mesh_input_focus && matches!(event, EngineEvent::Key(_)) {
        return false;
    }

    // Tab enters the native input cycle; preserve keyboard ownership there.
    if matches!(event, EngineEvent::Key(k) if k.state == cssimpler::renderer::ButtonState::Pressed && k.logical_key == cssimpler::renderer::KeyIdentity::Named("Tab".into()))
    {
        state.numeric_input_active.store(true, Ordering::Relaxed);
        return false;
    }
    if matches!(event, EngineEvent::Key(_)) && text_editing {
        // Enter applies a typed placement; the text field keeps focus. The
        // commit is queued behind the keystrokes it applies.
        if matches!(event, EngineEvent::Key(k) if k.state == cssimpler::renderer::ButtonState::Pressed && k.logical_key == cssimpler::renderer::KeyIdentity::Named("Enter".into()))
        {
            state.actions.push(super::actions::RunnerAction::Ui(
                super::actions::RunnerUiOp::CommitPlacement,
            ));
        }
        return false;
    }
    match event {
        EngineEvent::FocusChanged(false) => {
            state.last_2d_click = None;
            if matches!(state.drag_2d, Some(Drag2d::Marquee { .. })) {
                state.pointer_down = false;
                state.drag_2d = None;
                return true;
            }
            false
        }
        EngineEvent::ViewportChanged(vp) => {
            let new_size = (vp.width.max(1) as f32, vp.height.max(1) as f32);
            if state.window_size != new_size {
                state.window_size = new_size;
                return true;
            }
            false
        }
        EngineEvent::Key(_) => keyboard::handle_keyboard(state, event),
        EngineEvent::PointerButton { .. } => buttons::handle_buttons(state, event),
        EngineEvent::PointerMoved { .. } => motion::handle_motion(state, event),
        EngineEvent::Scroll { .. } => scroll::handle_scroll(state, event),
        _ => false,
    }
}
