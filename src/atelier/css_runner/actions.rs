//! Queued UI actions, operation dispatch and frame updates.

use super::board::interaction::set_active_2d_tool;
use super::board::types::ToolDraft2d;
use super::editing::{
    confirm_pending_sew, round_selected_corner, toggle_selected_curve, undo_redo_2d,
};
use super::mesh_preparation;
use super::state::{
    CssRunnerState, DEFAULT_2D_ZOOM_SPEED, MAX_2D_ZOOM_SPEED, MIN_2D_ZOOM_SPEED, ObjectCategory,
};
use clothcraft::authoring::CornerRule;
use clothcraft::css_ui::ViewLayoutMode;
use clothcraft::fitting::FittingScene;
use clothcraft::garment::presets::{GarmentKind, basic as basic_garment};
use clothcraft::gpu_avbd::ContactMethod;
use cssimpler::ActionQueue;
use cssimpler::app::Invalidation;
use cssimpler::renderer::FrameInfo;
use std::fs;
use std::sync::Arc;

/// One typed command from native UI callbacks to the runner update.
///
/// The runner delivers these FIFO through [`cssimpler::ActionQueue`], so repeated
/// clicks, payloads and their order are preserved instead of being coalesced into
/// a bitmask.
#[derive(Clone, Debug, PartialEq)]
pub enum RunnerAction {
    PlayPause,
    Step,
    Reset,
    Save,
    ExportSvg,
    ExportObj,
    CleanView,
    PaletteToggle,
    ThemeToggle,
    ViewSplit,
    View3d,
    View2d,
    DrawerToggle,
    TabPattern,
    TabSewing,
    TabForm,
    TabMaterial,
    TabEnv,
    CamReset,
    PieceNext,
    ToolSelect,
    ToolSew,
    ToolMeasure,
    ToolCorner,
    ToolGizmo,
    ToolGrab,
    ToolSew3d,
    ToolPin,
    ToolMeasure3d,
    ToolAvatarDiag,
    ToolEdit2d,
    ToolRect2d,
    ToolPen2d,
    ToolFree2d,
    ToolSew2d,
    ToolSplit2d,
    ToolInternal2d,
    ToolMeasure2d,
    ToolTrim2d,
    ToolCorner2d,
    PinModeToggle,
    CornerModeToggle,
    AccordionToggle,
    SolverMenu,
    SolverGpu,
    SolverCpu,
    SolverAl,
    Zoom2dIn,
    Zoom2dOut,
    Zoom2dFit,
    NextPiece2d,
    ToolCornerSelect2d,
    Accordion(usize),
    Piece(usize),
    /// A command carrying data, such as a numeric edit or preset choice.
    Ui(RunnerUiOp),
}

#[derive(Clone, Debug, PartialEq)]
pub enum RunnerUiOp {
    SetNumber(super::ui::number::NumberField, f64),
    MeshLevel(u8),
    MeshToggle(usize),
    MeshChoosePath(String),
    MeshReload,
    MeshInputFocus,
    MeshToggleOpen,
    MeshField(usize, String),
    PrepareMesh(bool),

    CycleMeasurementBits,
    ToggleSettings,
    CloseSettings,
    Adjust2dZoomSpeed(f64),
    Reset2dZoomSpeed,
    SetCornerMode(bool),
    SetCornerRule(CornerRule),
    AdjustCornerRadiusMm(f64),
    AdjustCornerTension(f64),
    ApplyCornerRoundSelected,
    ToggleSelectedCurve(bool),
    Undo2d,
    Redo2d,
    SelectPiece(usize),
    LoadPreset(GarmentKind),
    StatusNote(String),
    ConfirmSew { ease: bool },
    CancelSew,
}

