//! Board tools, gestures, cancellation and history regressions.

use super::super::super::{DocumentFiles, worker};
use super::super::actions::{RunnerAction, RunnerUiOp, apply_runner_action, apply_runner_ui_op};
use super::super::app::create_runner_app;
use super::super::board::corners::active_fillet_previews;
use super::super::board::geometry::{
    compute_2d_view_box_m, compute_2d_viewport_rect, panel_raw_points, panel_to_board_pt,
};
use super::super::board::interaction::set_active_2d_tool;
use super::super::board::native::BoardPath;
use super::super::board::rendering::render_runner_2d_paths;
use super::super::board::selection::compute_connected_2d_highlight;
use super::super::board::types::{Hover2dTarget, ToolDraft2d};
use super::super::editing::{
    round_selected_corner, sew_lengths_equal, toggle_selected_curve, undo_redo_2d,
};
use super::super::input::handle_runner_event;
use super::super::state::CssRunnerState;
use super::super::ui::shell::build_runner_ui;
use super::support::{
    board_to_screen_px, count_class, pointer_btn_ev, pointer_btn_mod_ev, pointer_move_ev,
    selection_test_state,
};
use clothcraft::authoring::{self, Command, CornerRule};
use clothcraft::css_ui::icons::{icon_corner_2d, icon_corner_select_2d};
use clothcraft::css_ui::{ViewLayoutMode, load_studio_stylesheet};
use clothcraft::fitting::FittingProject;
use clothcraft::garment::GarmentTemplate;
use clothcraft::garment::presets::GarmentKind;
use cssimpler::core::Node;
use cssimpler::renderer::{EngineEvent, FrameInfo};
use glam::DVec2;
use std::time::Duration;

#[test]
fn test_2d_native_full_canvas_bounds_and_cad_bar() {
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
        output: dir.path().join("out_full_canvas"),
        source: None,
        modified: false,
    };
    let (tx, latest) = worker::start_cpu().expect("start cpu worker");
    let state = CssRunnerState::new(project, scene, files, tx, latest);
    let mut app = create_runner_app(state, &stylesheet);

    let cad_control_ids = [
        "#cad-undo-btn",
        "#cad-redo-btn",
        "#cad-corner-radius-mode",
        "#cad-corner-shape-mode",
        "#cad-radius-dec",
        "#cad-radius-inc",
        "#cad-tension-dec",
        "#cad-tension-inc",
        "#cad-apply-round-btn",
        "#cad-rule-sharp",
        "#cad-rule-smooth",
        "#cad-rule-sym",
        "#cad-curve-on-btn",
        "#cad-curve-off-btn",
    ];

    for (frame_idx, (w, h, mode, view_act, clean_view, drawer_open)) in [
        (
            1600,
            980,
            ViewLayoutMode::Split,
            RunnerAction::ViewSplit,
            false,
            true,
        ),
        (
            1600,
            980,
            ViewLayoutMode::Split,
            RunnerAction::ViewSplit,
            true,
            false,
        ),
        (
            1600,
            980,
            ViewLayoutMode::TwoDOnly,
            RunnerAction::View2d,
            false,
            true,
        ),
        (
            1280,
            800,
            ViewLayoutMode::Split,
            RunnerAction::ViewSplit,
            false,
            true,
        ),
        (
            1024,
            768,
            ViewLayoutMode::Split,
            RunnerAction::ViewSplit,
            false,
            true,
        ),
        (
            1024,
            768,
            ViewLayoutMode::Split,
            RunnerAction::ViewSplit,
            false,
            false,
        ),
        (
            1024,
            768,
            ViewLayoutMode::TwoDOnly,
            RunnerAction::View2d,
            false,
            true,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        app.state_mut().window_size = (w as f32, h as f32);
        app.state_mut().clean_view = clean_view;
        app.state_mut().left_drawer_open = drawer_open;
        app.state_mut().actions.push(view_act);
        app.set_viewport(cssimpler::renderer::ViewportSize::new(w, h));
        app.frame(FrameInfo {
            delta: Duration::from_millis(16),
            frame_index: frame_idx as u64 + 1,
        });
        assert_eq!(app.state().layout_mode, mode);
        let vp2d = app
            .find_element_bounds_by_id("#viewport-2d")
            .expect("#viewport-2d bounds");
        let svg_bounds = app
            .find_element_bounds_by_id("#board-2d")
            .expect("#board-2d bounds");
        let expected_svg = compute_2d_viewport_rect(app.state());
        assert!(
            (svg_bounds.x - expected_svg.x).abs() < 3.0
                && (svg_bounds.y - expected_svg.y).abs() < 3.0
                && (svg_bounds.width - expected_svg.w).abs() < 3.0
                && (svg_bounds.height - expected_svg.h).abs() < 3.0,
            "CSS layout ({svg_bounds:?}) must align with hit-test rect ({expected_svg:?}) at {w}x{h}"
        );
        let bar_bounds = app
            .find_element_bounds_by_id("#cad-2d-bar")
            .expect("#cad-2d-bar bounds");
        assert!(bar_bounds.width > 50.0 && bar_bounds.height >= 24.0);
        assert!(
            bar_bounds.x >= vp2d.x - 1.0
                && bar_bounds.x + bar_bounds.width <= vp2d.x + vp2d.width + 1.0
                && bar_bounds.y >= vp2d.y - 1.0
                && bar_bounds.y + bar_bounds.height <= vp2d.y + vp2d.height + 1.0,
            "#cad-2d-bar must fit inside #viewport-2d at {w}x{h} (clean={clean_view}, drawer={drawer_open}): bar={bar_bounds:?} vp2d={vp2d:?}"
        );
        for cid in cad_control_ids {
            let b = app
                .find_element_bounds_by_id(cid)
                .unwrap_or_else(|| panic!("missing {cid} at {w}x{h}"));
            assert!(
                b.width > 0.0
                    && b.height > 0.0
                    && b.x >= vp2d.x - 1.0
                    && b.x + b.width <= vp2d.x + vp2d.width + 1.0
                    && b.y >= vp2d.y - 1.0
                    && b.y + b.height <= vp2d.y + vp2d.height + 1.0,
                "control {cid} unclipped inside #viewport-2d at {w}x{h} (clean={clean_view}, drawer={drawer_open}): {b:?} vs {vp2d:?}"
            );
        }
    }

    // Verify clicking #cad-2d-bar controls actually dispatches UI ops & mutates state
    app.state_mut().window_size = (1280.0, 800.0);
    app.state_mut().clean_view = false;
    app.state_mut().left_drawer_open = true;
    app.state_mut().actions.push(RunnerAction::ViewSplit);
    app.set_viewport(cssimpler::renderer::ViewportSize::new(1280, 800));
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 20,
    });

    for (idx, selector) in ["#cad-corner-shape-mode", "#cad-radius-inc", "#cad-rule-sym"]
        .into_iter()
        .enumerate()
    {
        let btn = app.find_element_bounds_by_id(selector).unwrap();
        assert!(
            cssimpler::renderer::dispatch_click(
                app.scene(),
                btn.x + btn.width * 0.5,
                btn.y + btn.height * 0.5,
            ),
            "click on {selector} must hit"
        );
        app.frame(FrameInfo {
            delta: Duration::from_millis(16),
            frame_index: 21 + idx as u64,
        });
    }
    assert!(!app.state().corner_mode_radius);
    assert!((app.state().corner_radius_mm - 12.0).abs() < 1e-6);
    assert_eq!(app.state().corner_rule, CornerRule::Symmetric);
}

