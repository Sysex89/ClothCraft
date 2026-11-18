//! Saving the native workspace document.

use super::super::super::{DocumentFiles, worker};
use super::super::state::CssRunnerState;
use clothcraft::fitting::FittingProject;
use clothcraft::garment::presets::GarmentKind;

fn save_test_state(dir: &std::path::Path, source: Option<std::path::PathBuf>) -> CssRunnerState {
    let avatar = dir.join("avatar.obj");
    std::fs::write(&avatar, "v 0 0 -10\nv 1 0 -10\nv 0 1 -10\nf 1 2 3\n").expect("write avatar");
    let project = FittingProject::basic(GarmentKind::Tshirt, 1, avatar);
    let scene = project.compile().expect("compile scene");
    let files = DocumentFiles {
        output: dir.join("out"),
        source,
        modified: false,
    };
    let (tx, latest) = worker::start_cpu().expect("start cpu worker");
    CssRunnerState::new(project, scene, files, tx, latest)
}

#[test]
fn save_writes_the_opened_project_and_reopens_it() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("garment.json");
    let mut state = save_test_state(dir.path(), Some(source.clone()));
    state.project.save(&source).expect("seed source document");
    state.project.template.panels[0].name = "Renamed by save test".into();

    assert_eq!(state.save_target(), (source.clone(), false));
    state.save_project();
    assert_eq!(state.status_message, format!("Saved {}", source.display()));
    assert!(
        !dir.path().join("out/fitting-project.json").exists(),
        "save must not fall back to the output directory when a source exists"
    );
    let reopened = FittingProject::load(&source).expect("reopen saved project");
    assert_eq!(reopened.template.panels[0].name, "Renamed by save test");
    assert_eq!(
        reopened.state.as_ref().map(|s| s.x.len()),
        Some(state.scene.state.x.len())
    );
    assert!(reopened.simulation.is_some());
    assert!(
        reopened.compile().is_ok(),
        "portable avatar asset must resolve"
    );
}

#[test]
fn save_without_a_source_reports_the_default_location() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let mut state = save_test_state(dir.path(), None);
    let expected = dir.path().join("out").join("fitting-project.json");
    assert_eq!(state.save_target(), (expected.clone(), true));
    state.save_project();
    assert!(expected.is_file());
    assert!(
        state.status_message.contains("default output location"),
        "{}",
        state.status_message
    );
    FittingProject::load(&expected).expect("reopen default save");
}

#[test]
fn rejected_save_leaves_the_previous_document_intact() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("garment.json");
    let mut state = save_test_state(dir.path(), Some(source.clone()));
    state.project.save(&source).expect("seed source document");
    let before = std::fs::read(&source).expect("read source");

    state.scene.state.x[0].x = f64::NAN;
    state.save_project();
    assert!(
        state.status_message.starts_with("Save error for ")
            && state.status_message.contains(&source.display().to_string()),
        "{}",
        state.status_message
    );
    assert_eq!(std::fs::read(&source).expect("reread source"), before);
}
