//! Selected-piece property accordions.

use super::super::super::actions::{RunnerAction, RunnerUiOp, queue_action, queue_ui_op};
use super::super::super::board::corners::active_fillet_previews;
use super::super::super::board::geometry::panel_edge_length_m;
use super::super::super::board::selection::{
    compute_connected_2d_highlight, effective_selected_points,
};
use super::super::super::state::CssRunnerState;
use super::super::number::{NumberField, number_field};
use clothcraft::authoring::CornerRule;
use clothcraft::css_ui::icons::{icon_chevron_down, icon_chevron_right};
use clothcraft::garment::GarmentTemplate;
use cssimpler::core::Node;
use cssimpler::ui;

/// Label/value rows for the Fabric Mechanics accordion, read from the
/// selected panel's fabric in the document (SI values, with g/m² and mm shown
/// for readability). Returns a single explanatory row instead of defaults when
/// no panel is selected or the panel names a fabric the template lacks.
pub(in super::super::super) fn fabric_mechanics_rows(
    template: &GarmentTemplate,
    selected_panel: usize,
) -> Vec<(&'static str, String)> {
    let Some(panel) = template.panels.get(selected_panel) else {
        return vec![("Fabric", "No panel selected".to_string())];
    };
    let Some(fabric) = template.fabrics.iter().find(|f| f.id == panel.fabric) else {
        return vec![("Fabric", format!("Unknown fabric id '{}'", panel.fabric))];
    };
    let p = &fabric.properties;
    vec![
        ("Fabric", format!("{} ({})", fabric.name, fabric.id)),
        ("Areal Density", format!("{:.0} g/m²", p.density * 1e3)),
        ("Thickness", format!("{:.2} mm", p.thickness * 1e3)),
        ("Warp Membrane", format!("{:.0} N/m", p.warp)),
        ("Weft Membrane", format!("{:.0} N/m", p.weft)),
        ("Shear", format!("{:.0} N/m", p.shear)),
        ("Bending Warp", format!("{:.2e} N·m", p.bend_warp)),
        ("Bending Weft", format!("{:.2e} N·m", p.bend_weft)),
    ]
}