#[test]
fn test_2d_canvas_click_corner_round_bezier_drag_and_undo() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let avatar = dir.path().join("avatar.obj");
    std::fs::write(&avatar, "v 0 0 -10\nv 1 0 -10\nv 0 1 -10\nf 1 2 3\n").expect("write avatar");
    let project = FittingProject::basic(GarmentKind::Tshirt, 0, avatar);
    let scene = project.compile().expect("compile scene");
    let files = DocumentFiles {
        output: dir.path().join("out_2d_edit"),
        source: None,
        modified: false,
    };
    let (tx, latest) = worker::start_cpu().expect("start cpu worker");
    let mut state = CssRunnerState::new(project, scene, files, tx, latest);
    assert_eq!(state.selected_piece_index, 0);

    fn count_class(nodes: &[BoardPath], target: &str) -> usize {
        nodes
            .iter()
            .filter(|p| p.class.split_whitespace().any(|c| c == target))
            .count()
    }

    // 1. Click interior of panel 1 ("back") -> switches selection to panel 1
    let back_center = DVec2::new(0.5, 0.35);
    let back_board = panel_to_board_pt(&state.project.template, 1, back_center);
    let (bx, by) = board_to_screen_px(&state, back_board);
    handle_runner_event(&mut state, &pointer_move_ev(bx, by));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    assert_eq!(state.selected_piece_index, 1);

    // 2. Click interior of panel 0 ("front") -> switches back to panel 0
    let front_center = DVec2::new(0.5, 0.35);
    let front_board = panel_to_board_pt(&state.project.template, 0, front_center);
    let (fx, fy) = board_to_screen_px(&state, front_board);
    handle_runner_event(&mut state, &pointer_move_ev(fx, fy));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    assert_eq!(state.selected_piece_index, 0);

    // 3. Click point #1 ("neck-bottom") -> selects corner sub-element
    let neck_pt = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .points[1];
    let neck_board = panel_to_board_pt(&state.project.template, 0, neck_pt);
    let (nx, ny) = board_to_screen_px(&state, neck_board);
    handle_runner_event(&mut state, &pointer_move_ev(nx, ny));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    assert_eq!(state.selected_2d_point, Some(("front".into(), 1)));

    // 4. Switch to Corner tool and click neck-bottom corner to round it
    state.corner_radius_mm = 35.0;
    state.active_2d_tool = "Corner".into();
    handle_runner_event(&mut state, &pointer_move_ev(nx, ny));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));

    let rounded_poly = state.project.template.panels[0].polygon.as_ref().unwrap();
    let curve_name = rounded_poly
        .curves
        .keys()
        .find(|k| k.starts_with("corner_round_"))
        .expect("Corner tool click must produce a corner_round_* Bézier curve")
        .clone();
    let initial_c1 = rounded_poly.curves[&curve_name][0];

    // 5. Render SVG and verify Bézier curve and handle elements appear
    let (svg_nodes, _) = render_runner_2d_paths(&state);
    assert!(count_class(&svg_nodes, "bezier-handle") >= 2);
    assert!(count_class(&svg_nodes, "cad-vertex") >= 10);

    // 6. Drag Bézier handle 0 of the rounded corner and release -> commits AdjustCornerHandle
    state.active_2d_tool = "Edit".into();
    let h0_board = panel_to_board_pt(&state.project.template, 0, initial_c1);
    let (hx, hy) = board_to_screen_px(&state, h0_board);
    handle_runner_event(&mut state, &pointer_move_ev(hx, hy));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
    handle_runner_event(&mut state, &pointer_move_ev(hx + 3.0, hy - 1.5));
    assert!(
        state.preview_template.is_some(),
        "live preview template must update while dragging Bézier handle"
    );
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    let updated_c1 = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .curves[&curve_name][0];
    assert!(
        updated_c1.distance(initial_c1) > 1e-4,
        "dragging Bézier handle must modify control point (initial={initial_c1:?}, updated={updated_c1:?}, status={:?})",
        state.status_message
    );

    // 7. Undo restores pre-drag handle and pre-round corner; Redo brings them back
    assert!(undo_redo_2d(&mut state, false));
    let undone_c1 = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .curves[&curve_name][0];
    assert!(undone_c1.distance(initial_c1) < 1e-9);
    assert!(undo_redo_2d(&mut state, false));
    assert!(
        state.project.template.panels[0]
            .polygon
            .as_ref()
            .unwrap()
            .curves
            .is_empty()
    );
    assert!(undo_redo_2d(&mut state, true));
    assert!(
        !state.project.template.panels[0]
            .polygon
            .as_ref()
            .unwrap()
            .curves
            .is_empty()
    );
    assert!(undo_redo_2d(&mut state, false));

    // 8. Click sewn corner #3 ("shoulder-right") -> highlights connected panel "back" + weld-link
    let sh_pt = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .points[3];
    let sh_board = panel_to_board_pt(&state.project.template, 0, sh_pt);
    let (sx, sy) = board_to_screen_px(&state, sh_board);
    handle_runner_event(&mut state, &pointer_move_ev(sx, sy));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    assert_eq!(state.selected_2d_point, Some(("front".into(), 3)));
    let conn = compute_connected_2d_highlight(&state);
    assert!(conn.connected_panels.contains("back"));
    assert!(conn.selected_corners.contains(&("back".into(), 3)));
    let (sewn_svg, _) = render_runner_2d_paths(&state);
    assert!(count_class(&sewn_svg, "connected") >= 1);
    assert!(count_class(&sewn_svg, "weld-link") >= 1);

    // 9. Select double-sewn corner #4 ("sleeve-top-right") and apply Round via UI op
    let c4_idx = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .edges
        .iter()
        .position(|e| e == "sleeve-top-right")
        .unwrap();
    let c4_pt = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .points[c4_idx];
    let (c4x, c4y) =
        board_to_screen_px(&state, panel_to_board_pt(&state.project.template, 0, c4_pt));
    handle_runner_event(&mut state, &pointer_move_ev(c4x, c4y));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    state.corner_radius_mm = 8.0;
    assert!(round_selected_corner(&mut state));
    assert!(
        state.project.template.panels[0]
            .polygon
            .as_ref()
            .unwrap()
            .curves
            .keys()
            .any(|k| k.starts_with("corner_round_"))
    );
    assert!(
        state.project.template.panels[1]
            .polygon
            .as_ref()
            .unwrap()
            .curves
            .keys()
            .any(|k| k.starts_with("corner_round_"))
    );
    assert!(
        state
            .project
            .template
            .seams
            .iter()
            .any(|s| s.id.contains("corner_round_"))
    );
    state.project.template.compile().unwrap();
    assert!(undo_redo_2d(&mut state, false));

    // 10. Toggle CurvePoint on sewn corner #3 and drag Bézier handle -> mirrors live & on commit
    state.selected_piece_index = 0;
    state.selected_2d_point = Some(("front".into(), 3));
    assert!(toggle_selected_curve(&mut state, true));
    assert!(
        state.project.template.panels[1]
            .polygon
            .as_ref()
            .unwrap()
            .curves
            .contains_key("shoulder-right")
    );
    let front_sh_c0 = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .curves["shoulder-right"][0];
    let back_sh_c0_before = state.project.template.panels[1]
        .polygon
        .as_ref()
        .unwrap()
        .curves["shoulder-right"][0];
    let (sh_hx, sh_hy) = board_to_screen_px(
        &state,
        panel_to_board_pt(&state.project.template, 0, front_sh_c0),
    );
    state.active_2d_tool = "Edit".into();
    handle_runner_event(&mut state, &pointer_move_ev(sh_hx, sh_hy));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
    handle_runner_event(&mut state, &pointer_move_ev(sh_hx + 3.0, sh_hy + 4.0));
    let preview = state
        .preview_template
        .as_ref()
        .expect("live preview must update across sewn Bezier handle drag");
    let back_preview_c0 = preview.panels[1].polygon.as_ref().unwrap().curves["shoulder-right"][0];
    assert!(back_preview_c0.distance(back_sh_c0_before) > 1e-4);
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    let back_sh_c0_after = state.project.template.panels[1]
        .polygon
        .as_ref()
        .unwrap()
        .curves["shoulder-right"][0];
    assert!(back_sh_c0_after.distance(back_sh_c0_before) > 1e-4);
    state.project.template.compile().unwrap();

    // 11. Drag sewn corner #3 in Edit tool -> keeps front and back shoulder seams equal length
    let pt3_before = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .points[3];
    let (p3x, p3y) = board_to_screen_px(
        &state,
        panel_to_board_pt(&state.project.template, 0, pt3_before),
    );
    handle_runner_event(&mut state, &pointer_move_ev(p3x, p3y));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
    handle_runner_event(&mut state, &pointer_move_ev(p3x + 5.0, p3y + 4.0));
    assert!(state.preview_template.is_some());
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    let fp_after = state.project.template.panels[0].polygon.as_ref().unwrap();
    let bp_after = state.project.template.panels[1].polygon.as_ref().unwrap();
    assert!(fp_after.points[3].distance(pt3_before) > 1e-4);
    assert!(
        (authoring::edge_length(fp_after, "shoulder-right").unwrap()
            - authoring::edge_length(bp_after, "shoulder-right").unwrap())
        .abs()
            < 1e-9
    );
    state.project.template.compile().unwrap();

    // 12. Undo/Redo restores/reapplies corner drag atomically on both panels
    assert!(undo_redo_2d(&mut state, false));
    assert!(
        state.project.template.panels[0]
            .polygon
            .as_ref()
            .unwrap()
            .points[3]
            .distance(pt3_before)
            < 1e-9
    );
    assert!(undo_redo_2d(&mut state, true));
    assert!(
        state.project.template.panels[0]
            .polygon
            .as_ref()
            .unwrap()
            .points[3]
            .distance(pt3_before)
            > 1e-4
    );
}

