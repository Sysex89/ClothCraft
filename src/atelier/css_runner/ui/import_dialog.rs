//! Native review modals for DXF, SVG and static-avatar imports.

use super::super::actions::{RunnerUiOp, queue_ui_op};
use super::super::board::native::polyline;
use super::super::imports::{ImportDialog, ImportKind};
use super::super::state::CssRunnerState;
use super::number::{NumberField, number_field};
use cssimpler::core::{
    Color, LengthPercentageCalc, Node, Style, VectorPaint, VectorPath, VectorScene, VectorViewBox,
};
use cssimpler::ui;

const PREVIEW_HEIGHT: f64 = 160.0;
const PREVIEW_MAX_WIDTH: f64 = 560.0;
const PREVIEW_MIN_WIDTH: f64 = 240.0;
const PREVIEW_MAX_POINTS: usize = 512;
const MAX_REPORTED_ITEMS: usize = 40;

pub(in super::super) fn build_runner_import_dialog(state: &CssRunnerState) -> Option<Node> {
    let dialog = state.import_dialog.as_ref()?;
    let title = match dialog.kind {
        ImportKind::Dxf => "Import DXF patterns",
        ImportKind::Svg => "Import SVG patterns",
        ImportKind::Avatar => "Import static avatar",
    };
    let body = match dialog.kind {
        ImportKind::Dxf | ImportKind::Svg => panel_body(state, dialog),
        ImportKind::Avatar => avatar_body(state, dialog),
    };
    Some(ui! {
        <div id="import-layer" class="import-layer">
            <div id="import-backdrop" class="import-backdrop"
                onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::ImportCancel)} />
            <div id="import-dialog" class="import-panel" onclick_shared={|| {}}>
                <div class="import-header">
                    <span id="import-title" class="import-title">{title}</span>
                    <button id="import-close" class="cad-2d-pill"
                        onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::ImportCancel)}
                        title="Close import (Escape)">"Close"</button>
                </div>
                {body}
            </div>
        </div>
    })
}

fn source_row(state: &CssRunnerState, dialog: &ImportDialog) -> Node {
    let source = dialog
        .source
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "No file chosen".into());
    let filter = match dialog.kind {
        ImportKind::Dxf => "Generic ASCII DXF · closed outlines only",
        ImportKind::Svg => "Physical SVG · closed outlines only · positive Y down",
        ImportKind::Avatar => "Static OBJ / glTF / GLB · metres · Y-up",
    };
    ui! {
        <div class="import-section">
            <div class="import-source-row">
                <button id="import-choose" class="cad-2d-pill"
                    onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::ImportChoose)}
                    title="Choose another source file">"Choose…"</button>
                <span id="import-source" class="import-source">{source}</span>
            </div>
            <span class="import-hint">{filter}</span>
        </div>
    }
}

fn status_rows(state: &CssRunnerState, dialog: &ImportDialog) -> Node {
    let summary = dialog.summary.clone();
    let error = dialog.error.as_ref().map(|error| {
        ui! { <div id="import-error" class="import-error">{error.clone()}</div> }
    });
    let stale = dialog.drawing()
        && !dialog.current(state.import_tolerance_mm, state.import_scale_mm_per_unit);
    let stale_node = stale.then(|| {
        ui! {
            <div id="import-stale" class="import-warning">
                "Parameters changed. Update the preview and review the outlines again."
            </div>
        }
    });
    ui! {
        <div class="import-status">
            <span id="import-summary" class="import-summary">{summary}</span>
            {stale_node}
            {error}
        </div>
    }
}

fn number_row(label: &str, control: Node) -> Node {
    ui! {
        <div class="import-number-row">
            <span class="import-number-label">{label.to_owned()}</span>
            {control}
        </div>
    }
}

fn panel_body(state: &CssRunnerState, dialog: &ImportDialog) -> Node {
    let is_dxf = dialog.kind == ImportKind::Dxf;
    let scale_row = is_dxf.then(|| {
        number_row(
            "DXF mm per unit (0 = file units)",
            number_field(state, "import-scale", NumberField::ImportScale),
        )
    });
    let refresh = ui! {
        <button id="import-refresh" class="cad-2d-pill"
            onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::ImportRefresh)}
            title="Re-read the source with the current parameters">"Update preview"</button>
    };
    let footer = panel_footer(state, dialog);
    let preview = preview_node(state, dialog);
    let rows = outline_rows(state, dialog);
    let reported = reported_items(dialog);
    let fabric = fabric_selector(state, dialog);
    let acknowledgement = acknowledgement_button(state, dialog);
    let kind_hint = if is_dxf {
        "DXF +Y is shown upward and maps to decreasing material V. New pieces are separated to the right of the avatar and cloth."
    } else {
        "SVG coordinates use physical units and positive Y down. New pieces are separated to the right of the avatar and cloth."
    };
    ui! {
        <div class="import-body">
            {source_row(state, dialog)}
            <div class="import-numbers">
                {scale_row}
                {number_row(
                    "Curve chord error (mm)",
                    number_field(state, "import-tolerance", NumberField::ImportTolerance),
                )}
                {number_row(
                    "Mesh spacing (mm, 3–500)",
                    number_field(state, "import-mesh", NumberField::ImportMesh),
                )}
                {refresh}
            </div>
            {status_rows(state, dialog)}
            {preview}
            <div class="import-list-label">"Closed outlines — select the pieces to import"</div>
            <div id="import-list" class="import-list">{rows}</div>
            <span class="import-hint">{kind_hint}</span>
            {reported}
            <div class="import-section">
                <span class="import-list-label">"Existing fabric"</span>
                <div id="import-fabrics" class="import-fabric-row">{fabric}</div>
            </div>
            {acknowledgement}
            {footer}
        </div>
    }
}

