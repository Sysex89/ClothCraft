//! Arrange number fields edit a draft and commit one Command::Place per edit.
//!
//! Typing goes through the rendered field's real input handler, which queues
//! values like a user's keystrokes; tests process the queue with `frame`.

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

struct Frames {
    index: u64,
}

impl Frames {
    fn run(&mut self, state: &mut CssRunnerState) {
        update_runner(
            state,
            FrameInfo {
                delta: Duration::from_millis(16),
                frame_index: self.index,
            },
        );
        self.index += 1;
    }
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

fn selected_field(state: &CssRunnerState, axis: PlacementAxis) -> NumberField {
    NumberField::Placement {
        panel: state.project.template.panels[state.selected_piece_index]
            .id
            .as_str()
            .into(),
        axis,
    }
}

/// The selected piece's `axis` field as the inspector renders it.
fn arrange_control(state: &CssRunnerState, axis: PlacementAxis) -> Node {
    number_field(state, "arrange-test", selected_field(state, axis))
}

fn control_child(control: &Node, index: usize) -> &cssimpler::core::ElementNode {
    let Node::Element(control) = control else {
        panic!("number field root is an element")
    };
    let Node::Element(child) = &control.children[index] else {
        panic!("number field child {index} is an element")
    };
    child
}

/// Click into the text input, as the window does after the engine press.
fn focus(control: &Node) {
    control_child(control, 0).handlers.mousedown().unwrap()();
}

/// Type the field's full text; the handler queues the parsed value.
fn type_text(control: &Node, text: &str) {
    control_child(control, 0).handlers.input().unwrap()(text);
}

fn press_drag_handle(state: &CssRunnerState, axis: PlacementAxis) {
    control_child(&arrange_control(state, axis), 1)
        .handlers
        .mousedown()
        .unwrap()();
}

fn tx_cm(state: &CssRunnerState, panel: usize) -> f64 {
    state.project.template.panels[panel].placement.translation.x * 100.0
}

fn rz(state: &CssRunnerState, panel: usize) -> f64 {
    state.project.template.panels[panel]
        .placement
        .rotation_degrees
        .z
}

fn shown(state: &CssRunnerState, axis: PlacementAxis) -> f64 {
    let panel = &state.project.template.panels[state.selected_piece_index].id;
    placement_value(state, panel, axis)
}

fn test_state(dir: &std::path::Path) -> CssRunnerState {
    let mut state = selection_test_state(dir);
    assert!(state.project.template.panels.len() >= 2);
    state.selected_piece_index = 0;
    state
}

#[test]
fn arrange_fields_commit_one_undoable_place_per_edit() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut state = test_state(dir.path());
    let original_tx = tx_cm(&state, 0);
    let original_rz = rz(&state, 0);
    let mut frames = Frames { index: 1 };

    // 1. Typing only edits the draft; the document changes when focus leaves.
    let tx = arrange_control(&state, PlacementAxis::TranslationX);
    focus(&tx);
    type_text(&tx, &format!("{}", original_tx + 12.5));
    frames.run(&mut state);
    assert_eq!(tx_cm(&state, 0), original_tx, "typing must not commit");
    assert_eq!(
        shown(&state, PlacementAxis::TranslationX),
        original_tx + 12.5
    );
    handle_runner_event(&mut state, &pointer_move_ev(5.0, 5.0));
    handle_runner_event(&mut state, &pointer_btn_ev(true));
    handle_runner_event(&mut state, &pointer_btn_ev(false));
    frames.run(&mut state);
    assert!((tx_cm(&state, 0) - (original_tx + 12.5)).abs() < 1e-9);
    assert!(state.placement_draft.is_none());
    assert!(undo_redo_2d(&mut state, false), "one undo step");
    assert_eq!(tx_cm(&state, 0), original_tx);
    assert!(
        !undo_redo_2d(&mut state, false),
        "only one step was recorded"
    );

    // 2. Escape while typing discards the draft without history.
    let rz_control = arrange_control(&state, PlacementAxis::RotationZ);
    focus(&rz_control);
    type_text(&rz_control, "45");
    frames.run(&mut state);
    assert!(!handle_runner_event(&mut state, &named_key("Escape")));
    frames.run(&mut state);
    assert!(state.placement_draft.is_none());
    assert_eq!(rz(&state, 0), original_rz);
    assert!(!undo_redo_2d(&mut state, false));

    // 3. Enter applies a typed value while the field keeps focus.
    focus(&rz_control);
    type_text(&rz_control, "30");
    assert!(!handle_runner_event(&mut state, &named_key("Enter")));
    frames.run(&mut state);
    assert_eq!(rz(&state, 0), 30.0);
    assert!(state.numeric_input_active.load(Ordering::Relaxed));
    assert!(undo_redo_2d(&mut state, false));
    state.numeric_input_active.store(false, Ordering::Relaxed);

    // 4. A drag previews in the field and commits once on release.
    handle_runner_event(&mut state, &pointer_move_ev(100.0, 100.0));
    press_drag_handle(&state, PlacementAxis::TranslationX);
    for (step, y) in [96.0, 92.0, 88.0].into_iter().enumerate() {
        handle_runner_event(&mut state, &pointer_move_ev(100.0, y));
        assert!(tick_number_drag(&mut state));
        frames.run(&mut state);
        assert_eq!(tx_cm(&state, 0), original_tx, "a drag must not commit");
        assert!(
            (shown(&state, PlacementAxis::TranslationX) - (original_tx + 0.5 * (step + 1) as f64))
                .abs()
                < 1e-9
        );
    }
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    frames.run(&mut state);
    assert!((tx_cm(&state, 0) - (original_tx + 1.5)).abs() < 1e-9);
    assert!(undo_redo_2d(&mut state, false));
    assert!(!undo_redo_2d(&mut state, false), "the drag was one step");