#[test]
fn test_2d_offboard_drag_and_mixed_mirror_seam_propagation() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let avatar = dir.path().join("avatar.obj");
    std::fs::write(&avatar, "v 0 0 -10\nv 1 0 -10\nv 0 1 -10\nf 1 2 3\n").expect("write avatar");
    let mut project = FittingProject::basic(GarmentKind::Tshirt, 0, avatar);

    // Build mixed topology: front <--(LinkedMirror X)--> mirror_panel, and front.shoulder-right <--(Seam)--> back.shoulder-right
    let front_id = project.template.panels[0].id.clone();
    let back_id = project.template.panels[1].id.clone();
    project.template = authoring::apply(
        &project.template,
        Command::LinkedMirror {
            panel: front_id.clone(),
            axis: clothcraft::garment::shapes::MirrorAxis::X,
        },
    )
    .unwrap()
    .template;
    let mirror_idx = project
        .template
        .panels
        .iter()
        .position(|p| p.id != front_id && p.id != back_id)
        .expect("linked mirror panel");
    let mirror_id = project.template.panels[mirror_idx].id.clone();

    let scene = project.compile().expect("compile scene");
    let files = DocumentFiles {
        output: dir.path().join("out_mixed"),
        source: None,
        modified: false,
    };
    let (tx, latest) = worker::start_cpu().expect("start cpu worker");
    let mut state = CssRunnerState::new(project, scene, files, tx, latest);
    state.active_2d_tool = "Edit".into();

    // Pick shoulder-right corner (index 3) on front panel, which is BOTH sewn to back AND mirror-linked to mirror_panel
    let sh_idx = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .edges
        .iter()
        .position(|e| e == "shoulder-right")
        .unwrap();
    let fp_pt_before = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .points[sh_idx];
    let bp_pt_before = state.project.template.panels[1]
        .polygon
        .as_ref()
        .unwrap()
        .points[sh_idx];
    let mp_pt_before = state.project.template.panels[mirror_idx]
        .polygon
        .as_ref()
        .unwrap()
        .points[sh_idx];

    let (sx, sy) = board_to_screen_px(
        &state,
        panel_to_board_pt(&state.project.template, 0, fp_pt_before),
    );
    let canvas_2d = compute_2d_viewport_rect(&state);
    let off_x = sx + 22.0;
    let off_y = (canvas_2d.y - 45.0).min(-20.0);
    assert!(!canvas_2d.contains(off_x, off_y));

    handle_runner_event(&mut state, &pointer_move_ev(sx, sy));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));

    // Drag completely outside the 2D SVG canvas bounds (off-board above canvas_2d)
    handle_runner_event(&mut state, &pointer_move_ev(off_x, off_y));
    assert!(
        state.preview_template.is_some(),
        "off-board pointer drag must keep updating live preview"
    );
    let prev_bp = state.preview_template.as_ref().unwrap().panels[1]
        .polygon
        .as_ref()
        .unwrap()
        .points[sh_idx];
    let prev_mp = state.preview_template.as_ref().unwrap().panels[mirror_idx]
        .polygon
        .as_ref()
        .unwrap()
        .points[sh_idx];
    assert!(prev_bp.distance(bp_pt_before) > 1e-4);
    assert!(prev_mp.distance(mp_pt_before) > 1e-4);

    // Release off-board -> must commit to front, back (sewn), and mirror_panel (mirror-linked)
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    let fp_after = state.project.template.panels[0].polygon.as_ref().unwrap();
    let bp_after = state.project.template.panels[1].polygon.as_ref().unwrap();
    let mp_after = state.project.template.panels[mirror_idx]
        .polygon
        .as_ref()
        .unwrap();
    assert!(fp_after.points[sh_idx].distance(fp_pt_before) > 1e-2);
    assert!(bp_after.points[sh_idx].distance(bp_pt_before) > 1e-4);
    assert!(mp_after.points[sh_idx].distance(mp_pt_before) > 1e-4);
    assert!(
        (authoring::edge_length(fp_after, "shoulder-right").unwrap()
            - authoring::edge_length(bp_after, "shoulder-right").unwrap())
        .abs()
            < 1e-9
    );

    // Undo restores cleanly across all 3 connected panels, then test CurvePoint + off-board Bezier handle drag
    assert!(undo_redo_2d(&mut state, false));
    state.selected_piece_index = 0;
    state.selected_2d_point = Some((front_id.clone(), sh_idx));
    assert!(toggle_selected_curve(&mut state, true));
    let h0 = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .curves["shoulder-right"][0];
    let mp_h1_before = state.project.template.panels[mirror_idx]
        .polygon
        .as_ref()
        .unwrap()
        .curves["shoulder-right"][1];
    let bp_h0_before = state.project.template.panels[1]
        .polygon
        .as_ref()
        .unwrap()
        .curves["shoulder-right"][0];
    let (hx, hy) = board_to_screen_px(&state, panel_to_board_pt(&state.project.template, 0, h0));
    let off_hy = (canvas_2d.y - 10.0).min(hy - 35.0);
    assert!(!canvas_2d.contains(hx + 12.0, off_hy));
    handle_runner_event(&mut state, &pointer_move_ev(hx, hy));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
    handle_runner_event(&mut state, &pointer_move_ev(hx + 12.0, off_hy));
    assert!(state.preview_template.is_some());
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    let mp_h1_after = state.project.template.panels[mirror_idx]
        .polygon
        .as_ref()
        .unwrap()
        .curves["shoulder-right"][1];
    let bp_h0_after = state.project.template.panels[1]
        .polygon
        .as_ref()
        .unwrap()
        .curves["shoulder-right"][0];
    assert!(mp_h1_after.distance(mp_h1_before) > 1e-3);
    assert!(bp_h0_after.distance(bp_h0_before) > 1e-3);
    assert!(!mirror_id.is_empty());
}

