//! Reviewed DXF, SVG and static-avatar imports for the fitting workspace.
//!
//! Parsing is CPU-only and never mutates the document. A review modal holds the
//! parsed source, the explicit outline selection and the conversion losses until
//! Apply installs one prepared authoring edit or one prepared avatar scene.

use super::editing;
use super::mesh_preparation;
use super::state::CssRunnerState;
use clothcraft::authoring::{Edit, History};
use clothcraft::avatar_import::{self, Candidate};
use clothcraft::garment::dxf_import::{Drawing as DxfDrawing, Options as DxfOptions};
use clothcraft::garment::meshing::MeshingSettings;
use clothcraft::garment::svg_import::{self, Drawing as SvgDrawing, Options as SvgOptions};
use clothcraft::pattern_import::{self, Source};
use glam::{DVec2, DVec3};
use std::{
    collections::BTreeSet,
    io::Read,
    path::{Path, PathBuf},
};

const SVG_MAX_BYTES: u64 = 3 * 1024 * 1024;
const SVG_MAX_POINTS: usize = 4096;
const SVG_MAX_CENTRE_DISTANCE_M: f64 = 1000.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportKind {
    Dxf,
    Svg,
    Avatar,
}

impl ImportKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dxf => "DXF",
            Self::Svg => "SVG",
            Self::Avatar => "Avatar",
        }
    }
}

/// One selectable source outline in the review modal.
#[derive(Clone, Debug)]
pub struct ImportRow {
    pub label: String,
    pub detail: String,
    pub points: Vec<DVec2>,
    pub closed: bool,
    pub issue: Option<String>,
}

/// Native review state for one import operation.
pub struct ImportDialog {
    pub kind: ImportKind,
    pub source: Option<PathBuf>,
    pub error: Option<String>,
    pub summary: String,
    pub issues: Vec<String>,
    pub losses: Vec<String>,
    pub rows: Vec<ImportRow>,
    pub selected: BTreeSet<usize>,
    pub acknowledged: bool,
    pub fabric: String,
    pub candidate: Option<Candidate>,
    dxf_source: Option<Source>,
    dxf_drawing: Option<DxfDrawing>,
    svg_text: Option<String>,
    svg_drawing: Option<SvgDrawing>,
    reviewed_tolerance_mm: f64,
    reviewed_scale_mm_per_unit: f64,
}

fn millimetres_to_metres(value: f64) -> f64 {
    value * 0.001
}

fn outline_bounds(points: &[DVec2]) -> Option<(DVec2, DVec2)> {
    let mut lo = DVec2::splat(f64::INFINITY);
    let mut hi = DVec2::splat(f64::NEG_INFINITY);
    let mut any = false;
    for p in points {
        if !p.is_finite() {
            continue;
        }
        lo = lo.min(*p);
        hi = hi.max(*p);
        any = true;
    }
    any.then_some((lo, hi))
}

fn dimensions(points: &[DVec2]) -> String {
    match outline_bounds(points) {
        Some((lo, hi)) => {
            let size = (hi - lo) * 1000.;
            format!("{:.1} × {:.1} mm", size.x, size.y)
        }
        None => "no finite geometry".into(),
    }
}

/// The prepared import needs the parsed drawing and the exact parameters that
/// produced it. These checks mirror the CPU adapters and never guess.
fn svg_outline_issue(outline: &svg_import::Outline) -> Option<String> {
    if !(3..=SVG_MAX_POINTS).contains(&outline.points.len()) {
        return Some(format!(
            "Needs 3..{SVG_MAX_POINTS} contour points, has {}",
            outline.points.len()
        ));
    }
    if outline
        .points
        .iter()
        .any(|p| !p.is_finite() || p.abs().max_element() > SVG_MAX_CENTRE_DISTANCE_M)
    {
        return Some("Nonfinite or out-of-range contour coordinate".into());
    }
    None
}

