//! Native garment workspace runner.
//!
//! State/actions, editing, board math/drawing, input and UI composition live in
//! focused modules. Public re-exports preserve the existing runner API.

#![allow(dead_code)]

mod actions;
mod app;
mod board;
mod documents;
mod editing;
mod input;
mod mesh_preparation;
mod placement;
mod state;
mod ui;

#[cfg(test)]
mod tests;

// The runner is compiled as a library API and as part of cloth-fit. Some API
// re-exports are used only by library consumers and integration tests.
#[allow(unused_imports)]
pub use actions::{RunnerAction, RunnerUiOp, update_runner};
#[allow(unused_imports)]
pub use app::{create_runner_app, run};
#[allow(unused_imports)]
pub use board::geometry::{
    cad_2d_bar_layout, compute_2d_view_box_m, compute_2d_viewport_rect, screen_to_board_m,
    screen_to_board_unclamped_m, viewport_2d_span_x, viewport_3d_rect,
};
#[allow(unused_imports)]
pub use board::native as native_board;
#[allow(unused_imports)]
pub use board::rendering::render_runner_2d_paths;
#[allow(unused_imports)]
pub use board::types::{Drag2d, Hover2dTarget, Rect2d, ToolDraft2d};
#[allow(unused_imports)]
pub use editing::{commit_authoring_command, undo_redo_2d};
#[allow(unused_imports)]
pub use input::handle_runner_event;
#[allow(unused_imports)]
pub use state::{CssRunnerState, ObjectCategory};
#[allow(unused_imports)]
pub use ui::shell::build_runner_ui;
#[allow(unused_imports)]
pub use ui::viewports::build_runner_2d_viewport;
