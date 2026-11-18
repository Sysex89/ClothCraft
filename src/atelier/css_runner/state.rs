//! Editor state and simulation/selection state helpers.

use super::super::worker::{self, Latest, Work};
use super::super::{DocumentFiles, perf};
use super::actions::RunnerAction;
use super::board::geometry::fit_2d_board;
use super::board::types::{Drag2d, Hover2dTarget, ToolDraft2d};
use super::mesh_preparation;
use clothcraft::authoring::corner_tool::CIRCLE_TENSION;
use clothcraft::authoring::{CornerRule, History};
use clothcraft::css_ui::{Theme, ViewLayoutMode};
use clothcraft::fitting::{FittingProject, FittingScene};
use clothcraft::garment::GarmentTemplate;
use clothcraft::gpu_avbd::ContactMethod;
use clothcraft::render::Camera;
use clothcraft::vbd::{VbdMetrics, VbdSettings};
use cssimpler::ActionQueue;
use cssimpler::frame_drag::FrameAlignedDrag;
use std::cell::RefCell;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, mpsc};

pub(super) const DEFAULT_2D_ZOOM_SPEED: f64 = 1.5;
pub(super) const MIN_2D_ZOOM_SPEED: f64 = 0.25;
pub(super) const MAX_2D_ZOOM_SPEED: f64 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ObjectCategory {
    #[default]
    Pattern2D,
    Sewing,
    Form3D,
    Material,
    Environment,
}

pub struct CssRunnerState {
    pub(super) actions: ActionQueue<RunnerAction>,
    pub(super) action_batch: Vec<RunnerAction>,
    pub(super) frame_drag: FrameAlignedDrag,
    pub(super) input_dirty: bool,
    pub(super) board_background: RefCell<Option<super::board::rendering::BoardBackgroundCache>>,
    pub(super) native_number_drag: bool,
    pub(super) number_drag: Arc<Mutex<super::input::number::NumberDragState>>,
    pub(super) numeric_text: Arc<Mutex<Option<(String, String)>>>,
    pub(super) numeric_input_active: Arc<AtomicBool>,
    pub smoke: bool,
    pub smoke_done: std::rc::Rc<std::cell::Cell<bool>>,
    pub usd_scene: Option<cssimpler::renderer::gpu::usd::UsdScene>,
    pub project: FittingProject,
    pub preview_template: Option<GarmentTemplate>,
    pub history: History,
    pub mesh_draft: mesh_preparation::MeshDraft,
    pub mesh_input_focus: bool,
    pub scene: FittingScene,
    pub initial_scene: FittingScene,
    pub files: DocumentFiles,
    pub settings: VbdSettings,
    pub worker_tx: mpsc::Sender<Work>,
    pub worker_latest: Latest,
    pub generation: u64,
    pub worker_pending: bool,
    pub playing: bool,
    pub step_count: u64,
    pub step_ms: f64,
    pub last_metrics: Option<VbdMetrics>,
    pub camera: Camera,
    pub theme: Theme,
    pub layout_mode: ViewLayoutMode,
    pub active_category: ObjectCategory,
    pub active_3d_tool: String,
    pub active_2d_tool: String,
    pub clean_view: bool,
    pub command_palette_open: bool,
    pub settings_open: bool,
    pub left_drawer_open: bool,
    pub selected_piece_index: usize,
    /// True only while the most recent selection targeted the whole piece
    /// (board body click, piece card, `[`/`]` or a piece action). Delete
    /// removes the panel only in that case, so repeated Delete presses after a
    /// point deletion never escalate to the panel that is always highlighted.
    pub piece_selected_explicitly: bool,
    pub selected_2d_point: Option<(String, usize)>,
    pub selected_2d_points: Vec<(String, usize)>,
    pub selected_2d_edge: Option<(String, String)>,
    pub hovered_2d: Option<Hover2dTarget>,
    pub drag_2d: Option<Drag2d>,
    pub draft_2d: Option<ToolDraft2d>,
    pub status_message: String,
    pub pointer_down: bool,
    pub(super) last_2d_click: Option<(std::time::Instant, (f32, f32), Hover2dTarget)>,
    pub dragging_3d: bool,
    pub last_pointer_pos: (f32, f32),
    pub window_size: (f32, f32),
    pub gpu_sim: bool,
    pub contact_method: ContactMethod,
    pub solver_menu_open: bool,
    pub perf_test: bool,
    pub perf: Option<perf::PerfRun>,
    pub cadence: perf::Cadence,
    pub should_exit: bool,
    pub pin_mode_contour: bool,
    pub corner_mode_radius: bool,
    pub corner_radius_mm: f64,
    pub corner_tension: f64,
    pub corner_rule: CornerRule,
    pub corner_preview_active: bool,
    pub open_accordion_idx: usize,
    pub accordion_open: [bool; 6],
    pub(super) board_2d_fit: (glam::DVec2, f64),
    pub board_2d_pan: (f64, f64),
    pub board_2d_zoom: f64,
    pub board_2d_zoom_speed: f64,
}