impl ImportDialog {
    pub fn new(kind: ImportKind) -> Self {
        Self {
            kind,
            source: None,
            error: None,
            summary: String::new(),
            issues: Vec::new(),
            losses: Vec::new(),
            rows: Vec::new(),
            selected: BTreeSet::new(),
            acknowledged: false,
            fabric: String::new(),
            candidate: None,
            dxf_source: None,
            dxf_drawing: None,
            svg_text: None,
            svg_drawing: None,
            reviewed_tolerance_mm: 0.1,
            reviewed_scale_mm_per_unit: 0.0,
        }
    }

    /// Load a source file and refresh its preview. Errors keep the modal open.
    pub fn choose(&mut self, path: &Path, tolerance_mm: f64, scale_mm_per_unit: f64) {
        self.error = None;
        self.summary.clear();
        self.issues.clear();
        self.losses.clear();
        self.rows.clear();
        self.selected.clear();
        self.acknowledged = false;
        self.candidate = None;
        self.dxf_source = None;
        self.dxf_drawing = None;
        self.svg_text = None;
        self.svg_drawing = None;
        self.source = Some(path.to_owned());
        match self.kind {
            ImportKind::Dxf => match Source::load(path) {
                Ok(source) => {
                    self.dxf_source = Some(source);
                    self.refresh(tolerance_mm, scale_mm_per_unit);
                }
                Err(error) => self.error = Some(error),
            },
            ImportKind::Svg => match read_svg_source(path) {
                Ok(text) => {
                    self.svg_text = Some(text);
                    self.refresh(tolerance_mm, scale_mm_per_unit);
                }
                Err(error) => self.error = Some(error),
            },
            ImportKind::Avatar => match Candidate::load(path) {
                Ok(candidate) => {
                    self.summary = format!(
                        "{} vertices · {} triangles",
                        candidate.vertices, candidate.triangles
                    );
                    self.candidate = Some(candidate);
                }
                Err(error) => self.error = Some(error),
            },
        }
    }

