//! Pure state mutations on AppState.
//! These don't need channels, async, or PTY runtime.

use tracing::{info, warn};

use crate::detect::{Agent, AgentState};
use crate::events::AppEvent;
use crate::layout::{find_in_direction, NavDirection, PaneId};
use crate::selection::Selection;
use crate::terminal::{EffectiveStateChange, TerminalStateMutation};
use crate::workspace::WorkspaceGitStatus;
use unicode_width::UnicodeWidthChar;

use super::state::{
    text_matches_query, AgentNotificationDelivery, AppState, Mode, NavigatorRow,
    NavigatorStateFilter, NavigatorTarget, PaneFocusTarget, PendingAgentNotification, ToastKind,
    ToastNotification, ToastTarget, ViewLayout,
};

fn is_background_completion_transition(prev_state: AgentState, new_state: AgentState) -> bool {
    matches!(new_state, AgentState::Idle)
        && matches!(prev_state, AgentState::Working | AgentState::Blocked)
}

fn is_completion_transition(change: &EffectiveStateChange) -> bool {
    is_completion_transition_parts(
        change.previous_state,
        change.state,
        change.previous_agent_label.as_deref(),
        change.agent_label.as_deref(),
    )
}

fn public_tab_id_for_index(ws: &crate::workspace::Workspace, tab_idx: usize) -> Option<String> {
    let tab_number = ws.public_tab_number(tab_idx)?;
    Some(crate::workspace::public_tab_id_for_number(
        &ws.id, tab_number,
    ))
}

pub fn is_completion_transition_parts(
    previous_state: AgentState,
    state: AgentState,
    previous_agent_label: Option<&str>,
    agent_label: Option<&str>,
) -> bool {
    is_background_completion_transition(previous_state, state)
        || (previous_state == AgentState::Unknown
            && state == AgentState::Idle
            && previous_agent_label.is_some()
            && previous_agent_label == agent_label)
}

pub fn active_tab_suppresses_notifications(
    is_active_tab: bool,
    outer_terminal_focus: Option<bool>,
) -> bool {
    is_active_tab && outer_terminal_focus != Some(false)
}

#[cfg(test)]
pub fn notification_sound_for_state_change(
    suppress_active_tab_notifications: bool,
    prev_state: AgentState,
    new_state: AgentState,
) -> Option<crate::sound::Sound> {
    if new_state == prev_state {
        return None;
    }

    match new_state {
        AgentState::Blocked => Some(crate::sound::Sound::Request),
        AgentState::Idle
            if is_background_completion_transition(prev_state, new_state)
                && !suppress_active_tab_notifications =>
        {
            Some(crate::sound::Sound::Done)
        }
        _ => None,
    }
}

pub fn notification_sound_for_state_change_with_agent_labels(
    suppress_active_tab_notifications: bool,
    prev_state: AgentState,
    new_state: AgentState,
    previous_agent_label: Option<&str>,
    agent_label: Option<&str>,
) -> Option<crate::sound::Sound> {
    if new_state == prev_state {
        return None;
    }

    match new_state {
        AgentState::Blocked => Some(crate::sound::Sound::Request),
        AgentState::Idle
            if is_completion_transition_parts(
                prev_state,
                new_state,
                previous_agent_label,
                agent_label,
            ) && !suppress_active_tab_notifications =>
        {
            Some(crate::sound::Sound::Done)
        }
        _ => None,
    }
}

fn notification_sound_for_effective_state_change(
    suppress_active_tab_notifications: bool,
    change: &EffectiveStateChange,
) -> Option<crate::sound::Sound> {
    if change.state == change.previous_state {
        return None;
    }

    match change.state {
        AgentState::Blocked => Some(crate::sound::Sound::Request),
        AgentState::Idle
            if is_completion_transition(change) && !suppress_active_tab_notifications =>
        {
            Some(crate::sound::Sound::Done)
        }
        _ => None,
    }
}

pub fn notification_toast_for_state_change_with_agent_labels(
    suppress_active_tab_notifications: bool,
    prev_state: AgentState,
    new_state: AgentState,
    previous_agent_label: Option<&str>,
    agent_label: Option<&str>,
) -> Option<ToastKind> {
    if suppress_active_tab_notifications || new_state == prev_state {
        return None;
    }

    match new_state {
        AgentState::Blocked => Some(ToastKind::NeedsAttention),
        AgentState::Idle
            if is_completion_transition_parts(
                prev_state,
                new_state,
                previous_agent_label,
                agent_label,
            ) =>
        {
            Some(ToastKind::Finished)
        }
        _ => None,
    }
}

fn notification_toast_for_effective_state_change(
    suppress_active_tab_notifications: bool,
    change: &EffectiveStateChange,
) -> Option<ToastKind> {
    if suppress_active_tab_notifications || change.state == change.previous_state {
        return None;
    }

    match change.state {
        AgentState::Blocked => Some(ToastKind::NeedsAttention),
        AgentState::Idle if is_completion_transition(change) => Some(ToastKind::Finished),
        _ => None,
    }
}

pub fn notification_toast_for_pane_state_update(
    suppress_active_tab_notifications: bool,
    update: &PaneStateUpdate,
) -> Option<ToastKind> {
    if suppress_active_tab_notifications || update.state == update.previous_state {
        return None;
    }

    notification_toast_for_state_change_with_agent_labels(
        suppress_active_tab_notifications,
        update.previous_state,
        update.state,
        update.previous_agent_label.as_deref(),
        update.agent_label.as_deref(),
    )
}

fn toast_agent_label(agent_label: &str) -> &str {
    agent_label
}

fn toast_event_text(kind: ToastKind) -> &'static str {
    match kind {
        ToastKind::NeedsAttention => "needs attention",
        ToastKind::Finished => "finished",
        ToastKind::UpdateInstalled => "updated",
    }
}

fn sound_for_toast_kind(
    kind: ToastKind,
    suppress_active_tab_notifications: bool,
) -> Option<crate::sound::Sound> {
    match kind {
        ToastKind::NeedsAttention => Some(crate::sound::Sound::Request),
        ToastKind::Finished if !suppress_active_tab_notifications => {
            Some(crate::sound::Sound::Done)
        }
        ToastKind::Finished | ToastKind::UpdateInstalled => None,
    }
}

