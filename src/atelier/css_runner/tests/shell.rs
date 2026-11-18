//! Shell layout, instance-local handlers and rendering regressions.

use super::super::super::{DocumentFiles, worker};
use super::super::actions::{RunnerAction, update_runner};
use super::super::app::create_runner_app;
use super::super::state::{CssRunnerState, ObjectCategory};
use clothcraft::css_ui::{Theme, ViewLayoutMode, load_studio_stylesheet};
use clothcraft::fitting::FittingProject;
use clothcraft::garment::presets::GarmentKind;
use clothcraft::gpu_avbd::ContactMethod;
use clothcraft::vbd::VbdMetrics;
use cssimpler::renderer::FrameInfo;
use std::time::Duration;

#[test]
fn test_css_runner_headless_frame_and_rendering() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let stylesheet = load_studio_stylesheet().expect("load studio stylesheet");
    let dir = tempfile::tempdir().expect("tempdir");
    let avatar = dir.path().join("avatar.obj");
    std::fs::write(&avatar, "v 0 0 -10\nv 1 0 -10\nv 0 1 -10\nf 1 2 3\n").expect("write avatar");
    let project = FittingProject::basic(GarmentKind::Tshirt, 1, avatar);
    let scene = project.compile().expect("compile scene");
    let files = DocumentFiles {
        output: dir.path().join("test_runner_out"),
        source: None,
        modified: false,
    };

    let (tx, latest) = worker::start_cpu().expect("start cpu worker");
    let state = CssRunnerState::new(project, scene, files, tx, latest);
    let mut app = create_runner_app(state, &stylesheet);
    app.set_viewport(cssimpler::renderer::ViewportSize::new(1600, 980));

    // Headless frame advance
    let scene = app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 0,
    });
    assert!(!scene.is_empty(), "runner scene should not be empty");

    let collider_points = app
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
    assert!(
        !collider_points.is_empty(),
        "collision geometry must be visible in USD"
    );
    // A click belongs to the instance that built the scene, even if another
    // instance advances first. An unrelated menu update must not reset the
    // switch's transition clock.
    let (other_tx, other_latest) = worker::start_cpu().expect("start second CPU worker");
    let other_files = DocumentFiles {
        output: dir.path().join("other"),
        source: None,
        modified: false,
    };
    let other = CssRunnerState::new(
        app.state().project.clone(),
        app.state().scene.clone(),
        other_files,
        other_tx,
        other_latest,
    );
    let mut other_app = create_runner_app(other, &stylesheet);
    other_app.set_viewport(cssimpler::renderer::ViewportSize::new(1600, 980));
    other_app.frame(FrameInfo {
        delta: Duration::ZERO,
        frame_index: 0,
    });
    let button = app.find_element_bounds_by_id("#theme-toggle").unwrap();
    assert!(cssimpler::renderer::dispatch_click(
        app.scene(),
        button.x + button.width / 2.0,
        button.y + button.height / 2.0
    ));
    other_app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 1,
    });
    assert_eq!(other_app.state().theme, Theme::Dark);
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 1,
    });
    assert_eq!(app.state().theme, Theme::Light);
    assert!(app.latest_stats().transition_active);
    let elapsed = app.latest_stats().transition_elapsed_us;
    let button = app
        .find_element_bounds_by_id("#sim-backend-indicator")
        .unwrap();
    assert!(cssimpler::renderer::dispatch_click(
        app.scene(),
        button.x + button.width / 2.0,
        button.y + button.height / 2.0
    ));
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 2,
    });
    assert!(app.state().solver_menu_open);
    assert!(app.latest_stats().transition_active);
    assert!(app.latest_stats().transition_elapsed_us > elapsed);
    let button = app
        .find_element_bounds_by_id("#sim-backend-indicator")
        .unwrap();
    assert!(cssimpler::renderer::dispatch_click(
        app.scene(),
        button.x + button.width / 2.0,
        button.y + button.height / 2.0
    ));
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 3,
    });
    assert!(!app.state().solver_menu_open);

    // Repeated worker results must not prevent either theme endpoint.
    let dark_x = other_app
        .find_element_bounds_by_id("#theme-switch-thumb")
        .unwrap()
        .x;
    app.state_mut().playing = true;
    for i in 0..40 {
        let state = app.state();
        let done = worker::Done {
            generation: state.generation,
            had_grab: false,
            result: Ok((
                state.scene.state.clone(),
                VbdMetrics {
                    step_ms: i as f64 * 0.1,
                    ..VbdMetrics::default()
                },
            )),
        };
        *state.worker_latest.lock().unwrap() = Some(done);
        app.frame(FrameInfo {
            delta: Duration::from_millis(16),
            frame_index: 100 + i,
        });
    }
    let light_x = app
        .find_element_bounds_by_id("#theme-switch-thumb")
        .unwrap()
        .x;
    assert!(
        (dark_x - light_x - 24.0).abs() < 0.1,
        "dark {dark_x}, light {light_x}"
    );
    let button = app.find_element_bounds_by_id("#theme-toggle").unwrap();
    assert!(cssimpler::renderer::dispatch_click(
        app.scene(),
        button.x + button.width / 2.0,
        button.y + button.height / 2.0
    ));
    for i in 0..40 {
        let state = app.state();
        let done = worker::Done {
            generation: state.generation,
            had_grab: false,
            result: Ok((
                state.scene.state.clone(),
                VbdMetrics {
                    step_ms: i as f64 * 0.1,
                    ..VbdMetrics::default()
                },
            )),
        };
        *state.worker_latest.lock().unwrap() = Some(done);
        app.frame(FrameInfo {
            delta: Duration::from_millis(16),
            frame_index: 200 + i,
        });
    }
    app.state_mut().playing = false;
    app.state_mut().step_count = 0;
    let final_x = app
        .find_element_bounds_by_id("#theme-switch-thumb")
        .unwrap()
        .x;
    assert!(
        (dark_x - final_x).abs() < 0.1,
        "dark {dark_x}, final {final_x}"
    );

    // Structural controls must update even while simulation is playing.
    app.state_mut().playing = true;
    let button = app
        .find_element_bounds_by_id("#sim-backend-indicator")
        .unwrap();
    assert!(cssimpler::renderer::dispatch_click(
        app.scene(),
        button.x + button.width / 2.0,
        button.y + button.height / 2.0
    ));
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 4,
    });
    assert!(app.state().solver_menu_open);
    assert!(app.find_element_bounds_by_id("#solver-dropdown").is_some());
    app.state_mut().playing = false;
    let button = app
        .find_element_bounds_by_id("#sim-backend-indicator")
        .unwrap();
    assert!(cssimpler::renderer::dispatch_click(
        app.scene(),
        button.x + button.width / 2.0,
        button.y + button.height / 2.0
    ));
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 5,
    });
    assert!(!app.state().solver_menu_open);

    // Verify layout containers
    assert!(app.find_element_bounds_by_id("#viewport-3d").is_some());
    let usd_bounds = app
        .find_element_bounds_by_id("#runner-viewport-3d-scene")
        .expect("USD viewport exists");
    assert!(
        usd_bounds.width > 100.0 && usd_bounds.height > 100.0,
        "USD viewport must have drawable area: {usd_bounds:?}"
    );
    assert!(app.find_element_bounds_by_id("#viewport-2d").is_some());
    assert!(app.find_element_bounds_by_id("#right-dock").is_some());

    // The menu is rendered in the live app; changing backend transfers the
    // displayed checkpoint without stepping or initializing a GPU.
    let checkpoint = app.state().scene.state.clone();
    let generation = app.state().generation;
    let button = app
        .find_element_bounds_by_id("#sim-backend-indicator")
        .expect("solver button");
    assert!(cssimpler::renderer::dispatch_click(
        app.scene(),
        button.x + button.width / 2.0,
        button.y + button.height / 2.0
    ));
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 10,
    });
    assert!(app.state().solver_menu_open);
    let menu = app
        .find_element_bounds_by_id("#solver-dropdown")
        .expect("open solver menu");
    assert!(menu.width > 100.0 && menu.height > 20.0);
    assert!(app.find_element_bounds_by_id("#solver-wgsl-ipc").is_some());
    let button = app
        .find_element_bounds_by_id("#solver-wgsl-ipc")
        .expect("solver button");
    assert!(cssimpler::renderer::dispatch_click(
        app.scene(),
        button.x + button.width / 2.0,
        button.y + button.height / 2.0
    ));
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 11,
    });
    assert!(app.state().gpu_sim);
    assert!(!app.state().playing && !app.state().solver_menu_open);
    assert_eq!(app.state().generation, generation + 1);
    assert_eq!(app.state().scene.state.x, checkpoint.x);
    assert_eq!(app.state().scene.state.v, checkpoint.v);
    assert_eq!(app.state().scene.state.time, checkpoint.time);
    let button = app
        .find_element_bounds_by_id("#sim-backend-indicator")
        .unwrap();
    assert!(cssimpler::renderer::dispatch_click(
        app.scene(),
        button.x + button.width / 2.0,
        button.y + button.height / 2.0
    ));
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 12,
    });
    let button = app.find_element_bounds_by_id("#solver-barrier-al").unwrap();
    assert!(cssimpler::renderer::dispatch_click(
        app.scene(),
        button.x + button.width / 2.0,
        button.y + button.height / 2.0
    ));
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 13,
    });
    assert_eq!(app.state().contact_method, ContactMethod::BarrierAl);
    assert_eq!(app.state().generation, generation + 2);
    assert_eq!(app.state().scene.state.x, checkpoint.x);
    let button = app
        .find_element_bounds_by_id("#sim-backend-indicator")
        .unwrap();
    assert!(cssimpler::renderer::dispatch_click(
        app.scene(),
        button.x + button.width / 2.0,
        button.y + button.height / 2.0
    ));
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 12,
    });
    let button = app.find_element_bounds_by_id("#solver-cpu").unwrap();
    assert!(cssimpler::renderer::dispatch_click(
        app.scene(),
        button.x + button.width / 2.0,
        button.y + button.height / 2.0
    ));
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 12,
    });
    assert!(!app.state().gpu_sim);
    assert_eq!(app.state().generation, generation + 3);
    assert_eq!(app.state().scene.state.x, checkpoint.x);

    // Headless RGBA8 rendering
    let width = 640;
    let height = 480;
    let mut rgba = vec![0_u8; width * height * 4];
    app.render_rgba8(
        FrameInfo {
            delta: Duration::from_millis(16),
            frame_index: 1,
        },
        &mut rgba,
        width,
        height,
        1.0,
        cssimpler::core::Color::rgba(0, 0, 0, 0),
    );
    assert!(
        rgba.iter().any(|&b| b > 0),
        "rendered buffer should have painted pixels"
    );

    // Deliver an accepted worker snapshot without running GPU work.
    let read_points = |state: &CssRunnerState| {
        let path = clothcraft::usd_scene::viewport_panel_path(&state.scene.garment.panels[0].id);
        state
            .usd_scene
            .as_ref()
            .unwrap()
            .stage()
            .attribute(format!("{path}.points"))
            .unwrap()
            .get::<Vec<cssimpler::renderer::gpu::openusd::gf::Vec3f>>()
            .unwrap()
            .unwrap()
    };
    let initial_points = read_points(app.state());
    let mut moved = app.state().scene.state.clone();
    moved.x[0].y += 0.01;
    *app.state().worker_latest.lock().unwrap() = Some(worker::Done {
        generation: app.state().generation,
        had_grab: false,
        result: Ok((moved, VbdMetrics::default())),
    });
    update_runner(
        app.state_mut(),
        FrameInfo {
            delta: Duration::from_millis(16),
            frame_index: 2,
        },
    );
    assert_ne!(read_points(app.state()), initial_points);
    // Save the visible accepted checkpoint, not the launch-time project cache.
    app.state_mut().save_project();
    let saved =
        FittingProject::load(&app.state().files.output.join("fitting-project.json")).unwrap();
    let checkpoint = saved.state.unwrap();
    assert_eq!(checkpoint.x, app.state().scene.state.x);
    assert_eq!(checkpoint.v, app.state().scene.state.v);
    assert_eq!(checkpoint.time, app.state().scene.state.time);
    assert_eq!(saved.simulation.unwrap().dt, app.state().settings.dt);

    app.set_viewport(cssimpler::renderer::ViewportSize::new(1600, 980));
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 3,
    });
    let bounds = app
        .find_element_bounds_by_id("#runner-viewport-3d-scene")
        .unwrap();
    assert!(
        bounds.width > 100.0
            && bounds.height > 100.0
            && bounds.x >= 0.0
            && bounds.x + bounds.width <= 1600.0
            && bounds.y + bounds.height <= 980.0,
        "live USD host must remain inside the window: {bounds:?}"
    );

    app.state_mut().reset_simulation();
    assert_eq!(read_points(app.state()), initial_points);

    // Native USD hosts are hidden if later text paint overlaps their area.
    // Check expanded diagnostics as well as Ready, across window sizes.
    for (width, height) in [(1600, 980), (1280, 800), (1024, 768), (800, 600)] {
        app.set_viewport(cssimpler::renderer::ViewportSize::new(width, height));
        for message in ["Ready", "Step 124: min dist 0.124 mm, res 0.0e0, 65.2 ms"] {
            app.state_mut().status_message = message.into();
            let roots = app.frame(FrameInfo {
                delta: Duration::from_millis(16),
                frame_index: 4,
            });
            let viewport = app
                .find_element_bounds_by_id("#runner-viewport-3d-scene")
                .unwrap();
            let paint = cssimpler::core::ExtractedScene::from_render_roots(&roots);
            for item in paint
                .items
                .iter()
                .filter(|item| item.kind == cssimpler::core::ExtractedPaintKind::TextRun)
            {
                let text = item.layout;
                let overlaps = text.x < viewport.x + viewport.width
                    && viewport.x < text.x + text.width
                    && text.y < viewport.y + viewport.height
                    && viewport.y < text.y + text.height;
                assert!(
                    !overlaps,
                    "text must not hide USD at {width}x{height}: {:?} {:?} over {viewport:?}",
                    item.text, text
                );
            }
        }
    }

    // Test Play / Step
    app.state_mut().playing = true;
    app.state_mut().trigger_worker_step(true);
    assert!(app.state().worker_pending);

    // Test Reset
    app.state_mut().reset_simulation();
    assert!(!app.state().playing);
    assert_eq!(app.state().step_count, 0);

    // Test Category Tab Switching
    assert_eq!(app.state().active_category, ObjectCategory::Pattern2D);
    app.state_mut().active_category = ObjectCategory::Sewing;
    assert_eq!(app.state().active_category, ObjectCategory::Sewing);

    // Test Tool Switching
    app.state_mut().active_2d_tool = "Corner".to_string();
    assert_eq!(app.state().active_2d_tool, "Corner");
}

