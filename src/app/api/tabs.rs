use std::path::PathBuf;

use crate::api::schema::{
    EventData, EventEnvelope, EventKind, ResponseResult, TabCreateParams, TabListParams,
    TabMoveParams, TabMoveToWorkspaceParams, TabRenameParams, TabTarget,
};
use crate::app::{App, Mode};

use super::responses::{encode_error, encode_success};

impl App {
    pub(super) fn handle_tab_list(&mut self, id: String, params: TabListParams) -> String {
        let tabs = if let Some(workspace_id) = params.workspace_id {
            let Some(ws_idx) = self.parse_workspace_id(&workspace_id) else {
                return workspace_not_found(id, &workspace_id);
            };
            let Some(_) = self.state.workspaces.get(ws_idx) else {
                return workspace_not_found(id, &workspace_id);
            };
            self.tab_list_info(ws_idx)
        } else {
            let mut tabs = Vec::new();
            for (ws_idx, ws) in self.state.workspaces.iter().enumerate() {
                for tab_idx in 0..ws.tabs.len() {
                    if let Some(tab) = self.tab_info(ws_idx, tab_idx) {
                        tabs.push(tab);
                    }
                }
            }
            tabs
        };

        encode_success(id, ResponseResult::TabList { tabs })
    }

    pub(super) fn handle_tab_get(&mut self, id: String, target: TabTarget) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&target.tab_id) else {
            return tab_not_found(id, &target.tab_id);
        };
        let Some(tab) = self.tab_info(ws_idx, tab_idx) else {
            return tab_not_found(id, &target.tab_id);
        };

        encode_success(id, ResponseResult::TabInfo { tab })
    }

    pub(super) fn handle_tab_create(&mut self, id: String, params: TabCreateParams) -> String {
        let TabCreateParams {
            workspace_id,
            cwd,
            focus,
            label,
            index: insert_index,
            env,
        } = params;
        let ws_idx = if let Some(workspace_id) = workspace_id {
            let Some(ws_idx) = self.parse_workspace_id(&workspace_id) else {
                return workspace_not_found(id, &workspace_id);
            };
            ws_idx
        } else if let Some(active) = self.state.active {
            active
        } else {
            return encode_error(id, "workspace_not_found", "no active workspace");
        };
        // `index` names a slot in the sidebar Tabs band, which is the order a
        // caller sees. Reconcile first so the band is current, then check bounds
        // against it, so a bad index fails before a PTY is spawned. The slot only
        // places the row; the tab itself is appended to its space like any other.
        self.state.reconcile_pane_section_order();
        let workspace_id = self.state.workspaces[ws_idx].id.clone();
        if let Some(insert_index) = insert_index {
            if insert_index > self.state.pane_section_order.order.len() {
                return encode_error(
                    id,
                    "tab_create_failed",
                    format!("index {insert_index} is out of bounds"),
                );
            }
        }
        let cwd = cwd.map(PathBuf::from).unwrap_or_else(|| {
            self.resolve_new_terminal_cwd(self.focused_pane_cwd_in_workspace(ws_idx))
        });
        let (rows, cols) = self.state.estimate_pane_size();
        let default_shell = self.state.default_shell.clone();
        let scrollback_limit_bytes = self.state.pane_scrollback_limit_bytes;
        let host_terminal_theme = self.state.host_terminal_theme;
        let host_terminal_appearance = self.state.host_terminal_appearance;
        let extra_env = match super::env::normalize_launch_env(env) {
            Ok(env) => env,
            Err((code, message)) => return encode_error(id, &code, message),
        };
        let result = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .ok_or_else(|| std::io::Error::other("workspace disappeared"))
            .and_then(|ws| {
                ws.create_tab(
                    rows,
                    cols,
                    cwd,
                    scrollback_limit_bytes,
                    host_terminal_theme,
                    host_terminal_appearance,
                    crate::pane::PaneShellConfig::new(&default_shell, self.state.shell_mode),
                    extra_env,
                )
            });
        match result {
            Ok((tab_idx, terminal, runtime)) => {
                self.terminal_runtimes.insert(terminal.id.clone(), runtime);
                self.state.terminals.insert(terminal.id.clone(), terminal);
                // Give the new tab its band row, which reconcile appends, then
                // move it to the requested slot.
                self.state.reconcile_pane_section_order();
                if let Some(insert_index) = insert_index {
                    let tab_number = self.state.workspaces[ws_idx].tabs[tab_idx].number;
                    self.state
                        .pin_pane_section_slot(&workspace_id, tab_number, insert_index);
                }
                self.state.remove_alias_shadowed_by_new_pane(
                    self.state.workspaces[ws_idx].tabs[tab_idx].root_pane,
                );
                if let Some(label) = label {
                    let tab_id = self.public_tab_id(ws_idx, tab_idx).unwrap_or_else(|| {
                        crate::workspace::public_tab_id_for_number(&workspace_id, tab_idx + 1)
                    });
                    if let Some(tab) = self
                        .state
                        .workspaces
                        .get_mut(ws_idx)
                        .and_then(|ws| ws.tabs.get_mut(tab_idx))
                    {
                        tab.set_custom_name(label);
                        crate::logging::tab_renamed(&workspace_id, &tab_id);
                    }
                }
                if focus {
                    self.state.switch_workspace_tab_sticky(ws_idx, tab_idx);
                    self.state.mode = Mode::Terminal;
                }
                self.schedule_session_save();
                self.emit_tab_created_events(ws_idx, tab_idx);
                encode_success(
                    id,
                    self.tab_created_result(ws_idx, tab_idx)
                        .expect("new tab should produce a complete create response"),
                )
            }
            Err(err) => encode_error(id, "tab_create_failed", err.to_string()),
        }
    }

    pub(super) fn handle_tab_focus(&mut self, id: String, target: TabTarget) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&target.tab_id) else {
            return tab_not_found(id, &target.tab_id);
        };
        self.state.switch_workspace_tab_sticky(ws_idx, tab_idx);
        let tab = self.tab_info(ws_idx, tab_idx).unwrap();

        encode_success(id, ResponseResult::TabInfo { tab })
    }

    /// Names the reporting pane's tab after the session title its agent
    /// published, unless the tab was renamed by hand.
    pub(crate) fn apply_reported_session_title(
        &mut self,
        pane_id: crate::layout::PaneId,
        osc_title: &str,
    ) {
        let Some((ws_idx, tab_idx)) = self.state.apply_reported_session_title(pane_id, osc_title)
        else {
            return;
        };
        let Some(label) = self.state.workspaces[ws_idx].tab_display_name(tab_idx) else {
            return;
        };
        let Some(tab_id) = self.public_tab_id(ws_idx, tab_idx) else {
            return;
        };
        crate::logging::tab_renamed(&self.state.workspaces[ws_idx].id.clone(), &tab_id);
        self.schedule_session_save();
        self.emit_event(EventEnvelope {
            event: EventKind::TabRenamed,
            data: EventData::TabRenamed {
                tab_id,
                workspace_id: self.public_workspace_id(ws_idx),
                label,
            },
        });
        self.render_dirty.request_generic();
        self.render_notify.notify_one();
    }

    pub(super) fn handle_tab_rename(&mut self, id: String, params: TabRenameParams) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&params.tab_id) else {
            return tab_not_found(id, &params.tab_id);
        };
        let workspace_id = self.state.workspaces[ws_idx].id.clone();
        let tab_id = self.public_tab_id(ws_idx, tab_idx).unwrap_or_else(|| {
            crate::workspace::public_tab_id_for_number(&workspace_id, tab_idx + 1)
        });
        let Some(tab) = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .and_then(|ws| ws.tabs.get_mut(tab_idx))
        else {
            return tab_not_found(id, &params.tab_id);
        };
        // A rename over the socket API is advisory: it names the tab now, but the
        // session name an agent publishes supersedes it. Only a rename typed in
        // the TUI pins the name.
        tab.set_reported_name(params.label.clone());
        crate::logging::tab_renamed(&workspace_id, &tab_id);
        if self.state.active == Some(ws_idx) {
            // Reflow the tab bar so the new label width takes effect immediately.
            // The tab bar renders into cached hit areas; without this refresh the
            // old geometry lingers until the next refresh (e.g. a tab switch),
            // leaving the visible label stale. Mirrors handle_tab_move.
            self.state.refresh_tab_bar_view();
        }
        self.schedule_session_save();
        self.emit_event(EventEnvelope {
            event: EventKind::TabRenamed,
            data: EventData::TabRenamed {
                tab_id: self.public_tab_id(ws_idx, tab_idx).unwrap(),
                workspace_id: self.public_workspace_id(ws_idx),
                label: params.label,
            },
        });
        let tab = self.tab_info(ws_idx, tab_idx).unwrap();

        encode_success(id, ResponseResult::TabInfo { tab })
    }

    pub(super) fn handle_tab_move(&mut self, id: String, params: TabMoveParams) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&params.tab_id) else {
            return tab_not_found(id, &params.tab_id);
        };
        let Some(ws) = self.state.workspaces.get(ws_idx) else {
            return tab_not_found(id, &params.tab_id);
        };
        if params.insert_index > ws.tabs.len() {
            return encode_error(
                id,
                "tab_move_failed",
                format!("insert_index {} is out of bounds", params.insert_index),
            );
        }

        let tab_id = self
            .public_tab_id(ws_idx, tab_idx)
            .unwrap_or_else(|| crate::workspace::public_tab_id_for_number(&ws.id, tab_idx + 1));
        let workspace_id = self.public_workspace_id(ws_idx);
        let insert_index = params.insert_index;
        let moved = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .is_some_and(|ws| ws.move_tab(tab_idx, insert_index));
        let tabs = self.tab_list_info(ws_idx);
        if moved {
            self.schedule_session_save();
            if self.state.active == Some(ws_idx) {
                self.state.tab_scroll_follow_active = true;
                self.state.refresh_tab_bar_view();
            }
            self.emit_event(EventEnvelope {
                event: EventKind::TabMoved,
                data: EventData::TabMoved {
                    tab_id,
                    workspace_id,
                    insert_index,
                    tabs: tabs.clone(),
                },
            });
        }

        encode_success(id, ResponseResult::TabList { tabs })
    }

    /// Move a whole tab, layout and all, into another workspace, creating the
    /// workspace when no workspace carries the requested label. The tab and its
    /// panes get new public ids there; old pane ids keep resolving through the
    /// alias table so processes started under them can still address their pane.
    pub(super) fn handle_tab_move_to_workspace(
        &mut self,
        id: String,
        params: TabMoveToWorkspaceParams,
    ) -> String {
        let source = if let Some(tab_id) = &params.tab_id {
            let Some(source) = self.parse_tab_id(tab_id) else {
                return tab_not_found(id, tab_id);
            };
            source
        } else if let Some(pane_id) = &params.pane_id {
            let Some((ws_idx, pane_id_raw)) = self.parse_pane_id(pane_id) else {
                return encode_error(id, "pane_not_found", format!("pane {pane_id} not found"));
            };
            let Some(tab_idx) = self.state.workspaces[ws_idx].find_tab_index_for_pane(pane_id_raw)
            else {
                return encode_error(id, "pane_not_found", format!("pane {pane_id} not found"));
            };
            (ws_idx, tab_idx)
        } else if let Some(ws_idx) = self.state.active {
            (ws_idx, self.state.workspaces[ws_idx].active_tab)
        } else {
            return encode_error(id, "tab_not_found", "no focused tab");
        };
        let (source_ws_idx, source_tab_idx) = source;
        let target = match &params.workspace {
            Some(workspace) => workspace.trim().to_string(),
            None => self.auto_workspace_label(source_ws_idx, source_tab_idx),
        };
        let target = target.as_str();
        if target.is_empty() {
            return encode_error(id, "invalid_params", "workspace must not be empty");
        }

        let target_ws_idx = self
            .state
            .workspaces
            .iter()
            .position(|ws| ws.id == target)
            .or_else(|| {
                self.state.workspaces.iter().position(|ws| {
                    ws.display_name_from(&self.state.terminals, &self.terminal_runtimes)
                        .eq_ignore_ascii_case(target)
                })
            });
        if target_ws_idx == Some(source_ws_idx) {
            let Some(tab) = self.tab_info(source_ws_idx, source_tab_idx) else {
                return tab_not_found(id, target);
            };
            return encode_success(id, ResponseResult::TabInfo { tab });
        }

        let source_ws = &self.state.workspaces[source_ws_idx];
        let previous_workspace_id = source_ws.id.clone();
        let Some(previous_tab_id) = self.public_tab_id(source_ws_idx, source_tab_idx) else {
            return tab_not_found(id, &previous_workspace_id);
        };
        let previous_row = crate::app::state::PaneSectionRef {
            workspace_id: previous_workspace_id.clone(),
            tab_number: source_ws.tabs[source_tab_idx].number,
        };
        let previous_pane_ids: Vec<_> = source_ws.tabs[source_tab_idx]
            .layout
            .pane_ids()
            .into_iter()
            .filter_map(|pane_id| Some((self.public_pane_id(source_ws_idx, pane_id)?, pane_id)))
            .collect();
        let was_focused =
            self.state.active == Some(source_ws_idx) && source_ws.active_tab == source_tab_idx;

        let Some(tab) = self.state.workspaces[source_ws_idx].take_tab_for_move(source_tab_idx)
        else {
            return tab_not_found(id, &previous_tab_id);
        };
        let created_workspace = target_ws_idx.is_none();
        let (mut target_ws_idx, target_tab_idx) = match target_ws_idx {
            Some(ws_idx) => (ws_idx, self.state.workspaces[ws_idx].insert_moved_tab(tab)),
            None => {
                let identity_cwd = tab
                    .terminal_id(tab.root_pane)
                    .and_then(|terminal_id| self.state.terminals.get(terminal_id))
                    .map(|terminal| terminal.cwd.clone())
                    .unwrap_or_else(|| self.state.workspaces[source_ws_idx].identity_cwd.clone());
                self.state
                    .workspaces
                    .push(crate::workspace::Workspace::from_moved_tab(
                        Some(target.to_string()),
                        identity_cwd,
                        tab,
                    ));
                (self.state.workspaces.len() - 1, 0)
            }
        };
        for (public_id, pane_id) in previous_pane_ids {
            self.state.public_pane_id_aliases.insert(public_id, pane_id);
        }

        let source_closed = self.state.workspaces[source_ws_idx].tabs.is_empty();
        if source_closed {
            self.state.remove_empty_workspace(source_ws_idx);
            if target_ws_idx > source_ws_idx {
                target_ws_idx -= 1;
            }
        }

        let target_ws = &self.state.workspaces[target_ws_idx];
        let new_row = crate::app::state::PaneSectionRef {
            workspace_id: target_ws.id.clone(),
            tab_number: target_ws.tabs[target_tab_idx].number,
        };
        self.state.rekey_pane_section_row(&previous_row, new_row);
        if was_focused {
            self.state
                .switch_workspace_tab(target_ws_idx, target_tab_idx);
        } else if !source_closed && self.state.active == Some(source_ws_idx) {
            self.state.refresh_tab_bar_view();
        }
        self.state.mark_session_dirty();
        self.schedule_session_save();

        self.emit_event(EventEnvelope {
            event: EventKind::TabClosed,
            data: EventData::TabClosed {
                tab_id: previous_tab_id,
                workspace_id: previous_workspace_id.clone(),
            },
        });
        if source_closed {
            self.emit_event(EventEnvelope {
                event: EventKind::WorkspaceClosed,
                data: EventData::WorkspaceClosed {
                    workspace_id: previous_workspace_id,
                    workspace: None,
                },
            });
        }
        if created_workspace {
            let workspace = self.workspace_info(target_ws_idx);
            self.emit_event(EventEnvelope {
                event: EventKind::WorkspaceCreated,
                data: EventData::WorkspaceCreated { workspace },
            });
        }
        self.emit_tab_created_events(target_ws_idx, target_tab_idx);
        let Some(tab) = self.tab_info(target_ws_idx, target_tab_idx) else {
            return encode_error(id, "tab_move_failed", "moved tab is unavailable");
        };

        encode_success(id, ResponseResult::TabInfo { tab })
    }

    /// The workspace label a tab belongs in by where its focused pane is: the
    /// main repository's folder name in a git checkout, so every worktree of a
    /// repo lands in one workspace, else the folder name.
    fn auto_workspace_label(&self, ws_idx: usize, tab_idx: usize) -> String {
        let Some(cwd) = self.state.workspaces[ws_idx]
            .tabs
            .get(tab_idx)
            .and_then(|tab| self.launch_cwd_for_pane_in_workspace(ws_idx, tab.layout.focused()))
        else {
            return String::new();
        };
        if let Some(space) = crate::workspace::git_space_metadata(&cwd) {
            return space.repo_name;
        }
        cwd.file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| cwd.display().to_string())
    }

    pub(super) fn handle_tab_close(&mut self, id: String, target: TabTarget) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&target.tab_id) else {
            return tab_not_found(id, &target.tab_id);
        };
        let Some(tab_id) = self.public_tab_id(ws_idx, tab_idx) else {
            return tab_not_found(id, &target.tab_id);
        };
        let workspace_id = self.public_workspace_id(ws_idx);
        let Some(ws) = self.state.workspaces.get(ws_idx) else {
            return tab_not_found(id, &target.tab_id);
        };
        let closes_workspace = ws.tabs.len() <= 1;
        let terminal_ids = self.state.terminal_ids_for_tab(ws_idx, tab_idx);
        let pane_ids = ws
            .tabs
            .get(tab_idx)
            .map(|tab| tab.layout.pane_ids())
            .unwrap_or_default();

        if closes_workspace {
            if self.state.confirm_implicit_worktree_group_close(ws_idx) {
                return encode_error(
                    id,
                    "confirmation_required",
                    "closing this tab would close a worktree group",
                );
            }
            let workspace = self.workspace_info(ws_idx);
            self.state.selected = ws_idx;
            self.state.close_selected_workspace();
            self.state.remove_plugin_pane_records(pane_ids);
            self.shutdown_detached_terminal_runtimes();
            self.emit_event(EventEnvelope {
                event: EventKind::TabClosed,
                data: EventData::TabClosed {
                    tab_id,
                    workspace_id: workspace_id.clone(),
                },
            });
            self.emit_event(EventEnvelope {
                event: EventKind::WorkspaceClosed,
                data: EventData::WorkspaceClosed {
                    workspace_id,
                    workspace: Some(workspace),
                },
            });
            return encode_success(id, ResponseResult::Ok {});
        }

        self.state.capture_closed_tab(ws_idx, tab_idx);
        let Some(ws) = self.state.workspaces.get_mut(ws_idx) else {
            return tab_not_found(id, &target.tab_id);
        };
        if !ws.close_tab(tab_idx) {
            return encode_error(
                id,
                "tab_close_failed",
                format!("tab {} could not be closed", target.tab_id),
            );
        }
        self.state.remove_plugin_pane_records(pane_ids);
        self.state.remove_unattached_terminal_ids(terminal_ids);
        self.shutdown_detached_terminal_runtimes();
        self.schedule_session_save();
        self.emit_event(EventEnvelope {
            event: EventKind::TabClosed,
            data: EventData::TabClosed {
                tab_id,
                workspace_id,
            },
        });

        encode_success(id, ResponseResult::Ok {})
    }

    fn tab_list_info(&self, ws_idx: usize) -> Vec<crate::api::schema::TabInfo> {
        self.state
            .workspaces
            .get(ws_idx)
            .map(|ws| {
                (0..ws.tabs.len())
                    .filter_map(|idx| self.tab_info(ws_idx, idx))
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn workspace_not_found(id: String, workspace_id: &str) -> String {
    encode_error(
        id,
        "workspace_not_found",
        format!("workspace {workspace_id} not found"),
    )
}

fn tab_not_found(id: String, tab_id: &str) -> String {
    encode_error(id, "tab_not_found", format!("tab {tab_id} not found"))
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{exiting_test_command, shutdown_test_runtimes};
    use super::*;
    use crate::{
        api::schema::SuccessResponse,
        config::{Config, ShellModeConfig},
        workspace::Workspace,
    };

    #[test]
    fn api_tab_close_last_tab_closes_workspace_and_emits_both_events() {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(&Config::default(), true, None, api_rx, event_hub.clone());
        app.state.workspaces = vec![Workspace::test_new("tabs")];
        app.state.active = Some(0);
        app.state.selected = 0;
        let tab_id = app.public_tab_id(0, 0).unwrap();
        let workspace_id = app.public_workspace_id(0);

        let response = app.handle_tab_close(
            "req".into(),
            TabTarget {
                tab_id: tab_id.clone(),
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(success.result, ResponseResult::Ok {});
        assert!(app.state.workspaces.is_empty());
        assert!(app.state.active.is_none());
        let events = event_hub.events_after(0);
        assert_eq!(
            events
                .iter()
                .map(|(_, event)| event.event)
                .collect::<Vec<_>>(),
            [EventKind::TabClosed, EventKind::WorkspaceClosed]
        );
        assert!(matches!(
            &events[0].1.data,
            EventData::TabClosed {
                tab_id: closed_tab_id,
                workspace_id: closed_workspace_id,
            } if closed_tab_id == &tab_id && closed_workspace_id == &workspace_id
        ));
        assert!(matches!(
            &events[1].1.data,
            EventData::WorkspaceClosed {
                workspace_id: closed_workspace_id,
                workspace: Some(workspace),
            } if closed_workspace_id == &workspace_id
                && workspace.workspace_id == workspace_id
        ));
    }

    #[test]
    fn api_tab_move_to_workspace_joins_existing_space_by_label() {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(&Config::default(), true, None, api_rx, event_hub.clone());
        let mut alpha = Workspace::test_new("alpha");
        alpha.test_add_tab(Some("mover"));
        alpha.switch_tab(1);
        app.state.workspaces = vec![alpha, Workspace::test_new("Work")];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.reconcile_pane_section_order();
        let moved_root = app.state.workspaces[0].tabs[1].root_pane;
        let old_pane_id = app.public_pane_id(0, moved_root).unwrap();
        let old_row = app
            .state
            .pane_section_index_of(&app.public_workspace_id(0), 2)
            .unwrap();

        let response = app.handle_tab_move_to_workspace(
            "req".into(),
            TabMoveToWorkspaceParams {
                tab_id: Some(app.public_tab_id(0, 1).unwrap()),
                pane_id: None,
                workspace: Some("work".into()),
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::TabInfo { tab } = success.result else {
            panic!("expected tab info");
        };
        assert_eq!(app.state.workspaces[0].tabs.len(), 1);
        assert_eq!(app.state.workspaces[1].tabs.len(), 2);
        assert_eq!(app.state.workspaces[1].tabs[1].root_pane, moved_root);
        assert_eq!(tab.tab_id, app.public_tab_id(1, 1).unwrap());
        assert_eq!(app.state.active, Some(1), "the focused tab is followed");
        assert_eq!(app.state.workspaces[1].active_tab, 1);
        assert_eq!(app.parse_pane_id(&old_pane_id), Some((1, moved_root)));
        let new_number = app.state.workspaces[1].tabs[1].number;
        assert_eq!(
            app.state
                .pane_section_index_of(&app.public_workspace_id(1), new_number),
            Some(old_row),
            "the band row keeps its slot"
        );
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn api_tab_move_to_workspace_creates_missing_space_and_closes_empty_source() {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(&Config::default(), true, None, api_rx, event_hub.clone());
        app.state.workspaces = vec![Workspace::test_new("keep"), Workspace::test_new("solo")];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        let moved_root = app.state.workspaces[1].tabs[0].root_pane;
        let solo_id = app.public_workspace_id(1);

        let response = app.handle_tab_move_to_workspace(
            "req".into(),
            TabMoveToWorkspaceParams {
                tab_id: Some(app.public_tab_id(1, 0).unwrap()),
                pane_id: None,
                workspace: Some("fresh".into()),
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        assert!(matches!(success.result, ResponseResult::TabInfo { .. }));
        assert_eq!(app.state.workspaces.len(), 2);
        assert_eq!(
            app.state.workspaces[1].custom_name.as_deref(),
            Some("fresh")
        );
        assert_eq!(app.state.workspaces[1].tabs[0].root_pane, moved_root);
        assert_eq!(app.state.active, Some(0), "an unfocused tab moves quietly");
        let kinds: Vec<_> = event_hub
            .events_after(0)
            .into_iter()
            .map(|(_, event)| event.event)
            .collect();
        assert!(kinds.contains(&EventKind::WorkspaceClosed));
        assert!(kinds.contains(&EventKind::WorkspaceCreated));
        assert!(app.parse_workspace_id(&solo_id).is_none());
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn api_tab_move_reorders_tabs_in_target_workspace() {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(&Config::default(), true, None, api_rx, event_hub.clone());
        let mut workspace = Workspace::test_new("tabs");
        workspace.test_add_tab(Some("two"));
        workspace.test_add_tab(Some("three"));
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.selected = 0;
        let moved_root = app.state.workspaces[0].tabs[0].root_pane;
        let moved_id = app.public_tab_id(0, 0).unwrap();

        let response = app.handle_tab_move(
            "req".into(),
            TabMoveParams {
                tab_id: moved_id.clone(),
                insert_index: 3,
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::TabList { tabs } = success.result else {
            panic!("expected tab list");
        };
        assert_eq!(app.state.workspaces[0].tabs[2].root_pane, moved_root);
        assert_eq!(tabs[2].tab_id, app.public_tab_id(0, 2).unwrap());
        let events = event_hub.events_after(0);
        assert!(events.iter().any(|(_, event)| {
            matches!(
                &event.data,
                EventData::TabMoved {
                    tab_id,
                    workspace_id,
                    insert_index: 3,
                    tabs,
                } if tab_id == &moved_id
                    && workspace_id == &app.public_workspace_id(0)
                    && tabs[2].tab_id == moved_id
            )
        }));
    }

    #[test]
    fn api_tab_rename_reflows_active_tab_bar() {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(&Config::default(), true, None, api_rx, event_hub);
        let workspace = Workspace::test_new("tabs");
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.view.tab_bar_rect = ratatui::layout::Rect::new(0, 0, 60, 1);
        app.state.refresh_tab_bar_view();

        let tab_id = app.public_tab_id(0, 0).unwrap();
        let width_before = app.state.view.tab_hit_areas[0].width;

        app.handle_tab_rename(
            "req".into(),
            TabRenameParams {
                tab_id,
                label: "a much longer custom tab label".into(),
            },
        );

        let width_after = app.state.view.tab_hit_areas[0].width;
        assert!(
            width_after > width_before,
            "tab bar should reflow to the new label width immediately: \
             before={width_before}, after={width_after}"
        );
    }

    #[tokio::test]
    async fn tab_create_index_places_the_tab_and_rejects_an_out_of_range_slot() {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(&Config::default(), true, None, api_rx, event_hub);
        app.state.default_shell = exiting_test_command().into();
        app.state.shell_mode = ShellModeConfig::NonLogin;
        let mut workspace = Workspace::test_new("tabs");
        workspace.test_add_tab(Some("two"));
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.ensure_test_terminals();

        let response = app.handle_tab_create(
            "req".into(),
            TabCreateParams {
                workspace_id: None,
                cwd: None,
                focus: false,
                label: Some("wedged".into()),
                index: Some(1),
                env: Default::default(),
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::TabCreated { tab, .. } = success.result else {
            panic!("expected a created tab");
        };
        // The label and the reported index follow the tab to its requested slot.
        assert_eq!(tab.index, 1);
        assert_eq!(tab.label, "wedged");
        // The slot moved the row, not the tab: inside its space the tab is still
        // appended, so it is last there while its row sits at slot 1.
        assert_eq!(app.state.workspaces[0].tabs.len(), 3);
        assert_eq!(app.tab_info(0, 2).unwrap().tab_id, tab.tab_id);

        // Past the end of the band is an error, and creates nothing.
        let response = app.handle_tab_create(
            "req".into(),
            TabCreateParams {
                workspace_id: None,
                cwd: None,
                focus: false,
                label: None,
                index: Some(9),
                env: Default::default(),
            },
        );
        assert!(response.contains("tab_create_failed"), "{response}");
        assert_eq!(app.state.workspaces[0].tabs.len(), 3);
        shutdown_test_runtimes(&mut app);
    }

    #[tokio::test]
    async fn tab_create_index_places_the_row_and_leaves_the_space_order_alone() {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(&Config::default(), true, None, api_rx, event_hub);
        app.state.default_shell = exiting_test_command().into();
        app.state.shell_mode = ShellModeConfig::NonLogin;

        let mut first = Workspace::test_new("first");
        first.test_add_tab(Some("first-two"));
        let second = Workspace::test_new("second");
        app.state.workspaces = vec![first, second];
        // Create into the second space.
        app.state.active = Some(1);
        app.state.selected = 1;
        app.state.ensure_test_terminals();

        // The band spans both spaces: [first/1, first/2, second/1].
        app.state.reconcile_pane_section_order();
        assert_eq!(app.state.pane_section_order.order.len(), 3);

        let response = app.handle_tab_create(
            "req".into(),
            TabCreateParams {
                workspace_id: None,
                cwd: None,
                focus: false,
                label: Some("wedged".into()),
                index: Some(1),
                env: Default::default(),
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::TabCreated { tab, .. } = success.result else {
            panic!("expected a created tab");
        };

        // Slot 1 sits between the first space's two rows, and that is where the
        // new row goes even though the tab belongs to the second space.
        assert_eq!(tab.index, 1);
        assert_eq!(app.state.pane_section_order.order.len(), 4);

        // The space is left out of it entirely: the tab is appended to its own
        // space like any other, so it is last there, and the first space keeps
        // both of its tabs in place.
        assert_eq!(app.state.workspaces[1].tabs.len(), 2);
        assert_eq!(app.tab_info(1, 1).unwrap().tab_id, tab.tab_id);
        assert_eq!(app.tab_info(1, 1).unwrap().label, "wedged");
        assert_eq!(app.state.workspaces[0].tabs.len(), 2);

        // The rows of the first space are pushed apart by the new one rather
        // than regrouped around it.
        assert_eq!(app.tab_info(0, 0).unwrap().index, 0);
        assert_eq!(app.tab_info(0, 1).unwrap().index, 2);

        shutdown_test_runtimes(&mut app);
    }

    #[tokio::test]
    async fn tab_create_follows_cached_focused_pane_cwd_without_runtime() {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(&Config::default(), true, None, api_rx, event_hub);
        app.state.default_shell = exiting_test_command().into();
        app.state.shell_mode = ShellModeConfig::NonLogin;
        let workspace = Workspace::test_new("tabs");
        let focused_pane = workspace.tabs[0].root_pane;
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.ensure_test_terminals();
        let cached_cwd = std::env::temp_dir();
        let terminal_id = app.state.workspaces[0]
            .terminal_id(focused_pane)
            .cloned()
            .unwrap();
        app.state.terminals.get_mut(&terminal_id).unwrap().cwd = cached_cwd.clone();

        let response = app.handle_tab_create(
            "req".into(),
            TabCreateParams {
                workspace_id: None,
                cwd: None,
                focus: false,
                label: None,
                index: None,
                env: Default::default(),
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        assert!(matches!(success.result, ResponseResult::TabCreated { .. }));
        let created = &app.state.workspaces[0].tabs[1];
        let created_terminal_id = created.terminal_id(created.root_pane).unwrap();
        let created_cwd = &app.state.terminals.get(created_terminal_id).unwrap().cwd;
        assert_eq!(
            crate::worktree::canonical_or_original(created_cwd),
            crate::worktree::canonical_or_original(&cached_cwd)
        );
        shutdown_test_runtimes(&mut app);
    }
}
