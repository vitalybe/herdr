use std::path::PathBuf;

use crate::app::{App, Mode};
use crate::layout::PaneId;
use crate::pane::PaneLaunchEnv;
use crate::popup_size::{resolve_popup_geometry, PopupSize};
use crate::terminal::{TerminalId, TerminalRuntime, TerminalState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PopupGeometry {
    pub width: Option<PopupSize>,
    pub height: Option<PopupSize>,
}

impl App {
    pub(crate) fn popup_runtime(&self) -> Option<&TerminalRuntime> {
        let terminal_id = &self.state.popup_pane.as_ref()?.terminal_id;
        self.terminal_runtimes.get(terminal_id)
    }

    pub(crate) fn close_popup_pane(&mut self) -> bool {
        let Some(popup) = self.state.popup_pane.take() else {
            return false;
        };
        self.state
            .direct_attach_resize_locks
            .remove(&popup.terminal_id);
        self.state.terminals.remove(&popup.terminal_id);
        self.shutdown_terminal_runtime(popup.terminal_id);
        self.state.mode = if self.state.active.is_some() {
            Mode::Terminal
        } else {
            Mode::Navigate
        };
        self.render_dirty.request_generic();
        self.render_notify.notify_one();
        true
    }

    pub(crate) fn try_route_paste_to_popup(&mut self, text: &str) -> bool {
        if self.state.popup_pane.is_none() {
            return false;
        }
        let Some(runtime) = self.popup_runtime() else {
            self.close_popup_pane();
            return true;
        };
        let _ = runtime.try_send_paste(text.to_owned());
        true
    }

    pub(crate) fn spawn_popup_shell_command(
        &mut self,
        command: &str,
        cwd: Option<PathBuf>,
        extra_env: Vec<(String, String)>,
        geometry: PopupGeometry,
    ) -> std::io::Result<()> {
        self.spawn_popup_command(
            cwd,
            extra_env,
            geometry,
            false,
            |pane_id, rows, cols, cwd, launch_env, app| {
                TerminalRuntime::spawn_shell_command(
                    pane_id,
                    rows,
                    cols,
                    cwd,
                    command,
                    launch_env,
                    crate::pane::AgentDetection::Disabled,
                    app.state.pane_scrollback_limit_bytes,
                    app.state.host_terminal_theme,
                    app.state.host_terminal_appearance,
                    app.event_tx.clone(),
                    app.render_notify.clone(),
                    app.render_dirty.clone(),
                )
                .map(|runtime| (runtime, None))
            },
        )
    }

    pub(crate) fn spawn_popup_argv_command(
        &mut self,
        argv: &[String],
        cwd: Option<PathBuf>,
        extra_env: Vec<(String, String)>,
        geometry: PopupGeometry,
    ) -> std::io::Result<()> {
        self.spawn_popup_command(
            cwd,
            extra_env,
            geometry,
            false,
            |pane_id, rows, cols, cwd, launch_env, app| {
                TerminalRuntime::spawn_argv_command(
                    pane_id,
                    rows,
                    cols,
                    cwd,
                    argv,
                    launch_env,
                    crate::pane::AgentDetection::Disabled,
                    app.state.pane_scrollback_limit_bytes,
                    app.state.host_terminal_theme,
                    app.state.host_terminal_appearance,
                    app.event_tx.clone(),
                    app.render_notify.clone(),
                    app.render_dirty.clone(),
                )
                .map(|runtime| (runtime, Some(argv.to_vec())))
            },
        )
    }

    fn spawn_popup_command<F>(
        &mut self,
        cwd: Option<PathBuf>,
        extra_env: Vec<(String, String)>,
        geometry: PopupGeometry,
        scratch: bool,
        spawn: F,
    ) -> std::io::Result<()>
    where
        F: FnOnce(
            PaneId,
            u16,
            u16,
            PathBuf,
            &PaneLaunchEnv,
            &mut App,
        ) -> std::io::Result<(TerminalRuntime, Option<Vec<String>>)>,
    {
        if self.state.popup_pane.is_some() {
            return Err(std::io::Error::other("popup already open"));
        }
        let Some(ws_idx) = self.state.active else {
            return Err(std::io::Error::other("no active workspace"));
        };
        let ws = self
            .state
            .workspaces
            .get(ws_idx)
            .ok_or_else(|| std::io::Error::other("active workspace disappeared"))?;
        let active_tab = ws
            .active_tab()
            .ok_or_else(|| std::io::Error::other("active tab disappeared"))?;
        let focused_pane = ws
            .focused_pane_id()
            .ok_or_else(|| std::io::Error::other("active tab has no focused pane"))?;
        let cwd = cwd.or_else(|| {
            active_tab.cwd_for_pane(focused_pane, &self.state.terminals, &self.terminal_runtimes)
        });
        let cwd = cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));
        let pane_id = PaneId::alloc();
        let terminal_id = TerminalId::alloc();
        // The scratch terminal can be promoted into a real pane, so it keeps its
        // pane identity; command popups stay anonymous.
        let launch_env = PaneLaunchEnv::from_extra(extra_env);
        let launch_env = if scratch {
            launch_env
        } else {
            launch_env.without_pane_identity()
        };
        let terminal_area = if self.state.view.terminal_area.width >= 4
            && self.state.view.terminal_area.height >= 4
        {
            self.state.view.terminal_area
        } else {
            let (estimated_rows, estimated_cols) = self.state.estimate_pane_size();
            ratatui::layout::Rect::new(0, 0, estimated_cols, estimated_rows)
        };
        let Some(resolved_geometry) =
            resolve_popup_geometry(geometry.width, geometry.height, terminal_area)
        else {
            return Err(std::io::Error::other("terminal area too small for popup"));
        };
        let rows = resolved_geometry.inner.height;
        let cols = resolved_geometry.inner.width;
        let (runtime, launch_argv) = spawn(pane_id, rows, cols, cwd.clone(), &launch_env, self)?;
        let terminal = match launch_argv {
            Some(argv) => TerminalState::new(terminal_id.clone(), cwd).with_launch_argv(argv),
            None => TerminalState::new(terminal_id.clone(), cwd),
        };
        self.terminal_runtimes.insert(terminal_id.clone(), runtime);
        self.state.terminals.insert(terminal_id.clone(), terminal);
        self.state.popup_pane = Some(crate::app::state::PopupPaneState {
            pane_id,
            terminal_id,
            width: geometry.width,
            height: geometry.height,
            scratch,
        });
        self.state.mode = Mode::Terminal;
        Ok(())
    }

    /// Show the session scratch terminal, or hide it when it is already up.
    /// The terminal keeps running while hidden, so reopening keeps its history.
    pub(crate) fn toggle_scratch_popup(&mut self) {
        if self.hide_scratch_popup() {
            return;
        }
        if self.state.popup_pane.is_some() {
            return;
        }
        if let Some(popup) = self.state.hidden_scratch_popup.take() {
            if self.terminal_runtimes.get(&popup.terminal_id).is_some() {
                self.state.popup_pane = Some(popup);
                self.state.mode = Mode::Terminal;
                self.render_dirty.request_generic();
                self.render_notify.notify_one();
                return;
            }
            self.state.terminals.remove(&popup.terminal_id);
        }
        if let Err(err) = self.spawn_scratch_popup() {
            tracing::warn!(err = %err, "failed to open scratch terminal");
        }
    }

    /// Hide a visible scratch terminal without ending its process.
    pub(crate) fn hide_scratch_popup(&mut self) -> bool {
        if !self
            .state
            .popup_pane
            .as_ref()
            .is_some_and(|popup| popup.scratch)
        {
            return false;
        }
        let Some(popup) = self.state.popup_pane.take() else {
            return false;
        };
        self.state
            .direct_attach_resize_locks
            .remove(&popup.terminal_id);
        self.state.hidden_scratch_popup = Some(popup);
        self.state.mode = if self.state.active.is_some() {
            Mode::Terminal
        } else {
            Mode::Navigate
        };
        self.render_dirty.request_generic();
        self.render_notify.notify_one();
        true
    }

    /// Move the scratch terminal into a tab of its own, keeping its history.
    /// The next toggle then starts a fresh scratch terminal.
    pub(crate) fn scratch_popup_to_tab(&mut self) -> bool {
        let Some(ws_idx) = self.state.active else {
            return false;
        };
        let popup = if self
            .state
            .popup_pane
            .as_ref()
            .is_some_and(|popup| popup.scratch)
        {
            self.state.popup_pane.take()
        } else {
            self.state.hidden_scratch_popup.take()
        };
        let Some(popup) = popup else {
            return false;
        };
        self.state
            .direct_attach_resize_locks
            .remove(&popup.terminal_id);
        if self.terminal_runtimes.get(&popup.terminal_id).is_none() {
            self.state.terminals.remove(&popup.terminal_id);
            return false;
        }
        let moved = crate::workspace::MovedPane {
            pane_id: popup.pane_id,
            pane_state: crate::pane::PaneState::new(popup.terminal_id),
        };
        let tab_idx = self.state.workspaces[ws_idx].create_tab_from_existing_pane(
            moved,
            None,
            self.event_tx.clone(),
            self.render_notify.clone(),
            self.render_dirty.clone(),
        );
        self.state.workspaces[ws_idx].active_tab = tab_idx;
        self.state.mode = Mode::Terminal;
        self.state.mark_session_dirty();
        self.schedule_session_save();
        self.render_dirty.request_generic();
        self.render_notify.notify_one();
        true
    }

    /// Drop the hidden scratch terminal once its process is gone.
    pub(crate) fn discard_hidden_scratch_popup(&mut self, pane_id: PaneId) -> bool {
        let Some(popup) = self
            .state
            .hidden_scratch_popup
            .take_if(|popup| popup.pane_id == pane_id)
        else {
            return false;
        };
        self.state
            .direct_attach_resize_locks
            .remove(&popup.terminal_id);
        self.state.terminals.remove(&popup.terminal_id);
        self.shutdown_terminal_runtime(popup.terminal_id);
        true
    }

    fn spawn_scratch_popup(&mut self) -> std::io::Result<()> {
        self.spawn_popup_command(
            None,
            Vec::new(),
            PopupGeometry::default(),
            true,
            |pane_id, rows, cols, cwd, launch_env, app| {
                TerminalRuntime::spawn(
                    pane_id,
                    rows,
                    cols,
                    cwd,
                    app.state.pane_scrollback_limit_bytes,
                    app.state.host_terminal_theme,
                    app.state.host_terminal_appearance,
                    crate::pane::PaneShellConfig::new(
                        &app.state.default_shell,
                        app.state.shell_mode,
                    ),
                    launch_env,
                    app.event_tx.clone(),
                    app.render_notify.clone(),
                    app.render_dirty.clone(),
                )
                .map(|runtime| (runtime, None))
            },
        )
    }
}

