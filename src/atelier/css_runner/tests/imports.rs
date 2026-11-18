//! Reviewed DXF, SVG and avatar import regressions.

use super::super::super::{DocumentFiles, worker};
use super::super::app::create_runner_app;
use super::super::imports::ImportKind;
use super::super::input::handle_runner_event;
use super::super::state::CssRunnerState;
use super::support::pointer_btn_ev;
use clothcraft::css_ui::load_studio_stylesheet;
use clothcraft::fitting::FittingProject;
use clothcraft::garment::presets::GarmentKind;
use cssimpler::renderer::{EngineEvent, FrameInfo};
use std::time::Duration;

fn state_with(dir: &std::path::Path) -> CssRunnerState {
    let avatar = dir.join("avatar.obj");
    std::fs::write(&avatar, "v 0 0 -10\nv 1 0 -10\nv 0 1 -10\nf 1 2 3\n").unwrap();
    let project = FittingProject::basic(GarmentKind::Tshirt, 0, avatar);
    let scene = project.compile().unwrap();
    let files = DocumentFiles {
        output: dir.join("out"),
        source: None,
        modified: false,
    };
    let (tx, latest) = worker::start_cpu().unwrap();
    CssRunnerState::new(project, scene, files, tx, latest)
}

fn dxf_document(entities: &str) -> String {
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    )
}

fn dxf_rectangle(x: f64, y: f64, w: f64, h: f64, layer: &str) -> String {
    format!(
        "0\nLWPOLYLINE\n8\n{layer}\n90\n4\n70\n1\n10\n{x}\n20\n{y}\n10\n{x}\n20\n{}\n10\n{}\n20\n{}\n10\n{}\n20\n{y}\n",
        y + h,
        x + w,
        y + h,
        x + w
    )
}

fn write_dxf(dir: &std::path::Path, entities: &str) -> std::path::PathBuf {
    let path = dir.join("patterns.dxf");
    std::fs::write(&path, dxf_document(entities)).unwrap();
    path
}

#[test]
fn dxf_review_imports_selected_outlines_as_one_undoable_edit() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut state = state_with(dir.path());
    let path = write_dxf(
        dir.path(),
        &format!(
            "{}{}",
            dxf_rectangle(0., 0., 100., 200., "CUT front"),
            dxf_rectangle(300., 0., 80., 120., "CUT back")
        ),
    );
    let panels_before = state.project.template.panels.len();
    state.open_import_path(ImportKind::Dxf, &path);
    {
        let dialog = state.import_dialog.as_ref().expect("review dialog");
        assert_eq!(dialog.rows.len(), 2);
        assert!(dialog.rows.iter().all(|row| row.issue.is_none()));
        assert!(dialog.current(state.import_tolerance_mm, state.import_scale_mm_per_unit));
        assert!(!dialog.fabric.is_empty());
        assert!(
            state
                .project
                .template
                .fabrics
                .iter()
                .any(|f| f.id == dialog.fabric)
        );
    }
    state.toggle_import_outline(0);
    state.toggle_import_outline(1);
    state.toggle_import_acknowledged();
    state.apply_import();
    assert!(state.import_dialog.is_none());
    assert_eq!(state.project.template.panels.len(), panels_before + 2);
    assert!(state.files.modified);
    assert!(state.history.can_undo());
    assert!(state.status_message.starts_with("Imported 2 DXF"));
    for panel in &state.project.template.panels[panels_before..] {
        assert!(panel.polygon.is_some());
        assert_eq!(panel.grain_degrees, 90.0);
    }
    // Undo restores the original document in one step.
    state
        .history
        .restore(&state.project.template, false)
        .expect("undo imported panels");
}

