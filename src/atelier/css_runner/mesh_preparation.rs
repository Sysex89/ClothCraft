//! Draft-only controls for static material-space preparation in the native runner.
use super::actions::{RunnerAction, queue_ui_op};
use super::{CssRunnerState, RunnerUiOp};
use clothcraft::garment::{GarmentTemplate, template::Panel};
use clothcraft::mesh_resolution::{features::MeshFeature, preparation};
use cssimpler::core::ElementNode as Element;
use cssimpler::{core::Node, ui};

pub struct MeshDraft {
    pub level: u8,
    pub open: bool,
    // Uniform mm, E warp/weft Pa, boundary ratio, transition mm, fold angle/offset mm,
    // gathering ratio/added Pa, down-fill ratio/pressure Pa/reference Pa.
    pub fields: [String; 13],
    // All panels, smooth transitions, fold endpoints, parallel rails, gathering, down-fill.
    pub options: [bool; 6],
    pub path: String,
    pub panel_id: Option<String>,
}
impl Default for MeshDraft {
    fn default() -> Self {
        Self {
            level: 8,
            open: false,
            fields: [
                "25", "110000", "110000", "0.5", "40", "90", "0.5", "0.5", "0", "0.95", "0",
                "1000", "0",
            ]
            .map(str::to_owned),
            options: [true, true, false, false, false, false],
            path: String::new(),
            panel_id: None,
        }
    }
}
impl MeshDraft {
    /// Logarithmic stops: coarse = 4x spacing, middle = estimate, fine = 0.25x.
    pub fn scale(&self) -> f64 {
        2_f64.powf(2.0 - f64::from(self.level) / 4.0)
    }
    fn number(&self, i: usize) -> Result<f64, String> {
        self.fields[i]
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| "Enter finite numeric mesh settings".into())
    }
    pub fn from_panel(panel: Option<&Panel>) -> Self {
        let mut draft = Self {
            panel_id: panel.map(|p| p.id.clone()),
            ..Self::default()
        };
        if let Some(poly) = panel.and_then(|p| p.polygon.as_ref()) {
            let settings = poly.meshing_settings();
            draft.fields[0] = (settings.target_edge_m * 1000.0).to_string();
            if let Some(plan) = settings.physics_sizing {
                draft.level = ((2.0 - plan.spacing_scale.log2()) * 4.0)
                    .round()
                    .clamp(0.0, 16.0) as u8;
                draft.fields[1] = plan.young_pa[0].to_string();
                draft.fields[2] = plan.young_pa[1].to_string();
                draft.fields[3] = plan.boundary_ratio.to_string();
                draft.fields[4] = (plan.transition_m * 1000.0).to_string();
                draft.options[1] = plan.smooth_transitions;
            }
            let path = poly.edges.first().cloned().unwrap_or_default();
            draft.choose_path(panel, path);
        }
        draft
    }
    pub fn choose_path(&mut self, panel: Option<&Panel>, path: String) {
        self.path = path;
        self.options[2..].fill(false);
        if let Some(plan) = panel
            .and_then(|p| p.polygon.as_ref())
            .and_then(|p| p.meshing.as_ref())
            .and_then(|m| m.physics_sizing.as_ref())
        {
            for feature in plan.features.iter().filter(|f| f.path() == self.path) {
                match feature {
                    MeshFeature::Fold {
                        endpoints,
                        parallel_lines,
                        angle_degrees,
                        offset_m,
                        ..
                    } => {
                        self.options[2] = *endpoints;
                        self.options[3] = *parallel_lines;
                        self.fields[5] = angle_degrees.to_string();
                        self.fields[6] = (offset_m * 1000.0).to_string();
                    }
                    MeshFeature::Gathering {
                        sewn_rest_ratio,
                        added_young_pa,
                        ..
                    } => {
                        self.options[4] = true;
                        self.fields[7] = sewn_rest_ratio.to_string();
                        self.fields[8] = added_young_pa[0].to_string();
                        self.fields[12] = added_young_pa[1].to_string();
                    }
                    MeshFeature::DownFilled {
                        sewn_rest_ratio,
                        pressure_pa,
                        pressure_reference_pa,
                        ..
                    } => {
                        self.options[5] = true;
                        self.fields[9] = sewn_rest_ratio.to_string();
                        self.fields[10] = pressure_pa.to_string();
                        self.fields[11] = pressure_reference_pa.to_string();
                    }
                }
            }
        }
    }
    pub fn template(
        &self,
        source: &GarmentTemplate,
        physics: bool,
        selected: Option<&str>,
    ) -> Result<GarmentTemplate, String> {
        let ids: Vec<_> = source
            .panels
            .iter()
            .filter(|p| p.polygon.is_some() && (self.options[0] || Some(p.id.as_str()) == selected))
            .map(|p| p.id.clone())
            .collect();
        if ids.is_empty() {
            return Err("Choose an editable polygon piece".into());
        }
        let mut next = if physics {
            preparation::configure(
                source,
                &ids,
                [self.number(1)?, self.number(2)?],
                self.number(3)?,
                self.number(4)? * 0.001,
                true,
            )?
        } else {
            source.clone()
        };
        for panel in &mut next.panels {
            if !ids.contains(&panel.id) {
                continue;
            }
            if let Some(poly) = &mut panel.polygon {
                let mut settings = poly.meshing_settings();
                if physics {
                    let plan = settings.physics_sizing.as_mut().unwrap();
                    plan.spacing_scale = self.scale();
                    plan.smooth_transitions = self.options[1];
                    plan.features = source
                        .panels
                        .iter()
                        .find(|p| p.id == panel.id)
                        .and_then(|p| p.polygon.as_ref())
                        .and_then(|p| p.meshing.as_ref())
                        .and_then(|m| m.physics_sizing.as_ref())
                        .map(|p| p.features.clone())
                        .unwrap_or_default();
                    if Some(panel.id.as_str()) == selected && !self.path.is_empty() {
                        // Replace only this authored path; retain other pieces and paths.
                        plan.features.retain(|f| f.path() != self.path);
                        if self.options[2] || self.options[3] {
                            plan.features.push(MeshFeature::Fold {
                                path: self.path.clone(),
                                endpoints: self.options[2],
                                parallel_lines: self.options[3],
                                angle_degrees: self.number(5)?,
                                offset_m: self.number(6)? * 0.001,
                            });
                        }
                        if self.options[4] {
                            plan.features.push(MeshFeature::Gathering {
                                path: self.path.clone(),
                                sewn_rest_ratio: self.number(7)?,
                                added_young_pa: [self.number(8)?, self.number(12)?],
                            });
                        }
                        if self.options[5] {
                            plan.features.push(MeshFeature::DownFilled {
                                path: self.path.clone(),
                                sewn_rest_ratio: self.number(9)?,
                                pressure_pa: self.number(10)?,
                                pressure_reference_pa: self.number(11)?,
                            });
                        }
                    }
                } else {
                    settings.physics_sizing = None;
                    settings.target_edge_m = self.number(0)? * 0.001 * self.scale();
                }
                settings.validate()?;
                poly.meshing = Some(settings);
            }
        }
        Ok(next)
    }
}