impl CssRunnerState {
    pub fn new(
        project: FittingProject,
        scene: FittingScene,
        files: DocumentFiles,
        worker_tx: mpsc::Sender<Work>,
        worker_latest: Latest,
    ) -> Self {
        let settings = project.simulation.clone().unwrap_or_default();
        let initial_scene = scene.clone();
        let mesh_draft = mesh_preparation::MeshDraft::from_panel(project.template.panels.first());
        let camera = clothcraft::usd_scene::default_scene_camera(
            &scene.state.x,
            Some(&scene.collider.surface),
        );

        let board_2d_fit = fit_2d_board(&project.template);
        Self {
            actions: ActionQueue::new(),
            action_batch: Vec::new(),
            frame_drag: FrameAlignedDrag::new(),
            input_dirty: false,
            board_background: RefCell::new(None),
            native_number_drag: false,
            number_drag: Arc::new(Mutex::new(Default::default())),
            numeric_text: Arc::new(Mutex::new(None)),
            numeric_input_active: Arc::new(AtomicBool::new(false)),
            smoke: false,
            smoke_done: Default::default(),
            usd_scene: None,
            project,
            preview_template: None,
            history: History::default(),
            mesh_draft,
            mesh_input_focus: false,
            scene,
            initial_scene,
            files,
            settings,
            worker_tx,
            worker_latest,
            generation: 1,
            worker_pending: false,
            playing: false,
            step_count: 0,
            step_ms: 0.0,
            last_metrics: None,
            camera,
            theme: Theme::Dark,
            layout_mode: ViewLayoutMode::Split,
            active_category: ObjectCategory::Pattern2D,
            active_3d_tool: "Gizmo".to_string(),
            active_2d_tool: "Edit".to_string(),
            clean_view: false,
            command_palette_open: false,
            settings_open: false,
            left_drawer_open: true,
            selected_piece_index: 0,
            piece_selected_explicitly: false,
            selected_2d_point: None,
            selected_2d_points: Vec::new(),
            selected_2d_edge: None,
            hovered_2d: None,
            drag_2d: None,
            draft_2d: None,
            status_message: "Ready".to_string(),
            pointer_down: false,
            last_2d_click: None,
            dragging_3d: false,
            last_pointer_pos: (0.0, 0.0),
            window_size: (1600.0, 980.0),
            gpu_sim: false,
            contact_method: ContactMethod::Ipc,
            solver_menu_open: false,
            perf_test: false,
            perf: None,
            cadence: Default::default(),
            should_exit: false,
            pin_mode_contour: true,
            corner_mode_radius: true,
            corner_radius_mm: 10.0,
            corner_tension: CIRCLE_TENSION,
            corner_rule: CornerRule::Smooth,
            corner_preview_active: false,
            open_accordion_idx: 0,
            accordion_open: [true, false, true, false, false, false],
            board_2d_fit,
            board_2d_pan: (0.0, 0.0),
            board_2d_zoom: 1.0,
            board_2d_zoom_speed: DEFAULT_2D_ZOOM_SPEED,
        }
    }

    pub(super) fn frame_all_2d(&mut self) {
        self.board_2d_fit = fit_2d_board(&self.project.template);
        self.board_2d_pan = (0.0, 0.0);
        self.board_2d_zoom = 1.0;
    }