pub fn notification_context(
    ws: &crate::workspace::Workspace,
    workspace_label: &str,
    ws_idx: usize,
    pane_id: PaneId,
) -> String {
    let mut context = format!("{} · {}", workspace_label, ws_idx + 1);
    if ws.tabs.len() > 1 {
        if let Some(tab_idx) = ws.find_tab_index_for_pane(pane_id) {
            if let Some(label) = ws.tab_display_name(tab_idx) {
                context.push_str(&format!(" · {label}"));
            }
        }
    }
    context
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneStateUpdate {
    pub pane_id: PaneId,
    pub ws_idx: usize,
    pub previous_agent_label: Option<String>,
    pub previous_known_agent: Option<Agent>,
    pub previous_state: AgentState,
    pub previous_seen: bool,
    pub previous_presentation: crate::terminal::EffectivePresentation,
    pub agent_label: Option<String>,
    pub known_agent: Option<Agent>,
    pub state: AgentState,
    pub seen: bool,
    pub presentation: crate::terminal::EffectivePresentation,
}

// ---------------------------------------------------------------------------
// Navigator operations
// ---------------------------------------------------------------------------

/// Resolve the index to focus when cycling a sidebar section.
///
/// `ids` is the section's entries in display order, `focused` the currently
/// focused pane, `remembered` the last pane selected within this section, and
/// `forward` the cycle direction. When the current focus is within the section,
/// cycle normally from it. When focus is outside the section, jump to the
/// remembered entry if it is still present; otherwise fall back to the direction
/// default (first entry forward, last entry backward). Returns `None` only when
/// the section is empty. Shared by the AppState and live App cycle paths so the
/// cross-section jump stays identical.
pub(crate) fn section_cycle_target_index(
    ids: &[PaneId],
    focused: Option<PaneId>,
    remembered: Option<PaneId>,
    forward: bool,
) -> Option<usize> {
    if ids.is_empty() {
        return None;
    }
    if let Some(current_idx) = focused.and_then(|pane_id| ids.iter().position(|id| *id == pane_id))
    {
        let target = if forward {
            (current_idx + 1) % ids.len()
        } else if current_idx == 0 {
            ids.len() - 1
        } else {
            current_idx - 1
        };
        return Some(target);
    }
    if let Some(remembered) = remembered {
        if let Some(idx) = ids.iter().position(|id| *id == remembered) {
            return Some(idx);
        }
    }
    Some(if forward { 0 } else { ids.len() - 1 })
}

impl AppState {
    pub(crate) fn current_pane_focus_target(&self) -> Option<PaneFocusTarget> {
        let ws_idx = self.active?;
        let ws = self.workspaces.get(ws_idx)?;
        let pane_id = ws.focused_pane_id()?;
        Some(PaneFocusTarget {
            workspace_id: ws.id.clone(),
            pane_id,
        })
    }

    pub(crate) fn pane_focus_target_indices(
        &self,
        target: &PaneFocusTarget,
    ) -> Option<(usize, usize)> {
        let ws_idx = self
            .workspaces
            .iter()
            .position(|ws| ws.id == target.workspace_id)?;
        let tab_idx = self.workspaces[ws_idx].find_tab_index_for_pane(target.pane_id)?;
        Some((ws_idx, tab_idx))
    }

    pub(crate) fn record_pane_focus_change(
        &mut self,
        previous: Option<PaneFocusTarget>,
        ws_idx: usize,
        pane_id: PaneId,
    ) {
        let Some(ws) = self.workspaces.get(ws_idx) else {
            return;
        };
        let target = PaneFocusTarget {
            workspace_id: ws.id.clone(),
            pane_id,
        };
        if previous.as_ref() != Some(&target) {
            self.previous_pane_focus = previous;
        }
    }

    fn record_pane_focus_after_navigation(&mut self, previous: Option<PaneFocusTarget>) {
        let current = self.current_pane_focus_target();
        if previous != current {
            self.previous_pane_focus = previous;
        }
    }

    pub(crate) fn focus_pane_in_workspace(&mut self, ws_idx: usize, pane_id: PaneId) -> bool {
        let Some(ws) = self.workspaces.get(ws_idx) else {
            return false;
        };
        let Some(tab_idx) = ws.find_tab_index_for_pane(pane_id) else {
            return false;
        };
        let previous = self.current_pane_focus_target();
        let target = PaneFocusTarget {
            workspace_id: ws.id.clone(),
            pane_id,
        };
        if previous.as_ref() == Some(&target) {
            return false;
        }

        self.switch_workspace_tab(ws_idx, tab_idx);
        if let Some(tab) = self
            .workspaces
            .get_mut(ws_idx)
            .and_then(|ws| ws.tabs.get_mut(tab_idx))
        {
            tab.layout.focus_pane(pane_id);
            self.previous_pane_focus = previous;
            self.record_section_focus_memory(pane_id);
            self.mark_session_dirty();
            return true;
        }
        false
    }

    /// Remember the last pane focused within each sidebar section so an
    /// agent-nav or pane-nav key can jump back to it when focus is currently
    /// outside that section. A pane in neither section list updates neither
    /// field. Client-only TUI presentation state.
    pub(crate) fn record_section_focus_memory(&mut self, pane_id: PaneId) {
        if crate::ui::agent_panel_entries(self)
            .iter()
            .any(|entry| entry.pane_id == pane_id)
        {
            self.last_agent_focus = Some(pane_id);
        } else if crate::ui::sidebar_pane_section_entries(self)
            .iter()
            .any(|entry| entry.pane_id == pane_id)
        {
            self.last_pane_section_focus = Some(pane_id);
        }
    }

    #[cfg(test)]
    pub(crate) fn open_navigator(&mut self) {
        let terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();
        self.open_navigator_from(&terminal_runtimes);
    }

    pub(crate) fn open_navigator_from(
        &mut self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    ) {
        self.navigator.query.clear();
        self.navigator.search_focused = true;
        self.navigator.state_filter = None;
        self.navigator.scroll = 0;
        self.navigator.expanded_workspaces.clear();

        for ws in &self.workspaces {
            self.navigator.expanded_workspaces.insert(ws.id.clone());
        }

        self.mode = Mode::Navigator;
        self.navigator.selected = self
            .current_navigator_row_index_from(terminal_runtimes)
            .unwrap_or(0);
        self.ensure_navigator_selection_visible_from(terminal_runtimes);
    }

    #[cfg(test)]
    pub(crate) fn navigator_rows(&self) -> Vec<NavigatorRow> {
        let terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();
        self.navigator_rows_from(&terminal_runtimes)
    }

    pub(crate) fn navigator_rows_from(
        &self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    ) -> Vec<NavigatorRow> {
        let query = self.navigator.query.trim().to_lowercase();
        let query_kind = navigator_query_kind(&query, self.navigator.state_filter);
        let mut rows = Vec::new();
        for (ws_idx, ws) in self.workspaces.iter().enumerate() {
            let workspace_label = ws.display_name_from(&self.terminals, terminal_runtimes);
            let activity = workspace_activity_summary(ws, &self.terminals);
            let workspace_search_text = format!("{workspace_label} {activity}").to_lowercase();
            let workspace_matches = match query_kind {
                NavigatorQueryKind::Empty => true,
                NavigatorQueryKind::State(filter) => {
                    let (state, seen) = ws.aggregate_state(&self.terminals);
                    navigator_state_filter_matches(filter, state, seen)
                }
                NavigatorQueryKind::Text => navigator_matches(&query, &workspace_search_text),
            };

            let child_rows = self.navigator_child_rows(ws_idx, query_kind, &query);
            if !workspace_matches && child_rows.is_empty() {
                continue;
            }

            let expanded = !matches!(query_kind, NavigatorQueryKind::Empty)
                || self.navigator.expanded_workspaces.contains(&ws.id);
            let (state, seen) = ws.aggregate_state(&self.terminals);
            let pane_count = ws.tabs.iter().map(|tab| tab.panes.len()).sum::<usize>();
            rows.push(NavigatorRow {
                target: NavigatorTarget::Workspace { ws_idx },
                depth: 0,
                label: format!("{workspace_label} ({pane_count})"),
                meta: activity,
                status: state,
                seen,
                is_current: self.active == Some(ws_idx),
                is_workspace: true,
                is_tab: false,
                expanded,
                search_text: workspace_search_text,
            });
            if expanded {
                rows.extend(child_rows);
            }
        }
        rows
    }

    fn navigator_child_rows(
        &self,
        ws_idx: usize,
        query_kind: NavigatorQueryKind,
        query: &str,
    ) -> Vec<NavigatorRow> {
        let Some(ws) = self.workspaces.get(ws_idx) else {
            return Vec::new();
        };
        let multi_tab = ws.tabs.len() > 1;
        let mut rows = Vec::new();
        for tab_idx in 0..ws.tabs.len() {
            let tab_row = multi_tab.then(|| self.navigator_tab_row(ws_idx, tab_idx));
            let tab_matches = tab_row.as_ref().is_some_and(|row| match query_kind {
                NavigatorQueryKind::Empty => true,
                NavigatorQueryKind::State(filter) => {
                    navigator_state_filter_matches(filter, row.status, row.seen)
                }
                NavigatorQueryKind::Text => navigator_matches(query, &row.search_text),
            });
            let pane_rows = self.navigator_pane_rows_for_tab(ws_idx, tab_idx, multi_tab);
            let filtered_panes = match query_kind {
                NavigatorQueryKind::Empty => pane_rows,
                NavigatorQueryKind::State(filter) => pane_rows
                    .into_iter()
                    .filter(|row| navigator_state_filter_matches(filter, row.status, row.seen))
                    .collect::<Vec<_>>(),
                NavigatorQueryKind::Text if tab_matches => pane_rows,
                NavigatorQueryKind::Text => pane_rows
                    .into_iter()
                    .filter(|row| navigator_matches(query, &row.search_text))
                    .collect::<Vec<_>>(),
            };

            if let Some(tab_row) = tab_row {
                if tab_matches || !filtered_panes.is_empty() {
                    rows.push(tab_row);
                }
            }
            rows.extend(filtered_panes);
        }
        rows
    }

    fn navigator_tab_row(&self, ws_idx: usize, tab_idx: usize) -> NavigatorRow {
        let ws = &self.workspaces[ws_idx];
        let tab = &ws.tabs[tab_idx];
        let label = ws
            .tab_display_name(tab_idx)
            .unwrap_or_else(|| (tab_idx + 1).to_string());
        let (status, seen) = tab_aggregate_state(tab, &self.terminals);
        let activity = tab_activity_summary(tab, &self.terminals);
        let pane_count = tab.panes.len();
        let meta = if activity.is_empty() {
            format!("{pane_count} panes")
        } else {
            format!("{pane_count} panes · {activity}")
        };
        let search_text = format!("{label} {meta}").to_lowercase();
        NavigatorRow {
            target: NavigatorTarget::Tab { ws_idx, tab_idx },
            depth: 1,
            label,
            meta,
            status,
            seen,
            is_current: false,
            is_workspace: false,
            is_tab: true,
            expanded: true,
            search_text,
        }
    }

    fn navigator_pane_rows_for_tab(
        &self,
        ws_idx: usize,
        tab_idx: usize,
        multi_tab: bool,
    ) -> Vec<NavigatorRow> {
        let Some(ws) = self.workspaces.get(ws_idx) else {
            return Vec::new();
        };
        let Some(tab) = ws.tabs.get(tab_idx) else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        for pane_id in tab.layout.pane_ids() {
            let Some(pane) = tab.panes.get(&pane_id) else {
                continue;
            };
            let terminal = self.terminals.get(&pane.attached_terminal_id);
            let pane_number = ws.public_pane_number(pane_id).unwrap_or(0);
            let label = terminal
                .and_then(|terminal| terminal.effective_title())
                .or_else(|| {
                    terminal
                        .and_then(|terminal| terminal.manual_label.as_deref().map(str::to_string))
                })
                .or_else(|| {
                    terminal.and_then(|terminal| terminal.agent_name.as_deref().map(str::to_string))
                })
                .or_else(|| {
                    terminal
                        .and_then(|terminal| terminal.effective_agent_label().map(str::to_string))
                })
                .or_else(|| {
                    launch_label(terminal.and_then(|terminal| terminal.launch_argv.as_ref()))
                })
                .unwrap_or_else(|| format!("pane {pane_number}"));
            let display_agent = terminal.and_then(|terminal| terminal.effective_display_agent());
            let agent_label = display_agent.as_deref().or_else(|| {
                terminal
                    .and_then(|terminal| terminal.agent_name.as_deref())
                    .or_else(|| terminal.and_then(|terminal| terminal.effective_agent_label()))
            });
            let custom_status = terminal.and_then(|terminal| terminal.effective_custom_status());
            let state = terminal
                .map(|terminal| terminal.state)
                .unwrap_or(AgentState::Unknown);
            let status_label = terminal
                .map(|terminal| terminal.effective_presentation().state_labels)
                .and_then(|labels| labels.get(state_label_text(state, pane.seen)).cloned());
            let status = custom_status
                .or(status_label)
                .or_else(|| agent_label.map(|_| state_label_text(state, pane.seen).to_string()));
            let meta = match (agent_label, status.as_deref()) {
                (Some(agent_label), Some(status)) => format!("{agent_label} · {status}"),
                (Some(agent_label), None) => agent_label.to_string(),
                (None, _) => "shell".to_string(),
            };
            let is_current = self.is_active_pane(ws_idx, tab_idx, pane_id);
            let search_text = format!("{label} {meta}").to_lowercase();
            rows.push(NavigatorRow {
                target: NavigatorTarget::Pane {
                    ws_idx,
                    tab_idx,
                    pane_id,
                },
                depth: if multi_tab { 2 } else { 1 },
                label,
                meta,
                status: state,
                seen: pane.seen,
                is_current,
                is_workspace: false,
                is_tab: false,
                expanded: false,
                search_text,
            });
        }
        rows
    }

    fn current_navigator_row_index_from(
        &self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    ) -> Option<usize> {
        let rows = self.navigator_rows_from(terminal_runtimes);
        rows.iter()
            .position(|row| matches!(row.target, NavigatorTarget::Pane { .. }) && row.is_current)
            .or_else(|| rows.iter().position(|row| row.is_current))
    }

    pub(crate) fn ensure_navigator_selection_visible_from(
        &mut self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    ) {
        let body = self.navigator_body_rect();
        let viewport = body.height as usize;
        if viewport == 0 {
            self.navigator.scroll = 0;
            return;
        }
        let max_scroll = self.navigator_max_scroll_from(terminal_runtimes, viewport);
        if self.navigator.selected < self.navigator.scroll {
            self.navigator.scroll = self.navigator.selected;
        } else if self.navigator.selected >= self.navigator.scroll.saturating_add(viewport) {
            self.navigator.scroll = self
                .navigator
                .selected
                .saturating_add(1)
                .saturating_sub(viewport);
        }
        self.navigator.scroll = self.navigator.scroll.min(max_scroll);
    }

    pub(crate) fn navigator_max_scroll_from(
        &self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
        viewport: usize,
    ) -> usize {
        if viewport == 0 {
            return 0;
        }
        self.navigator_rows_from(terminal_runtimes)
            .len()
            .saturating_sub(viewport)
    }

    pub(crate) fn move_navigator_selection_from(
        &mut self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
        delta: isize,
    ) {
        let count = self.navigator_rows_from(terminal_runtimes).len();
        if count == 0 {
            self.navigator.selected = 0;
            self.navigator.scroll = 0;
            return;
        }
        let current = self.navigator.selected.min(count - 1) as isize;
        self.navigator.selected = (current + delta).clamp(0, count as isize - 1) as usize;
        self.ensure_navigator_selection_visible_from(terminal_runtimes);
    }

    pub(crate) fn clamp_navigator_selection_from(
        &mut self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    ) {
        let count = self.navigator_rows_from(terminal_runtimes).len();
        self.navigator.selected = self.navigator.selected.min(count.saturating_sub(1));
        self.ensure_navigator_selection_visible_from(terminal_runtimes);
    }

    pub(crate) fn toggle_selected_navigator_workspace_from(
        &mut self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    ) {
        let Some(row) = self
            .navigator_rows_from(terminal_runtimes)
            .get(self.navigator.selected)
            .cloned()
        else {
            return;
        };
        let NavigatorTarget::Workspace { ws_idx } = row.target else {
            return;
        };
        let Some(workspace_id) = self.workspaces.get(ws_idx).map(|ws| ws.id.clone()) else {
            return;
        };
        if self.navigator.expanded_workspaces.contains(&workspace_id) {
            self.navigator.expanded_workspaces.remove(&workspace_id);
        } else {
            self.navigator.expanded_workspaces.insert(workspace_id);
        }
        self.clamp_navigator_selection_from(terminal_runtimes);
    }

    #[cfg(test)]
    pub(crate) fn accept_navigator_selection(&mut self) -> bool {
        let terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();
        self.accept_navigator_selection_from(&terminal_runtimes)
    }

    pub(crate) fn accept_navigator_selection_from(
        &mut self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    ) -> bool {
        let Some(row) = self
            .navigator_rows_from(terminal_runtimes)
            .get(self.navigator.selected)
            .cloned()
        else {
            return false;
        };
        self.focus_navigator_target(row.target)
    }

    pub(crate) fn focus_navigator_target(&mut self, target: NavigatorTarget) -> bool {
        match target {
            NavigatorTarget::Workspace { ws_idx } => {
                if ws_idx >= self.workspaces.len() {
                    return false;
                }
                self.switch_workspace(ws_idx);
                self.mode = Mode::Terminal;
                true
            }
            NavigatorTarget::Tab { ws_idx, tab_idx } => {
                if ws_idx >= self.workspaces.len() {
                    return false;
                }
                let tab_exists = self
                    .workspaces
                    .get(ws_idx)
                    .is_some_and(|ws| tab_idx < ws.tabs.len());
                if !tab_exists {
                    return false;
                }
                self.switch_workspace_tab_sticky(ws_idx, tab_idx);
                self.mode = Mode::Terminal;
                true
            }
            NavigatorTarget::Pane {
                ws_idx,
                tab_idx,
                pane_id,
            } => {
                if ws_idx >= self.workspaces.len() {
                    return false;
                }
                if self
                    .workspaces
                    .get(ws_idx)
                    .and_then(|ws| ws.tabs.get(tab_idx))
                    .is_some_and(|tab| tab.panes.contains_key(&pane_id))
                {
                    self.focus_pane_in_workspace(ws_idx, pane_id);
                    self.mode = Mode::Terminal;
                    return true;
                }
                false
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavigatorQueryKind {
    Empty,
    Text,
    State(NavigatorStateFilter),
}

fn navigator_query_kind(
    query: &str,
    state_filter: Option<NavigatorStateFilter>,
) -> NavigatorQueryKind {
    if let Some(filter) = state_filter {
        return NavigatorQueryKind::State(filter);
    }
    if query.is_empty() {
        NavigatorQueryKind::Empty
    } else {
        NavigatorQueryKind::Text
    }
}

fn navigator_state_filter_matches(
    filter: NavigatorStateFilter,
    state: AgentState,
    seen: bool,
) -> bool {
    match filter {
        NavigatorStateFilter::Blocked => state == AgentState::Blocked,
        NavigatorStateFilter::Working => state == AgentState::Working,
        NavigatorStateFilter::Idle => state == AgentState::Idle && seen,
        NavigatorStateFilter::Done => state == AgentState::Idle && !seen,
    }
}

fn navigator_matches(query: &str, text: &str) -> bool {
    text_matches_query(query, text)
}

fn launch_label(argv: Option<&Vec<String>>) -> Option<String> {
    let argv = argv?;
    let command = argv.first()?;
    std::path::Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .or_else(|| Some(command.clone()))
}

fn state_label_text(state: AgentState, seen: bool) -> &'static str {
    match (state, seen) {
        (AgentState::Blocked, _) => "blocked",
        (AgentState::Working, _) => "working",
        (AgentState::Idle, false) => "done",
        (AgentState::Idle, true) => "idle",
        (AgentState::Unknown, _) => "unknown",
    }
}

fn tab_aggregate_state(
    tab: &crate::workspace::Tab,
    terminals: &std::collections::HashMap<
        crate::terminal::TerminalId,
        crate::terminal::TerminalState,
    >,
) -> (AgentState, bool) {
    let mut aggregate = AgentState::Unknown;
    let mut seen = true;
    for pane in tab.panes.values() {
        let Some(terminal) = terminals.get(&pane.attached_terminal_id) else {
            continue;
        };
        if state_priority(terminal.state, pane.seen) > state_priority(aggregate, seen) {
            aggregate = terminal.state;
            seen = pane.seen;
        }
    }
    (aggregate, seen)
}

fn state_priority(state: AgentState, seen: bool) -> u8 {
    match (state, seen) {
        (AgentState::Blocked, _) => 5,
        (AgentState::Working, _) => 4,
        (AgentState::Idle, false) => 3,
        (AgentState::Idle, true) => 2,
        (AgentState::Unknown, _) => 1,
    }
}

fn tab_activity_summary(
    tab: &crate::workspace::Tab,
    terminals: &std::collections::HashMap<
        crate::terminal::TerminalId,
        crate::terminal::TerminalState,
    >,
) -> String {
    activity_summary_for_panes(tab.panes.values(), terminals)
}

fn workspace_activity_summary(
    ws: &crate::workspace::Workspace,
    terminals: &std::collections::HashMap<
        crate::terminal::TerminalId,
        crate::terminal::TerminalState,
    >,
) -> String {
    activity_summary_for_panes(ws.tabs.iter().flat_map(|tab| tab.panes.values()), terminals)
}

fn activity_summary_for_panes<'a>(
    panes: impl Iterator<Item = &'a crate::pane::PaneState>,
    terminals: &std::collections::HashMap<
        crate::terminal::TerminalId,
        crate::terminal::TerminalState,
    >,
) -> String {
    let mut blocked = 0usize;
    let mut working = 0usize;
    let mut done = 0usize;
    for pane in panes {
        let Some(terminal) = terminals.get(&pane.attached_terminal_id) else {
            continue;
        };
        match (terminal.state, pane.seen) {
            (AgentState::Blocked, _) => blocked += 1,
            (AgentState::Working, _) => working += 1,
            (AgentState::Idle, false) => done += 1,
            _ => {}
        }
    }

    let mut parts = Vec::new();
    if blocked > 0 {
        parts.push(format!("{blocked} blocked"));
    }
    if working > 0 {
        parts.push(format!("{working} working"));
    }
    if done > 0 {
        parts.push(format!("{done} done"));
    }
    parts.join(" · ")
}

// ---------------------------------------------------------------------------
// Workspace operations
// ---------------------------------------------------------------------------

impl AppState {
    pub(crate) fn next_agent_metadata_expiry(&self) -> Option<std::time::Instant> {
        self.terminals
            .values()
            .filter_map(|terminal| terminal.next_agent_metadata_expiry())
            .min()
    }

    pub(crate) fn expire_agent_metadata_at(
        &mut self,
        scheduled_deadline: std::time::Instant,
        now: std::time::Instant,
    ) -> Vec<PaneStateUpdate> {
        let pane_terminals: Vec<_> = self
            .workspaces
            .iter()
            .enumerate()
            .flat_map(|(ws_idx, ws)| {
                ws.tabs.iter().flat_map(move |tab| {
                    tab.layout
                        .pane_ids()
                        .into_iter()
                        .filter_map(move |pane_id| {
                            ws.pane_state(pane_id)
                                .map(|pane| (ws_idx, pane_id, pane.attached_terminal_id.clone()))
                        })
                })
            })
            .collect();
        pane_terminals
            .into_iter()
            .filter_map(|(ws_idx, pane_id, terminal_id)| {
                let previous_seen = self.workspaces[ws_idx].pane_state(pane_id)?.seen;
                let mutation = self
                    .terminals
                    .get_mut(&terminal_id)?
                    .expire_agent_metadata_at(scheduled_deadline, now)?;
                let change = mutation.effective_state_change?;
                let seen = self.apply_pane_state_change(ws_idx, pane_id, &change)?;
                let update = PaneStateUpdate {
                    pane_id,
                    ws_idx,
                    previous_agent_label: change.previous_agent_label.clone(),
                    previous_known_agent: change.previous_known_agent,
                    previous_state: change.previous_state,
                    previous_seen,
                    previous_presentation: change.previous_presentation.clone(),
                    agent_label: change.agent_label.clone(),
                    known_agent: change.known_agent,
                    state: change.state,
                    seen,
                    presentation: change.presentation.clone(),
                };
                Some(update)
            })
            .collect()
    }

    pub(crate) fn pane_is_in_active_tab(&self, ws_idx: usize, pane_id: PaneId) -> bool {
        let Some(active_ws_idx) = self.active else {
            return false;
        };
        if active_ws_idx != ws_idx {
            return false;
        }
        self.workspaces[ws_idx]
            .find_tab_index_for_pane(pane_id)
            .is_some_and(|tab_idx| tab_idx == self.workspaces[ws_idx].active_tab)
    }

    pub fn switch_workspace(&mut self, idx: usize) {
        if idx < self.workspaces.len() {
            let previous_focus = self.current_pane_focus_target();
            self.selection = None;
            self.selection_autoscroll = None;
            self.active = Some(idx);
            self.selected = idx;
            let workspace_id = self.workspaces[idx].id.clone();
            crate::logging::workspace_focused(&workspace_id);
            self.mark_session_dirty();
            self.ensure_workspace_visible(idx);
            if let Some(ws) = self.workspaces.get_mut(idx) {
                // Clicking a space activates the workspace's latest active tab.
                let active_tab = ws.active_tab;
                ws.switch_tab(active_tab);
                let tab_id =
                    public_tab_id_for_index(ws, active_tab).unwrap_or_else(|| workspace_id.clone());
                crate::logging::tab_focused(&workspace_id, &tab_id);
            }
            self.tab_scroll_follow_active = true;
            self.refresh_tab_bar_view();
            self.record_pane_focus_after_navigation(previous_focus);
        }
    }

    pub(crate) fn switch_workspace_tab(&mut self, ws_idx: usize, tab_idx: usize) -> bool {
        if ws_idx >= self.workspaces.len() {
            return false;
        }
        if self
            .workspaces
            .get(ws_idx)
            .is_none_or(|ws| tab_idx >= ws.tabs.len())
        {
            return false;
        }

        let previous_focus = self.current_pane_focus_target();
        let workspace_changed = self.active != Some(ws_idx);
        self.selection = None;
        self.selection_autoscroll = None;
        self.active = Some(ws_idx);
        self.selected = ws_idx;
        let workspace_id = self.workspaces[ws_idx].id.clone();
        if workspace_changed {
            crate::logging::workspace_focused(&workspace_id);
        }
        self.mark_session_dirty();
        self.ensure_workspace_visible(ws_idx);
        if let Some(ws) = self.workspaces.get_mut(ws_idx) {
            ws.switch_tab(tab_idx);
            let tab_id =
                public_tab_id_for_index(ws, tab_idx).unwrap_or_else(|| workspace_id.clone());
            crate::logging::tab_focused(&workspace_id, &tab_id);
        }
        self.tab_scroll_follow_active = true;
        self.refresh_tab_bar_view();
        self.record_pane_focus_after_navigation(previous_focus);
        true
    }

    /// Like `switch_workspace_tab`, but records `tab_idx` as the workspace's home
    /// tab. Used by deliberate tab selections (tab bar click, new tab, navigator
    /// tab target) so the choice survives later transient agent-panel jumps.
    pub(crate) fn switch_workspace_tab_sticky(&mut self, ws_idx: usize, tab_idx: usize) -> bool {
        if !self.switch_workspace_tab(ws_idx, tab_idx) {
            return false;
        }
        if let Some(ws) = self.workspaces.get_mut(ws_idx) {
            ws.home_tab = tab_idx;
        }
        true
    }

    pub(crate) fn ensure_workspace_visible(&mut self, idx: usize) {
        if idx >= self.workspaces.len() {
            return;
        }

        if self.view.layout == ViewLayout::Mobile && self.mode == Mode::Navigate {
            self.ensure_mobile_workspace_visible(idx);
            return;
        }

        if self.sidebar_collapsed {
            return;
        }

        let entries = crate::ui::workspace_list_entries(self);
        let Some(target_entry_idx) = entries.iter().position(|entry| {
            matches!(
                entry,
                crate::ui::WorkspaceListEntry::Workspace { ws_idx, .. } if *ws_idx == idx
            )
        }) else {
            return;
        };

        self.workspace_scroll = crate::ui::normalized_workspace_scroll(
            self,
            self.view.sidebar_rect,
            self.workspace_scroll,
        );
        let mut cards = crate::ui::compute_workspace_card_areas(self, self.view.sidebar_rect);
        if cards.iter().any(|card| card.ws_idx == idx) {
            return;
        }

        if target_entry_idx < self.workspace_scroll {
            self.workspace_scroll = target_entry_idx;
            return;
        }

        while !cards.iter().any(|card| card.ws_idx == idx) {
            let previous_scroll = self.workspace_scroll;
            self.workspace_scroll = self.workspace_scroll.saturating_add(1);
            if self.workspace_scroll == previous_scroll {
                break;
            }
            self.workspace_scroll = crate::ui::normalized_workspace_scroll(
                self,
                self.view.sidebar_rect,
                self.workspace_scroll,
            );
            if self.workspace_scroll == previous_scroll {
                break;
            }
            cards = crate::ui::compute_workspace_card_areas(self, self.view.sidebar_rect);
            if cards.is_empty() {
                break;
            }
        }
    }

    fn ensure_mobile_workspace_visible(&mut self, idx: usize) {
        let viewport = crate::ui::mobile_switcher_areas(self).viewport;
        if viewport.height == 0 {
            return;
        }

        let row_range = crate::ui::mobile_switcher_workspace_doc_range(self, idx);
        let visible_start = self.mobile_switcher_scroll;
        let visible_end = visible_start.saturating_add(viewport.height as usize);
        if row_range.start < visible_start {
            self.mobile_switcher_scroll = row_range.start;
        } else if row_range.end > visible_end {
            self.mobile_switcher_scroll = row_range.end.saturating_sub(viewport.height as usize);
        }
        self.mobile_switcher_scroll = self
            .mobile_switcher_scroll
            .min(crate::ui::mobile_switcher_max_scroll(self));
    }

    pub fn switch_tab(&mut self, idx: usize) {
        if let Some(ws_idx) = self.active {
            let previous_focus = self.current_pane_focus_target();
            self.selection = None;
            self.selection_autoscroll = None;
            let Some(ws) = self.workspaces.get_mut(ws_idx) else {
                return;
            };
            ws.switch_tab_sticky(idx);
            let workspace_id = ws.id.clone();
            let tab_id = public_tab_id_for_index(ws, idx).unwrap_or_else(|| workspace_id.clone());
            crate::logging::tab_focused(&workspace_id, &tab_id);
            self.mark_session_dirty();
            self.tab_scroll_follow_active = true;
            self.refresh_tab_bar_view();
            self.record_pane_focus_after_navigation(previous_focus);
        }
    }

    pub(crate) fn mark_active_tab_seen(&mut self) -> bool {
        let Some(ws_idx) = self.active else {
            return false;
        };
        let Some(tab) = self
            .workspaces
            .get_mut(ws_idx)
            .and_then(crate::workspace::Workspace::active_tab_mut)
        else {
            return false;
        };

        let mut changed = false;
        for pane in tab.panes.values_mut() {
            if !pane.seen {
                pane.seen = true;
                changed = true;
            }
        }
        changed
    }

    pub(crate) fn visible_workspace_order(&self) -> Vec<usize> {
        // Mobile always shows the worktree tree expanded, so its visible order
        // must ignore collapse state to match what the switcher renders.
        let entries = if self.view.layout == ViewLayout::Mobile {
            crate::ui::workspace_list_entries_expanded(self)
        } else {
            crate::ui::workspace_list_entries(self)
        };
        let order = entries
            .into_iter()
            .map(|entry| match entry {
                crate::ui::WorkspaceListEntry::Workspace { ws_idx, .. } => ws_idx,
            })
            .collect::<Vec<_>>();
        if order.is_empty() {
            (0..self.workspaces.len()).collect()
        } else {
            order
        }
    }

    pub(crate) fn workspace_at_visible_position(&self, position: usize) -> Option<usize> {
        self.visible_workspace_order().get(position).copied()
    }

    pub(crate) fn move_selected_workspace_by_visible_delta(&mut self, delta: isize) {
        if self.workspaces.is_empty() {
            return;
        }
        let order = self.visible_workspace_order();
        let current_pos = order
            .iter()
            .position(|idx| *idx == self.selected)
            .unwrap_or(0);
        let target_pos = current_pos
            .saturating_add_signed(delta)
            .min(order.len().saturating_sub(1));
        if let Some(ws_idx) = order.get(target_pos).copied() {
            self.selected = ws_idx;
            self.ensure_workspace_visible(ws_idx);
        }
    }

    pub fn next_workspace(&mut self) {
        if self.workspaces.is_empty() {
            return;
        }
        let current = self.active.unwrap_or(self.selected);
        let order = self.visible_workspace_order();
        let current_pos = order.iter().position(|idx| *idx == current).unwrap_or(0);
        let next = order[(current_pos + 1) % order.len()];
        self.switch_workspace(next);
    }

    pub fn previous_workspace(&mut self) {
        if self.workspaces.is_empty() {
            return;
        }
        let current = self.active.unwrap_or(self.selected);
        let order = self.visible_workspace_order();
        let current_pos = order.iter().position(|idx| *idx == current).unwrap_or(0);
        let prev = if current_pos == 0 {
            order[order.len() - 1]
        } else {
            order[current_pos - 1]
        };
        self.switch_workspace(prev);
    }

    pub fn move_workspace(&mut self, source_idx: usize, insert_idx: usize) -> bool {
        if source_idx >= self.workspaces.len() || insert_idx > self.workspaces.len() {
            return false;
        }

        let target_idx = if source_idx < insert_idx {
            insert_idx - 1
        } else {
            insert_idx
        };
        if source_idx == target_idx {
            return false;
        }

        self.mark_session_dirty();

        let active_id = self.active.map(|idx| self.workspaces[idx].id.clone());
        let selected_id = self
            .workspaces
            .get(self.selected)
            .map(|workspace| workspace.id.clone());

        let workspace = self.workspaces.remove(source_idx);
        self.workspaces.insert(target_idx, workspace);

        self.active = active_id.and_then(|id| self.workspaces.iter().position(|ws| ws.id == id));
        self.selected = selected_id
            .and_then(|id| self.workspaces.iter().position(|ws| ws.id == id))
            .unwrap_or(0);
        self.ensure_workspace_visible(self.selected);
        true
    }

    /// Reconcile the flat manual agent order against the current set of agent
    /// panes. Called from the compute_view mutation phase so render stays pure.
    ///
    /// Drops stale entries, seeds the natural display order on first run, then
    /// places genuinely new agents: above the topmost existing pane of the same
    /// workspace, or at the very top when that workspace has no pane in the
    /// order yet.
    pub(crate) fn reconcile_agent_manual_order(&mut self) {
        // Flat agent-pane set in natural display order (workspaces x panes),
        // matching the flatten used by the ordering function.
        let mut flat: Vec<(crate::layout::PaneId, usize)> = Vec::new();
        let mut pane_workspace: std::collections::HashMap<crate::layout::PaneId, usize> =
            std::collections::HashMap::new();
        for (ws_idx, ws) in self.workspaces.iter().enumerate() {
            for detail in ws.pane_details(&self.terminals) {
                flat.push((detail.pane_id, ws_idx));
                pane_workspace.insert(detail.pane_id, ws_idx);
            }
        }
        let current: std::collections::HashSet<crate::layout::PaneId> =
            flat.iter().map(|(pane_id, _)| *pane_id).collect();

        // Drop stale pane entries and prune the known set. Line-splits are user
        // data, never derived from panes, so they are always retained.
        self.agent_manual_order.order.retain(|entry| match entry {
            crate::app::state::ManualEntry::Pane(pane_id) => current.contains(pane_id),
            crate::app::state::ManualEntry::LineSplit { .. } => true,
        });
        self.agent_manual_order
            .known
            .retain(|pane_id| current.contains(pane_id));

        if !self.agent_manual_order.seeded {
            // First reconcile: establish the natural pane order. Append panes not
            // yet present (line-splits, if any, keep their positions untouched).
            for (pane_id, _) in &flat {
                if self.agent_manual_order.known.insert(*pane_id) {
                    self.agent_manual_order
                        .order
                        .push(crate::app::state::ManualEntry::Pane(*pane_id));
                }
            }
            self.agent_manual_order.seeded = true;
            return;
        }

        for (pane_id, ws_idx) in &flat {
            if self.agent_manual_order.known.contains(pane_id) {
                continue;
            }
            // Genuinely new agent: insert above the topmost (earliest) pane entry
            // belonging to the same workspace, else at the very top. Line-splits
            // are skipped when locating that pane.
            let insert_pos = self
                .agent_manual_order
                .order
                .iter()
                .position(|entry| match entry {
                    crate::app::state::ManualEntry::Pane(other) => {
                        pane_workspace.get(other) == Some(ws_idx)
                    }
                    crate::app::state::ManualEntry::LineSplit { .. } => false,
                })
                .unwrap_or(0);
            self.agent_manual_order
                .order
                .insert(insert_pos, crate::app::state::ManualEntry::Pane(*pane_id));
            self.agent_manual_order.known.insert(*pane_id);
        }
    }

    /// Move a manual-order entry (agent pane or line-split) to a new position in
    /// the flat manual order. The `insert_idx` is a slot in the current order
    /// (before removal), clamped to bounds. Cross-workspace moves are allowed.
    /// Client-only presentation state, PTY-free. Returns true when the order
    /// changed.
    pub(crate) fn move_agent_entry(
        &mut self,
        source: crate::app::state::ManualEntryRef,
        insert_idx: usize,
    ) -> bool {
        use crate::app::state::{ManualEntry, ManualEntryRef};
        let Some(from) =
            self.agent_manual_order
                .order
                .iter()
                .position(|entry| match (entry, source) {
                    (ManualEntry::Pane(pane_id), ManualEntryRef::Pane(source_pane_id)) => {
                        *pane_id == source_pane_id
                    }
                    (ManualEntry::LineSplit { id, .. }, ManualEntryRef::LineSplit(source_id)) => {
                        *id == source_id
                    }
                    _ => false,
                })
        else {
            return false;
        };

        let insert_idx = insert_idx.min(self.agent_manual_order.order.len());
        let target_idx = if from < insert_idx {
            insert_idx - 1
        } else {
            insert_idx
        };
        if from == target_idx {
            return false;
        }

        self.mark_session_dirty();
        let entry = self.agent_manual_order.order.remove(from);
        self.agent_manual_order.order.insert(target_idx, entry);
        true
    }

    /// Translate an insert index expressed in visible tree-row space (what the
    /// drop indicator uses) into an index in the flat `agent_manual_order.order`
    /// (what [`move_agent_entry`] mutates). The two spaces differ once the tree
    /// nests children under parents; in the flat case this is the identity.
    ///
    /// The dragged item lands at the base-order position of the row it was
    /// dropped before. Because tree grouping is reapplied on every render, a
    /// child always re-nests under its parent regardless of where it lands in
    /// the base order, so a drag can never detach a child from its parent.
    pub(crate) fn agent_manual_base_index_for_tree_insert(&self, tree_insert_idx: usize) -> usize {
        use crate::app::state::ManualEntry;
        let rows = crate::ui::agent_panel_rows(self);
        let order = &self.agent_manual_order.order;
        let Some(row) = rows.get(tree_insert_idx) else {
            return order.len();
        };
        let pos = match row {
            crate::ui::AgentPanelRow::Agent(entry) => order
                .iter()
                .position(|e| matches!(e, ManualEntry::Pane(p) if *p == entry.pane_id)),
            crate::ui::AgentPanelRow::LineSplit { id, .. } => order
                .iter()
                .position(|e| matches!(e, ManualEntry::LineSplit { id: oid, .. } if oid == id)),
        };
        pos.unwrap_or(order.len())
    }

    /// True when the pane at `(ws_idx, pane_id)` is backed by an agent terminal.
    pub(crate) fn pane_is_agent(&self, ws_idx: usize, pane_id: crate::layout::PaneId) -> bool {
        self.workspaces
            .get(ws_idx)
            .and_then(|ws| ws.pane_state(pane_id))
            .and_then(|pane| self.terminals.get(&pane.attached_terminal_id))
            .is_some_and(|terminal| terminal.is_agent_terminal())
    }

    /// Live parent pane of the agent at `(ws_idx, pane_id)`, resolved from its
    /// stable parent link. `None` for roots or when the parent no longer exists.
    fn agent_parent_pane(
        &self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> Option<(usize, crate::layout::PaneId)> {
        self.workspaces
            .get(ws_idx)
            .and_then(|ws| ws.pane_state(pane_id))
            .and_then(|pane| pane.parent.as_ref())
            .and_then(|parent| self.resolve_pane_parent(parent))
    }

    /// Walk the parent chain upward from `(start_ws, start_pane)`, including the
    /// start pane itself, and report whether it reaches `(needle_ws,
    /// needle_pane)`. A visited set guards against pre-existing cycles so the
    /// walk always terminates. Used to reject reparent operations that would
    /// create a cycle.
    fn agent_parent_chain_contains(
        &self,
        start_ws: usize,
        start_pane: crate::layout::PaneId,
        needle_ws: usize,
        needle_pane: crate::layout::PaneId,
    ) -> bool {
        let mut current = Some((start_ws, start_pane));
        let mut visited = std::collections::HashSet::new();
        while let Some((ws_idx, pane_id)) = current {
            if ws_idx == needle_ws && pane_id == needle_pane {
                return true;
            }
            if !visited.insert((ws_idx, pane_id)) {
                break;
            }
            current = self.agent_parent_pane(ws_idx, pane_id);
        }
        false
    }

    /// Classify a manual-mode agent drag drop into a pending reparent operation,
    /// if any. Returns `Some` only when the drop should change the dragged
    /// agent's parent (attach under a parent whose children band the drop lands
    /// in, or detach to the top level); returns `None` for a plain reorder (no
    /// parent change), for line-splits, or for invalid targets (self/cycle).
    ///
    /// `tree_insert_idx` is the insertion slot in visible tree-row space, as
    /// produced by the drop indicator. Attaching requires the target parent to
    /// already have visible children (they form the drop band); a childless
    /// agent has no band to drop into, so it can never gain its first child by
    /// dragging.
    pub(crate) fn agent_reparent_intent_for_drop(
        &self,
        source: crate::app::state::ManualEntryRef,
        tree_insert_idx: usize,
    ) -> Option<crate::app::state::PendingAgentReparent> {
        use crate::app::state::{AgentReparentAction, ManualEntryRef, PendingAgentReparent};
        use crate::ui::AgentPanelRow;

        let source_pane = match source {
            ManualEntryRef::Pane(pane_id) => pane_id,
            ManualEntryRef::LineSplit(_) => return None,
        };

        let rows = crate::ui::agent_panel_rows(self);

        // Locate the dragged agent row and its (contiguous) subtree span so we
        // can skip it when scanning for the drop's neighbour.
        let source_idx = rows.iter().position(
            |row| matches!(row, AgentPanelRow::Agent(entry) if entry.pane_id == source_pane),
        )?;
        let source_entry = match &rows[source_idx] {
            AgentPanelRow::Agent(entry) => entry,
            AgentPanelRow::LineSplit { .. } => return None,
        };
        let source_ws = source_entry.ws_idx;
        let source_depth = source_entry.depth;
        let source_label = source_entry.primary_label.clone();
        // Subtree end: first later row at depth <= source depth.
        let mut subtree_end = rows.len();
        for (idx, row) in rows.iter().enumerate().skip(source_idx + 1) {
            let depth = match row {
                AgentPanelRow::Agent(entry) => entry.depth,
                AgentPanelRow::LineSplit { .. } => 0,
            };
            if depth <= source_depth {
                subtree_end = idx;
                break;
            }
        }
        let is_in_source_subtree = |idx: usize| idx >= source_idx && idx < subtree_end;

        // The row the dragged item would land after, ignoring its own subtree.
        let mut prev: Option<&AgentPanelRow> = None;
        for idx in (0..tree_insert_idx.min(rows.len())).rev() {
            if is_in_source_subtree(idx) {
                continue;
            }
            prev = Some(&rows[idx]);
            break;
        }

        // Determine the enclosing parent of the drop slot.
        let target_parent: Option<(usize, crate::layout::PaneId)> = match prev {
            Some(AgentPanelRow::Agent(entry)) if entry.has_children && !entry.collapsed => {
                // Dropped right after an expanded parent: becomes its first child.
                Some((entry.ws_idx, entry.pane_id))
            }
            Some(AgentPanelRow::Agent(entry)) if entry.depth >= 1 => {
                // Dropped among a parent's children: attach to that parent.
                entry
                    .parent_pane
                    .map(|parent_pane| (entry.ws_idx, parent_pane))
            }
            _ => None,
        };

        let current_parent = self.agent_parent_pane(source_ws, source_pane);
        if target_parent == current_parent {
            return None;
        }

        match target_parent {
            Some((parent_ws, parent_pane)) => {
                // Reject self-parenting and cycles.
                if parent_ws == source_ws && parent_pane == source_pane {
                    return None;
                }
                if self.agent_parent_chain_contains(parent_ws, parent_pane, source_ws, source_pane)
                {
                    return None;
                }
                let parent_label = rows.iter().find_map(|row| match row {
                    AgentPanelRow::Agent(entry)
                        if entry.ws_idx == parent_ws && entry.pane_id == parent_pane =>
                    {
                        Some(entry.primary_label.clone())
                    }
                    _ => None,
                })?;
                Some(PendingAgentReparent {
                    child_ws: source_ws,
                    child_pane: source_pane,
                    child_label: source_label,
                    parent_label,
                    action: AgentReparentAction::SetParent {
                        parent_ws,
                        parent_pane,
                    },
                    return_mode: self.mode,
                })
            }
            None => {
                // Detach only makes sense when the agent currently has a parent.
                let (parent_ws, parent_pane) = current_parent?;
                let parent_label = rows
                    .iter()
                    .find_map(|row| match row {
                        AgentPanelRow::Agent(entry)
                            if entry.ws_idx == parent_ws && entry.pane_id == parent_pane =>
                        {
                            Some(entry.primary_label.clone())
                        }
                        _ => None,
                    })
                    .unwrap_or_default();
                Some(PendingAgentReparent {
                    child_ws: source_ws,
                    child_pane: source_pane,
                    child_label: source_label,
                    parent_label,
                    action: AgentReparentAction::ClearParent,
                    return_mode: self.mode,
                })
            }
        }
    }

    /// Apply a confirmed reparent: set or clear the dragged agent's stable parent
    /// link. Re-validates agent-ness and cycles defensively in case state moved
    /// since the drop. Returns true when the link changed.
    pub(crate) fn apply_agent_reparent(
        &mut self,
        pending: &crate::app::state::PendingAgentReparent,
    ) -> bool {
        use crate::app::state::AgentReparentAction;

        if !self.pane_is_agent(pending.child_ws, pending.child_pane) {
            return false;
        }

        let new_parent_ref = match pending.action {
            AgentReparentAction::SetParent {
                parent_ws,
                parent_pane,
            } => {
                if !self.pane_is_agent(parent_ws, parent_pane) {
                    return false;
                }
                if parent_ws == pending.child_ws && parent_pane == pending.child_pane {
                    return false;
                }
                if self.agent_parent_chain_contains(
                    parent_ws,
                    parent_pane,
                    pending.child_ws,
                    pending.child_pane,
                ) {
                    return false;
                }
                let Some(parent_ws_ref) = self.workspaces.get(parent_ws) else {
                    return false;
                };
                let Some(pane_number) = parent_ws_ref.public_pane_number(parent_pane) else {
                    return false;
                };
                Some(crate::pane::PaneParentRef {
                    workspace_id: parent_ws_ref.id.clone(),
                    pane_number,
                })
            }
            AgentReparentAction::ClearParent => None,
        };

        let Some(pane_state) = self
            .workspaces
            .get_mut(pending.child_ws)
            .and_then(|ws| ws.pane_state_mut(pending.child_pane))
        else {
            return false;
        };
        if pane_state.parent == new_parent_ref {
            return false;
        }
        pane_state.parent = new_parent_ref;
        self.mark_session_dirty();
        true
    }

    /// Reconcile the client-only Panes-section order with the live set of
    /// non-agent panes across all workspaces. Called from the compute_view
    /// mutation phase so render stays pure.
    ///
    /// Drops references to panes that no longer exist or that became agent panes,
    /// seeds the natural display order on first run, then places genuinely new
    /// non-agent panes at the top of the list.
    pub(crate) fn reconcile_pane_section_order(&mut self) {
        use crate::app::state::{PaneManualEntry, PaneSectionRef};
        // Flat set of live non-agent panes in natural display order
        // (workspaces x tabs x panes).
        let mut flat: Vec<PaneSectionRef> = Vec::new();
        for ws in &self.workspaces {
            for (_tab_idx, _pane_id, pane_number) in ws.non_agent_panes(&self.terminals) {
                flat.push(PaneSectionRef {
                    workspace_id: ws.id.clone(),
                    pane_number,
                });
            }
        }
        let current: std::collections::HashSet<PaneSectionRef> = flat.iter().cloned().collect();

        // Drop stale pane references (closed panes or panes that became agent
        // panes) and prune the known set to match. Line-splits are user data,
        // never derived from panes, so they are always retained.
        self.pane_section_order.order.retain(|entry| match entry {
            PaneManualEntry::Pane(pane_ref) => current.contains(pane_ref),
            PaneManualEntry::LineSplit { .. } => true,
        });
        self.pane_section_order
            .known
            .retain(|pane_ref| current.contains(pane_ref));

        if !self.pane_section_order.seeded {
            // First reconcile: establish the natural pane order. Append panes not
            // yet present (line-splits, if any, keep their positions untouched).
            for pane_ref in &flat {
                if self.pane_section_order.known.insert(pane_ref.clone()) {
                    self.pane_section_order
                        .order
                        .push(PaneManualEntry::Pane(pane_ref.clone()));
                }
            }
            self.pane_section_order.seeded = true;
            return;
        }

        // Genuinely new non-agent panes go to the top of the list, keeping their
        // natural relative order among themselves. Line-splits are left in place.
        let mut insert_at = 0usize;
        for pane_ref in &flat {
            if self.pane_section_order.known.contains(pane_ref) {
                continue;
            }
            let at = insert_at.min(self.pane_section_order.order.len());
            self.pane_section_order
                .order
                .insert(at, PaneManualEntry::Pane(pane_ref.clone()));
            self.pane_section_order.known.insert(pane_ref.clone());
            insert_at = at + 1;
        }
    }

    /// Move a Panes-section entry (non-agent pane or line-split) to a new position
    /// in the flat order. `insert_idx` is a slot in the current order (before
    /// removal), clamped to bounds. Cross-space moves are allowed. This is
    /// client-only presentation state and never changes the real pane order inside
    /// any workspace. Returns true when the order changed.
    pub(crate) fn move_pane_section_entry(
        &mut self,
        source: crate::app::state::PaneManualEntryRef,
        insert_idx: usize,
    ) -> bool {
        use crate::app::state::{PaneManualEntry, PaneManualEntryRef};
        let Some(from) =
            self.pane_section_order
                .order
                .iter()
                .position(|entry| match (entry, &source) {
                    (PaneManualEntry::Pane(pane_ref), PaneManualEntryRef::Pane(source_ref)) => {
                        pane_ref == source_ref
                    }
                    (
                        PaneManualEntry::LineSplit { id, .. },
                        PaneManualEntryRef::LineSplit(source_id),
                    ) => id == source_id,
                    _ => false,
                })
        else {
            return false;
        };

        let insert_idx = insert_idx.min(self.pane_section_order.order.len());
        let target_idx = if from < insert_idx {
            insert_idx - 1
        } else {
            insert_idx
        };
        if from == target_idx {
            return false;
        }

        self.mark_session_dirty();
        let entry = self.pane_section_order.order.remove(from);
        self.pane_section_order.order.insert(target_idx, entry);
        true
    }

    pub fn scroll_tabs_left(&mut self) {
        self.tab_scroll_follow_active = false;
        self.tab_scroll = self.tab_scroll.saturating_sub(1);
        self.refresh_tab_bar_view();
    }

    pub fn scroll_tabs_right(&mut self) {
        self.tab_scroll_follow_active = false;
        self.tab_scroll = self.tab_scroll.saturating_add(1);
        self.refresh_tab_bar_view();
    }

    pub fn next_tab(&mut self) {
        if let Some(ws) = self.active.and_then(|i| self.workspaces.get(i)) {
            if !ws.tabs.is_empty() {
                let next = (ws.active_tab + 1) % ws.tabs.len();
                self.switch_tab(next);
            }
        }
    }

    pub fn previous_tab(&mut self) {
        if let Some(ws) = self.active.and_then(|i| self.workspaces.get(i)) {
            if !ws.tabs.is_empty() {
                let prev = if ws.active_tab == 0 {
                    ws.tabs.len() - 1
                } else {
                    ws.active_tab - 1
                };
                self.switch_tab(prev);
            }
        }
    }

    pub fn next_agent(&mut self) {
        self.cycle_agent_entry(true);
    }

    pub fn previous_agent(&mut self) {
        self.cycle_agent_entry(false);
    }

    /// Visible agent panes in tree order (agents only, collapsed descendants
    /// excluded), used for keyboard navigation and numeric focus so navigation
    /// skips collapsed subtrees exactly as the panel shows them. Shared with the
    /// live TUI navigation path so both cycle the same visible order.
    pub(crate) fn visible_agent_targets(&self) -> Vec<(usize, crate::layout::PaneId)> {
        crate::ui::agent_panel_rows(self)
            .into_iter()
            .filter_map(|row| match row {
                crate::ui::AgentPanelRow::Agent(entry) => Some((entry.ws_idx, entry.pane_id)),
                crate::ui::AgentPanelRow::LineSplit { .. } => None,
            })
            .collect()
    }

    pub fn focus_agent_entry(&mut self, idx: usize) -> bool {
        let targets = self.visible_agent_targets();
        let Some(&(ws_idx, pane_id)) = targets.get(idx) else {
            return false;
        };
        // Scroll math tracks the full row list (agents + line-splits), so map the
        // agent to its row index for ensure-visible.
        let row_idx = crate::ui::agent_panel_row_index_of_pane(self, pane_id).unwrap_or(idx);

        if self.active == Some(ws_idx) && self.workspaces[ws_idx].focused_pane_id() == Some(pane_id)
        {
            self.ensure_agent_panel_entry_visible(row_idx);
            return true;
        }

        if self.focus_pane_in_workspace(ws_idx, pane_id) {
            self.ensure_agent_panel_entry_visible(row_idx);
            return true;
        }
        false
    }

    fn cycle_agent_entry(&mut self, forward: bool) {
        let targets = self.visible_agent_targets();
        let ids: Vec<PaneId> = targets.iter().map(|(_, pane_id)| *pane_id).collect();
        let focused = self
            .active
            .and_then(|idx| self.workspaces.get(idx))
            .and_then(crate::workspace::Workspace::focused_pane_id);
        let Some(target_idx) =
            section_cycle_target_index(&ids, focused, self.last_agent_focus, forward)
        else {
            return;
        };

        self.focus_agent_entry(target_idx);
    }

    pub fn next_pane(&mut self) {
        self.cycle_pane_section_entry(true);
    }

    pub fn previous_pane(&mut self) {
        self.cycle_pane_section_entry(false);
    }

    /// Non-agent panes in Panes-section order (line-splits excluded), used for
    /// keyboard cycling so navigation follows exactly what the section shows.
    /// Mirrors [`Self::visible_agent_targets`].
    fn pane_section_targets(&self) -> Vec<(usize, crate::layout::PaneId)> {
        crate::ui::sidebar_pane_section_entries(self)
            .into_iter()
            .map(|entry| (entry.ws_idx, entry.pane_id))
            .collect()
    }

    /// Focus the Panes-section entry at `idx`, switching workspace and tab as a
    /// click on that row would, and scroll it into view. Mirrors
    /// [`Self::focus_agent_entry`].
    fn focus_pane_section_entry(&mut self, idx: usize) -> bool {
        let targets = self.pane_section_targets();
        let Some(&(ws_idx, pane_id)) = targets.get(idx) else {
            return false;
        };

        if self.active == Some(ws_idx) && self.workspaces[ws_idx].focused_pane_id() == Some(pane_id)
        {
            self.ensure_pane_section_row_visible(pane_id);
            return true;
        }

        if self.focus_pane_in_workspace(ws_idx, pane_id) {
            self.ensure_pane_section_row_visible(pane_id);
            return true;
        }
        false
    }

    fn cycle_pane_section_entry(&mut self, forward: bool) {
        let targets = self.pane_section_targets();
        let ids: Vec<PaneId> = targets.iter().map(|(_, pane_id)| *pane_id).collect();
        let focused = self
            .active
            .and_then(|idx| self.workspaces.get(idx))
            .and_then(crate::workspace::Workspace::focused_pane_id);
        let Some(target_idx) =
            section_cycle_target_index(&ids, focused, self.last_pane_section_focus, forward)
        else {
            return;
        };

        self.focus_pane_section_entry(target_idx);
    }

    /// Ensure the agent-panel row for `pane_id` is scrolled into view, mapping the
    /// pane to its row index in the full (agents + line-splits) row list.
    pub(crate) fn ensure_agent_panel_pane_visible(&mut self, pane_id: crate::layout::PaneId) {
        if let Some(row_idx) = crate::ui::agent_panel_row_index_of_pane(self, pane_id) {
            self.ensure_agent_panel_entry_visible(row_idx);
        }
    }

    pub(crate) fn ensure_agent_panel_entry_visible(&mut self, idx: usize) {
        if self.sidebar_collapsed {
            return;
        }

        let detail_area = crate::ui::agents_detail_rect(
            self.view.sidebar_rect,
            self.sidebar_section_split,
            self.sidebar_pane_section_split,
            crate::ui::sidebar_shows_pane_section(self),
        );
        let metrics = crate::ui::agent_panel_scroll_metrics(self, detail_area);
        let visible = metrics.viewport_rows;
        if visible == 0 {
            return;
        }

        if idx < self.agent_panel_scroll {
            self.agent_panel_scroll = idx;
        } else if idx >= self.agent_panel_scroll.saturating_add(visible) {
            self.agent_panel_scroll = idx.saturating_add(1).saturating_sub(visible);
        }

        let max_scroll =
            crate::ui::agent_panel_scroll_metrics(self, detail_area).max_offset_from_bottom;
        self.agent_panel_scroll = self.agent_panel_scroll.min(max_scroll);
    }

    /// Scroll the Panes section so the row for `pane_id` is visible, if that pane
    /// is currently listed. Mirrors `ensure_agent_panel_entry_visible`.
    pub(crate) fn ensure_pane_section_row_visible(&mut self, pane_id: PaneId) {
        if self.sidebar_collapsed {
            return;
        }
        // Scroll math tracks the full row list (panes + line-splits), so map the
        // pane to its row index for ensure-visible.
        let Some(resolved_idx) = crate::ui::pane_section_row_index_of_pane(self, pane_id) else {
            return;
        };

        let pane_section_area = crate::ui::pane_section_rect(
            self.view.sidebar_rect,
            self.sidebar_section_split,
            self.sidebar_pane_section_split,
            crate::ui::sidebar_shows_pane_section(self),
        );
        let metrics = crate::ui::pane_section_scroll_metrics(self, pane_section_area);
        let visible = metrics.viewport_rows;
        if visible == 0 {
            return;
        }

        if resolved_idx < self.pane_section_scroll {
            self.pane_section_scroll = resolved_idx;
        } else if resolved_idx >= self.pane_section_scroll.saturating_add(visible) {
            self.pane_section_scroll = resolved_idx.saturating_add(1).saturating_sub(visible);
        }
        self.pane_section_scroll = self.pane_section_scroll.min(metrics.max_offset_from_bottom);
    }

    pub(crate) fn terminal_ids_for_workspace(
        &self,
        ws_idx: usize,
    ) -> Vec<crate::terminal::TerminalId> {
        self.workspaces
            .get(ws_idx)
            .into_iter()
            .flat_map(|ws| &ws.tabs)
            .flat_map(|tab| tab.panes.values())
            .map(|pane| pane.attached_terminal_id.clone())
            .collect()
    }

    pub(crate) fn pane_ids_for_workspace(&self, ws_idx: usize) -> Vec<PaneId> {
        self.workspaces
            .get(ws_idx)
            .into_iter()
            .flat_map(|ws| &ws.tabs)
            .flat_map(|tab| tab.layout.pane_ids())
            .collect()
    }

    pub(crate) fn terminal_ids_for_tab(
        &self,
        ws_idx: usize,
        tab_idx: usize,
    ) -> Vec<crate::terminal::TerminalId> {
        self.workspaces
            .get(ws_idx)
            .and_then(|ws| ws.tabs.get(tab_idx))
            .into_iter()
            .flat_map(|tab| tab.panes.values())
            .map(|pane| pane.attached_terminal_id.clone())
            .collect()
    }

    pub(crate) fn pane_ids_for_tab(&self, ws_idx: usize, tab_idx: usize) -> Vec<PaneId> {
        self.workspaces
            .get(ws_idx)
            .and_then(|ws| ws.tabs.get(tab_idx))
            .map(|tab| tab.layout.pane_ids())
            .unwrap_or_default()
    }

    pub(crate) fn terminal_id_for_pane(
        &self,
        ws_idx: usize,
        pane_id: PaneId,
    ) -> Option<crate::terminal::TerminalId> {
        self.workspaces
            .get(ws_idx)?
            .pane_state(pane_id)
            .map(|pane| pane.attached_terminal_id.clone())
    }

    pub(crate) fn remove_unattached_terminal_ids(
        &mut self,
        terminal_ids: impl IntoIterator<Item = crate::terminal::TerminalId>,
    ) {
        for terminal_id in terminal_ids {
            let still_attached = self.workspaces.iter().any(|ws| {
                ws.tabs.iter().any(|tab| {
                    tab.panes
                        .values()
                        .any(|pane| pane.attached_terminal_id == terminal_id)
                })
            });
            if !still_attached
                && self.terminals.remove(&terminal_id).is_some()
                && !self.terminal_runtime_shutdowns.contains(&terminal_id)
            {
                self.terminal_runtime_shutdowns.push(terminal_id);
            }
        }
    }

    pub(crate) fn remove_plugin_pane_records(
        &mut self,
        pane_ids: impl IntoIterator<Item = PaneId>,
    ) {
        for pane_id in pane_ids {
            self.plugin_panes.remove(&pane_id);
        }
    }

    pub fn close_selected_workspace(&mut self) {
        if self.workspaces.is_empty() {
            return;
        }
        self.selection = None;
        self.selection_autoscroll = None;
        self.mark_session_dirty();
        let close_indices = self
            .workspaces
            .get(self.selected)
            .and_then(|ws| ws.worktree_space())
            .filter(|space| !space.is_linked_worktree)
            .map(|space| {
                self.workspaces
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, ws)| {
                        ws.worktree_space()
                            .is_some_and(|member| member.key == space.key)
                            .then_some(idx)
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|indices| indices.len() >= 2)
            .unwrap_or_else(|| vec![self.selected]);

        let mut terminal_ids = Vec::new();
        let mut pane_ids = Vec::new();
        for idx in &close_indices {
            terminal_ids.extend(self.terminal_ids_for_workspace(*idx));
            pane_ids.extend(self.pane_ids_for_workspace(*idx));
            if let Some(workspace_id) = self.workspaces.get(*idx).map(|ws| ws.id.clone()) {
                crate::logging::workspace_closed(&workspace_id);
            }
        }
        self.remove_plugin_pane_records(pane_ids);
        for idx in close_indices.iter().rev() {
            self.workspaces.remove(*idx);
        }
        self.remove_unattached_terminal_ids(terminal_ids);
        if self.workspaces.is_empty() {
            self.active = None;
            self.selected = 0;
            self.workspace_scroll = 0;
            self.tab_scroll = 0;
            self.tab_scroll_follow_active = true;
        } else {
            if self.selected >= self.workspaces.len() {
                self.selected = self.workspaces.len() - 1;
            }
            self.active = Some(self.selected);
            self.workspace_scroll = self
                .workspace_scroll
                .min(self.workspaces.len().saturating_sub(1));
            self.ensure_workspace_visible(self.selected);
            self.tab_scroll_follow_active = true;
            self.refresh_tab_bar_view();
        }
    }

    pub(crate) fn refresh_tab_bar_view(&mut self) {
        let area = self.view.tab_bar_rect;
        let Some(ws) = self.active.and_then(|idx| self.workspaces.get(idx)) else {
            self.tab_scroll = 0;
            self.view.tab_hit_areas.clear();
            self.view.tab_scroll_left_hit_area = ratatui::layout::Rect::default();
            self.view.tab_scroll_right_hit_area = ratatui::layout::Rect::default();
            self.view.new_tab_hit_area = ratatui::layout::Rect::default();
            return;
        };

        let layout = crate::ui::compute_tab_bar_view(
            ws,
            area,
            self.tab_scroll,
            self.tab_scroll_follow_active,
            self.mouse_capture,
        );
        self.tab_scroll = layout.scroll;
        self.view.tab_hit_areas = layout.tab_hit_areas;
        self.view.tab_scroll_left_hit_area = layout.scroll_left_hit_area;
        self.view.tab_scroll_right_hit_area = layout.scroll_right_hit_area;
        self.view.new_tab_hit_area = layout.new_tab_hit_area;
    }
}

// ---------------------------------------------------------------------------
// Pane operations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaneZoomCommand {
    Toggle,
    On,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaneZoomNoopReason {
    SinglePane,
    AlreadyZoomed,
    AlreadyUnzoomed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PaneZoomOutcome {
    pub changed: bool,
    pub focus_changed: bool,
    pub reason: Option<PaneZoomNoopReason>,
    pub zoomed: bool,
}

impl AppState {
    pub fn navigate_pane(&mut self, direction: NavDirection) {
        let Some(ws_idx) = self.active else {
            return;
        };
        let Some(tab) = self.workspaces.get(ws_idx).and_then(|ws| ws.active_tab()) else {
            return;
        };
        let panes = if tab.zoomed {
            tab.layout.panes(self.view.terminal_area)
        } else {
            self.view.pane_infos.clone()
        };

        if let Some(focused) = panes.iter().find(|p| p.is_focused) {
            if let Some(target) = find_in_direction(focused, direction, &panes) {
                self.focus_pane_in_workspace(ws_idx, target);
            }
        }
    }

    pub fn swap_pane(&mut self, direction: NavDirection) -> bool {
        let Some(ws_idx) = self.active else {
            return false;
        };
        let Some(tab) = self.workspaces.get(ws_idx).and_then(|ws| ws.active_tab()) else {
            return false;
        };
        let panes = if tab.zoomed {
            tab.layout.panes(self.view.terminal_area)
        } else {
            self.view.pane_infos.clone()
        };

        let Some(focused) = panes.iter().find(|p| p.is_focused) else {
            return false;
        };
        let Some(target) = find_in_direction(focused, direction, &panes) else {
            return false;
        };
        let source = focused.id;
        let Some(tab) = self
            .workspaces
            .get_mut(ws_idx)
            .and_then(|ws| ws.active_tab_mut())
        else {
            return false;
        };
        if tab.layout.swap_panes(source, target) {
            self.mark_session_dirty();
            true
        } else {
            false
        }
    }

    #[cfg(test)]
    pub fn resize_pane(&mut self, direction: NavDirection) {
        if let Some(first) = self.view.pane_infos.first() {
            let area = self
                .view
                .pane_infos
                .iter()
                .fold(first.rect, |acc, p| acc.union(p.rect));
            if let Some(tab) = self
                .active
                .and_then(|i| self.workspaces.get_mut(i))
                .and_then(|ws| ws.active_tab_mut())
            {
                tab.layout.resize_focused(direction, 0.05, area);
                self.mark_session_dirty();
            }
        }
    }

    pub fn cycle_pane(&mut self, reverse: bool) {
        let Some(ws_idx) = self.active else {
            return;
        };
        let Some(tab) = self.workspaces.get(ws_idx).and_then(|ws| ws.active_tab()) else {
            return;
        };
        let ids = tab.layout.pane_ids();
        if let Some(pos) = ids.iter().position(|id| *id == tab.layout.focused()) {
            let target = if reverse {
                ids[(pos + ids.len() - 1) % ids.len()]
            } else {
                ids[(pos + 1) % ids.len()]
            };
            self.focus_pane_in_workspace(ws_idx, target);
        }
    }

    pub fn last_pane(&mut self) {
        let Some(target) = self.previous_pane_focus.clone() else {
            return;
        };
        let Some((ws_idx, tab_idx)) = self.pane_focus_target_indices(&target) else {
            self.previous_pane_focus = None;
            return;
        };
        let current = self.current_pane_focus_target();
        if current.as_ref() == Some(&target) {
            self.previous_pane_focus = None;
            return;
        }

        self.switch_workspace_tab(ws_idx, tab_idx);
        if let Some(tab) = self
            .workspaces
            .get_mut(ws_idx)
            .and_then(|ws| ws.tabs.get_mut(tab_idx))
        {
            tab.layout.focus_pane(target.pane_id);
            self.previous_pane_focus = current;
            self.mark_session_dirty();
        }
    }

    pub(crate) fn apply_pane_zoom(
        &mut self,
        ws_idx: usize,
        pane_id: PaneId,
        command: PaneZoomCommand,
    ) -> Option<PaneZoomOutcome> {
        let tab_idx = self
            .workspaces
            .get(ws_idx)?
            .find_tab_index_for_pane(pane_id)?;
        let focus_changed = self.focus_pane_in_workspace(ws_idx, pane_id);
        let tab = self
            .workspaces
            .get_mut(ws_idx)
            .and_then(|ws| ws.tabs.get_mut(tab_idx))?;
        if tab.layout.pane_count() <= 1 {
            return Some(PaneZoomOutcome {
                changed: false,
                focus_changed,
                reason: Some(PaneZoomNoopReason::SinglePane),
                zoomed: tab.zoomed,
            });
        }

        let desired = match command {
            PaneZoomCommand::Toggle => !tab.zoomed,
            PaneZoomCommand::On => true,
            PaneZoomCommand::Off => false,
        };
        let reason = match (command, tab.zoomed) {
            (PaneZoomCommand::On, true) => Some(PaneZoomNoopReason::AlreadyZoomed),
            (PaneZoomCommand::Off, false) => Some(PaneZoomNoopReason::AlreadyUnzoomed),
            _ => None,
        };
        if reason.is_some() {
            return Some(PaneZoomOutcome {
                changed: false,
                focus_changed,
                reason,
                zoomed: tab.zoomed,
            });
        }

        tab.zoomed = desired;
        let zoomed = tab.zoomed;
        self.mark_session_dirty();
        Some(PaneZoomOutcome {
            changed: true,
            focus_changed,
            reason: None,
            zoomed,
        })
    }

    pub fn toggle_zoom(&mut self) {
        let Some(ws_idx) = self.active else {
            return;
        };
        let Some(pane_id) = self
            .workspaces
            .get(ws_idx)
            .and_then(crate::workspace::Workspace::focused_pane_id)
        else {
            return;
        };
        self.apply_pane_zoom(ws_idx, pane_id, PaneZoomCommand::Toggle);
    }

    pub(crate) fn workspace_close_would_close_worktree_group(&self, ws_idx: usize) -> bool {
        self.workspaces
            .get(ws_idx)
            .and_then(|ws| ws.worktree_space())
            .filter(|space| !space.is_linked_worktree)
            .is_some_and(|space| {
                self.workspaces
                    .iter()
                    .filter(|ws| {
                        ws.worktree_space()
                            .is_some_and(|member| member.key == space.key)
                    })
                    .count()
                    >= 2
            })
    }

    pub(crate) fn confirm_implicit_worktree_group_close(&mut self, ws_idx: usize) -> bool {
        if self.confirm_close && self.workspace_close_would_close_worktree_group(ws_idx) {
            self.selected = ws_idx;
            self.mode = Mode::ConfirmClose;
            true
        } else {
            false
        }
    }

    fn close_focused_pane_would_close_workspace(&self, ws_idx: usize) -> bool {
        self.workspaces.get(ws_idx).is_some_and(|ws| {
            let pane_count = ws
                .active_tab()
                .map(|tab| tab.layout.pane_count())
                .unwrap_or(0);
            pane_count <= 1 && ws.tabs.len() <= 1
        })
    }

    pub(crate) fn close_pane_would_close_workspace(&self, ws_idx: usize, pane_id: PaneId) -> bool {
        self.workspaces.get(ws_idx).is_some_and(|ws| {
            ws.find_tab_index_for_pane(pane_id).is_some_and(|tab_idx| {
                ws.tabs[tab_idx].layout.pane_count() <= 1 && ws.tabs.len() <= 1
            })
        })
    }

    /// Close the focused pane. Returns true when the close was deferred to confirmation.
    pub fn close_pane(&mut self) -> bool {
        let active = self.active;
        if active.is_some_and(|ws_idx| {
            self.close_focused_pane_would_close_workspace(ws_idx)
                && self.workspace_close_would_close_worktree_group(ws_idx)
        }) {
            if let Some(ws_idx) = active {
                if self.confirm_implicit_worktree_group_close(ws_idx) {
                    return true;
                }
            }
        }

        self.selection = None;
        self.selection_autoscroll = None;
        self.mark_session_dirty();
        let terminal_ids = active
            .and_then(|i| {
                self.workspaces
                    .get(i)
                    .and_then(|ws| ws.focused_pane_id().map(|pane_id| (i, pane_id)))
            })
            .and_then(|(i, pane_id)| self.terminal_id_for_pane(i, pane_id))
            .into_iter()
            .collect::<Vec<_>>();
        let pane_ids = active
            .and_then(|i| self.workspaces.get(i).and_then(|ws| ws.focused_pane_id()))
            .into_iter()
            .collect::<Vec<_>>();
        let should_close_workspace = active
            .and_then(|i| self.workspaces.get_mut(i))
            .is_some_and(|ws| ws.close_focused());
        self.remove_plugin_pane_records(pane_ids);
        if should_close_workspace {
            if let Some(active) = active {
                self.selected = active;
            }
            self.close_selected_workspace();
        } else {
            self.remove_unattached_terminal_ids(terminal_ids);
        }
        false
    }

    /// Close the active tab. Returns true when the close was deferred to confirmation.
    pub fn close_tab(&mut self) -> bool {
        if self.active.is_some_and(|ws_idx| {
            self.workspaces
                .get(ws_idx)
                .is_some_and(|ws| ws.tabs.len() <= 1)
                && self.workspace_close_would_close_worktree_group(ws_idx)
        }) {
            if let Some(ws_idx) = self.active {
                if self.confirm_implicit_worktree_group_close(ws_idx) {
                    return true;
                }
            }
        }

        self.selection = None;
        self.selection_autoscroll = None;
        self.mark_session_dirty();
        let should_close_workspace = self
            .active
            .and_then(|i| self.workspaces.get(i))
            .is_some_and(|ws| ws.tabs.len() <= 1);
        if should_close_workspace {
            if let Some(active) = self.active {
                self.selected = active;
            }
            self.close_selected_workspace();
            return false;
        }
        if let Some(ws_idx) = self.active {
            let terminal_ids = self
                .workspaces
                .get(ws_idx)
                .map(|ws| self.terminal_ids_for_tab(ws_idx, ws.active_tab))
                .unwrap_or_default();
            let pane_ids = self
                .workspaces
                .get(ws_idx)
                .map(|ws| self.pane_ids_for_tab(ws_idx, ws.active_tab))
                .unwrap_or_default();
            let Some(ws) = self.workspaces.get_mut(ws_idx) else {
                return false;
            };
            let workspace_id = ws.id.clone();
            let closing_tab_id =
                public_tab_id_for_index(ws, ws.active_tab).unwrap_or_else(|| workspace_id.clone());
            ws.close_active_tab();
            self.remove_plugin_pane_records(pane_ids);
            self.remove_unattached_terminal_ids(terminal_ids);
            crate::logging::tab_closed(&workspace_id, &closing_tab_id);
            self.tab_scroll_follow_active = true;
            self.refresh_tab_bar_view();
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

impl AppState {
    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.selection_autoscroll = None;
    }

    pub(crate) fn stop_selection_autoscroll_state(&mut self) {
        self.selection_autoscroll = None;
    }

    pub(crate) fn copy_word_at_pane_cell(
        &mut self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
        pane_id: crate::layout::PaneId,
        viewport_row: u16,
        col: u16,
    ) -> bool {
        // Resolve the active pane cell the double-click landed on.
        let Some(ws_idx) = self
            .active
            .filter(|idx| self.workspaces.get(*idx).is_some())
        else {
            return false;
        };

        let Some(info) = self.pane_info_by_id(pane_id) else {
            return false;
        };
        if viewport_row >= info.inner_rect.height || col >= info.inner_rect.width {
            return false;
        }

        // Leave mouse input to terminal apps that requested it.
        let Some(rt) = self.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, pane_id)
        else {
            return false;
        };
        if rt
            .input_state()
            .is_some_and(crate::pane::InputState::mouse_reporting_enabled)
        {
            return false;
        }

        // Read the visible row and identify the clicked token bounds.
        let metrics = self.pane_scroll_metrics(terminal_runtimes, pane_id);
        let row_selection = Selection::range(
            pane_id,
            viewport_row,
            0,
            info.inner_rect.width.saturating_sub(1),
            metrics,
        );
        let Some(row_text) = rt.extract_selection(&row_selection) else {
            return false;
        };
        let Some((start_col, end_col)) = word_bounds_at_column(&row_text, col) else {
            return false;
        };

        // Copy the token and keep its selection visible as short-lived feedback.
        let mut selection = Selection::range(pane_id, viewport_row, start_col, end_col, metrics);
        if !selection.finish() {
            return false;
        }

        let Some(text) = rt
            .extract_selection(&selection)
            .filter(|text| !text.is_empty())
        else {
            self.clear_selection();
            return false;
        };
        self.request_clipboard_write = Some(text.into_bytes());
        self.selection = Some(selection);
        self.selection_autoscroll = None;
        info!("copied double-clicked token to clipboard");
        true
    }

    pub(crate) fn url_at_pane_cell(
        &self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
        pane_id: crate::layout::PaneId,
        viewport_row: u16,
        col: u16,
    ) -> Option<String> {
        let ws_idx = self
            .active
            .filter(|idx| self.workspaces.get(*idx).is_some())?;
        let info = self.pane_info_by_id(pane_id)?;
        if viewport_row >= info.inner_rect.height || col >= info.inner_rect.width {
            return None;
        }

        let rt = self.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, pane_id)?;
        let screen_col = info.inner_rect.x.saturating_add(col);
        let screen_row = info.inner_rect.y.saturating_add(viewport_row);
        if let Some((_, _, uri)) = rt
            .visible_hyperlinks(info.inner_rect)
            .into_iter()
            .find(|((x, y), _, _)| *x == screen_col && *y == screen_row)
        {
            return safe_web_url(&uri).map(str::to_owned);
        }

        let metrics = self.pane_scroll_metrics(terminal_runtimes, pane_id);
        let row_selection = Selection::range(
            pane_id,
            viewport_row,
            0,
            info.inner_rect.width.saturating_sub(1),
            metrics,
        );
        let row_text = rt.extract_selection(&row_selection)?;
        url_at_column(&row_text, col).map(str::to_owned)
    }

    pub fn copy_selection(&mut self, terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry) {
        let mut sel = match self.selection.take() {
            Some(sel) => sel,
            None => return,
        };
        if !sel.finish() {
            return;
        }

        let ws_idx = match self.active {
            Some(ws_idx) if self.workspaces.get(ws_idx).is_some() => ws_idx,
            _ => return,
        };

        let text = self
            .runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, sel.pane_id)
            .and_then(|rt| rt.extract_selection(&sel));
        if let Some(text) = text {
            if !text.is_empty() {
                self.request_clipboard_write = Some(text.into_bytes());
                info!("copied selection to clipboard");
            }
        }

        self.clear_selection();
    }
}

pub(crate) fn safe_web_url(url: &str) -> Option<&str> {
    (url.starts_with("http://") || url.starts_with("https://")).then_some(url)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TextCell {
    ch: char,
    start_col: u16,
    end_col: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CellSpan {
    start: usize,
    end: usize,
}

impl CellSpan {
    fn contains(self, idx: usize) -> bool {
        idx >= self.start && idx <= self.end
    }

    fn columns(self, cells: &[TextCell]) -> (u16, u16) {
        (cells[self.start].start_col, cells[self.end].end_col)
    }
}

/// Finds the terminal display-column bounds for the token under a double-click.
///
/// The algorithm first maps text to terminal cells so wide characters and
/// zero-width marks use display columns, then prefers structured spans that
/// users expect to copy whole (URLs and quoted paths), and finally falls back
/// to a separator-delimited token.
fn word_bounds_at_column(row: &str, col: u16) -> Option<(u16, u16)> {
    // Map the row into display cells before doing any word-boundary work.
    let cells = text_cells(row);
    let clicked_idx = cell_index_at_column(&cells, col)?;

    // Prefer spans that can legally include punctuation or spaces.
    let span = url_span_at_column(&cells, clicked_idx)
        .or_else(|| quoted_path_span_at_column(&cells, clicked_idx))
        .or_else(|| token_span_at_column(&cells, clicked_idx))?;

    // Convert the internal cell span back to inclusive terminal columns.
    Some(span.columns(&cells))
}

pub(crate) fn url_at_column(row: &str, col: u16) -> Option<&str> {
    let cells = text_cells(row);
    let clicked_idx = cell_index_at_column(&cells, col)?;
    let span = url_span_at_column(&cells, clicked_idx)?;
    let start_byte = byte_index_for_cell(row, span.start);
    let end_byte = byte_index_after_cell(row, span.end);
    safe_web_url(row.get(start_byte..end_byte)?)
}

fn token_span_at_column(cells: &[TextCell], clicked_idx: usize) -> Option<CellSpan> {
    if is_word_separator(cells[clicked_idx].ch) {
        return None;
    }

    let mut start = clicked_idx;
    while start > 0 && !is_word_separator(cells[start - 1].ch) {
        start -= 1;
    }

    let mut end = clicked_idx;
    while end + 1 < cells.len() && !is_word_separator(cells[end + 1].ch) {
        end += 1;
    }

    trim_token_edges(cells, CellSpan { start, end }).filter(|span| span.contains(clicked_idx))
}

fn text_cells(row: &str) -> Vec<TextCell> {
    let mut next_col = 0u16;
    row.chars()
        .map(|ch| {
            let width = UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
            let start_col = if width == 0 {
                next_col.saturating_sub(1)
            } else {
                next_col
            };
            if width > 0 {
                next_col = next_col.saturating_add(width);
            }
            TextCell {
                ch,
                start_col,
                end_col: next_col.saturating_sub(1),
            }
        })
        .collect()
}

fn cell_index_at_column(cells: &[TextCell], col: u16) -> Option<usize> {
    cells
        .iter()
        .position(|cell| cell.start_col <= col && col <= cell.end_col)
}

fn byte_index_for_cell(row: &str, cell_idx: usize) -> usize {
    row.char_indices()
        .nth(cell_idx)
        .map(|(idx, _)| idx)
        .unwrap_or(row.len())
}

fn byte_index_after_cell(row: &str, cell_idx: usize) -> usize {
    row.char_indices()
        .nth(cell_idx.saturating_add(1))
        .map(|(idx, _)| idx)
        .unwrap_or(row.len())
}

fn url_span_at_column(cells: &[TextCell], clicked_idx: usize) -> Option<CellSpan> {
    let mut start = 0;
    while start < cells.len() {
        if starts_with_chars(&cells[start..], "http://")
            || starts_with_chars(&cells[start..], "https://")
        {
            let mut end = start;
            while end + 1 < cells.len() && !cells[end + 1].ch.is_whitespace() {
                end += 1;
            }
            if clicked_idx >= start && clicked_idx <= end {
                let span = trim_url_edges(cells, CellSpan { start, end })?;
                return span.contains(clicked_idx).then_some(span);
            }
            start = end + 1;
        } else {
            start += 1;
        }
    }
    None
}

fn trim_url_edges(cells: &[TextCell], span: CellSpan) -> Option<CellSpan> {
    let start = span.start;
    let mut end = span.end;
    while start <= end && should_trim_trailing_url_cell(cells, start, end) {
        if end == 0 {
            return None;
        }
        end -= 1;
    }
    (start <= end).then_some(CellSpan { start, end })
}

fn should_trim_trailing_url_cell(cells: &[TextCell], start: usize, end: usize) -> bool {
    match cells[end].ch {
        '"' | '\'' | '`' | '.' | ',' | ';' | ':' | '!' | '?' => true,
        ')' => !trailing_url_closer_is_balanced(cells, start, end, '(', ')'),
        ']' => !trailing_url_closer_is_balanced(cells, start, end, '[', ']'),
        '}' => !trailing_url_closer_is_balanced(cells, start, end, '{', '}'),
        _ => false,
    }
}

fn trailing_url_closer_is_balanced(
    cells: &[TextCell],
    start: usize,
    end: usize,
    open: char,
    close: char,
) -> bool {
    let mut balance = 0i32;
    for cell in &cells[start..end] {
        if cell.ch == open {
            balance += 1;
        } else if cell.ch == close {
            balance -= 1;
        }
    }
    balance > 0
}

fn quoted_path_span_at_column(cells: &[TextCell], clicked_idx: usize) -> Option<CellSpan> {
    let clicked = cells.get(clicked_idx)?.ch;
    if clicked == '"' || clicked == '\'' || clicked == '`' {
        return None;
    }

    for quote in ['"', '\'', '`'] {
        let mut start = None;
        for (idx, cell) in cells.iter().copied().enumerate() {
            let ch = cell.ch;
            if ch != quote || is_escaped(cells, idx) {
                continue;
            }
            if let Some(open) = start {
                if clicked_idx > open
                    && clicked_idx < idx
                    && cells[open + 1..idx].iter().any(|cell| cell.ch == '/')
                {
                    return Some(CellSpan {
                        start: open + 1,
                        end: idx - 1,
                    });
                }
                start = None;
            } else {
                start = Some(idx);
            }
        }
    }
    None
}

fn is_escaped(cells: &[TextCell], idx: usize) -> bool {
    let mut slashes = 0;
    let mut cursor = idx;
    while cursor > 0 && cells[cursor - 1].ch == '\\' {
        slashes += 1;
        cursor -= 1;
    }
    slashes % 2 == 1
}

fn starts_with_chars(cells: &[TextCell], prefix: &str) -> bool {
    prefix
        .chars()
        .enumerate()
        .all(|(idx, expected)| cells.get(idx).is_some_and(|cell| cell.ch == expected))
}

fn is_word_separator(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '|' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '!'
        )
}

fn trim_token_edges(cells: &[TextCell], span: CellSpan) -> Option<CellSpan> {
    let mut start = span.start;
    let mut end = span.end;
    while start <= end && is_leading_token_wrapper(cells[start].ch) {
        start += 1;
    }
    if start < end && cells[end].ch == '$' && is_trailing_token_wrapper(cells[end - 1].ch) {
        end -= 1;
    }
    while start <= end && is_trailing_token_wrapper(cells[end].ch) {
        if end == 0 {
            return None;
        }
        end -= 1;
    }
    (start <= end).then_some(CellSpan { start, end })
}

fn is_leading_token_wrapper(ch: char) -> bool {
    matches!(ch, '(' | '[' | '{' | '<' | '"' | '\'' | '`')
}

fn is_trailing_token_wrapper(ch: char) -> bool {
    matches!(
        ch,
        ')' | ']' | '}' | '>' | '"' | '\'' | '`' | '.' | ',' | ';' | ':' | '!' | '?'
    )
}

// ---------------------------------------------------------------------------
// Event handling
// ---------------------------------------------------------------------------

impl AppState {
    pub fn apply_workspace_git_statuses(
        &mut self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
        results: Vec<WorkspaceGitStatus>,
    ) -> bool {
        let mut changed = false;
        for result in results {
            let Some(ws_idx) = self
                .workspaces
                .iter()
                .position(|ws| ws.id == result.workspace_id)
            else {
                continue;
            };

            if self.workspaces[ws_idx]
                .resolved_identity_cwd_from(&self.terminals, terminal_runtimes)
                .as_ref()
                != Some(&result.resolved_identity_cwd)
            {
                continue;
            }

            let ws = &mut self.workspaces[ws_idx];
            if ws.cached_git_branch != result.branch {
                ws.cached_git_branch = result.branch;
                changed = true;
            }
            if ws.cached_git_ahead_behind != result.ahead_behind {
                ws.cached_git_ahead_behind = result.ahead_behind;
                changed = true;
            }
            if ws.cached_git_space != result.space {
                ws.cached_git_space = result.space;
                changed = true;
            }
        }
        changed
    }

    pub fn handle_app_event(&mut self, event: AppEvent) -> Vec<PaneStateUpdate> {
        match event {
            AppEvent::PaneDied { pane_id } => {
                self.handle_pane_died(pane_id);
                Vec::new()
            }
            AppEvent::UpdateReady {
                version,
                install_command,
            } => {
                self.update_available = Some(version.clone());
                self.update_install_command = install_command.clone();
                self.latest_release_notes_available = true;
                self.update_dismissed = true;
                if matches!(
                    self.toast_config.delivery,
                    crate::config::ToastDelivery::Herdr
                ) {
                    self.toast = Some(ToastNotification {
                        kind: ToastKind::UpdateInstalled,
                        title: format!("v{version} available"),
                        context: crate::update::update_install_instruction(&install_command),
                        position: None,
                        target: None,
                    });
                }
                Vec::new()
            }
            AppEvent::AgentDetectionManifestsUpdated { updated, status } => {
                self.agent_manifest_update_status = status;
                self.refresh_agent_manifest_summaries();
                if !updated.is_empty()
                    && matches!(
                        self.toast_config.delivery,
                        crate::config::ToastDelivery::Herdr
                    )
                {
                    let agent_list = updated
                        .iter()
                        .map(|item| {
                            format!(
                                "{} {}",
                                crate::detect::agent_label(item.agent),
                                item.version
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.toast = Some(ToastNotification {
                        kind: ToastKind::UpdateInstalled,
                        title: "Agent detection rules updated".to_string(),
                        context: agent_list,
                        position: None,
                        target: None,
                    });
                }
                Vec::new()
            }
            AppEvent::StateChanged {
                pane_id,
                agent,
                state,
                visible_blocker,
                visible_working,
                process_exited,
                observed_at,
            } => self
                .update_terminal_state(pane_id, |terminal| {
                    Some(terminal.set_detected_state_with_screen_signals_at(
                        agent,
                        state,
                        visible_blocker,
                        false,
                        visible_working,
                        process_exited,
                        observed_at,
                    ))
                })
                .into_iter()
                .collect(),
            AppEvent::HookStateReported {
                pane_id,
                source,
                agent_label,
                state,
                message,
                custom_status,
                seq,
                session_ref,
            } => {
                if crate::agent_resume::is_reserved_native_state_source(&source, &agent_label) {
                    self.update_terminal_state(pane_id, |terminal| {
                        terminal.set_agent_session_ref(source, agent_label, session_ref, seq)
                    })
                    .into_iter()
                    .collect()
                } else {
                    self.update_terminal_state(pane_id, |terminal| {
                        terminal.set_hook_authority_with_session_ref(
                            source,
                            agent_label,
                            state,
                            message,
                            custom_status,
                            session_ref,
                            seq,
                        )
                    })
                    .into_iter()
                    .collect()
                }
            }
            AppEvent::AgentSessionReported {
                pane_id,
                source,
                agent_label,
                seq,
                session_ref,
                session_start_source,
            } => self
                .update_terminal_state(pane_id, |terminal| {
                    terminal.set_agent_session_ref_for_session_start(
                        source,
                        agent_label,
                        session_ref,
                        seq,
                        session_start_source,
                    )
                })
                .into_iter()
                .collect(),
            AppEvent::HookMetadataReported {
                pane_id,
                source,
                agent_label,
                applies_to_source,
                title,
                display_agent,
                custom_status,
                state_labels,
                clear_title,
                clear_display_agent,
                clear_custom_status,
                clear_state_labels,
                seq,
                ttl,
            } => self
                .update_terminal_state(pane_id, |terminal| {
                    terminal.set_agent_metadata(crate::terminal::AgentMetadataReport {
                        source,
                        agent_label,
                        applies_to_source,
                        title,
                        display_agent,
                        custom_status,
                        state_labels,
                        clear_title,
                        clear_display_agent,
                        clear_custom_status,
                        clear_state_labels,
                        ttl,
                        seq,
                    })
                })
                .into_iter()
                .collect(),
            AppEvent::HookAuthorityCleared {
                pane_id,
                source,
                seq,
            } => self
                .update_terminal_state(pane_id, |terminal| {
                    terminal.clear_hook_authority_with_mutation(source.as_deref(), seq)
                })
                .into_iter()
                .collect(),
            AppEvent::HookAgentReleased {
                pane_id,
                source,
                agent_label,
                seq,
                ..
            } => {
                if crate::agent_resume::is_reserved_native_state_source(&source, &agent_label) {
                    Vec::new()
                } else {
                    self.update_terminal_state(pane_id, |terminal| {
                        terminal.release_agent_with_mutation(&source, &agent_label, seq)
                    })
                    .into_iter()
                    .collect()
                }
            }
            // Intercepted in App::handle_internal_event before reaching this
            // dispatch; never touches AppState.
            AppEvent::ClipboardWrite { .. } => Vec::new(),
            AppEvent::TerminalCwdReported { pane_id, cwd } => {
                if !cwd.is_absolute() || !cwd.is_dir() {
                    return Vec::new();
                }
                let Some(terminal_id) = self.workspaces.iter().find_map(|ws| {
                    ws.pane_state(pane_id)
                        .map(|pane| pane.attached_terminal_id.clone())
                }) else {
                    return Vec::new();
                };
                let Some(terminal) = self.terminals.get_mut(&terminal_id) else {
                    return Vec::new();
                };
                if terminal.cwd != cwd {
                    terminal.cwd = cwd;
                    self.mark_session_dirty();
                }
                Vec::new()
            }
            AppEvent::GitStatusRefreshed {
                results,
                cache_updates,
            } => {
                let _ = results;
                let _ = cache_updates;
                Vec::new()
            }
            AppEvent::WorktreeAddFinished(_) => Vec::new(),
            AppEvent::WorktreeRemoveFinished(_) => Vec::new(),
            AppEvent::PluginCommandFinished { .. } => Vec::new(),
        }
    }

    fn update_terminal_state<F>(&mut self, pane_id: PaneId, update: F) -> Option<PaneStateUpdate>
    where
        F: FnOnce(&mut crate::terminal::TerminalState) -> Option<TerminalStateMutation>,
    {
        let ws_idx = self
            .workspaces
            .iter()
            .position(|ws| ws.pane_state(pane_id).is_some())?;
        let terminal_id = self.workspaces[ws_idx]
            .pane_state(pane_id)?
            .attached_terminal_id
            .clone();
        let previous_seen = self.workspaces[ws_idx].pane_state(pane_id)?.seen;
        let mutation = {
            let terminal = self.terminals.get_mut(&terminal_id)?;
            update(terminal)?
        };
        if mutation.session_ref_changed {
            self.mark_session_dirty();
        }
        let change = mutation.effective_state_change?;
        if change.previous_state != change.state {
            self.next_agent_state_change_seq += 1;
            if let Some(terminal) = self.terminals.get_mut(&terminal_id) {
                terminal.last_agent_state_change_seq = Some(self.next_agent_state_change_seq);
            }
        }
        let seen = self.apply_pane_state_change(ws_idx, pane_id, &change)?;
        let update = PaneStateUpdate {
            pane_id,
            ws_idx,
            previous_agent_label: change.previous_agent_label.clone(),
            previous_known_agent: change.previous_known_agent,
            previous_state: change.previous_state,
            previous_seen,
            previous_presentation: change.previous_presentation.clone(),
            agent_label: change.agent_label.clone(),
            known_agent: change.known_agent,
            state: change.state,
            seen,
            presentation: change.presentation.clone(),
        };
        Some(update)
    }

    pub(crate) fn publish_pane_process_exit_if_agent(
        &mut self,
        pane_id: PaneId,
    ) -> Option<PaneStateUpdate> {
        let observed_at = std::time::Instant::now();
        self.update_terminal_state(pane_id, |terminal| {
            let agent = terminal.effective_known_agent().or(terminal.detected_agent);
            if agent.is_none() && !terminal.full_lifecycle_hook_authority_active() {
                return None;
            }
            Some(terminal.set_detected_state_with_screen_signals_at(
                agent,
                AgentState::Idle,
                false,
                true,
                false,
                true,
                observed_at,
            ))
        })
    }

    fn apply_pane_state_change(
        &mut self,
        ws_idx: usize,
        pane_id: PaneId,
        change: &EffectiveStateChange,
    ) -> Option<bool> {
        let is_active_tab = self.pane_is_in_active_tab(ws_idx, pane_id);
        let suppress_active_tab_notifications =
            active_tab_suppresses_notifications(is_active_tab, self.outer_terminal_focus);
        let pane = self.workspaces[ws_idx]
            .tabs
            .iter_mut()
            .find_map(|tab| tab.panes.get_mut(&pane_id))?;

        if change.state != AgentState::Idle {
            pane.seen = true;
        } else if is_completion_transition(change) {
            pane.seen = suppress_active_tab_notifications;
        }
        let seen = pane.seen;

        if let Some(delivery) = self.record_or_deliver_agent_notification(ws_idx, pane_id, change) {
            self.apply_agent_notification_delivery(&delivery);
        }

        Some(seen)
    }

    fn record_or_deliver_agent_notification(
        &mut self,
        ws_idx: usize,
        pane_id: PaneId,
        change: &EffectiveStateChange,
    ) -> Option<AgentNotificationDelivery> {
        self.pending_agent_notifications.remove(&pane_id);

        let is_active_tab = self.pane_is_in_active_tab(ws_idx, pane_id);
        let suppress_active_tab_notifications =
            active_tab_suppresses_notifications(is_active_tab, self.outer_terminal_focus);

        let client_notification_kind = notification_toast_for_effective_state_change(
            suppress_active_tab_notifications,
            change,
        );
        let sound = notification_sound_for_effective_state_change(
            suppress_active_tab_notifications,
            change,
        );
        if client_notification_kind.is_none() && sound.is_none() {
            return None;
        }

        let agent_label = change.agent_label.clone()?;
        let kind = client_notification_kind.unwrap_or(match sound {
            Some(crate::sound::Sound::Request) => ToastKind::NeedsAttention,
            Some(crate::sound::Sound::Done) | None => ToastKind::Finished,
        });
        let workspace_id = self.workspaces[ws_idx].id.clone();

        if self.toast_config.delay_seconds == 0 {
            return self.agent_notification_delivery(
                ws_idx,
                pane_id,
                workspace_id,
                agent_label,
                change.known_agent,
                kind,
                change.state,
            );
        }

        self.pending_agent_notifications.insert(
            pane_id,
            PendingAgentNotification {
                pane_id,
                workspace_id,
                agent_label,
                known_agent: change.known_agent,
                kind,
                state: change.state,
                deadline: {
                    let now = std::time::Instant::now();
                    let delay_seconds = self
                        .toast_config
                        .delay_seconds
                        .min(crate::config::MAX_TOAST_DELAY_SECONDS);
                    now.checked_add(std::time::Duration::from_secs(delay_seconds))
                        .unwrap_or(now)
                },
            },
        );
        None
    }

    fn agent_notification_delivery(
        &self,
        ws_idx: usize,
        pane_id: PaneId,
        workspace_id: String,
        agent_label: String,
        known_agent: Option<Agent>,
        kind: ToastKind,
        expected_state: AgentState,
    ) -> Option<AgentNotificationDelivery> {
        let terminal_state = self
            .workspaces
            .get(ws_idx)?
            .pane_state(pane_id)
            .and_then(|pane| self.terminals.get(&pane.attached_terminal_id))?;
        if terminal_state.state != expected_state {
            return None;
        }
        if terminal_state.effective_agent_label() != Some(agent_label.as_str()) {
            return None;
        }

        let is_active_tab = self.pane_is_in_active_tab(ws_idx, pane_id);
        let suppress_active_tab_notifications =
            active_tab_suppresses_notifications(is_active_tab, self.outer_terminal_focus);
        let sound = sound_for_toast_kind(kind, suppress_active_tab_notifications)
            .filter(|_| self.sound.allows(known_agent));
        let build_toast = || {
            let workspace_label = self.workspaces[ws_idx].display_name();
            let context =
                notification_context(&self.workspaces[ws_idx], &workspace_label, ws_idx, pane_id);
            ToastNotification {
                kind,
                title: format!(
                    "{} {}",
                    toast_agent_label(&agent_label),
                    toast_event_text(kind)
                ),
                context,
                position: None,
                target: Some(ToastTarget {
                    workspace_id: workspace_id.clone(),
                    pane_id,
                }),
            }
        };
        let toast = (!is_active_tab).then(build_toast);
        let client_notification = (!suppress_active_tab_notifications).then(build_toast);

        if toast.is_none() && client_notification.is_none() && sound.is_none() {
            return None;
        }

        Some(AgentNotificationDelivery {
            pane_id,
            workspace_id,
            agent_label,
            known_agent,
            kind,
            toast,
            client_notification,
            sound,
        })
    }

    fn apply_agent_notification_delivery(&mut self, delivery: &AgentNotificationDelivery) {
        if self.local_sound_playback {
            if let Some(sound) = delivery.sound {
                crate::sound::play(sound, &self.sound);
            }
        }

        if matches!(
            self.toast_config.delivery,
            crate::config::ToastDelivery::Herdr
        ) {
            if let Some(toast) = delivery.toast.clone() {
                self.toast = Some(toast);
            }
        }
    }

    pub fn next_pending_agent_notification_deadline(&self) -> Option<std::time::Instant> {
        self.pending_agent_notifications
            .values()
            .map(|pending| pending.deadline)
            .min()
    }

    pub fn drain_due_agent_notifications(
        &mut self,
        now: std::time::Instant,
    ) -> Vec<AgentNotificationDelivery> {
        let due_panes: Vec<PaneId> = self
            .pending_agent_notifications
            .iter()
            .filter_map(|(&pane_id, pending)| (pending.deadline <= now).then_some(pane_id))
            .collect();
        let mut deliveries = Vec::new();

        for pane_id in due_panes {
            let Some(pending) = self.pending_agent_notifications.remove(&pane_id) else {
                continue;
            };
            let Some(ws_idx) = self
                .workspaces
                .iter()
                .position(|ws| ws.id == pending.workspace_id)
            else {
                continue;
            };
            let Some(delivery) = self.agent_notification_delivery(
                ws_idx,
                pending.pane_id,
                pending.workspace_id,
                pending.agent_label,
                pending.known_agent,
                pending.kind,
                pending.state,
            ) else {
                continue;
            };
            self.apply_agent_notification_delivery(&delivery);
            deliveries.push(delivery);
        }

        deliveries
    }

    fn handle_pane_died(&mut self, pane_id: PaneId) {
        self.pending_agent_notifications.remove(&pane_id);
        self.plugin_panes.remove(&pane_id);
        let ws_idx = self
            .workspaces
            .iter()
            .position(|ws| ws.find_tab_index_for_pane(pane_id).is_some());

        let Some(ws_idx) = ws_idx else {
            warn!(pane = pane_id.raw(), "PaneDied for unknown pane");
            return;
        };

        if self
            .selection
            .as_ref()
            .is_some_and(|s| s.pane_id == pane_id)
        {
            self.selection = None;
            self.selection_autoscroll = None;
        }

        let pane_terminal_id = self.terminal_id_for_pane(ws_idx, pane_id);
        let workspace_terminal_ids = self.terminal_ids_for_workspace(ws_idx);
        self.pane_id_aliases.retain(|_, alias| *alias != pane_id);
        self.public_pane_id_aliases
            .retain(|_, alias| *alias != pane_id);
        let should_close_workspace = {
            let ws = &mut self.workspaces[ws_idx];
            ws.remove_pane(pane_id)
        };
        self.mark_session_dirty();

        if should_close_workspace {
            self.workspaces.remove(ws_idx);
            self.remove_unattached_terminal_ids(workspace_terminal_ids);
            if self.workspaces.is_empty() {
                self.active = None;
                self.selected = 0;
                if self.mode == Mode::Terminal {
                    self.mode = Mode::Navigate;
                }
            } else {
                if let Some(active) = self.active {
                    if active >= self.workspaces.len() {
                        self.active = Some(self.workspaces.len() - 1);
                    }
                }
                if self.selected >= self.workspaces.len() {
                    self.selected = self.workspaces.len() - 1;
                }
            }
        } else {
            self.remove_unattached_terminal_ids(pane_terminal_id);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{AgentReparentAction, ManualEntryRef};
    use crate::detect::{Agent, AgentState};
    use crate::workspace::Workspace;
    use ratatui::layout::Direction;

    fn app_with_workspaces(names: &[&str]) -> AppState {
        let mut state = AppState::test_new();
        state.toast_config.delay_seconds = 0;
        for name in names {
            let ws = Workspace::test_new(name);
            state.workspaces.push(ws);
        }
        state.ensure_test_terminals();
        if !state.workspaces.is_empty() {
            state.active = Some(0);
            state.mode = Mode::Terminal;
        }
        state
    }

    fn mark_linked_worktree(state: &mut AppState, ws_idx: usize) {
        state.workspaces[ws_idx].worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
            key: "repo-key".into(),
            label: "herdr".into(),
            repo_root: "/repo/herdr".into(),
            checkout_path: format!("/repo/worktree-{ws_idx}").into(),
            is_linked_worktree: true,
        });
    }

    fn mark_parent_worktree(state: &mut AppState, ws_idx: usize) {
        state.workspaces[ws_idx].worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
            key: "repo-key".into(),
            label: "herdr".into(),
            repo_root: "/repo/herdr".into(),
            checkout_path: "/repo/herdr".into(),
            is_linked_worktree: false,
        });
    }

    #[test]
    fn notification_context_formats_resolved_workspace_label() {
        let state = app_with_workspaces(&["stale"]);
        let root = state.workspaces[0].tabs[0].root_pane;

        assert_eq!(
            notification_context(&state.workspaces[0], "__herdr_projects__", 0, root),
            "__herdr_projects__ · 1"
        );
    }

    fn selected_word(row: &str, col: u16) -> Option<String> {
        let (start, end) = word_bounds_at_column(row, col)?;
        Some(text_in_cell_range(row, start, end))
    }

    fn selected_url<'a>(row: &'a str, click: &str) -> Option<&'a str> {
        url_at_column(row, col_of(row, click))
    }

    fn text_in_cell_range(row: &str, start_col: u16, end_col: u16) -> String {
        text_cells(row)
            .into_iter()
            .filter(|cell| cell.start_col >= start_col && cell.end_col <= end_col)
            .map(|cell| cell.ch)
            .collect()
    }

    fn col_of(row: &str, needle: &str) -> u16 {
        let byte_idx = row
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not found in {row:?}"));
        let prefix = &row[..byte_idx];
        prefix
            .chars()
            .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(0) as u16)
            .sum()
    }

    fn assert_selects(row: &str, click: &str, expected: &str) {
        assert_eq!(
            selected_word(row, col_of(row, click)).as_deref(),
            Some(expected),
            "row={row:?}, click={click:?}"
        );
    }

    fn assert_selects_nothing(row: &str, click: &str) {
        assert_eq!(
            selected_word(row, col_of(row, click)),
            None,
            "row={row:?}, click={click:?}"
        );
    }

    #[test]
    fn double_click_word_bounds_cover_terminal_text() {
        let cases = [
            (
                "see https://example.com/a-b_c?q=x@y.",
                "example.com",
                "https://example.com/a-b_c?q=x@y",
            ),
            (
                "open \"https://example.com/a,b;c?q=x\";",
                "example.com",
                "https://example.com/a,b;c?q=x",
            ),
            (
                "see https://en.wikipedia.org/wiki/Foo_(bar_(baz)),",
                "wikipedia",
                "https://en.wikipedia.org/wiki/Foo_(bar_(baz))",
            ),
            (
                "see https://example.com/a(b[c{d}e]f),",
                "example.com",
                "https://example.com/a(b[c{d}e]f)",
            ),
            (
                "see (https://example.com/a(b(c)d)))",
                "example.com",
                "https://example.com/a(b(c)d)",
            ),
            (
                "open /tmp/foo-bar/baz_qux/",
                "foo-bar",
                "/tmp/foo-bar/baz_qux/",
            ),
            (
                "open ./src/app/actions.rs:795",
                "actions",
                "./src/app/actions.rs:795",
            ),
            (
                "open ../herdr-worktrees/issue-1",
                "herdr",
                "../herdr-worktrees/issue-1",
            ),
            (
                "edit src/app/actions.rs,then",
                "actions",
                "src/app/actions.rs",
            ),
            (
                "cat \"/tmp/build output/log.txt\"",
                "output",
                "/tmp/build output/log.txt",
            ),
            (
                "cat '/Users/me/Library/Application Support/app/config.json'",
                "Support",
                "/Users/me/Library/Application Support/app/config.json",
            ),
            ("echo 你好-world done", "好", "你好-world"),
            ("先跑 cargo test", "cargo", "cargo"),
            (
                "export PATH=$HOME/.cargo/bin:$PATH",
                "$HOME",
                "PATH=$HOME/.cargo/bin:$PATH",
            ),
            (
                "git checkout feature/foo-bar_baz",
                "foo",
                "feature/foo-bar_baz",
            ),
            ("refs #123 and @owner/name", "#123", "#123"),
            ("refs #123 and @owner/name", "owner", "@owner/name"),
            ("cargo test --package=herdr", "--package", "--package=herdr"),
            (
                "cargo test app::actions::tests",
                "app::",
                "app::actions::tests",
            ),
            (
                "image ghcr.io/org/app:latest",
                "ghcr",
                "ghcr.io/org/app:latest",
            ),
            ("ERROR [worker-1] request_id=abc-123", "worker", "worker-1"),
            (
                "tmux|newhoo|fixhoo|newmoo|notification|window_bell|herdr",
                "newhoo",
                "newhoo",
            ),
            (
                "render_status_line(app, area)",
                "render",
                "render_status_line",
            ),
            ("render_status_line(app, area)", "app", "app"),
            ("render_status_line(app, area)", "area", "area"),
            ("if !enabled {", "enabled", "enabled"),
            ("println!(\"hi\")", "println", "println"),
            ("( master)$", "master", "master"),
            ("regex foo$", "foo", "foo$"),
        ];

        for (row, click, expected) in cases {
            assert_selects(row, click, expected);
        }

        let row = "echo 你好-world done";
        assert_eq!(
            selected_word(row, col_of(row, "好") + 1).as_deref(),
            Some("你好-world")
        );
    }

    #[test]
    fn double_click_word_bounds_ignore_delimiters() {
        for (row, click) in [
            (
                "tmux|newhoo|fixhoo|newmoo|notification|window_bell|herdr",
                "|",
            ),
            ("alpha,beta;gamma", ","),
            ("alpha,beta;gamma", ";"),
            ("render_status_line(app, area)", "("),
            ("render_status_line(app, area)", ")"),
            ("if !enabled {", "!"),
            ("if !enabled {", "{"),
            ("(done).", "("),
            ("(done).", "."),
        ] {
            assert_selects_nothing(row, click);
        }
    }

    #[test]
    fn url_at_column_returns_safe_visible_url_only() {
        assert_eq!(
            selected_url("see https://example.com/a(b)c.", "example"),
            Some("https://example.com/a(b)c")
        );
        assert_eq!(
            selected_url("[docs](https://example.com/docs),", "example"),
            Some("https://example.com/docs")
        );
        assert_eq!(
            selected_url("[docs](https://example.com/docs)", "docs"),
            None
        );
        assert_eq!(selected_url("open file:///tmp/report", "file"), None);
    }

    #[test]
    fn navigator_rows_show_tab_nodes_only_for_multi_tab_workspaces() {
        let mut state = app_with_workspaces(&["single", "multi"]);
        state.workspaces[1].test_add_tab(Some("tests"));
        state.ensure_test_terminals();

        state.open_navigator();
        let rows = state.navigator_rows();

        assert!(!rows.iter().any(|row| matches!(
            row.target,
            crate::app::state::NavigatorTarget::Tab { ws_idx: 0, .. }
        )));
        assert!(rows.iter().any(|row| matches!(
            row.target,
            crate::app::state::NavigatorTarget::Tab {
                ws_idx: 1,
                tab_idx: 0
            }
        )));
        assert!(rows.iter().any(|row| matches!(
            row.target,
            crate::app::state::NavigatorTarget::Tab {
                ws_idx: 1,
                tab_idx: 1
            }
        )));
    }

    #[tokio::test]
    async fn navigator_rows_match_live_root_runtime_cwd_workspace_label() {
        let unique = format!(
            "herdr-navigator-runtime-cwd-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let stale_cwd = root.join("issue-264-nix-support");
        let live_cwd = root.join("herdr");
        std::fs::create_dir_all(stale_cwd.join(".git")).unwrap();
        std::fs::create_dir_all(live_cwd.join(".git")).unwrap();

        let mut state = AppState::test_new();
        let mut workspace = Workspace::test_new("stale-name");
        workspace.custom_name = None;
        workspace.identity_cwd = stale_cwd.clone();
        let pane = workspace.tabs[0].root_pane;
        state.workspaces = vec![workspace];
        state.ensure_test_terminals();
        let terminal_id = state.workspaces[0].terminal_id(pane).cloned().unwrap();
        state.terminals.get_mut(&terminal_id).unwrap().cwd = stale_cwd;

        let (events, _) = tokio::sync::mpsc::channel(4);
        let runtime = crate::terminal::TerminalRuntime::spawn(
            pane,
            24,
            80,
            live_cwd.clone(),
            0,
            crate::terminal_theme::TerminalTheme::default(),
            crate::pane::PaneShellConfig::new("/bin/sh", crate::config::ShellModeConfig::NonLogin),
            &crate::pane::PaneLaunchEnv::default(),
            events,
            std::sync::Arc::new(tokio::sync::Notify::new()),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while runtime.cwd() != Some(live_cwd.clone()) && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let mut runtime_registry = crate::terminal::TerminalRuntimeRegistry::new();
        runtime_registry.insert(terminal_id, runtime);
        state.open_navigator_from(&runtime_registry);
        state.navigator.query = "herdr".into();
        let rows = state.navigator_rows_from(&runtime_registry);

        for (_, runtime) in runtime_registry.drain() {
            runtime.shutdown();
        }
        let _ = std::fs::remove_dir_all(root);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "herdr (1)");
    }

    #[test]
    fn navigator_rows_include_shell_and_agent_panes() {
        let mut state = app_with_workspaces(&["one"]);
        let shell = state.workspaces[0].tabs[0].root_pane;
        let agent = state.workspaces[0].test_split(Direction::Horizontal);
        state.ensure_test_terminals();

        let agent_terminal_id = state.workspaces[0].terminal_id(agent).cloned().unwrap();
        let terminal = state.terminals.get_mut(&agent_terminal_id).unwrap();
        terminal.set_detected_state(Some(Agent::Claude), AgentState::Working);

        state.open_navigator();
        let rows = state.navigator_rows();

        assert!(rows.iter().any(|row| matches!(
            row.target,
            crate::app::state::NavigatorTarget::Pane { pane_id, .. } if pane_id == shell
        )));
        assert!(rows.iter().any(|row| matches!(
            row.target,
            crate::app::state::NavigatorTarget::Pane { pane_id, .. } if pane_id == agent
        ) && row.meta.contains("claude")));
    }

    #[test]
    fn opening_navigator_selects_current_pane_and_expands_attention_workspaces() {
        let mut state = app_with_workspaces(&["one", "two"]);
        let blocked = state.workspaces[1].tabs[0].root_pane;
        let blocked_terminal_id = state.workspaces[1].terminal_id(blocked).cloned().unwrap();
        state
            .terminals
            .get_mut(&blocked_terminal_id)
            .unwrap()
            .set_detected_state(Some(Agent::Codex), AgentState::Blocked);

        state.open_navigator();
        let selected = state.navigator_rows()[state.navigator.selected].clone();

        assert!(selected.is_current);
        assert!(state
            .navigator
            .expanded_workspaces
            .contains(&state.workspaces[0].id));
        assert!(state
            .navigator
            .expanded_workspaces
            .contains(&state.workspaces[1].id));
    }

    #[test]
    fn accepting_navigator_pane_switches_workspace_tab_and_focus() {
        let mut state = app_with_workspaces(&["one", "two"]);
        let target = state.workspaces[1].tabs[0].root_pane;
        state.open_navigator();
        state
            .navigator
            .expanded_workspaces
            .insert(state.workspaces[1].id.clone());
        state.navigator.selected = state
            .navigator_rows()
            .iter()
            .position(|row| {
                matches!(
                    row.target,
                    crate::app::state::NavigatorTarget::Pane { pane_id, .. } if pane_id == target
                )
            })
            .unwrap();

        assert!(state.accept_navigator_selection());

        assert_eq!(state.active, Some(1));
        assert_eq!(state.workspaces[1].focused_pane_id(), Some(target));
        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn navigator_idle_search_matches_idle_agents_not_plain_shells() {
        let mut state = app_with_workspaces(&["one"]);
        let shell = state.workspaces[0].tabs[0].root_pane;
        let agent = state.workspaces[0].test_split(Direction::Horizontal);
        state.ensure_test_terminals();

        let agent_terminal_id = state.workspaces[0].terminal_id(agent).cloned().unwrap();
        state
            .terminals
            .get_mut(&agent_terminal_id)
            .unwrap()
            .set_detected_state(Some(Agent::Claude), AgentState::Idle);

        state.open_navigator();
        state.navigator.query = "idle".into();
        let rows = state.navigator_rows();

        assert!(rows.iter().any(|row| matches!(
            row.target,
            crate::app::state::NavigatorTarget::Pane { pane_id, .. } if pane_id == agent
        )));
        assert!(!rows.iter().any(|row| matches!(
            row.target,
            crate::app::state::NavigatorTarget::Pane { pane_id, .. } if pane_id == shell
        )));
    }

    #[test]
    fn navigator_search_only_matches_visible_row_text() {
        let mut state = app_with_workspaces(&["one"]);
        state.workspaces[0].identity_cwd = "/tmp/herdr-worktrees/issue-work".into();

        state.open_navigator();
        state.navigator.query = "work".into();

        assert!(state.navigator_rows().is_empty());
    }

    #[test]
    fn navigator_state_filter_is_separate_from_text_search() {
        let mut state = app_with_workspaces(&["one"]);
        let shell = state.workspaces[0].tabs[0].root_pane;
        let working = state.workspaces[0].test_split(Direction::Horizontal);
        state.ensure_test_terminals();

        let shell_terminal_id = state.workspaces[0].terminal_id(shell).cloned().unwrap();
        state
            .terminals
            .get_mut(&shell_terminal_id)
            .unwrap()
            .set_manual_label("wheel notes".into());
        let working_terminal_id = state.workspaces[0].terminal_id(working).cloned().unwrap();
        state
            .terminals
            .get_mut(&working_terminal_id)
            .unwrap()
            .set_detected_state(Some(Agent::Codex), AgentState::Working);

        state.open_navigator();
        state.navigator.state_filter = Some(NavigatorStateFilter::Working);
        let state_rows = state.navigator_rows();

        assert!(state_rows.iter().any(|row| matches!(
            row.target,
            crate::app::state::NavigatorTarget::Pane { pane_id, .. } if pane_id == working
        )));
        assert!(!state_rows.iter().any(|row| matches!(
            row.target,
            crate::app::state::NavigatorTarget::Pane { pane_id, .. } if pane_id == shell
        )));

        state.navigator.state_filter = None;
        state.navigator.query = "w".into();
        let text_rows = state.navigator_rows();

        assert!(text_rows.iter().any(|row| matches!(
            row.target,
            crate::app::state::NavigatorTarget::Pane { pane_id, .. } if pane_id == shell
        )));
        assert!(
            text_rows.iter().any(|row| matches!(
                row.target,
                crate::app::state::NavigatorTarget::Pane { pane_id, .. } if pane_id == working
            )),
            "literal one-letter search may still match visible state text"
        );
    }

    #[test]
    fn navigator_search_filters_panes_but_keeps_workspace_context() {
        let mut state = app_with_workspaces(&["one"]);
        let root = state.workspaces[0].tabs[0].root_pane;
        let terminal_id = state.workspaces[0].terminal_id(root).cloned().unwrap();
        state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .set_manual_label("weekly review".into());
        state.open_navigator();
        state.navigator.query = "weekly".into();

        let rows = state.navigator_rows();

        assert!(rows.iter().any(|row| row.is_workspace));
        assert!(rows
            .iter()
            .any(|row| !row.is_workspace && row.label.contains("weekly")));
    }

    #[test]
    fn apply_workspace_git_statuses_updates_matching_workspace() {
        let mut state = app_with_workspaces(&["one", "two"]);
        let first_id = state.workspaces[0].id.clone();
        let first_cwd = state.workspaces[0].resolved_identity_cwd().unwrap();
        let second_id = state.workspaces[1].id.clone();

        let terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();
        let changed = state.apply_workspace_git_statuses(
            &terminal_runtimes,
            vec![WorkspaceGitStatus {
                workspace_id: first_id,
                resolved_identity_cwd: first_cwd,
                branch: Some("main".into()),
                ahead_behind: Some((2, 1)),
                space: None,
            }],
        );

        assert!(changed);
        assert_eq!(state.workspaces[0].branch().as_deref(), Some("main"));
        assert_eq!(state.workspaces[0].git_ahead_behind(), Some((2, 1)));
        assert_eq!(state.workspaces[1].id, second_id);
        assert_eq!(state.workspaces[1].git_ahead_behind(), None);
    }

    #[test]
    fn apply_workspace_git_statuses_ignores_stale_cwd() {
        let mut state = app_with_workspaces(&["one"]);
        let workspace_id = state.workspaces[0].id.clone();
        state.workspaces[0].cached_git_branch = Some("old".into());
        state.workspaces[0].cached_git_ahead_behind = Some((1, 0));

        let terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();
        let changed = state.apply_workspace_git_statuses(
            &terminal_runtimes,
            vec![WorkspaceGitStatus {
                workspace_id,
                resolved_identity_cwd: std::path::PathBuf::from("/definitely/not/current"),
                branch: Some("main".into()),
                ahead_behind: Some((0, 1)),
                space: None,
            }],
        );

        assert!(!changed);
        assert_eq!(state.workspaces[0].branch().as_deref(), Some("old"));
        assert_eq!(state.workspaces[0].git_ahead_behind(), Some((1, 0)));
    }

    #[test]
    fn apply_workspace_git_statuses_clears_missing_git_status() {
        let mut state = app_with_workspaces(&["one"]);
        let workspace_id = state.workspaces[0].id.clone();
        let cwd = state.workspaces[0].resolved_identity_cwd().unwrap();
        state.workspaces[0].cached_git_branch = Some("main".into());
        state.workspaces[0].cached_git_ahead_behind = Some((1, 2));

        let terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();
        let changed = state.apply_workspace_git_statuses(
            &terminal_runtimes,
            vec![WorkspaceGitStatus {
                workspace_id,
                resolved_identity_cwd: cwd,
                branch: None,
                ahead_behind: None,
                space: None,
            }],
        );

        assert!(changed);
        assert_eq!(state.workspaces[0].branch(), None);
        assert_eq!(state.workspaces[0].git_ahead_behind(), None);
    }

    #[test]
    fn apply_workspace_git_statuses_does_not_change_worktree_membership() {
        let mut state = app_with_workspaces(&["one"]);
        mark_linked_worktree(&mut state, 0);
        let workspace_id = state.workspaces[0].id.clone();
        let cwd = state.workspaces[0].resolved_identity_cwd().unwrap();
        let membership = state.workspaces[0].worktree_space().cloned();

        let terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();
        let changed = state.apply_workspace_git_statuses(
            &terminal_runtimes,
            vec![WorkspaceGitStatus {
                workspace_id,
                resolved_identity_cwd: cwd,
                branch: Some("scratch".into()),
                ahead_behind: None,
                space: Some(crate::workspace::GitSpaceMetadata {
                    key: "other-repo-key".into(),
                    checkout_key: "/other/checkout".into(),
                    label: "other".into(),
                    repo_root: "/other/repo".into(),
                    is_linked_worktree: false,
                }),
            }],
        );

        assert!(changed);
        assert_eq!(state.workspaces[0].worktree_space().cloned(), membership);
    }

    fn mark_agent(state: &mut AppState, ws_idx: usize, tab_idx: usize, pane_id: PaneId) {
        set_agent_state(state, ws_idx, tab_idx, pane_id, AgentState::Idle);
    }

    fn set_agent_state(
        state: &mut AppState,
        ws_idx: usize,
        tab_idx: usize,
        pane_id: PaneId,
        agent_state: AgentState,
    ) {
        state.ensure_test_terminals();
        let terminal_id = state.workspaces[ws_idx].tabs[tab_idx]
            .panes
            .get(&pane_id)
            .unwrap()
            .attached_terminal_id
            .clone();
        if let Some(terminal) = state.terminals.get_mut(&terminal_id) {
            terminal.set_detected_state(Some(Agent::Pi), agent_state);
        }
    }

    fn transition_agent_state(state: &mut AppState, pane_id: PaneId, agent_state: AgentState) {
        state
            .update_terminal_state(pane_id, |terminal| {
                Some(terminal.set_detected_state_with_screen_signals_at(
                    Some(Agent::Pi),
                    agent_state,
                    matches!(agent_state, AgentState::Blocked),
                    false,
                    false,
                    false,
                    std::time::Instant::now(),
                ))
            })
            .expect("agent state transition should update pane state");
    }

    #[test]
    fn next_agent_cycles_agent_panel_entries() {
        let mut first = Workspace::test_new("one");
        let first_root = first.tabs[0].root_pane;
        let first_second = first.test_split(Direction::Horizontal);
        first.tabs[0].layout.focus_pane(first_root);
        let second = Workspace::test_new("two");
        let second_root = second.tabs[0].root_pane;

        let mut state = AppState::test_new();
        state.workspaces = vec![first, second];
        state.ensure_test_terminals();
        state.active = Some(0);
        state.selected = 0;
        state.mode = Mode::Terminal;
        mark_agent(&mut state, 0, 0, first_root);
        mark_agent(&mut state, 0, 0, first_second);
        mark_agent(&mut state, 1, 0, second_root);

        state.next_agent();
        assert_eq!(state.active, Some(0));
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(first_second));

        state.next_agent();
        assert_eq!(state.active, Some(1));
        assert_eq!(state.workspaces[1].focused_pane_id(), Some(second_root));

        state.previous_agent();
        assert_eq!(state.active, Some(0));
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(first_second));
        state.assert_invariants_for_test();
    }

    /// Link `child` to `parent` by the stable (workspace id + public pane number)
    /// reference, exactly as `agent start --parent` does.
    fn link_agent_parent(state: &mut AppState, parent: PaneId, child: PaneId) {
        let ws = &mut state.workspaces[0];
        let number = ws.public_pane_number(parent).expect("parent has number");
        let workspace_id = ws.id.clone();
        ws.pane_state_mut(child).expect("child pane").parent = Some(crate::pane::PaneParentRef {
            workspace_id,
            pane_number: number,
        });
    }

    /// Mark `pane`'s agent subtree collapsed by its stable key, as clicking the
    /// collapse chevron does.
    fn collapse_agent(state: &mut AppState, pane: PaneId) {
        let ws = &state.workspaces[0];
        let number = ws.public_pane_number(pane).expect("pane has number");
        let key = crate::workspace::public_pane_id_for_number(&ws.id, number);
        state.collapsed_agent_keys.insert(key);
    }

    // One workspace with a parent agent, two child agents linked beneath it, and
    // a second top-level agent below the parent's subtree. Focus starts on the
    // parent. Visible tree order is [parent, child_a, child_b, top2].
    fn app_with_agent_subtree_and_sibling() -> (AppState, PaneId, PaneId, PaneId, PaneId) {
        let mut ws = Workspace::test_new("one");
        let parent = ws.tabs[0].root_pane;
        let child_a = ws.test_split(Direction::Horizontal);
        let child_b = ws.test_split(Direction::Horizontal);
        let top2 = ws.test_split(Direction::Horizontal);
        ws.tabs[0].layout.focus_pane(parent);

        let mut state = AppState::test_new();
        state.workspaces = vec![ws];
        state.ensure_test_terminals();
        state.active = Some(0);
        state.selected = 0;
        state.mode = Mode::Terminal;

        mark_agent(&mut state, 0, 0, parent);
        mark_agent(&mut state, 0, 0, child_a);
        mark_agent(&mut state, 0, 0, child_b);
        mark_agent(&mut state, 0, 0, top2);
        link_agent_parent(&mut state, parent, child_a);
        link_agent_parent(&mut state, parent, child_b);

        (state, parent, child_a, child_b, top2)
    }

    fn visible_agent_order(state: &AppState) -> Vec<PaneId> {
        state
            .visible_agent_targets()
            .iter()
            .map(|(_, pane_id)| *pane_id)
            .collect()
    }

    #[test]
    fn next_agent_follows_visible_tree_order_into_children() {
        let (mut state, parent, child_a, child_b, top2) = app_with_agent_subtree_and_sibling();

        // Anchor the expectation against the order the sidebar renders.
        assert_eq!(
            visible_agent_order(&state),
            vec![parent, child_a, child_b, top2]
        );

        // Parent -> first visible child.
        state.next_agent();
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(child_a));

        // Middle child -> next child.
        state.next_agent();
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(child_b));

        // Last visible child -> next top-level agent below the subtree, not back
        // to the parent.
        state.next_agent();
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(top2));
        state.assert_invariants_for_test();
    }

    #[test]
    fn previous_agent_walks_visible_tree_order_in_reverse() {
        let (mut state, parent, child_a, child_b, top2) = app_with_agent_subtree_and_sibling();
        state.focus_pane_in_workspace(0, top2);
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(top2));

        state.previous_agent();
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(child_b));

        state.previous_agent();
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(child_a));

        state.previous_agent();
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(parent));
        state.assert_invariants_for_test();
    }

    #[test]
    fn next_agent_skips_collapsed_children() {
        let (mut state, parent, child_a, child_b, top2) = app_with_agent_subtree_and_sibling();
        collapse_agent(&mut state, parent);

        // Collapsed children drop out of the visible order entirely.
        assert_eq!(visible_agent_order(&state), vec![parent, top2]);

        // From the collapsed parent, next lands on the next top-level agent.
        state.next_agent();
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(top2));

        // A full cycle never visits the hidden children; it wraps to the parent.
        state.next_agent();
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(parent));
        state.assert_invariants_for_test();
        let _ = (child_a, child_b);
    }

    #[test]
    fn focus_agent_entry_indexes_visible_tree_order() {
        let (mut state, parent, child_a, child_b, top2) = app_with_agent_subtree_and_sibling();

        assert!(state.focus_agent_entry(0));
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(parent));
        assert!(state.focus_agent_entry(1));
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(child_a));
        assert!(state.focus_agent_entry(2));
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(child_b));
        assert!(state.focus_agent_entry(3));
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(top2));

        // Collapsing the parent removes its children from the indexable order, so
        // index 1 now addresses the next top-level agent.
        collapse_agent(&mut state, parent);
        assert!(state.focus_agent_entry(1));
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(top2));
        state.assert_invariants_for_test();
    }

    // Visible tree order for app_with_agent_subtree_and_sibling:
    //   0 parent (expanded, has children)
    //   1 child_a (depth 1)
    //   2 child_b (depth 1)
    //   3 top2   (depth 0, childless)

    #[test]
    fn drop_among_children_attaches_to_that_parent() {
        let (state, parent, _child_a, _child_b, top2) = app_with_agent_subtree_and_sibling();
        // Drop top2 between child_a and child_b (tree slot 2).
        let intent = state
            .agent_reparent_intent_for_drop(ManualEntryRef::Pane(top2), 2)
            .expect("drop among children yields a reparent");
        assert_eq!(intent.child_pane, top2);
        assert_eq!(
            intent.action,
            AgentReparentAction::SetParent {
                parent_ws: 0,
                parent_pane: parent
            }
        );
    }

    #[test]
    fn drop_right_after_expanded_parent_attaches_as_first_child() {
        let (state, parent, _child_a, _child_b, top2) = app_with_agent_subtree_and_sibling();
        // Drop top2 immediately after the parent row (tree slot 1).
        let intent = state
            .agent_reparent_intent_for_drop(ManualEntryRef::Pane(top2), 1)
            .expect("drop after expanded parent yields a reparent");
        assert_eq!(
            intent.action,
            AgentReparentAction::SetParent {
                parent_ws: 0,
                parent_pane: parent
            }
        );
    }

    #[test]
    fn drop_at_top_level_detaches_a_child() {
        let (state, _parent, child_a, _child_b, _top2) = app_with_agent_subtree_and_sibling();
        // Drag child_a to the very end, after the childless top-level agent.
        let intent = state
            .agent_reparent_intent_for_drop(ManualEntryRef::Pane(child_a), 4)
            .expect("dropping a child at top level yields a detach");
        assert_eq!(intent.child_pane, child_a);
        assert_eq!(intent.action, AgentReparentAction::ClearParent);
    }

    #[test]
    fn drop_next_to_childless_agent_does_not_attach() {
        let (state, _parent, child_a, _child_b, _top2) = app_with_agent_subtree_and_sibling();
        // Dropping right after top2 (a childless leaf) never makes child_a its
        // child; a childless agent has no children band to drop into.
        let intent = state.agent_reparent_intent_for_drop(ManualEntryRef::Pane(child_a), 4);
        assert!(matches!(
            intent.map(|i| i.action),
            Some(AgentReparentAction::ClearParent)
        ));
    }

    #[test]
    fn reorder_within_same_group_is_not_a_reparent() {
        let (state, _parent, child_a, _child_b, _top2) = app_with_agent_subtree_and_sibling();
        // Moving child_a within its parent's own child band changes nothing about
        // parentage, so there is no modal, just a plain reorder.
        assert!(state
            .agent_reparent_intent_for_drop(ManualEntryRef::Pane(child_a), 3)
            .is_none());
    }

    #[test]
    fn top_level_move_of_parentless_agent_is_not_a_reparent() {
        let (state, _parent, _child_a, _child_b, top2) = app_with_agent_subtree_and_sibling();
        // top2 has no parent; dropping it at another top-level slot is a reorder.
        assert!(state
            .agent_reparent_intent_for_drop(ManualEntryRef::Pane(top2), 0)
            .is_none());
    }

    #[test]
    fn line_split_source_never_reparents() {
        let (state, _parent, _child_a, _child_b, _top2) = app_with_agent_subtree_and_sibling();
        assert!(state
            .agent_reparent_intent_for_drop(
                ManualEntryRef::LineSplit(crate::app::state::LineSplitId(0)),
                1
            )
            .is_none());
    }

    #[test]
    fn apply_agent_reparent_sets_and_clears_parent() {
        let (mut state, parent, child_a, _child_b, top2) = app_with_agent_subtree_and_sibling();

        // Attach top2 under parent.
        let set = state
            .agent_reparent_intent_for_drop(ManualEntryRef::Pane(top2), 2)
            .expect("attach intent");
        assert!(state.apply_agent_reparent(&set));
        assert_eq!(state.agent_parent_pane(0, top2), Some((0, parent)));

        // Detach child_a back to the top level.
        let clear = crate::app::state::PendingAgentReparent {
            child_ws: 0,
            child_pane: child_a,
            child_label: String::new(),
            parent_label: String::new(),
            action: AgentReparentAction::ClearParent,
            return_mode: Mode::Terminal,
        };
        assert!(state.apply_agent_reparent(&clear));
        assert_eq!(state.agent_parent_pane(0, child_a), None);
        state.assert_invariants_for_test();
    }

    #[test]
    fn apply_agent_reparent_rejects_cycles() {
        let (mut state, parent, child_a, _child_b, _top2) = app_with_agent_subtree_and_sibling();
        // Making parent a child of its own descendant child_a is a cycle.
        let cyclic = crate::app::state::PendingAgentReparent {
            child_ws: 0,
            child_pane: parent,
            child_label: String::new(),
            parent_label: String::new(),
            action: AgentReparentAction::SetParent {
                parent_ws: 0,
                parent_pane: child_a,
            },
            return_mode: Mode::Terminal,
        };
        assert!(!state.apply_agent_reparent(&cyclic));
        // parent stays a root.
        assert_eq!(state.agent_parent_pane(0, parent), None);
    }

    #[test]
    fn next_pane_cycles_pane_section_entries_with_wrap() {
        // Two workspaces: ws0 has a split tab (two non-agent panes), ws1 has one.
        let mut first = Workspace::test_new("one");
        let first_root = first.tabs[0].root_pane;
        first.test_split(Direction::Horizontal);
        first.tabs[0].layout.focus_pane(first_root);
        let second = Workspace::test_new("two");

        let mut state = AppState::test_new();
        state.workspaces = vec![first, second];
        state.ensure_test_terminals();
        state.active = Some(0);
        state.selected = 0;
        state.mode = Mode::Terminal;
        state.reconcile_pane_section_order();

        // Drive cycling against whatever order the Panes section shows.
        let order: Vec<(usize, crate::layout::PaneId)> =
            crate::ui::sidebar_pane_section_entries(&state)
                .iter()
                .map(|entry| (entry.ws_idx, entry.pane_id))
                .collect();
        assert_eq!(order.len(), 3, "two panes in ws0 plus one in ws1");

        // Anchor focus on the first section entry.
        state.focus_pane_in_workspace(order[0].0, order[0].1);
        assert_eq!(state.active, Some(order[0].0));
        assert_eq!(
            state.workspaces[order[0].0].focused_pane_id(),
            Some(order[0].1)
        );

        state.next_pane();
        assert_eq!(state.active, Some(order[1].0));
        assert_eq!(
            state.workspaces[order[1].0].focused_pane_id(),
            Some(order[1].1)
        );

        state.next_pane();
        assert_eq!(state.active, Some(order[2].0));
        assert_eq!(
            state.workspaces[order[2].0].focused_pane_id(),
            Some(order[2].1)
        );

        // Forward from the last entry wraps to the first.
        state.next_pane();
        assert_eq!(state.active, Some(order[0].0));
        assert_eq!(
            state.workspaces[order[0].0].focused_pane_id(),
            Some(order[0].1)
        );

        // Backward from the first entry wraps to the last.
        state.previous_pane();
        assert_eq!(state.active, Some(order[2].0));
        assert_eq!(
            state.workspaces[order[2].0].focused_pane_id(),
            Some(order[2].1)
        );
        state.assert_invariants_for_test();
    }

    #[test]
    fn pane_section_nav_is_noop_when_section_empty() {
        // The workspace's only pane is an agent, so the Panes section is empty.
        let mut state = app_with_workspaces(&["one"]);
        let root = state.workspaces[0].tabs[0].root_pane;
        mark_agent(&mut state, 0, 0, root);
        state.reconcile_pane_section_order();
        assert!(crate::ui::sidebar_pane_section_entries(&state).is_empty());

        state.next_pane();
        assert_eq!(state.active, Some(0));
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(root));

        state.previous_pane();
        assert_eq!(state.active, Some(0));
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(root));
        state.assert_invariants_for_test();
    }

    #[test]
    fn pane_section_nav_with_single_entry_keeps_focus() {
        let mut state = app_with_workspaces(&["one"]);
        let root = state.workspaces[0].tabs[0].root_pane;
        state.reconcile_pane_section_order();
        assert_eq!(crate::ui::sidebar_pane_section_entries(&state).len(), 1);

        state.next_pane();
        assert_eq!(state.active, Some(0));
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(root));

        state.previous_pane();
        assert_eq!(state.active, Some(0));
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(root));
        state.assert_invariants_for_test();
    }

    // Build ws0 with two agents (root, p2) and one non-agent pane (p3), focused
    // on root. Returns (state, p3) where p3 is the sole Panes-section entry.
    fn state_two_agents_one_pane() -> (AppState, crate::layout::PaneId) {
        let mut ws = Workspace::test_new("one");
        let root = ws.tabs[0].root_pane;
        let p2 = ws.test_split(Direction::Horizontal);
        let p3 = ws.test_split(Direction::Horizontal);
        ws.tabs[0].layout.focus_pane(root);

        let mut state = AppState::test_new();
        state.workspaces = vec![ws];
        state.ensure_test_terminals();
        state.active = Some(0);
        state.selected = 0;
        state.mode = Mode::Terminal;
        mark_agent(&mut state, 0, 0, root);
        mark_agent(&mut state, 0, 0, p2);
        state.reconcile_pane_section_order();

        assert_eq!(crate::ui::agent_panel_entries(&state).len(), 2);
        let panes: Vec<crate::layout::PaneId> = crate::ui::sidebar_pane_section_entries(&state)
            .iter()
            .map(|entry| entry.pane_id)
            .collect();
        assert_eq!(panes, vec![p3]);
        (state, p3)
    }

    #[test]
    fn next_agent_on_pane_jumps_to_last_selected_agent() {
        let (mut state, pane) = state_two_agents_one_pane();

        // Select the second agent, then step off into the non-agent pane.
        assert!(state.focus_agent_entry(1));
        let remembered = state.workspaces[0].focused_pane_id().unwrap();
        assert_eq!(state.last_agent_focus, Some(remembered));
        state.focus_pane_in_workspace(0, pane);
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(pane));

        // next_agent jumps back to the remembered agent, not the default first.
        state.next_agent();
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(remembered));
        state.assert_invariants_for_test();
    }

    #[test]
    fn previous_agent_on_pane_jumps_to_same_last_selected_agent() {
        let (mut state, pane) = state_two_agents_one_pane();

        assert!(state.focus_agent_entry(1));
        let remembered = state.workspaces[0].focused_pane_id().unwrap();
        state.focus_pane_in_workspace(0, pane);
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(pane));

        // Direction does not matter for the cross-section jump.
        state.previous_agent();
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(remembered));
        state.assert_invariants_for_test();
    }

    #[test]
    fn pane_nav_on_agent_jumps_to_last_selected_pane_either_direction() {
        // ws0: root is an agent, p2/p3 are non-agent Panes-section entries.
        let mut ws = Workspace::test_new("one");
        let root = ws.tabs[0].root_pane;
        let p2 = ws.test_split(Direction::Horizontal);
        let p3 = ws.test_split(Direction::Horizontal);
        ws.tabs[0].layout.focus_pane(root);

        let mut state = AppState::test_new();
        state.workspaces = vec![ws];
        state.ensure_test_terminals();
        state.active = Some(0);
        state.selected = 0;
        state.mode = Mode::Terminal;
        mark_agent(&mut state, 0, 0, root);
        state.reconcile_pane_section_order();

        let panes: Vec<crate::layout::PaneId> = crate::ui::sidebar_pane_section_entries(&state)
            .iter()
            .map(|entry| entry.pane_id)
            .collect();
        assert_eq!(panes.len(), 2);
        assert!(panes.contains(&p2) && panes.contains(&p3));

        // Select the second Panes-section entry, then focus the agent.
        assert!(state.focus_pane_section_entry(1));
        let remembered = state.workspaces[0].focused_pane_id().unwrap();
        assert_eq!(state.last_pane_section_focus, Some(remembered));
        state.focus_pane_in_workspace(0, root);
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(root));

        // next_pane jumps to the remembered pane.
        state.next_pane();
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(remembered));

        // previous_pane from the agent jumps to the same remembered pane.
        state.focus_pane_in_workspace(0, root);
        state.previous_pane();
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(remembered));
        state.assert_invariants_for_test();
    }

    #[test]
    fn agent_nav_without_memory_falls_back_to_direction_default() {
        let (mut state, pane) = state_two_agents_one_pane();
        let agents: Vec<crate::layout::PaneId> = crate::ui::agent_panel_entries(&state)
            .iter()
            .map(|entry| entry.pane_id)
            .collect();

        // Sit on the non-agent pane; no agent has been selected this session.
        state.focus_pane_in_workspace(0, pane);
        assert_eq!(state.last_agent_focus, None);

        // Forward defaults to the first agent.
        state.next_agent();
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(agents[0]));

        // Backward defaults to the last agent. Clear the memory the forward jump
        // just recorded so this again exercises the no-memory path.
        state.focus_pane_in_workspace(0, pane);
        state.last_agent_focus = None;
        state.previous_agent();
        assert_eq!(
            state.workspaces[0].focused_pane_id(),
            Some(agents[agents.len() - 1])
        );
        state.assert_invariants_for_test();
    }

    #[test]
    fn agent_nav_with_stale_memory_falls_back_to_default() {
        let (mut state, pane) = state_two_agents_one_pane();
        let agents: Vec<crate::layout::PaneId> = crate::ui::agent_panel_entries(&state)
            .iter()
            .map(|entry| entry.pane_id)
            .collect();

        // Remembered entry points at a pane that is not in the agents list
        // (as if the agent had since closed).
        state.last_agent_focus = Some(pane);
        state.focus_pane_in_workspace(0, pane);

        state.next_agent();
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(agents[0]));
        state.assert_invariants_for_test();
    }

    #[test]
    fn agent_nav_within_section_cycles_and_updates_memory() {
        let (mut state, _pane) = state_two_agents_one_pane();
        // Focus starts on the first agent (root), so cycling stays in-section.
        let agents: Vec<crate::layout::PaneId> = crate::ui::agent_panel_entries(&state)
            .iter()
            .map(|entry| entry.pane_id)
            .collect();
        let start = state.workspaces[0].focused_pane_id().unwrap();
        assert!(agents.contains(&start));

        state.next_agent();
        let landed = state.workspaces[0].focused_pane_id().unwrap();
        assert_ne!(landed, start, "within-section next moves to another agent");
        assert!(agents.contains(&landed));
        // Normal cycling records the new selection as the section memory.
        assert_eq!(state.last_agent_focus, Some(landed));
        state.assert_invariants_for_test();
    }

    #[test]
    fn section_cycle_target_index_handles_all_cases() {
        let a = crate::layout::PaneId::from_raw(1);
        let b = crate::layout::PaneId::from_raw(2);
        let c = crate::layout::PaneId::from_raw(3);
        let ids = [a, b, c];

        // Empty list yields no target.
        assert_eq!(
            section_cycle_target_index(&[], Some(a), Some(b), true),
            None
        );

        // Within section: normal forward/backward cycling with wrap.
        assert_eq!(
            section_cycle_target_index(&ids, Some(a), None, true),
            Some(1)
        );
        assert_eq!(
            section_cycle_target_index(&ids, Some(a), None, false),
            Some(2)
        );
        assert_eq!(
            section_cycle_target_index(&ids, Some(c), None, true),
            Some(0)
        );

        // Outside section with valid memory: jump regardless of direction.
        assert_eq!(
            section_cycle_target_index(&ids, None, Some(b), true),
            Some(1)
        );
        assert_eq!(
            section_cycle_target_index(&ids, None, Some(b), false),
            Some(1)
        );

        // Outside section, no memory: direction default (first / last).
        assert_eq!(section_cycle_target_index(&ids, None, None, true), Some(0));
        assert_eq!(section_cycle_target_index(&ids, None, None, false), Some(2));

        // Outside section, stale memory: direction default.
        let stale = crate::layout::PaneId::from_raw(99);
        assert_eq!(
            section_cycle_target_index(&ids, None, Some(stale), true),
            Some(0)
        );
    }

    #[test]
    fn focus_agent_entry_uses_agent_panel_order() {
        let mut first = Workspace::test_new("one");
        let first_root = first.tabs[0].root_pane;
        let first_second = first.test_split(Direction::Horizontal);
        first.tabs[0].layout.focus_pane(first_root);
        let second = Workspace::test_new("two");
        let second_root = second.tabs[0].root_pane;

        let mut state = AppState::test_new();
        state.workspaces = vec![first, second];
        state.active = Some(0);
        state.selected = 0;
        state.mode = Mode::Terminal;
        mark_agent(&mut state, 0, 0, first_root);
        mark_agent(&mut state, 0, 0, first_second);
        mark_agent(&mut state, 1, 0, second_root);

        assert!(state.focus_agent_entry(2));

        assert_eq!(state.active, Some(1));
        assert_eq!(state.workspaces[1].focused_pane_id(), Some(second_root));
        state.assert_invariants_for_test();
    }

    #[test]
    fn focus_agent_entry_succeeds_for_already_focused_agent() {
        let mut state = app_with_workspaces(&["one"]);
        let root = state.workspaces[0].tabs[0].root_pane;
        mark_agent(&mut state, 0, 0, root);

        assert!(state.focus_agent_entry(0));
        assert_eq!(state.active, Some(0));
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(root));
        state.assert_invariants_for_test();
    }

    #[test]
    fn next_agent_cycles_priority_sorted_agent_panel_entries() {
        let mut first = Workspace::test_new("one");
        let first_root = first.tabs[0].root_pane;
        let first_second = first.test_split(Direction::Horizontal);
        first.tabs[0].layout.focus_pane(first_root);
        let second = Workspace::test_new("two");
        let second_root = second.tabs[0].root_pane;

        let mut state = AppState::test_new();
        state.workspaces = vec![first, second];
        state.ensure_test_terminals();
        state.active = Some(0);
        state.selected = 0;
        state.mode = Mode::Terminal;
        state.agent_panel_sort = crate::app::state::AgentPanelSort::Priority;
        set_agent_state(&mut state, 0, 0, first_root, AgentState::Idle);
        set_agent_state(&mut state, 0, 0, first_second, AgentState::Working);
        set_agent_state(&mut state, 1, 0, second_root, AgentState::Blocked);

        state.next_agent();

        assert_eq!(state.active, Some(1));
        assert_eq!(state.workspaces[1].focused_pane_id(), Some(second_root));
        state.assert_invariants_for_test();
    }

    #[test]
    fn priority_sort_keeps_recently_changed_idle_agent_above_older_idle_agent() {
        let mut workspace = Workspace::test_new("one");
        let first = workspace.tabs[0].root_pane;
        let second = workspace.test_split(Direction::Horizontal);
        workspace.tabs[0].layout.focus_pane(first);

        let mut state = AppState::test_new();
        state.workspaces = vec![workspace];
        state.ensure_test_terminals();
        state.active = Some(0);
        state.selected = 0;
        state.mode = Mode::Terminal;
        state.agent_panel_sort = crate::app::state::AgentPanelSort::Priority;

        transition_agent_state(&mut state, first, AgentState::Idle);
        transition_agent_state(&mut state, second, AgentState::Working);
        assert_eq!(crate::ui::agent_panel_entries(&state)[0].pane_id, second);

        transition_agent_state(&mut state, second, AgentState::Idle);

        assert_eq!(crate::ui::agent_panel_entries(&state)[0].pane_id, second);
        state.assert_invariants_for_test();
    }

    #[test]
    fn previous_agent_keeps_wrapped_target_visible_in_agent_panel() {
        let mut workspace = Workspace::test_new("one");
        let root = workspace.tabs[0].root_pane;
        for idx in 1..8 {
            workspace.test_add_tab(Some(&format!("tab-{idx}")));
        }

        let mut state = AppState::test_new();
        state.workspaces = vec![workspace];
        state.ensure_test_terminals();
        state.active = Some(0);
        state.selected = 0;
        state.mode = Mode::Terminal;
        for tab_idx in 0..state.workspaces[0].tabs.len() {
            let pane_id = state.workspaces[0].tabs[tab_idx].root_pane;
            mark_agent(&mut state, 0, tab_idx, pane_id);
        }
        state.workspaces[0].tabs[0].layout.focus_pane(root);
        // Height accommodates the three-band sidebar (Spaces/Panes/Agents) while
        // keeping the Agents band small enough that 8 agents must scroll.
        crate::ui::compute_view(&mut state, ratatui::layout::Rect::new(0, 0, 80, 24));

        state.previous_agent();

        let last_idx = state.workspaces[0].tabs.len() - 1;
        assert_eq!(state.workspaces[0].active_tab, last_idx);
        assert!(state.agent_panel_scroll > 0);
        state.assert_invariants_for_test();
    }

    #[test]
    fn switch_workspace_updates_active_and_selected() {
        let mut state = app_with_workspaces(&["a", "b", "c"]);
        state.switch_workspace(2);
        assert_eq!(state.active, Some(2));
        assert_eq!(state.selected, 2);
    }

    #[test]
    fn last_pane_toggles_to_previous_focus_in_active_tab() {
        let mut state = app_with_workspaces(&["test"]);
        let root = state.workspaces[0].tabs[0].root_pane;
        let right = state.workspaces[0].test_split(Direction::Horizontal);

        state.focus_pane_in_workspace(0, root);
        state.focus_pane_in_workspace(0, right);
        state.last_pane();

        assert_eq!(state.workspaces[0].focused_pane_id(), Some(root));

        state.last_pane();

        assert_eq!(state.workspaces[0].focused_pane_id(), Some(right));
    }

    #[test]
    fn removing_background_pane_preserves_last_pane_history() {
        let mut state = app_with_workspaces(&["test"]);
        let root = state.workspaces[0].tabs[0].root_pane;
        let right = state.workspaces[0].test_split(Direction::Horizontal);
        let background = state.workspaces[0].test_split(Direction::Horizontal);

        state.focus_pane_in_workspace(0, root);
        state.focus_pane_in_workspace(0, right);
        state.workspaces[0].remove_pane(background);
        state.last_pane();

        assert_eq!(state.workspaces[0].focused_pane_id(), Some(root));
    }

    #[test]
    fn last_pane_jumps_across_workspaces_and_tabs() {
        let mut state = app_with_workspaces(&["one", "two"]);
        let first_root = state.workspaces[0].tabs[0].root_pane;
        let second_tab = state.workspaces[1].test_add_tab(Some("logs"));
        let second_tab_root = state.workspaces[1].tabs[second_tab].root_pane;

        state.focus_pane_in_workspace(0, first_root);
        state.focus_pane_in_workspace(1, second_tab_root);
        state.last_pane();

        assert_eq!(state.active, Some(0));
        assert_eq!(state.workspaces[0].active_tab, 0);
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(first_root));

        state.last_pane();

        assert_eq!(state.active, Some(1));
        assert_eq!(state.workspaces[1].active_tab, second_tab);
        assert_eq!(state.workspaces[1].focused_pane_id(), Some(second_tab_root));
    }

    #[test]
    fn last_pane_tracks_tab_and_workspace_switches() {
        let mut state = app_with_workspaces(&["one", "two"]);
        let first_root = state.workspaces[0].tabs[0].root_pane;
        let first_second_tab = state.workspaces[0].test_add_tab(Some("logs"));
        let first_second_root = state.workspaces[0].tabs[first_second_tab].root_pane;
        let second_root = state.workspaces[1].tabs[0].root_pane;

        state.switch_tab(first_second_tab);
        state.last_pane();

        assert_eq!(state.active, Some(0));
        assert_eq!(state.workspaces[0].active_tab, 0);
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(first_root));

        state.last_pane();

        assert_eq!(state.active, Some(0));
        assert_eq!(state.workspaces[0].active_tab, first_second_tab);
        assert_eq!(
            state.workspaces[0].focused_pane_id(),
            Some(first_second_root)
        );

        state.switch_workspace(1);
        state.last_pane();

        assert_eq!(state.active, Some(0));
        assert_eq!(state.workspaces[0].active_tab, first_second_tab);
        assert_eq!(
            state.workspaces[0].focused_pane_id(),
            Some(first_second_root)
        );

        state.last_pane();

        assert_eq!(state.active, Some(1));
        assert_eq!(state.workspaces[1].focused_pane_id(), Some(second_root));
    }

    #[test]
    fn last_pane_tracks_cross_workspace_tab_selection() {
        let mut state = app_with_workspaces(&["one", "two"]);
        let first_root = state.workspaces[0].tabs[0].root_pane;
        let second_first_root = state.workspaces[1].tabs[0].root_pane;
        let second_tab = state.workspaces[1].test_add_tab(Some("logs"));
        let second_tab_root = state.workspaces[1].tabs[second_tab].root_pane;

        state.switch_workspace_tab(1, second_tab);
        state.last_pane();

        assert_eq!(state.active, Some(0));
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(first_root));

        state.last_pane();

        assert_eq!(state.active, Some(1));
        assert_eq!(state.workspaces[1].active_tab, second_tab);
        assert_eq!(state.workspaces[1].focused_pane_id(), Some(second_tab_root));
        assert_ne!(second_first_root, second_tab_root);
    }

    #[test]
    fn switch_workspace_keeps_selected_visible_in_scrolled_sidebar() {
        let mut state = app_with_workspaces(&["a", "b", "c", "d", "e", "f", "g", "h"]);
        crate::ui::compute_view(&mut state, ratatui::layout::Rect::new(0, 0, 80, 14));

        state.switch_workspace(7);
        crate::ui::compute_view(&mut state, ratatui::layout::Rect::new(0, 0, 80, 14));

        assert!(state
            .view
            .workspace_card_areas
            .iter()
            .any(|card| card.ws_idx == 7));
    }

    #[test]
    fn switch_workspace_marks_panes_seen() {
        let mut state = app_with_workspaces(&["a", "b"]);
        // Mark a pane in workspace 1 as unseen
        let id = *state.workspaces[1].panes.keys().next().unwrap();
        state.workspaces[1].panes.get_mut(&id).unwrap().seen = false;

        state.switch_workspace(1);
        assert!(state.workspaces[1].panes.get(&id).unwrap().seen);
    }

    #[test]
    fn switch_workspace_out_of_bounds_is_noop() {
        let mut state = app_with_workspaces(&["a"]);
        state.switch_workspace(5);
        assert_eq!(state.active, Some(0));
    }

    #[test]
    fn space_click_activates_latest_active_tab() {
        let mut state = app_with_workspaces(&["one", "two"]);
        let root = state.workspaces[0].tabs[0].root_pane;
        let logs_tab = state.workspaces[0].test_add_tab(Some("logs"));

        // Deliberately select the logs tab: it becomes both the active and home
        // tab.
        state.switch_tab(logs_tab);
        assert_eq!(state.workspaces[0].active_tab, logs_tab);
        assert_eq!(state.workspaces[0].home_tab, logs_tab);

        // An agent-panel jump to a pane in the first tab moves the active tab
        // (the latest active tab) without touching the home tab.
        state.focus_pane_in_workspace(0, root);
        assert_eq!(state.workspaces[0].active_tab, 0);
        assert_eq!(state.workspaces[0].home_tab, logs_tab);

        // Leaving and returning to the workspace restores the latest active tab,
        // not the deliberately-chosen home tab.
        state.switch_workspace(1);
        state.switch_workspace(0);
        assert_eq!(state.workspaces[0].active_tab, 0);
    }

    #[test]
    fn deliberate_tab_selection_updates_home_tab_but_focus_pane_does_not() {
        let mut state = app_with_workspaces(&["one"]);
        let root = state.workspaces[0].tabs[0].root_pane;
        let logs_tab = state.workspaces[0].test_add_tab(Some("logs"));

        // Keyboard tab switch is a deliberate selection.
        state.switch_tab(logs_tab);
        assert_eq!(state.workspaces[0].home_tab, logs_tab);

        // Focusing a pane (e.g. via the agent panel) is transient.
        state.focus_pane_in_workspace(0, root);
        assert_eq!(state.workspaces[0].active_tab, 0);
        assert_eq!(state.workspaces[0].home_tab, logs_tab);

        // The sticky tab API records the home tab; the plain one does not.
        state.switch_workspace_tab(0, 0);
        assert_eq!(state.workspaces[0].home_tab, logs_tab);
        state.switch_workspace_tab_sticky(0, 0);
        assert_eq!(state.workspaces[0].home_tab, 0);
    }

    #[test]
    fn move_workspace_reorders_without_changing_logical_selection() {
        let mut state = app_with_workspaces(&["a", "b", "c"]);
        let active_id = state.workspaces[1].id.clone();
        let selected_id = state.workspaces[2].id.clone();
        state.active = Some(1);
        state.selected = 2;

        state.move_workspace(1, 0);

        let names: Vec<_> = state
            .workspaces
            .iter()
            .map(|ws| ws.display_name())
            .collect();
        assert_eq!(names, vec!["b", "a", "c"]);
        assert_eq!(state.active, Some(0));
        assert_eq!(state.selected, 2);
        assert_eq!(state.workspaces[state.active.unwrap()].id, active_id);
        assert_eq!(state.workspaces[state.selected].id, selected_id);
    }

    #[test]
    fn move_workspace_accepts_insert_at_end() {
        let mut state = app_with_workspaces(&["a", "b", "c"]);

        state.move_workspace(0, state.workspaces.len());

        let names: Vec<_> = state
            .workspaces
            .iter()
            .map(|ws| ws.display_name())
            .collect();
        assert_eq!(names, vec!["b", "c", "a"]);
    }

    fn manual_order_pane_ids(state: &AppState) -> Vec<crate::layout::PaneId> {
        state
            .agent_manual_order
            .order
            .iter()
            .filter_map(|entry| match entry {
                crate::app::state::ManualEntry::Pane(pane_id) => Some(*pane_id),
                crate::app::state::ManualEntry::LineSplit { .. } => None,
            })
            .collect()
    }

    /// Pane references (line-splits excluded) in Panes-section order.
    fn pane_section_pane_refs(state: &AppState) -> Vec<crate::app::state::PaneSectionRef> {
        state
            .pane_section_order
            .order
            .iter()
            .filter_map(|entry| match entry {
                crate::app::state::PaneManualEntry::Pane(pane_ref) => Some(pane_ref.clone()),
                crate::app::state::PaneManualEntry::LineSplit { .. } => None,
            })
            .collect()
    }

    /// Workspace ids of the pane rows (line-splits excluded) in Panes-section
    /// order.
    fn pane_section_workspace_ids(state: &AppState) -> Vec<String> {
        pane_section_pane_refs(state)
            .into_iter()
            .map(|pane_ref| pane_ref.workspace_id)
            .collect()
    }

    /// Two workspaces (ws0 has two agent panes, ws1 has one) with all panes
    /// marked as agents. Returns (state, [a, b, c]).
    fn app_with_agents() -> (
        AppState,
        crate::layout::PaneId,
        crate::layout::PaneId,
        crate::layout::PaneId,
    ) {
        let mut first = Workspace::test_new("one");
        let a = first.tabs[0].root_pane;
        let b = first.test_split(Direction::Horizontal);
        first.tabs[0].layout.focus_pane(a);
        let second = Workspace::test_new("two");
        let c = second.tabs[0].root_pane;

        let mut state = AppState::test_new();
        state.workspaces = vec![first, second];
        state.ensure_test_terminals();
        state.active = Some(0);
        state.selected = 0;
        state.mode = Mode::Terminal;
        state.agent_panel_sort = crate::app::state::AgentPanelSort::Manual;
        mark_agent(&mut state, 0, 0, a);
        mark_agent(&mut state, 0, 0, b);
        mark_agent(&mut state, 1, 0, c);
        (state, a, b, c)
    }

    #[test]
    fn manual_order_seeds_natural_display_order() {
        let (mut state, a, b, c) = app_with_agents();

        state.reconcile_agent_manual_order();

        assert!(state.agent_manual_order.seeded);
        assert_eq!(manual_order_pane_ids(&state), vec![a, b, c]);
        state.assert_invariants_for_test();
    }

    #[test]
    fn pane_section_enumerates_one_entry_per_non_agent_pane() {
        // A single workspace whose one tab is split into two panes yields two
        // Panes-section entries (one per pane, not one per tab). Marking one pane
        // as an agent drops just that pane's entry.
        let mut first = Workspace::test_new("one");
        let a = first.tabs[0].root_pane;
        let b = first.test_split(Direction::Horizontal);
        let mut state = AppState::test_new();
        state.workspaces = vec![first];
        state.ensure_test_terminals();
        state.active = Some(0);
        state.mode = Mode::Terminal;
        state.reconcile_pane_section_order();

        let entries = crate::ui::sidebar_pane_section_entries(&state);
        let pane_ids: Vec<_> = entries.iter().map(|entry| entry.pane_id).collect();
        assert_eq!(entries.len(), 2, "both panes of the split tab are listed");
        assert!(pane_ids.contains(&a) && pane_ids.contains(&b));

        // Make pane `b` an agent pane; only the non-agent pane `a` remains.
        mark_agent(&mut state, 0, 0, b);
        state.reconcile_pane_section_order();
        let entries = crate::ui::sidebar_pane_section_entries(&state);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].pane_id, a);
    }

    #[test]
    fn pane_section_manual_order_reorders_across_spaces_visually() {
        use crate::app::state::{PaneManualEntry, PaneManualEntryRef};
        let mut state = app_with_workspaces(&["one", "two"]);
        // Each workspace has a single plain (non-agent) tab.
        state.reconcile_pane_section_order();
        let ws0_id = state.workspaces[0].id.clone();
        let ws1_id = state.workspaces[1].id.clone();
        assert_eq!(
            pane_section_workspace_ids(&state),
            vec![ws0_id.clone(), ws1_id.clone()]
        );
        let tab0_number = state.workspaces[0].tabs[0].number;
        let tab1_number = state.workspaces[1].tabs[0].number;

        // Move workspace two's tab to the front: a cross-space visual reorder.
        let PaneManualEntry::Pane(source_ref) = state.pane_section_order.order[1].clone() else {
            panic!("expected pane entry");
        };
        assert!(state.move_pane_section_entry(PaneManualEntryRef::Pane(source_ref), 0));
        assert_eq!(pane_section_workspace_ids(&state), vec![ws1_id, ws0_id]);

        // The real tabs inside each workspace are untouched.
        assert_eq!(state.workspaces[0].tabs.len(), 1);
        assert_eq!(state.workspaces[1].tabs.len(), 1);
        assert_eq!(state.workspaces[0].tabs[0].number, tab0_number);
        assert_eq!(state.workspaces[1].tabs[0].number, tab1_number);
    }

    #[test]
    fn pane_section_reconcile_drops_dead_and_places_new_at_top() {
        let mut state = app_with_workspaces(&["one"]);
        state.workspaces[0].test_add_tab(Some("logs")); // adds a 2nd non-agent pane
        state.ensure_test_terminals();
        state.reconcile_pane_section_order();
        assert_eq!(state.pane_section_order.order.len(), 2);

        // A genuinely new non-agent pane lands at the top of the list.
        let new_tab = state.workspaces[0].test_add_tab(Some("new"));
        state.ensure_test_terminals();
        state.reconcile_pane_section_order();
        let new_pane = state.workspaces[0].tabs[new_tab].root_pane;
        let new_number = state.workspaces[0].public_pane_number(new_pane).unwrap();
        assert_eq!(
            pane_section_pane_refs(&state)[0].pane_number,
            new_number,
            "new pane lands at the top"
        );
        assert_eq!(state.pane_section_order.order.len(), 3);

        // Turning the first tab's pane into an agent pane drops it from the
        // Panes section.
        let ws0_id = state.workspaces[0].id.clone();
        let tab0_pane = state.workspaces[0].tabs[0].root_pane;
        let tab0_number = state.workspaces[0].public_pane_number(tab0_pane).unwrap();
        mark_agent(&mut state, 0, 0, tab0_pane);
        state.reconcile_pane_section_order();
        assert!(
            !pane_section_pane_refs(&state)
                .iter()
                .any(|r| r.pane_number == tab0_number && r.workspace_id == ws0_id),
            "agent pane must drop out of the Panes section"
        );
        assert_eq!(state.pane_section_order.order.len(), 2);
    }

    #[test]
    fn pane_section_reconcile_leaves_line_splits_untouched_and_places_new_panes() {
        use crate::app::state::{PaneManualEntry, PaneManualEntryRef};
        let mut state = app_with_workspaces(&["one"]);
        let tab1 = state.workspaces[0].test_add_tab(Some("logs"));
        state.ensure_test_terminals();
        state.reconcile_pane_section_order();
        assert_eq!(pane_section_pane_refs(&state).len(), 2);

        // Insert a line-split between the two panes (flat index 1).
        let split = state
            .pane_section_order
            .new_line_split("scheduled".to_string(), 1);

        // A genuinely new pane lands at the top; the split keeps its slot.
        let new_tab = state.workspaces[0].test_add_tab(Some("new"));
        state.ensure_test_terminals();
        state.reconcile_pane_section_order();
        let new_pane = state.workspaces[0].tabs[new_tab].root_pane;
        let new_number = state.workspaces[0].public_pane_number(new_pane).unwrap();
        assert!(
            matches!(&state.pane_section_order.order[0], PaneManualEntry::Pane(r) if r.pane_number == new_number),
            "new pane inserted at the top of the order"
        );
        assert!(
            state.pane_section_order.order.iter().any(|entry| matches!(
                entry,
                PaneManualEntry::LineSplit { id, name } if *id == split && name == "scheduled"
            )),
            "line-split retained across new-pane placement"
        );

        // Closing a pane prunes its ref but keeps the line-split in place.
        let pane1 = state.workspaces[0].tabs[tab1].root_pane;
        state.workspaces[0].close_pane(pane1);
        state.ensure_test_terminals();
        state.reconcile_pane_section_order();
        assert!(
            state.pane_section_order.order.iter().any(|entry| matches!(
                entry,
                PaneManualEntry::LineSplit { id, .. } if *id == split
            )),
            "line-split survives pane removal"
        );

        // The split can still be moved to the front via the entry-move path.
        assert!(state.move_pane_section_entry(PaneManualEntryRef::LineSplit(split), 0));
        assert!(matches!(
            state.pane_section_order.order.first(),
            Some(PaneManualEntry::LineSplit { id, .. }) if *id == split
        ));
        state.assert_invariants_for_test();
    }

    #[test]
    fn move_pane_section_entry_moves_line_split_and_clamps() {
        use crate::app::state::{PaneManualEntry, PaneManualEntryRef};
        let mut state = app_with_workspaces(&["one", "two"]);
        state.reconcile_pane_section_order();
        let split = state.pane_section_order.new_line_split("x".to_string(), 0);

        // Clamp an out-of-range insert index to the end.
        assert!(state.move_pane_section_entry(PaneManualEntryRef::LineSplit(split), 999));
        assert!(matches!(
            state.pane_section_order.order.last(),
            Some(PaneManualEntry::LineSplit { id, .. }) if *id == split
        ));
        state.assert_invariants_for_test();
    }

    #[test]
    fn pane_section_entries_skip_line_splits() {
        let mut state = app_with_workspaces(&["one", "two"]);
        state.reconcile_pane_section_order();
        // Scatter line-splits through the order; the pane-only enumeration (used
        // for focus/navigation) must never surface them.
        state
            .pane_section_order
            .new_line_split("top".to_string(), 0);
        state
            .pane_section_order
            .new_line_split("mid".to_string(), 2);
        let entries = crate::ui::sidebar_pane_section_entries(&state);
        assert_eq!(
            entries.len(),
            2,
            "only panes are enumerated, splits skipped"
        );
        state.assert_invariants_for_test();
    }

    #[test]
    fn manual_order_new_same_workspace_agent_lands_above_topmost() {
        // Seed with ws0 pane A and ws1 pane C only.
        let first = Workspace::test_new("one");
        let a = first.tabs[0].root_pane;
        let second = Workspace::test_new("two");
        let c = second.tabs[0].root_pane;

        let mut state = AppState::test_new();
        state.workspaces = vec![first, second];
        state.ensure_test_terminals();
        state.active = Some(0);
        state.selected = 0;
        state.mode = Mode::Terminal;
        state.agent_panel_sort = crate::app::state::AgentPanelSort::Manual;
        mark_agent(&mut state, 0, 0, a);
        mark_agent(&mut state, 1, 0, c);
        state.reconcile_agent_manual_order();
        assert_eq!(manual_order_pane_ids(&state), vec![a, c]);

        // Add a new agent pane to ws0; it should land above the topmost ws0 pane.
        let b = state.workspaces[0].test_split(Direction::Horizontal);
        state.ensure_test_terminals();
        mark_agent(&mut state, 0, 0, b);
        state.reconcile_agent_manual_order();

        assert_eq!(manual_order_pane_ids(&state), vec![b, a, c]);
        state.assert_invariants_for_test();
    }

    #[test]
    fn manual_order_new_workspace_agent_lands_at_top() {
        let first = Workspace::test_new("one");
        let a = first.tabs[0].root_pane;

        let mut state = AppState::test_new();
        state.workspaces = vec![first];
        state.ensure_test_terminals();
        state.active = Some(0);
        state.selected = 0;
        state.mode = Mode::Terminal;
        state.agent_panel_sort = crate::app::state::AgentPanelSort::Manual;
        mark_agent(&mut state, 0, 0, a);
        state.reconcile_agent_manual_order();
        assert_eq!(manual_order_pane_ids(&state), vec![a]);

        // A brand new workspace with no pane in the order lands at the very top.
        let second = Workspace::test_new("two");
        let c = second.tabs[0].root_pane;
        state.workspaces.push(second);
        state.ensure_test_terminals();
        mark_agent(&mut state, 1, 0, c);
        state.reconcile_agent_manual_order();

        assert_eq!(manual_order_pane_ids(&state), vec![c, a]);
        state.assert_invariants_for_test();
    }

    #[test]
    fn move_agent_reorders_within_and_cross_workspace() {
        let (mut state, a, b, c) = app_with_agents();
        state.reconcile_agent_manual_order();
        assert_eq!(manual_order_pane_ids(&state), vec![a, b, c]);

        // Move the ws1 pane C to the very top (cross-workspace).
        assert!(state.move_agent_entry(crate::app::state::ManualEntryRef::Pane(c), 0));
        assert_eq!(manual_order_pane_ids(&state), vec![c, a, b]);

        // Move A down to the end (within-list).
        assert!(state.move_agent_entry(
            crate::app::state::ManualEntryRef::Pane(a),
            state.agent_manual_order.order.len()
        ));
        assert_eq!(manual_order_pane_ids(&state), vec![c, b, a]);
        state.assert_invariants_for_test();
    }

    #[test]
    fn move_agent_clamps_out_of_range_insert_index() {
        let (mut state, a, b, c) = app_with_agents();
        state.reconcile_agent_manual_order();

        assert!(state.move_agent_entry(crate::app::state::ManualEntryRef::Pane(a), 999));
        assert_eq!(manual_order_pane_ids(&state), vec![b, c, a]);
        state.assert_invariants_for_test();
    }

    #[test]
    fn move_agent_noop_for_unknown_pane() {
        let (mut state, _a, _b, _c) = app_with_agents();
        state.reconcile_agent_manual_order();
        let before = manual_order_pane_ids(&state);

        assert!(!state.move_agent_entry(
            crate::app::state::ManualEntryRef::Pane(crate::layout::PaneId::from_raw(9999)),
            0
        ));
        assert_eq!(manual_order_pane_ids(&state), before);
    }

    #[test]
    fn reconcile_leaves_line_splits_untouched_and_places_new_panes() {
        use crate::app::state::{ManualEntry, ManualEntryRef};
        let (mut state, a, b, c) = app_with_agents();
        state.reconcile_agent_manual_order();
        assert_eq!(manual_order_pane_ids(&state), vec![a, b, c]);

        // Insert a line-split between a and b (flat index 1).
        let split = state
            .agent_manual_order
            .new_line_split("scheduled".to_string(), 1);

        // Closing a pane prunes it but keeps the line-split in place.
        state.workspaces[0].close_pane(b);
        state.reconcile_agent_manual_order();
        let ids_and_splits: Vec<_> = state
            .agent_manual_order
            .order
            .iter()
            .map(|entry| match entry {
                ManualEntry::Pane(pane_id) => format!("pane:{pane_id:?}"),
                ManualEntry::LineSplit { id, name } => format!("split:{}:{name}", id.0),
            })
            .collect();
        assert_eq!(
            ids_and_splits,
            vec![
                format!("pane:{a:?}"),
                format!("split:{}:scheduled", split.0),
                format!("pane:{c:?}"),
            ]
        );
        assert!(state
            .agent_manual_order
            .order
            .iter()
            .any(|entry| matches!(entry, ManualEntry::LineSplit { id, .. } if *id == split)));

        // Moving the line-split still works via the entry-move path.
        assert!(state.move_agent_entry(ManualEntryRef::LineSplit(split), 0));
        assert!(matches!(
            state.agent_manual_order.order.first(),
            Some(ManualEntry::LineSplit { id, .. }) if *id == split
        ));
        state.assert_invariants_for_test();
    }

    #[test]
    fn move_agent_entry_moves_line_split_and_clamps() {
        use crate::app::state::{ManualEntry, ManualEntryRef};
        let (mut state, _a, _b, _c) = app_with_agents();
        state.reconcile_agent_manual_order();
        let split = state.agent_manual_order.new_line_split("x".to_string(), 0);

        // Clamp an out-of-range insert index to the end.
        assert!(state.move_agent_entry(ManualEntryRef::LineSplit(split), 999));
        assert!(matches!(
            state.agent_manual_order.order.last(),
            Some(ManualEntry::LineSplit { id, .. }) if *id == split
        ));
        state.assert_invariants_for_test();
    }

    #[test]
    fn keyboard_cycle_skips_line_split_rows() {
        let (mut state, a, b, c) = app_with_agents();
        state.reconcile_agent_manual_order();
        // Line-splits scattered through the order must never receive focus.
        state
            .agent_manual_order
            .new_line_split("top".to_string(), 0);
        state
            .agent_manual_order
            .new_line_split("mid".to_string(), 2);

        state.workspaces[0].tabs[0].layout.focus_pane(a);
        let mut focused = Vec::new();
        for _ in 0..3 {
            state.next_agent();
            if let Some(pane) = state
                .active
                .and_then(|i| state.workspaces[i].focused_pane_id())
            {
                focused.push(pane);
            }
        }
        // Cycling visits only agent panes, wrapping across all three.
        assert!(focused.iter().all(|pane| [a, b, c].contains(pane)));
        assert!(focused.contains(&b) && focused.contains(&c));
    }

    #[test]
    fn manual_order_drops_stale_pane_on_close() {
        let (mut state, a, b, c) = app_with_agents();
        state.reconcile_agent_manual_order();
        assert_eq!(manual_order_pane_ids(&state), vec![a, b, c]);

        // close_pane returns false while the workspace still has other panes;
        // it still removes the pane, which is what matters here.
        state.workspaces[0].close_pane(b);
        state.reconcile_agent_manual_order();

        assert_eq!(manual_order_pane_ids(&state), vec![a, c]);
        assert!(!state.agent_manual_order.known.contains(&b));
        state.assert_invariants_for_test();
    }

    #[test]
    fn manual_order_snapshot_remaps_panes_by_public_key() {
        let (mut state, a, b, c) = app_with_agents();
        state.reconcile_agent_manual_order();
        state.move_agent_entry(crate::app::state::ManualEntryRef::Pane(c), 0);
        assert_eq!(manual_order_pane_ids(&state), vec![c, a, b]);

        let keys = state.agent_manual_order.to_public_keys(&state.workspaces);
        assert_eq!(keys.len(), 3);

        // Simulate the PaneId remap that restore performs: reassign fresh pane
        // ids to the same public pane numbers.
        let mut remap = std::collections::HashMap::new();
        for ws in &mut state.workspaces {
            let new_map: std::collections::HashMap<crate::layout::PaneId, usize> = ws
                .public_pane_numbers
                .iter()
                .map(|(old, number)| {
                    let new_id = crate::layout::PaneId::alloc();
                    remap.insert(*old, new_id);
                    (new_id, *number)
                })
                .collect();
            ws.public_pane_numbers = new_map;
        }

        let rebuilt =
            crate::app::state::AgentManualOrder::from_public_keys(&keys, &state.workspaces);
        assert!(rebuilt.seeded);
        // Order preserved by stable keys.
        assert_eq!(rebuilt.to_public_keys(&state.workspaces), keys);
        // Panes were actually remapped to the new ids.
        let rebuilt_ids: Vec<_> = rebuilt
            .order
            .iter()
            .filter_map(|entry| match entry {
                crate::app::state::ManualEntry::Pane(pane_id) => Some(*pane_id),
                crate::app::state::ManualEntry::LineSplit { .. } => None,
            })
            .collect();
        assert_eq!(rebuilt_ids, vec![remap[&c], remap[&a], remap[&b]]);
    }

    #[test]
    fn manual_order_seed_preserves_existing_multi_agent_order() {
        // Enabling manual mode on an already-populated multi-agent state must
        // preserve the visible (natural) order.
        let (mut state, a, b, c) = app_with_agents();
        assert!(!state.agent_manual_order.seeded);

        state.reconcile_agent_manual_order();

        assert_eq!(manual_order_pane_ids(&state), vec![a, b, c]);
    }

    #[test]
    fn close_workspace_adjusts_indices() {
        let mut state = app_with_workspaces(&["a", "b", "c"]);
        state.selected = 1;
        state.active = Some(1);

        state.close_selected_workspace();

        assert_eq!(state.workspaces.len(), 2);
        assert_eq!(state.selected, 1);
        assert_eq!(state.active, Some(1));
        assert_eq!(state.workspaces[1].custom_name.as_deref(), Some("c"));
    }

    #[test]
    fn close_parent_worktree_workspace_closes_group() {
        let mut state = app_with_workspaces(&["main", "issue", "notes"]);
        state.workspaces[0].worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
            key: "repo-key".into(),
            label: "herdr".into(),
            repo_root: "/repo/herdr".into(),
            checkout_path: "/repo/herdr".into(),
            is_linked_worktree: false,
        });
        state.workspaces[1].worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
            key: "repo-key".into(),
            label: "herdr".into(),
            repo_root: "/repo/herdr".into(),
            checkout_path: "/repo/herdr-issue".into(),
            is_linked_worktree: true,
        });
        state.selected = 0;
        state.active = Some(0);

        state.close_selected_workspace();

        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.workspaces[0].display_name(), "notes");
        assert_eq!(state.active, Some(0));
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn close_last_workspace_clears_active() {
        let mut state = app_with_workspaces(&["only"]);
        state.selected = 0;
        state.close_selected_workspace();

        assert!(state.workspaces.is_empty());
        assert_eq!(state.active, None);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn close_workspace_at_end_adjusts_selected() {
        let mut state = app_with_workspaces(&["a", "b"]);
        state.selected = 1;
        state.active = Some(1);

        state.close_selected_workspace();

        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.selected, 0);
        assert_eq!(state.active, Some(0));
    }

    #[test]
    fn pane_died_last_pane_removes_workspace() {
        let mut state = app_with_workspaces(&["a", "b"]);
        let pane_id = *state.workspaces[0].panes.keys().next().unwrap();

        state.handle_pane_died(pane_id);

        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.workspaces[0].custom_name.as_deref(), Some("b"));
        state.assert_invariants_for_test();
    }

    #[test]
    fn pane_died_last_workspace_enters_navigate() {
        let mut state = app_with_workspaces(&["only"]);
        state.mode = Mode::Terminal;
        let pane_id = *state.workspaces[0].panes.keys().next().unwrap();

        state.handle_pane_died(pane_id);

        assert!(state.workspaces.is_empty());
        assert_eq!(state.mode, Mode::Navigate);
        state.assert_invariants_for_test();
    }

    #[test]
    fn pane_died_multi_pane_keeps_workspace() {
        let mut state = app_with_workspaces(&["test"]);
        let second_id = state.workspaces[0].test_split(Direction::Horizontal);
        state.ensure_test_terminals();

        state.handle_pane_died(second_id);

        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.workspaces[0].panes.len(), 1);
        state.assert_invariants_for_test();
    }

    #[test]
    fn pane_died_unknown_pane_is_noop() {
        let mut state = app_with_workspaces(&["test"]);
        let fake_id = PaneId::from_raw(9999);

        state.handle_pane_died(fake_id);

        assert_eq!(state.workspaces.len(), 1);
        state.assert_invariants_for_test();
    }

    #[test]
    fn pane_died_unrelated_pane_preserves_selection() {
        // Two workspaces; user is selecting text in workspace 0.
        // A pane in workspace 1 dies — selection must be preserved.
        let mut state = app_with_workspaces(&["active", "bg"]);
        let active_pane = *state.workspaces[0].panes.keys().next().unwrap();
        let bg_pane = *state.workspaces[1].panes.keys().next().unwrap();

        state.selection = Some(crate::selection::Selection::anchor(active_pane, 0, 0, None));
        state.selection_autoscroll = Some(crate::app::state::SelectionAutoscroll {
            direction: crate::app::state::SelectionAutoscrollDirection::Down,
            last_mouse_screen_col: 0,
            last_mouse_screen_row: 23,
            inner_rect: ratatui::layout::Rect::new(0, 0, 80, 24),
        });

        state.handle_pane_died(bg_pane);

        assert!(state.selection.is_some());
        assert!(state.selection_autoscroll.is_some());
        state.assert_invariants_for_test();
    }

    #[test]
    fn pane_died_same_pane_clears_selection() {
        let mut state = app_with_workspaces(&["test"]);
        let first_id = state.workspaces[0].tabs[0].root_pane;
        let second_id = state.workspaces[0].test_split(Direction::Horizontal);
        state.ensure_test_terminals();

        state.selection = Some(crate::selection::Selection::anchor(second_id, 0, 0, None));
        state.selection_autoscroll = Some(crate::app::state::SelectionAutoscroll {
            direction: crate::app::state::SelectionAutoscrollDirection::Down,
            last_mouse_screen_col: 0,
            last_mouse_screen_row: 23,
            inner_rect: ratatui::layout::Rect::new(0, 0, 80, 24),
        });

        state.handle_pane_died(second_id);

        // first_id still alive, workspace stays, but selection was on the dying pane
        assert!(state.selection.is_none());
        assert!(state.selection_autoscroll.is_none());
        assert_eq!(state.workspaces[0].panes.len(), 1);
        assert_eq!(state.workspaces[0].panes.keys().next().unwrap(), &first_id);
        state.assert_invariants_for_test();
    }

    #[test]
    fn state_changed_updates_pane() {
        let mut state = app_with_workspaces(&["test"]);
        let pane_id = *state.workspaces[0].panes.keys().next().unwrap();

        state.handle_app_event(AppEvent::StateChanged {
            pane_id,
            agent: Some(Agent::Pi),
            state: AgentState::Working,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });

        let terminal_id = state.workspaces[0]
            .panes
            .get(&pane_id)
            .unwrap()
            .attached_terminal_id
            .clone();
        let terminal = state.terminals.get(&terminal_id).unwrap();
        assert_eq!(terminal.state, AgentState::Working);
        assert_eq!(terminal.detected_agent, Some(Agent::Pi));
    }

    #[test]
    fn state_changed_idle_in_background_marks_unseen() {
        let mut state = app_with_workspaces(&["active", "background"]);
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        state.active = Some(0);
        let bg_pane_id = *state.workspaces[1].panes.keys().next().unwrap();

        // First set it to Working
        let bg_terminal_id = state.workspaces[1]
            .panes
            .get(&bg_pane_id)
            .unwrap()
            .attached_terminal_id
            .clone();
        state.terminals.get_mut(&bg_terminal_id).unwrap().state = AgentState::Working;

        // Now transition to Idle while in background
        state.handle_app_event(AppEvent::StateChanged {
            pane_id: bg_pane_id,
            agent: Some(Agent::Pi),
            state: AgentState::Idle,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });

        let pane = state.workspaces[1].panes.get(&bg_pane_id).unwrap();
        assert!(!pane.seen);
        assert!(matches!(
            state.toast.as_ref().map(|toast| toast.kind),
            Some(ToastKind::Finished)
        ));
    }

    #[test]
    fn active_tab_completion_marks_pane_seen() {
        let mut state = app_with_workspaces(&["active"]);
        state.active = Some(0);
        state.outer_terminal_focus = Some(true);
        let pane_id = *state.workspaces[0].panes.keys().next().unwrap();
        let terminal_id = state.workspaces[0]
            .panes
            .get(&pane_id)
            .unwrap()
            .attached_terminal_id
            .clone();
        state.terminals.get_mut(&terminal_id).unwrap().state = AgentState::Working;
        state.workspaces[0].panes.get_mut(&pane_id).unwrap().seen = false;

        state.handle_app_event(AppEvent::StateChanged {
            pane_id,
            agent: Some(Agent::Pi),
            state: AgentState::Idle,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });

        let terminal = state.terminals.get(&terminal_id).unwrap();
        assert_eq!(terminal.state, AgentState::Idle);
        let pane = state.workspaces[0].panes.get(&pane_id).unwrap();
        assert!(pane.seen);
    }

    #[test]
    fn initial_idle_in_background_stays_seen() {
        let mut state = app_with_workspaces(&["active", "background"]);
        state.active = Some(0);
        let bg_pane_id = *state.workspaces[1].panes.keys().next().unwrap();

        state.handle_app_event(AppEvent::StateChanged {
            pane_id: bg_pane_id,
            agent: Some(Agent::Pi),
            state: AgentState::Idle,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });

        let pane = state.workspaces[1].panes.get(&bg_pane_id).unwrap();
        assert!(pane.seen);
    }

    #[test]
    fn idle_after_known_unknown_agent_in_background_marks_done() {
        let mut state = app_with_workspaces(&["active", "background"]);
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        state.active = Some(0);
        let bg_pane_id = *state.workspaces[1].panes.keys().next().unwrap();

        state.handle_app_event(AppEvent::StateChanged {
            pane_id: bg_pane_id,
            agent: Some(Agent::Pi),
            state: AgentState::Unknown,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });
        state.handle_app_event(AppEvent::StateChanged {
            pane_id: bg_pane_id,
            agent: Some(Agent::Pi),
            state: AgentState::Idle,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });

        let pane = state.workspaces[1].panes.get(&bg_pane_id).unwrap();
        assert!(!pane.seen);
    }

    #[test]
    fn waiting_sound_plays_even_in_active_workspace() {
        assert_eq!(
            notification_sound_for_state_change(true, AgentState::Working, AgentState::Blocked),
            Some(crate::sound::Sound::Request)
        );
    }

    #[test]
    fn done_sound_only_plays_in_background() {
        assert_eq!(
            notification_sound_for_state_change(false, AgentState::Working, AgentState::Idle),
            Some(crate::sound::Sound::Done)
        );
        assert_eq!(
            notification_sound_for_state_change(true, AgentState::Working, AgentState::Idle),
            None
        );
        assert_eq!(
            notification_sound_for_state_change(false, AgentState::Unknown, AgentState::Idle),
            None
        );
    }

    #[test]
    fn background_waiting_sets_attention_toast() {
        let mut state = app_with_workspaces(&["active", "background"]);
        state.active = Some(0);
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        let bg_pane_id = *state.workspaces[1].panes.keys().next().unwrap();

        state.handle_app_event(AppEvent::StateChanged {
            pane_id: bg_pane_id,
            agent: Some(Agent::Pi),
            state: AgentState::Blocked,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });

        let toast = state.toast.as_ref().unwrap();
        assert_eq!(toast.kind, ToastKind::NeedsAttention);
        assert_eq!(toast.title, "pi needs attention");
        assert_eq!(toast.context, "background · 2");
    }

    #[test]
    fn delayed_background_waiting_schedules_before_toast() {
        let mut state = app_with_workspaces(&["active", "background"]);
        state.active = Some(0);
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        state.toast_config.delay_seconds = 1;
        let bg_pane_id = *state.workspaces[1].panes.keys().next().unwrap();

        state.handle_app_event(AppEvent::StateChanged {
            pane_id: bg_pane_id,
            agent: Some(Agent::Pi),
            state: AgentState::Blocked,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });

        assert!(state.toast.is_none());
        assert!(state.pending_agent_notifications.contains_key(&bg_pane_id));

        let deadline = state.next_pending_agent_notification_deadline().unwrap();
        let deliveries = state.drain_due_agent_notifications(deadline);
        assert_eq!(deliveries.len(), 1);

        let toast = state.toast.as_ref().unwrap();
        assert_eq!(toast.kind, ToastKind::NeedsAttention);
        assert_eq!(toast.title, "pi needs attention");
        assert_eq!(toast.context, "background · 2");
        assert!(state.pending_agent_notifications.is_empty());
    }

    #[test]
    fn delayed_background_waiting_cancels_when_agent_resumes_working() {
        let mut state = app_with_workspaces(&["active", "background"]);
        state.active = Some(0);
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        state.toast_config.delay_seconds = 1;
        let bg_pane_id = *state.workspaces[1].panes.keys().next().unwrap();

        state.handle_app_event(AppEvent::StateChanged {
            pane_id: bg_pane_id,
            agent: Some(Agent::Pi),
            state: AgentState::Blocked,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });
        let deadline = state.next_pending_agent_notification_deadline().unwrap();

        state.handle_app_event(AppEvent::StateChanged {
            pane_id: bg_pane_id,
            agent: Some(Agent::Pi),
            state: AgentState::Working,
            visible_blocker: false,
            visible_working: true,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });

        assert!(state.pending_agent_notifications.is_empty());
        assert!(state.drain_due_agent_notifications(deadline).is_empty());
        assert!(state.toast.is_none());
    }

    #[test]
    fn delayed_background_waiting_is_suppressed_if_pane_becomes_active() {
        let mut state = app_with_workspaces(&["active", "background"]);
        state.active = Some(0);
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        state.toast_config.delay_seconds = 1;
        let bg_pane_id = *state.workspaces[1].panes.keys().next().unwrap();

        state.handle_app_event(AppEvent::StateChanged {
            pane_id: bg_pane_id,
            agent: Some(Agent::Pi),
            state: AgentState::Blocked,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });
        let deadline = state.next_pending_agent_notification_deadline().unwrap();
        state.active = Some(1);

        assert!(state.drain_due_agent_notifications(deadline).is_empty());
        assert!(state.toast.is_none());
    }

    #[test]
    fn delayed_active_tab_unfocused_keeps_client_notification_available() {
        let mut state = app_with_workspaces(&["active"]);
        state.active = Some(0);
        state.outer_terminal_focus = Some(false);
        state.toast_config.delivery = crate::config::ToastDelivery::System;
        state.toast_config.delay_seconds = 1;
        let pane_id = *state.workspaces[0].panes.keys().next().unwrap();

        state.handle_app_event(AppEvent::StateChanged {
            pane_id,
            agent: Some(Agent::Pi),
            state: AgentState::Blocked,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });

        let deadline = state.next_pending_agent_notification_deadline().unwrap();
        let deliveries = state.drain_due_agent_notifications(deadline);

        assert_eq!(deliveries.len(), 1);
        assert!(deliveries[0].toast.is_none());
        assert!(deliveries[0].client_notification.is_some());
        assert!(state.toast.is_none());
    }

    #[test]
    fn delayed_background_waiting_is_cleared_when_pane_dies() {
        let mut state = app_with_workspaces(&["active", "background"]);
        state.active = Some(0);
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        state.toast_config.delay_seconds = 1;
        let bg_pane_id = *state.workspaces[1].panes.keys().next().unwrap();

        state.handle_app_event(AppEvent::StateChanged {
            pane_id: bg_pane_id,
            agent: Some(Agent::Pi),
            state: AgentState::Blocked,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });
        let deadline = state.next_pending_agent_notification_deadline().unwrap();
        state.handle_app_event(AppEvent::PaneDied {
            pane_id: bg_pane_id,
        });

        assert!(state.pending_agent_notifications.is_empty());
        assert!(state.drain_due_agent_notifications(deadline).is_empty());
        assert!(state.toast.is_none());
    }

    #[test]
    fn hook_reported_unknown_agent_sets_toast_title_from_label() {
        let mut state = app_with_workspaces(&["active", "background"]);
        state.active = Some(0);
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        let bg_pane_id = *state.workspaces[1].panes.keys().next().unwrap();

        state.handle_app_event(AppEvent::HookStateReported {
            pane_id: bg_pane_id,
            source: "custom:hermes".into(),
            agent_label: "hermes".into(),
            state: AgentState::Blocked,
            message: None,
            custom_status: None,
            seq: None,
            session_ref: None,
        });

        let toast = state.toast.as_ref().unwrap();
        assert_eq!(toast.kind, ToastKind::NeedsAttention);
        assert_eq!(toast.title, "hermes needs attention");
        assert_eq!(toast.context, "background · 2");
    }

    #[test]
    fn visible_blocker_overrides_hook_working_and_notifies() {
        let mut state = app_with_workspaces(&["active", "background"]);
        state.active = Some(0);
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        let bg_pane_id = *state.workspaces[1].panes.keys().next().unwrap();
        let bg_terminal_id = state.workspaces[1]
            .panes
            .get(&bg_pane_id)
            .unwrap()
            .attached_terminal_id
            .clone();

        state.handle_app_event(AppEvent::StateChanged {
            pane_id: bg_pane_id,
            agent: Some(Agent::Codex),
            state: AgentState::Idle,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });
        state.handle_app_event(AppEvent::HookStateReported {
            pane_id: bg_pane_id,
            source: "herdr:codex".into(),
            agent_label: "codex".into(),
            state: AgentState::Working,
            message: None,
            custom_status: None,
            seq: Some(1),
            session_ref: None,
        });
        state.handle_app_event(AppEvent::StateChanged {
            pane_id: bg_pane_id,
            agent: Some(Agent::Codex),
            state: AgentState::Blocked,
            visible_blocker: true,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });

        let terminal = state.terminals.get(&bg_terminal_id).unwrap();
        assert_eq!(terminal.state, AgentState::Blocked);
        let toast = state.toast.as_ref().unwrap();
        assert_eq!(toast.kind, ToastKind::NeedsAttention);
        assert_eq!(toast.title, "codex needs attention");
    }

    #[test]
    fn reserved_native_state_report_does_not_override_screen_state() {
        let mut state = app_with_workspaces(&["active"]);
        state.active = Some(0);
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        let pane_id = *state.workspaces[0].panes.keys().next().unwrap();
        let terminal_id = state.workspaces[0]
            .panes
            .get(&pane_id)
            .unwrap()
            .attached_terminal_id
            .clone();

        state.handle_app_event(AppEvent::StateChanged {
            pane_id,
            agent: Some(Agent::Claude),
            state: AgentState::Working,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });
        state.handle_app_event(AppEvent::HookStateReported {
            pane_id,
            source: "herdr:claude".into(),
            agent_label: "claude".into(),
            state: AgentState::Blocked,
            message: None,
            custom_status: None,
            seq: Some(1),
            session_ref: crate::agent_resume::AgentSessionRef::id("claude-session"),
        });
        let terminal = state.terminals.get(&terminal_id).unwrap();
        assert_eq!(terminal.state, AgentState::Working);
        assert!(terminal.hook_authority.is_none());
        assert!(terminal.persisted_agent_session.is_some());

        state.handle_app_event(AppEvent::StateChanged {
            pane_id,
            agent: Some(Agent::Claude),
            state: AgentState::Idle,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });

        let terminal = state.terminals.get(&terminal_id).unwrap();
        assert_eq!(terminal.state, AgentState::Idle);
        assert!(state.toast.is_none());
    }

    #[test]
    fn reserved_native_release_report_does_not_clear_screen_state() {
        let mut state = app_with_workspaces(&["active"]);
        let pane_id = *state.workspaces[0].panes.keys().next().unwrap();
        let terminal_id = state.workspaces[0]
            .panes
            .get(&pane_id)
            .unwrap()
            .attached_terminal_id
            .clone();

        state.handle_app_event(AppEvent::StateChanged {
            pane_id,
            agent: Some(Agent::Claude),
            state: AgentState::Working,
            visible_blocker: false,
            visible_working: true,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });
        state.handle_app_event(AppEvent::HookAgentReleased {
            pane_id,
            source: "herdr:claude".into(),
            agent_label: "claude".into(),
            known_agent: Some(Agent::Claude),
            seq: Some(1),
        });

        let terminal = state.terminals.get(&terminal_id).unwrap();
        assert_eq!(terminal.state, AgentState::Working);
        assert_eq!(terminal.detected_agent, Some(Agent::Claude));
    }

    #[test]
    fn devin_state_report_refreshes_session_without_overriding_screen_state() {
        let mut state = app_with_workspaces(&["active"]);
        let pane_id = *state.workspaces[0].panes.keys().next().unwrap();
        let terminal_id = state.workspaces[0]
            .panes
            .get(&pane_id)
            .unwrap()
            .attached_terminal_id
            .clone();

        state.handle_app_event(AppEvent::StateChanged {
            pane_id,
            agent: Some(Agent::Devin),
            state: AgentState::Idle,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });
        state.handle_app_event(AppEvent::HookStateReported {
            pane_id,
            source: "herdr:devin".into(),
            agent_label: "devin".into(),
            state: AgentState::Working,
            message: None,
            custom_status: None,
            seq: Some(1),
            session_ref: crate::agent_resume::AgentSessionRef::id("devin-session"),
        });

        let terminal = state.terminals.get(&terminal_id).unwrap();
        assert_eq!(terminal.state, AgentState::Idle);
        assert!(terminal.hook_authority.is_none());
        assert!(terminal.persisted_agent_session.is_some());
    }

    #[test]
    fn hidden_custom_session_ref_only_update_marks_session_dirty_without_visible_update() {
        let mut state = app_with_workspaces(&["active"]);
        let pane_id = *state.workspaces[0].panes.keys().next().unwrap();
        let test_dir = std::env::current_dir().unwrap();
        let first_session = test_dir.join("one.jsonl").display().to_string();
        let second_session = test_dir.join("two.jsonl").display().to_string();

        let first_updates = state.handle_app_event(AppEvent::HookStateReported {
            pane_id,
            source: "custom:pi".into(),
            agent_label: "pi".into(),
            state: AgentState::Working,
            message: None,
            custom_status: None,
            seq: Some(20),
            session_ref: crate::agent_resume::AgentSessionRef::path(first_session),
        });
        assert_eq!(first_updates.len(), 1);
        state.session_dirty = false;

        let second_updates = state.handle_app_event(AppEvent::HookStateReported {
            pane_id,
            source: "custom:pi".into(),
            agent_label: "pi".into(),
            state: AgentState::Working,
            message: None,
            custom_status: None,
            seq: Some(21),
            session_ref: crate::agent_resume::AgentSessionRef::path(second_session),
        });

        assert!(second_updates.is_empty());
        assert!(state.session_dirty);
    }

    #[test]
    fn terminal_cwd_report_updates_terminal_cwd_and_marks_session_dirty() {
        let mut state = app_with_workspaces(&["active"]);
        let pane_id = *state.workspaces[0].panes.keys().next().unwrap();
        let terminal_id = state.workspaces[0]
            .pane_state(pane_id)
            .unwrap()
            .attached_terminal_id
            .clone();
        let cwd =
            std::env::temp_dir().join(format!("herdr-cwd-report-test-{}", std::process::id()));
        std::fs::create_dir_all(&cwd).unwrap();
        state.session_dirty = false;

        let updates = state.handle_app_event(AppEvent::TerminalCwdReported {
            pane_id,
            cwd: cwd.clone(),
        });

        assert!(updates.is_empty());
        assert_eq!(state.terminals.get(&terminal_id).unwrap().cwd, cwd);
        assert!(state.session_dirty);
        let _ = std::fs::remove_dir_all(cwd);
    }

    #[test]
    fn background_idle_sets_finished_toast() {
        let mut state = app_with_workspaces(&["active", "background"]);
        state.active = Some(0);
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        let bg_pane_id = *state.workspaces[1].panes.keys().next().unwrap();
        let bg_terminal_id = state.workspaces[1]
            .panes
            .get(&bg_pane_id)
            .unwrap()
            .attached_terminal_id
            .clone();
        state.terminals.get_mut(&bg_terminal_id).unwrap().state = AgentState::Working;

        state.handle_app_event(AppEvent::StateChanged {
            pane_id: bg_pane_id,
            agent: Some(Agent::Droid),
            state: AgentState::Idle,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });

        let toast = state.toast.as_ref().unwrap();
        assert_eq!(toast.kind, ToastKind::Finished);
        assert_eq!(toast.title, "droid finished");
        assert_eq!(toast.context, "background · 2");
        let target = toast.target.as_ref().expect("toast target");
        assert_eq!(&target.workspace_id, &state.workspaces[1].id);
        assert_eq!(target.pane_id, bg_pane_id);
    }

    #[test]
    fn background_toast_includes_tab_name_when_workspace_has_multiple_tabs() {
        let mut state = app_with_workspaces(&["active", "background"]);
        state.active = Some(0);
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        state.workspaces[1].tabs[0].set_custom_name("main".into());
        let second_tab = state.workspaces[1].test_add_tab(Some("logs"));
        state.ensure_test_terminals();
        let bg_pane_id = state.workspaces[1].tabs[second_tab].root_pane;

        state.handle_app_event(AppEvent::StateChanged {
            pane_id: bg_pane_id,
            agent: Some(Agent::Pi),
            state: AgentState::Blocked,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });

        let toast = state.toast.as_ref().unwrap();
        assert_eq!(toast.kind, ToastKind::NeedsAttention);
        assert_eq!(toast.title, "pi needs attention");
        assert_eq!(toast.context, "background · 2 · logs");
    }

    #[test]
    fn background_tab_in_active_workspace_still_sets_toast() {
        let mut state = app_with_workspaces(&["active"]);
        state.active = Some(0);
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        state.workspaces[0].tabs[0].set_custom_name("main".into());
        let second_tab = state.workspaces[0].test_add_tab(Some("logs"));
        state.ensure_test_terminals();
        let bg_pane_id = state.workspaces[0].tabs[second_tab].root_pane;

        state.handle_app_event(AppEvent::StateChanged {
            pane_id: bg_pane_id,
            agent: Some(Agent::Pi),
            state: AgentState::Blocked,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });

        let toast = state.toast.as_ref().unwrap();
        assert_eq!(toast.kind, ToastKind::NeedsAttention);
        assert_eq!(toast.title, "pi needs attention");
        assert_eq!(toast.context, "active · 1 · logs");
    }

    #[test]
    fn active_workspace_active_tab_does_not_set_toast() {
        let mut state = app_with_workspaces(&["active"]);
        state.active = Some(0);
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        let pane_id = *state.workspaces[0].panes.keys().next().unwrap();

        state.handle_app_event(AppEvent::StateChanged {
            pane_id,
            agent: Some(Agent::Pi),
            state: AgentState::Blocked,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });

        assert!(state.toast.is_none());
    }

    #[test]
    fn active_workspace_active_tab_keeps_herdr_toast_suppressed_when_outer_terminal_is_unfocused() {
        let mut state = app_with_workspaces(&["active"]);
        state.active = Some(0);
        state.outer_terminal_focus = Some(false);
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        let pane_id = *state.workspaces[0].panes.keys().next().unwrap();

        state.handle_app_event(AppEvent::StateChanged {
            pane_id,
            agent: Some(Agent::Pi),
            state: AgentState::Blocked,
            visible_blocker: false,
            visible_working: false,
            process_exited: false,
            observed_at: std::time::Instant::now(),
        });

        assert!(state.toast.is_none());
    }

    #[test]
    fn active_tab_suppression_preserves_unknown_focus_behavior() {
        assert!(active_tab_suppresses_notifications(true, None));
        assert!(active_tab_suppresses_notifications(true, Some(true)));
        assert!(!active_tab_suppresses_notifications(true, Some(false)));
        assert!(!active_tab_suppresses_notifications(false, None));
    }

    #[test]
    fn update_ready_sets_manual_update_toast() {
        let mut state = AppState::test_new();
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;

        let updates = state.handle_app_event(AppEvent::UpdateReady {
            version: "0.5.0".into(),
            install_command: "herdr update".into(),
        });

        assert!(updates.is_empty());
        assert_eq!(state.update_available.as_deref(), Some("0.5.0"));
        assert!(state.latest_release_notes_available);
        assert!(state.update_dismissed);
        let toast = state.toast.as_ref().expect("update toast");
        assert_eq!(toast.kind, ToastKind::UpdateInstalled);
        assert_eq!(toast.title, "v0.5.0 available");
        assert_eq!(
            toast.context,
            "detach, run `herdr update`, then follow its restart guidance"
        );
    }

    #[test]
    fn update_ready_uses_event_install_command_in_toast() {
        let mut state = AppState::test_new();
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;

        state.handle_app_event(AppEvent::UpdateReady {
            version: "0.5.0".into(),
            install_command: "brew update && brew upgrade herdr".into(),
        });

        assert_eq!(
            state.update_install_command,
            "brew update && brew upgrade herdr"
        );
        let toast = state.toast.as_ref().expect("update toast");
        assert_eq!(
            toast.context,
            "detach, run `brew update && brew upgrade herdr`, then restart this Herdr session when ready"
        );
    }

    #[test]
    fn agent_detection_manifest_update_event_updates_status_and_toast() {
        let mut state = AppState::test_new();
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        let status = crate::detect::manifest_update::ManifestUpdateStatus {
            last_result: Some("checked".to_string()),
            ..Default::default()
        };

        let updates = state.handle_app_event(AppEvent::AgentDetectionManifestsUpdated {
            updated: vec![crate::detect::manifest_update::ManifestUpdateCommit {
                agent: Agent::Codex,
                version: crate::detect::manifest_update::ManifestVersion::parse("2026.06.10.1")
                    .unwrap(),
            }],
            status,
        });

        assert!(updates.is_empty());
        assert_eq!(
            state.agent_manifest_update_status.last_result.as_deref(),
            Some("checked")
        );
        let toast = state.toast.as_ref().expect("manifest update toast");
        assert_eq!(toast.kind, ToastKind::UpdateInstalled);
        assert_eq!(toast.title, "Agent detection rules updated");
        assert_eq!(toast.context, "codex 2026.06.10.1");
    }

    #[test]
    fn toggle_zoom_works() {
        let mut state = app_with_workspaces(&["test"]);
        state.workspaces[0].test_split(Direction::Horizontal);

        assert!(!state.workspaces[0].zoomed);
        state.toggle_zoom();
        assert!(state.workspaces[0].zoomed);
        state.toggle_zoom();
        assert!(!state.workspaces[0].zoomed);
    }

    #[test]
    fn toggle_zoom_single_pane_noop() {
        let mut state = app_with_workspaces(&["test"]);
        state.toggle_zoom();
        assert!(!state.workspaces[0].zoomed);
    }

    #[test]
    fn navigate_pane_changes_focus_while_zoomed() {
        let mut state = app_with_workspaces(&["test"]);
        let root = state.workspaces[0].tabs[0].root_pane;
        let right = state.workspaces[0].test_split(Direction::Horizontal);
        state.workspaces[0].layout.focus_pane(root);
        state.workspaces[0].zoomed = true;
        crate::ui::compute_view(&mut state, ratatui::layout::Rect::new(0, 0, 100, 20));

        assert_eq!(state.view.pane_infos.len(), 1);
        assert_eq!(state.view.pane_infos[0].id, root);

        state.navigate_pane(NavDirection::Right);
        crate::ui::compute_view(&mut state, ratatui::layout::Rect::new(0, 0, 100, 20));

        assert!(state.workspaces[0].zoomed);
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(right));
        assert_eq!(state.view.pane_infos.len(), 1);
        assert_eq!(state.view.pane_infos[0].id, right);
        assert!(state.view.pane_infos[0].inner_rect.x > state.view.pane_infos[0].rect.x);
    }

    #[test]
    fn swap_pane_direction_preserves_focus_and_swaps_layout_cells() {
        let mut state = app_with_workspaces(&["test"]);
        let root = state.workspaces[0].tabs[0].root_pane;
        let right = state.workspaces[0].test_split(Direction::Horizontal);
        state.workspaces[0].layout.focus_pane(root);
        crate::ui::compute_view(&mut state, ratatui::layout::Rect::new(0, 0, 100, 20));
        let before_root_rect = state
            .view
            .pane_infos
            .iter()
            .find(|info| info.id == root)
            .unwrap()
            .rect;
        let before_right_rect = state
            .view
            .pane_infos
            .iter()
            .find(|info| info.id == right)
            .unwrap()
            .rect;

        assert!(state.swap_pane(NavDirection::Right));
        crate::ui::compute_view(&mut state, ratatui::layout::Rect::new(0, 0, 100, 20));

        assert_eq!(state.workspaces[0].focused_pane_id(), Some(root));
        assert_eq!(
            state
                .view
                .pane_infos
                .iter()
                .find(|info| info.id == root)
                .unwrap()
                .rect,
            before_right_rect
        );
        assert_eq!(
            state
                .view
                .pane_infos
                .iter()
                .find(|info| info.id == right)
                .unwrap()
                .rect,
            before_root_rect
        );
    }

    #[test]
    fn swap_pane_direction_stays_zoomed_and_mutates_hidden_layout() {
        let mut state = app_with_workspaces(&["test"]);
        let root = state.workspaces[0].tabs[0].root_pane;
        let right = state.workspaces[0].test_split(Direction::Horizontal);
        state.workspaces[0].layout.focus_pane(root);
        state.workspaces[0].zoomed = true;
        crate::ui::compute_view(&mut state, ratatui::layout::Rect::new(0, 0, 100, 20));

        assert!(state.swap_pane(NavDirection::Right));
        crate::ui::compute_view(&mut state, ratatui::layout::Rect::new(0, 0, 100, 20));

        assert!(state.workspaces[0].zoomed);
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(root));
        assert_eq!(state.view.pane_infos.len(), 1);
        assert_eq!(state.view.pane_infos[0].id, root);

        state.workspaces[0].zoomed = false;
        crate::ui::compute_view(&mut state, ratatui::layout::Rect::new(0, 0, 100, 20));
        let root_rect = state
            .view
            .pane_infos
            .iter()
            .find(|info| info.id == root)
            .unwrap()
            .rect;
        let right_rect = state
            .view
            .pane_infos
            .iter()
            .find(|info| info.id == right)
            .unwrap()
            .rect;

        assert!(root_rect.x > right_rect.x);
    }

    #[test]
    fn close_pane_removes_from_workspace() {
        let mut state = app_with_workspaces(&["test"]);
        let closed = state.workspaces[0].test_split(Direction::Horizontal);
        state.ensure_test_terminals();
        assert_eq!(state.workspaces[0].panes.len(), 2);
        state.plugin_panes.insert(
            closed,
            crate::app::state::PluginPaneRecord {
                plugin_id: "example.pane".into(),
                entrypoint: "board".into(),
            },
        );

        state.close_pane();
        assert_eq!(state.workspaces[0].panes.len(), 1);
        assert!(!state.plugin_panes.contains_key(&closed));
        state.assert_invariants_for_test();
    }

    #[test]
    fn pane_process_exit_publish_marks_agent_idle_before_pane_removal() {
        let mut state = app_with_workspaces(&["active", "background"]);
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        state.active = Some(1);
        state.ensure_test_terminals();
        let pane_id = state.workspaces[0].tabs[0].root_pane;
        let terminal_id = state.terminal_id_for_pane(0, pane_id).unwrap();
        state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .set_detected_state(Some(Agent::Pi), AgentState::Working);
        assert_eq!(
            state.terminals.get(&terminal_id).unwrap().state,
            AgentState::Working
        );

        let update = state
            .publish_pane_process_exit_if_agent(pane_id)
            .expect("process exit update");

        assert!(!state.pane_is_in_active_tab(update.ws_idx, pane_id));
        assert_eq!(update.previous_state, AgentState::Working);
        assert_eq!(update.state, AgentState::Idle);
        assert_eq!(update.agent_label.as_deref(), Some("pi"));
        assert_eq!(update.known_agent, Some(Agent::Pi));
        assert!(matches!(
            state.toast.as_ref().map(|toast| toast.kind),
            Some(ToastKind::Finished)
        ));
    }

    #[test]
    fn close_pane_removes_unattached_terminal_state() {
        let mut state = app_with_workspaces(&["test"]);
        let pane_id = state.workspaces[0].test_split(Direction::Horizontal);
        state.ensure_test_terminals();
        let terminal_id = state.terminal_id_for_pane(0, pane_id).unwrap();

        state.close_pane();

        assert!(!state.terminals.contains_key(&terminal_id));
        state.assert_invariants_for_test();
    }

    #[test]
    fn close_tab_removes_unattached_terminal_states() {
        let mut state = app_with_workspaces(&["test"]);
        let tab_idx = state.workspaces[0].test_add_tab(Some("logs"));
        state.ensure_test_terminals();
        state.workspaces[0].switch_tab(tab_idx);
        let pane_id = state.workspaces[0].tabs[tab_idx].root_pane;
        let terminal_id = state.terminal_id_for_pane(0, pane_id).unwrap();
        state.plugin_panes.insert(
            pane_id,
            crate::app::state::PluginPaneRecord {
                plugin_id: "example.pane".into(),
                entrypoint: "board".into(),
            },
        );

        state.close_tab();

        assert!(!state.terminals.contains_key(&terminal_id));
        assert!(!state.plugin_panes.contains_key(&pane_id));
        state.assert_invariants_for_test();
    }

    #[test]
    fn close_workspace_removes_unattached_terminal_states() {
        let mut state = app_with_workspaces(&["one", "two"]);
        let pane_id = state.workspaces[0].tabs[0].root_pane;
        let terminal_id = state.terminal_id_for_pane(0, pane_id).unwrap();
        state.plugin_panes.insert(
            pane_id,
            crate::app::state::PluginPaneRecord {
                plugin_id: "example.pane".into(),
                entrypoint: "board".into(),
            },
        );

        state.close_selected_workspace();

        assert!(!state.terminals.contains_key(&terminal_id));
        assert!(!state.plugin_panes.contains_key(&pane_id));
        state.assert_invariants_for_test();
    }

    #[test]
    fn close_tab_closes_active_workspace_not_selected_workspace() {
        let mut state = app_with_workspaces(&["selected", "active"]);
        let active_terminal_id = state
            .terminal_id_for_pane(1, state.workspaces[1].tabs[0].root_pane)
            .unwrap();
        state.active = Some(1);
        state.selected = 0;

        state.close_tab();

        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.workspaces[0].display_name(), "selected");
        assert!(!state.terminals.contains_key(&active_terminal_id));
        state.assert_invariants_for_test();
    }

    #[test]
    fn close_pane_last_pane_closes_active_workspace_not_selected_workspace() {
        let mut state = app_with_workspaces(&["selected", "active"]);
        let active_terminal_id = state
            .terminal_id_for_pane(1, state.workspaces[1].tabs[0].root_pane)
            .unwrap();
        state.active = Some(1);
        state.selected = 0;

        state.close_pane();

        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.workspaces[0].display_name(), "selected");
        assert!(!state.terminals.contains_key(&active_terminal_id));
        state.assert_invariants_for_test();
    }

    #[test]
    fn close_pane_last_pane_in_parent_worktree_group_prompts() {
        let mut state = app_with_workspaces(&["parent", "child"]);
        mark_parent_worktree(&mut state, 0);
        mark_linked_worktree(&mut state, 1);
        state.active = Some(0);
        state.selected = 1;

        let deferred = state.close_pane();

        assert!(deferred);
        assert_eq!(state.mode, Mode::ConfirmClose);
        assert_eq!(state.selected, 0);
        assert_eq!(state.workspaces.len(), 2);
    }

    #[test]
    fn close_tab_in_linked_worktree_closes_workspace_only() {
        let mut state = app_with_workspaces(&["selected", "active"]);
        mark_linked_worktree(&mut state, 1);
        state.active = Some(1);
        state.selected = 0;

        state.close_tab();

        assert_eq!(state.request_remove_linked_worktree, None);
        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.workspaces[0].display_name(), "selected");
    }

    #[test]
    fn close_tab_last_tab_in_parent_worktree_group_prompts() {
        let mut state = app_with_workspaces(&["parent", "child"]);
        mark_parent_worktree(&mut state, 0);
        mark_linked_worktree(&mut state, 1);
        state.active = Some(0);
        state.selected = 1;

        let deferred = state.close_tab();

        assert!(deferred);
        assert_eq!(state.mode, Mode::ConfirmClose);
        assert_eq!(state.selected, 0);
        assert_eq!(state.workspaces.len(), 2);
    }

    #[test]
    fn close_pane_last_pane_in_linked_worktree_closes_workspace_only() {
        let mut state = app_with_workspaces(&["selected", "active"]);
        mark_linked_worktree(&mut state, 1);
        state.active = Some(1);
        state.selected = 0;

        state.close_pane();

        assert_eq!(state.request_remove_linked_worktree, None);
        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.workspaces[0].display_name(), "selected");
    }

    #[test]
    fn close_pane_last_pane_in_parent_worktree_group_closes_when_confirmation_disabled() {
        let mut state = app_with_workspaces(&["parent", "child", "notes"]);
        mark_parent_worktree(&mut state, 0);
        mark_linked_worktree(&mut state, 1);
        state.confirm_close = false;
        state.active = Some(0);
        state.selected = 0;

        let deferred = state.close_pane();

        assert!(!deferred);
        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.workspaces[0].display_name(), "notes");
    }
}