#[test]
fn test_all_runner_buttons_click_and_layout_bounds() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let stylesheet = load_studio_stylesheet().expect("load studio stylesheet");
    let dir = tempfile::tempdir().expect("tempdir");
    let avatar = dir.path().join("avatar.obj");
    std::fs::write(&avatar, "v 0 0 -10\nv 1 0 -10\nv 0 1 -10\nf 1 2 3\n").expect("write avatar");
    let project = FittingProject::basic(GarmentKind::Tshirt, 0, avatar);
    let scene = project.compile().expect("compile scene");
    let files = DocumentFiles {
        output: dir.path().join("out_buttons"),
        source: None,
        modified: false,
    };
    let (tx, latest) = worker::start_cpu().expect("start cpu worker");
    let state = CssRunnerState::new(project, scene, files, tx, latest);
    let mut app = create_runner_app(state, &stylesheet);

    fn check_container_buttons(
        roots: &[cssimpler::renderer::RenderNode],
        container_id: &str,
        win_w: f32,
        win_h: f32,
    ) {
        fn find_node<'a>(
            nodes: &'a [cssimpler::renderer::RenderNode],
            id: &str,
        ) -> Option<&'a cssimpler::renderer::RenderNode> {
            for n in nodes {
                if n.id() == Some(id) {
                    return Some(n);
                }
                if let Some(found) = find_node(&n.children, id) {
                    return Some(found);
                }
            }
            None
        }
        fn collect_clickable(
            node: &cssimpler::renderer::RenderNode,
            out: &mut Vec<cssimpler::core::LayoutBox>,
        ) {
            if node.handlers.click().is_some() {
                out.push(node.layout);
            }
            for child in &node.children {
                collect_clickable(child, out);
            }
        }
        let root = find_node(roots, container_id)
            .unwrap_or_else(|| panic!("missing container #{container_id}"));
        let sew_button = match container_id {
            "rail-2d" => Some("tool-sew-2d"),
            "rail-3d" => Some("tool-sew-3d"),
            _ => None,
        };
        if let Some(id) = sew_button {
            let button = find_node(&root.children, id).expect("missing Sew button");
            assert!(
                button.children.iter().any(|child| {
                    matches!(&child.kind, cssimpler::core::RenderKind::Svg(scene)
                        if scene.paths.len() == 2 && scene.bounds.is_some())
                }),
                "expected visible edge and stitch paths inside #{id}"
            );
        }
        let mut btns = Vec::new();
        collect_clickable(root, &mut btns);
        assert!(
            !btns.is_empty(),
            "expected clickable controls inside #{container_id}"
        );
        for (i, b) in btns.iter().enumerate() {
            assert!(
                b.width >= 10.0
                    && b.height >= 10.0
                    && b.x >= -1.0
                    && b.y >= -1.0
                    && b.x + b.width <= win_w + 1.0
                    && b.y + b.height <= win_h + 1.0,
                "button #{i} in #{container_id} out of bounds at {win_w}x{win_h}: {b:?}"
            );
        }
    }

    // 1. Audit layout bounds across 1600x980 and 1280x800
    for (w, h) in [(1600_usize, 980_usize), (1280_usize, 800_usize)] {
        app.state_mut().window_size = (w as f32, h as f32);
        app.set_viewport(cssimpler::renderer::ViewportSize::new(w, h));
        let roots = app.frame(FrameInfo {
            delta: Duration::from_millis(16),
            frame_index: w as u64,
        });
        for cid in [
            "top-bar",
            "rail-3d",
            "rail-2d",
            "left-browser",
            "right-dock",
        ] {
            check_container_buttons(&roots, cid, w as f32, h as f32);
        }
    }

    // 2. Click buttons across #rail-3d, #rail-2d, #right-dock, and #command-palette-trigger
    app.state_mut().window_size = (1600.0, 980.0);
    app.set_viewport(cssimpler::renderer::ViewportSize::new(1600, 980));
    let mut frame = 100_u64;
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: frame,
    });

    // Verify clicking all 2D tools via actions
    for (act, expected) in [
        (RunnerAction::ToolRect2d, "Rect"),
        (RunnerAction::ToolPen2d, "Pen"),
        (RunnerAction::ToolFree2d, "Freehand"),
        (RunnerAction::ToolSew2d, "Sew"),
        (RunnerAction::ToolSplit2d, "Split"),
        (RunnerAction::ToolInternal2d, "Internal Line"),
        (RunnerAction::ToolMeasure2d, "Measure"),
        (RunnerAction::ToolTrim2d, "Trim"),
        (RunnerAction::ToolCornerSelect2d, "Corner Select"),
        (RunnerAction::ToolCorner2d, "Corner"),
        (RunnerAction::ToolEdit2d, "Edit"),
    ] {
        frame += 1;
        app.state_mut().actions.push(act);
        app.frame(FrameInfo {
            delta: Duration::from_millis(16),
            frame_index: frame,
        });
        assert_eq!(app.state().active_2d_tool, expected);
    }

    // Verify clicking all 3D tools via actions
    for (act, expected) in [
        (RunnerAction::ToolGrab, "Grab"),
        (RunnerAction::ToolSew3d, "3D Sew"),
        (RunnerAction::ToolPin, "Pin"),
        (RunnerAction::ToolMeasure3d, "Measure"),
        (RunnerAction::ToolAvatarDiag, "Avatar Diagnostics"),
        (RunnerAction::ToolGizmo, "Gizmo"),
    ] {
        frame += 1;
        app.state_mut().actions.push(act);
        app.frame(FrameInfo {
            delta: Duration::from_millis(16),
            frame_index: frame,
        });
        assert_eq!(app.state().active_3d_tool, expected);
    }

    // Verify clicking #command-palette-trigger opens palette and closing works
    let pal_btn = app
        .find_element_bounds_by_id("#command-palette-trigger")
        .unwrap();
    assert!(cssimpler::renderer::dispatch_click(
        app.scene(),
        pal_btn.x + pal_btn.width * 0.5,
        pal_btn.y + pal_btn.height * 0.5
    ));
    frame += 1;
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: frame,
    });
    assert!(app.state().command_palette_open);
    let pal_box = app
        .find_element_bounds_by_id("#command-palette-container")
        .unwrap();
    assert!(pal_box.width > 100.0 && pal_box.height > 100.0);

    // Toggle palette back off
    frame += 1;
    app.state_mut().actions.push(RunnerAction::PaletteToggle);
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: frame,
    });
    assert!(!app.state().command_palette_open);

    // Clean view + layout modes + drawer toggle roundtrip
    for (act, expected_mode) in [
        (RunnerAction::View3d, ViewLayoutMode::ThreeDOnly),
        (RunnerAction::View2d, ViewLayoutMode::TwoDOnly),
        (RunnerAction::ViewSplit, ViewLayoutMode::Split),
    ] {
        frame += 1;
        app.state_mut().actions.push(act);
        app.frame(FrameInfo {
            delta: Duration::from_millis(16),
            frame_index: frame,
        });
        assert_eq!(app.state().layout_mode, expected_mode);
    }
}