#[test]
fn dxf_stale_parameters_and_ambiguous_selection_do_not_mutate() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut state = state_with(dir.path());
    let path = write_dxf(
        dir.path(),
        &format!(
            "{}{}",
            dxf_rectangle(0., 0., 100., 200., "OUTER"),
            dxf_rectangle(25., 25., 50., 50., "INNER")
        ),
    );
    let before = serde_json::to_value(&state.project.template).unwrap();
    state.open_import_path(ImportKind::Dxf, &path);
    state.toggle_import_outline(0);
    state.toggle_import_outline(1);
    state.toggle_import_acknowledged();

    // A changed tolerance invalidates the reviewed drawing until Update preview.
    state.import_tolerance_mm = 0.25;
    state.apply_import();
    let dialog = state.import_dialog.as_ref().expect("dialog stays open");
    assert!(
        dialog
            .error
            .as_deref()
            .is_some_and(|error| error.contains("Review the current parameters")),
        "{:?}",
        dialog.error
    );
    assert_eq!(
        serde_json::to_value(&state.project.template).unwrap(),
        before
    );

    state.refresh_import();
    state.toggle_import_outline(0);
    state.toggle_import_outline(1);
    state.toggle_import_acknowledged();
    state.apply_import();
    let dialog = state.import_dialog.as_ref().expect("ambiguous stays open");
    assert!(
        dialog
            .error
            .as_deref()
            .is_some_and(|error| error.contains("nested") || error.contains("overlap")),
        "{:?}",
        dialog.error
    );
    assert_eq!(
        serde_json::to_value(&state.project.template).unwrap(),
        before
    );
    assert!(!state.files.modified);
}

#[test]
fn dxf_open_paths_are_reported_and_cannot_be_selected() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut state = state_with(dir.path());
    let open =
        "0\nLWPOLYLINE\n8\nCUT\n90\n3\n70\n0\n10\n0\n20\n0\n10\n100\n20\n0\n10\n0\n20\n100\n";
    let path = write_dxf(dir.path(), open);
    state.open_import_path(ImportKind::Dxf, &path);
    let dialog = state.import_dialog.as_ref().unwrap();
    assert_eq!(dialog.rows.len(), 1);
    assert!(dialog.rows[0].issue.is_some());
    state.toggle_import_outline(0);
    assert_eq!(state.import_dialog.as_ref().unwrap().selected_count(), 0);
    // An out-of-range selection is ignored rather than panicking.
    state.toggle_import_outline(99);
    assert_eq!(state.import_dialog.as_ref().unwrap().selected_count(), 0);
}

#[test]
fn svg_import_appends_physical_outlines() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut state = state_with(dir.path());
    let path = dir.path().join("patterns.svg");
    std::fs::write(
        &path,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="100mm">
            <rect x="0" y="0" width="80mm" height="60mm"/>
            <rect x="100mm" y="0" width="80mm" height="60mm"/>
        </svg>"#,
    )
    .unwrap();
    let panels_before = state.project.template.panels.len();
    state.open_import_path(ImportKind::Svg, &path);
    {
        let dialog = state.import_dialog.as_ref().unwrap();
        assert_eq!(dialog.rows.len(), 2);
        assert!(dialog.rows.iter().all(|row| row.issue.is_none()));
    }
    state.toggle_import_outline(0);
    state.toggle_import_outline(1);
    state.toggle_import_acknowledged();
    state.apply_import();
    assert!(state.import_dialog.is_none(), "{:?}", state.status_message);
    assert_eq!(state.project.template.panels.len(), panels_before + 2);
    let ids: Vec<_> = state.project.template.panels[panels_before..]
        .iter()
        .map(|panel| panel.id.as_str())
        .collect();
    assert_eq!(ids, vec!["svg_1", "svg_2"]);
    assert!(
        state
            .project
            .template
            .panels
            .iter()
            .any(|panel| (panel.pattern.width - 0.08).abs() < 1e-9)
    );
}

#[test]
fn avatar_import_replaces_static_geometry_and_discards_history() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut state = state_with(dir.path());
    state.history.record(state.project.template.clone());
    let generation = state.generation;
    let path = dir.path().join("new.obj");
    std::fs::write(&path, "v 2 0 -10\nv 3 0 -10\nv 2 1 -10\nf 1 2 3\n").unwrap();

    state.open_import_path(ImportKind::Avatar, &path);
    {
        let dialog = state.import_dialog.as_ref().unwrap();
        let candidate = dialog.candidate.as_ref().expect("avatar candidate");
        assert_eq!((candidate.vertices, candidate.triangles), (3, 1));
        assert!(candidate.warnings.iter().any(|w| w.contains("metres")));
    }
    state.apply_import();
    assert!(state.import_dialog.is_none());
    assert_ne!(state.generation, generation);
    assert!(state.staged_avatar.is_some());
    assert!(state.project.avatar_path.exists());
    assert!(!state.history.can_undo());
    assert!(state.files.modified);
    assert_eq!(state.scene.collider.surface.vertices[0].x, 2.0);
    assert_eq!(state.scene.state.time, 0.0);
    assert!(state.status_message.starts_with("Replaced avatar"));
}