pub fn update_runner(state: &mut CssRunnerState, frame: FrameInfo) -> Invalidation {
    // Consume one coalesced pointer batch per displayed frame. The event handler
    // feeds FrameAlignedDrag and defers pointer-driven invalidation to this update.
    let drag_update = state.frame_drag.begin_frame(frame.frame_index);
    let input_changed = std::mem::take(&mut state.input_dirty);
    if drag_update.as_ref().is_some_and(|update| update.overflowed) {
        state.frame_drag.reset();
    }
    let number_changed = super::input::number::tick_number_drag(state);
    // 1. Poll simulation worker result
    let done = state
        .worker_latest
        .lock()
        .ok()
        .and_then(|mut guard| guard.take());
    let mut had_result = false;
    if let Some(done) = done
        && done.generation == state.generation
    {
        state.worker_pending = false;
        match done.result {
            Ok((new_state, metrics)) => {
                had_result = true;
                state.scene.state = new_state;
                state.step_count += 1;
                state.step_ms = metrics.step_ms;
                let residual = metrics.barrier_al.as_ref().map_or_else(
                    || {
                        metrics
                            .residual_rms_n
                            .map_or("N/A".into(), |r| format!("{r:.1e} N RMS"))
                    },
                    |al| {
                        format!(
                            "{:.1e} N max, gap {:.1e} m",
                            al.original_force_max_n, al.primal_m
                        )
                    },
                );
                state.status_message = format!(
                    "Step {}: min dist {:.3} mm, residual {}, {:.1} ms",
                    state.step_count,
                    metrics.minimum_distance_m * 1000.,
                    residual,
                    metrics.step_ms
                );
                state.last_metrics = Some(metrics);
                state.sync_usd_points();
            }
            Err(e) => {
                state.playing = false;
                state.status_message = format!("Worker error: {e}");
                state.send_control_state(false);
            }
        }
    }

    state.cadence.update(
        std::time::Instant::now(),
        state.scene.state.time,
        state.generation,
        state.playing,
    );

    if let Some(perf) = state.perf.as_mut() {
        let dt = state.settings.dt;
        if let Some(report) = perf.tick(state.scene.state.time, frame.frame_index, dt, true) {
            let _ = fs::create_dir_all(&state.files.output);
            let perf_path = state.files.output.join("ui-perf.json");
            let _ = fs::write(
                &perf_path,
                serde_json::to_string_pretty(&report).unwrap_or_default(),
            );
            state.should_exit = true;
        } else if perf.expired() {
            state.should_exit = true;
        }
    }

    // 2. Process queued typed actions in click order.
    let previous_piece = state.selected_piece_index;
    let mut batch = std::mem::take(&mut state.action_batch);
    state.actions.drain_into(&mut batch);
    let had_actions = !batch.is_empty();
    let has_accordion_action = batch
        .iter()
        .any(|action| matches!(action, RunnerAction::Accordion(_)));
    for action in batch.drain(..) {
        apply_runner_action(state, action, has_accordion_action);
    }
    state.action_batch = batch;
    if state.selected_piece_index != previous_piece {
        state.clear_2d_subselection();
        state.piece_selected_explicitly = true;
    }

    let selected_mesh_panel = state
        .project
        .template
        .panels
        .get(state.selected_piece_index);
    if state.mesh_draft.panel_id.as_deref() != selected_mesh_panel.map(|p| p.id.as_str()) {
        state.mesh_draft = mesh_preparation::MeshDraft::from_panel(selected_mesh_panel);
    }

    if number_changed || input_changed || had_actions || (had_result && !state.playing) {
        Invalidation::Structure
    } else if had_result {
        Invalidation::Paint
    } else {
        Invalidation::Clean
    }
}

