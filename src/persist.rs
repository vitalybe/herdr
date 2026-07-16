//! Session persistence — save/restore workspaces, layouts, and working directories.
//!
//! Stored at `~/.config/herdr/session.json`.
//! Optional pane screen history is stored separately at `session-history.json`.
//! Installed plugins are persisted separately at `plugins.json`.

mod io;
pub mod plugin_registry;
mod restore;
mod snapshot;

pub use self::io::{clear, clear_history, load, load_history, save};
pub use self::restore::restore;
#[cfg(unix)]
pub use self::restore::{handoff_pane_aliases, restore_handoff};
pub(crate) use self::restore::{restore_one_tab, restore_one_workspace};
pub use self::snapshot::{
    capture, capture_history, AgentManualEntrySnapshot, DirectionSnapshot, LayoutSnapshot,
    PaneSectionEntrySnapshot, SessionHistorySnapshot, SessionSnapshot, TabSnapshot,
    WorkspaceSnapshot,
};
pub(crate) use self::snapshot::{capture_tab_for_undo, capture_workspace_for_undo};