    /// Re-parse the stored source with the current tolerance/scale.
    pub fn refresh(&mut self, tolerance_mm: f64, scale_mm_per_unit: f64) {
        self.error = None;
        self.summary.clear();
        self.dxf_drawing = None;
        self.svg_drawing = None;
        self.rows.clear();
        self.selected.clear();
        self.acknowledged = false;
        self.issues.clear();
        self.losses.clear();
        if self.source.is_none() {
            self.error = Some("Choose a file first".into());
            return;
        }
        if !(tolerance_mm.is_finite() && tolerance_mm > 0.0) {
            self.error = Some("Curve tolerance must be positive and finite".into());
            return;
        }
        let result = match self.kind {
            ImportKind::Dxf => self.refresh_dxf(tolerance_mm, scale_mm_per_unit),
            ImportKind::Svg => self.refresh_svg(tolerance_mm),
            ImportKind::Avatar => return,
        };
        match result {
            Ok(()) => {
                self.reviewed_tolerance_mm = tolerance_mm;
                self.reviewed_scale_mm_per_unit = scale_mm_per_unit;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn refresh_dxf(&mut self, tolerance_mm: f64, scale_mm_per_unit: f64) -> Result<(), String> {
        let scale = (scale_mm_per_unit.is_finite() && scale_mm_per_unit > 0.0)
            .then_some(millimetres_to_metres(scale_mm_per_unit));
        let options = DxfOptions {
            metres_per_unit: scale,
            chord_error_m: millimetres_to_metres(tolerance_mm),
        };
        let drawing = self
            .dxf_source
            .as_ref()
            .ok_or("Choose a DXF file first")?
            .inspect(options)?;
        self.summary = format!(
            "{} paths · {} unsupported entities · {:.6} mm/unit{}",
            drawing.paths.len(),
            drawing.issues.len(),
            drawing.metres_per_unit * 1000.,
            if drawing.scale_overridden {
                " (override)"
            } else {
                " (file units)"
            }
        );
        self.issues = drawing
            .issues
            .iter()
            .map(|issue| format!("{} · {}: {}", issue.layer, issue.entity, issue.reason))
            .collect();
        self.rows = drawing
            .paths
            .iter()
            .enumerate()
            .map(|(index, path)| ImportRow {
                label: format!("{} · {} · {}", index + 1, path.layer, path.entity),
                detail: format!(
                    "{} points · {}{}",
                    path.points.len(),
                    dimensions(&path.points),
                    if path.closed { "" } else { " · open" }
                ),
                points: path.points.clone(),
                closed: path.closed,
                issue: pattern_import::contour_issue(path).map(str::to_owned),
            })
            .collect();
        self.dxf_drawing = Some(drawing);
        Ok(())
    }

    fn refresh_svg(&mut self, tolerance_mm: f64) -> Result<(), String> {
        let drawing = svg_import::read_svg(
            self.svg_text.as_deref().ok_or("Choose an SVG file first")?,
            SvgOptions {
                chord_error_m: millimetres_to_metres(tolerance_mm),
            },
        )?;
        self.summary = format!(
            "{} closed outlines · physical dimensions in millimetres",
            drawing.outlines.len()
        );
        self.losses = drawing.losses.clone();
        self.rows = drawing
            .outlines
            .iter()
            .enumerate()
            .map(|(index, outline)| ImportRow {
                label: format!("{} · {}", index + 1, outline.name),
                detail: format!(
                    "{} points · {}",
                    outline.points.len(),
                    dimensions(&outline.points)
                ),
                points: outline.points.clone(),
                closed: true,
                issue: svg_outline_issue(outline),
            })
            .collect();
        self.svg_drawing = Some(drawing);
        Ok(())
    }

    /// Whether the displayed preview used the current parameters.
    pub fn current(&self, tolerance_mm: f64, scale_mm_per_unit: f64) -> bool {
        let parsed = self.dxf_drawing.is_some() || self.svg_drawing.is_some();
        parsed
            && self.reviewed_tolerance_mm == tolerance_mm
            && self.reviewed_scale_mm_per_unit == scale_mm_per_unit
    }

    pub fn drawing(&self) -> bool {
        self.dxf_drawing.is_some() || self.svg_drawing.is_some()
    }

    pub fn toggle(&mut self, index: usize) {
        if self.rows.get(index).is_none_or(|row| row.issue.is_some()) {
            return;
        }
        if !self.selected.insert(index) {
            self.selected.remove(&index);
        }
        self.acknowledged = false;
    }

    pub fn selected_count(&self) -> usize {
        self.selected.len()
    }

    pub fn preview_bounds(&self) -> Option<(DVec2, DVec2)> {
        let mut lo = DVec2::splat(f64::INFINITY);
        let mut hi = DVec2::splat(f64::NEG_INFINITY);
        let mut any = false;
        for row in &self.rows {
            if let Some((a, b)) = outline_bounds(&row.points) {
                lo = lo.min(a);
                hi = hi.max(b);
                any = true;
            }
        }
        any.then_some((lo, hi))
    }

    /// Prepare the reviewed panels as one undoable authoring edit.
    pub fn prepare_edit(
        &self,
        template: &clothcraft::garment::GarmentTemplate,
        origin: DVec3,
        tolerance_mm: f64,
        scale_mm_per_unit: f64,
        mesh_mm: f64,
    ) -> Result<Edit, String> {
        if !self.current(tolerance_mm, scale_mm_per_unit) {
            return Err("Review the current parameters and outlines before importing".into());
        }
        if !self.acknowledged {
            return Err("Acknowledge the import scope and losses before importing".into());
        }
        if self.selected.is_empty() {
            return Err("Choose at least one closed outline".into());
        }
        let meshing = MeshingSettings {
            target_edge_m: millimetres_to_metres(mesh_mm),
            ..Default::default()
        };
        meshing.validate()?;
        match self.kind {
            ImportKind::Dxf => pattern_import::prepare(
                template,
                self.dxf_drawing.as_ref().ok_or("No reviewed DXF drawing")?,
                &self.selected.iter().copied().collect::<Vec<_>>(),
                &self.fabric,
                meshing,
                origin,
            ),
            ImportKind::Svg => svg_import::prepare(
                template,
                self.svg_drawing.as_ref().ok_or("No reviewed SVG drawing")?,
                &self.selected.iter().copied().collect::<Vec<_>>(),
                &self.fabric,
                meshing,
                origin,
            ),
            ImportKind::Avatar => Err("Choose an avatar file first".into()),
        }
    }
}

fn read_svg_source(path: &Path) -> Result<String, String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(SVG_MAX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() as u64 > SVG_MAX_BYTES {
        return Err("SVG exceeds the 3 MiB input limit".into());
    }
    Ok(text)
}

/// Separate the imported row along X from both the current drape and the avatar.
pub fn pattern_arrangement_origin(
    project: &clothcraft::fitting::FittingProject,
    scene: &clothcraft::fitting::FittingScene,
) -> DVec3 {
    let right = scene
        .state
        .x
        .iter()
        .chain(scene.collider.surface.vertices.iter())
        .map(|p| p.x)
        .fold(0., f64::max);
    let thickness = project
        .template
        .fabrics
        .iter()
        .map(|f| f.properties.thickness)
        .fold(0., f64::max);
    DVec3::new(right + 0.2 + scene.collider.clearance + thickness, 1.5, 0.5)
}

impl CssRunnerState {
    pub fn open_import_dialog(&mut self, kind: ImportKind) {
        let path = match kind {
            ImportKind::Dxf => rfd::FileDialog::new()
                .set_title("Import DXF patterns")
                .add_filter("DXF", &["dxf"])
                .pick_file(),
            ImportKind::Svg => rfd::FileDialog::new()
                .set_title("Import SVG patterns")
                .add_filter("SVG", &["svg"])
                .pick_file(),
            ImportKind::Avatar => rfd::FileDialog::new()
                .set_title("Import static avatar")
                .add_filter("Static avatar", &["obj", "gltf", "glb"])
                .pick_file(),
        };
        match path {
            Some(path) => self.open_import_path(kind, &path),
            None => {
                self.status_message = format!("{} import cancelled", kind.label());
            }
        }
    }

    /// Path-based entry point used by the native picker and headless tests.
    pub fn open_import_path(&mut self, kind: ImportKind, path: &Path) {
        self.settings_open = false;
        self.command_palette_open = false;
        self.solver_menu_open = false;
        self.playing = false;
        self.send_control_state(false);
        let mut dialog = ImportDialog::new(kind);
        dialog.fabric = editing::default_fabric_id(self);
        dialog.choose(
            path,
            self.import_tolerance_mm,
            self.import_scale_mm_per_unit,
        );
        self.status_message = match &dialog.error {
            Some(error) => format!("{} import error: {error}", kind.label()),
            None => format!("Review {} import from {}", kind.label(), path.display()),
        };
        self.import_dialog = Some(dialog);
    }

    pub fn refresh_import(&mut self) {
        let (tolerance, scale) = (self.import_tolerance_mm, self.import_scale_mm_per_unit);
        if let Some(dialog) = &mut self.import_dialog {
            dialog.refresh(tolerance, scale);
        }
    }

    pub fn toggle_import_outline(&mut self, index: usize) {
        if let Some(dialog) = &mut self.import_dialog {
            dialog.toggle(index);
        }
    }

    pub fn set_import_fabric(&mut self, fabric: String) {
        if let Some(dialog) = &mut self.import_dialog {
            dialog.fabric = fabric;
        }
    }

    pub fn toggle_import_acknowledged(&mut self) {
        if let Some(dialog) = &mut self.import_dialog {
            dialog.acknowledged = !dialog.acknowledged;
        }
    }

    pub fn close_import_dialog(&mut self) {
        if self.import_dialog.take().is_some() {
            self.status_message = "Import cancelled".into();
        }
    }

    pub fn apply_import(&mut self) {
        let Some(dialog) = self.import_dialog.take() else {
            return;
        };
        if dialog.kind == ImportKind::Avatar {
            self.apply_avatar_import(dialog);
        } else {
            self.apply_panel_import(dialog);
        }
    }

    fn apply_panel_import(&mut self, dialog: ImportDialog) {
        let origin = pattern_arrangement_origin(&self.project, &self.scene);
        let result = dialog.prepare_edit(
            &self.project.template,
            origin,
            self.import_tolerance_mm,
            self.import_scale_mm_per_unit,
            self.import_mesh_mm,
        );
        match result {
            Ok(edit) => {
                let old = self.project.template.clone();
                let added = edit.template.panels.len().saturating_sub(old.panels.len());
                let issues = dialog.issues.len() + dialog.losses.len();
                let mut message = format!(
                    "Imported {added} {} panel(s) as separated, unsewn pieces",
                    dialog.kind.label()
                );
                if issues > 0 {
                    message.push_str(&format!("; {issues} excluded or converted item(s)"));
                }
                self.files.modified = true;
                editing::install_authoring_edit(self, edit, Some(old), message);
            }
            Err(error) => {
                let mut dialog = dialog;
                dialog.error = Some(error);
                self.import_dialog = Some(dialog);
            }
        }
    }

    fn apply_avatar_import(&mut self, dialog: ImportDialog) {
        let Some(candidate) = &dialog.candidate else {
            let mut dialog = dialog;
            dialog.error = Some("Choose an avatar file first".into());
            self.import_dialog = Some(dialog);
            return;
        };
        let result = candidate.prepare(&self.project, self.settings.clone());
        match result {
            Ok(prepared) => {
                let source = dialog.source.clone().unwrap_or_default();
                let vertices = candidate.vertices;
                let triangles = candidate.triangles;
                self.install_avatar(prepared);
                self.status_message = format!(
                    "Replaced avatar from {} ({} vertices, {} triangles); drape reset and history cleared",
                    source.display(),
                    vertices,
                    triangles
                );
            }
            Err(error) => {
                let mut dialog = dialog;
                dialog.error = Some(error);
                self.import_dialog = Some(dialog);
            }
        }
    }

    fn install_avatar(&mut self, prepared: avatar_import::Prepared) {
        let (project, scene, asset) = prepared.into_parts();
        self.project = project;
        self.scene = scene;
        self.initial_scene = self.scene.clone();
        self.staged_avatar = Some(asset);
        self.history = History::default();
        self.files.modified = true;
        self.generation = self.generation.wrapping_add(1);
        self.worker_pending = false;
        self.playing = false;
        self.step_count = 0;
        self.step_ms = 0.0;
        self.last_metrics = None;
        self.clear_2d_subselection();
        self.selected_piece_index = 0;
        self.mesh_draft =
            mesh_preparation::MeshDraft::from_panel(self.project.template.panels.first());
        self.preview_template = None;
        self.draft_2d = None;
        self.drag_2d = None;
        self.pointer_down = false;
        if let Some(usd) = &self.usd_scene {
            if let Err(error) = clothcraft::usd_scene::update_viewport_surface(
                usd,
                &self.scene.collider.surface,
                self.shading,
            ) {
                self.status_message = format!("USD collider update error: {error}");
            }
            if let Err(error) = clothcraft::usd_scene::update_panel_viewport_positions(
                usd,
                &self.scene.garment,
                &self.scene.state,
                self.shading,
            ) {
                self.status_message = format!("USD garment update error: {error}");
            }
        }
        self.import_dialog = None;
        self.send_control_state(false);
    }
}