#[test]
fn test_2d_interactive_rail_tools_workflow() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let avatar = dir.path().join("avatar.obj");
    std::fs::write(&avatar, "v 0 0 -10\nv 1 0 -10\nv 0 1 -10\nf 1 2 3\n").expect("write avatar");
    let project = FittingProject::basic(GarmentKind::Tshirt, 0, avatar);
    let scene = project.compile().expect("compile scene");
    let files = DocumentFiles {
        output: dir.path().join("out_tools"),
        source: None,
        modified: false,
    };
    let (tx, latest) = worker::start_cpu().expect("start cpu worker");
    let mut state = CssRunnerState::new(project, scene, files, tx, latest);
    let initial_panels = state.project.template.panels.len();

    fn count_class(nodes: &[BoardPath], target: &str) -> usize {
        nodes
            .iter()
            .filter(|p| p.class.split_whitespace().any(|c| c == target))
            .count()
    }

    // 1. Rect tool: drag empty board space to create a rectangle panel
    set_active_2d_tool(&mut state, "Rect");
    let (rx0, ry0) = board_to_screen_px(&state, DVec2::new(2.2, 0.2));
    let (rx1, ry1) = board_to_screen_px(&state, DVec2::new(2.45, 0.45));
    handle_runner_event(&mut state, &pointer_move_ev(rx0, ry0));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
    handle_runner_event(&mut state, &pointer_move_ev(rx1, ry1));
    let (svg, _) = render_runner_2d_paths(&state);
    assert!(count_class(&svg, "tool-draft-rect") >= 1);
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    assert_eq!(state.project.template.panels.len(), initial_panels + 1);

    // 2. Pen tool: click 4 points + Enter -> creates polygon panel
    set_active_2d_tool(&mut state, "Pen");
    for pt in [
        DVec2::new(2.2, -0.6),
        DVec2::new(2.45, -0.6),
        DVec2::new(2.45, -0.35),
        DVec2::new(2.2, -0.35),
    ] {
        let (px, py) = board_to_screen_px(&state, pt);
        handle_runner_event(&mut state, &pointer_move_ev(px, py));
        assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
        assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    }
    let (svg_pen, _) = render_runner_2d_paths(&state);
    assert!(count_class(&svg_pen, "tool-draft-pen") >= 1);
    assert!(handle_runner_event(
        &mut state,
        &EngineEvent::Key(cssimpler::renderer::KeyboardEvent {
            logical_key: cssimpler::renderer::KeyIdentity::Named("Enter".into()),
            physical_key: None,
            location: cssimpler::renderer::KeyLocation::Standard,
            state: cssimpler::renderer::ButtonState::Pressed,
            repeat: false,
            modifiers: cssimpler::renderer::KeyboardModifiers::default(),
        })
    ));
    assert_eq!(state.project.template.panels.len(), initial_panels + 2);

    // 3. Freehand tool: stroke loop + release -> creates fitted panel
    set_active_2d_tool(&mut state, "Freehand");
    let loop_pts = [
        DVec2::new(0.15, 0.85),
        DVec2::new(0.45, 0.85),
        DVec2::new(0.45, 1.15),
        DVec2::new(0.15, 1.15),
    ];
    let (fx0, fy0) = board_to_screen_px(&state, loop_pts[0]);
    handle_runner_event(&mut state, &pointer_move_ev(fx0, fy0));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
    for pt in &loop_pts[1..] {
        let (fx, fy) = board_to_screen_px(&state, *pt);
        handle_runner_event(&mut state, &pointer_move_ev(fx, fy));
    }
    let (svg_free, _) = render_runner_2d_paths(&state);
    assert!(count_class(&svg_free, "tool-draft-freehand") >= 1);
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    assert_eq!(state.project.template.panels.len(), initial_panels + 3);

    // 4. Internal Line tool on panel 0 + Enter -> adds internal line
    set_active_2d_tool(&mut state, "Internal Line");
    for local_pt in [DVec2::new(0.45, 0.25), DVec2::new(0.55, 0.45)] {
        let b = panel_to_board_pt(&state.project.template, 0, local_pt);
        let (lx, ly) = board_to_screen_px(&state, b);
        handle_runner_event(&mut state, &pointer_move_ev(lx, ly));
        assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
        assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    }
    let (svg_int, _) = render_runner_2d_paths(&state);
    assert!(count_class(&svg_int, "tool-draft-internal") >= 1);
    assert!(handle_runner_event(
        &mut state,
        &EngineEvent::Key(cssimpler::renderer::KeyboardEvent {
            logical_key: cssimpler::renderer::KeyIdentity::Named("Enter".into()),
            physical_key: None,
            location: cssimpler::renderer::KeyLocation::Standard,
            state: cssimpler::renderer::ButtonState::Pressed,
            repeat: false,
            modifiers: cssimpler::renderer::KeyboardModifiers::default(),
        })
    ));
    assert_eq!(
        state.project.template.panels[0]
            .polygon
            .as_ref()
            .unwrap()
            .lines
            .len(),
        1
    );

    // 5. Measure tool: two boundary-anchored clicks on panel 0 -> saves measurement & shows overlay
    set_active_2d_tool(&mut state, "Measure");
    let p0 = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .points[0];
    let p2 = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .points[2];
    let (m0x, m0y) = board_to_screen_px(&state, panel_to_board_pt(&state.project.template, 0, p0));
    let (m1x, m1y) = board_to_screen_px(&state, panel_to_board_pt(&state.project.template, 0, p2));
    handle_runner_event(&mut state, &pointer_move_ev(m0x, m0y));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    handle_runner_event(&mut state, &pointer_move_ev(m1x, m1y));
    let (svg_msr, _) = render_runner_2d_paths(&state);
    assert!(count_class(&svg_msr, "tool-draft-measure") >= 1);
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    assert!(state.status_message.contains("mm"));

    // 6. Sew tool: click first edge on rect panel and second edge on pen panel -> pending
    //    confirmation; the seam exists only after an explicit confirm.
    let seams_before = state.project.template.seams.len();
    set_active_2d_tool(&mut state, "Sew");
    let p_rect = state.project.template.panels[initial_panels]
        .polygon
        .as_ref()
        .unwrap();
    let e_rect_mid = p_rect.points[1].lerp(p_rect.points[2], 0.5);
    let (srx, sry) = board_to_screen_px(
        &state,
        panel_to_board_pt(&state.project.template, initial_panels, e_rect_mid),
    );
    handle_runner_event(&mut state, &pointer_move_ev(srx, sry));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    let (svg_sew, _) = render_runner_2d_paths(&state);
    assert!(count_class(&svg_sew, "tool-draft-sew") >= 1);

    let p_pen = state.project.template.panels[initial_panels + 1]
        .polygon
        .as_ref()
        .unwrap();
    let e_pen_mid = p_pen.points[3].lerp(p_pen.points[0], 0.5);
    let (spx, spy) = board_to_screen_px(
        &state,
        panel_to_board_pt(&state.project.template, initial_panels + 1, e_pen_mid),
    );
    handle_runner_event(&mut state, &pointer_move_ev(spx, spy));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    assert_eq!(state.project.template.seams.len(), seams_before);
    let Some(ToolDraft2d::SewConfirm {
        a_len_m, b_len_m, ..
    }) = state.draft_2d.clone()
    else {
        panic!("second pick must wait for confirmation");
    };
    let (svg_sew, _) = render_runner_2d_paths(&state);
    assert!(count_class(&svg_sew, "tool-draft-sew") >= 2);
    apply_runner_ui_op(
        &mut state,
        RunnerUiOp::ConfirmSew {
            ease: !sew_lengths_equal(a_len_m, b_len_m),
        },
    );
    assert_eq!(state.project.template.seams.len(), seams_before + 1);
    assert!(!state.project.template.seams.last().unwrap().reverse_b);

    // 7. Trim tool: click internal line on panel 0 -> removes it
    set_active_2d_tool(&mut state, "Trim");
    let mid_line = panel_to_board_pt(&state.project.template, 0, DVec2::new(0.5, 0.35));
    let (tx, ty) = board_to_screen_px(&state, mid_line);
    handle_runner_event(&mut state, &pointer_move_ev(tx, ty));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    assert!(
        state.project.template.panels[0]
            .polygon
            .as_ref()
            .unwrap()
            .lines
            .is_empty()
    );
}