fn avatar_body(state: &CssRunnerState, dialog: &ImportDialog) -> Node {
    let details = dialog.candidate.as_ref().map(|candidate| {
        let [lo, hi] = candidate.bounds;
        let size = hi - lo;
        let warnings: Vec<Node> = candidate
            .warnings
            .iter()
            .map(|warning| ui! { <div class="import-warning-line">{warning.clone()}</div> })
            .collect();
        ui! {
            <div class="import-section">
                <span id="import-summary" class="import-summary">
                    {format!("{} vertices · {} triangles", candidate.vertices, candidate.triangles)}
                </span>
                <span id="import-avatar-size" class="import-source">
                    {format!("Size X/Y/Z: {:.4} / {:.4} / {:.4} m", size.x, size.y, size.z)}
                </span>
                <span class="import-hint">
                    {format!("Min: {:.4}, {:.4}, {:.4} m · Max: {:.4}, {:.4}, {:.4} m", lo.x, lo.y, lo.z, hi.x, hi.y, hi.z)}
                </span>
                {warnings}
            </div>
        }
    });
    let error = dialog.error.as_ref().map(|error| {
        ui! { <div id="import-error" class="import-error">{error.clone()}</div> }
    });
    let footer = footer_buttons(
        state,
        dialog.candidate.is_some(),
        "Replace avatar",
        RunnerUiOp::ImportApply,
    );
    ui! {
        <div class="import-body">
            {source_row(state, dialog)}
            {details}
            {error}
            <div class="import-warning">
                "Authored patterns, seams, materials and placement are kept. Replacing the avatar resets draped positions and velocities and clears undo history. Save the current drape first if it is needed."
            </div>
            {footer}
        </div>
    }
}

fn outline_rows(state: &CssRunnerState, dialog: &ImportDialog) -> Vec<Node> {
    dialog
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let id = format!("import-row-{index}");
            if let Some(issue) = &row.issue {
                return ui! {
                    <div id={id} class="import-row disabled" title={issue.clone()}>
                        <span class="import-row-label">{row.label.clone()}</span>
                        <span class="import-row-detail">{row.detail.clone()}</span>
                        <span class="import-row-issue">{issue.clone()}</span>
                    </div>
                };
            }
            let class = if dialog.selected.contains(&index) {
                "import-row active"
            } else {
                "import-row"
            };
            ui! {
                <button id={id} class={class}
                    onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::ImportToggleOutline(index))}>
                    <span class="import-row-label">{row.label.clone()}</span>
                    <span class="import-row-detail">{row.detail.clone()}</span>
                </button>
            }
        })
        .collect()
}

fn reported_items(dialog: &ImportDialog) -> Option<Node> {
    let mut lines: Vec<Node> = Vec::new();
    let total = dialog.issues.len() + dialog.losses.len();
    if total == 0 {
        return None;
    }
    for issue in dialog.issues.iter().take(MAX_REPORTED_ITEMS) {
        lines.push(ui! { <div class="import-issue">{issue.clone()}</div> });
    }
    for loss in dialog.losses.iter().take(MAX_REPORTED_ITEMS) {
        lines.push(ui! { <div class="import-issue">{loss.clone()}</div> });
    }
    let omitted = total.saturating_sub(MAX_REPORTED_ITEMS);
    let more = (omitted > 0).then(|| {
        ui! { <div class="import-issue">{format!("… {omitted} more reported item(s)")}</div> }
    });
    Some(ui! {
        <div class="import-section">
            <span class="import-list-label">{format!("Excluded or converted ({total})")}</span>
            <div class="import-issues">{lines}{more}</div>
        </div>
    })
}

fn fabric_selector(state: &CssRunnerState, dialog: &ImportDialog) -> Vec<Node> {
    if state.project.template.fabrics.is_empty() {
        return vec![
            ui! { <span class="import-warning">"No fabric is defined in this document"</span> },
        ];
    }
    state
        .project
        .template
        .fabrics
        .iter()
        .map(|fabric| {
            let class = if dialog.fabric == fabric.id {
                "import-fabric active"
            } else {
                "import-fabric"
            };
            ui! {
                <button id={format!("import-fabric-{}", fabric.id)} class={class}
                    title={fabric.name.clone()}
                    onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::ImportSetFabric(fabric.id.clone()))}>
                    {fabric.name.clone()}
                </button>
            }
        })
        .collect()
}

