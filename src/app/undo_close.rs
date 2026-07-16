//! Reopen recently closed tabs and workspaces.
//!
//! Closing a tab or workspace captures a snapshot onto [`AppState::closed_entries`]
//! (see `actions.rs`). Reopening rebuilds that structure with fresh shells using
//! the same restore machinery as session startup. Running processes and
//! scrollback are not revived, and agents are not auto-resumed.

use super::state::{ClosedEntry, ClosedWorkspaceEntry};
use super::{App, Mode};

impl App {
    /// Reopen the most recently closed tab or workspace. Returns true when
    /// something was restored.
    pub(crate) fn undo_last_close(&mut self) -> bool {
        let Some(entry) = self.state.closed_entries.pop() else {
            return false;
        };
        match entry {
            ClosedEntry::Workspaces(entries) => self.reopen_closed_workspaces(entries),
            ClosedEntry::Tab {
                workspace_id,
                index,
                snapshot,
            } => self.reopen_closed_tab(&workspace_id, index, *snapshot),
        }
    }

    fn reopen_closed_workspaces(&mut self, mut entries: Vec<ClosedWorkspaceEntry>) -> bool {
        // Insert lowest original index first so later inserts land after it.
        entries.sort_by_key(|entry| entry.index);
        let focus_id = entries.first().and_then(|entry| entry.snapshot.id.clone());

        let (rows, cols) = self.state.estimate_pane_size();
        let scrollback_limit_bytes = self.state.pane_scrollback_limit_bytes;
        let default_shell = self.state.default_shell.clone();
        let shell_mode = self.state.shell_mode;

        let mut restored_any = false;
        for entry in entries {
            let Some(parts) = crate::persist::restore_one_workspace(
                &entry.snapshot,
                rows,
                cols,
                scrollback_limit_bytes,
                crate::pane::PaneShellConfig::new(&default_shell, shell_mode),
                self.event_tx.clone(),
                self.render_notify.clone(),
                self.render_dirty.clone(),
            ) else {
                continue;
            };
            for terminal in parts.terminals {
                self.state.terminals.insert(terminal.id.clone(), terminal);
            }
            for (terminal_id, runtime) in parts.terminal_runtimes {
                self.terminal_runtimes.insert(terminal_id, runtime);
            }
            let insert_idx = entry.index.min(self.state.workspaces.len());
            self.state.workspaces.insert(insert_idx, parts.workspace);
            self.emit_workspace_open_events(insert_idx);
            restored_any = true;
        }

        if !restored_any {
            return false;
        }

        let focus_idx = focus_id
            .and_then(|id| self.state.workspaces.iter().position(|ws| ws.id == id))
            .unwrap_or(0)
            .min(self.state.workspaces.len().saturating_sub(1));
        self.state.selected = focus_idx;
        self.state.switch_workspace(focus_idx);
        self.state.mode = Mode::Terminal;
        self.schedule_session_save();
        true
    }

    fn reopen_closed_tab(
        &mut self,
        workspace_id: &str,
        index: usize,
        snapshot: crate::persist::TabSnapshot,
    ) -> bool {
        let Some(ws_idx) = self
            .state
            .workspaces
            .iter()
            .position(|ws| ws.id == workspace_id)
        else {
            // The workspace itself was closed after this tab; nothing to splice
            // into. Its own workspace-undo entry restores this tab with it.
            return false;
        };

        let (rows, cols) = self.state.estimate_pane_size();
        let scrollback_limit_bytes = self.state.pane_scrollback_limit_bytes;
        let default_shell = self.state.default_shell.clone();
        let shell_mode = self.state.shell_mode;
        let (first_public_pane_number, tab_number, ws_id) = {
            let ws = &self.state.workspaces[ws_idx];
            (
                ws.next_public_pane_number,
                ws.next_public_tab_number,
                ws.id.clone(),
            )
        };

        let Some(parts) = crate::persist::restore_one_tab(
            &snapshot,
            &ws_id,
            tab_number,
            first_public_pane_number,
            rows,
            cols,
            scrollback_limit_bytes,
            crate::pane::PaneShellConfig::new(&default_shell, shell_mode),
            self.event_tx.clone(),
            self.render_notify.clone(),
            self.render_dirty.clone(),
        ) else {
            return false;
        };

        for terminal in parts.terminals {
            self.state.terminals.insert(terminal.id.clone(), terminal);
        }
        for (terminal_id, runtime) in parts.terminal_runtimes {
            self.terminal_runtimes.insert(terminal_id, runtime);
        }

        let insert_idx = {
            let ws = &mut self.state.workspaces[ws_idx];
            ws.public_pane_numbers.extend(parts.public_pane_numbers);
            ws.next_public_pane_number = ws
                .next_public_pane_number
                .max(parts.next_public_pane_number);
            ws.next_public_tab_number = ws.next_public_tab_number.max(tab_number + 1);
            let insert_idx = index.min(ws.tabs.len());
            ws.tabs.insert(insert_idx, parts.tab);
            insert_idx
        };

        self.state.switch_workspace_tab_sticky(ws_idx, insert_idx);
        self.state.mode = Mode::Terminal;
        self.state.tab_scroll_follow_active = true;
        self.state.refresh_tab_bar_view();
        self.emit_tab_created_events(ws_idx, insert_idx);
        self.schedule_session_save();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::workspace::Workspace;

    #[cfg(windows)]
    fn test_shell() -> &'static str {
        "C:\\Windows\\System32\\whoami.exe"
    }

    #[cfg(not(windows))]
    fn test_shell() -> &'static str {
        "/bin/sh"
    }

    fn test_app() -> App {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(&Config::default(), true, None, api_rx, event_hub);
        app.state.default_shell = test_shell().to_string();
        app.state.shell_mode = crate::config::ShellModeConfig::NonLogin;
        app
    }

    #[tokio::test]
    async fn undo_reopens_closed_tab_in_place() {
        let mut app = test_app();
        let mut workspace = Workspace::test_new("tabs");
        workspace.test_add_tab(Some("logs"));
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.workspaces[0].switch_tab(1);

        app.state.close_tab();
        assert_eq!(app.state.workspaces[0].tabs.len(), 1);
        assert_eq!(app.state.closed_entries.len(), 1);

        assert!(app.undo_last_close());
        assert_eq!(app.state.workspaces[0].tabs.len(), 2);
        assert!(app.state.closed_entries.is_empty());
        // The reopened tab keeps its name and lands back at index 1.
        assert_eq!(
            app.state.workspaces[0].tabs[1].custom_name.as_deref(),
            Some("logs")
        );
    }

    #[tokio::test]
    async fn undo_reopens_closed_workspace_in_place() {
        let mut app = test_app();
        app.state.workspaces = vec![Workspace::test_new("a"), Workspace::test_new("b")];
        app.state.active = Some(1);
        app.state.selected = 0;
        let closed_id = app.state.workspaces[0].id.clone();

        app.state.close_selected_workspace();
        assert_eq!(app.state.workspaces.len(), 1);
        assert_eq!(app.state.closed_entries.len(), 1);

        assert!(app.undo_last_close());
        assert_eq!(app.state.workspaces.len(), 2);
        assert!(app.state.closed_entries.is_empty());
        // The reopened workspace keeps its stable id and returns to index 0.
        assert_eq!(app.state.workspaces[0].id, closed_id);
        assert_eq!(app.state.active, Some(0));
    }

    #[tokio::test]
    async fn undo_with_empty_stack_is_noop() {
        let mut app = test_app();
        app.state.workspaces = vec![Workspace::test_new("a")];
        app.state.active = Some(0);
        assert!(!app.undo_last_close());
    }
}