#[test]
fn sewing_dart_legs_pairs_the_tip_and_closes_the_mouth_in_either_click_order() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut state = selection_test_state(dir.path());
    // Start the contour at the shared tip, so the two legs straddle its wrap.
    let points = vec![
        DVec2::new(0.15, 0.12),
        DVec2::new(0.20, 0.00),
        DVec2::new(0.30, 0.00),
        DVec2::new(0.30, 0.25),
        DVec2::new(0.00, 0.25),
        DVec2::ZERO,
        DVec2::new(0.10, 0.00),
    ];
    let fabric = state.project.template.panels[0].fabric.clone();
    assert!(super::super::editing::commit_authoring_command(
        &mut state,
        Command::Polygon {
            points: points.clone(),
            tangents: vec![],
            meshing: Default::default(),
            fabric,
            replace: true,
        },
        "Created open dart",
    ));
    set_active_2d_tool(&mut state, "Sew");
    for order in [[6, 0], [0, 6]] {
        for edge in order {
            let mid = points[edge].midpoint(points[(edge + 1) % points.len()]);
            let board = panel_to_board_pt(&state.project.template, 0, mid);
            let (x, y) = board_to_screen_px(&state, board);
            handle_runner_event(&mut state, &pointer_move_ev(x, y));
            assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
            assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
        }
        // Equal legs: an explicit plain confirmation keeps the picked direction.
        assert!(state.scene.garment.seams.is_empty());
        assert!(super::super::editing::confirm_pending_sew(
            &mut state, false
        ));
        let garment = &state.scene.garment;
        let seam = &garment.seams[0];
        // In the open dart the largest gap is the 100 mm mouth. Folding
        // symmetric stations together preserves both 130 mm legs exactly.
        assert!((seam.max_gap(&garment.initial_state.x) - 0.10).abs() < 1e-10);
        let folded = garment
            .mesh
            .uv
            .iter()
            .map(|p| glam::DVec3::new((p.x - 0.15).abs(), p.y, 0.0))
            .collect::<Vec<_>>();
        assert!(seam.max_gap(&folded) < 1e-10);
        assert!(undo_redo_2d(&mut state, false));
        assert!(state.scene.garment.seams.is_empty());
    }
}

#[test]
fn test_selection_double_click_splits_curved_edge_and_undoes() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut state = selection_test_state(dir.path());
    let before = state.project.template.clone();
    // The rightmost panel's second edge defines the right edge of the view.
    let panel_idx = 1;
    let edge_idx = 1;
    let poly = before.panels[panel_idx].polygon.as_ref().unwrap();
    let sampled = poly.edge_points(edge_idx).unwrap();
    let local = sampled[sampled.len() / 2];
    let anchor = panel_to_board_pt(&before, panel_idx, poly.points[0]);
    let anchor_px = board_to_screen_px(&state, anchor);
    let pos = board_to_screen_px(&state, panel_to_board_pt(&before, panel_idx, local));
    handle_runner_event(&mut state, &pointer_move_ev(pos.0, pos.1));
    assert_eq!(
        state.hovered_2d,
        Some(Hover2dTarget::Edge {
            panel_idx,
            edge_idx
        })
    );
    handle_runner_event(&mut state, &pointer_btn_ev(true));
    handle_runner_event(&mut state, &pointer_btn_ev(false));
    assert!(
        !state.files.modified,
        "A single click must only select the edge"
    );
    handle_runner_event(&mut state, &pointer_btn_ev(true));
    handle_runner_event(&mut state, &pointer_btn_ev(false));
    let split = state.project.template.panels[panel_idx]
        .polygon
        .as_ref()
        .unwrap();
    assert_eq!(split.points.len(), poly.points.len() + 1);
    assert!(
        panel_to_board_pt(
            &state.project.template,
            panel_idx,
            split.points[edge_idx + 1]
        )
        .distance(panel_to_board_pt(&before, panel_idx, local))
            < 0.001
    );
    assert_eq!(board_to_screen_px(&state, anchor), anchor_px);
    assert!(undo_redo_2d(&mut state, false));
    assert_eq!(board_to_screen_px(&state, anchor), anchor_px);
    assert_eq!(
        serde_json::to_value(&state.project.template).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    assert!(undo_redo_2d(&mut state, true));
    assert_eq!(board_to_screen_px(&state, anchor), anchor_px);
    let roundtrip: GarmentTemplate =
        serde_json::from_value(serde_json::to_value(&state.project.template).unwrap()).unwrap();
    assert!(roundtrip.compile().is_ok());
    assert_eq!(
        state.project.template.panels[panel_idx]
            .polygon
            .as_ref()
            .unwrap()
            .points
            .len(),
        poly.points.len() + 1
    );
}

#[test]
fn dragging_an_outer_anchor_keeps_the_board_camera_fixed() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut state = selection_test_state(dir.path());
    let before = state.project.template.clone();
    let panel_idx = 1;
    let anchor = 1;
    let original = before.panels[panel_idx].polygon.as_ref().unwrap();
    let from = panel_to_board_pt(&before, panel_idx, original.points[anchor]);
    let to = from + DVec2::new(0.08, 0.);
    let start = board_to_screen_px(&state, from);
    let end = board_to_screen_px(&state, to);
    let initial_view = compute_2d_view_box_m(&state);

    handle_runner_event(&mut state, &pointer_move_ev(start.0, start.1));
    handle_runner_event(&mut state, &pointer_btn_ev(true));
    handle_runner_event(&mut state, &pointer_move_ev(end.0, end.1));

    let preview = state.preview_template.as_ref().expect("point drag preview");
    assert_eq!(compute_2d_view_box_m(&state), initial_view);

    let check_points = |template: &GarmentTemplate| {
        let polygon = template.panels[panel_idx].polygon.as_ref().unwrap();
        for (i, &point) in polygon.points.iter().enumerate() {
            let expected = if i == anchor {
                to
            } else {
                panel_to_board_pt(&before, panel_idx, original.points[i])
            };
            assert!(panel_to_board_pt(template, panel_idx, point).distance(expected) < 1e-6);
        }
    };
    check_points(preview);
    handle_runner_event(&mut state, &pointer_btn_ev(false));
    check_points(&state.project.template);
    assert_eq!(compute_2d_view_box_m(&state), initial_view);
    assert!(undo_redo_2d(&mut state, false));
    assert_eq!(compute_2d_view_box_m(&state), initial_view);
    assert!(undo_redo_2d(&mut state, true));
    assert_eq!(compute_2d_view_box_m(&state), initial_view);

    apply_runner_action(&mut state, RunnerAction::Zoom2dFit, false);
    assert_ne!(compute_2d_view_box_m(&state), initial_view);
}

