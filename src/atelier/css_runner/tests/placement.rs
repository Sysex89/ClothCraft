//! Arrange number fields edit a draft and commit one Command::Place per edit.

use super::super::super::worker;
use super::super::actions::{RunnerUiOp, apply_runner_ui_op, update_runner};
use super::super::editing::undo_redo_2d;
use super::super::input::handle_runner_event;
use super::super::input::number::tick_number_drag;
use super::super::placement::{PlacementAxis, placement_value};
use super::super::state::CssRunnerState;
use super::super::ui::number::{NumberField, number_field};
use super::support::{pointer_btn_ev, pointer_move_ev, selection_test_state};
use cssimpler::core::Node;
use cssimpler::renderer::{EngineEvent, FrameInfo};
use std::sync::atomic::Ordering;
use std::time::Duration;

fn frame(state: &mut CssRunnerState, index: u64) {
    update_runner(
        state,
        FrameInfo {
            delta: Duration::from_millis(16),
            frame_index: index,
        },
    );
}

fn named_key(name: &str) -> EngineEvent {
    EngineEvent::Key(cssimpler::renderer::KeyboardEvent {
        logical_key: cssimpler::renderer::KeyIdentity::Named(name.into()),
        physical_key: None,
        location: cssimpler::renderer::KeyLocation::Standard,
        state: cssimpler::renderer::ButtonState::Pressed,
        repeat: false,
        modifiers: cssimpler::renderer::KeyboardModifiers::default(),
    })
}

fn press_drag_handle(state: &CssRunnerState, axis: PlacementAxis) {
    let Node::Element(control) = number_field(state, "arrange-test", NumberField::Placement(axis))
    else {
        panic!("number field root is an element")
    };
    let Node::Element(handle) = &control.children[1] else {
        panic!("number field has a drag handle")
    };
    handle.handlers.mousedown().unwrap()();
}

/// Simulate typing a value into a focused placement field.
fn type_value(state: &mut CssRunnerState, axis: PlacementAxis, value: f64) {
    state.numeric_input_active.store(true, Ordering::Relaxed);
    apply_runner_ui_op(
        state,
        RunnerUiOp::SetNumber(NumberField::Placement(axis), value),
    );
}

fn tx_cm(state: &CssRunnerState, panel: usize) -> f64 {
    state.project.template.panels[panel].placement.translation.x * 100.0
}

#[test]
fn arrange_fields_commit_one_undoable_place_per_edit() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut state = selection_test_state(dir.path());
    assert!(state.project.template.panels.len() >= 2);
    state.selected_piece_index = 0;
    let original_tx = tx_cm(&state, 0);
    let original_rz = state.project.template.panels[0]
        .placement
        .rotation_degrees
        .z;
    let mut index = 1;

    // 1. Typing only edits the draft; the document changes when focus leaves.
    type_value(&mut state, PlacementAxis::TranslationX, original_tx + 12.5);
    frame(&mut state, index);
    index += 1;
    assert_eq!(tx_cm(&state, 0), original_tx, "typing must not commit");
    assert_eq!(
        placement_value(&state, PlacementAxis::TranslationX),
        original_tx + 12.5
    );
    handle_runner_event(&mut state, &pointer_move_ev(5.0, 5.0));
    handle_runner_event(&mut state, &pointer_btn_ev(true));
    handle_runner_event(&mut state, &pointer_btn_ev(false));
    frame(&mut state, index);
    index += 1;
    assert!((tx_cm(&state, 0) - (original_tx + 12.5)).abs() < 1e-9);
    assert!(state.placement_draft.is_none());
    assert!(undo_redo_2d(&mut state, false), "one undo step");
    assert_eq!(tx_cm(&state, 0), original_tx);
    assert!(
        !undo_redo_2d(&mut state, false),
        "only one step was recorded"
    );

    // 2. Escape while typing discards the draft without history.
    type_value(&mut state, PlacementAxis::RotationZ, 45.0);
    assert!(!handle_runner_event(&mut state, &named_key("Escape")));
    assert!(state.placement_draft.is_none());
    frame(&mut state, index);
    index += 1;
    assert_eq!(
        state.project.template.panels[0]
            .placement
            .rotation_degrees
            .z,
        original_rz
    );
    assert!(!undo_redo_2d(&mut state, false));

    // 3. Enter applies a typed value while the field keeps focus.
    type_value(&mut state, PlacementAxis::RotationZ, 30.0);
    assert!(!handle_runner_event(&mut state, &named_key("Enter")));
    assert_eq!(
        state.project.template.panels[0]
            .placement
            .rotation_degrees
            .z,
        30.0
    );
    assert!(state.numeric_input_active.load(Ordering::Relaxed));
    assert!(undo_redo_2d(&mut state, false));
    state.numeric_input_active.store(false, Ordering::Relaxed);

    // 4. A drag previews in the field and commits once on release.
    handle_runner_event(&mut state, &pointer_move_ev(100.0, 100.0));
    press_drag_handle(&state, PlacementAxis::TranslationX);
    for (step, y) in [96.0, 92.0, 88.0].into_iter().enumerate() {
        handle_runner_event(&mut state, &pointer_move_ev(100.0, y));
        assert!(tick_number_drag(&mut state));
        frame(&mut state, index);
        index += 1;
        assert_eq!(tx_cm(&state, 0), original_tx, "a drag must not commit");
        assert!(
            (placement_value(&state, PlacementAxis::TranslationX)
                - (original_tx + 0.5 * (step + 1) as f64))
                .abs()
                < 1e-9
        );
    }
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    frame(&mut state, index);
    index += 1;
    assert!((tx_cm(&state, 0) - (original_tx + 1.5)).abs() < 1e-9);
    assert!(undo_redo_2d(&mut state, false));
    assert!(!undo_redo_2d(&mut state, false), "the drag was one step");

    // 5. An unchanged value records nothing.
    type_value(&mut state, PlacementAxis::TranslationX, original_tx);
    state.numeric_input_active.store(false, Ordering::Relaxed);
    frame(&mut state, index);
    index += 1;
    assert!(state.placement_draft.is_none());
    assert!(!undo_redo_2d(&mut state, false));

    // 6. Selecting another piece commits the draft to the edited piece and
    //    keeps the new selection.
    type_value(&mut state, PlacementAxis::TranslationX, original_tx - 7.0);
    handle_runner_event(&mut state, &pointer_btn_ev(true));
    apply_runner_ui_op(&mut state, RunnerUiOp::SelectPiece(1));
    handle_runner_event(&mut state, &pointer_btn_ev(false));
    frame(&mut state, index);
    assert!((tx_cm(&state, 0) - (original_tx - 7.0)).abs() < 1e-9);
    assert_eq!(state.selected_piece_index, 1);
    assert_eq!(
        placement_value(&state, PlacementAxis::TranslationX),
        tx_cm(&state, 1)
    );
}
