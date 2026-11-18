//! The 2D Sew tool waits for an explicit easing choice before committing.

use super::super::super::{DocumentFiles, worker};
use super::super::actions::{RunnerUiOp, apply_runner_ui_op};
use super::super::board::geometry::panel_edge_length_m;
use super::super::board::types::ToolDraft2d;
use super::super::editing::{confirm_pending_sew, sew_lengths_equal, undo_redo_2d};
use super::super::input::handle_runner_event;
use super::super::state::CssRunnerState;
use clothcraft::fitting::FittingProject;
use clothcraft::garment::GarmentTemplate;
use clothcraft::garment::presets::GarmentKind;
use cssimpler::renderer::EngineEvent;

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

type EdgeLen = (String, String, f64);

/// One pair of boundaries on different panels whose lengths are equal (as the
/// seam compiler measures equality) and one pair that clearly differs.
fn sew_pairs(template: &GarmentTemplate) -> ((EdgeLen, EdgeLen), (EdgeLen, EdgeLen)) {
    let mut edges = Vec::new();
    for panel in &template.panels {
        if let Some(poly) = &panel.polygon {
            for edge in &poly.edges {
                if let Some(len) = panel_edge_length_m(panel, edge) {
                    edges.push((panel.id.clone(), edge.clone(), len));
                }
            }
        }
    }
    let mut equal = None;
    let mut unequal = None;
    for (i, x) in edges.iter().enumerate() {
        for y in &edges[i + 1..] {
            if x.0 == y.0 {
                continue;
            }
            if sew_lengths_equal(x.2, y.2) {
                equal.get_or_insert((x.clone(), y.clone()));
            } else if (x.2 - y.2).abs() > 1e-3 {
                unequal.get_or_insert((x.clone(), y.clone()));
            }
        }
    }
    (
        equal.expect("fixture has an equal-length pair"),
        unequal.expect("fixture has an unequal-length pair"),
    )
}

fn pending_sew(pair: &(EdgeLen, EdgeLen)) -> ToolDraft2d {
    ToolDraft2d::SewConfirm {
        a: (pair.0.0.clone(), pair.0.1.clone()),
        b: (pair.1.0.clone(), pair.1.1.clone()),
        reverse: false,
        a_len_m: pair.0.2,
        b_len_m: pair.1.2,
    }
}

#[test]
fn native_sewing_requires_an_explicit_easing_choice() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let avatar = dir.path().join("avatar.obj");
    std::fs::write(&avatar, "v 0 0 -10\nv 1 0 -10\nv 0 1 -10\nf 1 2 3\n").expect("write avatar");
    let mut project = FittingProject::basic(GarmentKind::Tshirt, 1, avatar);
    // Free every boundary so the fixture's own seams cannot overlap the test seams.
    project.template.seams.clear();
    let scene = project.compile().expect("compile scene");
    let files = DocumentFiles {
        output: dir.path().join("out"),
        source: None,
        modified: false,
    };
    let (tx, latest) = worker::start_cpu().expect("start cpu worker");
    let mut state = CssRunnerState::new(project, scene, files, tx, latest);
    let (equal, unequal) = sew_pairs(&state.project.template);

    // Unequal lengths: plain confirmation (button or Enter) is refused and
    // keeps the draft; nothing is committed.
    state.draft_2d = Some(pending_sew(&unequal));
    assert!(!confirm_pending_sew(&mut state, false));
    assert!(handle_runner_event(&mut state, &named_key("Enter")));
    assert!(state.project.template.seams.is_empty());
    assert!(matches!(
        state.draft_2d,
        Some(ToolDraft2d::SewConfirm { .. })
    ));
    assert!(state.status_message.contains("Sew with easing"));

    // Escape cancels: document and history unchanged.
    assert!(handle_runner_event(&mut state, &named_key("Escape")));
    assert!(state.draft_2d.is_none());
    assert!(state.project.template.seams.is_empty());
    assert!(
        !undo_redo_2d(&mut state, false),
        "cancel must not create history"
    );

    // Cancel button behaves the same way.
    state.draft_2d = Some(pending_sew(&unequal));
    apply_runner_ui_op(&mut state, RunnerUiOp::CancelSew);
    assert!(state.draft_2d.is_none());
    assert!(state.project.template.seams.is_empty());

    // Explicit easing commits one eased seam as one undo entry.
    state.draft_2d = Some(pending_sew(&unequal));
    apply_runner_ui_op(&mut state, RunnerUiOp::ConfirmSew { ease: true });
    assert_eq!(
        state.project.template.seams.len(),
        1,
        "{}",
        state.status_message
    );
    assert!(state.project.template.seams[0].allow_ease);
    assert!(state.status_message.contains("easing"));
    assert!(undo_redo_2d(&mut state, false));
    assert!(state.project.template.seams.is_empty());

    // Equal lengths: Enter sews without easing.
    state.draft_2d = Some(pending_sew(&equal));
    assert!(handle_runner_event(&mut state, &named_key("Enter")));
    assert_eq!(
        state.project.template.seams.len(),
        1,
        "{}",
        state.status_message
    );
    assert!(!state.project.template.seams[0].allow_ease);
    assert!(state.draft_2d.is_none());
}