#[test]
fn test_selection_marquee_selects_on_release_and_escape_cancels() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut state = selection_test_state(dir.path());
    let before = state.project.template.clone();
    let pan = state.board_2d_pan;
    let points: Vec<_> = before
        .panels
        .iter()
        .enumerate()
        .flat_map(|(i, p)| {
            panel_raw_points(p)
                .into_iter()
                .enumerate()
                .map(move |(j, v)| (i, j, v))
        })
        .collect();
    let lo = points
        .iter()
        .map(|&(i, _, v)| panel_to_board_pt(&before, i, v))
        .fold(DVec2::splat(f64::INFINITY), DVec2::min)
        - DVec2::splat(0.01);
    let hi = points
        .iter()
        .map(|&(i, _, v)| panel_to_board_pt(&before, i, v))
        .fold(DVec2::splat(f64::NEG_INFINITY), DVec2::max)
        + DVec2::splat(0.01);
    let start = board_to_screen_px(&state, hi);
    let end = board_to_screen_px(&state, lo);
    handle_runner_event(&mut state, &pointer_move_ev(start.0, start.1));
    handle_runner_event(&mut state, &pointer_btn_mod_ev(true, true, false));
    handle_runner_event(&mut state, &pointer_move_ev(end.0, end.1));
    assert!(state.selected_2d_points.is_empty());
    assert_eq!(
        count_class(&render_runner_2d_paths(&state).0, "selection-marquee"),
        1
    );
    // Releasing Shift before the button still completes the captured gesture.
    handle_runner_event(&mut state, &pointer_btn_ev(false));
    assert_eq!(state.selected_2d_points.len(), points.len());
    assert_eq!(state.board_2d_pan, pan);
    assert_eq!(
        serde_json::to_value(&state.project.template).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    assert!(!state.files.modified);
    let selected = state.selected_2d_points.clone();
    handle_runner_event(&mut state, &pointer_btn_mod_ev(true, true, false));
    handle_runner_event(&mut state, &pointer_move_ev(start.0, start.1));
    handle_runner_event(
        &mut state,
        &EngineEvent::Key(cssimpler::renderer::KeyboardEvent {
            logical_key: cssimpler::renderer::KeyIdentity::Named("Escape".into()),
            physical_key: None,
            location: cssimpler::renderer::KeyLocation::Standard,
            state: cssimpler::renderer::ButtonState::Pressed,
            repeat: false,
            modifiers: cssimpler::renderer::KeyboardModifiers::default(),
        }),
    );
    handle_runner_event(&mut state, &pointer_btn_ev(false));
    assert_eq!(state.selected_2d_points, selected);
    assert!(state.drag_2d.is_none());
    assert_eq!(
        count_class(&render_runner_2d_paths(&state).0, "selection-marquee"),
        0
    );
    state.clear_2d_subselection();
    let anchor = panel_to_board_pt(
        &before,
        0,
        before.panels[0].polygon.as_ref().unwrap().points[0],
    );
    let a = board_to_screen_px(&state, anchor - DVec2::splat(0.005));
    let b = board_to_screen_px(&state, anchor + DVec2::splat(0.005));
    handle_runner_event(&mut state, &pointer_move_ev(a.0, a.1));
    handle_runner_event(&mut state, &pointer_btn_mod_ev(true, true, false));
    handle_runner_event(&mut state, &pointer_move_ev(b.0, b.1));
    handle_runner_event(&mut state, &pointer_btn_ev(false));
    assert_eq!(
        state.selected_2d_points,
        vec![(before.panels[0].id.clone(), 0)]
    );
    handle_runner_event(&mut state, &pointer_btn_mod_ev(true, true, false));
    handle_runner_event(&mut state, &EngineEvent::FocusChanged(false));
    assert!(state.drag_2d.is_none());
    assert!(!state.pointer_down);
    assert_eq!(state.selected_2d_points.len(), 1);
}