#[cfg(test)]
impl App {
    pub(crate) fn install_test_popup_runtime(
        &mut self,
        runtime: TerminalRuntime,
    ) -> (PaneId, TerminalId) {
        let pane_id = PaneId::alloc();
        let terminal_id = TerminalId::alloc();
        self.terminal_runtimes.insert(terminal_id.clone(), runtime);
        self.state.terminals.insert(
            terminal_id.clone(),
            TerminalState::new(terminal_id.clone(), PathBuf::from("/popup")),
        );
        self.state.popup_pane = Some(crate::app::state::PopupPaneState {
            pane_id,
            terminal_id: terminal_id.clone(),
            width: None,
            height: None,
            scratch: false,
        });
        (pane_id, terminal_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_popup() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            true,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = vec![crate::workspace::Workspace::test_new("popup")];
        app.state.active = Some(0);
        app.state.selected = 0;
        let terminal_id = TerminalId::alloc();
        app.state.terminals.insert(
            terminal_id.clone(),
            TerminalState::new(terminal_id.clone(), PathBuf::from("/popup")),
        );
        app.state.popup_pane = Some(crate::app::state::PopupPaneState {
            pane_id: PaneId::alloc(),
            terminal_id,
            width: None,
            height: None,
            scratch: false,
        });
        app
    }

    fn app_with_scratch_popup() -> (App, PaneId, TerminalId) {
        let mut app = app_with_popup();
        let (runtime, _rx) = crate::terminal::TerminalRuntime::test_with_channel(40, 12);
        let (pane_id, terminal_id) = app.install_test_popup_runtime(runtime);
        if let Some(popup) = app.state.popup_pane.as_mut() {
            popup.scratch = true;
        }
        (app, pane_id, terminal_id)
    }

    #[tokio::test]
    async fn scratch_popup_hides_and_reopens_with_the_same_terminal() {
        let (mut app, _pane_id, terminal_id) = app_with_scratch_popup();

        assert!(app.hide_scratch_popup());
        assert!(app.state.popup_pane.is_none());
        assert_eq!(
            app.state
                .hidden_scratch_popup
                .as_ref()
                .map(|popup| popup.terminal_id.clone()),
            Some(terminal_id.clone())
        );
        assert!(app.state.terminals.contains_key(&terminal_id));

        app.toggle_scratch_popup();

        assert_eq!(
            app.state
                .popup_pane
                .as_ref()
                .map(|popup| popup.terminal_id.clone()),
            Some(terminal_id)
        );
        assert!(app.state.hidden_scratch_popup.is_none());
    }

    #[tokio::test]
    async fn scratch_popup_to_tab_keeps_the_running_terminal() {
        let (mut app, pane_id, terminal_id) = app_with_scratch_popup();
        let tabs_before = app.state.workspaces[0].tabs.len();

        assert!(app.scratch_popup_to_tab());

        assert!(app.state.popup_pane.is_none());
        assert!(app.state.hidden_scratch_popup.is_none());
        let ws = &app.state.workspaces[0];
        assert_eq!(ws.tabs.len(), tabs_before + 1);
        assert_eq!(ws.active_tab, ws.tabs.len() - 1);
        assert_eq!(
            ws.tabs[ws.active_tab].panes[&pane_id].attached_terminal_id,
            terminal_id
        );
        assert!(app.terminal_runtimes.get(&terminal_id).is_some());
    }

    #[tokio::test]
    async fn scratch_popup_to_tab_leaves_the_next_toggle_a_fresh_terminal() {
        let (mut app, _pane_id, terminal_id) = app_with_scratch_popup();
        assert!(app.hide_scratch_popup());

        assert!(app.scratch_popup_to_tab());

        assert!(app.state.hidden_scratch_popup.is_none());
        assert_ne!(
            app.state.workspaces[0].tabs.len(),
            0,
            "converted tab should exist"
        );
        assert!(app
            .state
            .workspaces
            .iter()
            .flat_map(|ws| ws.tabs.iter())
            .any(|tab| tab
                .panes
                .values()
                .any(|pane| pane.attached_terminal_id == terminal_id)));
    }

    #[test]
    fn close_popup_uses_terminal_mode_with_active_workspace() {
        let mut app = app_with_popup();
        app.state.mode = Mode::Navigate;

        assert!(app.close_popup_pane());

        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[test]
    fn close_popup_uses_navigate_mode_without_active_workspace() {
        let mut app = app_with_popup();
        app.state.workspaces.clear();
        app.state.active = None;
        app.state.mode = Mode::Navigate;

        assert!(app.close_popup_pane());

        assert_eq!(app.state.mode, Mode::Navigate);
    }

    #[test]
    fn close_popup_clears_direct_attach_resize_lock() {
        let mut app = app_with_popup();
        let terminal_id = app.state.popup_pane.as_ref().unwrap().terminal_id.clone();
        app.state
            .direct_attach_resize_locks
            .insert(terminal_id.clone());

        assert!(app.close_popup_pane());

        assert!(!app.state.direct_attach_resize_locks.contains(&terminal_id));
    }

    #[test]
    fn popup_survives_background_workspace_removal() {
        let mut app = app_with_popup();
        app.state.workspaces.clear();
        app.state.active = None;

        app.state.assert_invariants_for_test();

        assert!(app.state.popup_pane.is_some());
    }

    #[test]
    fn popup_close_api_closes_only_active_popup() {
        let mut app = app_with_popup();
        let close = || crate::api::schema::Request {
            id: "close-popup".into(),
            method: crate::api::schema::Method::PopupClose(
                crate::api::schema::EmptyParams::default(),
            ),
        };

        let response = app.handle_api_request(close());
        let response: crate::api::schema::SuccessResponse =
            serde_json::from_str(&response).unwrap();
        assert_eq!(response.result, crate::api::schema::ResponseResult::Ok {});

        let response = app.handle_api_request(close());
        let response: crate::api::schema::ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(response.error.code, "popup_not_open");
    }
}
