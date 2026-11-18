//! Left browser quick actions.

use super::super::super::{DocumentFiles, worker};
use super::super::state::CssRunnerState;
use super::super::ui::asset_browser::{build_runner_left_browser, import_quick_actions};
use clothcraft::fitting::FittingProject;
use clothcraft::garment::presets::GarmentKind;
use cssimpler::core::Node;

fn disabled_tooltips(node: &Node, out: &mut Vec<String>) {
    if let Node::Element(el) = node {
        if el.classes.iter().any(|c| c == "btn-action")
            && el.classes.iter().any(|c| c == "disabled")
        {
            out.push(el.attribute("title").unwrap_or_default().to_string());
        }
        for child in &el.children {
            disabled_tooltips(child, out);
        }
    }
}

#[test]
fn import_quick_actions_are_disabled_and_name_working_alternatives() {
    let actions = import_quick_actions();
    assert_eq!(
        actions.iter().map(|a| a.label).collect::<Vec<_>>(),
        ["DXF", "SVG", "Avatar"]
    );
    for action in &actions {
        assert!(
            !action.tooltip.to_lowercase().contains("file menu"),
            "{} tooltip refers to a nonexistent File menu",
            action.label
        );
        assert!(
            action
                .tooltip
                .to_lowercase()
                .contains("not available in this window yet")
        );
    }
    assert!(actions[2].tooltip.contains("--avatar"));

    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let avatar = dir.path().join("avatar.obj");
    std::fs::write(&avatar, "v 0 0 -10\nv 1 0 -10\nv 0 1 -10\nf 1 2 3\n").expect("write avatar");
    let project = FittingProject::basic(GarmentKind::Tshirt, 1, avatar);
    let scene = project.compile().expect("compile scene");
    let files = DocumentFiles {
        output: dir.path().join("out"),
        source: None,
        modified: false,
    };
    let (tx, latest) = worker::start_cpu().expect("start cpu worker");
    let state = CssRunnerState::new(project, scene, files, tx, latest);
    let browser = build_runner_left_browser(&state);
    assert!(
        !format!("{browser:?}").to_lowercase().contains("file menu"),
        "rendered browser refers to a nonexistent File menu"
    );
    let mut tooltips = Vec::new();
    disabled_tooltips(&browser, &mut tooltips);
    assert_eq!(
        tooltips,
        actions
            .iter()
            .map(|a| a.tooltip.to_string())
            .collect::<Vec<_>>()
    );
}