pub(super) fn apply_runner_action(
    state: &mut CssRunnerState,
    action: RunnerAction,
    has_accordion_action: bool,
) {
    match action {
        RunnerAction::PlayPause => {
            state.playing = !state.playing;
            state.send_control_state(false);
        }
        RunnerAction::Step => state.send_control_state(true),
        RunnerAction::Reset => state.reset_simulation(),
        RunnerAction::Save => state.save_project(),
        RunnerAction::ExportSvg => state.export_pattern_svg(),
        RunnerAction::ExportObj => state.export_obj(),
        RunnerAction::CleanView => state.clean_view = !state.clean_view,
        RunnerAction::PaletteToggle => {
            state.command_palette_open = !state.command_palette_open;
            state.settings_open = false;
        }
        RunnerAction::ThemeToggle => state.theme = state.theme.toggled(),
        RunnerAction::ViewSplit => state.layout_mode = ViewLayoutMode::Split,
        RunnerAction::View3d => state.layout_mode = ViewLayoutMode::ThreeDOnly,
        RunnerAction::View2d => state.layout_mode = ViewLayoutMode::TwoDOnly,
        RunnerAction::DrawerToggle => state.left_drawer_open = !state.left_drawer_open,
        RunnerAction::TabPattern => state.active_category = ObjectCategory::Pattern2D,
        RunnerAction::TabSewing => state.active_category = ObjectCategory::Sewing,
        RunnerAction::TabForm => state.active_category = ObjectCategory::Form3D,
        RunnerAction::TabMaterial => state.active_category = ObjectCategory::Material,
        RunnerAction::TabEnv => state.active_category = ObjectCategory::Environment,
        RunnerAction::CamReset => {
            state.camera = clothcraft::usd_scene::default_scene_camera(
                &state.scene.state.x,
                Some(&state.scene.collider.surface),
            );
            state.sync_usd_camera();
            state.status_message = "Reset 3D camera".to_string();
        }
        RunnerAction::PieceNext => {
            let count = state.project.template.panels.len().max(1);
            state.selected_piece_index = (state.selected_piece_index + 1) % count;
        }
        RunnerAction::ToolGizmo | RunnerAction::ToolSelect => {
            state.active_3d_tool = "Gizmo".to_string();
            state.status_message = "3D Tool: Gizmo — drag 3D viewport to orbit".to_string();
        }
        RunnerAction::ToolGrab => {
            state.active_3d_tool = "Grab".to_string();
            state.status_message = "3D Tool: Grab".to_string();
        }
        RunnerAction::ToolSew3d => {
            state.active_3d_tool = "3D Sew".to_string();
            state.active_category = ObjectCategory::Sewing;
            state.status_message = "3D Tool: 3D Sew (inspect seams in Object Browser)".to_string();
        }
        RunnerAction::ToolPin => {
            state.active_3d_tool = "Pin".to_string();
            state.accordion_open[3] = true;
            state.open_accordion_idx = 3;
            state.status_message = "3D Tool: Pin".to_string();
        }
        RunnerAction::ToolMeasure3d => {
            state.active_3d_tool = "Measure".to_string();
            state.status_message = "3D Tool: 3D Measure".to_string();
        }
        RunnerAction::ToolAvatarDiag => {
            state.active_3d_tool = "Avatar Diagnostics".to_string();
            state.active_category = ObjectCategory::Environment;
            state.accordion_open[5] = true;
            state.open_accordion_idx = 5;
            state.status_message = "3D Tool: Avatar Diagnostics".to_string();
        }
        RunnerAction::ToolEdit2d => set_active_2d_tool(state, "Edit"),
        RunnerAction::ToolRect2d => set_active_2d_tool(state, "Rect"),
        RunnerAction::ToolPen2d => set_active_2d_tool(state, "Pen"),
        RunnerAction::ToolFree2d => set_active_2d_tool(state, "Freehand"),
        RunnerAction::ToolSew | RunnerAction::ToolSew2d => set_active_2d_tool(state, "Sew"),
        RunnerAction::ToolSplit2d => set_active_2d_tool(state, "Split"),
        RunnerAction::ToolInternal2d => set_active_2d_tool(state, "Internal Line"),
        RunnerAction::ToolMeasure | RunnerAction::ToolMeasure2d => {
            set_active_2d_tool(state, "Measure")
        }
        RunnerAction::ToolTrim2d => set_active_2d_tool(state, "Trim"),
        RunnerAction::ToolCornerSelect2d => set_active_2d_tool(state, "Corner Select"),
        RunnerAction::ToolCorner | RunnerAction::ToolCorner2d => {
            set_active_2d_tool(state, "Corner")
        }
        RunnerAction::PinModeToggle => state.pin_mode_contour = !state.pin_mode_contour,
        RunnerAction::CornerModeToggle => {
            state.corner_mode_radius = !state.corner_mode_radius;
            state.corner_preview_active = true;
        }
        RunnerAction::Accordion(index) => {
            if let Some(open) = state.accordion_open.get_mut(index) {
                *open = !*open;
                state.open_accordion_idx = index;
            }
        }
        RunnerAction::AccordionToggle => {
            // A direct accordion selection in the same batch owns the result,
            // matching the previous bitmask batch semantics.
            if !has_accordion_action {
                state.open_accordion_idx = (state.open_accordion_idx + 1) % 6;
                state.accordion_open[state.open_accordion_idx] =
                    !state.accordion_open[state.open_accordion_idx];
            }
        }
        RunnerAction::Piece(index) => {
            if state.project.template.panels.len() > index {
                state.selected_piece_index = index;
            }
        }
        RunnerAction::SolverMenu => {
            state.solver_menu_open = !state.solver_menu_open;
            state.settings_open = false;
        }
        RunnerAction::SolverGpu => state.select_solver(true),
        RunnerAction::SolverAl => state.select_solver_method(true, ContactMethod::BarrierAl),
        RunnerAction::SolverCpu => state.select_solver(false),
        RunnerAction::Zoom2dIn => {
            state.board_2d_zoom = (state.board_2d_zoom * 1.25).clamp(0.25, 10.0);
        }
        RunnerAction::Zoom2dOut => {
            state.board_2d_zoom = (state.board_2d_zoom / 1.25).clamp(0.25, 10.0);
        }
        RunnerAction::Zoom2dFit => state.frame_all_2d(),
        RunnerAction::NextPiece2d => {
            let count = state.project.template.panels.len().max(1);
            state.selected_piece_index = (state.selected_piece_index + 1) % count;
            if let Some(p) = state
                .project
                .template
                .panels
                .get(state.selected_piece_index)
            {
                state.status_message = format!("Selected piece: {} ({})", p.name, p.id);
            }
        }
        RunnerAction::Ui(op) => apply_runner_ui_op(state, op),
    }
}

