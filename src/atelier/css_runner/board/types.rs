//! Transient board picking, drag and draft types.

use glam::DVec2;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect2d {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect2d {
    pub fn contains(self, px: f32, py: f32) -> bool {
        self.w > 0.0
            && self.h > 0.0
            && px >= self.x
            && px <= self.x + self.w
            && py >= self.y
            && py <= self.y + self.h
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Hover2dTarget {
    Panel(usize),
    Edge {
        panel_idx: usize,
        edge_idx: usize,
    },
    Point {
        panel_idx: usize,
        point_idx: usize,
    },
    BezierHandle {
        panel_idx: usize,
        edge_idx: usize,
        handle_index: usize,
    },
    CornerPreviewHandle {
        panel_idx: usize,
        edge_name: String,
    },
}

#[derive(Clone, Debug)]
pub enum Drag2d {
    Marquee {
        start_board_m: DVec2,
        current_board_m: DVec2,
        start_pointer: (f32, f32),
        moved: bool,
        click_point: Option<(String, usize)>,
    },
    Pan {
        start_pan: (f64, f64),
        start_pointer: (f32, f32),
    },
    Point {
        panel_id: String,
        point_idx: usize,
        points: Vec<(String, usize)>,
        start_board_m: DVec2,
        start_pointer: (f32, f32),
        moved: bool,
        last_local: DVec2,
        last_board_m: DVec2,
        collapse_on_click: Option<(String, usize)>,
    },
    BezierHandle {
        panel_id: String,
        edge_name: String,
        handle_index: usize,
        start_pointer: (f32, f32),
        moved: bool,
        last_local: DVec2,
    },
    CornerRadius {
        panel_id: String,
        edge_name: String,
        batch_corners: Vec<(String, String)>,
        corner_board_m: DVec2,
        bisector: DVec2,
        denom: f64,
        max_radius_m: f64,
        start_pointer: (f32, f32),
        moved: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolDraft2d {
    Rect {
        start_board_m: DVec2,
        current_board_m: DVec2,
    },
    Pen {
        points_board_m: Vec<DVec2>,
        hover_board_m: Option<DVec2>,
    },
    Freehand {
        samples_board_m: Vec<DVec2>,
    },
    Sew {
        first_edge: Option<(String, String)>,
    },
    /// Both boundaries are picked; nothing is committed until the user
    /// confirms. Unequal lengths can only be sewn with explicit easing.
    SewConfirm {
        a: (String, String),
        b: (String, String),
        /// Traverse `b` against `a`, as for adjacent edges of one contour.
        reverse: bool,
        a_len_m: f64,
        b_len_m: f64,
    },
    InternalLine {
        panel_id: String,
        points_local_m: Vec<DVec2>,
        hover_local_m: Option<DVec2>,
    },
    Measure {
        panel_id: String,
        first: clothcraft::garment::polygon::MeasurementAnchor,
        hover_board_m: Option<DVec2>,
    },
}