pub fn apply(state: &mut CssRunnerState, physics: bool) -> Result<(), String> {
    if state.playing
        || state.worker_pending
        || state.draft_2d.is_some()
        || state.drag_2d.is_some()
        || state.preview_template.is_some()
    {
        return Err(
            "Pause, wait for the current step, and finish or cancel the active edit".into(),
        );
    }
    let mut candidate = state.project.clone();
    candidate.template = state.mesh_draft.template(
        &candidate.template,
        physics,
        candidate
            .template
            .panels
            .get(state.selected_piece_index)
            .map(|p| p.id.as_str()),
    )?;
    candidate.state = None;
    candidate.mesh_signature = None;
    candidate.prearrange = false;
    // Compile before mutating the document, history, worker generation or scene.
    let scene = candidate.compile()?;
    state.history.record(state.project.template.clone());
    state.project = candidate;
    state.scene = scene;
    state.initial_scene = state.scene.clone();
    state.generation = state.generation.wrapping_add(1);
    state.step_count = 0;
    state.step_ms = 0.0;
    state.last_metrics = None;
    state.files.modified = true;
    state.clear_2d_subselection();
    if let Some(usd) = &state.usd_scene {
        let _ = clothcraft::usd_scene::update_panel_viewport_mesh(
            usd,
            &state.scene.garment,
            &state.scene.state,
            state.shading,
        );
    }
    state.send_control_state(false);
    state.status_message = format!(
        "{} mesh prepared; reset to arrangement ({} vertices, {} triangles)",
        if physics {
            "Physics-inspired"
        } else {
            "Uniform"
        },
        state.scene.garment.mesh.uv.len(),
        state.scene.garment.mesh.triangles.len()
    );
    Ok(())
}

