use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
#[cfg(test)]
use ratatui::layout::Direction;
use ratatui::layout::Rect;

use crate::{
    app::{
        state::{
            AppState, ContextMenuKind, ContextMenuState, MenuListState, Mode, NavigatorStateFilter,
        },
        App,
    },
    input::TerminalKey,
    layout::NavDirection,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModalAction {
    Continue,
    Save,
    Clear,
    Cancel,
    Confirm,
    Apply,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModalKeyBinding {
    Enter,
    Esc,
    CtrlC,
}

impl ModalKeyBinding {
    fn matches(self, key: &KeyEvent) -> bool {
        match self {
            Self::Enter => key.code == KeyCode::Enter,
            Self::Esc => key.code == KeyCode::Esc,
            Self::CtrlC => {
                key.code == KeyCode::Char('c')
                    && key.modifiers == crossterm::event::KeyModifiers::CONTROL
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ModalActionSpec<A> {
    pub action: A,
    pub bindings: &'static [ModalKeyBinding],
}

pub(super) fn modal_action_from_key<A: Copy>(
    key: &KeyEvent,
    specs: &[ModalActionSpec<A>],
) -> Option<A> {
    specs
        .iter()
        .find(|spec| spec.bindings.iter().any(|binding| binding.matches(key)))
        .map(|spec| spec.action)
}

pub(super) fn modal_action_from_buttons<A: Copy>(
    col: u16,
    row: u16,
    buttons: &[(Rect, A)],
) -> Option<A> {
    buttons.iter().find_map(|(rect, action)| {
        (col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height)
            .then_some(*action)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GlobalMenuAction {
    Detach,
    WhatsNew,
    Keybinds,
    ReloadConfig,
    Settings,
}

pub(super) fn global_menu_actions(state: &AppState) -> Vec<GlobalMenuAction> {
    let mut actions = vec![
        GlobalMenuAction::Settings,
        GlobalMenuAction::Keybinds,
        GlobalMenuAction::ReloadConfig,
    ];
    if state.update_available.is_some() || state.latest_release_notes_available {
        actions.push(GlobalMenuAction::WhatsNew);
    }
    actions.push(GlobalMenuAction::Detach);
    actions
}

pub(super) fn open_global_menu(state: &mut AppState) {
    state.global_menu = MenuListState::new(0);
    state.mode = Mode::GlobalMenu;
}

pub(super) fn open_keybind_help(state: &mut AppState) {
    state.keybind_help.scroll = 0;
    state.mode = Mode::KeybindHelp;
}

fn open_update_release_notes(state: &mut AppState) {
    let Some(notes) = crate::release_notes::load_latest() else {
        return;
    };

    state.release_notes = Some(crate::app::state::ReleaseNotesState {
        version: notes.version,
        body: notes.body,
        scroll: 0,
        preview: notes.preview,
    });
    state.mode = Mode::ReleaseNotes;
}

pub(super) fn request_detach(state: &mut AppState) {
    if state.detach_exits {
        state.should_quit = true;
    } else {
        state.detach_requested = true;
    }
}

pub(super) fn apply_global_menu_action(state: &mut AppState, action: GlobalMenuAction) {
    match action {
        GlobalMenuAction::Detach => {
            leave_modal(state);
            request_detach(state);
        }
        GlobalMenuAction::WhatsNew => open_update_release_notes(state),
        GlobalMenuAction::Keybinds => open_keybind_help(state),
        GlobalMenuAction::ReloadConfig => {
            state.request_reload_config = true;
            leave_modal(state);
        }
        GlobalMenuAction::Settings => super::settings::open_settings(state),
    }
}

pub(crate) fn handle_global_menu_key(state: &mut AppState, key: KeyEvent) {
    let actions = global_menu_actions(state);
    match key.code {
        KeyCode::Esc => leave_modal(state),
        KeyCode::Up | KeyCode::Char('k') => state.global_menu.move_prev(),
        KeyCode::Down | KeyCode::Char('j') => state.global_menu.move_next(actions.len()),
        KeyCode::Enter => {
            if let Some(action) = actions.get(state.global_menu.highlighted).copied() {
                apply_global_menu_action(state, action);
            }
        }
        _ => {}
    }
}

pub(crate) fn handle_navigator_key(
    state: &mut AppState,
    terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    key: KeyEvent,
) {
    if state.navigator.search_focused {
        match key.code {
            KeyCode::Esc => {
                state.navigator.query.clear();
                state.navigator.state_filter = None;
                state.navigator.search_focused = false;
                state.clamp_navigator_selection_from(terminal_runtimes);
            }
            KeyCode::Enter => {
                state.accept_navigator_selection_from(terminal_runtimes);
            }
            KeyCode::Backspace => {
                state.navigator.state_filter = None;
                state.navigator.query.pop();
                state.clamp_navigator_selection_from(terminal_runtimes);
            }
            KeyCode::Up => state.move_navigator_selection_from(terminal_runtimes, -1),
            KeyCode::Down => state.move_navigator_selection_from(terminal_runtimes, 1),
            KeyCode::Char('n') if key.modifiers == KeyModifiers::CONTROL => {
                state.move_navigator_selection_from(terminal_runtimes, 1)
            }
            KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                state.move_navigator_selection_from(terminal_runtimes, -1)
            }
            KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                state.navigator.query.clear();
                state.navigator.state_filter = None;
                state.clamp_navigator_selection_from(terminal_runtimes);
            }
            KeyCode::Char(c)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                insert_navigator_search_text(state, terminal_runtimes, &c.to_string());
            }
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Esc => {
            if state.navigator.query.is_empty() && state.navigator.state_filter.is_none() {
                leave_modal(state);
            } else {
                state.navigator.query.clear();
                state.navigator.state_filter = None;
                state.clamp_navigator_selection_from(terminal_runtimes);
            }
        }
        KeyCode::Enter => {
            state.accept_navigator_selection_from(terminal_runtimes);
        }
        KeyCode::Char('/') => {
            state.navigator.query.clear();
            state.navigator.state_filter = None;
            state.navigator.search_focused = true;
            state.clamp_navigator_selection_from(terminal_runtimes);
        }
        KeyCode::Backspace if state.navigator.state_filter.is_some() => {
            state.navigator.state_filter = None;
            state.clamp_navigator_selection_from(terminal_runtimes);
        }
        KeyCode::Char('a') if key.modifiers.is_empty() => {
            state.navigator.query.clear();
            state.navigator.state_filter = None;
            state.clamp_navigator_selection_from(terminal_runtimes);
        }
        KeyCode::Char('b') if key.modifiers.is_empty() => {
            state.navigator.query.clear();
            state.navigator.state_filter = Some(NavigatorStateFilter::Blocked);
            state.clamp_navigator_selection_from(terminal_runtimes);
        }
        KeyCode::Char('w') if key.modifiers.is_empty() => {
            state.navigator.query.clear();
            state.navigator.state_filter = Some(NavigatorStateFilter::Working);
            state.clamp_navigator_selection_from(terminal_runtimes);
        }
        KeyCode::Char('i') if key.modifiers.is_empty() => {
            state.navigator.query.clear();
            state.navigator.state_filter = Some(NavigatorStateFilter::Idle);
            state.clamp_navigator_selection_from(terminal_runtimes);
        }
        KeyCode::Char('d') if key.modifiers.is_empty() => {
            state.navigator.query.clear();
            state.navigator.state_filter = Some(NavigatorStateFilter::Done);
            state.clamp_navigator_selection_from(terminal_runtimes);
        }
        KeyCode::Char('j') | KeyCode::Down => {
            state.move_navigator_selection_from(terminal_runtimes, 1)
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.move_navigator_selection_from(terminal_runtimes, -1)
        }
        KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => state
            .move_navigator_selection_from(
                terminal_runtimes,
                (state.navigator_body_rect().height / 2).max(1) as isize,
            ),
        KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => state
            .move_navigator_selection_from(
                terminal_runtimes,
                -((state.navigator_body_rect().height / 2).max(1) as isize),
            ),
        KeyCode::Char(' ') => state.toggle_selected_navigator_workspace_from(terminal_runtimes),
        KeyCode::Home => {
            state.navigator.selected = 0;
            state.ensure_navigator_selection_visible_from(terminal_runtimes);
        }
        KeyCode::End | KeyCode::Char('G') => {
            state.navigator.selected = state
                .navigator_rows_from(terminal_runtimes)
                .len()
                .saturating_sub(1);
            state.ensure_navigator_selection_visible_from(terminal_runtimes);
        }
        _ => {}
    }
}

pub(crate) fn insert_navigator_search_text(
    state: &mut AppState,
    terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    text: &str,
) {
    if !state.navigator.search_focused {
        return;
    }
    state.navigator.state_filter = None;
    state.navigator.query.push_str(text);
    state.clamp_navigator_selection_from(terminal_runtimes);
}

pub(crate) fn handle_keybind_help_key(state: &mut AppState, key: KeyEvent) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => state.scroll_keybind_help(-1),
        KeyCode::Down | KeyCode::Char('j') => state.scroll_keybind_help(1),
        KeyCode::PageUp => state.scroll_keybind_help(-8),
        KeyCode::PageDown => state.scroll_keybind_help(8),
        KeyCode::Home => state.keybind_help.scroll = 0,
        KeyCode::End => state.keybind_help.scroll = state.keybind_help_max_scroll(),
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') => leave_modal(state),
        _ => {}
    }
}

pub(super) fn open_rename_workspace(
    state: &mut AppState,
    terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    ws_idx: usize,
) {
    state.selected = ws_idx;
    state.rename_pane_target = None;
    state.rename_tab_target = None;
    let name = state.workspaces[ws_idx].display_name_from(&state.terminals, terminal_runtimes);
    state.set_name_input(name);
    state.name_input_replace_on_type = false;
    state.mode = Mode::RenameWorkspace;
}

pub(super) fn open_rename_active_tab(state: &mut AppState, replace_on_type: bool) {
    state.creating_new_tab = false;
    state.requested_new_tab_name = None;
    state.rename_pane_target = None;
    state.rename_tab_target = None;
    if let Some(ws) = state.active.and_then(|i| state.workspaces.get(i)) {
        if let Some(name) = ws.active_tab_display_name() {
            state.set_name_input(name);
            state.name_input_replace_on_type = replace_on_type;
            state.mode = Mode::RenameTab;
        }
    }
}

/// Open the tab rename modal targeting an explicit `(ws_idx, tab_idx)`, which may
/// be a non-active tab in any workspace. The RenameTab commit prefers
/// `rename_tab_target` when set.
pub(super) fn open_rename_tab(state: &mut AppState, ws_idx: usize, tab_idx: usize) {
    let Some(ws) = state.workspaces.get(ws_idx) else {
        return;
    };
    let Some(name) = ws.tab_display_name(tab_idx) else {
        return;
    };
    state.creating_new_tab = false;
    state.requested_new_tab_name = None;
    state.rename_pane_target = None;
    state.rename_tab_target = Some((ws_idx, tab_idx));
    state.set_name_input(name);
    state.name_input_replace_on_type = false;
    state.mode = Mode::RenameTab;
}

pub(super) fn open_rename_pane(state: &mut AppState, pane_id: crate::layout::PaneId) {
    let Some(ws_idx) = state.active else {
        return;
    };
    open_rename_pane_in_workspace(state, ws_idx, pane_id);
}

/// Open the pane rename modal targeting an explicit `(ws_idx, pane_id)`, which
/// may be a pane in a non-active workspace (used by the sidebar Panes section,
/// which lists panes across spaces). The RenamePane commit resolves the pane by
/// its globally unique id, so it renames the right pane regardless of which
/// workspace is active.
pub(super) fn open_rename_pane_in_workspace(
    state: &mut AppState,
    ws_idx: usize,
    pane_id: crate::layout::PaneId,
) {
    let Some(ws) = state.workspaces.get(ws_idx) else {
        return;
    };
    let Some(pane) = ws.pane_state(pane_id) else {
        return;
    };
    let manual_label = state
        .terminals
        .get(&pane.attached_terminal_id)
        .and_then(|terminal| terminal.manual_label.clone());
    state.creating_new_tab = false;
    state.requested_new_tab_name = None;
    state.rename_pane_target = Some(pane_id);
    state.rename_tab_target = None;
    // Keep the renamed pane's row visible in the Panes section if it is listed.
    state.ensure_pane_section_row_visible(pane_id);
    let replace_on_type = manual_label.is_none();
    state.set_name_input(manual_label.unwrap_or_default());
    state.name_input_replace_on_type = replace_on_type;
    state.mode = Mode::RenamePane;
}

/// Open the agent rename modal (`Mode::RenameAgent`). No longer reachable from
/// the TUI double-click (which now renames the tab), but retained to keep the
/// `Mode::RenameAgent` commit path (and the enum variant it constructs) alive
/// alongside the live `herdr agent rename` API/CLI flow. Currently exercised
/// only by tests.
#[allow(
    dead_code,
    reason = "keeps Mode::RenameAgent + its commit path constructed; TUI entry now renames the tab"
)]
pub(super) fn open_rename_agent(
    state: &mut AppState,
    ws_idx: usize,
    pane_id: crate::layout::PaneId,
) {
    let Some(ws) = state.workspaces.get(ws_idx) else {
        return;
    };
    let Some(pane) = ws.pane_state(pane_id) else {
        return;
    };
    let terminal = state.terminals.get(&pane.attached_terminal_id);
    state.creating_new_tab = false;
    state.requested_new_tab_name = None;
    state.rename_pane_target = Some(pane_id);
    state.rename_tab_target = None;
    let agent_name = terminal.and_then(|t| t.agent_name.clone());
    let replace_on_type = agent_name.is_none();
    state.set_name_input(agent_name.unwrap_or_default());
    state.name_input_replace_on_type = replace_on_type;
    state.mode = Mode::RenameAgent;
}

pub(super) fn open_rename_line_split(state: &mut AppState, id: crate::app::state::LineSplitId) {
    let Some(name) = state
        .agent_manual_order
        .order
        .iter()
        .find_map(|entry| match entry {
            crate::app::state::ManualEntry::LineSplit { id: entry_id, name } if *entry_id == id => {
                Some(name.clone())
            }
            _ => None,
        })
    else {
        return;
    };
    state.creating_new_tab = false;
    state.requested_new_tab_name = None;
    state.rename_pane_target = None;
    state.rename_tab_target = None;
    state.rename_line_split_target = Some(id);
    state.set_name_input(name);
    state.name_input_replace_on_type = true;
    state.mode = Mode::RenameLineSplit;
}

/// Write `name` into the line-split identified by `rename_line_split_target`.
/// Client-only presentation state, so this mutates the manual order directly and
/// marks the session dirty rather than dispatching a server API request.
fn commit_line_split_rename(state: &mut AppState, name: String) {
    let Some(id) = state.rename_line_split_target else {
        return;
    };
    for entry in &mut state.agent_manual_order.order {
        if let crate::app::state::ManualEntry::LineSplit {
            id: entry_id,
            name: entry_name,
        } = entry
        {
            if *entry_id == id {
                *entry_name = name;
                state.mark_session_dirty();
                return;
            }
        }
    }
}

fn next_new_tab_default_name(state: &AppState) -> String {
    state
        .active
        .and_then(|i| state.workspaces.get(i))
        .map(|ws| (ws.tabs.len() + 1).to_string())
        .unwrap_or_else(|| "1".to_string())
}

pub(super) fn open_new_tab_dialog(state: &mut AppState) {
    state.creating_new_tab = true;
    state.requested_new_tab_name = None;
    state.rename_pane_target = None;
    state.rename_tab_target = None;
    let name = next_new_tab_default_name(state);
    state.set_name_input(name);
    state.name_input_replace_on_type = true;
    state.mode = Mode::RenameTab;
}

pub(super) fn leave_modal(state: &mut AppState) {
    if state.active.is_some() {
        state.mode = Mode::Terminal;
    } else {
        state.mode = Mode::Navigate;
    }
}

pub(super) const ONBOARDING_WELCOME_ACTIONS: &[ModalActionSpec<ModalAction>] = &[ModalActionSpec {
    action: ModalAction::Continue,
    bindings: &[ModalKeyBinding::Enter],
}];

pub(super) const RELEASE_NOTES_ACTIONS: &[ModalActionSpec<ModalAction>] = &[ModalActionSpec {
    action: ModalAction::Close,
    bindings: &[ModalKeyBinding::Enter, ModalKeyBinding::Esc],
}];

pub(super) const RENAME_ACTIONS: &[ModalActionSpec<ModalAction>] = &[
    ModalActionSpec {
        action: ModalAction::Save,
        bindings: &[ModalKeyBinding::Enter],
    },
    ModalActionSpec {
        action: ModalAction::Clear,
        bindings: &[ModalKeyBinding::CtrlC],
    },
    ModalActionSpec {
        action: ModalAction::Cancel,
        bindings: &[ModalKeyBinding::Esc],
    },
];

pub(super) const CONFIRM_CLOSE_ACTIONS: &[ModalActionSpec<ModalAction>] = &[
    ModalActionSpec {
        action: ModalAction::Confirm,
        bindings: &[ModalKeyBinding::Enter],
    },
    ModalActionSpec {
        action: ModalAction::Cancel,
        bindings: &[ModalKeyBinding::Esc],
    },
];

pub(super) const SETTINGS_ACTIONS: &[ModalActionSpec<ModalAction>] = &[
    ModalActionSpec {
        action: ModalAction::Apply,
        bindings: &[ModalKeyBinding::Enter],
    },
    ModalActionSpec {
        action: ModalAction::Close,
        bindings: &[ModalKeyBinding::Esc],
    },
];

#[cfg(test)]
pub(super) fn apply_rename_action(state: &mut AppState, action: ModalAction) {
    match action {
        ModalAction::Save => {
            let new_name = if state.name_input.trim().is_empty() {
                state.name_input.clone()
            } else {
                state.name_input.trim().to_string()
            };
            match state.mode {
                Mode::RenameWorkspace if !state.workspaces.is_empty() && !new_name.is_empty() => {
                    let workspace_id = state.workspaces[state.selected].id.clone();
                    state.workspaces[state.selected].set_custom_name(new_name);
                    crate::logging::workspace_renamed(&workspace_id);
                    state.mark_session_dirty();
                }
                Mode::RenameTab if state.creating_new_tab => {
                    state.request_new_tab = true;
                    let default_name = next_new_tab_default_name(state);
                    state.requested_new_tab_name =
                        if new_name.is_empty() || new_name == default_name {
                            None
                        } else {
                            Some(new_name)
                        };
                }
                Mode::RenameTab => {
                    let target = state.rename_tab_target.or_else(|| {
                        state
                            .active
                            .map(|ws_idx| (ws_idx, state.workspaces[ws_idx].active_tab))
                    });
                    if let Some((ws_idx, tab_idx)) = target {
                        if let Some(ws) = state.workspaces.get_mut(ws_idx) {
                            let workspace_id = ws.id.clone();
                            let keep_auto_name =
                                ws.tabs.get(tab_idx).is_some_and(|tab| tab.is_auto_named())
                                    && ws
                                        .tab_display_name(tab_idx)
                                        .is_some_and(|name| new_name == name);
                            let tab_id = ws
                                .public_tab_number(tab_idx)
                                .map(|number| {
                                    crate::workspace::public_tab_id_for_number(
                                        &workspace_id,
                                        number,
                                    )
                                })
                                .unwrap_or_else(|| workspace_id.clone());
                            if let Some(tab) = ws.tabs.get_mut(tab_idx) {
                                if !new_name.is_empty() && !keep_auto_name {
                                    tab.set_custom_name(new_name);
                                    crate::logging::tab_renamed(&workspace_id, &tab_id);
                                    state.mark_session_dirty();
                                }
                            }
                        }
                    }
                }
                Mode::RenamePane => {
                    // Resolve the pane by its globally unique id across all
                    // workspaces, so a rename opened from the sidebar Panes
                    // section renames the right pane even when it lives in a
                    // non-active workspace.
                    if let Some(pane_id) = state.rename_pane_target {
                        let terminal_id = state
                            .workspaces
                            .iter()
                            .find_map(|ws| ws.pane_state(pane_id))
                            .map(|pane| pane.attached_terminal_id.clone());
                        if let Some(terminal_id) = terminal_id {
                            if let Some(terminal) = state.terminals.get_mut(&terminal_id) {
                                terminal.set_manual_label(new_name);
                                state.mark_session_dirty();
                            }
                        }
                    }
                }
                Mode::RenameAgent => {
                    if let (Some(ws_idx), Some(pane_id)) = (state.active, state.rename_pane_target)
                    {
                        if let Some(ws) = state.workspaces.get(ws_idx) {
                            if let Some(pane) = ws.pane_state(pane_id) {
                                let terminal_id = pane.attached_terminal_id.clone();
                                if let Some(terminal) = state.terminals.get_mut(&terminal_id) {
                                    if new_name.is_empty() {
                                        terminal.clear_agent_name();
                                    } else {
                                        terminal.set_agent_name(new_name.clone());
                                        terminal.set_manual_label(new_name);
                                    }
                                    state.mark_session_dirty();
                                }
                            }
                        }
                    }
                }
                Mode::RenameLineSplit => {
                    // Line-splits are client-only presentation state; an empty
                    // name is allowed (renders as a plain rule).
                    commit_line_split_rename(state, new_name);
                }
                _ => {}
            }
            state.creating_new_tab = false;
            state.rename_pane_target = None;
            state.rename_line_split_target = None;
            state.rename_tab_target = None;
            clear_rename_input(state);
            leave_modal(state);
        }
        ModalAction::Clear => {
            clear_rename_input(state);
        }
        ModalAction::Cancel => {
            state.creating_new_tab = false;
            state.requested_new_tab_name = None;
            state.rename_pane_target = None;
            state.rename_line_split_target = None;
            state.rename_tab_target = None;
            clear_rename_input(state);
            leave_modal(state);
        }
        _ => {}
    }
}

fn clear_rename_input(state: &mut AppState) {
    state.name_input.clear();
    state.name_input_replace_on_type = false;
    state.name_input_cursor = 0;
}

/// Byte offset of the given char index within `name_input`. Returns the string
/// length when the index is at (or past) the end, so all edits stay on char
/// boundaries and never split multi-byte characters.
fn name_input_byte_offset(state: &AppState, char_idx: usize) -> usize {
    state
        .name_input
        .char_indices()
        .nth(char_idx)
        .map(|(byte, _)| byte)
        .unwrap_or(state.name_input.len())
}

pub(crate) fn insert_rename_input_text(state: &mut AppState, text: &str) {
    if state.name_input_replace_on_type {
        clear_rename_input(state);
    }
    let byte = name_input_byte_offset(state, state.name_input_cursor);
    state.name_input.insert_str(byte, text);
    state.name_input_cursor += text.chars().count();
}

/// Backspace: delete the character before the caret.
fn delete_rename_input_char(state: &mut AppState) {
    if state.name_input_replace_on_type {
        clear_rename_input(state);
        return;
    }
    if state.name_input_cursor == 0 {
        return;
    }
    let start = name_input_byte_offset(state, state.name_input_cursor - 1);
    let end = name_input_byte_offset(state, state.name_input_cursor);
    state.name_input.replace_range(start..end, "");
    state.name_input_cursor -= 1;
}

/// Forward delete: delete the character after the caret.
fn delete_rename_input_forward_char(state: &mut AppState) {
    if state.name_input_replace_on_type {
        clear_rename_input(state);
        return;
    }
    if state.name_input_cursor >= state.name_input_len_chars() {
        return;
    }
    let start = name_input_byte_offset(state, state.name_input_cursor);
    let end = name_input_byte_offset(state, state.name_input_cursor + 1);
    state.name_input.replace_range(start..end, "");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenameWordDeleteClass {
    Word,
    Separator,
}

fn rename_word_delete_class(ch: char) -> RenameWordDeleteClass {
    if ch.is_alphanumeric() || ch == '_' {
        RenameWordDeleteClass::Word
    } else {
        RenameWordDeleteClass::Separator
    }
}

/// Char index reached by moving one word to the left of `cursor`: skip any
/// whitespace, then skip the run of the same character class.
fn rename_word_left_index(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut idx = cursor.min(chars.len());
    while idx > 0 && chars[idx - 1].is_whitespace() {
        idx -= 1;
    }
    if idx == 0 {
        return 0;
    }
    let class = rename_word_delete_class(chars[idx - 1]);
    while idx > 0
        && !chars[idx - 1].is_whitespace()
        && rename_word_delete_class(chars[idx - 1]) == class
    {
        idx -= 1;
    }
    idx
}

/// Char index reached by moving one word to the right of `cursor`: skip the run
/// at the caret, then any whitespace, landing at the start of the next word.
fn rename_word_right_index(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut idx = cursor.min(len);
    if idx < len && !chars[idx].is_whitespace() {
        let class = rename_word_delete_class(chars[idx]);
        while idx < len
            && !chars[idx].is_whitespace()
            && rename_word_delete_class(chars[idx]) == class
        {
            idx += 1;
        }
    }
    while idx < len && chars[idx].is_whitespace() {
        idx += 1;
    }
    idx
}

fn delete_rename_input_word(state: &mut AppState) {
    if state.name_input_replace_on_type {
        clear_rename_input(state);
        return;
    }
    let target = rename_word_left_index(&state.name_input, state.name_input_cursor);
    let start = name_input_byte_offset(state, target);
    let end = name_input_byte_offset(state, state.name_input_cursor);
    state.name_input.replace_range(start..end, "");
    state.name_input_cursor = target;
}

/// Disarm replace-on-type without wiping the prefilled text. Called by caret
/// movement so the first arrow/Home/End keeps the text and just positions the
/// caret.
fn disarm_replace_on_type(state: &mut AppState) {
    state.name_input_replace_on_type = false;
}

fn move_rename_cursor_left(state: &mut AppState) {
    disarm_replace_on_type(state);
    state.name_input_cursor = state.name_input_cursor.saturating_sub(1);
}

fn move_rename_cursor_right(state: &mut AppState) {
    disarm_replace_on_type(state);
    if state.name_input_cursor < state.name_input_len_chars() {
        state.name_input_cursor += 1;
    }
}

fn move_rename_cursor_home(state: &mut AppState) {
    disarm_replace_on_type(state);
    state.name_input_cursor = 0;
}

fn move_rename_cursor_end(state: &mut AppState) {
    disarm_replace_on_type(state);
    state.name_input_cursor = state.name_input_len_chars();
}

fn move_rename_cursor_word_left(state: &mut AppState) {
    disarm_replace_on_type(state);
    state.name_input_cursor = rename_word_left_index(&state.name_input, state.name_input_cursor);
}

fn move_rename_cursor_word_right(state: &mut AppState) {
    disarm_replace_on_type(state);
    state.name_input_cursor = rename_word_right_index(&state.name_input, state.name_input_cursor);
}

fn handle_rename_edit_key(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Char('u') if ctrl => {
            clear_rename_input(state);
        }
        KeyCode::Backspace if key.modifiers.contains(KeyModifiers::SUPER) => {
            clear_rename_input(state);
        }
        KeyCode::Backspace if ctrl || alt => {
            delete_rename_input_word(state);
        }
        KeyCode::Char('h' | 'w') if ctrl => {
            delete_rename_input_word(state);
        }
        KeyCode::Backspace => delete_rename_input_char(state),
        KeyCode::Delete => delete_rename_input_forward_char(state),
        KeyCode::Left if ctrl || alt => move_rename_cursor_word_left(state),
        KeyCode::Right if ctrl || alt => move_rename_cursor_word_right(state),
        KeyCode::Left => move_rename_cursor_left(state),
        KeyCode::Right => move_rename_cursor_right(state),
        KeyCode::Home => move_rename_cursor_home(state),
        KeyCode::End => move_rename_cursor_end(state),
        KeyCode::Char('a') if ctrl => move_rename_cursor_home(state),
        KeyCode::Char('e') if ctrl => move_rename_cursor_end(state),
        KeyCode::Char(c) if key.modifiers.difference(KeyModifiers::SHIFT).is_empty() => {
            insert_rename_input_text(state, &c.to_string());
        }
        _ => {}
    }
}

#[cfg(test)]
pub(crate) fn handle_rename_key(state: &mut AppState, key: KeyEvent) {
    if let Some(action) = modal_action_from_key(&key, RENAME_ACTIONS) {
        apply_rename_action(state, action);
        return;
    }

    handle_rename_edit_key(state, key);
}

#[cfg(test)]
pub(crate) fn handle_resize_key(state: &mut AppState, raw_key: TerminalKey) {
    let key = raw_key.as_key_event();
    if key.code == KeyCode::Esc
        || key.code == KeyCode::Enter
        || state.keybinds.resize_mode.matches_prefix_key(raw_key)
        || state.keybinds.resize_mode.matches_direct_key(raw_key)
    {
        if state.active.is_some() {
            state.mode = Mode::Terminal;
        } else {
            state.mode = Mode::Navigate;
        }
        return;
    }

    match key.code {
        KeyCode::Char('h') | KeyCode::Left => state.resize_pane(NavDirection::Left),
        KeyCode::Char('l') | KeyCode::Right => state.resize_pane(NavDirection::Right),
        KeyCode::Char('j') | KeyCode::Down => state.resize_pane(NavDirection::Down),
        KeyCode::Char('k') | KeyCode::Up => state.resize_pane(NavDirection::Up),
        _ => {}
    }
}

pub(super) fn open_confirm_close(state: &mut AppState) {
    state.mode = Mode::ConfirmClose;
}

#[cfg(test)]
pub(super) fn confirm_close_accept(state: &mut AppState) {
    state.close_selected_workspace();
    if state.workspaces.is_empty() {
        state.mode = Mode::Navigate;
    } else {
        state.mode = Mode::Terminal;
    }
}

pub(super) fn confirm_close_cancel(state: &mut AppState) {
    state.mode = Mode::Navigate;
}

#[cfg(test)]
pub(crate) fn handle_confirm_close_key(state: &mut AppState, key: KeyEvent) {
    match modal_action_from_key(&key, CONFIRM_CLOSE_ACTIONS) {
        Some(ModalAction::Confirm) => confirm_close_accept(state),
        Some(ModalAction::Cancel) => confirm_close_cancel(state),
        _ => {}
    }
}

#[cfg(test)]
pub(super) fn apply_context_menu_action(
    state: &mut AppState,
    terminal_runtimes: &mut crate::terminal::TerminalRuntimeRegistry,
    menu: ContextMenuState,
    idx: usize,
) {
    let item = menu.items().get(idx).copied();
    match (menu.kind, item) {
        (ContextMenuKind::GitWorkspace { ws_idx, .. }, Some("New worktree")) => {
            state.request_new_linked_worktree = Some(ws_idx);
            leave_modal(state);
        }
        (ContextMenuKind::GitWorkspace { ws_idx, .. }, Some("Delete worktree checkout...")) => {
            state.request_remove_linked_worktree = Some(ws_idx);
            leave_modal(state);
        }
        (ContextMenuKind::GitWorkspace { ws_idx, .. }, Some("Open worktree...")) => {
            state.request_open_existing_worktree = Some(ws_idx);
            leave_modal(state);
        }
        (
            ContextMenuKind::GitWorkspace {
                ws_idx, collapsed, ..
            },
            Some("Collapse" | "Expand"),
        ) => {
            if let Some(key) = state
                .workspaces
                .get(ws_idx)
                .and_then(|ws| ws.worktree_space())
                .map(|space| space.key.clone())
            {
                if collapsed {
                    state.collapsed_space_keys.remove(&key);
                } else {
                    state.collapsed_space_keys.insert(key);
                }
                state.mark_session_dirty();
            }
            leave_modal(state);
        }
        (
            ContextMenuKind::Workspace { ws_idx } | ContextMenuKind::GitWorkspace { ws_idx, .. },
            Some("Rename"),
        ) => {
            open_rename_workspace(state, terminal_runtimes, ws_idx);
        }
        (
            ContextMenuKind::Workspace { ws_idx } | ContextMenuKind::GitWorkspace { ws_idx, .. },
            Some("Close" | "Close group"),
        ) => {
            state.selected = ws_idx;
            if state.confirm_close {
                open_confirm_close(state);
            } else {
                state.close_selected_workspace();
                state.mode = Mode::Navigate;
            }
        }
        (ContextMenuKind::Tab { ws_idx, tab_idx }, Some("New tab")) => {
            state.selected = ws_idx;
            state.active = Some(ws_idx);
            state.switch_tab(tab_idx);
            open_new_tab_dialog(state);
        }
        (ContextMenuKind::Tab { ws_idx, tab_idx }, Some("Rename")) => {
            state.selected = ws_idx;
            state.active = Some(ws_idx);
            state.switch_tab(tab_idx);
            open_rename_active_tab(state, false);
        }
        (ContextMenuKind::Tab { ws_idx, tab_idx }, Some("Close")) => {
            state.selected = ws_idx;
            state.active = Some(ws_idx);
            state.switch_tab(tab_idx);
            if !state.close_tab() {
                state.mode = if state.active.is_some() {
                    Mode::Terminal
                } else {
                    Mode::Navigate
                };
            }
        }
        (ContextMenuKind::Pane { pane_id, .. }, Some("Rename pane")) => {
            open_rename_pane(state, pane_id);
        }
        (
            ContextMenuKind::Pane {
                ws_idx, pane_id, ..
            },
            Some("Clear pane name"),
        ) => {
            if let Some(ws) = state.workspaces.get(ws_idx) {
                if let Some(pane) = ws.pane_state(pane_id) {
                    let terminal_id = pane.attached_terminal_id.clone();
                    if let Some(terminal) = state.terminals.get_mut(&terminal_id) {
                        terminal.clear_manual_label();
                        state.mark_session_dirty();
                    }
                }
            }
            state.mode = Mode::Terminal;
        }
        (
            ContextMenuKind::Pane {
                ws_idx,
                tab_idx,
                pane_id,
                source_pane_id,
                ..
            },
            Some("Swap with focused pane"),
        ) => {
            if let Some(source_pane_id) = source_pane_id {
                state.selected = ws_idx;
                state.active = Some(ws_idx);
                state.switch_tab(tab_idx);
                if let Some(tab) = state
                    .workspaces
                    .get_mut(ws_idx)
                    .and_then(|ws| ws.tabs.get_mut(tab_idx))
                {
                    if tab.layout.swap_panes(source_pane_id, pane_id) {
                        tab.layout.focus_pane(source_pane_id);
                        state.mark_session_dirty();
                    }
                }
            }
            state.mode = Mode::Terminal;
        }
        (
            ContextMenuKind::Pane {
                ws_idx,
                tab_idx,
                pane_id,
                ..
            },
            Some("Split right"),
        ) => {
            state.selected = ws_idx;
            state.active = Some(ws_idx);
            state.switch_tab(tab_idx);
            state.focus_pane_in_workspace(ws_idx, pane_id);
            state.split_pane(terminal_runtimes, Direction::Horizontal);
            state.mode = Mode::Terminal;
        }
        (
            ContextMenuKind::Pane {
                ws_idx,
                tab_idx,
                pane_id,
                ..
            },
            Some("Split down"),
        ) => {
            state.selected = ws_idx;
            state.active = Some(ws_idx);
            state.switch_tab(tab_idx);
            state.focus_pane_in_workspace(ws_idx, pane_id);
            state.split_pane(terminal_runtimes, Direction::Vertical);
            state.mode = Mode::Terminal;
        }
        (
            ContextMenuKind::Pane {
                ws_idx,
                tab_idx,
                pane_id,
                ..
            },
            Some("Zoom"),
        ) => {
            state.selected = ws_idx;
            state.active = Some(ws_idx);
            state.switch_tab(tab_idx);
            state.focus_pane_in_workspace(ws_idx, pane_id);
            state.toggle_zoom();
            state.mode = Mode::Terminal;
        }
        (
            ContextMenuKind::Pane {
                ws_idx,
                tab_idx,
                pane_id,
                ..
            },
            Some("Close pane"),
        ) => {
            state.selected = ws_idx;
            state.active = Some(ws_idx);
            state.switch_tab(tab_idx);
            state.focus_pane_in_workspace(ws_idx, pane_id);
            if !state.close_pane() {
                state.mode = if state.active.is_some() {
                    Mode::Terminal
                } else {
                    Mode::Navigate
                };
            }
        }
        (ContextMenuKind::LineSplit { id }, Some("Rename")) => {
            open_rename_line_split(state, id);
        }
        (ContextMenuKind::LineSplit { id }, Some("Delete")) => {
            delete_line_split(state, id);
            leave_modal(state);
        }
        _ => leave_modal(state),
    }
}

/// Remove the line-split with `id` from the manual order (client-only).
fn delete_line_split(state: &mut AppState, id: crate::app::state::LineSplitId) {
    let before = state.agent_manual_order.order.len();
    state.agent_manual_order.order.retain(|entry| {
        !matches!(entry, crate::app::state::ManualEntry::LineSplit { id: entry_id, .. } if *entry_id == id)
    });
    if state.agent_manual_order.order.len() != before {
        state.mark_session_dirty();
    }
}

#[cfg(test)]
pub(crate) fn handle_context_menu_key(
    state: &mut AppState,
    terminal_runtimes: &mut crate::terminal::TerminalRuntimeRegistry,
    key: KeyEvent,
) {
    match key.code {
        KeyCode::Esc => {
            state.context_menu = None;
            leave_modal(state);
        }
        KeyCode::Up => {
            if let Some(menu) = &mut state.context_menu {
                menu.list.move_prev();
            }
        }
        KeyCode::Down => {
            if let Some(menu) = &mut state.context_menu {
                menu.list.move_next(menu.items().len());
            }
        }
        KeyCode::Enter => {
            if let Some(menu) = state.context_menu.take() {
                let idx = menu.list.highlighted;
                apply_context_menu_action(state, terminal_runtimes, menu, idx);
            }
        }
        _ => {}
    }
}

impl App {
    pub(crate) fn handle_rename_key_via_api(&mut self, key: KeyEvent) {
        if let Some(action) = modal_action_from_key(&key, RENAME_ACTIONS) {
            self.apply_rename_mouse_action_via_api(action);
            return;
        }

        handle_rename_edit_key(&mut self.state, key);
    }

    fn save_rename_modal_via_api(&mut self) {
        let new_name = if self.state.name_input.trim().is_empty() {
            self.state.name_input.clone()
        } else {
            self.state.name_input.trim().to_string()
        };

        match self.state.mode {
            Mode::RenameWorkspace if !self.state.workspaces.is_empty() && !new_name.is_empty() => {
                let workspace_id = self.public_workspace_id(self.state.selected);
                self.dispatch_tui_api_request(
                    "tui.workspace.rename",
                    crate::api::schema::Method::WorkspaceRename(
                        crate::api::schema::WorkspaceRenameParams {
                            workspace_id,
                            label: new_name,
                        },
                    ),
                );
            }
            Mode::RenameTab if self.state.creating_new_tab => {
                let default_name = next_new_tab_default_name(&self.state);
                let label = if new_name.is_empty() || new_name == default_name {
                    None
                } else {
                    Some(new_name)
                };
                self.dispatch_tui_api_request(
                    "tui.tab.create_named",
                    crate::api::schema::Method::TabCreate(crate::api::schema::TabCreateParams {
                        workspace_id: None,
                        cwd: None,
                        focus: true,
                        label,
                        env: Default::default(),
                    }),
                );
            }
            Mode::RenameTab if !new_name.is_empty() => {
                // Prefer an explicit tab target (e.g. an agent-row double-click);
                // otherwise rename the active workspace's active tab.
                let target = self.state.rename_tab_target.or_else(|| {
                    self.state
                        .active
                        .map(|ws_idx| (ws_idx, self.state.workspaces[ws_idx].active_tab))
                });
                let Some((ws_idx, tab_idx)) = target.filter(|&(ws_idx, tab_idx)| {
                    self.state
                        .workspaces
                        .get(ws_idx)
                        .is_some_and(|ws| tab_idx < ws.tabs.len())
                }) else {
                    cancel_rename_modal(&mut self.state);
                    return;
                };
                let keep_auto_name = self.state.workspaces[ws_idx]
                    .tabs
                    .get(tab_idx)
                    .is_some_and(|tab| tab.is_auto_named())
                    && self.state.workspaces[ws_idx]
                        .tab_display_name(tab_idx)
                        .is_some_and(|name| new_name == name);
                if !keep_auto_name {
                    if let Some(tab_id) = self.public_tab_id(ws_idx, tab_idx) {
                        self.dispatch_tui_api_request(
                            "tui.tab.rename",
                            crate::api::schema::Method::TabRename(
                                crate::api::schema::TabRenameParams {
                                    tab_id,
                                    label: new_name,
                                },
                            ),
                        );
                    }
                }
            }
            Mode::RenamePane => {
                if let (Some(ws_idx), Some(pane_id)) =
                    (self.state.active, self.state.rename_pane_target)
                {
                    if let Some(pane_id) = self.public_pane_id(ws_idx, pane_id) {
                        self.dispatch_tui_api_request(
                            "tui.pane.rename",
                            crate::api::schema::Method::PaneRename(
                                crate::api::schema::PaneRenameParams {
                                    pane_id,
                                    label: Some(new_name),
                                },
                            ),
                        );
                    }
                }
            }
            Mode::RenameAgent => {
                if let (Some(ws_idx), Some(pane_id)) =
                    (self.state.active, self.state.rename_pane_target)
                {
                    if let Some(target) = self.public_pane_id(ws_idx, pane_id) {
                        let name = if new_name.is_empty() {
                            None
                        } else {
                            Some(new_name)
                        };
                        self.dispatch_tui_api_request(
                            "tui.agent.rename",
                            crate::api::schema::Method::AgentRename(
                                crate::api::schema::AgentRenameParams { target, name },
                            ),
                        );
                    }
                }
            }
            Mode::RenameLineSplit => {
                // Client-only: write the name directly, no server dispatch.
                commit_line_split_rename(&mut self.state, new_name);
            }
            _ => {}
        }

        cancel_rename_modal(&mut self.state);
    }

    pub(super) fn apply_rename_mouse_action_via_api(&mut self, action: ModalAction) {
        match action {
            ModalAction::Save => self.save_rename_modal_via_api(),
            ModalAction::Clear => {
                clear_rename_input(&mut self.state);
            }
            ModalAction::Cancel => cancel_rename_modal(&mut self.state),
            _ => {}
        }
    }

    pub(super) fn confirm_close_accept_via_api(&mut self) {
        let ws_idx = self.state.selected;
        if ws_idx < self.state.workspaces.len() {
            self.close_workspace_idx_via_api(ws_idx);
        }
        self.state.mode = if self.state.active.is_some() {
            Mode::Terminal
        } else {
            Mode::Navigate
        };
    }

    pub(crate) fn handle_resize_key_via_api(&mut self, raw_key: TerminalKey) {
        let key = raw_key.as_key_event();
        if key.code == KeyCode::Esc
            || key.code == KeyCode::Enter
            || self.state.keybinds.resize_mode.matches_prefix_key(raw_key)
            || self.state.keybinds.resize_mode.matches_direct_key(raw_key)
        {
            self.state.mode = if self.state.active.is_some() {
                Mode::Terminal
            } else {
                Mode::Navigate
            };
            return;
        }

        let direction = match key.code {
            KeyCode::Char('h') | KeyCode::Left => Some(NavDirection::Left),
            KeyCode::Char('l') | KeyCode::Right => Some(NavDirection::Right),
            KeyCode::Char('j') | KeyCode::Down => Some(NavDirection::Down),
            KeyCode::Char('k') | KeyCode::Up => Some(NavDirection::Up),
            _ => None,
        };
        if let Some(direction) = direction {
            self.dispatch_tui_api_request(
                "tui.pane.resize",
                crate::api::schema::Method::PaneResize(crate::api::schema::PaneResizeParams {
                    pane_id: None,
                    direction: super::navigate::api_pane_direction(direction),
                    amount: None,
                }),
            );
        }
    }

    pub(crate) fn handle_confirm_close_key_via_api(&mut self, key: KeyEvent) {
        match modal_action_from_key(&key, CONFIRM_CLOSE_ACTIONS) {
            Some(ModalAction::Confirm) => {
                self.confirm_close_accept_via_api();
            }
            Some(ModalAction::Cancel) => confirm_close_cancel(&mut self.state),
            _ => {}
        }
    }

    pub(crate) fn handle_context_menu_key_via_api(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.state.context_menu = None;
                leave_modal(&mut self.state);
            }
            KeyCode::Up => {
                if let Some(menu) = &mut self.state.context_menu {
                    menu.list.move_prev();
                }
            }
            KeyCode::Down => {
                if let Some(menu) = &mut self.state.context_menu {
                    menu.list.move_next(menu.items().len());
                }
            }
            KeyCode::Enter => {
                if let Some(menu) = self.state.context_menu.take() {
                    let idx = menu.list.highlighted;
                    self.apply_context_menu_action_via_api(menu, idx);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn apply_context_menu_action_via_api(&mut self, menu: ContextMenuState, idx: usize) {
        let item = menu.items().get(idx).copied();
        match (menu.kind, item) {
            (ContextMenuKind::GitWorkspace { ws_idx, .. }, Some("New worktree")) => {
                self.state.request_new_linked_worktree = Some(ws_idx);
                leave_modal(&mut self.state);
            }
            (ContextMenuKind::GitWorkspace { ws_idx, .. }, Some("Delete worktree checkout...")) => {
                self.state.request_remove_linked_worktree = Some(ws_idx);
                leave_modal(&mut self.state);
            }
            (ContextMenuKind::GitWorkspace { ws_idx, .. }, Some("Open worktree...")) => {
                self.state.request_open_existing_worktree = Some(ws_idx);
                leave_modal(&mut self.state);
            }
            (
                ContextMenuKind::GitWorkspace {
                    ws_idx, collapsed, ..
                },
                Some("Collapse" | "Expand"),
            ) => {
                if let Some(key) = self
                    .state
                    .workspaces
                    .get(ws_idx)
                    .and_then(|ws| ws.worktree_space())
                    .map(|space| space.key.clone())
                {
                    if collapsed {
                        self.state.collapsed_space_keys.remove(&key);
                    } else {
                        self.state.collapsed_space_keys.insert(key);
                    }
                    self.state.mark_session_dirty();
                }
                leave_modal(&mut self.state);
            }
            (
                ContextMenuKind::Workspace { ws_idx }
                | ContextMenuKind::GitWorkspace { ws_idx, .. },
                Some("Rename"),
            ) => open_rename_workspace(&mut self.state, &self.terminal_runtimes, ws_idx),
            (
                ContextMenuKind::Workspace { ws_idx }
                | ContextMenuKind::GitWorkspace { ws_idx, .. },
                Some("Close" | "Close group"),
            ) => {
                self.state.selected = ws_idx;
                if self.state.confirm_close {
                    open_confirm_close(&mut self.state);
                } else {
                    self.close_workspace_idx_via_api(ws_idx);
                    self.state.mode = Mode::Navigate;
                }
            }
            (ContextMenuKind::Tab { ws_idx, tab_idx }, Some("New tab")) => {
                self.focus_workspace_idx_via_api(ws_idx);
                self.focus_tab_idx_via_api(tab_idx);
                open_new_tab_dialog(&mut self.state);
            }
            (ContextMenuKind::Tab { ws_idx, tab_idx }, Some("Rename")) => {
                self.focus_workspace_idx_via_api(ws_idx);
                self.focus_tab_idx_via_api(tab_idx);
                open_rename_active_tab(&mut self.state, false);
            }
            (ContextMenuKind::Tab { ws_idx, tab_idx }, Some("Close")) => {
                self.focus_workspace_idx_via_api(ws_idx);
                self.focus_tab_idx_via_api(tab_idx);
                self.close_active_tab_via_api();
            }
            (ContextMenuKind::Pane { pane_id, .. }, Some("Rename pane")) => {
                open_rename_pane(&mut self.state, pane_id);
            }
            (
                ContextMenuKind::Pane {
                    ws_idx, pane_id, ..
                },
                Some("Clear pane name"),
            ) => {
                if let Some(pane_id) = self.public_pane_id(ws_idx, pane_id) {
                    self.dispatch_tui_api_request(
                        "tui.pane.clear_name",
                        crate::api::schema::Method::PaneRename(
                            crate::api::schema::PaneRenameParams {
                                pane_id,
                                label: None,
                            },
                        ),
                    );
                }
                self.state.mode = Mode::Terminal;
            }
            (
                ContextMenuKind::Pane {
                    ws_idx,
                    pane_id,
                    source_pane_id: Some(source_pane_id),
                    ..
                },
                Some("Swap with focused pane"),
            ) => {
                let source_public_id = self.public_pane_id(ws_idx, source_pane_id);
                let target_public_id = self.public_pane_id(ws_idx, pane_id);
                if let (Some(source_public_id), Some(target_public_id)) =
                    (source_public_id, target_public_id)
                {
                    self.dispatch_tui_api_request(
                        "tui.pane.swap_exact",
                        crate::api::schema::Method::PaneSwap(crate::api::schema::PaneSwapParams {
                            pane_id: None,
                            direction: None,
                            source_pane_id: Some(source_public_id),
                            target_pane_id: Some(target_public_id),
                        }),
                    );
                    self.focus_pane_internal_via_api(ws_idx, source_pane_id);
                }
                self.state.mode = Mode::Terminal;
            }
            (
                ContextMenuKind::Pane {
                    ws_idx, pane_id, ..
                },
                Some("Split right"),
            ) => {
                self.focus_pane_internal_via_api(ws_idx, pane_id);
                self.split_focused_pane_via_api(crate::api::schema::SplitDirection::Right);
                self.state.mode = Mode::Terminal;
            }
            (
                ContextMenuKind::Pane {
                    ws_idx, pane_id, ..
                },
                Some("Split down"),
            ) => {
                self.focus_pane_internal_via_api(ws_idx, pane_id);
                self.split_focused_pane_via_api(crate::api::schema::SplitDirection::Down);
                self.state.mode = Mode::Terminal;
            }
            (
                ContextMenuKind::Pane {
                    ws_idx, pane_id, ..
                },
                Some("Zoom"),
            ) => {
                self.focus_pane_internal_via_api(ws_idx, pane_id);
                self.zoom_focused_pane_via_api();
                self.state.mode = Mode::Terminal;
            }
            (
                ContextMenuKind::Pane {
                    ws_idx, pane_id, ..
                },
                Some("Close pane"),
            ) => {
                self.focus_pane_internal_via_api(ws_idx, pane_id);
                self.close_focused_pane_via_api();
                self.state.mode = if self.state.active.is_some() {
                    Mode::Terminal
                } else {
                    Mode::Navigate
                };
            }
            (ContextMenuKind::LineSplit { id }, Some("Rename")) => {
                open_rename_line_split(&mut self.state, id);
            }
            (ContextMenuKind::LineSplit { id }, Some("Delete")) => {
                delete_line_split(&mut self.state, id);
                leave_modal(&mut self.state);
            }
            _ => leave_modal(&mut self.state),
        }
    }
}

fn cancel_rename_modal(state: &mut AppState) {
    state.creating_new_tab = false;
    state.requested_new_tab_name = None;
    state.rename_pane_target = None;
    state.rename_line_split_target = None;
    state.rename_tab_target = None;
    clear_rename_input(state);
    leave_modal(state);
}

impl AppState {
    pub(super) fn global_menu_item_at(&self, col: u16, row: u16) -> Option<GlobalMenuAction> {
        let rect = self.global_menu_rect();
        if col <= rect.x
            || col >= rect.x + rect.width.saturating_sub(1)
            || row <= rect.y
            || row >= rect.y + rect.height.saturating_sub(1)
        {
            return None;
        }
        let idx = (row - rect.y - 1) as usize;
        global_menu_actions(self).get(idx).copied()
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::layout::Rect;

    use super::super::{capture_snapshot, state_with_workspaces};
    use super::*;

    fn config_env_lock() -> &'static std::sync::Mutex<()> {
        crate::config::test_config_env_lock()
    }

    fn temp_config_path(name: &str) -> std::path::PathBuf {
        let unique = format!(
            "herdr-modal-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::env::temp_dir().join(unique).join("config.toml")
    }

    #[test]
    fn custom_resize_key_exits_resize_mode() {
        let mut state = state_with_workspaces(&["test"]);
        state.mode = Mode::Resize;
        state.keybinds.resize_mode = crate::config::ActionKeybinds::prefix("g");

        handle_resize_key(
            &mut state,
            TerminalKey::new(KeyCode::Char('g'), KeyModifiers::empty()),
        );

        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn direct_resize_key_exits_resize_mode() {
        let mut state = state_with_workspaces(&["test"]);
        state.mode = Mode::Resize;
        state.keybinds.resize_mode = crate::config::ActionKeybinds::direct("ctrl+alt+r");

        handle_resize_key(
            &mut state,
            TerminalKey::new(
                KeyCode::Char('r'),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            ),
        );

        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn resize_key_exit_matches_enhanced_shifted_punctuation() {
        let mut state = state_with_workspaces(&["test"]);
        state.mode = Mode::Resize;
        state.keybinds.resize_mode = crate::config::ActionKeybinds::prefix("?");

        handle_resize_key(
            &mut state,
            TerminalKey::new(KeyCode::Char('/'), KeyModifiers::SHIFT)
                .with_shifted_codepoint('?' as u32),
        );

        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn detach_requests_client_detach_in_persistence_mode() {
        let mut state = state_with_workspaces(&["test"]);
        state.detach_exits = false;

        request_detach(&mut state);

        assert!(state.detach_requested);
        assert!(!state.should_quit);
    }

    #[test]
    fn detach_exits_in_no_session_mode() {
        let mut state = state_with_workspaces(&["test"]);
        state.detach_exits = true;

        request_detach(&mut state);

        assert!(state.should_quit);
        assert!(!state.detach_requested);
    }

    #[test]
    fn global_menu_whats_new_opens_saved_release_notes() {
        let _guard = config_env_lock().lock().unwrap();
        let path = temp_config_path("whats-new-saved-release-notes");
        std::env::set_var(crate::config::CONFIG_PATH_ENV_VAR, &path);
        crate::release_notes::save_pending(env!("CARGO_PKG_VERSION"), "### Changed\n- Menu")
            .unwrap();

        let mut state = state_with_workspaces(&["test"]);
        state.latest_release_notes_available = true;

        assert!(global_menu_actions(&state).contains(&GlobalMenuAction::WhatsNew));

        apply_global_menu_action(&mut state, GlobalMenuAction::WhatsNew);

        assert_eq!(state.mode, Mode::ReleaseNotes);
        assert_eq!(
            state
                .release_notes
                .as_ref()
                .map(|notes| notes.body.as_str()),
            Some("### Changed\n- Menu")
        );

        std::env::remove_var(crate::config::CONFIG_PATH_ENV_VAR);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn rename_modal_keyboard_and_mouse_share_actions() {
        let mut state = state_with_workspaces(&["test"]);
        state.mode = Mode::RenameWorkspace;
        state.set_name_input("hello");

        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        assert!(state.name_input.is_empty());

        state.set_name_input("renamed");
        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
        );
        assert_eq!(state.mode, Mode::Terminal);
        assert_eq!(state.workspaces[0].display_name(), "renamed");
        let snapshot = capture_snapshot(&state);
        assert_eq!(
            snapshot.workspaces[0].custom_name.as_deref(),
            Some("renamed")
        );

        state.view.sidebar_rect = Rect::new(0, 0, 26, 20);
        state.view.terminal_area = Rect::new(26, 0, 80, 20);
        state.mode = Mode::RenameWorkspace;
        state.set_name_input("mouse");
        let inner = state.rename_modal_inner().unwrap();
        let (save, _, _) = crate::ui::rename_button_rects(inner);
        let action = modal_action_from_buttons(save.x, save.y, &[(save, ModalAction::Save)]);
        assert_eq!(action, Some(ModalAction::Save));
    }

    #[test]
    fn tab_rename_updates_captured_snapshot() {
        let mut state = state_with_workspaces(&["test"]);
        state.mode = Mode::RenameTab;
        state.set_name_input("logs");

        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
        );

        let snapshot = capture_snapshot(&state);
        assert_eq!(
            snapshot.workspaces[0].tabs[0].custom_name.as_deref(),
            Some("logs")
        );
    }

    #[test]
    fn rename_cancel_returns_to_terminal_when_workspace_is_active() {
        let mut state = state_with_workspaces(&["test"]);
        state.mode = Mode::RenameTab;
        state.set_name_input("test");

        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()),
        );

        assert_eq!(state.mode, Mode::Terminal);
        assert!(state.name_input.is_empty());
    }

    #[test]
    fn rename_modal_replaces_prefilled_text_on_first_type() {
        let mut state = state_with_workspaces(&["test"]);
        state.mode = Mode::RenameTab;
        state.set_name_input("2");
        state.name_input_replace_on_type = true;

        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty()),
        );
        assert_eq!(state.name_input, "n");
        assert!(!state.name_input_replace_on_type);

        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::empty()),
        );
        assert_eq!(state.name_input, "ne");
    }

    #[test]
    fn rename_modal_replaces_prefilled_text_on_paste() {
        let mut state = state_with_workspaces(&["test"]);
        state.mode = Mode::RenameTab;
        state.set_name_input("2");
        state.name_input_replace_on_type = true;

        insert_rename_input_text(&mut state, "feature/logs");

        assert_eq!(state.name_input, "feature/logs");
        assert!(!state.name_input_replace_on_type);

        insert_rename_input_text(&mut state, "-copy");

        assert_eq!(state.name_input, "feature/logs-copy");
    }

    #[test]
    fn rename_modal_handles_line_editing_shortcuts() {
        let mut state = state_with_workspaces(&["test"]);
        state.mode = Mode::RenameWorkspace;
        state.set_name_input("website zero");

        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()),
        );
        assert_eq!(state.name_input, "website zer");

        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL),
        );
        assert_eq!(state.name_input, "website ");

        state.set_name_input("website-zero");
        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT),
        );
        assert_eq!(state.name_input, "website-");

        state.set_name_input("website-zero");
        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL),
        );
        assert_eq!(state.name_input, "website-");

        state.set_name_input("website-zero");
        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
        );
        assert_eq!(state.name_input, "website-");

        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::SUPER),
        );
        assert!(state.name_input.is_empty());

        state.set_name_input("website zero");
        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        );
        assert!(state.name_input.is_empty());
    }

    #[test]
    fn rename_modal_does_not_insert_modified_shortcut_chars() {
        let mut state = state_with_workspaces(&["test"]);
        state.mode = Mode::RenameWorkspace;
        state.set_name_input("website");

        // Ctrl+A is a caret-home shortcut, not an inserted 'a'.
        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
        );
        assert_eq!(state.name_input, "website");
        assert_eq!(state.name_input_cursor, 0);

        // Shift+Z inserts a normal capital letter at the caret (now at home).
        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::SHIFT),
        );
        assert_eq!(state.name_input, "Zwebsite");
        assert_eq!(state.name_input_cursor, 1);
    }

    #[test]
    fn navigator_search_accepts_pasted_text_when_focused() {
        let mut state = state_with_workspaces(&["alpha", "beta"]);
        let terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();
        state.mode = Mode::Navigator;
        state.navigator.search_focused = true;
        state.navigator.state_filter = Some(NavigatorStateFilter::Working);

        insert_navigator_search_text(&mut state, &terminal_runtimes, "beta");

        assert_eq!(state.navigator.query, "beta");
        assert_eq!(state.navigator.state_filter, None);
    }

    #[test]
    fn navigator_search_ignores_paste_when_search_is_not_focused() {
        let mut state = state_with_workspaces(&["alpha", "beta"]);
        let terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();
        state.mode = Mode::Navigator;
        state.navigator.search_focused = false;

        insert_navigator_search_text(&mut state, &terminal_runtimes, "beta");

        assert!(state.navigator.query.is_empty());
    }

    #[test]
    fn open_rename_active_tab_can_prefill_default_new_tab_name() {
        let mut state = state_with_workspaces(&["test"]);
        state.workspaces[0].test_add_tab(None);
        state.workspaces[0].switch_tab(1);

        open_rename_active_tab(&mut state, true);

        assert_eq!(state.mode, Mode::RenameTab);
        assert_eq!(state.name_input, "2");
        assert!(state.name_input_replace_on_type);
    }

    #[test]
    fn cancel_new_tab_dialog_leaves_workspace_unchanged() {
        let mut state = state_with_workspaces(&["test"]);
        open_new_tab_dialog(&mut state);

        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()),
        );

        assert_eq!(state.mode, Mode::Terminal);
        assert!(!state.creating_new_tab);
        assert!(!state.request_new_tab);
        assert!(state.requested_new_tab_name.is_none());
        assert_eq!(state.workspaces[0].tabs.len(), 1);
    }

    #[test]
    fn saving_new_tab_dialog_requests_creation_with_name() {
        let mut state = state_with_workspaces(&["test"]);
        open_new_tab_dialog(&mut state);
        state.set_name_input("logs");
        state.name_input_replace_on_type = false;

        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
        );

        assert_eq!(state.mode, Mode::Terminal);
        assert!(!state.creating_new_tab);
        assert!(state.request_new_tab);
        assert_eq!(state.requested_new_tab_name.as_deref(), Some("logs"));
    }

    #[test]
    fn saving_new_tab_dialog_with_default_name_keeps_tab_auto_named() {
        let mut state = state_with_workspaces(&["test"]);
        open_new_tab_dialog(&mut state);

        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
        );

        assert_eq!(state.mode, Mode::Terminal);
        assert!(!state.creating_new_tab);
        assert!(state.request_new_tab);
        assert!(state.requested_new_tab_name.is_none());
    }

    #[test]
    fn closing_first_auto_tab_compacts_remaining_auto_tab_label_and_next_prompt() {
        let mut state = state_with_workspaces(&["test"]);
        open_new_tab_dialog(&mut state);
        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
        );

        state.workspaces[0].test_add_tab(state.requested_new_tab_name.as_deref());
        state.request_new_tab = false;
        state.requested_new_tab_name = None;

        state.workspaces[0].close_tab(0);
        state.workspaces[0].switch_tab(0);

        assert_eq!(
            state.workspaces[0].tab_display_name(0).as_deref(),
            Some("1")
        );
        assert!(state.workspaces[0].tabs[0].custom_name.is_none());

        open_new_tab_dialog(&mut state);
        assert_eq!(state.name_input, "2");
    }

    #[test]
    fn renaming_auto_tab_to_its_default_number_keeps_it_auto_named() {
        let mut state = state_with_workspaces(&["test"]);
        state.workspaces[0].test_add_tab(None);
        state.workspaces[0].switch_tab(1);

        open_rename_active_tab(&mut state, false);
        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
        );

        assert_eq!(state.mode, Mode::Terminal);
        assert!(state.workspaces[0].tabs[1].custom_name.is_none());
        assert_eq!(
            state.workspaces[0].tab_display_name(1).as_deref(),
            Some("2")
        );
    }

    #[test]
    fn confirm_close_keyboard_actions_are_direct_not_focused() {
        let mut state = state_with_workspaces(&["a", "b"]);
        state.mode = Mode::ConfirmClose;
        state.selected = 1;

        handle_confirm_close_key(
            &mut state,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()),
        );
        assert_eq!(state.mode, Mode::Navigate);
        assert_eq!(state.workspaces.len(), 2);

        state.mode = Mode::ConfirmClose;
        handle_confirm_close_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
        );
        assert_eq!(state.workspaces.len(), 1);
    }

    #[test]
    fn confirm_close_for_linked_worktree_closes_workspace_only() {
        let mut state = state_with_workspaces(&["main", "issue"]);
        state.mode = Mode::ConfirmClose;
        state.selected = 1;
        state.workspaces[1].worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
            key: "repo-key".into(),
            label: "herdr".into(),
            repo_root: "/repo/herdr".into(),
            checkout_path: "/repo/herdr-issue".into(),
            is_linked_worktree: true,
        });

        handle_confirm_close_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
        );

        assert_eq!(state.request_remove_linked_worktree, None);
        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.workspaces[0].display_name(), "main");
        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn context_menu_close_group_opens_group_close_confirmation() {
        let mut state = state_with_workspaces(&["main", "issue"]);
        state.active = Some(0);
        state.selected = 1;
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
        let menu = ContextMenuState {
            kind: ContextMenuKind::GitWorkspace {
                ws_idx: 0,
                is_linked_worktree: false,
                has_worktree_children: true,
                collapsed: false,
            },
            x: 0,
            y: 0,
            list: MenuListState::new(0),
        };
        let mut terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();

        apply_context_menu_action(&mut state, &mut terminal_runtimes, menu, 1);

        assert_eq!(state.selected, 0);
        assert_eq!(state.mode, Mode::ConfirmClose);

        confirm_close_accept(&mut state);

        assert!(state.workspaces.is_empty());
        assert_eq!(state.mode, Mode::Navigate);
    }

    fn manual_state_with_line_split(name: &str) -> (AppState, crate::app::state::LineSplitId) {
        let mut state = state_with_workspaces(&["one"]);
        state.ensure_test_terminals();
        state.mode = Mode::Terminal;
        state.agent_panel_sort = crate::app::state::AgentPanelSort::Manual;
        let id = state.agent_manual_order.new_line_split(name.to_string(), 0);
        (state, id)
    }

    fn line_split_name(state: &AppState, id: crate::app::state::LineSplitId) -> Option<String> {
        state
            .agent_manual_order
            .order
            .iter()
            .find_map(|entry| match entry {
                crate::app::state::ManualEntry::LineSplit { id: entry_id, name }
                    if *entry_id == id =>
                {
                    Some(name.clone())
                }
                _ => None,
            })
    }

    #[test]
    fn open_rename_line_split_prefills_current_name() {
        let (mut state, id) = manual_state_with_line_split("scheduled");
        open_rename_line_split(&mut state, id);

        assert_eq!(state.mode, Mode::RenameLineSplit);
        assert_eq!(state.rename_line_split_target, Some(id));
        assert_eq!(state.name_input, "scheduled");
        assert!(state.name_input_replace_on_type);
        // Line-split rename uses the shared caret infrastructure: the caret lands
        // at the end of the prefilled name and responds to movement keys.
        assert_eq!(state.name_input_cursor, "scheduled".chars().count());
        handle_rename_edit_key(
            &mut state,
            KeyEvent::new(KeyCode::Home, KeyModifiers::empty()),
        );
        assert_eq!(state.name_input_cursor, 0);
        handle_rename_edit_key(
            &mut state,
            KeyEvent::new(KeyCode::Right, KeyModifiers::empty()),
        );
        assert_eq!(state.name_input_cursor, 1);
    }

    #[test]
    fn apply_rename_action_writes_line_split_name_and_allows_empty() {
        let (mut state, id) = manual_state_with_line_split("scheduled");
        open_rename_line_split(&mut state, id);
        state.name_input = "later".to_string();
        state.name_input_replace_on_type = false;
        apply_rename_action(&mut state, ModalAction::Save);

        assert_eq!(line_split_name(&state, id).as_deref(), Some("later"));
        assert_eq!(state.rename_line_split_target, None);
        assert_eq!(state.mode, Mode::Terminal);

        // Committing an empty name is allowed (renders as a plain rule).
        open_rename_line_split(&mut state, id);
        state.name_input.clear();
        state.name_input_replace_on_type = false;
        apply_rename_action(&mut state, ModalAction::Save);
        assert_eq!(line_split_name(&state, id).as_deref(), Some(""));
        state.assert_invariants_for_test();
    }

    #[test]
    fn context_menu_delete_removes_line_split() {
        let (mut state, id) = manual_state_with_line_split("scheduled");
        let mut terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();
        let menu = ContextMenuState {
            kind: ContextMenuKind::LineSplit { id },
            x: 0,
            y: 0,
            list: MenuListState::new(0),
        };
        // "Delete" is the second item for a line-split menu.
        apply_context_menu_action(&mut state, &mut terminal_runtimes, menu, 1);

        assert!(line_split_name(&state, id).is_none());
        assert!(state.agent_manual_order.order.is_empty());
        state.assert_invariants_for_test();
    }

    #[test]
    fn context_menu_rename_opens_line_split_rename() {
        let (mut state, id) = manual_state_with_line_split("scheduled");
        let mut terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();
        let menu = ContextMenuState {
            kind: ContextMenuKind::LineSplit { id },
            x: 0,
            y: 0,
            list: MenuListState::new(0),
        };
        apply_context_menu_action(&mut state, &mut terminal_runtimes, menu, 0);

        assert_eq!(state.mode, Mode::RenameLineSplit);
        assert_eq!(state.rename_line_split_target, Some(id));
    }

    #[test]
    fn context_menu_close_pane_last_parent_group_pane_keeps_confirmation_mode() {
        let mut state = state_with_workspaces(&["main", "issue"]);
        state.active = Some(0);
        state.selected = 1;
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
        let pane_id = state.workspaces[0].tabs[0].root_pane;
        let menu = ContextMenuState {
            kind: ContextMenuKind::Pane {
                ws_idx: 0,
                tab_idx: 0,
                pane_id,
                source_pane_id: None,
                has_manual_label: false,
            },
            x: 0,
            y: 0,
            list: MenuListState::new(0),
        };
        let idx = menu
            .items()
            .iter()
            .position(|item| *item == "Close pane")
            .expect("close pane item");
        let mut terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();

        apply_context_menu_action(&mut state, &mut terminal_runtimes, menu, idx);

        assert_eq!(state.selected, 0);
        assert_eq!(state.mode, Mode::ConfirmClose);
        assert_eq!(state.workspaces.len(), 2);
    }

    fn rename_state(text: &str) -> AppState {
        let mut state = state_with_workspaces(&["test"]);
        state.mode = Mode::RenameWorkspace;
        state.set_name_input(text);
        state
    }

    fn press(state: &mut AppState, code: KeyCode) {
        handle_rename_key(state, KeyEvent::new(code, KeyModifiers::empty()));
    }

    #[test]
    fn caret_moves_left_right_and_clamps_at_bounds() {
        let mut state = rename_state("abc");
        assert_eq!(state.name_input_cursor, 3);

        press(&mut state, KeyCode::Left);
        assert_eq!(state.name_input_cursor, 2);
        press(&mut state, KeyCode::Left);
        press(&mut state, KeyCode::Left);
        assert_eq!(state.name_input_cursor, 0);
        // Clamps at start.
        press(&mut state, KeyCode::Left);
        assert_eq!(state.name_input_cursor, 0);

        press(&mut state, KeyCode::Right);
        assert_eq!(state.name_input_cursor, 1);
        press(&mut state, KeyCode::Right);
        press(&mut state, KeyCode::Right);
        assert_eq!(state.name_input_cursor, 3);
        // Clamps at end.
        press(&mut state, KeyCode::Right);
        assert_eq!(state.name_input_cursor, 3);
    }

    #[test]
    fn caret_home_and_end_jump_to_bounds() {
        let mut state = rename_state("abc");
        press(&mut state, KeyCode::Home);
        assert_eq!(state.name_input_cursor, 0);
        press(&mut state, KeyCode::End);
        assert_eq!(state.name_input_cursor, 3);

        // Ctrl+A / Ctrl+E mirror Home / End.
        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
        );
        assert_eq!(state.name_input_cursor, 0);
        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL),
        );
        assert_eq!(state.name_input_cursor, 3);
    }

    #[test]
    fn insert_happens_at_caret() {
        let mut state = rename_state("abc");
        press(&mut state, KeyCode::Left); // caret between b and c
        press(&mut state, KeyCode::Char('X'));
        assert_eq!(state.name_input, "abXc");
        assert_eq!(state.name_input_cursor, 3);
    }

    #[test]
    fn backspace_deletes_before_caret_and_delete_after() {
        let mut state = rename_state("abcd");
        press(&mut state, KeyCode::Left); // caret between c and d
        press(&mut state, KeyCode::Backspace);
        assert_eq!(state.name_input, "abd");
        assert_eq!(state.name_input_cursor, 2);

        press(&mut state, KeyCode::Delete);
        assert_eq!(state.name_input, "ab");
        assert_eq!(state.name_input_cursor, 2);

        // Delete at end is a no-op.
        press(&mut state, KeyCode::Delete);
        assert_eq!(state.name_input, "ab");

        // Backspace at start is a no-op.
        press(&mut state, KeyCode::Home);
        press(&mut state, KeyCode::Backspace);
        assert_eq!(state.name_input, "ab");
        assert_eq!(state.name_input_cursor, 0);
    }

    #[test]
    fn paste_inserts_at_caret() {
        let mut state = rename_state("abc");
        press(&mut state, KeyCode::Home);
        press(&mut state, KeyCode::Right); // between a and b
        insert_rename_input_text(&mut state, "XY");
        assert_eq!(state.name_input, "aXYbc");
        assert_eq!(state.name_input_cursor, 3);
    }

    #[test]
    fn replace_on_type_places_caret_after_inserted_text() {
        let mut state = rename_state("2");
        state.name_input_replace_on_type = true;

        press(&mut state, KeyCode::Char('n'));
        assert_eq!(state.name_input, "n");
        assert_eq!(state.name_input_cursor, 1);
        assert!(!state.name_input_replace_on_type);
    }

    #[test]
    fn arrow_first_cancels_replace_on_type_without_wiping() {
        let mut state = rename_state("prefill");
        state.name_input_replace_on_type = true;

        press(&mut state, KeyCode::Left);
        assert_eq!(state.name_input, "prefill");
        assert!(!state.name_input_replace_on_type);
        assert_eq!(state.name_input_cursor, 6);

        // Subsequent typing now inserts at the caret instead of replacing.
        press(&mut state, KeyCode::Char('X'));
        assert_eq!(state.name_input, "prefilXl");
    }

    #[test]
    fn word_movement_steps_over_words() {
        let mut state = rename_state("feature/logs here");
        // caret at end (17)
        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL),
        );
        assert_eq!(state.name_input_cursor, 13); // start of "here"
        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL),
        );
        assert_eq!(state.name_input_cursor, 8); // start of "logs"

        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL),
        );
        assert_eq!(state.name_input_cursor, 13); // moved over "logs" to "here"
    }

    #[test]
    fn caret_edits_around_cjk_stay_on_char_boundaries() {
        // Each CJK char is multiple bytes; edits must never split them.
        let mut state = rename_state("提交X反馈");
        assert_eq!(state.name_input_cursor, 5);

        press(&mut state, KeyCode::Home);
        press(&mut state, KeyCode::Right); // between 提 and 交
        press(&mut state, KeyCode::Char('a'));
        assert_eq!(state.name_input, "提a交X反馈");
        assert_eq!(state.name_input_cursor, 2);

        // Forward-delete removes the whole CJK char after the caret.
        press(&mut state, KeyCode::Delete);
        assert_eq!(state.name_input, "提aX反馈");
        assert_eq!(state.name_input_cursor, 2);

        // Backspace removes the char before the caret.
        press(&mut state, KeyCode::Backspace);
        assert_eq!(state.name_input, "提X反馈");
        assert_eq!(state.name_input_cursor, 1);

        // Move to end and delete a trailing CJK char.
        press(&mut state, KeyCode::End);
        press(&mut state, KeyCode::Backspace);
        assert_eq!(state.name_input, "提X反");
    }
}
