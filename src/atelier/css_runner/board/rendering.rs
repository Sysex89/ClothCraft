//! Native vector board scenes and visual overlays.

use super::super::state::CssRunnerState;
use super::corners::{
    active_fillet_previews, connected_fillet_max_radius_m, fillet_drag_handle_local,
};
use super::geometry::{
    compute_2d_view_box_m, compute_2d_viewport_rect, panel_raw_points, panel_to_board_pt,
};
use super::native::{BoardPath, circle, dashed_commands, point, polyline, rectangle};
use super::selection::{compute_connected_2d_highlight, effective_selected_points};
use super::types::{Drag2d, Hover2dTarget, ToolDraft2d};
use clothcraft::authoring::corner_tool::{CIRCLE_TENSION, validate_fillet_preview};
use clothcraft::css_ui::Theme;
use clothcraft::garment::curves;
use cssimpler::core::PathCommand::{Close, CubicTo, LineTo, MoveTo};
use cssimpler::core::{Color, VectorViewBox};
use glam::DVec2;

/// Everything the static board background (colour, grid, axes) depends on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct BoardBackgroundKey {
    theme: Theme,
    min_x_bits: u64,
    min_y_bits: u64,
    width_bits: u64,
    height_bits: u64,
    px_bits: u64,
}

/// Retains the background paths so their shared geometry identity survives a
/// view rebuild. cssimpler's layer-cache key follows `VectorGeometry` identity;
/// rebuilding identical commands would otherwise miss the cached grid raster.
pub(crate) struct BoardBackgroundCache {
    pub(super) key: BoardBackgroundKey,
    paths: Vec<BoardPath>,
}

