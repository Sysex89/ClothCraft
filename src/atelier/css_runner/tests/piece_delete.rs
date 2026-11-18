//! Deleting a whole piece from the 2D board with Delete/Backspace.

use super::super::super::{DocumentFiles, worker};
use super::super::board::geometry::panel_to_board_pt;
use super::super::board::hit_testing::hit_test_2d;
use super::super::board::types::Hover2dTarget;
use super::super::editing::{commit_authoring_command, undo_redo_2d};
use super::super::input::handle_runner_event;
use super::super::state::CssRunnerState;
use super::support::{board_to_screen_px, pointer_btn_ev, pointer_move_ev};
use clothcraft::authoring::Command;
use clothcraft::fitting::FittingProject;
use clothcraft::garment::presets::GarmentKind;
use cssimpler::renderer::EngineEvent;
use glam::DVec2;

fn key(st: &mut CssRunnerState, name: &str) -> bool {
    let logical_key = if name.chars().count() == 1 {
        cssimpler::renderer::KeyIdentity::Character(name.into())
    } else {
        cssimpler::renderer::KeyIdentity::Named(name.into())
    };
    handle_runner_event(
        st,
        &EngineEvent::Key(cssimpler::renderer::KeyboardEvent {
            logical_key,
            physical_key: None,
            location: cssimpler::renderer::KeyLocation::Standard,
            state: cssimpler::renderer::ButtonState::Pressed,
            repeat: false,
            modifiers: cssimpler::renderer::KeyboardModifiers::default(),
        }),
    )
}

fn click_board(st: &mut CssRunnerState, board_m: DVec2) {
    let (sx, sy) = board_to_screen_px(st, board_m);
    handle_runner_event(st, &pointer_move_ev(sx, sy));
    assert!(handle_runner_event(st, &pointer_btn_ev(true)));
    assert!(handle_runner_event(st, &pointer_btn_ev(false)));
}

/// A board point whose hit test is the panel body, not a corner or edge.
fn body_point(st: &CssRunnerState, pidx: usize) -> DVec2 {
    let tmpl = &st.project.template;
    let pts: Vec<DVec2> = tmpl.panels[pidx]
        .polygon
        .as_ref()
        .unwrap()
        .points
        .iter()
        .map(|&q| panel_to_board_pt(tmpl, pidx, q))
        .collect();
    let lo = pts.iter().fold(DVec2::splat(f64::MAX), |a, &b| a.min(b));
    let hi = pts.iter().fold(DVec2::splat(f64::MIN), |a, &b| a.max(b));
    (1..20)
        .flat_map(|i| (1..20).map(move |j| (i, j)))
        .map(|(i, j)| lo + (hi - lo) * DVec2::new(i as f64 / 20.0, j as f64 / 20.0))
        .find(|&b| matches!(hit_test_2d(st, b), Some(Hover2dTarget::Panel(i)) if i == pidx))
        .expect("panel body point")
}

fn seams_on(st: &CssRunnerState, pid: &str) -> usize {
    st.project
        .template
        .seams
        .iter()
        .filter(|s| s.references().any(|r| r.panel == pid))
        .count()
}

#[test]
fn test_2d_delete_key_removes_explicitly_selected_piece() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let avatar = dir.path().join("avatar.obj");
    std::fs::write(&avatar, "v 0 0 -10\nv 1 0 -10\nv 0 1 -10\nf 1 2 3\n").expect("write avatar");
    let project = FittingProject::basic(GarmentKind::Tshirt, 0, avatar);
    let scene = project.compile().expect("compile scene");
    let files = DocumentFiles {
        output: dir.path().join("out_delete_piece"),
        source: None,
        modified: false,
    };
    let (tx, latest) = worker::start_cpu().expect("start cpu worker");
    let mut state = CssRunnerState::new(project, scene, files, tx, latest);
    state.active_2d_tool = "Edit".into();

    // GarmentTemplate has no PartialEq; compare what save/reload preserves.
    let snap = |st: &CssRunnerState| serde_json::to_value(&st.project.template).unwrap();
    let original = snap(&state);
    let original_ids: Vec<String> = state
        .project
        .template
        .panels
        .iter()
        .map(|p| p.id.clone())
        .collect();
    let panel_count = original_ids.len();
    assert!(panel_count > 1);

    // 1. The default highlighted piece is not an explicit selection.
    assert!(!state.piece_selected_explicitly);
    key(&mut state, "Delete");
    assert_eq!(snap(&state), original);

    // 2. Deleting a point, then pressing Delete again, never escalates to the piece.
    let nr = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .edges
        .iter()
        .position(|e| e == "neck-right")
        .unwrap();
    let corner = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .points[nr];
    let corner_board = panel_to_board_pt(&state.project.template, 0, corner);
    click_board(&mut state, corner_board);
    assert_eq!(state.selected_2d_points, vec![("front".into(), nr)]);
    assert!(!state.piece_selected_explicitly);
    assert!(key(&mut state, "Delete"));
    let after_point = snap(&state);
    assert_ne!(after_point, original);
    key(&mut state, "Backspace");
    assert_eq!(snap(&state), after_point);
    assert!(undo_redo_2d(&mut state, false));
    assert_eq!(snap(&state), original);

    // 3. Clicking the piece body selects it explicitly; Delete removes the
    //    piece and its seams as one undo step.
    let front_seams = seams_on(&state, "front");
    assert!(front_seams > 0);
    let body = body_point(&state, 0);
    click_board(&mut state, body);
    assert_eq!(state.selected_piece_index, 0);
    assert!(state.piece_selected_explicitly);
    assert!(key(&mut state, "Delete"));
    let tmpl = &state.project.template;
    assert_eq!(tmpl.panels.len(), panel_count - 1);
    assert!(tmpl.panels.iter().all(|p| p.id != "front"));
    assert_eq!(seams_on(&state, "front"), 0);
    assert!(state.selected_piece_index < tmpl.panels.len());
    assert!(!state.piece_selected_explicitly);
    state
        .project
        .template
        .compile()
        .expect("remaining garment compiles");
    let after_delete = snap(&state);
    key(&mut state, "Delete");
    assert_eq!(snap(&state), after_delete);
    assert!(undo_redo_2d(&mut state, false));
    assert_eq!(snap(&state), original);
    assert_eq!(seams_on(&state, "front"), front_seams);

    // 4. `]` selects the next piece explicitly; Escape withdraws that selection.
    assert!(key(&mut state, "]"));
    assert!(state.piece_selected_explicitly);
    assert!(key(&mut state, "Escape"));
    assert!(!state.piece_selected_explicitly);
    key(&mut state, "Delete");
    assert_eq!(snap(&state), original);

    // 5. The last piece is kept: the rejected command leaves the document unchanged.
    let others = original_ids[1..].to_vec();
    assert!(commit_authoring_command(
        &mut state,
        Command::RemovePanels { panels: others },
        "Removed other pieces",
    ));
    assert_eq!(state.project.template.panels.len(), 1);
    let single = snap(&state);
    let body = body_point(&state, 0);
    click_board(&mut state, body);
    assert!(state.piece_selected_explicitly);
    assert!(key(&mut state, "Delete"));
    assert_eq!(snap(&state), single);
}