pub(in super::super::super) fn build_runner_property_accordions(state: &CssRunnerState) -> Node {
    let panels = &state.project.template.panels;
    let selected_panel = panels.get(state.selected_piece_index);
    let selected_name = selected_panel.map_or("None", |p| p.name.as_str());
    let selected_id = selected_panel.map_or("none", |p| p.id.as_str());

    let (width_mm, height_mm) = if let Some(p) = selected_panel {
        if let Some(ref poly) = p.polygon {
            let min_x = poly
                .points
                .iter()
                .map(|pt| pt.x)
                .fold(f64::INFINITY, f64::min);
            let max_x = poly
                .points
                .iter()
                .map(|pt| pt.x)
                .fold(f64::NEG_INFINITY, f64::max);
            let min_y = poly
                .points
                .iter()
                .map(|pt| pt.y)
                .fold(f64::INFINITY, f64::min);
            let max_y = poly
                .points
                .iter()
                .map(|pt| pt.y)
                .fold(f64::NEG_INFINITY, f64::max);
            ((max_x - min_x) * 1000.0, (max_y - min_y) * 1000.0)
        } else {
            (p.pattern.width * 1000.0, p.pattern.height * 1000.0)
        }
    } else {
        (0.0, 0.0)
    };

    let area_dm2 = (width_mm * height_mm) / 10000.0;
    let perimeter_mm = (width_mm + height_mm) * 2.0;

    let last_pen = state
        .last_metrics
        .as_ref()
        .map_or("0.00 mm".to_string(), |m| {
            format!("{:.2} mm", m.minimum_distance_m * 1000.0)
        });
    let last_res = state.last_metrics.as_ref().map_or("N/A".to_string(), |m| {
        m.barrier_al.as_ref().map_or_else(
            || {
                m.residual_rms_n
                    .map_or("N/A".into(), |r| format!("{r:.1e}"))
            },
            |al| format!("{:.1e} N max", al.original_force_max_n),
        )
    });

    let pins_count = selected_panel.map_or(0, |p| p.pins.len() + p.pin_points.len());

    let open_0 = state.accordion_open[0];
    let open_1 = state.accordion_open[1];
    let open_2 = state.accordion_open[2];
    let open_3 = state.accordion_open[3];
    let open_4 = state.accordion_open[4];
    let open_5 = state.accordion_open[5];

    // Accordion 0: Information
    let acc_info = ui! {
        <div class={if open_0 { "accordion open" } else { "accordion collapsed" }}>
            <button class="accordion-header" onclick_shared={queue_action(&state.actions, RunnerAction::Accordion(0))}>
                <span>"1. Piece Information"</span>
                {if open_0 { icon_chevron_down() } else { icon_chevron_right() }}
            </button>
            {if open_0 {
                ui! {
                    <div class="accordion-content">
                        <div class="prop-row">
                            <span class="prop-label">"Name"</span>
                            <span class="prop-value">{selected_name}</span>
                        </div>
                        <div class="prop-row">
                            <span class="prop-label">"ID"</span>
                            <span class="prop-value">{selected_id}</span>
                        </div>
                        <div class="prop-row">
                            <span class="prop-label">"Width"</span>
                            <span class="prop-value">{format!("{width_mm:.1} mm")}</span>
                        </div>
                        <div class="prop-row">
                            <span class="prop-label">"Height"</span>
                            <span class="prop-value">{format!("{height_mm:.1} mm")}</span>
                        </div>
                        <div class="prop-row">
                            <span class="prop-label">"Area"</span>
                            <span class="prop-value">{format!("{area_dm2:.2} dm²")}</span>
                        </div>
                        <div class="prop-row">
                            <span class="prop-label">"Perimeter"</span>
                            <span class="prop-value">{format!("{perimeter_mm:.1} mm")}</span>
                        </div>
                    </div>
                }
            } else {
                ui! { <span class="hidden" /> }
            }}
        </div>
    };

    // Accordion 1: Selected Line, Corner & Seam Allowance
    let template = state.active_template();
    let connected = compute_connected_2d_highlight(state);
    let connected_label = if connected.connected_panels.is_empty() {
        "None".to_string()
    } else {
        connected
            .connected_panels
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    };
    let sel_pts = effective_selected_points(state);
    let has_sel_pts = !sel_pts.is_empty();
    let corner_menu_active =
        state.active_2d_tool == "Corner" || (state.corner_preview_active && has_sel_pts);
    let corner_menu_box_class = if corner_menu_active {
        "cad-corner-menu-box active"
    } else {
        "cad-corner-menu-box"
    };
    let preview_count = if open_1 {
        active_fillet_previews(state).len()
    } else {
        0
    };
    let corner_menu_badge = if preview_count > 0 {
        format!("Live preview ({preview_count})")
    } else if state.active_2d_tool == "Corner" {
        "Menu Active".to_string()
    } else if has_sel_pts {
        "Adjust to preview".to_string()
    } else {
        "Select corner".to_string()
    };
    let sel_info: (String, String, String, String) = if sel_pts.len() > 1 {
        let mut any_curved = false;
        let mut any_straight = false;
        for (pid, pt_idx) in &sel_pts {
            if let Some(p) = template.panels.iter().find(|p| &p.id == pid)
                && let Some(ref poly) = p.polygon
                && !poly.edges.is_empty()
                && *pt_idx < poly.edges.len()
            {
                let edge_name = &poly.edges[*pt_idx];
                let prev_name = &poly.edges[(*pt_idx + poly.edges.len() - 1) % poly.edges.len()];
                if poly.curves.contains_key(edge_name) || poly.curves.contains_key(prev_name) {
                    any_curved = true;
                } else {
                    any_straight = true;
                }
            }
        }
        let ctype = match (any_curved, any_straight) {
            (true, true) => "Mixed",
            (true, false) => "Cubic Bézier",
            _ => "Straight",
        };
        (
            format!("{} corners selected", sel_pts.len()),
            ctype.into(),
            "Multiple".into(),
            "—".into(),
        )
    } else if let Some((pid, pt_idx)) = sel_pts.into_iter().next() {
        if let Some(p) = template.panels.iter().find(|p| p.id == pid)
            && let Some(ref poly) = p.polygon
            && let Some(edge_name) = poly.edges.get(pt_idx)
        {
            let prev_name = &poly.edges[(pt_idx + poly.edges.len() - 1) % poly.edges.len()];
            let curved = poly.curves.contains_key(edge_name) || poly.curves.contains_key(prev_name);
            let ctype = if curved { "Cubic Bézier" } else { "Straight" }.to_string();
            let len_str = panel_edge_length_m(p, edge_name)
                .map_or("—".into(), |m| format!("{:.1} mm", m * 1000.0));
            let sa_str = format!("{:.1} mm", poly.allowance_of(edge_name) * 1000.0);
            (
                format!("Corner #{pt_idx} ({edge_name})"),
                ctype,
                len_str,
                sa_str,
            )
        } else {
            (
                format!("Corner #{pt_idx}"),
                "—".into(),
                "—".into(),
                "0.0 mm".into(),
            )
        }
    } else if let Some((ref pid, ref ename)) = state.selected_2d_edge {
        if let Some(p) = template.panels.iter().find(|p| &p.id == pid)
            && let Some(ref poly) = p.polygon
        {
            let ctype = if poly.curves.contains_key(ename) {
                "Cubic Bézier"
            } else {
                "Straight"
            }
            .to_string();
            let len_str = panel_edge_length_m(p, ename)
                .map_or("—".into(), |m| format!("{:.1} mm", m * 1000.0));
            let sa_str = format!("{:.1} mm", poly.allowance_of(ename) * 1000.0);
            (format!("Edge {ename}"), ctype, len_str, sa_str)
        } else {
            (
                format!("Edge {ename}"),
                "—".into(),
                "—".into(),
                "0.0 mm".into(),
            )
        }
    } else {
        let curve_count = selected_panel
            .and_then(|p| p.polygon.as_ref())
            .map_or(0, |poly| poly.curves.len());
        (
            "Click point or edge".into(),
            if curve_count > 0 {
                format!("{curve_count} Bézier curves")
            } else {
                "Straight contour".into()
            },
            "—".into(),
            "0.0 mm".into(),
        )
    };
    let mode_radius_prop_btn = if state.corner_mode_radius {
        "cad-prop-btn active"
    } else {
        "cad-prop-btn"
    };
    let mode_shape_prop_btn = if !state.corner_mode_radius {
        "cad-prop-btn active"
    } else {
        "cad-prop-btn"
    };
    let rule_sharp_btn = if state.corner_rule == CornerRule::Sharp {
        "cad-prop-btn active"
    } else {
        "cad-prop-btn"
    };
    let rule_smooth_btn = if state.corner_rule == CornerRule::Smooth {
        "cad-prop-btn active"
    } else {
        "cad-prop-btn"
    };
    let rule_sym_btn = if state.corner_rule == CornerRule::Symmetric {
        "cad-prop-btn active"
    } else {
        "cad-prop-btn"
    };

    let acc_line = ui! {
        <div class={if open_1 { "accordion open" } else { "accordion collapsed" }}>
            <button class="accordion-header" onclick_shared={queue_action(&state.actions, RunnerAction::Accordion(1))}>
                <span>"2. Boundary & Allowance"</span>
                {if open_1 { icon_chevron_down() } else { icon_chevron_right() }}
            </button>
            {if open_1 {
                ui! {
                    <div class="accordion-content">
                        <div class="prop-row">
                            <span class="prop-label">"2D Selection"</span>
                            <span class="prop-value">{sel_info.0}</span>
                        </div>
                        <div class="prop-row">
                            <span class="prop-label">"Linked Pieces"</span>
                            <span class="prop-value">{connected_label}</span>
                        </div>
                        <div class="prop-row">
                            <span class="prop-label">"Curvature Type"</span>
                            <span class="prop-value">{sel_info.1}</span>
                        </div>
                        <div class="prop-row">
                            <span class="prop-label">"Boundary Length"</span>
                            <span class="prop-value">{sel_info.2}</span>
                        </div>
                        <div class="prop-row">
                            <span class="prop-label">"Seam Allowance"</span>
                            <span class="prop-value">{sel_info.3}</span>
                        </div>
                        <div id="corner-rounding-menu" class={corner_menu_box_class}>
                            <div class="cad-corner-menu-header">
                                <span>"Corner Rounding Menu"</span>
                                <span id="corner-menu-badge" class="cad-corner-menu-badge">{corner_menu_badge}</span>
                            </div>
                            <div class="cad-prop-actions">
                                <button id="prop-corner-radius-mode" class={mode_radius_prop_btn} onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::SetCornerMode(true))}>"Radius"</button>
                                <button id="prop-corner-shape-mode" class={mode_shape_prop_btn} onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::SetCornerMode(false))}>"Shape"</button>
                            </div>
                            <div class="cad-prop-actions">
                                <button id="prop-radius-dec" class="cad-prop-btn" onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::AdjustCornerRadiusMm(-2.0))}>"-2 mm"</button>
                                {number_field(state, "prop-radius-value", NumberField::CornerRadius)}
                                <button id="prop-radius-inc" class="cad-prop-btn" onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::AdjustCornerRadiusMm(2.0))}>"+2 mm"</button>
                            </div>
                            <div class="cad-prop-actions">
                                <button id="prop-tension-dec" class="cad-prop-btn" onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::AdjustCornerTension(-0.05))}>"T-"</button>
                                {number_field(state, "prop-tension-value", NumberField::CornerTension)}
                                <button id="prop-tension-inc" class="cad-prop-btn" onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::AdjustCornerTension(0.05))}>"T+"</button>
                            </div>
                            <div class="cad-prop-actions">
                                <button class={rule_sharp_btn} onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::SetCornerRule(CornerRule::Sharp))}>"Sharp"</button>
                                <button class={rule_smooth_btn} onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::SetCornerRule(CornerRule::Smooth))}>"Smooth"</button>
                                <button class={rule_sym_btn} onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::SetCornerRule(CornerRule::Symmetric))}>"Sym"</button>
                            </div>
                            <div class="cad-prop-actions">
                                <button id="prop-apply-round" class="cad-prop-btn primary" onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::ApplyCornerRoundSelected)}>{format!("Round ({:.0} mm)", state.corner_radius_mm)}</button>
                                <button class="cad-prop-btn" onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::ToggleSelectedCurve(true))}>"To Bézier"</button>
                                <button class="cad-prop-btn" onclick_shared={queue_ui_op(&state.actions, RunnerUiOp::ToggleSelectedCurve(false))}>"To Straight"</button>
                            </div>
                        </div>
                    </div>
                }
            } else {
                ui! { <span class="hidden" /> }
            }}
        </div>
    };

    // Accordion 2: 3D Arrangement
    let acc_arrange = ui! {
        <div class={if open_2 { "accordion open" } else { "accordion collapsed" }}>
            <button class="accordion-header" onclick_shared={queue_action(&state.actions, RunnerAction::Accordion(2))}>
                <span>"3. 3D Arrangement"</span>
                {if open_2 { icon_chevron_down() } else { icon_chevron_right() }}
            </button>
            {if open_2 {
                ui! {
                    <div class="accordion-content">
                        <div class="prop-row">
                            <span class="prop-label">"Translation X"</span>
                            <span class="prop-value">{format!("{:.1} cm", selected_panel.map_or(0.0, |p| p.placement.translation.x * 100.0))}</span>
                        </div>
                        <div class="prop-row">
                            <span class="prop-label">"Translation Y"</span>
                            <span class="prop-value">{format!("{:.1} cm", selected_panel.map_or(0.0, |p| p.placement.translation.y * 100.0))}</span>
                        </div>
                        <div class="prop-row">
                            <span class="prop-label">"Translation Z"</span>
                            <span class="prop-value">{format!("{:.1} cm", selected_panel.map_or(0.0, |p| p.placement.translation.z * 100.0))}</span>
                        </div>
                        <div class="prop-row">
                            <span class="prop-label">"Rotation (X/Y/Z)"</span>
                            <span class="prop-value">{format!("{:.0}°/{:.0}°/{:.0}°",
                                selected_panel.map_or(0.0, |p| p.placement.rotation_degrees.x),
                                selected_panel.map_or(0.0, |p| p.placement.rotation_degrees.y),
                                selected_panel.map_or(0.0, |p| p.placement.rotation_degrees.z))}</span>
                        </div>
                    </div>
                }
            } else {
                ui! { <span class="hidden" /> }
            }}
        </div>
    };

    // Accordion 3: Pins
    let acc_pins = ui! {
        <div class={if open_3 { "accordion open" } else { "accordion collapsed" }}>
            <button class="accordion-header" onclick_shared={queue_action(&state.actions, RunnerAction::Accordion(3))}>
                <span>"4. Pins & Restraints"</span>
                {if open_3 { icon_chevron_down() } else { icon_chevron_right() }}
            </button>
            {if open_3 {
                ui! {
                    <div class="accordion-content">
                        <div class="prop-row">
                            <span class="prop-label">"Pinned Points"</span>
                            <span class="prop-value">{format!("{pins_count}")}</span>
                        </div>
                        <div class="prop-row">
                            <span class="prop-label">"Active Pin Mode"</span>
                            <span class="prop-value">{if state.pin_mode_contour { "Contour" } else { "Interior" }}</span>
                        </div>
                        <div class="prop-row">
                            <span class="prop-label">"Pin Stiffness"</span>
                            <span class="prop-value">"1e5 N/m"</span>
                        </div>
                    </div>
                }
            } else {
                ui! { <span class="hidden" /> }
            }}
        </div>
    };

    // Accordion 4: Fabric & Mechanical Properties
    let acc_fabric = ui! {
        <div class={if open_4 { "accordion open" } else { "accordion collapsed" }}>
            <button class="accordion-header" onclick_shared={queue_action(&state.actions, RunnerAction::Accordion(4))}>
                <span>"5. Fabric Mechanics"</span>
                {if open_4 { icon_chevron_down() } else { icon_chevron_right() }}
            </button>
            {if open_4 {
                ui! {
                    <div class="accordion-content">
                        {fabric_mechanics_rows(&state.project.template, state.selected_piece_index)
                            .into_iter()
                            .map(|(label, value)| ui! {
                                <div class="prop-row">
                                    <span class="prop-label">{label}</span>
                                    <span class="prop-value">{value}</span>
                                </div>
                            })
                            .collect::<Vec<_>>()}
                    </div>
                }
            } else {
                ui! { <span class="hidden" /> }
            }}
        </div>
    };

    // Accordion 5: Simulation Diagnostics
    let acc_sim = ui! {
        <div class={if open_5 { "accordion open" } else { "accordion collapsed" }}>
            <button class="accordion-header" onclick_shared={queue_action(&state.actions, RunnerAction::Accordion(5))}>
                <span>"6. Simulation Diagnostics"</span>
                {if open_5 { icon_chevron_down() } else { icon_chevron_right() }}
            </button>

            {if open_5 {
                ui! {
                    <div class="accordion-content">
                        <div class="prop-row">
                            <span class="prop-label">"Solver Step Time"</span>
                            <span class="prop-value">{format!("{:.2} ms", state.step_ms)}</span>
                        </div>
                        <div class="prop-row">
                            <span class="prop-label">"Force Residual"</span>
                            <span class="prop-value">{last_res}</span>
                        </div>
                        <div class="prop-row">
                            <span class="prop-label">"Minimum Detected Gap"</span>
                            <span class="prop-value">{last_pen}</span>
                        </div>
                        <div class="prop-row">
                            <span class="prop-label">"Active Backend"</span>
                            <span class="prop-value">{if state.gpu_sim { state.contact_method.label() } else { "CPU (OGC)" }}</span>
                        </div>
                    </div>
                }
            } else {
                ui! { <span class="hidden" /> }
            }}

        </div>
    };

    ui! {
        <div class="accordions-container">
            {acc_info}
            {acc_line}
            {acc_arrange}
            {acc_pins}
            {acc_fabric}
            {super::super::super::mesh_preparation::render(state)}
            {acc_sim}
        </div>
    }
}