pub fn render(state: &CssRunnerState) -> Node {
    if !state.mesh_draft.open {
        return ui! { <div id="native-mesh-preparation" class="accordion collapsed">
            <button id="mesh-preparation-header" class="accordion-header" onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::MeshToggleOpen)}>"Mesh preparation ▸"</button>
        </div> };
    }
    let stops: Vec<Node> = (0..=16)
        .map(|level| {
            let drag = state.pointer_down;
            let ops = state.actions.clone();
            let mut node = Element::new("button")
                .with_id(format!("mesh-level-{level}"))
                .with_class(if level == state.mesh_draft.level {
                    "mesh-stop active"
                } else {
                    "mesh-stop"
                })
                .with_attribute(
                    "title",
                    format!("{:.2}x spacing", 2_f64.powf(2.0 - f64::from(level) / 4.0)),
                )
                .on_click_shared(queue_ui_op(&state.actions, RunnerUiOp::MeshLevel(level)))
                .on_mousedown_shared(queue_ui_op(&state.actions, RunnerUiOp::MeshLevel(level)))
                .on_mouseenter_shared(move || {
                    if drag {
                        ops.push(RunnerAction::Ui(RunnerUiOp::MeshLevel(level)));
                    }
                });
            node.children.push(Node::Text(
                if level == state.mesh_draft.level {
                    "|"
                } else {
                    "-"
                }
                .into(),
            ));
            Node::Element(node)
        })
        .collect();
    let labels = [
        "Uniform base spacing (mm)",
        "Warp E estimate (Pa)",
        "Weft E estimate (Pa)",
        "Boundary / base spacing",
        "Transition (mm)",
        "Fold angle (degrees)",
        "Parallel offset (mm)",
        "Gathering sewn/rest ratio",
        "Added gathering warp E (Pa)",
        "Down-fill sewn/rest ratio",
        "Sizing pressure (Pa)",
        "Pressure reference (Pa)",
        "Added gathering weft E (Pa)",
    ];
    let fields: Vec<Node> = [0,1,2,3,4,5,6,7,8,12,9,10,11].into_iter().filter(|i| *i < 5 || ((*i == 5 || *i == 6) && (state.mesh_draft.options[2] || state.mesh_draft.options[3])) || ((*i == 7 || *i == 8 || *i == 12) && state.mesh_draft.options[4]) || ((*i >= 9 && *i <= 11) && state.mesh_draft.options[5])).map(|i| {
        let label = labels[i];
        let ops = state.actions.clone();
        let input = Element::new("input").with_id(format!("mesh-value-{i}"))
            .on_mousedown_shared(queue_ui_op(&state.actions, RunnerUiOp::MeshInputFocus))
            .with_attribute("type", "text").with_attribute("value", &state.mesh_draft.fields[i])
            .on_input_shared(move |value| { ops.push(RunnerAction::Ui(RunnerUiOp::MeshField(i, value.into()))); });
        ui! { <div class="prop-row"><span class="prop-label">{label}</span>{Node::Element(input)}</div> }
    }).collect();
    let toggles: Vec<Node> = ["Scope: all polygon pieces", "Smooth refinement transitions", "Refine fold endpoints", "Add parallel fold lines", "Gathering refinement", "Down-fill sizing estimate"].iter().enumerate().map(|(i,label)| {
        ui! { <button id={format!("mesh-option-{i}")} class="mesh-toggle" onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::MeshToggle(i))}>
            {format!("{} {label}", if state.mesh_draft.options[i] { "[x]" } else { "[ ]" })}
        </button> }
    }).collect();
    let panel = state
        .project
        .template
        .panels
        .get(state.selected_piece_index);
    let paths: Vec<Node> = panel.and_then(|p| p.polygon.as_ref()).map(|p| p.edges.iter().cloned().chain(p.holes.iter().flat_map(|h| h.edges.clone())).chain(p.lines.iter().map(|l| l.id.clone())).map(|path| {
        ui! { <button class={if state.mesh_draft.path == path { "mesh-path active" } else { "mesh-path" }} onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::MeshChoosePath(path.clone()))}>{path}</button> }
    }).collect()).unwrap_or_default();
    let estimated_spacing = panel
        .and_then(|p| {
            state
                .project
                .template
                .fabrics
                .iter()
                .find(|f| f.id == p.fabric)
                .map(|f| (p, f))
        })
        .and_then(|(p, f)| {
            preparation::PhysicsSizing {
                young_pa: [
                    state.mesh_draft.number(1).ok()?,
                    state.mesh_draft.number(2).ok()?,
                ],
                bending_nm: [f.properties.bend_warp, f.properties.bend_weft],
                axis_degrees: p.grain_degrees,
                boundary_ratio: 1.0,
                transition_m: 0.04,
                seed: 42,
                spacing_scale: state.mesh_draft.scale(),
                smooth_transitions: true,
                features: vec![],
            }
            .base()
            .ok()
        })
        .map(|h| {
            format!(
                "Estimated base warp/weft: {:.2} / {:.2} mm",
                h[0] * 1000.0,
                h[1] * 1000.0
            )
        })
        .unwrap_or_else(|| "Enter valid E estimates for base spacing".into());
    let feature_summary = panel
        .and_then(|p| p.polygon.as_ref())
        .and_then(|p| p.meshing.as_ref())
        .and_then(|m| m.physics_sizing.as_ref())
        .map(|p| {
            p.features
                .iter()
                .map(|f| {
                    format!(
                        "{}: {}",
                        f.path(),
                        match f {
                            MeshFeature::Fold { .. } => "fold",
                            MeshFeature::Gathering { .. } => "gathering",
                            MeshFeature::DownFilled { .. } => "down-fill",
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let busy = state.playing
        || state.worker_pending
        || state.draft_2d.is_some()
        || state.drag_2d.is_some()
        || state.preview_template.is_some();
    let mut physics = Element::new("button")
        .with_id("prepare-physics-mesh")
        .with_class("action-btn")
        .on_click_shared(queue_ui_op(&state.actions, RunnerUiOp::PrepareMesh(true)));
    let mut uniform = Element::new("button")
        .with_id("prepare-uniform-mesh")
        .with_class("action-btn")
        .on_click_shared(queue_ui_op(&state.actions, RunnerUiOp::PrepareMesh(false)));
    if busy {
        physics = physics.with_attribute("disabled", "disabled");
        uniform = uniform.with_attribute("disabled", "disabled");
    }
    physics
        .children
        .push(Node::Text("Prepare physics mesh and reset".into()));
    uniform
        .children
        .push(Node::Text("Use uniform mesh and reset".into()));
    ui! {
        <div id="native-mesh-preparation" class="accordion open">
            <button id="mesh-preparation-header" class="accordion-header" onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::MeshToggleOpen)}>"Mesh preparation ▾"</button>
            <div class="accordion-content">
                <div class="prop-row"><span>"Coarse"</span><span>"Fine"</span></div>
                <div class="mesh-slider">{stops}</div>
                <p>{format!("{:.2}x spacing · uniform {:.2} mm", state.mesh_draft.scale(), state.mesh_draft.number(0).unwrap_or(0.0) * state.mesh_draft.scale())}</p>
                <p>{estimated_spacing}</p>
                {toggles}
                <p>{format!("Feature target: {} / {}", panel.map_or("no piece", |p| p.id.as_str()), state.mesh_draft.path)}</p>
                <div class="mesh-paths">{paths}</div>
                {fields}
                <p>{format!("Saved features: {feature_summary}")}</p>
                <button class="action-btn" onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::MeshReload)}>"Reload settings from selected piece"</button>
                <p>"Base settings apply to the chosen scope; feature switches apply to the named path on the selected piece. Applying resets the drape."</p>
                <p>"Sizing hints only: these controls do not apply pressure, shrinkage or crease forces. Ratio 1 means no compression. Parallel lines require an internal fold line."</p>
                {Node::Element(physics)}
                {Node::Element(uniform)}
                <p>{if busy { "Pause and finish or cancel edits before applying." } else { "Current mesh counts appear in the status bar." }}</p>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{DocumentFiles, worker};
    use super::super::{handle_runner_event, undo_redo_2d, update_runner};
    use super::*;
    use clothcraft::{fitting::FittingProject, garment::presets::GarmentKind};
    use cssimpler::{
        app::App,
        core::Color,
        renderer::{EngineEvent, FrameInfo},
    };
    use std::fs;
    fn fixture(dir: &std::path::Path) -> CssRunnerState {
        let avatar = dir.join("avatar.obj");
        fs::write(&avatar, "v 0 0 -10\nv 1 0 -10\nv 0 1 -10\nf 1 2 3\n").unwrap();
        let mut project = FittingProject::basic(GarmentKind::Tshirt, 0, avatar);
        project.template = GarmentTemplate::load(std::path::Path::new(
            "examples/templates/curved-panels.json",
        ))
        .unwrap();
        project.template.seams.clear();
        let scene = project.compile().unwrap();
        let (tx, latest) = worker::start_cpu().unwrap();
        CssRunnerState::new(
            project,
            scene,
            DocumentFiles {
                output: dir.join("out"),
                source: None,
                modified: false,
            },
            tx,
            latest,
        )
    }
    fn node<'a>(root: &'a Node, id: &str) -> Option<&'a Element> {
        if let Node::Element(e) = root {
            if e.id.as_deref() == Some(id) {
                return Some(e);
            }
            for child in &e.children {
                if let Some(e) = node(child, id) {
                    return Some(e);
                }
            }
        }
        None
    }
    fn drain(state: &mut CssRunnerState) {
        let mut batch = std::mem::take(&mut state.action_batch);
        state.actions.drain_into(&mut batch);
        for action in batch.drain(..) {
            super::super::actions::apply_runner_action(state, action, false);
        }
        state.action_batch = batch;
    }
    #[test]
    fn native_mesh_controls_stage_inputs_and_dispatch_real_backend_transaction() {
        let _guard = worker::WORKER_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let mut state = fixture(dir.path());
        state.mesh_draft.open = true;
        let original = serde_json::to_value(&state.project.template).unwrap();
        let ui = render(&state);
        node(&ui, "mesh-level-4").unwrap().handlers.click().unwrap()();
        node(&ui, "mesh-option-4")
            .unwrap()
            .handlers
            .click()
            .unwrap()();
        drain(&mut state);
        node(&render(&state), "mesh-value-7")
            .unwrap()
            .handlers
            .input()
            .unwrap()("0.7");
        drain(&mut state);
        assert_eq!(state.mesh_draft.scale(), 2.0);
        assert_eq!(
            original,
            serde_json::to_value(&state.project.template).unwrap()
        );
        let ui = render(&state);
        node(&ui, "prepare-physics-mesh")
            .unwrap()
            .handlers
            .click()
            .unwrap()();
        drain(&mut state);
        assert!(
            state
                .status_message
                .starts_with("Physics-inspired mesh prepared"),
            "{}",
            state.status_message
        );
        assert!(state.history.can_undo());
        assert!(state.files.modified);
        assert_eq!(state.scene.state.time, 0.0);
        let plan = state.project.template.panels[0]
            .polygon
            .as_ref()
            .unwrap()
            .meshing
            .as_ref()
            .unwrap()
            .physics_sizing
            .as_ref()
            .unwrap();
        assert_eq!(plan.spacing_scale, 2.0);
        assert!(matches!(
            plan.features[0],
            MeshFeature::Gathering {
                sewn_rest_ratio: 0.7,
                ..
            }
        ));
        let path = dir.path().join("mesh.json");
        state.project.save(&path).unwrap();
    }
    #[test]
    fn native_mesh_preparation_is_atomic_guarded_and_undoable() {
        let _guard = worker::WORKER_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let mut state = fixture(dir.path());
        let original = serde_json::to_value(&state.project.template).unwrap();
        let x = state.scene.state.x.clone();
        let generation = state.generation;
        state.mesh_draft.fields[1] = "NaN".into();
        assert!(apply(&mut state, true).is_err());
        assert_eq!(
            original,
            serde_json::to_value(&state.project.template).unwrap()
        );
        assert_eq!(x, state.scene.state.x);
        assert_eq!(generation, state.generation);
        assert!(!state.history.can_undo());
        state.mesh_draft.fields[1] = "110000".into();
        state.playing = true;
        assert!(apply(&mut state, true).is_err());
        state.playing = false;
        state.worker_pending = true;
        assert!(apply(&mut state, true).is_err());
        state.worker_pending = false;
        state.mesh_draft.level = 0;
        apply(&mut state, true).unwrap();
        assert!(
            !state.worker_pending,
            "An idle configuration message is not a pending physics step"
        );
        let prepared = serde_json::to_value(&state.project.template).unwrap();
        let path = dir.path().join("saved.json");
        state.project.save(&path).unwrap();
        let loaded = FittingProject::load(&path).unwrap();
        assert_eq!(
            loaded.compile().unwrap().garment.mesh.uv,
            state.scene.garment.mesh.uv
        );
        let draft = MeshDraft::from_panel(loaded.template.panels.first());
        assert_eq!(draft.scale(), 4.0);
        assert!(undo_redo_2d(&mut state, false));
        assert_eq!(
            original,
            serde_json::to_value(&state.project.template).unwrap()
        );
        assert!(undo_redo_2d(&mut state, true));
        assert_eq!(
            prepared,
            serde_json::to_value(&state.project.template).unwrap()
        );
    }
    #[test]
    fn native_mesh_scope_preserves_other_pieces_and_feature_paths() {
        let _guard = worker::WORKER_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let state = fixture(dir.path());
        let before = &state.project.template;
        let selected = before.panels[0].id.as_str();
        let mut draft = MeshDraft::from_panel(before.panels.first());
        draft.options[0] = false;
        draft.options[4] = true;
        let next = draft.template(before, true, Some(selected)).unwrap();
        for (a, b) in before.panels.iter().zip(&next.panels).skip(1) {
            assert_eq!(
                serde_json::to_value(a).unwrap(),
                serde_json::to_value(b).unwrap()
            );
        }
        let old_path = draft.path.clone();
        let second = before.panels[0].polygon.as_ref().unwrap().edges[1].clone();
        draft.choose_path(next.panels.first(), second);
        draft.options[5] = true;
        let next = draft.template(&next, true, Some(selected)).unwrap();
        let plan = next.panels[0]
            .polygon
            .as_ref()
            .unwrap()
            .meshing
            .as_ref()
            .unwrap()
            .physics_sizing
            .as_ref()
            .unwrap();
        assert!(plan.features.iter().any(|f| f.path() == old_path));
        assert_eq!(plan.features.len(), 2);
    }
    #[test]
    fn native_mesh_layout_renders_all_controls_and_text_input_owns_keyboard() {
        use cssimpler::renderer::{
            ButtonState, KeyIdentity, KeyboardEvent, PointerPosition, TextInputEvent, ViewportSize,
        };
        use std::time::Duration;
        let _guard = worker::WORKER_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let mut state = fixture(dir.path());
        state.mesh_draft.open = true;
        state.mesh_draft.options[2..].fill(true);
        // Isolate the production panel in a CPU-rendered, scrollable test surface.
        let css = format!(
            "{}\n.mesh-preview {{ width: 480px; height: 1500px; color: var(--text); background: var(--bg); }}",
            clothcraft::css_ui::STUDIO_V2_CSS
        );
        let stylesheet = cssimpler::style::parse_stylesheet(&css).unwrap();
        let mut app = App::new(
            state,
            &stylesheet,
            update_runner,
            |s| ui! { <div class="mesh-preview">{render(s)}</div> },
        )
        .with_event_handler(|state, event| {
            if matches!(event, EngineEvent::Key(_)) {
                handle_runner_event(state, event)
            } else {
                false
            }
        });
        app.set_viewport(ViewportSize::new(480, 1500));
        let frame = |index| FrameInfo {
            delta: Duration::from_millis(16),
            frame_index: index,
        };
        let roots = app.frame(frame(0));
        let slider = app.find_element_bounds_by_id("#mesh-level-8").unwrap();
        assert!(slider.width > 8.0 && slider.height >= 20.0);
        for id in [
            "#mesh-value-5",
            "#mesh-value-8",
            "#mesh-value-12",
            "#mesh-value-10",
            "#prepare-physics-mesh",
        ] {
            let bounds = app.find_element_bounds_by_id(id).unwrap();
            assert!(
                bounds.width > 20.0 && bounds.height > 10.0,
                "{id}: {bounds:?}"
            );
        }
        if let Ok(path) = std::env::var("CLOTHCRAFT_MESH_UI_CAPTURE") {
            let mut rgba = vec![0; 480 * 1500 * 4];
            cssimpler::renderer::render_to_rgba8(
                &roots,
                &mut rgba,
                480,
                1500,
                Color::rgb(15, 17, 23),
            );
            image::save_buffer(path, &rgba, 480, 1500, image::ColorType::Rgba8).unwrap();
        }
        let field = app.find_element_bounds_by_id("#mesh-value-1").unwrap();
        app.handle_engine_event(&EngineEvent::PointerMoved {
            position: PointerPosition {
                x: field.x + field.width - 4.0,
                y: field.y + field.height / 2.0,
            },
            modifiers: Default::default(),
        });
        app.handle_engine_event(&EngineEvent::PointerButton {
            button: cssimpler::renderer::PointerButton::Primary,
            state: ButtonState::Pressed,
            modifiers: Default::default(),
        });
        // The native input system owns text; the runner must not interpret space as Play.
        node(&render(app.state()), "mesh-value-1")
            .unwrap()
            .handlers
            .mousedown()
            .unwrap()();
        app.frame(frame(1));
        app.handle_engine_event(&EngineEvent::TextInput(TextInputEvent::Commit("7".into())));
        app.frame(frame(2));
        assert!(app.state().mesh_draft.fields[1].contains('7'));
        assert!(!handle_runner_event(
            app.state_mut(),
            &EngineEvent::Key(KeyboardEvent {
                logical_key: KeyIdentity::Character(" ".into()),
                physical_key: None,
                location: Default::default(),
                state: ButtonState::Pressed,
                repeat: false,
                modifiers: Default::default()
            })
        ));
        assert!(!app.state().playing);
    }
}