#[test]
fn test_2d_multi_point_selection_round_curve_drag_and_delete() {
    let _guard = worker::WORKER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let avatar = dir.path().join("avatar.obj");
    std::fs::write(&avatar, "v 0 0 -10\nv 1 0 -10\nv 0 1 -10\nf 1 2 3\n").expect("write avatar");
    let project = FittingProject::basic(GarmentKind::Tshirt, 0, avatar);
    let scene = project.compile().expect("compile scene");
    let files = DocumentFiles {
        output: dir.path().join("out_multi_2d"),
        source: None,
        modified: false,
    };
    let (tx, latest) = worker::start_cpu().expect("start cpu worker");
    let mut state = CssRunnerState::new(project, scene, files, tx, latest);
    state.active_2d_tool = "Edit".into();
    state.corner_radius_mm = 8.0;

    let idx_by_edge = |tmpl: &GarmentTemplate, pidx: usize, name: &str| -> usize {
        tmpl.panels[pidx]
            .polygon
            .as_ref()
            .unwrap()
            .edges
            .iter()
            .position(|e| e == name)
            .unwrap()
    };
    let click_corner =
        |st: &mut CssRunnerState, pidx: usize, pt_idx: usize, shift: bool, ctrl: bool| {
            let pt = st.project.template.panels[pidx]
                .polygon
                .as_ref()
                .unwrap()
                .points[pt_idx];
            let (sx, sy) =
                board_to_screen_px(st, panel_to_board_pt(&st.project.template, pidx, pt));
            handle_runner_event(st, &pointer_move_ev(sx, sy));
            assert!(handle_runner_event(
                st,
                &pointer_btn_mod_ev(true, shift, ctrl)
            ));
            assert!(handle_runner_event(
                st,
                &pointer_btn_mod_ev(false, shift, ctrl)
            ));
            (sx, sy)
        };

    let c_right = idx_by_edge(&state.project.template, 0, "sleeve-top-right");
    let c_left = idx_by_edge(&state.project.template, 0, "shoulder-left");

    // 1. Click first sewn corner, Shift+click second sewn corner, Ctrl+click toggle off & on
    click_corner(&mut state, 0, c_right, false, false);
    assert_eq!(state.selected_2d_points, vec![("front".into(), c_right)]);
    click_corner(&mut state, 0, c_left, true, false);
    assert_eq!(
        state.selected_2d_points,
        vec![("front".into(), c_right), ("front".into(), c_left)]
    );
    // Ctrl+click toggles c_left off, then back on
    click_corner(&mut state, 0, c_left, false, true);
    assert_eq!(state.selected_2d_points, vec![("front".into(), c_right)]);
    click_corner(&mut state, 0, c_left, false, true);
    assert_eq!(
        state.selected_2d_points,
        vec![("front".into(), c_right), ("front".into(), c_left)]
    );

    // Connected highlights & active fillet previews cover both corners + back sewn partners
    let conn = compute_connected_2d_highlight(&state);
    assert!(conn.selected_corners.contains(&("back".into(), c_right)));
    assert!(conn.selected_corners.contains(&("back".into(), c_left)));
    state.active_2d_tool = "Corner".into();
    let previews = active_fillet_previews(&state);
    assert_eq!(previews.len(), 4);

    // 2. Pressing Enter even after switching back to Edit tool rounds ALL selected corners + sewn partners
    state.active_2d_tool = "Edit".into();
    assert!(handle_runner_event(
        &mut state,
        &EngineEvent::Key(cssimpler::renderer::KeyboardEvent {
            logical_key: cssimpler::renderer::KeyIdentity::Named("Enter".into()),
            physical_key: None,
            location: cssimpler::renderer::KeyLocation::Standard,
            state: cssimpler::renderer::ButtonState::Pressed,
            repeat: false,
            modifiers: cssimpler::renderer::KeyboardModifiers::default(),
        })
    ));
    for (pidx, panel) in state.project.template.panels.iter().enumerate().take(2) {
        let rc = panel
            .polygon
            .as_ref()
            .unwrap()
            .curves
            .keys()
            .filter(|k| k.starts_with("corner_round_"))
            .count();
        assert_eq!(rc, 2, "panel {pidx} must have 2 rounded corners");
    }
    assert_eq!(
        state
            .project
            .template
            .seams
            .iter()
            .filter(|s| s.id.contains("corner_round"))
            .count(),
        2
    );
    state.project.template.compile().unwrap();

    // Single-step Undo restores original unrounded T-shirt across all panels & seams
    assert!(undo_redo_2d(&mut state, false));
    assert!(
        state.project.template.panels[0]
            .polygon
            .as_ref()
            .unwrap()
            .curves
            .is_empty()
    );
    assert!(
        state.project.template.panels[1]
            .polygon
            .as_ref()
            .unwrap()
            .curves
            .is_empty()
    );

    // 2b. Shift+click corners across DIFFERENT panels (front + back neck-bottom), then click Corner tool to round both
    let nb_front = idx_by_edge(&state.project.template, 0, "neck-bottom");
    let nb_back = idx_by_edge(&state.project.template, 1, "neck-bottom");
    click_corner(&mut state, 0, nb_front, false, false);
    click_corner(&mut state, 1, nb_back, true, false);
    assert_eq!(
        state.selected_2d_points,
        vec![("front".into(), nb_front), ("back".into(), nb_back)]
    );
    state.active_2d_tool = "Corner".into();
    click_corner(&mut state, 0, nb_front, false, false);
    assert!(
        state.project.template.panels[0]
            .polygon
            .as_ref()
            .unwrap()
            .curves
            .keys()
            .any(|k| k.starts_with("corner_round_"))
    );
    assert!(
        state.project.template.panels[1]
            .polygon
            .as_ref()
            .unwrap()
            .curves
            .keys()
            .any(|k| k.starts_with("corner_round_"))
    );
    assert!(undo_redo_2d(&mut state, false));
    state.active_2d_tool = "Edit".into();

    // 3. Re-select both corners and toggle curves (CurvePoints) on both + sewn partners
    click_corner(&mut state, 0, c_right, false, false);
    click_corner(&mut state, 0, c_left, true, false);
    assert!(toggle_selected_curve(&mut state, true));
    assert!(
        state.project.template.panels[0]
            .polygon
            .as_ref()
            .unwrap()
            .curves
            .len()
            >= 2
    );
    assert!(
        state.project.template.panels[1]
            .polygon
            .as_ref()
            .unwrap()
            .curves
            .len()
            >= 2
    );
    assert!(toggle_selected_curve(&mut state, false));
    assert!(
        state.project.template.panels[0]
            .polygon
            .as_ref()
            .unwrap()
            .curves
            .is_empty()
    );
    assert!(
        state.project.template.panels[1]
            .polygon
            .as_ref()
            .unwrap()
            .curves
            .is_empty()
    );

    // 4. Drag one of two selected corners -> moves BOTH corners + sewn partners, click without drag collapses
    click_corner(&mut state, 0, c_right, false, false);
    click_corner(&mut state, 0, c_left, true, false);
    let p_right_before = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .points[c_right];
    let p_left_before = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .points[c_left];
    let (sx, sy) = board_to_screen_px(
        &state,
        panel_to_board_pt(&state.project.template, 0, p_right_before),
    );
    handle_runner_event(&mut state, &pointer_move_ev(sx, sy));
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(true)));
    handle_runner_event(&mut state, &pointer_move_ev(sx + 6.0, sy + 5.0));
    assert!(state.preview_template.is_some());
    assert!(handle_runner_event(&mut state, &pointer_btn_ev(false)));
    let p_right_after = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .points[c_right];
    let p_left_after = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .points[c_left];
    assert!(p_right_after.distance(p_right_before) > 1e-4);
    assert!(p_left_after.distance(p_left_before) > 1e-4);
    state.project.template.compile().unwrap();
    assert!(undo_redo_2d(&mut state, false));

    // Clicking one corner in multi-selection without dragging collapses selection to that corner
    click_corner(&mut state, 0, c_right, false, false);
    click_corner(&mut state, 0, c_left, true, false);
    assert_eq!(state.selected_2d_points.len(), 2);
    click_corner(&mut state, 0, c_left, false, false);
    assert_eq!(state.selected_2d_points, vec![("front".into(), c_left)]);

    // 5. Delete key removes multiple selected unsewn corners atomically in one step
    let nr_front = idx_by_edge(&state.project.template, 0, "neck-right");
    let n_before = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .points
        .len();
    click_corner(&mut state, 0, nb_front, false, false);
    click_corner(&mut state, 0, nr_front, true, false);
    assert!(handle_runner_event(
        &mut state,
        &EngineEvent::Key(cssimpler::renderer::KeyboardEvent {
            logical_key: cssimpler::renderer::KeyIdentity::Named("Delete".into()),
            physical_key: None,
            location: cssimpler::renderer::KeyLocation::Standard,
            state: cssimpler::renderer::ButtonState::Pressed,
            repeat: false,
            modifiers: cssimpler::renderer::KeyboardModifiers::default(),
        })
    ));
    let n_after = state.project.template.panels[0]
        .polygon
        .as_ref()
        .unwrap()
        .points
        .len();
    assert_eq!(n_after, n_before - 2);
    assert!(state.selected_2d_points.is_empty());
    assert!(undo_redo_2d(&mut state, false));
    assert_eq!(
        state.project.template.panels[0]
            .polygon
            .as_ref()
            .unwrap()
            .points
            .len(),
        n_before
    );
}