#[allow(clippy::type_complexity)]
fn app_with_dialog(
    path: &std::path::Path,
    kind: ImportKind,
) -> (
    cssimpler::app::App<
        'static,
        CssRunnerState,
        fn(&mut CssRunnerState, FrameInfo) -> cssimpler::app::Invalidation,
        fn(&CssRunnerState) -> cssimpler::core::Node,
        cssimpler::app::Invalidation,
    >,
    tempfile::TempDir,
) {
    let stylesheet = Box::leak(Box::new(
        load_studio_stylesheet().expect("load studio stylesheet"),
    ));
    let dir = tempfile::tempdir().unwrap();
    let mut state = state_with(dir.path());
    state.open_import_path(kind, path);
    let mut app = create_runner_app(state, stylesheet);
    app.set_viewport(cssimpler::renderer::ViewportSize::new(1600, 980));
    app.frame(FrameInfo {
        delta: Duration::ZERO,
        frame_index: 0,
    });
    (app, dir)
}

#[test]
fn import_modal_renders_preview_selection_and_apply_controls() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let path = write_dxf(
        dir.path(),
        &format!(
            "{}{}",
            dxf_rectangle(0., 0., 100., 200., "CUT front"),
            dxf_rectangle(300., 0., 80., 120., "CUT back")
        ),
    );
    let (mut app, _dir) = app_with_dialog(&path, ImportKind::Dxf);
    assert!(app.find_element_bounds_by_id("#import-dialog").is_some());
    assert!(
        app.find_element_bounds_by_id("#import-preview").is_some(),
        "outline preview must render"
    );
    assert!(app.find_element_bounds_by_id("#import-row-0").is_some());
    assert!(app.find_element_bounds_by_id("#import-row-1").is_some());
    assert!(app.find_element_bounds_by_id("#import-ack").is_some());
    // Disabled until the reviewer selects and acknowledges.
    assert!(app.find_element_bounds_by_id("#import-apply").is_some());

    // Drive the row and acknowledgement callbacks through the native DOM.
    let mut frame = 1;
    for id in ["#import-row-0", "#import-row-1", "#import-ack"] {
        let bounds = app.find_element_bounds_by_id(id).unwrap();
        assert!(
            cssimpler::renderer::dispatch_click(
                app.scene(),
                bounds.x + bounds.width * 0.5,
                bounds.y + bounds.height * 0.5
            ),
            "click {id}"
        );
        app.frame(FrameInfo {
            delta: Duration::from_millis(16),
            frame_index: frame,
        });
        frame += 1;
    }
    assert_eq!(
        app.state().import_dialog.as_ref().unwrap().selected_count(),
        2
    );
    assert!(app.state().import_dialog.as_ref().unwrap().acknowledged);
    let panels_before = app.state().project.template.panels.len();
    let apply = app.find_element_bounds_by_id("#import-apply").unwrap();
    assert!(cssimpler::renderer::dispatch_click(
        app.scene(),
        apply.x + apply.width * 0.5,
        apply.y + apply.height * 0.5
    ));
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: frame,
    });
    assert_eq!(app.state().project.template.panels.len(), panels_before + 2);
    assert!(app.state().import_dialog.is_none());
    assert!(app.find_element_bounds_by_id("#import-dialog").is_none());
}

