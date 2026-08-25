use serde::{Deserialize, Serialize};

use crate::terminal::TerminalId;

/// Stable reference to a parent agent pane.
///
/// A child agent records the pane it was spawned from as its parent. The
/// reference is keyed by the parent's stable identity - its workspace id plus
/// its public pane number - rather than the volatile [`crate::layout::PaneId`],
/// so the parent/child link survives the pane-id remap on restore. This mirrors
/// how the manual agent order and tab-section order persist stable references.
///
/// This is a shared runtime fact (set at spawn time via the agent CLI) and is
/// exposed over the JSON API. Resolve it back to a live pane with
/// [`crate::app::state::AppState::resolve_pane_parent`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneParentRef {
    pub workspace_id: String,
    pub pane_number: usize,
}

/// Viewport state for a pane.
///
/// Terminal identity, cwd, labels, and agent metadata live in TerminalState.
/// The parent link is the exception: it is a stable cross-pane reference rather
/// than terminal state, so it lives here on the pane itself.
pub struct PaneState {
    pub attached_terminal_id: TerminalId,
    /// Whether the user has seen this pane since its last state change to Idle.
    /// False = "Done" (agent finished while user was in another workspace).
    pub seen: bool,
    /// Whether unmodified right-click gestures should be forwarded to the pane application.
    pub right_click_passthrough: bool,
    /// Stable reference to this pane's parent agent, set at spawn time when the
    /// agent was started with `--parent`. `None` for root agents. Persisted and
    /// exposed over the JSON API.
    pub parent: Option<PaneParentRef>,
}

impl PaneState {
    pub fn new(attached_terminal_id: TerminalId) -> Self {
        Self {
            attached_terminal_id,
            seen: true,
            right_click_passthrough: false,
            parent: None,
        }
    }
}