    // 5. An unchanged value records nothing.
    focus(&tx);
    type_text(&tx, &format!("{original_tx}"));
    state.numeric_input_active.store(false, Ordering::Relaxed);
    frames.run(&mut state);
    assert!(state.placement_draft.is_none());
    assert!(!undo_redo_2d(&mut state, false));

    // 6. Selecting another piece commits the draft to the edited piece and
    //    keeps the new selection.
    focus(&tx);
    type_text(&tx, &format!("{}", original_tx - 7.0));
    handle_runner_event(&mut state, &pointer_btn_ev(true));
    apply_runner_ui_op(&mut state, RunnerUiOp::SelectPiece(1));
    handle_runner_event(&mut state, &pointer_btn_ev(false));
    frames.run(&mut state);
    assert!((tx_cm(&state, 0) - (original_tx - 7.0)).abs() < 1e-9);
    assert_eq!(state.selected_piece_index, 1);
    assert_eq!(shown(&state, PlacementAxis::TranslationX), tx_cm(&state, 1));
}

/// Escape must discard keystrokes that were queued but not yet processed.
#[test]
fn escape_discards_keystrokes_still_in_the_queue() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut state = test_state(dir.path());
    let original_rz = rz(&state, 0);
    let mut frames = Frames { index: 1 };

    let control = arrange_control(&state, PlacementAxis::RotationZ);
    focus(&control);
    type_text(&control, "4");
    type_text(&control, "45");
    // No frame between the keystrokes and Escape.
    assert!(!handle_runner_event(&mut state, &named_key("Escape")));
    for _ in 0..3 {
        frames.run(&mut state);
    }
    assert!(state.placement_draft.is_none());
    assert_eq!(rz(&state, 0), original_rz);
    assert!(!undo_redo_2d(&mut state, false), "nothing was recorded");
}

/// Enter must apply the latest keystroke, as one undo step.
#[test]
fn enter_applies_the_latest_queued_keystroke_once() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut state = test_state(dir.path());
    let original_rz = rz(&state, 0);
    let mut frames = Frames { index: 1 };

    let control = arrange_control(&state, PlacementAxis::RotationZ);
    focus(&control);
    type_text(&control, "3");
    type_text(&control, "30");
    // No frame between the keystrokes and Enter.
    assert!(!handle_runner_event(&mut state, &named_key("Enter")));
    frames.run(&mut state);
    assert_eq!(rz(&state, 0), 30.0);
    assert!(state.placement_draft.is_none());
    // Leaving the field afterwards records nothing more.
    handle_runner_event(&mut state, &pointer_btn_ev(true));
    handle_runner_event(&mut state, &pointer_btn_ev(false));
    frames.run(&mut state);
    assert!(undo_redo_2d(&mut state, false));
    assert_eq!(rz(&state, 0), original_rz);
    assert!(!undo_redo_2d(&mut state, false), "Enter recorded one step");
}

/// A value queued for one piece must not land on a piece selected before the
/// queue is processed, and committing it must not steal the selection.
#[test]
fn queued_edit_keeps_its_piece_when_the_selection_changes() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut state = test_state(dir.path());
    let original_tx = [tx_cm(&state, 0), tx_cm(&state, 1)];
    let mut frames = Frames { index: 1 };

    // The field was rendered for piece 0 and the keystroke is still queued.
    let control = arrange_control(&state, PlacementAxis::TranslationX);
    focus(&control);
    type_text(&control, &format!("{}", original_tx[0] + 20.0));
    // The selection changes before the queue is processed.
    apply_runner_ui_op(&mut state, RunnerUiOp::SelectPiece(1));
    handle_runner_event(&mut state, &pointer_btn_ev(true));
    handle_runner_event(&mut state, &pointer_btn_ev(false));
    frames.run(&mut state);

    assert!((tx_cm(&state, 0) - (original_tx[0] + 20.0)).abs() < 1e-9);
    assert_eq!(tx_cm(&state, 1), original_tx[1], "piece 1 is untouched");
    assert_eq!(
        state.selected_piece_index, 1,
        "the commit keeps the selection"
    );
    assert!(undo_redo_2d(&mut state, false));
    assert_eq!(tx_cm(&state, 0), original_tx[0]);
    assert!(!undo_redo_2d(&mut state, false));
}

/// A field for a piece deleted before its value is processed changes nothing.
#[test]
fn queued_edit_for_a_deleted_piece_is_dropped() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut state = test_state(dir.path());
    let mut frames = Frames { index: 1 };

    let control = arrange_control(&state, PlacementAxis::TranslationX);
    focus(&control);
    state.project.template.panels.remove(0);
    let before = serde_json::to_value(&state.project.template).unwrap();
    type_text(&control, "42");
    state.numeric_input_active.store(false, Ordering::Relaxed);
    frames.run(&mut state);
    assert!(state.placement_draft.is_none());
    assert_eq!(
        serde_json::to_value(&state.project.template).unwrap(),
        before
    );
    assert!(!undo_redo_2d(&mut state, false));
}
