//! Native typing and ClothCraft drag-handle integration, without a GPU run.
use super::super::{
    actions::{RunnerUiOp, apply_runner_ui_op, update_runner},
    input::{handle_runner_event, number::tick_number_drag},
    placement::{PlacementAxis, placement_value},
    ui::number::{NumberField, number_field},
};
use super::support::{pointer_btn_ev, pointer_move_ev, selection_test_state};
use cssimpler::{
    app::App,
    core::Node,
    renderer::{EngineEvent, FrameInfo, TextInputEvent},
};
use std::{sync::atomic::Ordering, time::Duration};

fn press_handle(state: &super::super::state::CssRunnerState, field: NumberField) {
    let Node::Element(control) = number_field(state, "test-number", field) else {
        panic!()
    };
    let Node::Element(handle) = &control.children[1] else {
        panic!()
    };
    handle.handlers.mousedown().unwrap()();
}

#[test]
fn number_drag_tracks_motion_with_shift_and_control_precision() {
    let _guard = super::super::super::worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut state = selection_test_state(dir.path());
    handle_runner_event(&mut state, &pointer_move_ev(100.0, 100.0));
    press_handle(&state, NumberField::CornerRadius);
    let initial = state.corner_radius_mm;
    assert!(!handle_runner_event(
        &mut state,
        &pointer_move_ev(100.0, 96.0)
    ));
    assert_eq!(
        state.corner_radius_mm, initial,
        "motion is applied once per displayed frame"
    );
    assert!(tick_number_drag(&mut state));
    assert_eq!(state.corner_radius_mm, initial + 1.0);
    for _ in 0..20 {
        assert!(!tick_number_drag(&mut state));
    }
    assert_eq!(
        state.corner_radius_mm,
        initial + 1.0,
        "stationary hold must never repeat"
    );
    let mut movement = pointer_move_ev(100.0, 92.0);
    if let EngineEvent::PointerMoved {
        ref mut modifiers, ..
    } = movement
    {
        modifiers.shift = true;
    }
    handle_runner_event(&mut state, &movement);
    assert!(tick_number_drag(&mut state));
    assert_eq!(state.corner_radius_mm, initial + 11.0);
    let mut movement = pointer_move_ev(100.0, 88.0);
    if let EngineEvent::PointerMoved {
        ref mut modifiers, ..
    } = movement
    {
        modifiers.control = true;
    }
    handle_runner_event(&mut state, &movement);
    assert!(tick_number_drag(&mut state));
    assert!((state.corner_radius_mm - initial - 11.1).abs() < 1e-9);
    let fine = state.corner_radius_mm;
    handle_runner_event(&mut state, &pointer_move_ev(100.0, 92.0));
    assert!(tick_number_drag(&mut state));
    assert!((state.corner_radius_mm - (fine - 1.0)).abs() < 1e-9);
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    let released = state.corner_radius_mm;
    handle_runner_event(&mut state, &pointer_move_ev(100.0, 80.0));
    assert_eq!(state.corner_radius_mm, released);
    press_handle(&state, NumberField::CornerRadius);
    handle_runner_event(&mut state, &EngineEvent::FocusChanged(false));
    assert!(!tick_number_drag(&mut state));
}