#[test]
fn test_2d_corner_select_live_preview_and_hover_tooltips() {
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
        output: dir.path().join("out_corner_preview"),
        source: None,
        modified: false,
    };
    let (tx, latest) = worker::start_cpu().expect("start cpu worker");
    let state = CssRunnerState::new(project, scene, files, tx, latest);
    let mut app = create_runner_app(state, &stylesheet);
    app.set_viewport(cssimpler::renderer::ViewportSize::new(1600, 980));
    app.frame(FrameInfo {
        delta: Duration::from_millis(16),
        frame_index: 1,
    });

    // 1. Switch to Corner Select via 'v' shortcut (distinct from Corner Rounding Menu 'c')
    assert!(handle_runner_event(
        app.state_mut(),
        &EngineEvent::Key(cssimpler::renderer::KeyboardEvent {
            logical_key: cssimpler::renderer::KeyIdentity::Character("v".into()),
            physical_key: None,
            location: cssimpler::renderer::KeyLocation::Standard,
            state: cssimpler::renderer::ButtonState::Pressed,
            repeat: false,
            modifiers: cssimpler::renderer::KeyboardModifiers::default(),
        })
    ));
    assert_eq!(app.state().active_2d_tool, "Corner Select");
    assert!(app.state().accordion_open[1]);
    assert!(active_fillet_previews(app.state()).is_empty());

    let idx_by_edge = |tmpl: &GarmentTemplate, pidx: usize, name: &str| -> usize {
        tmpl.panels[pidx]
            .polygon
            .as_ref()
            .unwrap()
            .edges
            .iter()
            .position(|e| e == name)
            .unwrap()
    };
    let c_right = idx_by_edge(&app.state().project.template, 0, "sleeve-top-right");
    let c_left = idx_by_edge(&app.state().project.template, 0, "shoulder-left");

    // 2. Click + Shift+click two sewn corners in Corner Select mode -> selects without committing or dragging
    for (idx, shift) in [(c_right, false), (c_left, true)] {
        let pt = app.state().project.template.panels[0]
            .polygon
            .as_ref()
            .unwrap()
            .points[idx];
        let (sx, sy) = board_to_screen_px(
            app.state(),
            panel_to_board_pt(&app.state().project.template, 0, pt),
        );
        handle_runner_event(app.state_mut(), &pointer_move_ev(sx, sy));
        assert!(handle_runner_event(
            app.state_mut(),
            &pointer_btn_mod_ev(true, shift, false)
        ));
        assert!(app.state().drag_2d.is_none());
        assert!(handle_runner_event(
            app.state_mut(),
            &pointer_btn_mod_ev(false, shift, false)
        ));
    }
    assert_eq!(app.state().selected_2d_points.len(), 2);
    assert!(
        app.state().project.template.panels[0]
            .polygon
            .as_ref()
            .unwrap()
            .curves
            .is_empty()
    );
    assert!(active_fillet_previews(app.state()).is_empty());

    // 3. Adjust corner rounding settings while Corner Select is still active -> live fillet preview appears & updates
    let (svg_before, _) = render_runner_2d_paths(app.state());
    apply_runner_ui_op(app.state_mut(), RunnerUiOp::AdjustCornerRadiusMm(4.0));
    assert!(app.state().corner_preview_active);
    let p_after_r = active_fillet_previews(app.state());
    assert_eq!(
        p_after_r.len(),
        4,
        "must preview both corners + sewn partners"
    );
    let c1_r14 = p_after_r[0].2.control_1;
    let (svg_preview_r14, _) = render_runner_2d_paths(app.state());
    assert_ne!(format!("{svg_before:?}"), format!("{svg_preview_r14:?}"));

    apply_runner_ui_op(app.state_mut(), RunnerUiOp::AdjustCornerRadiusMm(-6.0));
    let p_after_r8 = active_fillet_previews(app.state());
    assert_eq!(p_after_r8.len(), 4);
    assert!(p_after_r8[0].2.control_1.distance(c1_r14) > 1e-4);

    apply_runner_ui_op(app.state_mut(), RunnerUiOp::SetCornerMode(false));
    apply_runner_ui_op(app.state_mut(), RunnerUiOp::AdjustCornerTension(-0.15));
    let p_after_tension = active_fillet_previews(app.state());
    assert_eq!(p_after_tension.len(), 4);
    assert!(
        p_after_tension[0]
            .2
            .control_1
            .distance(p_after_r8[0].2.control_1)
            > 1e-4
    );
    // Geometry remains uncommitted during live preview
    assert!(
        app.state().project.template.panels[0]
            .polygon
            .as_ref()
            .unwrap()
            .curves
            .is_empty()
    );

    // Switching to Corner Rounding Menu tool preserves selection & preview; clicking Round commits and clears preview
    set_active_2d_tool(app.state_mut(), "Corner");
    assert_eq!(app.state().active_2d_tool, "Corner");
    assert_eq!(active_fillet_previews(app.state()).len(), 4);
    apply_runner_ui_op(app.state_mut(), RunnerUiOp::ApplyCornerRoundSelected);
    assert!(!app.state().corner_preview_active);
    assert_eq!(
        app.state().project.template.panels[0]
            .polygon
            .as_ref()
            .unwrap()
            .curves
            .len(),
        2
    );
    assert!(undo_redo_2d(app.state_mut(), false));

    // 4. Verify distinct Corner icons in #rail-2d and hover tooltips across #rail-2d, #rail-3d, #top-bar
    assert_ne!(
        format!("{:?}", icon_corner_select_2d()),
        format!("{:?}", icon_corner_2d())
    );

    let ui_tree = build_runner_ui(app.state());
    let base_scene = cssimpler::style::build_render_tree_in_viewport_with_interaction(
        &ui_tree,
        &stylesheet,
        1600,
        980,
        &cssimpler::core::ElementInteractionState::default(),
    );

    fn find_dom_title<'a>(node: &'a Node, id: &str) -> Option<&'a str> {
        match node {
            Node::Element(el) => {
                if el.id.as_deref() == Some(id) {
                    return el.attribute("title");
                }
                for child in &el.children {
                    if let Some(found) = find_dom_title(child, id) {
                        return Some(found);
                    }
                }
                None
            }
            _ => None,
        }
    }
    fn find_by_id<'a>(
        node: &'a cssimpler::renderer::RenderNode,
        id: &str,
    ) -> Option<&'a cssimpler::renderer::RenderNode> {
        if node.id() == Some(id) {
            return Some(node);
        }
        for child in &node.children {
            if let Some(found) = find_by_id(child, id) {
                return Some(found);
            }
        }
        None
    }
    fn node_text(node: &cssimpler::renderer::RenderNode) -> Option<&str> {
        match &node.kind {
            cssimpler::core::RenderKind::Text(t) => Some(t.as_str()),
            _ => None,
        }
    }
    fn collect_all_text(node: &cssimpler::renderer::RenderNode, out: &mut Vec<String>) {
        if let Some(t) = node_text(node) {
            out.push(t.to_string());
        }
        for child in &node.children {
            collect_all_text(child, out);
        }
    }
    fn find_node_with_text<'a>(
        node: &'a cssimpler::renderer::RenderNode,
        needle: &str,
    ) -> Option<&'a cssimpler::renderer::RenderNode> {
        if node_text(node) == Some(needle)
            || node.children.iter().any(|c| node_text(c) == Some(needle))
        {
            return Some(node);
        }
        for child in &node.children {
            if let Some(found) = find_node_with_text(child, needle) {
                return Some(found);
            }
        }
        None
    }

    for (btn_id, expected_tooltip) in [
        (
            "#tool-corner-select-2d",
            "Select Corners (V / Shift+Click multi)",
        ),
        ("#tool-corner-round-2d", "Corner Rounding Menu & Fillet (C)"),
        (
            "#avatar-diagnostics-btn",
            "Avatar & Dress Form Diagnostics (Shift+A)",
        ),
        ("#sim-play-btn", "Start/Stop simulation (Space)"),
    ] {
        let trimmed_id = btn_id.trim_start_matches('#');
        assert_eq!(find_dom_title(&ui_tree, trimmed_id), Some(expected_tooltip));
        let btn_node = find_by_id(&base_scene, trimmed_id)
            .unwrap_or_else(|| panic!("missing button {btn_id} in render tree"));
        // Hover the SVG child if present to verify ancestor :hover::after propagation too
        let hover_path = btn_node
            .children
            .first()
            .and_then(|c| c.element_path.clone())
            .or_else(|| btn_node.element_path.clone())
            .expect("element_path for hover");
        let hovered_scene = cssimpler::style::build_render_tree_in_viewport_with_interaction(
            &ui_tree,
            &stylesheet,
            1600,
            980,
            &cssimpler::core::ElementInteractionState {
                hovered: Some(hover_path),
                active: None,
            },
        );
        let mut texts = Vec::new();
        collect_all_text(&hovered_scene, &mut texts);
        assert!(
            texts.iter().any(|t| t == expected_tooltip),
            "hovering {btn_id} must render tooltip {expected_tooltip:?}; texts={texts:?}"
        );
        let tip_node =
            find_node_with_text(&hovered_scene, expected_tooltip).expect("tooltip render node");
        assert!(
            tip_node.layout.width > 20.0
                && tip_node.layout.height > 12.0
                && tip_node.layout.x >= 0.0
                && tip_node.layout.y >= 0.0
                && tip_node.layout.x + tip_node.layout.width <= 1600.0
                && tip_node.layout.y + tip_node.layout.height <= 980.0,
            "tooltip for {btn_id} must fit cleanly on screen: {:?}",
            tip_node.layout
        );
    }
}