pub fn render_runner_2d_paths(state: &CssRunnerState) -> (Vec<BoardPath>, VectorViewBox) {
    let template = state.active_template();
    let panels = &template.panels;

    struct PanelCurve {
        edge_idx: usize,
        edge_name: String,
        c1: (f64, f64),
        c2: (f64, f64),
    }

    struct PanelGeo {
        panel_idx: usize,
        id: String,
        points: Vec<(f64, f64)>,
        edge_names: Vec<String>,
        curves: Vec<PanelCurve>,
        internal_lines: Vec<Vec<(f64, f64)>>,
        holes: Vec<Vec<(f64, f64)>>,
        min_x: f64,
        min_y: f64,
        max_x: f64,
        max_y: f64,
        center: (f64, f64),
        grain_deg: f64,
        is_selected: bool,
        is_connected: bool,
    }

    let to_mm = |p: DVec2| -> (f64, f64) { (p.x * 1000.0, p.y * 1000.0) };
    let connected = compute_connected_2d_highlight(state);

    let mut panel_geos = Vec::with_capacity(panels.len());
    for (idx, panel) in panels.iter().enumerate() {
        let is_selected = idx == state.selected_piece_index;
        let is_connected = !is_selected && connected.connected_panels.contains(&panel.id);
        let raw_pts = panel_raw_points(panel);
        let points: Vec<(f64, f64)> = raw_pts
            .iter()
            .map(|&pt| to_mm(panel_to_board_pt(template, idx, pt)))
            .collect();

        let mut edge_names = Vec::new();
        let mut curves = Vec::new();
        let mut internal_lines = Vec::new();
        let mut holes = Vec::new();

        if let Some(ref poly) = panel.polygon {
            edge_names = poly.edges.clone();
            for (edge_idx, edge_name) in poly.edges.iter().enumerate() {
                if let Some([c1, c2]) = poly.curves.get(edge_name) {
                    curves.push(PanelCurve {
                        edge_idx,
                        edge_name: edge_name.clone(),
                        c1: to_mm(panel_to_board_pt(template, idx, *c1)),
                        c2: to_mm(panel_to_board_pt(template, idx, *c2)),
                    });
                }
            }
            for line in &poly.lines {
                let pts: Vec<(f64, f64)> = line
                    .points
                    .iter()
                    .map(|&p| to_mm(panel_to_board_pt(template, idx, p)))
                    .collect();
                if pts.len() >= 2 {
                    internal_lines.push(pts);
                }
            }
            for hole in &poly.holes {
                let pts: Vec<(f64, f64)> = hole
                    .points
                    .iter()
                    .map(|&p| to_mm(panel_to_board_pt(template, idx, p)))
                    .collect();
                if pts.len() >= 3 {
                    holes.push(pts);
                }
            }
        } else {
            for i in 0..points.len() {
                edge_names.push(format!("edge_{i}"));
            }
        }

        let mut s_min_x = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        let mut s_max_x = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
        let mut s_min_y = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let mut s_max_y = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        for c in &curves {
            s_min_x = s_min_x.min(c.c1.0).min(c.c2.0);
            s_max_x = s_max_x.max(c.c1.0).max(c.c2.0);
            s_min_y = s_min_y.min(c.c1.1).min(c.c2.1);
            s_max_y = s_max_y.max(c.c1.1).max(c.c2.1);
        }
        let center = ((s_min_x + s_max_x) * 0.5, (s_min_y + s_max_y) * 0.5);

        panel_geos.push(PanelGeo {
            panel_idx: idx,
            id: panel.id.clone(),
            points,
            edge_names,
            curves,
            internal_lines,
            holes,
            min_x: s_min_x,
            min_y: s_min_y,
            max_x: s_max_x,
            max_y: s_max_y,
            center,
            grain_deg: panel.grain_degrees,
            is_selected,
            is_connected,
        });
    }

    // Display commands use millimetres; authored geometry remains f64 metres.
    let (vb_min_x_m, vb_min_y_m, view_width_m, view_height_m) = compute_2d_view_box_m(state);
    let vb_min_x = vb_min_x_m * 1000.0;
    let vb_min_y = vb_min_y_m * 1000.0;
    let vb_w = view_width_m * 1000.0;
    let vb_h = view_height_m * 1000.0;
    let view_box = VectorViewBox::new(vb_min_x as f32, vb_min_y as f32, vb_w as f32, vb_h as f32);

    let svg_rect = compute_2d_viewport_rect(state);
    let px = (vb_w / (svg_rect.w.max(1.0) as f64)).max(0.25);

    // Explicit native paints include their alpha channel.
    let is_dark = state.theme == Theme::Dark;
    let bg_color = if is_dark {
        Some(Color::rgb(13, 15, 23))
    } else {
        Some(Color::rgb(248, 250, 252))
    };
    let grid_minor = if is_dark {
        Some(Color::rgb(22, 28, 40))
    } else {
        Some(Color::rgb(226, 232, 240))
    };
    let grid_major = if is_dark {
        Some(Color::rgb(35, 46, 66))
    } else {
        Some(Color::rgb(203, 213, 225))
    };
    let origin_color = if is_dark {
        Some(Color::rgba(56, 189, 248, 71))
    } else {
        Some(Color::rgba(37, 99, 235, 61))
    };
    let unsel_stroke = if is_dark {
        Some(Color::rgb(56, 189, 248))
    } else {
        Some(Color::rgb(37, 99, 235))
    };
    let unsel_fill = if is_dark {
        Some(Color::rgba(56, 189, 248, 20))
    } else {
        Some(Color::rgba(37, 99, 235, 15))
    };
    let sel_stroke = if is_dark {
        Some(Color::rgb(245, 158, 11))
    } else {
        Some(Color::rgb(217, 119, 6))
    };
    let sel_fill = if is_dark {
        Some(Color::rgba(245, 158, 11, 36))
    } else {
        Some(Color::rgba(217, 119, 6, 28))
    };
    let conn_fill = Some(Color::rgba(16, 185, 129, 26));
    let handle_color = if is_dark {
        Some(Color::rgb(56, 189, 248))
    } else {
        Some(Color::rgb(2, 132, 199))
    };
    let handle_soft = if is_dark {
        Some(Color::rgba(56, 189, 248, 115))
    } else {
        Some(Color::rgba(2, 132, 199, 102))
    };
    let vertex_fill = if is_dark {
        Some(Color::rgb(15, 23, 42))
    } else {
        Some(Color::rgb(255, 255, 255))
    };
    let grain_color = if is_dark {
        Some(Color::rgba(96, 165, 250, 184))
    } else {
        Some(Color::rgba(59, 130, 246, 178))
    };
    let accent_gold = Some(Color::rgb(250, 204, 21));
    let accent_emerald = Some(Color::rgb(16, 185, 129));
    let hover_cyan = Some(Color::rgb(125, 211, 252));

    let background_key = BoardBackgroundKey {
        theme: state.theme,
        min_x_bits: vb_min_x.to_bits(),
        min_y_bits: vb_min_y.to_bits(),
        width_bits: vb_w.to_bits(),
        height_bits: vb_h.to_bits(),
        px_bits: px.to_bits(),
    };
    let cached_background = {
        let cache = state.board_background.borrow();
        cache
            .as_ref()
            .filter(|cache| cache.key == background_key)
            .map(|cache| cache.paths.clone())
    };
    let mut nodes = match cached_background {
        Some(paths) => paths,
        None => {
            let mut background = Vec::new();
            // Background rect
            background.push(BoardPath::new(
                rectangle(vb_min_x, vb_min_y, vb_w, vb_h),
                bg_color,
                None,
                0.0,
                "",
            ));

            // Drafting grid (minor 100 mm, major 500 mm)
            let grid_step = 100.0_f64;
            let start_gx = (vb_min_x / grid_step).floor() * grid_step;
            let mut gx = start_gx;
            while gx <= vb_min_x + vb_w {
                let is_major = ((gx / 500.0) - (gx / 500.0).round()).abs() < 1e-4;
                let stroke = if is_major { grid_major } else { grid_minor };
                let sw = if is_major { 1.1 * px } else { 0.7 * px };
                let pts = polyline(&[(gx, vb_min_y), (gx, vb_min_y + vb_h)], false);
                background.push(BoardPath::new(pts, None, stroke, sw, ""));
                gx += grid_step;
            }
            let start_gy = (vb_min_y / grid_step).floor() * grid_step;
            let mut gy = start_gy;
            while gy <= vb_min_y + vb_h {
                let is_major = ((gy / 500.0) - (gy / 500.0).round()).abs() < 1e-4;
                let stroke = if is_major { grid_major } else { grid_minor };
                let sw = if is_major { 1.1 * px } else { 0.7 * px };
                let pts = polyline(&[(vb_min_x, gy), (vb_min_x + vb_w, gy)], false);
                background.push(BoardPath::new(pts, None, stroke, sw, ""));
                gy += grid_step;
            }

            // Origin axes reference
            if vb_min_x <= 0.0 && vb_min_x + vb_w >= 0.0 {
                background.push(BoardPath::new(
                    polyline(&[(0.0, vb_min_y), (0.0, vb_min_y + vb_h)], false),
                    None,
                    origin_color,
                    1.2 * px,
                    "",
                ));
            }
            if vb_min_y <= 0.0 && vb_min_y + vb_h >= 0.0 {
                background.push(BoardPath::new(
                    polyline(&[(vb_min_x, 0.0), (vb_min_x + vb_w, 0.0)], false),
                    None,
                    origin_color,
                    1.2 * px,
                    "",
                ));
            }

            for path in &mut background {
                path.background = true;
            }
            state
                .board_background
                .borrow_mut()
                .replace(BoardBackgroundCache {
                    key: background_key,
                    paths: background.clone(),
                });
            background
        }
    };

    let fillet_previews = active_fillet_previews(state);
    let corner_tool_active = state.active_2d_tool == "Corner";
    let sel_pts = effective_selected_points(state);

    // Panels rendering
    for p in &panel_geos {
        let is_hovered_panel =
            matches!(state.hovered_2d, Some(Hover2dTarget::Panel(i)) if i == p.panel_idx);
        let stroke_color = if p.is_selected {
            sel_stroke
        } else if p.is_connected {
            accent_emerald
        } else if is_hovered_panel {
            hover_cyan
        } else {
            unsel_stroke
        };
        let fill_color = if p.is_selected {
            sel_fill
        } else if p.is_connected {
            conn_fill
        } else {
            unsel_fill
        };
        let stroke_w = if p.is_selected {
            2.0 * px
        } else if p.is_connected {
            1.8 * px
        } else {
            1.3 * px
        };
        let piece_class = if p.is_selected {
            "pattern-piece active"
        } else if p.is_connected {
            "pattern-piece connected"
        } else {
            "pattern-piece"
        };

        // Outer boundary path (with cubic Bézier curves)
        let n_pts = p.points.len();
        if n_pts > 0 {
            let mut path_d = vec![MoveTo(point(p.points[0]))];
            for i in 0..n_pts {
                let next_idx = (i + 1) % n_pts;
                let (next_x, next_y) = p.points[next_idx];
                if let Some(c) = p.curves.iter().find(|c| c.edge_idx == i) {
                    path_d.push(CubicTo {
                        control1: point(c.c1),
                        control2: point(c.c2),
                        end: point((next_x, next_y)),
                    });
                } else {
                    path_d.push(LineTo(point((next_x, next_y))));
                }
            }
            path_d.push(Close);

            nodes.push(BoardPath::new(
                path_d,
                fill_color,
                stroke_color,
                stroke_w,
                piece_class,
            ));
        }

        // Hovered, selected, or connected partner edge overlay
        for (edge_idx, edge_name) in p.edge_names.iter().enumerate() {
            let is_primary_sel_edge = state
                .selected_2d_edge
                .as_ref()
                .is_some_and(|(pid, en)| pid == &p.id && en == edge_name);
            let is_conn_sel_edge = !is_primary_sel_edge
                && connected
                    .selected_edges
                    .contains(&(p.id.clone(), edge_name.clone()));
            let is_hov_edge = connected
                .hovered_edges
                .contains(&(p.id.clone(), edge_name.clone()));
            if is_primary_sel_edge || is_conn_sel_edge || is_hov_edge {
                let (a_x, a_y) = p.points[edge_idx];
                let (b_x, b_y) = p.points[(edge_idx + 1) % n_pts];
                let seg_d = if let Some(c) = p.curves.iter().find(|c| c.edge_idx == edge_idx) {
                    vec![
                        MoveTo(point((a_x, a_y))),
                        CubicTo {
                            control1: point(c.c1),
                            control2: point(c.c2),
                            end: point((b_x, b_y)),
                        },
                    ]
                } else {
                    polyline(&[(a_x, a_y), (b_x, b_y)], false)
                };
                let e_col = if is_primary_sel_edge {
                    accent_gold
                } else if is_conn_sel_edge {
                    accent_emerald
                } else {
                    hover_cyan
                };
                let e_w = if is_primary_sel_edge {
                    3.0 * px
                } else if is_conn_sel_edge {
                    2.6 * px
                } else {
                    2.4 * px
                };
                nodes.push(BoardPath::new(seg_d, None, e_col, e_w, ""));
            }
        }

        // Internal lines (dashed M..L path)
        let il_col = if is_dark {
            Some(Color::rgb(100, 116, 139))
        } else {
            Some(Color::rgb(148, 163, 184))
        };
        for line_pts in &p.internal_lines {
            let d = dashed_commands(line_pts, 5.0 * px, 4.0 * px);
            if !d.is_empty() {
                nodes.push(BoardPath::new(d, None, il_col, 1.1 * px, ""));
            }
        }

        // Holes
        for hole_pts in &p.holes {
            let h_pts_str = polyline(hole_pts, true);
            nodes.push(BoardPath::new(
                h_pts_str,
                bg_color,
                stroke_color,
                1.1 * px,
                "",
            ));
        }

        // Bézier Indicators, Tangent Stalks & Handles
        let show_full_handles = p.is_selected
            || p.is_connected
            || matches!(
                state.hovered_2d,
                Some(Hover2dTarget::BezierHandle { panel_idx, .. }) if panel_idx == p.panel_idx
            );
        for c in &p.curves {
            let (p0_x, p0_y) = p.points[c.edge_idx];
            let (p3_x, p3_y) = p.points[(c.edge_idx + 1) % n_pts];
            let stalk_col = if show_full_handles {
                handle_color
            } else {
                handle_soft
            };
            let d1 = dashed_commands(&[(p0_x, p0_y), c.c1], 3.5 * px, 2.5 * px);
            let d2 = dashed_commands(&[(p3_x, p3_y), c.c2], 3.5 * px, 2.5 * px);
            nodes.push(BoardPath::new(d1, None, stalk_col, 1.0 * px, ""));
            nodes.push(BoardPath::new(d2, None, stalk_col, 1.0 * px, ""));

            let h0_active = matches!(
                state.hovered_2d,
                Some(Hover2dTarget::BezierHandle { panel_idx, edge_idx, handle_index: 0 })
                    if panel_idx == p.panel_idx && edge_idx == c.edge_idx
            );
            let h1_active = matches!(
                state.hovered_2d,
                Some(Hover2dTarget::BezierHandle { panel_idx, edge_idx, handle_index: 1 })
                    if panel_idx == p.panel_idx && edge_idx == c.edge_idx
            );
            let hr0 = if h0_active {
                4.8 * px
            } else if show_full_handles {
                3.6 * px
            } else {
                2.5 * px
            };
            let hr1 = if h1_active {
                4.8 * px
            } else if show_full_handles {
                3.6 * px
            } else {
                2.5 * px
            };
            let h0_stroke = if h0_active {
                accent_gold
            } else {
                Some(Color::rgb(255, 255, 255))
            };
            let h1_stroke = if h1_active {
                accent_gold
            } else {
                Some(Color::rgb(255, 255, 255))
            };

            nodes.push(BoardPath::new(
                circle(c.c1.0, c.c1.1, hr0),
                stalk_col,
                h0_stroke,
                1.1 * px,
                "bezier-handle",
            ));
            nodes.push(BoardPath::new(
                circle(c.c2.0, c.c2.1, hr1),
                stalk_col,
                h1_stroke,
                1.1 * px,
                "bezier-handle",
            ));
        }

        // Sleek Corner & Curve Anchor Indicators (with connected weld highlights)
        let base_stroke = if p.is_selected {
            sel_stroke
        } else if p.is_connected {
            accent_emerald
        } else {
            unsel_stroke
        };
        let v_sw = if p.is_selected || p.is_connected {
            1.4 * px
        } else {
            1.1 * px
        };

        for (pt_idx, &(pt_x, pt_y)) in p.points.iter().enumerate() {
            let prev_idx = (pt_idx + n_pts - 1) % n_pts;
            let has_incident_curve = p
                .curves
                .iter()
                .any(|c| c.edge_idx == pt_idx || c.edge_idx == prev_idx);

            let is_primary_sel_pt = sel_pts
                .iter()
                .any(|(pid, pi)| pid == &p.id && *pi == pt_idx);
            let is_conn_sel_pt =
                !is_primary_sel_pt && connected.selected_corners.contains(&(p.id.clone(), pt_idx));
            let is_hov_pt = connected.hovered_corners.contains(&(p.id.clone(), pt_idx));

            // Corner tool highlight ring on roundable vertices of active/connected/hovered panel
            if corner_tool_active
                && (p.is_selected || p.is_connected || is_hovered_panel || is_hov_pt)
                && let Some(edge_name) = p.edge_names.get(pt_idx)
                && let Ok(mr) = connected_fillet_max_radius_m(template, &p.id, edge_name)
                && mr > 0.0005
                && validate_fillet_preview(
                    template,
                    &p.id,
                    edge_name,
                    (state.corner_radius_mm / 1000.0).clamp(
                        0.0005_f64.min(mr * 0.5),
                        (mr * 0.5).max(0.0005_f64.min(mr * 0.5)),
                    ),
                    CIRCLE_TENSION,
                )
                .is_ok()
            {
                nodes.push(BoardPath::new(
                    circle(pt_x, pt_y, 5.0 * px),
                    None,
                    accent_emerald,
                    1.2 * px,
                    "",
                ));
            }

            if is_primary_sel_pt {
                nodes.push(BoardPath::new(
                    circle(pt_x, pt_y, 6.4 * px),
                    None,
                    accent_gold,
                    1.6 * px,
                    "",
                ));
            } else if is_conn_sel_pt {
                nodes.push(BoardPath::new(
                    circle(pt_x, pt_y, 6.0 * px),
                    None,
                    accent_emerald,
                    1.5 * px,
                    "",
                ));
            } else if is_hov_pt {
                nodes.push(BoardPath::new(
                    circle(pt_x, pt_y, 5.5 * px),
                    None,
                    hover_cyan,
                    1.3 * px,
                    "",
                ));
            }

            let v_fill = if is_primary_sel_pt {
                accent_gold
            } else if is_conn_sel_pt {
                accent_emerald
            } else {
                vertex_fill
            };
            let v_stroke = if is_primary_sel_pt {
                accent_gold
            } else if is_conn_sel_pt {
                accent_emerald
            } else {
                base_stroke
            };

            if has_incident_curve {
                // Smooth / curved anchor indicator (sleek small circle)
                let r = if is_primary_sel_pt || is_conn_sel_pt || is_hov_pt {
                    3.2 * px
                } else if p.is_selected || p.is_connected {
                    2.5 * px
                } else {
                    1.9 * px
                };
                nodes.push(BoardPath::new(
                    circle(pt_x, pt_y, r),
                    v_fill,
                    v_stroke,
                    v_sw,
                    "cad-vertex",
                ));
            } else {
                // Sharp G0 CAD corner indicator (crisp small square)
                let half = if is_primary_sel_pt || is_conn_sel_pt || is_hov_pt {
                    3.0 * px
                } else if p.is_selected || p.is_connected {
                    2.4 * px
                } else {
                    1.8 * px
                };
                nodes.push(BoardPath::new(
                    rectangle(pt_x - half, pt_y - half, half * 2.0, half * 2.0),
                    v_fill,
                    v_stroke,
                    v_sw,
                    "cad-vertex",
                ));
            }
        }

        // Grainline with bidirectional arrowheads
        let rad = p.grain_deg.to_radians();
        let grain_len = ((p.max_x - p.min_x).min(p.max_y - p.min_y) * 0.28).max(40.0);
        let (cx, cy) = p.center;
        let gx0 = cx - grain_len * rad.sin();
        let gy0 = cy - grain_len * rad.cos();
        let gx1 = cx + grain_len * rad.sin();
        let gy1 = cy + grain_len * rad.cos();
        let g_pts = polyline(&[(gx0, gy0), (gx1, gy1)], false);
        nodes.push(BoardPath::new(g_pts, None, grain_color, 1.2 * px, ""));
        let wing = 5.0 * px;
        let w_cos = rad.cos() * wing;
        let w_sin = rad.sin() * wing;
        let ah1 = polyline(
            &[
                (gx1 - w_sin - w_cos, gy1 - w_cos + w_sin),
                (gx1, gy1),
                (gx1 - w_sin + w_cos, gy1 - w_cos - w_sin),
            ],
            false,
        );
        let ah0 = polyline(
            &[
                (gx0 + w_sin - w_cos, gy0 + w_cos + w_sin),
                (gx0, gy0),
                (gx0 + w_sin + w_cos, gy0 + w_cos - w_sin),
            ],
            false,
        );
        nodes.push(BoardPath::new(ah1, None, grain_color, 1.2 * px, ""));
        nodes.push(BoardPath::new(ah0, None, grain_color, 1.2 * px, ""));

        // Clean center crosshair
        let ch_len = 5.5 * px;
        let ch_stroke = if p.is_selected {
            sel_stroke
        } else if p.is_connected {
            accent_emerald
        } else {
            grid_major
        };
        nodes.push(BoardPath::new(
            polyline(&[(cx - ch_len, cy), (cx + ch_len, cy)], false),
            None,
            ch_stroke,
            1.0 * px,
            "",
        ));
        nodes.push(BoardPath::new(
            polyline(&[(cx, cy - ch_len), (cx, cy + ch_len)], false),
            None,
            ch_stroke,
            1.0 * px,
            "",
        ));
    }

    // Welded / sewn corner link lines across connected pieces
    for ((pid_a, pt_a), (pid_b, pt_b)) in &connected.weld_links {
        if let (Some(ga), Some(gb)) = (
            panel_geos.iter().find(|g| &g.id == pid_a),
            panel_geos.iter().find(|g| &g.id == pid_b),
        ) && let (Some(&pa), Some(&pb)) = (ga.points.get(*pt_a), gb.points.get(*pt_b))
        {
            let wd = dashed_commands(&[pa, pb], 4.0 * px, 3.0 * px);
            if !wd.is_empty() {
                nodes.push(BoardPath::new(
                    wd,
                    None,
                    Some(Color::rgba(250, 204, 21, 184)),
                    1.4 * px,
                    "weld-link",
                ));
            }
        }
    }

    // Live Corner Fillet Preview Overlays (simultaneously across all connected pieces)
    for (panel_idx, _edge_name, preview, _eff_r) in fillet_previews {
        if let Ok(arc_local) = curves::flatten(
            [
                preview.start,
                preview.control_1,
                preview.control_2,
                preview.end,
            ],
            curves::FLATNESS_M,
        ) {
            let arc_mm: Vec<(f64, f64)> = arc_local
                .into_iter()
                .map(|pt| to_mm(panel_to_board_pt(template, panel_idx, pt)))
                .collect();
            let arc_d = dashed_commands(&arc_mm, 4.5 * px, 2.5 * px);
            if !arc_d.is_empty() {
                nodes.push(BoardPath::new(arc_d, None, accent_emerald, 2.4 * px, ""));
            }
        }
        let s_mm = to_mm(panel_to_board_pt(template, panel_idx, preview.start));
        let e_mm = to_mm(panel_to_board_pt(template, panel_idx, preview.end));
        let c_mm = to_mm(panel_to_board_pt(template, panel_idx, preview.corner));
        let k_mm = to_mm(panel_to_board_pt(
            template,
            panel_idx,
            fillet_drag_handle_local(&preview),
        ));
        let guide_d = dashed_commands(&[c_mm, k_mm], 3.0 * px, 2.5 * px);
        if !guide_d.is_empty() {
            nodes.push(BoardPath::new(
                guide_d,
                None,
                Some(Color::rgba(16, 185, 129, 140)),
                1.0 * px,
                "",
            ));
        }
        nodes.push(BoardPath::new(
            circle(s_mm.0, s_mm.1, 2.8 * px),
            accent_emerald,
            Some(Color::rgb(255, 255, 255)),
            0.9 * px,
            "",
        ));
        nodes.push(BoardPath::new(
            circle(e_mm.0, e_mm.1, 2.8 * px),
            accent_emerald,
            Some(Color::rgb(255, 255, 255)),
            0.9 * px,
            "",
        ));
        let knob_active = matches!(
            state.hovered_2d,
            Some(Hover2dTarget::CornerPreviewHandle { panel_idx: hi, .. }) if hi == panel_idx
        ) || matches!(state.drag_2d, Some(Drag2d::CornerRadius { .. }));
        nodes.push(BoardPath::new(
            circle(
                k_mm.0,
                k_mm.1,
                if knob_active { 5.4 * px } else { 4.4 * px },
            ),
            accent_emerald,
            if knob_active {
                accent_gold
            } else {
                Some(Color::rgb(255, 255, 255))
            },
            1.4 * px,
            "bezier-handle",
        ));
    }

    // Seam links between actual sewn boundary midpoints
    let edge_anchor_mm = |panel_id: &str, edge: Option<&str>, fallback: (f64, f64)| -> (f64, f64) {
        if let Some(edge_name) = edge
            && let Some(pidx) = panels.iter().position(|p| p.id == panel_id)
            && let Some(ref poly) = panels[pidx].polygon
            && let Some(eidx) = poly.edges.iter().position(|e| e == edge_name)
            && let Ok(pts) = poly.edge_points(eidx)
            && pts.len() >= 2
        {
            let total: f64 = pts.windows(2).map(|w| w[0].distance(w[1])).sum();
            let mut target = total * 0.5;
            for pair in pts.windows(2) {
                let len = pair[0].distance(pair[1]);
                if len > 1e-12 && target <= len {
                    let mid = pair[0].lerp(pair[1], target / len);
                    return to_mm(panel_to_board_pt(template, pidx, mid));
                }
                target -= len;
            }
            return to_mm(panel_to_board_pt(template, pidx, pts[pts.len() / 2]));
        }
        fallback
    };

    for seam in &template.seams {
        let p_a = panel_geos.iter().find(|p| p.id == seam.a.panel);
        let p_b = panel_geos.iter().find(|p| p.id == seam.b.panel);
        if let (Some(a), Some(b)) = (p_a, p_b) {
            let pt_a = edge_anchor_mm(&a.id, seam.a.edge.as_deref(), a.center);
            let pt_b = edge_anchor_mm(&b.id, seam.b.edge.as_deref(), b.center);
            let d = dashed_commands(&[pt_a, pt_b], 5.5 * px, 4.5 * px);
            if !d.is_empty() {
                nodes.push(BoardPath::new(
                    d,
                    None,
                    Some(Color::rgba(16, 185, 129, 140)),
                    1.1 * px,
                    "",
                ));
            }
        }
    }

    if state.active_2d_tool == "Measure" {
        for (idx, panel) in template.panels.iter().enumerate() {
            let Some(poly) = &panel.polygon else { continue };
            for measurement in &poly.measurements {
                if let Ok(points) = poly.measurement_points(measurement) {
                    let ends = points.map(|p| to_mm(panel_to_board_pt(template, idx, p)));
                    nodes.push(BoardPath::new(
                        polyline(&ends, false),
                        None,
                        hover_cyan,
                        2. * px,
                        "",
                    ));
                }
            }
        }
    }
    // Active 2D tool draft overlays (Rect, Pen, Freehand, InternalLine, Measure, Sew)
    if let Some(ref draft) = state.draft_2d {
        match draft {
            ToolDraft2d::Rect {
                start_board_m,
                current_board_m,
            } => {
                let a = to_mm(*start_board_m);
                let b = to_mm(*current_board_m);
                let rx = a.0.min(b.0);
                let ry = a.1.min(b.1);
                let rw = (a.0 - b.0).abs().max(0.5);
                let rh = (a.1 - b.1).abs().max(0.5);
                let outline = vec![
                    (rx, ry),
                    (rx + rw, ry),
                    (rx + rw, ry + rh),
                    (rx, ry + rh),
                    (rx, ry),
                ];
                let d = dashed_commands(&outline, 5.0 * px, 3.0 * px);
                nodes.push(BoardPath::new(
                    rectangle(rx, ry, rw, rh),
                    Some(Color::rgba(59, 130, 246, 36)),
                    None,
                    0.0,
                    "tool-draft-rect",
                ));
                if !d.is_empty() {
                    nodes.push(BoardPath::new(d, None, hover_cyan, 1.8 * px, ""));
                }
            }
            ToolDraft2d::Pen {
                points_board_m,
                hover_board_m,
            } => {
                let mut pts_mm: Vec<(f64, f64)> =
                    points_board_m.iter().map(|&pt| to_mm(pt)).collect();
                if let Some(h) = hover_board_m {
                    pts_mm.push(to_mm(*h));
                }
                if pts_mm.len() >= 2 {
                    let pts_str = polyline(&pts_mm, false);
                    nodes.push(BoardPath::new(
                        pts_str,
                        None,
                        hover_cyan,
                        1.8 * px,
                        "tool-draft-pen",
                    ));
                }
                for (idx, pt) in points_board_m.iter().enumerate() {
                    let (px_mm, py_mm) = to_mm(*pt);
                    let r = if idx == 0 { 4.2 * px } else { 3.0 * px };
                    let fill = if idx == 0 { accent_emerald } else { hover_cyan };
                    nodes.push(BoardPath::new(
                        circle(px_mm, py_mm, r),
                        fill,
                        Some(Color::rgb(255, 255, 255)),
                        1.0 * px,
                        "",
                    ));
                }
            }
            ToolDraft2d::Freehand { samples_board_m } => {
                if samples_board_m.len() >= 2 {
                    let pts_mm: Vec<_> = samples_board_m.iter().map(|&pt| to_mm(pt)).collect();
                    let pts_str = polyline(&pts_mm, false);
                    nodes.push(BoardPath::new(
                        pts_str,
                        None,
                        accent_gold,
                        1.8 * px,
                        "tool-draft-freehand",
                    ));
                }
            }
            ToolDraft2d::Sew { .. } | ToolDraft2d::SewConfirm { .. } => {
                let pending: Vec<&(String, String)> = match draft {
                    ToolDraft2d::Sew { first_edge } => first_edge.iter().collect(),
                    ToolDraft2d::SewConfirm { a, b, .. } => vec![a, b],
                    _ => Vec::new(),
                };
                for (pid, ename) in pending {
                    let Some(pg) = panel_geos.iter().find(|g| &g.id == pid) else {
                        continue;
                    };
                    let Some(eidx) = pg.edge_names.iter().position(|e| e == ename) else {
                        continue;
                    };
                    let (ax, ay) = pg.points[eidx];
                    let (bx, by) = pg.points[(eidx + 1) % pg.points.len()];
                    let seg_d = if let Some(c) = pg.curves.iter().find(|c| c.edge_idx == eidx) {
                        vec![
                            MoveTo(point((ax, ay))),
                            CubicTo {
                                control1: point(c.c1),
                                control2: point(c.c2),
                                end: point((bx, by)),
                            },
                        ]
                    } else {
                        polyline(&[(ax, ay), (bx, by)], false)
                    };
                    nodes.push(BoardPath::new(
                        seg_d,
                        None,
                        accent_gold,
                        3.2 * px,
                        "tool-draft-sew",
                    ));
                }
            }
            ToolDraft2d::InternalLine {
                panel_id,
                points_local_m,
                hover_local_m,
            } => {
                if let Some(pidx) = panels.iter().position(|p| &p.id == panel_id) {
                    let mut pts_mm: Vec<(f64, f64)> = points_local_m
                        .iter()
                        .map(|&pt| to_mm(panel_to_board_pt(template, pidx, pt)))
                        .collect();
                    if let Some(h) = hover_local_m {
                        pts_mm.push(to_mm(panel_to_board_pt(template, pidx, *h)));
                    }
                    let d = dashed_commands(&pts_mm, 4.0 * px, 3.0 * px);
                    if !d.is_empty() {
                        nodes.push(BoardPath::new(
                            d,
                            None,
                            accent_gold,
                            1.8 * px,
                            "tool-draft-internal",
                        ));
                    }
                    for &(mx, my) in &pts_mm {
                        nodes.push(BoardPath::new(
                            circle(mx, my, 2.8 * px),
                            accent_gold,
                            Some(Color::rgb(255, 255, 255)),
                            0.9 * px,
                            "",
                        ));
                    }
                }
            }
            ToolDraft2d::Measure {
                panel_id,
                first,
                hover_board_m,
            } => {
                if let Some((idx, panel)) = template
                    .panels
                    .iter()
                    .enumerate()
                    .find(|(_, p)| &p.id == panel_id)
                    && let Some(a) = panel
                        .polygon
                        .as_ref()
                        .and_then(|p| p.measurement_anchor_position(first).ok())
                {
                    let am = to_mm(panel_to_board_pt(template, idx, a));
                    if let Some(b) = *hover_board_m {
                        let bm = to_mm(b);
                        let d = dashed_commands(&[am, bm], 4.0 * px, 3.0 * px);
                        if !d.is_empty() {
                            nodes.push(BoardPath::new(
                                d,
                                None,
                                hover_cyan,
                                2.0 * px,
                                "tool-draft-measure",
                            ));
                        }
                        nodes.push(BoardPath::new(
                            circle(bm.0, bm.1, 3.5 * px),
                            hover_cyan,
                            Some(Color::rgb(255, 255, 255)),
                            1.0 * px,
                            "",
                        ));
                    }
                    nodes.push(BoardPath::new(
                        circle(am.0, am.1, 3.8 * px),
                        accent_gold,
                        Some(Color::rgb(255, 255, 255)),
                        1.1 * px,
                        "",
                    ));
                }
            }
        }
    }

    if let Some(Drag2d::Marquee {
        start_board_m,
        current_board_m,
        moved: true,
        ..
    }) = &state.drag_2d
    {
        let lo = start_board_m.min(*current_board_m) * 1000.0;
        let size = (*current_board_m - *start_board_m).abs() * 1000.0;
        nodes.push(BoardPath::new(
            rectangle(lo.x, lo.y, size.x, size.y),
            Some(Color::rgba(66, 185, 232, 34)),
            hover_cyan,
            px,
            "selection-marquee",
        ));
    }
    (nodes, view_box)
}