    pub fn sync_selected_point(&mut self) {
        self.selected_2d_point = self.selected_2d_points.last().cloned();
    }

    pub fn set_single_2d_point(&mut self, panel_id: String, point_idx: usize) {
        self.selected_2d_points.clear();
        self.selected_2d_points.push((panel_id.clone(), point_idx));
        self.selected_2d_point = Some((panel_id, point_idx));
        self.selected_2d_edge = None;
        self.piece_selected_explicitly = false;
    }

    pub fn toggle_2d_point(&mut self, panel_id: String, point_idx: usize) {
        self.piece_selected_explicitly = false;
        if self.selected_2d_points.is_empty()
            && let Some(prev) = self.selected_2d_point.take()
        {
            self.selected_2d_points.push(prev);
        }
        if let Some(pos) = self
            .selected_2d_points
            .iter()
            .position(|(p, i)| p == &panel_id && *i == point_idx)
        {
            self.selected_2d_points.remove(pos);
        } else {
            self.selected_2d_points.push((panel_id, point_idx));
        }
        self.selected_2d_edge = None;
        self.sync_selected_point();
    }

    pub fn clear_2d_subselection(&mut self) {
        self.piece_selected_explicitly = false;
        self.selected_2d_points.clear();
        self.selected_2d_point = None;
        self.selected_2d_edge = None;
        self.corner_preview_active = false;
    }

    pub fn active_template(&self) -> &GarmentTemplate {
        self.preview_template
            .as_ref()
            .unwrap_or(&self.project.template)
    }

    pub fn send_control_state(&mut self, once: bool) {
        let work = Work {
            generation: self.generation,
            scene: self.scene.clone(),
            settings: self.settings.clone(),
            grab: None,
            gpu: self.gpu_sim,
            contact_method: self.contact_method,
            signal: worker::RepaintSignal::noop(),
            playing: self.playing,
            once,
        };
        if self.worker_tx.send(work).is_ok() && (self.playing || once) {
            self.worker_pending = true;
        }
    }

    pub fn trigger_worker_step(&mut self, once: bool) {
        self.send_control_state(once);
    }

    pub fn sync_usd_camera(&mut self) {
        if let Some(scene) = &self.usd_scene
            && let Err(error) = clothcraft::usd_scene::update_viewport_camera(scene, &self.camera)
        {
            self.status_message = format!("USD camera error: {error}");
        }
    }

    pub fn sync_usd_points(&mut self) {
        if let Some(scene) = &self.usd_scene {
            for panel in &self.scene.garment.panels {
                if let Err(error) = clothcraft::usd_scene::update_viewport_points(
                    scene,
                    &clothcraft::usd_scene::viewport_panel_path(&panel.id),
                    &self.scene.state.x[panel.vertices.clone()],
                ) {
                    self.status_message = format!("USD update error: {error}");
                }
            }
        }
    }

    /// Transfer the last displayed accepted x/v/time; discard in-flight results
    /// by generation. Playback pauses; rest geometry and document history stay intact.
    pub fn select_solver(&mut self, gpu: bool) {
        self.select_solver_method(gpu, ContactMethod::Ipc);
    }
    pub fn select_solver_method(&mut self, gpu: bool, method: ContactMethod) {
        self.solver_menu_open = false;
        if self.gpu_sim == gpu && self.contact_method == method {
            return;
        }
        self.gpu_sim = gpu;
        self.contact_method = method;
        self.generation = self.generation.wrapping_add(1);
        self.playing = false;
        self.worker_pending = false;
        self.last_metrics = None;
        self.step_ms = 0.0;
        self.status_message =
            "Solver changed; paused at displayed state. Press Play or Step.".into();
        self.send_control_state(false);
    }

    pub fn reset_simulation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.scene = self.initial_scene.clone();
        self.step_count = 0;
        self.step_ms = 0.0;
        self.last_metrics = None;
        self.worker_pending = false;
        self.playing = false;
        self.status_message = "Reset simulation state".to_string();
        self.sync_usd_points();
        self.send_control_state(false);
    }
}
