//! Native application creation and runner entry point.

use super::super::{DocumentFiles, RunOptions, perf, usd_smoke, worker};
use super::actions::update_runner;
use super::input::handle_runner_event;
use super::state::CssRunnerState;
use super::ui::shell::build_runner_ui;
use clothcraft::css_ui::load_studio_stylesheet;
use clothcraft::fitting::{FittingProject, FittingScene};
use clothcraft::gpu_avbd::ContactMethod;
use cssimpler::app::{App, Invalidation, RenderMode};
use cssimpler::core::Node;
use cssimpler::renderer::{FrameInfo, WindowConfig};
use cssimpler::style::Stylesheet;

#[allow(clippy::type_complexity)]
pub fn create_runner_app<'a>(
    mut state: CssRunnerState,
    stylesheet: &'a Stylesheet,
) -> App<
    'a,
    CssRunnerState,
    fn(&mut CssRunnerState, FrameInfo) -> Invalidation,
    fn(&CssRunnerState) -> Node,
    Invalidation,
> {
    let preview_path = state.files.output.join("runner_preview.usda");
    if let Some(parent) = preview_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let usda = clothcraft::usd_scene::garment_stage_usda(&state.scene.garment, &state.scene.state)
        .unwrap_or_default();
    let usda = clothcraft::usd_scene::append_viewport_surface(usda, &state.scene.collider.surface);
    let _ = std::fs::write(&preview_path, usda);

    let usd_scene = cssimpler::renderer::gpu::usd::UsdScene::open(&preview_path)
        .expect("load generated USD viewport scene");
    if let Err(err) = clothcraft::usd_scene::init_viewport_camera(&usd_scene, &state.camera) {
        state.status_message = format!("USD camera init error: {err}");
    }
    state.usd_scene = Some(usd_scene.clone());

    let smoke = state.smoke;
    let smoke_done = state.smoke_done.clone();
    let smoke_output = state.files.output.join("ui.png");
    let app = App::new(
        state,
        stylesheet,
        update_runner as fn(&mut CssRunnerState, FrameInfo) -> Invalidation,
        build_runner_ui as fn(&CssRunnerState) -> Node,
    )
    .with_event_handler(handle_runner_event)
    .with_render_mode(RenderMode::OnInvalidation)
    .with_frame_aligned_input(true)
    .with_continuous_updates(true)
    .with_exit_when(|state| state.smoke_done.get() || state.should_exit);

    let cam_mode = cssimpler::renderer::gpu::usd::UsdCameraMode::Authored(
        clothcraft::usd_scene::VIEWPORT_CAMERA_PATH.into(),
    );
    if smoke {
        app.with_gpu_viewport(
            "runner-viewport-3d-scene",
            Default::default(),
            move |init| {
                let redraw = init.redraw.clone();
                let renderer =
                    cssimpler::renderer::gpu::usd::UsdRenderer::new(init, usd_scene, cam_mode)?;
                Ok(Box::new(usd_smoke::SmokeViewport {
                    renderer,
                    output: smoke_output,
                    done: smoke_done,
                    redraw,
                    frames: 0,
                }))
            },
        )
    } else {
        app.with_usd_viewport("runner-viewport-3d-scene", usd_scene, cam_mode)
    }
}

pub(super) fn init_gpu_device() -> Result<(wgpu::Device, wgpu::Queue), String> {
    clothcraft::gpu_avbd::request_device()
}

pub fn run(
    project: FittingProject,
    scene: FittingScene,
    files: DocumentFiles,
    options: RunOptions,
) -> Result<(), String> {
    let RunOptions {
        barrier_al,
        smoke,
        perf_test,
        gpu_sim,
    } = options;
    let stylesheet =
        Box::leak(Box::new(load_studio_stylesheet().map_err(|e| {
            format!("Failed to load Studio V2 stylesheet: {e}")
        })?));

    let (tx, latest) = if gpu_sim {
        let (device, queue) = init_gpu_device()?;
        worker::start(device, queue).map_err(|e| format!("Failed to start GPU worker: {e}"))?
    } else {
        worker::start_switchable().map_err(|e| format!("Failed to start CPU worker: {e}"))?
    };

    let mut state = CssRunnerState::new(project, scene, files, tx, latest);
    state.native_number_drag = !smoke;
    state.smoke = smoke;
    state.perf_test = perf_test;
    state.gpu_sim = gpu_sim;
    state.contact_method = if barrier_al {
        ContactMethod::BarrierAl
    } else {
        ContactMethod::Ipc
    };
    if perf_test {
        state.perf = Some(perf::PerfRun::new());
        state.playing = true;
        state.send_control_state(false);
    }
    let smoke_done = state.smoke_done.clone();
    let app = create_runner_app(state, stylesheet);
    let config = WindowConfig::new("ClothCraft | Simulation Studio V2", 1600, 980);
    app.run(config)
        .map_err(|e| format!("CSSimpler application error: {e}"))?;
    if smoke && !smoke_done.get() {
        return Err("Native smoke closed before USD capture completed".into());
    }
    Ok(())
}