#[test]
fn escape_and_backdrop_close_the_import_modal_without_mutating() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let path = write_dxf(dir.path(), &dxf_rectangle(0., 0., 100., 200., "CUT"));
    let (mut app, _dir) = app_with_dialog(&path, ImportKind::Dxf);
    let before = serde_json::to_value(&app.state().project.template).unwrap();
    let escape = EngineEvent::Key(cssimpler::renderer::KeyboardEvent {
        logical_key: cssimpler::renderer::KeyIdentity::Named("Escape".into()),
        physical_key: None,
        location: cssimpler::renderer::KeyLocation::Standard,
        state: cssimpler::renderer::ButtonState::Pressed,
        repeat: false,
        modifiers: Default::default(),
    });
    assert!(handle_runner_event(app.state_mut(), &escape));
    assert!(app.state().import_dialog.is_none());
    assert_eq!(
        serde_json::to_value(&app.state().project.template).unwrap(),
        before
    );
    // A later primary click still reaches the workspace normally.
    assert!(!handle_runner_event(app.state_mut(), &pointer_btn_ev(true)));

    // The backdrop also dismisses without mutating the document.
    app.state_mut().open_import_path(ImportKind::Dxf, &path);
    app.state_mut().input_dirty = true;
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 3,
    });
    assert!(app.find_element_bounds_by_id("#import-dialog").is_some());
    assert!(cssimpler::renderer::dispatch_click(app.scene(), 5.0, 5.0));
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 4,
    });
    assert!(app.state().import_dialog.is_none());
    assert_eq!(
        serde_json::to_value(&app.state().project.template).unwrap(),
        before
    );
}

#[test]
fn avatar_modal_apply_replaces_the_usd_collider() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new avatar.obj");
    std::fs::write(&path, "v 2 0 -10\nv 3 0 -10\nv 2 1 -10\nf 1 2 3\n").unwrap();
    let (mut app, _dir) = app_with_dialog(&path, ImportKind::Avatar);
    assert!(app.find_element_bounds_by_id("#import-dialog").is_some());
    assert!(
        app.find_element_bounds_by_id("#import-avatar-size")
            .is_some()
    );
    let generation = app.state().generation;
    let apply = app.find_element_bounds_by_id("#import-apply").unwrap();
    assert!(cssimpler::renderer::dispatch_click(
        app.scene(),
        apply.x + apply.width * 0.5,
        apply.y + apply.height * 0.5
    ));
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 1,
    });
    assert!(app.state().import_dialog.is_none());
    assert_ne!(app.state().generation, generation);
    assert!(app.state().staged_avatar.is_some());
    assert_eq!(app.state().scene.collider.surface.vertices[0].x, 2.0);
    assert!(app.state().status_message.starts_with("Replaced avatar"));

    #[cfg(feature = "desktop")]
    {
        let points = app
            .state()
            .usd_scene
            .as_ref()
            .unwrap()
            .stage()
            .attribute("/Collider.points")
            .unwrap()
            .get::<Vec<cssimpler::renderer::gpu::openusd::gf::Vec3f>>()
            .unwrap()
            .unwrap();
        assert_eq!(points.len(), 3);
        assert_eq!(points[0].x, 2.0);
    }
}

#[test]
fn browser_import_buttons_are_wired_to_native_actions() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let stylesheet = Box::leak(Box::new(
        load_studio_stylesheet().expect("load studio stylesheet"),
    ));
    let dir = tempfile::tempdir().unwrap();
    let state = state_with(dir.path());
    let mut app = create_runner_app(state, stylesheet);
    app.set_viewport(cssimpler::renderer::ViewportSize::new(1600, 980));
    app.frame(FrameInfo {
        delta: Duration::ZERO,
        frame_index: 0,
    });
    fn find<'a>(
        nodes: &'a [cssimpler::renderer::RenderNode],
        id: &str,
    ) -> Option<&'a cssimpler::renderer::RenderNode> {
        for node in nodes {
            if node.id() == Some(id) {
                return Some(node);
            }
            if let Some(found) = find(&node.children, id) {
                return Some(found);
            }
        }
        None
    }
    for id in ["import-dxf", "import-svg", "import-avatar"] {
        let node = find(app.scene(), id).unwrap_or_else(|| panic!("missing #{id}"));
        assert!(
            node.handlers.click().is_some(),
            "#{id} must route to its import action"
        );
    }
    // No stale "use the CLI" import note remains in the rendered browser.
    let paint = cssimpler::core::ExtractedScene::from_render_roots(app.scene());
    for item in &paint.items {
        if let cssimpler::core::ExtractedPaintKind::TextRun = item.kind
            && let Some(text) = &item.text
        {
            assert!(
                !text.contains("Use CLI"),
                "stale import helper text: {text}"
            );
        }
    }
}
