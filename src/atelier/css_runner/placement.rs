//! Pending 3D placement edits from the Arrange number fields.
//!
//! Typing or dragging a placement field only updates a draft for one piece.
//! The draft becomes a single `Command::Place` (one undo step, one restart of
//! that piece) when the edit ends: the drag is released, the text field loses
//! focus, Enter is pressed, or another pointer press begins. Escape discards it.
//!
//! Every draft change travels through the runner's action queue: field values
//! as `SetNumber`, endings as `CommitPlacement` / `CancelPlacement`. The queue
//! preserves event order, so a keystroke queued before Enter or Escape is
//! applied first. Each value names the piece its field was built for, so a
//! selection change between the event and its processing cannot retarget it.

use super::editing::commit_authoring_command;
use super::state::CssRunnerState;
use clothcraft::authoring::Command;
use clothcraft::garment::template::Placement;
use std::sync::atomic::Ordering;

/// One editable component of a piece's 3D placement, in centimetres or degrees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacementAxis {
    TranslationX,
    TranslationY,
    TranslationZ,
    RotationX,
    RotationY,
    RotationZ,
}

impl PlacementAxis {
    pub const ALL: [Self; 6] = [
        Self::TranslationX,
        Self::TranslationY,
        Self::TranslationZ,
        Self::RotationX,
        Self::RotationY,
        Self::RotationZ,
    ];

    pub fn is_rotation(self) -> bool {
        matches!(self, Self::RotationX | Self::RotationY | Self::RotationZ)
    }

    /// The displayed value: centimetres for translation, degrees for rotation.
    pub fn get(self, p: &Placement) -> f64 {
        match self {
            Self::TranslationX => p.translation.x * 100.0,
            Self::TranslationY => p.translation.y * 100.0,
            Self::TranslationZ => p.translation.z * 100.0,
            Self::RotationX => p.rotation_degrees.x,
            Self::RotationY => p.rotation_degrees.y,
            Self::RotationZ => p.rotation_degrees.z,
        }
    }

    pub fn set(self, p: &mut Placement, value: f64) {
        match self {
            Self::TranslationX => p.translation.x = value / 100.0,
            Self::TranslationY => p.translation.y = value / 100.0,
            Self::TranslationZ => p.translation.z = value / 100.0,
            Self::RotationX => p.rotation_degrees.x = value,
            Self::RotationY => p.rotation_degrees.y = value,
            Self::RotationZ => p.rotation_degrees.z = value,
        }
    }
}

/// An uncommitted placement for one piece, identified by panel id so a later
/// selection change still commits it to the piece that was edited.
#[derive(Clone, Debug)]
pub struct PlacementDraft {
    pub panel_id: String,
    pub placement: Placement,
}

fn same_placement(a: &Placement, b: &Placement) -> bool {
    a.translation == b.translation && a.rotation_degrees == b.rotation_degrees
}

/// Value shown in a placement field for `panel`: its pending draft, otherwise
/// the document's placement (zero if the piece no longer exists).
pub fn placement_value(state: &CssRunnerState, panel: &str, axis: PlacementAxis) -> f64 {
    let placement = match &state.placement_draft {
        Some(draft) if draft.panel_id == panel => &draft.placement,
        _ => match state.project.template.panels.iter().find(|p| p.id == panel) {
            Some(p) => &p.placement,
            None => return 0.0,
        },
    };
    axis.get(placement)
}

/// Record a typed or dragged value in `panel`'s draft. A draft for another
/// piece is committed first; values for a deleted piece are ignored.
pub fn set_placement_draft(
    state: &mut CssRunnerState,
    panel: &str,
    axis: PlacementAxis,
    value: f64,
) {
    if state
        .placement_draft
        .as_ref()
        .is_none_or(|draft| draft.panel_id != panel)
    {
        commit_placement_draft(state);
        let Some(p) = state.project.template.panels.iter().find(|p| p.id == panel) else {
            return;
        };
        state.placement_draft = Some(PlacementDraft {
            panel_id: p.id.clone(),
            placement: p.placement.clone(),
        });
    }
    if let Some(draft) = state.placement_draft.as_mut() {
        axis.set(&mut draft.placement, value);
    }
}

/// Discard the pending draft without touching the document or history.
pub fn cancel_placement_draft(state: &mut CssRunnerState) -> bool {
    state.placement_draft.take().is_some()
}

/// Commit the pending draft as one `Command::Place`. An unchanged draft, or one
/// for a piece that no longer exists, is dropped without touching history.
/// The current selection is kept: the commit may be processed after the user
/// has already selected another piece.
pub fn commit_placement_draft(state: &mut CssRunnerState) -> bool {
    let Some(PlacementDraft {
        panel_id,
        placement,
    }) = state.placement_draft.take()
    else {
        return false;
    };
    let Some(panel) = state
        .project
        .template
        .panels
        .iter()
        .find(|p| p.id == panel_id)
    else {
        return false;
    };
    if same_placement(&panel.placement, &placement) {
        return false;
    }
    let name = panel.name.clone();
    let t = placement.translation * 100.0;
    let r = placement.rotation_degrees;
    let message = format!(
        "Placed {name} at {:.1}/{:.1}/{:.1} cm, {:.0}°/{:.0}°/{:.0}°",
        t.x, t.y, t.z, r.x, r.y, r.z
    );
    // Place does not add, remove or reorder pieces, so the index stays valid.
    let selected = state.selected_piece_index;
    let committed = commit_authoring_command(
        state,
        Command::Place {
            panel: panel_id,
            placement,
        },
        message,
    );
    state.selected_piece_index = selected;
    committed
}

/// Commit the draft once editing has ended: no number drag in progress and no
/// numeric text field focused. Runs after each frame's queued actions.
pub fn commit_finished_placement_draft(state: &mut CssRunnerState) -> bool {
    if state.placement_draft.is_none()
        || state.numeric_input_active.load(Ordering::Relaxed)
        || state.number_drag.lock().unwrap().active.is_some()
    {
        return false;
    }
    commit_placement_draft(state)
}