#[test]
fn viewport_shading_toggle_rewrites_usd_normals() {
    use cssimpler::renderer::gpu::openusd::gf;
    use cssimpler::renderer::gpu::openusd_schemas::geom::AttributeMetadata;

    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let stylesheet = load_studio_stylesheet().expect("load studio stylesheet");
    let dir = tempfile::tempdir().expect("tempdir");
    let avatar = dir.path().join("avatar.obj");
    std::fs::write(&avatar, "v 0 0 -10\nv 1 0 -10\nv 0 1 -10\nf 1 2 3\n").expect("write avatar");
    let project = FittingProject::basic(GarmentKind::Tshirt, 1, avatar);
    let scene = project.compile().expect("compile scene");
    let files = DocumentFiles {
        output: dir.path().join("shading_out"),
        source: None,
        modified: false,
    };
    let (tx, latest) = worker::start_cpu().expect("start cpu worker");
    let state = CssRunnerState::new(project, scene, files, tx, latest);
    let mut app = create_runner_app(state, &stylesheet);
    app.set_viewport(cssimpler::renderer::ViewportSize::new(1600, 980));
    app.frame(FrameInfo {
        delta: Duration::ZERO,
        frame_index: 0,
    });

    let panel_path =
        clothcraft::usd_scene::viewport_panel_path(&app.state().scene.garment.panels[0].id);
    let read = |state: &CssRunnerState| {
        let attribute = state
            .usd_scene
            .as_ref()
            .unwrap()
            .stage()
            .attribute(format!("{panel_path}.normals"))
            .unwrap();
        let interpolation = attribute.interpolation().unwrap().unwrap();
        let values = attribute.get::<Vec<gf::Vec3f>>().unwrap().unwrap();
        (interpolation, values)
    };

    assert_eq!(
        app.state().shading,
        clothcraft::usd_scene::ShadingMode::Smooth
    );
    let (interpolation, smooth) = read(app.state());
    assert_eq!(interpolation.as_str(), "vertex");
    assert_eq!(
        smooth.len(),
        app.state().scene.garment.panels[0].vertices.len()
    );

    let flat_button = app.find_element_bounds_by_id("#shading-flat").unwrap();
    assert!(cssimpler::renderer::dispatch_click(
        app.scene(),
        flat_button.x + flat_button.width * 0.5,
        flat_button.y + flat_button.height * 0.5
    ));
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 1,
    });
    assert_eq!(
        app.state().shading,
        clothcraft::usd_scene::ShadingMode::Flat
    );
    let (interpolation, flat) = read(app.state());
    assert_eq!(interpolation.as_str(), "faceVarying");
    assert_eq!(
        flat.len(),
        app.state().scene.garment.panels[0].triangles.len() * 3
    );
    assert!(
        flat.as_chunks::<3>()
            .0
            .iter()
            .all(|corner| corner[0] == corner[1] && corner[1] == corner[2])
    );

    let smooth_button = app.find_element_bounds_by_id("#shading-smooth").unwrap();
    assert!(cssimpler::renderer::dispatch_click(
        app.scene(),
        smooth_button.x + smooth_button.width * 0.5,
        smooth_button.y + smooth_button.height * 0.5
    ));
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 2,
    });
    assert_eq!(
        app.state().shading,
        clothcraft::usd_scene::ShadingMode::Smooth
    );
    let (interpolation, roundtrip) = read(app.state());
    assert_eq!(interpolation.as_str(), "vertex");
    assert_eq!(roundtrip, smooth);

    // A moved vertex must refresh the authored normals, not leave launch-time values.
    let panel = &app.state().scene.garment.panels[0];
    let vertex = app.state().scene.garment.mesh.triangles[panel.triangles.start].vertices[0];
    let direction = smooth[0];
    app.state_mut().scene.state.x[vertex] += glam::DVec3::new(
        f64::from(direction.x),
        f64::from(direction.y),
        f64::from(direction.z),
    ) * 0.05;
    app.state_mut().sync_usd_points();
    let (interpolation, moved) = read(app.state());
    assert_eq!(interpolation.as_str(), "vertex");
    assert_ne!(moved, smooth);
}