#[test]
fn numeric_input_uses_native_text_editing_and_clamps_model_values() {
    let _guard = super::super::super::worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let state = selection_test_state(dir.path());
    let stylesheet = clothcraft::css_ui::load_studio_stylesheet().unwrap();
    let mut app = App::new(
        state,
        &stylesheet,
        |state: &mut super::super::state::CssRunnerState, frame: FrameInfo| {
            update_runner(state, frame)
        },
        |state: &super::super::state::CssRunnerState| {
            number_field(state, "test-number", NumberField::ZoomSpeed)
        },
    )
    .with_event_handler(handle_runner_event);
    app.set_viewport(cssimpler::renderer::ViewportSize::new(200, 60));
    let frame = FrameInfo {
        delta: Duration::ZERO,
        frame_index: 0,
    };
    app.frame(frame);
    let b = app.find_element_bounds_by_id("#test-number").unwrap();
    app.handle_engine_event(&pointer_move_ev(b.x + 5.0, b.y + b.height / 2.0));
    app.handle_engine_event(&pointer_btn_ev(true));
    // The window renderer dispatches MouseDown after the engine press.
    let Node::Element(control) = number_field(app.state(), "test-number", NumberField::ZoomSpeed)
    else {
        panic!()
    };
    let Node::Element(input) = &control.children[0] else {
        panic!()
    };
    input.handlers.mousedown().unwrap()();
    app.handle_engine_event(&pointer_btn_ev(false));
    let mut select_all = EngineEvent::Key(cssimpler::renderer::KeyboardEvent {
        logical_key: cssimpler::renderer::KeyIdentity::Character("a".into()),
        physical_key: None,
        location: cssimpler::renderer::KeyLocation::Standard,
        state: cssimpler::renderer::ButtonState::Pressed,
        repeat: false,
        modifiers: Default::default(),
    });
    if let EngineEvent::Key(ref mut k) = select_all {
        k.modifiers.control = true;
    }
    app.handle_engine_event(&select_all);
    for ch in ["2", ".", "7", "5"] {
        app.handle_engine_event(&EngineEvent::TextInput(TextInputEvent::Commit(ch.into())));
        app.frame(frame);
    }
    assert_eq!(app.state().board_2d_zoom_speed, 2.75);
    assert_eq!(
        app.state().board_2d_zoom,
        1.0,
        "typing must not trigger zoom shortcuts"
    );
    assert!(app.state().numeric_input_active.load(Ordering::Relaxed));
    let panel: std::sync::Arc<str> = app.state().project.template.panels[0].id.as_str().into();
    for (field, value, expected) in [
        (NumberField::ZoomSpeed, 99.0, 4.0),
        (NumberField::CornerRadius, -5.0, 1.0),
        (NumberField::CornerTension, 5.0, 1.5),
        (
            NumberField::Placement {
                panel: panel.clone(),
                axis: PlacementAxis::TranslationX,
            },
            999.0,
            300.0,
        ),
        (
            NumberField::Placement {
                panel: panel.clone(),
                axis: PlacementAxis::RotationZ,
            },
            -400.0,
            -180.0,
        ),
        (NumberField::ImportTolerance, 99.0, 5.0),
        (NumberField::ImportMesh, -5.0, 3.0),
        (NumberField::ImportScale, -5.0, 0.0),
    ] {
        apply_runner_ui_op(app.state_mut(), RunnerUiOp::SetNumber(field.clone(), value));
        let actual = match field {
            NumberField::ZoomSpeed => app.state().board_2d_zoom_speed,
            NumberField::CornerRadius => app.state().corner_radius_mm,
            NumberField::CornerTension => app.state().corner_tension,
            NumberField::Placement { panel, axis } => placement_value(app.state(), &panel, axis),
            NumberField::ImportTolerance => app.state().import_tolerance_mm,
            NumberField::ImportMesh => app.state().import_mesh_mm,
            NumberField::ImportScale => app.state().import_scale_mm_per_unit,
        };
        assert_eq!(actual, expected);
    }
}

#[test]
fn drag_cancel_restores_value_and_limits_are_enforced() {
    let _guard = super::super::super::worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut state = selection_test_state(dir.path());
    let initial = state.corner_tension;
    let initial_mode = state.corner_mode_radius;
    let initial_preview = state.corner_preview_active;
    handle_runner_event(&mut state, &pointer_move_ev(0.0, 500.0));
    press_handle(&state, NumberField::CornerTension);
    handle_runner_event(&mut state, &pointer_move_ev(0.0, 0.0));
    assert!(tick_number_drag(&mut state));

    assert_eq!(state.corner_tension, 1.5);
    let escape = EngineEvent::Key(cssimpler::renderer::KeyboardEvent {
        logical_key: cssimpler::renderer::KeyIdentity::Named("Escape".into()),
        physical_key: None,
        location: cssimpler::renderer::KeyLocation::Standard,
        state: cssimpler::renderer::ButtonState::Pressed,
        repeat: false,
        modifiers: Default::default(),
    });
    assert!(handle_runner_event(&mut state, &escape));
    assert_eq!(state.corner_tension, initial);
    assert_eq!(state.corner_mode_radius, initial_mode);
    assert_eq!(state.corner_preview_active, initial_preview);
    assert!(!tick_number_drag(&mut state));
    press_handle(&state, NumberField::CornerTension);
    handle_runner_event(&mut state, &pointer_move_ev(0.0, 500.0));
    assert!(tick_number_drag(&mut state));

    assert_eq!(state.corner_tension, 0.05);
}

#[test]
fn control_drag_exposes_finer_decimal_places_for_zoom_and_tension() {
    let _guard = super::super::super::worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut state = selection_test_state(dir.path());
    for (field, start, expected) in [
        (NumberField::ZoomSpeed, 1.5, "1.525"),
        (NumberField::CornerTension, 0.5, "0.505"),
    ] {
        apply_runner_ui_op(&mut state, RunnerUiOp::SetNumber(field.clone(), start));
        handle_runner_event(&mut state, &pointer_move_ev(0.0, 100.0));
        press_handle(&state, field.clone());
        let movement = EngineEvent::PointerMoved {
            position: cssimpler::renderer::PointerPosition { x: 0.0, y: 96.0 },
            modifiers: cssimpler::renderer::KeyboardModifiers {
                control: true,
                ..Default::default()
            },
        };
        handle_runner_event(&mut state, &movement);
        assert!(tick_number_drag(&mut state));
        let Node::Element(control) = number_field(&state, "fine", field) else {
            panic!()
        };
        let Node::Element(input) = &control.children[0] else {
            panic!()
        };
        assert_eq!(input.attribute("value"), Some(expected));
        handle_runner_event(&mut state, &pointer_btn_ev(false));
    }
}