fn acknowledgement_button(state: &CssRunnerState, dialog: &ImportDialog) -> Node {
    let class = if dialog.acknowledged {
        "import-check active"
    } else {
        "import-check"
    };
    ui! {
        <button id="import-ack" class={class}
            onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::ImportToggleAck)}>
            <span class="import-check-box">{if dialog.acknowledged { "✓" } else { "" }}</span>
            <span class="import-check-label">
                "Import only the selected outlines; ignore every other path and reported loss"
            </span>
        </button>
    }
}

fn panel_footer(state: &CssRunnerState, dialog: &ImportDialog) -> Node {
    let mesh_ok = (3.0..=500.0).contains(&state.import_mesh_mm);
    let ready = dialog.current(state.import_tolerance_mm, state.import_scale_mm_per_unit)
        && dialog.acknowledged
        && !dialog.selected.is_empty()
        && mesh_ok
        && dialog.error.is_none();
    footer_buttons(
        state,
        ready,
        &format!("Add {} panel(s)", dialog.selected.len()),
        RunnerUiOp::ImportApply,
    )
}

fn footer_buttons(state: &CssRunnerState, ready: bool, label: &str, apply_op: RunnerUiOp) -> Node {
    let cancel = ui! {
        <button id="import-cancel" class="cad-2d-pill"
            onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::ImportCancel)}>"Cancel"</button>
    };
    let apply = if ready {
        ui! {
            <button id="import-apply" class="btn-action btn-action-accent"
                onclick_shared={queue_ui_op(&state.actions, apply_op)}>{label.to_owned()}</button>
        }
    } else {
        ui! {
            <button id="import-apply" class="btn-action btn-action-accent disabled"
                title="Review the current parameters, choose outlines and acknowledge the import">
                {label.to_owned()}
            </button>
        }
    };
    ui! {
        <div class="import-footer">{cancel}{apply}</div>
    }
}

fn preview_node(state: &CssRunnerState, dialog: &ImportDialog) -> Option<Node> {
    let (lo, hi) = dialog.preview_bounds()?;
    let size = hi - lo;
    if !(size.is_finite() && size.x > 1e-9 && size.y > 1e-9) {
        return None;
    }
    let width = (f64::from(state.window_size.0) * 0.62).clamp(PREVIEW_MIN_WIDTH, PREVIEW_MAX_WIDTH);
    let height = PREVIEW_HEIGHT;
    let pad = 8.0;
    let scale = ((width - 2.0 * pad) / size.x).min((height - 2.0 * pad) / size.y);
    let offset_x = (width - size.x * scale) * 0.5;
    let offset_y = (height - size.y * scale) * 0.5;
    let y_up = dialog.kind == ImportKind::Dxf;
    let mut paths = Vec::new();
    for (index, row) in dialog.rows.iter().enumerate() {
        if row.points.len() < 2 {
            continue;
        }
        let step = (row.points.len() / PREVIEW_MAX_POINTS).max(1);
        let mut screen: Vec<(f64, f64)> = row
            .points
            .iter()
            .step_by(step)
            .map(|p| {
                let x = offset_x + (p.x - lo.x) * scale;
                let y = if y_up {
                    offset_y + (hi.y - p.y) * scale
                } else {
                    offset_y + (p.y - lo.y) * scale
                };
                (x, y)
            })
            .collect();
        if let Some(last) = row.points.last() {
            let x = offset_x + (last.x - lo.x) * scale;
            let y = if y_up {
                offset_y + (hi.y - last.y) * scale
            } else {
                offset_y + (last.y - lo.y) * scale
            };
            if screen.last().is_none_or(|p| (p.0 - x).hypot(p.1 - y) > 0.5) {
                screen.push((x, y));
            }
        }
        let paint = VectorPaint {
            fill: None,
            stroke: Some(if row.issue.is_some() {
                Color::rgba(239, 68, 68, 190)
            } else if dialog.selected.contains(&index) {
                Color::rgba(56, 189, 248, 255)
            } else {
                Color::rgba(148, 163, 184, 170)
            }),
            stroke_width: if dialog.selected.contains(&index) {
                1.8
            } else {
                1.0
            },
        };
        if let Ok(path) = VectorPath::new(polyline(&screen, row.closed), paint) {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        return None;
    }
    let scene = VectorScene::new(
        VectorViewBox::new(0.0, 0.0, width as f32, height as f32),
        paths,
    )
    .ok()?;
    let vector: Node = Node::vector(scene)
        .with_id("import-preview-scene")
        .with_class("import-preview-scene")
        .into();
    let mut style = Style::default();
    style.layout.calc.width = Some(LengthPercentageCalc::new(0.0, width as f32));
    style.layout.calc.height = Some(LengthPercentageCalc::new(0.0, height as f32));
    Some(
        Node::element("div")
            .with_id("import-preview")
            .with_class("import-preview-box")
            .with_style(style)
            .with_children([vector])
            .into(),
    )
}