pub(super) fn queue_action(
    actions: &ActionQueue<RunnerAction>,
    action: RunnerAction,
) -> impl Fn() + Send + Sync + 'static {
    let actions = actions.clone();
    move || actions.push(action.clone())
}

pub(super) fn queue_ui_op(
    actions: &ActionQueue<RunnerAction>,
    op: RunnerUiOp,
) -> impl Fn() + Send + Sync + 'static {
    queue_action(actions, RunnerAction::Ui(op))
}

pub(super) fn apply_runner_ui_op(state: &mut CssRunnerState, op: RunnerUiOp) {
    match op {
        RunnerUiOp::SetNumber(field, value) => {
            if !value.is_finite() {
                return;
            }
            use super::ui::number::NumberField;
            match field {
                NumberField::ZoomSpeed => {
                    state.board_2d_zoom_speed = value.clamp(MIN_2D_ZOOM_SPEED, MAX_2D_ZOOM_SPEED)
                }
                NumberField::CornerRadius => {
                    state.corner_radius_mm = value.clamp(1.0, 250.0);
                    state.corner_preview_active = true;
                }
                NumberField::CornerTension => {
                    state.corner_tension = value.clamp(0.05, 1.5);
                    state.corner_mode_radius = false;
                    state.corner_preview_active = true;
                }
            }
        }
        RunnerUiOp::MeshToggleOpen => {
            state.mesh_draft.open = !state.mesh_draft.open;
        }
        RunnerUiOp::MeshInputFocus => {
            state.mesh_input_focus = true;
        }
        RunnerUiOp::MeshToggle(i) => {
            if let Some(option) = state.mesh_draft.options.get_mut(i) {
                *option = !*option;
            }
        }
        RunnerUiOp::MeshChoosePath(path) => {
            state.mesh_draft.choose_path(
                state
                    .project
                    .template
                    .panels
                    .get(state.selected_piece_index),
                path,
            );
        }
        RunnerUiOp::MeshReload => {
            state.mesh_draft = mesh_preparation::MeshDraft::from_panel(
                state
                    .project
                    .template
                    .panels
                    .get(state.selected_piece_index),
            );
        }
        RunnerUiOp::MeshLevel(level) => {
            state.mesh_draft.level = level.min(16);
        }
        RunnerUiOp::MeshField(field, value) => {
            if let Some(slot) = state.mesh_draft.fields.get_mut(field) {
                *slot = value;
            }
        }
        RunnerUiOp::PrepareMesh(physics) => {
            if let Err(error) = mesh_preparation::apply(state, physics) {
                state.status_message = format!("Mesh preparation rejected: {error}");
            }
        }

        RunnerUiOp::CycleMeasurementBits => {
            let bits = match state.project.template.measurement_precision() {
                8 => 10,
                10 => 16,
                _ => 8,
            };
            super::editing::commit_authoring_command(
                state,
                clothcraft::authoring::Command::SetMeasurementBits(bits),
                format!("Measurement precision: {bits} bits"),
            );
        }
        RunnerUiOp::ToggleSettings => {
            state.settings_open = !state.settings_open;
            if state.settings_open {
                state.last_2d_click = None;
                state.command_palette_open = false;
                state.solver_menu_open = false;
            }
        }
        RunnerUiOp::CloseSettings => state.settings_open = false,
        RunnerUiOp::Adjust2dZoomSpeed(delta) => {
            if delta.is_finite() {
                state.board_2d_zoom_speed =
                    (state.board_2d_zoom_speed + delta).clamp(MIN_2D_ZOOM_SPEED, MAX_2D_ZOOM_SPEED);
            }
        }
        RunnerUiOp::Reset2dZoomSpeed => state.board_2d_zoom_speed = DEFAULT_2D_ZOOM_SPEED,
        RunnerUiOp::SetCornerMode(radius) => {
            state.corner_mode_radius = radius;
            state.corner_preview_active = true;
            state.status_message = if radius {
                "Corner mode: Radius (circular fillet)".into()
            } else {
                format!("Corner mode: Shape (tension {:.2})", state.corner_tension)
            };
        }
        RunnerUiOp::SetCornerRule(rule) => {
            state.corner_rule = rule;
            state.corner_preview_active = true;
            state.status_message = format!("Continuity rule: {:?}", rule);
        }
        RunnerUiOp::AdjustCornerRadiusMm(delta) => {
            state.corner_radius_mm = (state.corner_radius_mm + delta).clamp(1.0, 250.0);
            state.corner_preview_active = true;
            state.status_message = format!("Corner radius: {:.1} mm", state.corner_radius_mm);
        }
        RunnerUiOp::AdjustCornerTension(delta) => {
            state.corner_tension = (state.corner_tension + delta).clamp(0.05, 1.5);
            state.corner_mode_radius = false;
            state.corner_preview_active = true;
            state.status_message = format!("Corner tension: {:.2}", state.corner_tension);
        }
        RunnerUiOp::ApplyCornerRoundSelected => {
            round_selected_corner(state);
        }
        RunnerUiOp::ToggleSelectedCurve(curved) => {
            toggle_selected_curve(state, curved);
        }
        RunnerUiOp::Undo2d => {
            undo_redo_2d(state, false);
        }
        RunnerUiOp::Redo2d => {
            undo_redo_2d(state, true);
        }
        RunnerUiOp::SelectPiece(idx) => {
            if let Some(p) = state.project.template.panels.get(idx) {
                let msg = format!("Selected piece: {} ({})", p.name, p.id);
                state.selected_piece_index = idx;
                state.clear_2d_subselection();
                state.piece_selected_explicitly = true;
                state.status_message = msg;
            }
        }
        RunnerUiOp::LoadPreset(kind) => {
            let subdivisions = state
                .project
                .template
                .panels
                .first()
                .and_then(|p| p.polygon.as_ref())
                .map_or(1, |poly| poly.subdivisions);
            let next_template = basic_garment(kind, subdivisions);
            match next_template.compile() {
                Ok(compiled) => {
                    let old = state.project.template.clone();
                    state.history.record(old);
                    state.preview_template = None;
                    state.drag_2d = None;
                    state.draft_2d = None;
                    state.hovered_2d = None;
                    state.selected_piece_index = 0;
                    state.clear_2d_subselection();

                    let next_garment = Arc::new(compiled);
                    let next_state = next_garment.initial_state.clone();
                    state.project.template = next_template;
                    state.frame_all_2d();
                    state.project.state = None;
                    state.project.mesh_signature = None;
                    state.generation = state.generation.wrapping_add(1);
                    state.worker_pending = false;
                    state.playing = false;
                    state.step_count = 0;
                    state.scene = FittingScene {
                        garment: Arc::clone(&next_garment),
                        collider: Arc::clone(&state.scene.collider),
                        state: next_state.clone(),
                        quasistatic: state.scene.quasistatic,
                        next_dt: state.scene.next_dt,
                    };
                    state.initial_scene = FittingScene {
                        garment: next_garment,
                        collider: Arc::clone(&state.scene.collider),
                        state: next_state,
                        quasistatic: state.scene.quasistatic,
                        next_dt: state.scene.next_dt,
                    };
                    state.camera = clothcraft::usd_scene::default_scene_camera(
                        &state.scene.state.x,
                        Some(&state.scene.collider.surface),
                    );
                    if let Some(ref usd) = state.usd_scene {
                        let _ = clothcraft::usd_scene::update_panel_viewport_mesh(
                            usd,
                            &state.scene.garment,
                            &state.scene.state,
                        );
                    }
                    state.sync_usd_camera();
                    state.send_control_state(false);
                    state.status_message = format!("Loaded {} preset", kind.name());
                }
                Err(err) => {
                    state.status_message = format!("Preset compile error: {err}");
                }
            }
        }
        RunnerUiOp::StatusNote(msg) => {
            state.status_message = msg;
        }
        RunnerUiOp::ConfirmSew { ease } => {
            confirm_pending_sew(state, ease);
        }
        RunnerUiOp::CancelSew => {
            if matches!(state.draft_2d, Some(ToolDraft2d::SewConfirm { .. })) {
                state.draft_2d = None;
                state.status_message = "Sewing cancelled; no seam was added".into();
            }
        }
    }
}
