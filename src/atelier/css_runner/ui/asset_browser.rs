//! Bundled templates, import/export actions and piece cards.

use super::super::actions::{RunnerAction, RunnerUiOp, queue_action, queue_ui_op};
use super::super::board::selection::compute_connected_2d_highlight;
use super::super::state::CssRunnerState;
use clothcraft::garment::presets::GarmentKind;
use cssimpler::core::Node;
use cssimpler::ui;

/// An import quick action shown in the left browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct ImportQuickAction {
    pub label: &'static str,
    pub tooltip: &'static str,
}

/// Import quick actions for the native workspace. None is wired yet: the CPU
/// importers exist (`pattern_import`, `garment::svg_import`, `avatar_import`)
/// but the native dialogs were not ported after FC-0022, so the buttons render
/// disabled and the tooltip names the working alternative.
pub(in super::super) fn import_quick_actions() -> [ImportQuickAction; 3] {
    [
        ImportQuickAction {
            label: "DXF",
            tooltip: "DXF import is not available in this window yet",
        },
        ImportQuickAction {
            label: "SVG",
            tooltip: "SVG import is not available in this window yet",
        },
        ImportQuickAction {
            label: "Avatar",
            tooltip: "Not available in this window yet; start cloth-fit with --avatar <file.obj|.gltf|.glb>",
        },
    ]
}

pub(in super::super) fn build_runner_left_browser(state: &CssRunnerState) -> Node {
    if !state.left_drawer_open || state.clean_view {
        return ui! { <aside class="left-drawer collapsed"></aside> };
    }

    let panels = &state.project.template.panels;
    let connected = compute_connected_2d_highlight(state);
    let mut piece_cards = Vec::new();

    for (idx, p) in panels.iter().enumerate() {
        let is_selected = idx == state.selected_piece_index;
        let is_connected = !is_selected && connected.connected_panels.contains(&p.id);
        let card_class = if is_selected {
            "piece-card active"
        } else if is_connected {
            "piece-card connected"
        } else {
            "piece-card"
        };
        let label = p.name.clone();
        let id_str = if is_connected {
            format!("{} · linked", p.id)
        } else {
            p.id.clone()
        };
        let meta_str = format!("Fabric: {} · Grain: {:.0}°", p.fabric, p.grain_degrees);
        piece_cards.push(ui! {
            <div class={card_class} onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::SelectPiece(idx))}>
                <div class="piece-header">
                    <span class="piece-name">{label}</span>
                    <span class="piece-id">{id_str}</span>
                </div>
                <div class="piece-meta">
                    <span>{meta_str}</span>
                </div>
            </div>
        });
    }

    ui! {
        <aside id="left-browser" class="sidebar left-browser-panel">
            <header class="browser-header">
                <h2 class="browser-title">"Assets & Pieces"</h2>
                <span class="browser-badge">{format!("{} pieces", panels.len())}</span>
            </header>
            <div class="drawer-content">
                <div class="browser-section">
                    <div class="section-header">
                        <span class="section-title">"Bundled Templates"</span>
                        <span class="section-subtitle">"Engineering fixtures (metres)"</span>
                    </div>
                    <div class="template-cards-container">
                        <div class="template-entry pointer-item" onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::LoadPreset(GarmentKind::Tshirt))}>
                            <div class="template-header">
                                <span class="template-title">"T-shirt"</span>
                                <span class="template-meta">"2 panels · 4 seams"</span>
                            </div>
                            <span class="template-description">"Two-panel engineering fixture for MakeHuman mannequin."</span>
                        </div>
                        <div class="template-entry pointer-item" onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::LoadPreset(GarmentKind::Dress))}>
                            <div class="template-header">
                                <span class="template-title">"Dress"</span>
                                <span class="template-meta">"2 panels · 4 seams"</span>
                            </div>
                            <span class="template-description">"Two-panel dress fixture with curved seams."</span>
                        </div>
                        <div class="template-entry pointer-item" onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::StatusNote("Apron fixture is planned (use T-shirt or Dress)".into()))}>
                            <div class="template-header">
                                <span class="template-title">"Apron"</span>
                                <span class="template-meta">"3 panels · 4 seams"</span>
                            </div>
                            <span class="template-description">"Workwear fixture with ties and bib."</span>
                        </div>
                    </div>
                </div>

                <div class="browser-section">
                    <div class="section-header">
                        <span class="section-title">"Import & Export"</span>
                    </div>
                    <div class="quick-action-group">
                        <span class="group-label">"Import"</span>
                        <div class="quick-actions-row">
                            {import_quick_actions()
                                .into_iter()
                                .map(|action| ui! {
                                    <button class="btn-action disabled" title={action.tooltip}>{action.label}</button>
                                })
                                .collect::<Vec<_>>()}
                        </div>
                    </div>
                    <div class="quick-action-group">
                        <span class="group-label">"Export"</span>
                        <div class="quick-actions-row">
                            <button class="btn-action" onclick_shared={queue_action(&state.actions, RunnerAction::ExportObj)}>"Export OBJ"</button>
                            <button class="btn-action btn-action-accent" onclick_shared={queue_action(&state.actions, RunnerAction::ExportSvg)}>"True-Scale SVG"</button>
                        </div>
                        <div class="export-accuracy-note-box">
                            <p class="export-note-text">
                                "Physical SVG includes stored seam allowances & sewing notches. 100 mm reference ruler verified."
                            </p>
                        </div>
                    </div>
                </div>

                <div class="piece-cards-section">
                    <div class="section-header">
                        <span class="section-title">"Garment Pieces"</span>
                    </div>
                    <div class="piece-cards-list">
                        {piece_cards}
                    </div>
                    <button class="action-btn" onclick_shared={queue_action(&state.actions, RunnerAction::PieceNext)}>"Select Next Piece"</button>
                </div>
            </div>
        </aside>
    }
}
