use std::collections::HashMap;
use std::path::PathBuf;

use ratatui::layout::Direction;
use serde::{Deserialize, Serialize};

use crate::layout::Node;
use crate::terminal::TerminalRuntimeRegistry;
use crate::workspace::Workspace;

/// Current snapshot format version.
pub(super) const SNAPSHOT_VERSION: u32 = 9;

/// Serializable snapshot of the entire herdr session.
#[derive(Serialize, Deserialize)]
pub struct SessionSnapshot {
    /// Format version — used to detect incompatible changes.
    #[serde(default)]
    pub version: u32,
    pub workspaces: Vec<WorkspaceSnapshot>,
    pub active: Option<usize>,
    pub selected: usize,
    #[serde(default)]
    pub sidebar_width: Option<u16>,
    #[serde(default)]
    pub sidebar_section_split: Option<f32>,
    /// Ratio of the sidebar region below the Spaces band allocated to the Panes
    /// section. Optional for back-compat.
    #[serde(default)]
    pub sidebar_pane_section_split: Option<f32>,
    #[serde(default)]
    pub collapsed_space_keys: std::collections::HashSet<String>,
    /// Public pane ids of collapsed agent-tree parents (TUI presentation state).
    /// Optional for back-compat.
    #[serde(default)]
    pub collapsed_agent_keys: std::collections::HashSet<String>,
    /// Section-namespaced keys of collapsed line-split dividers (TUI presentation
    /// state). Optional for back-compat.
    #[serde(default)]
    pub collapsed_line_split_keys: std::collections::HashSet<String>,
    /// Flat manual agent ordering (TUI presentation state). Serialized by stable
    /// keys so it survives the PaneId remap on restore. Optional for back-compat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_manual_order: Option<AgentManualOrderSnapshot>,
    /// Flat Panes-section ordering (TUI presentation state). Serialized by stable
    /// (workspace id + public pane number) references so it survives the PaneId
    /// remap on restore. Optional for back-compat; older tab-based snapshots
    /// omit or mismatch this field and the section simply rebuilds from the
    /// current panes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_section_order: Option<PaneSectionOrderSnapshot>,
}

/// Persisted flat Panes-section ordering. Pane entries reference non-agent panes
/// by stable keys (workspace id + public pane number) rather than a positional
/// index; line-splits carry their id and name.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct PaneSectionOrderSnapshot {
    pub entries: Vec<PaneSectionEntrySnapshot>,
}

/// A single persisted Panes-section entry. Panes keep the stable
/// (workspace id + public pane number) keying; line-splits carry their id and
/// name. Untagged so pre-line-split snapshots (bare pane objects, version 7 and
/// earlier) still parse as the `Pane` variant.
#[derive(Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum PaneSectionEntrySnapshot {
    Pane {
        workspace_id: String,
        pane_number: usize,
    },
    LineSplit {
        line_split_id: u64,
        name: String,
    },
}

/// Persisted flat manual agent ordering. Entries reference panes by stable keys
/// (workspace id + public pane number) rather than the volatile `PaneId`.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AgentManualOrderSnapshot {
    pub entries: Vec<AgentManualEntrySnapshot>,
}

/// A single persisted manual-order entry. Panes keep the stable
/// (workspace id + public pane number) keying; line-splits carry their id and
/// name. Untagged so pre-line-split snapshots (bare pane objects) still parse.
#[derive(Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum AgentManualEntrySnapshot {
    Pane {
        workspace_id: String,
        pane_number: usize,
    },
    LineSplit {
        line_split_id: u64,
        name: String,
    },
}

#[derive(Serialize, Deserialize)]
pub struct SessionHistorySnapshot {
    /// Format version follows the matching session snapshot version.
    #[serde(default)]
    pub version: u32,
    pub workspaces: Vec<WorkspaceHistorySnapshot>,
}

#[derive(Serialize, Deserialize)]
pub struct WorkspaceHistorySnapshot {
    pub tabs: Vec<TabHistorySnapshot>,
}

#[derive(Serialize, Deserialize)]
pub struct TabHistorySnapshot {
    pub panes: HashMap<u32, PaneHistorySnapshot>,
}

#[derive(Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub custom_name: Option<String>,
    pub identity_cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_space: Option<crate::workspace::WorktreeSpaceMembership>,
    #[serde(default)]
    pub public_pane_numbers: HashMap<u32, usize>,
    #[serde(default)]
    pub next_public_pane_number: usize,
    #[serde(default)]
    pub public_tab_numbers: Vec<usize>,
    #[serde(default)]
    pub next_public_tab_number: usize,
    pub tabs: Vec<TabSnapshot>,
    #[serde(default)]
    pub active_tab: usize,
    #[serde(default)]
    pub home_tab: usize,
}

#[derive(Deserialize)]
struct LegacyWorkspaceSnapshot {
    #[serde(default)]
    custom_name: Option<String>,
    layout: LayoutSnapshot,
    panes: HashMap<u32, PaneSnapshot>,
    zoomed: bool,
    #[serde(default)]
    focused: Option<u32>,
    #[serde(default)]
    root_pane: Option<u32>,
}

#[derive(Serialize, Deserialize)]
pub struct TabSnapshot {
    #[serde(default)]
    pub custom_name: Option<String>,
    pub layout: LayoutSnapshot,
    pub panes: HashMap<u32, PaneSnapshot>,
    pub zoomed: bool,
    #[serde(default)]
    pub focused: Option<u32>,
    #[serde(default)]
    pub root_pane: Option<u32>,
}

#[derive(Serialize, Deserialize)]
pub struct PaneSnapshot {
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session: Option<PaneAgentSessionSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_argv: Option<Vec<String>>,
    /// Stable reference to this pane's parent agent (workspace id + public pane
    /// number). Present only for child agents started with `--parent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<crate::pane::PaneParentRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneAgentSessionSnapshot {
    pub source: String,
    pub agent: String,
    pub kind: crate::agent_resume::AgentSessionRefKind,
    pub value: String,
}

#[derive(Serialize, Deserialize)]
pub struct PaneHistorySnapshot {
    pub ansi: String,
    pub lines: usize,
}

/// Serializable BSP tree.
#[derive(Serialize, Deserialize)]
pub enum LayoutSnapshot {
    Pane(u32),
    Split {
        direction: DirectionSnapshot,
        ratio: f32,
        first: Box<LayoutSnapshot>,
        second: Box<LayoutSnapshot>,
    },
}

#[derive(Serialize, Deserialize)]
pub enum DirectionSnapshot {
    Horizontal,
    Vertical,
}

impl From<LegacyWorkspaceSnapshot> for WorkspaceSnapshot {
    fn from(snap: LegacyWorkspaceSnapshot) -> Self {
        let identity_cwd = legacy_identity_cwd(&snap);
        let tab = TabSnapshot {
            custom_name: None,
            layout: snap.layout,
            panes: snap.panes,
            zoomed: snap.zoomed,
            focused: snap.focused,
            root_pane: snap.root_pane,
        };

        Self {
            id: None,
            custom_name: snap.custom_name,
            identity_cwd,
            worktree_space: None,
            public_pane_numbers: HashMap::new(),
            next_public_pane_number: 0,
            public_tab_numbers: Vec::new(),
            next_public_tab_number: 0,
            tabs: vec![tab],
            active_tab: 0,
            home_tab: 0,
        }
    }
}

#[derive(Deserialize)]
struct RawSessionSnapshot {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    workspaces: Vec<serde_json::Value>,
    #[serde(default)]
    active: Option<usize>,
    #[serde(default)]
    selected: usize,
    #[serde(default)]
    sidebar_width: Option<u16>,
    #[serde(default)]
    sidebar_section_split: Option<f32>,
    #[serde(default)]
    sidebar_pane_section_split: Option<f32>,
    #[serde(default)]
    collapsed_space_keys: std::collections::HashSet<String>,
    #[serde(default)]
    collapsed_agent_keys: std::collections::HashSet<String>,
    #[serde(default)]
    collapsed_line_split_keys: std::collections::HashSet<String>,
    #[serde(default)]
    agent_manual_order: Option<AgentManualOrderSnapshot>,
    #[serde(default)]
    pane_section_order: Option<PaneSectionOrderSnapshot>,
}

fn migrate_snapshot(raw: RawSessionSnapshot) -> Result<SessionSnapshot, String> {
    Ok(SessionSnapshot {
        version: raw.version,
        workspaces: raw
            .workspaces
            .into_iter()
            .map(migrate_workspace)
            .collect::<Result<Vec<_>, _>>()?,
        active: raw.active,
        selected: raw.selected,
        sidebar_width: raw.sidebar_width,
        sidebar_section_split: raw.sidebar_section_split,
        sidebar_pane_section_split: raw.sidebar_pane_section_split,
        collapsed_space_keys: raw.collapsed_space_keys,
        collapsed_agent_keys: raw.collapsed_agent_keys,
        collapsed_line_split_keys: raw.collapsed_line_split_keys,
        agent_manual_order: raw.agent_manual_order,
        pane_section_order: raw.pane_section_order,
    })
}

fn migrate_workspace(raw: serde_json::Value) -> Result<WorkspaceSnapshot, String> {
    if raw.get("identity_cwd").is_some() {
        return serde_json::from_value(raw).map_err(|e| e.to_string());
    }

    if raw.get("layout").is_some() {
        let legacy =
            serde_json::from_value::<LegacyWorkspaceSnapshot>(raw).map_err(|e| e.to_string())?;
        return Ok(legacy.into());
    }

    Err("workspace snapshot is neither current nor legacy format".to_string())
}

fn legacy_identity_cwd(snap: &LegacyWorkspaceSnapshot) -> PathBuf {
    let root_pane = snap
        .root_pane
        .or_else(|| first_pane_id_in_layout(&snap.layout));

    root_pane
        .and_then(|pane_id| snap.panes.get(&pane_id))
        .map(|pane| pane.cwd.clone())
        .or_else(|| {
            first_pane_id_in_layout(&snap.layout)
                .and_then(|pane_id| snap.panes.get(&pane_id))
                .map(|pane| pane.cwd.clone())
        })
        .or_else(|| {
            snap.panes
                .keys()
                .min()
                .and_then(|pane_id| snap.panes.get(pane_id))
                .map(|pane| pane.cwd.clone())
        })
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()))
}

fn first_pane_id_in_layout(layout: &LayoutSnapshot) -> Option<u32> {
    match layout {
        LayoutSnapshot::Pane(id) => Some(*id),
        LayoutSnapshot::Split { first, second, .. } => {
            first_pane_id_in_layout(first).or_else(|| first_pane_id_in_layout(second))
        }
    }
}

/// Capture the current app state into a serializable snapshot.
// Snapshot capture legitimately aggregates the many independent persisted
// facets (geometry, collapse state, orderings) into one owned snapshot; a
// parameter object would just move the same fields elsewhere.
#[allow(clippy::too_many_arguments)]
pub fn capture(
    workspaces: &[Workspace],
    terminals: &std::collections::HashMap<
        crate::terminal::TerminalId,
        crate::terminal::TerminalState,
    >,
    terminal_runtimes: &TerminalRuntimeRegistry,
    active: Option<usize>,
    selected: usize,
    sidebar_width: u16,
    sidebar_section_split: f32,
    sidebar_pane_section_split: f32,
    collapsed_space_keys: std::collections::HashSet<String>,
    collapsed_agent_keys: std::collections::HashSet<String>,
    collapsed_line_split_keys: std::collections::HashSet<String>,
    agent_manual_order_keys: Vec<crate::app::state::ManualOrderEntryKey>,
    pane_section_order_keys: Vec<crate::app::state::PaneManualEntryKey>,
) -> SessionSnapshot {
    let pane_section_order =
        (!pane_section_order_keys.is_empty()).then(|| PaneSectionOrderSnapshot {
            entries: pane_section_order_keys
                .into_iter()
                .map(|key| match key {
                    crate::app::state::PaneManualEntryKey::Pane {
                        workspace_id,
                        pane_number,
                    } => PaneSectionEntrySnapshot::Pane {
                        workspace_id,
                        pane_number,
                    },
                    crate::app::state::PaneManualEntryKey::LineSplit { id, name } => {
                        PaneSectionEntrySnapshot::LineSplit {
                            line_split_id: id,
                            name,
                        }
                    }
                })
                .collect(),
        });
    let agent_manual_order =
        (!agent_manual_order_keys.is_empty()).then(|| AgentManualOrderSnapshot {
            entries: agent_manual_order_keys
                .into_iter()
                .map(|key| match key {
                    crate::app::state::ManualOrderEntryKey::Pane {
                        workspace_id,
                        pane_number,
                    } => AgentManualEntrySnapshot::Pane {
                        workspace_id,
                        pane_number,
                    },
                    crate::app::state::ManualOrderEntryKey::LineSplit { id, name } => {
                        AgentManualEntrySnapshot::LineSplit {
                            line_split_id: id,
                            name,
                        }
                    }
                })
                .collect(),
        });
    SessionSnapshot {
        version: SNAPSHOT_VERSION,
        workspaces: workspaces
            .iter()
            .map(|workspace| capture_workspace(workspace, terminals, terminal_runtimes))
            .collect(),
        active,
        selected,
        sidebar_width: Some(sidebar_width),
        sidebar_section_split: Some(sidebar_section_split),
        sidebar_pane_section_split: Some(sidebar_pane_section_split),
        collapsed_space_keys,
        collapsed_agent_keys,
        collapsed_line_split_keys,
        agent_manual_order,
        pane_section_order,
    }
}

/// Capture a workspace snapshot for the undo-close stack. Unlike the session
/// snapshot path this has no live runtime registry, so pane cwds fall back to
/// each terminal's recorded cwd. Reopening spawns fresh shells there.
pub(crate) fn capture_workspace_for_undo(
    ws: &Workspace,
    terminals: &std::collections::HashMap<
        crate::terminal::TerminalId,
        crate::terminal::TerminalState,
    >,
) -> WorkspaceSnapshot {
    capture_workspace(ws, terminals, &TerminalRuntimeRegistry::new())
}

/// Capture a single tab snapshot for the undo-close stack. See
/// [`capture_workspace_for_undo`] for the cwd fallback behavior.
pub(crate) fn capture_tab_for_undo(
    tab: &crate::workspace::Tab,
    terminals: &std::collections::HashMap<
        crate::terminal::TerminalId,
        crate::terminal::TerminalState,
    >,
) -> TabSnapshot {
    capture_tab(tab, terminals, &TerminalRuntimeRegistry::new())
}

fn capture_workspace(
    ws: &Workspace,
    terminals: &std::collections::HashMap<
        crate::terminal::TerminalId,
        crate::terminal::TerminalState,
    >,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        id: Some(ws.id.clone()),
        custom_name: ws.custom_name.clone(),
        identity_cwd: ws
            .resolved_identity_cwd_from(terminals, terminal_runtimes)
            .unwrap_or_else(|| ws.identity_cwd.clone()),
        worktree_space: ws.worktree_space.clone(),
        public_pane_numbers: ws
            .public_pane_numbers
            .iter()
            .map(|(pane_id, number)| (pane_id.raw(), *number))
            .collect(),
        next_public_pane_number: ws.next_public_pane_number,
        public_tab_numbers: ws.tabs.iter().map(|tab| tab.number).collect(),
        next_public_tab_number: ws.next_public_tab_number,
        tabs: ws
            .tabs
            .iter()
            .map(|tab| capture_tab(tab, terminals, terminal_runtimes))
            .collect(),
        active_tab: ws.active_tab,
        home_tab: ws.home_tab,
    }
}

fn capture_tab(
    tab: &crate::workspace::Tab,
    terminals: &std::collections::HashMap<
        crate::terminal::TerminalId,
        crate::terminal::TerminalState,
    >,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> TabSnapshot {
    let mut panes = HashMap::new();
    for id in tab.panes.keys() {
        let cwd = tab
            .cwd_for_pane(*id, terminals, terminal_runtimes)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));
        let label = tab
            .panes
            .get(id)
            .and_then(|pane| terminals.get(&pane.attached_terminal_id))
            .and_then(|terminal| terminal.manual_label.clone());
        let agent_name = tab
            .panes
            .get(id)
            .and_then(|pane| terminals.get(&pane.attached_terminal_id))
            .and_then(|terminal| terminal.agent_name.clone());
        let launch_argv = tab
            .panes
            .get(id)
            .and_then(|pane| terminals.get(&pane.attached_terminal_id))
            .and_then(|terminal| terminal.launch_argv.clone());
        let agent_session =
            tab.panes
                .get(id)
                .and_then(|pane| terminals.get(&pane.attached_terminal_id))
                .and_then(|terminal| {
                    if let Some(authority) = terminal.hook_authority.as_ref() {
                        if let Some(session_ref) = authority.session_ref.as_ref() {
                            return Some(PaneAgentSessionSnapshot {
                                source: authority.source.clone(),
                                agent: authority.agent_label.clone(),
                                kind: session_ref.kind,
                                value: session_ref.value.clone(),
                            });
                        }
                    }
                    terminal.persisted_agent_session.as_ref().map(|session| {
                        PaneAgentSessionSnapshot {
                            source: session.source.clone(),
                            agent: session.agent.clone(),
                            kind: session.session_ref.kind,
                            value: session.session_ref.value.clone(),
                        }
                    })
                });
        let parent = tab.panes.get(id).and_then(|pane| pane.parent.clone());
        panes.insert(
            id.raw(),
            PaneSnapshot {
                cwd,
                label,
                agent_name,
                agent_session,
                launch_argv,
                parent,
            },
        );
    }
    TabSnapshot {
        custom_name: tab.custom_name.clone(),
        layout: capture_node(tab.layout.root()),
        panes,
        zoomed: tab.zoomed,
        focused: Some(tab.layout.focused().raw()),
        root_pane: Some(tab.root_pane.raw()),
    }
}

/// Capture pane screen history separately from the structural session snapshot.
pub fn capture_history(
    workspaces: &[Workspace],
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> SessionHistorySnapshot {
    SessionHistorySnapshot {
        version: SNAPSHOT_VERSION,
        workspaces: workspaces
            .iter()
            .map(|workspace| WorkspaceHistorySnapshot {
                tabs: workspace
                    .tabs
                    .iter()
                    .map(|tab| TabHistorySnapshot {
                        panes: capture_tab_history(tab, terminal_runtimes),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn capture_tab_history(
    tab: &crate::workspace::Tab,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> HashMap<u32, PaneHistorySnapshot> {
    let mut panes = HashMap::new();
    for (id, pane) in &tab.panes {
        if let Some(history) = capture_pane_history(Some(pane), terminal_runtimes) {
            panes.insert(id.raw(), history);
        }
    }
    panes
}

fn capture_pane_history(
    pane: Option<&crate::pane::PaneState>,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> Option<PaneHistorySnapshot> {
    let ansi = terminal_runtimes
        .get(&pane?.attached_terminal_id)?
        .snapshot_history()?;
    let lines = ansi.lines().count();
    Some(PaneHistorySnapshot { ansi, lines })
}

pub(super) fn capture_node(node: &Node) -> LayoutSnapshot {
    match node {
        Node::Pane(id) => LayoutSnapshot::Pane(id.raw()),
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => LayoutSnapshot::Split {
            direction: match direction {
                Direction::Horizontal => DirectionSnapshot::Horizontal,
                Direction::Vertical => DirectionSnapshot::Vertical,
            },
            ratio: *ratio,
            first: Box::new(capture_node(first)),
            second: Box::new(capture_node(second)),
        },
    }
}

pub(super) fn parse_snapshot(content: &str) -> Result<SessionSnapshot, String> {
    let raw = serde_json::from_str::<RawSessionSnapshot>(content).map_err(|e| e.to_string())?;
    if raw.version > SNAPSHOT_VERSION {
        return Err(format!(
            "snapshot version {} is newer than supported {}",
            raw.version, SNAPSHOT_VERSION
        ));
    }
    migrate_snapshot(raw)
}

pub(super) fn parse_history_snapshot(content: &str) -> Result<SessionHistorySnapshot, String> {
    let snapshot =
        serde_json::from_str::<SessionHistorySnapshot>(content).map_err(|e| e.to_string())?;
    if snapshot.version > SNAPSHOT_VERSION {
        return Err(format!(
            "history snapshot version {} is newer than supported {}",
            snapshot.version, SNAPSHOT_VERSION
        ));
    }
    Ok(snapshot)
}

pub(super) fn snapshot_file_version(content: &str) -> Option<u32> {
    serde_json::from_str::<RawSessionSnapshot>(content)
        .ok()
        .map(|raw| raw.version)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use ratatui::layout::{Direction, Rect};

    use super::*;
    use crate::app::{AppState, Mode};
    use crate::layout::NavDirection;
    use crate::workspace::Workspace;

    fn session_fixture(name: &str) -> &'static str {
        match name {
            "current-herdr" => {
                include_str!("../../tests/fixtures/session/current-herdr-session.json")
            }
            "current-herdr-dev" => {
                include_str!("../../tests/fixtures/session/current-herdr-dev-session.json")
            }
            "legacy-pre-tabs-v2" => {
                include_str!("../../tests/fixtures/session/legacy-pre-tabs-v2.json")
            }
            other => panic!("unknown session fixture: {other}"),
        }
    }

    fn test_session_path(name: &str) -> String {
        std::env::current_dir()
            .unwrap()
            .join(name)
            .display()
            .to_string()
    }

    fn state_with_workspaces(names: &[&str]) -> AppState {
        let mut state = AppState::test_new();
        state.workspaces = names.iter().map(|name| Workspace::test_new(name)).collect();
        state.ensure_test_terminals();
        if !state.workspaces.is_empty() {
            state.active = Some(0);
            state.selected = 0;
            state.mode = Mode::Terminal;
        }
        state
    }

    #[test]
    fn pane_parent_link_survives_capture_and_parse() {
        let mut state = AppState::test_new();
        let mut ws = Workspace::test_new("one");
        ws.active_tab = 0;
        let parent = ws.tabs[0].root_pane;
        let child = ws.test_split(ratatui::layout::Direction::Horizontal);
        let parent_number = ws.public_pane_number(parent).unwrap();
        let child_number = ws.public_pane_number(child).unwrap();
        let workspace_id = ws.id.clone();
        ws.pane_state_mut(child).unwrap().parent = Some(crate::pane::PaneParentRef {
            workspace_id: workspace_id.clone(),
            pane_number: parent_number,
        });
        state.workspaces = vec![ws];
        state.ensure_test_terminals();
        state.active = Some(0);
        state.selected = 0;

        let child_raw = child.raw();
        let snap = capture_from_state(&state);
        let json = serde_json::to_string(&snap).unwrap();
        let parsed = parse_snapshot(&json).unwrap();

        let restored_pane = parsed.workspaces[0].tabs[0]
            .panes
            .get(&child_raw)
            .expect("child pane persisted");
        let parent_ref = restored_pane
            .parent
            .as_ref()
            .expect("parent link persisted");
        assert_eq!(parent_ref.workspace_id, workspace_id);
        assert_eq!(parent_ref.pane_number, parent_number);

        // The reference is by stable public number, so it survives the PaneId
        // remap: a fresh state whose panes carry different PaneIds but the same
        // workspace id + public numbers still resolves the parent.
        let mut remapped = AppState::test_new();
        let mut ws2 = Workspace::test_new("one");
        ws2.id = workspace_id.clone();
        ws2.active_tab = 0;
        let new_parent = ws2.tabs[0].root_pane;
        let new_child = ws2.test_split(ratatui::layout::Direction::Horizontal);
        // Force the same public numbers the snapshot referenced.
        ws2.public_pane_numbers.clear();
        ws2.public_pane_numbers.insert(new_parent, parent_number);
        ws2.public_pane_numbers.insert(new_child, child_number);
        ws2.pane_state_mut(new_child).unwrap().parent = Some(parent_ref.clone());
        remapped.workspaces = vec![ws2];
        remapped.ensure_test_terminals();
        assert_ne!(new_parent, parent, "remap uses a fresh PaneId");
        let (ws_idx, resolved) = remapped
            .resolve_pane_parent(parent_ref)
            .expect("stable ref resolves after remap");
        assert_eq!(ws_idx, 0);
        assert_eq!(resolved, new_parent);
    }

    fn capture_from_state(state: &AppState) -> SessionSnapshot {
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        capture_from_state_with_runtimes(state, &terminal_runtimes)
    }

    fn capture_from_state_with_runtimes(
        state: &AppState,
        terminal_runtimes: &TerminalRuntimeRegistry,
    ) -> SessionSnapshot {
        capture(
            &state.workspaces,
            &state.terminals,
            terminal_runtimes,
            state.active,
            state.selected,
            state.sidebar_width,
            state.sidebar_section_split,
            state.sidebar_pane_section_split,
            state.collapsed_space_keys.clone(),
            state.collapsed_agent_keys.clone(),
            state.collapsed_line_split_keys.clone(),
            state.agent_manual_order.to_public_keys(&state.workspaces),
            state.pane_section_order.to_entry_keys(),
        )
    }

    fn capture_history_from_state_with_runtimes(
        state: &AppState,
        terminal_runtimes: &TerminalRuntimeRegistry,
    ) -> SessionHistorySnapshot {
        capture_history(&state.workspaces, terminal_runtimes)
    }

    fn root_split_ratio(tab: &TabSnapshot) -> Option<f32> {
        match &tab.layout {
            LayoutSnapshot::Split { ratio, .. } => Some(*ratio),
            LayoutSnapshot::Pane(_) => None,
        }
    }

    #[test]
    fn round_trip_empty_session() {
        let snap = SessionSnapshot {
            version: SNAPSHOT_VERSION,
            workspaces: vec![],
            active: None,
            selected: 0,
            sidebar_width: Some(26),
            sidebar_section_split: Some(0.5),
            collapsed_space_keys: std::collections::HashSet::new(),
            collapsed_agent_keys: std::collections::HashSet::new(),
            collapsed_line_split_keys: std::collections::HashSet::new(),
            agent_manual_order: None,
            sidebar_pane_section_split: None,
            pane_section_order: None,
        };
        let json = serde_json::to_string(&snap).unwrap();
        let restored = parse_snapshot(&json).unwrap();
        assert!(restored.workspaces.is_empty());
        assert_eq!(restored.active, None);
        assert_eq!(restored.sidebar_width, Some(26));
        assert_eq!(restored.sidebar_section_split, Some(0.5));
    }

    #[test]
    fn pane_section_order_snapshot_roundtrip() {
        let snap = SessionSnapshot {
            version: SNAPSHOT_VERSION,
            workspaces: vec![],
            active: None,
            selected: 0,
            sidebar_width: Some(26),
            sidebar_section_split: Some(0.4),
            sidebar_pane_section_split: Some(0.6),
            collapsed_space_keys: Default::default(),
            collapsed_agent_keys: Default::default(),
            collapsed_line_split_keys: Default::default(),
            agent_manual_order: None,
            pane_section_order: Some(PaneSectionOrderSnapshot {
                entries: vec![
                    PaneSectionEntrySnapshot::Pane {
                        workspace_id: "w2".into(),
                        pane_number: 1,
                    },
                    PaneSectionEntrySnapshot::LineSplit {
                        line_split_id: 5,
                        name: "scheduled".into(),
                    },
                    PaneSectionEntrySnapshot::Pane {
                        workspace_id: "w1".into(),
                        pane_number: 3,
                    },
                ],
            }),
        };
        let json = serde_json::to_string(&snap).unwrap();
        let restored = parse_snapshot(&json).unwrap();
        assert_eq!(restored.sidebar_pane_section_split, Some(0.6));
        let entries = restored
            .pane_section_order
            .expect("pane section order persisted")
            .entries;
        assert_eq!(entries.len(), 3);
        assert!(matches!(
            &entries[0],
            PaneSectionEntrySnapshot::Pane { workspace_id, pane_number }
                if workspace_id == "w2" && *pane_number == 1
        ));
        assert!(matches!(
            &entries[1],
            PaneSectionEntrySnapshot::LineSplit { line_split_id, name }
                if *line_split_id == 5 && name == "scheduled"
        ));
        assert!(matches!(
            &entries[2],
            PaneSectionEntrySnapshot::Pane { workspace_id, pane_number }
                if workspace_id == "w1" && *pane_number == 3
        ));
    }

    #[test]
    fn agent_manual_order_survives_capture_and_parse() {
        let mut state = state_with_workspaces(&["one", "two"]);
        state.agent_panel_sort = crate::app::state::AgentPanelSort::Manual;
        for ws_idx in 0..state.workspaces.len() {
            let pane = state.workspaces[ws_idx].tabs[0].root_pane;
            let terminal_id = state.workspaces[ws_idx].tabs[0].panes[&pane]
                .attached_terminal_id
                .clone();
            state
                .terminals
                .get_mut(&terminal_id)
                .unwrap()
                .detected_agent = Some(crate::detect::Agent::Pi);
        }
        state.reconcile_agent_manual_order();
        let keys = state.agent_manual_order.to_public_keys(&state.workspaces);
        assert_eq!(keys.len(), 2);

        let expected: Vec<(String, usize)> = keys
            .iter()
            .map(|key| match key {
                crate::app::state::ManualOrderEntryKey::Pane {
                    workspace_id,
                    pane_number,
                } => (workspace_id.clone(), *pane_number),
                crate::app::state::ManualOrderEntryKey::LineSplit { .. } => {
                    panic!("unexpected line-split in pane-only fixture")
                }
            })
            .collect();

        let pane_keys = |entries: &[AgentManualEntrySnapshot]| -> Vec<(String, usize)> {
            entries
                .iter()
                .map(|entry| match entry {
                    AgentManualEntrySnapshot::Pane {
                        workspace_id,
                        pane_number,
                    } => (workspace_id.clone(), *pane_number),
                    AgentManualEntrySnapshot::LineSplit { .. } => {
                        panic!("unexpected line-split in pane-only fixture")
                    }
                })
                .collect()
        };

        let snap = capture_from_state(&state);
        let captured = pane_keys(
            &snap
                .agent_manual_order
                .as_ref()
                .expect("manual order captured")
                .entries,
        );
        assert_eq!(captured, expected);

        let json = serde_json::to_string(&snap).unwrap();
        let parsed = parse_snapshot(&json).unwrap();
        let parsed_keys = pane_keys(
            &parsed
                .agent_manual_order
                .expect("manual order parsed")
                .entries,
        );
        assert_eq!(parsed_keys, expected);
    }

    #[test]
    fn line_split_survives_capture_serialize_and_restore() {
        use crate::app::state::{AgentManualOrder, ManualEntry};

        let mut state = state_with_workspaces(&["one", "two"]);
        state.agent_panel_sort = crate::app::state::AgentPanelSort::Manual;
        for ws_idx in 0..state.workspaces.len() {
            let pane = state.workspaces[ws_idx].tabs[0].root_pane;
            let terminal_id = state.workspaces[ws_idx].tabs[0].panes[&pane]
                .attached_terminal_id
                .clone();
            state
                .terminals
                .get_mut(&terminal_id)
                .unwrap()
                .detected_agent = Some(crate::detect::Agent::Pi);
        }
        state.reconcile_agent_manual_order();
        // Line-split between the two agents.
        let split_id = state
            .agent_manual_order
            .new_line_split("scheduled".to_string(), 1);

        let snap = capture_from_state(&state);
        assert_eq!(snap.version, SNAPSHOT_VERSION);
        assert_eq!(SNAPSHOT_VERSION, 9);

        let json = serde_json::to_string(&snap).unwrap();
        let parsed = parse_snapshot(&json).unwrap();

        // Rebuild the manual order as restore does, against the same workspaces.
        let keys: Vec<crate::app::state::ManualOrderEntryKey> = parsed
            .agent_manual_order
            .expect("manual order parsed")
            .entries
            .iter()
            .map(|entry| match entry {
                AgentManualEntrySnapshot::Pane {
                    workspace_id,
                    pane_number,
                } => crate::app::state::ManualOrderEntryKey::Pane {
                    workspace_id: workspace_id.clone(),
                    pane_number: *pane_number,
                },
                AgentManualEntrySnapshot::LineSplit {
                    line_split_id,
                    name,
                } => crate::app::state::ManualOrderEntryKey::LineSplit {
                    id: *line_split_id,
                    name: name.clone(),
                },
            })
            .collect();
        let rebuilt = AgentManualOrder::from_public_keys(&keys, &state.workspaces);

        // Position preserved: line-split is the middle entry with its id/name.
        assert!(matches!(
            &rebuilt.order[1],
            ManualEntry::LineSplit { id, name } if *id == split_id && name == "scheduled"
        ));
        assert!(matches!(rebuilt.order[0], ManualEntry::Pane(_)));
        assert!(matches!(rebuilt.order[2], ManualEntry::Pane(_)));
        // Next id sits above the restored maximum.
        assert!(rebuilt.next_line_split_id > split_id.0);
    }

    #[test]
    fn pane_section_line_split_survives_capture_serialize_and_restore() {
        use crate::app::state::{PaneManualEntry, PaneSectionOrder};

        let mut state = state_with_workspaces(&["one", "two"]);
        state.reconcile_pane_section_order();
        // Two non-agent panes; insert a line-split between them.
        assert_eq!(state.pane_section_order.order.len(), 2);
        let split_id = state
            .pane_section_order
            .new_line_split("scheduled".to_string(), 1);

        let snap = capture_from_state(&state);
        assert_eq!(snap.version, SNAPSHOT_VERSION);

        let json = serde_json::to_string(&snap).unwrap();
        let parsed = parse_snapshot(&json).unwrap();

        // Rebuild the Panes-section order as restore does.
        let keys: Vec<crate::app::state::PaneManualEntryKey> = parsed
            .pane_section_order
            .expect("pane section order parsed")
            .entries
            .iter()
            .map(|entry| match entry {
                PaneSectionEntrySnapshot::Pane {
                    workspace_id,
                    pane_number,
                } => crate::app::state::PaneManualEntryKey::Pane {
                    workspace_id: workspace_id.clone(),
                    pane_number: *pane_number,
                },
                PaneSectionEntrySnapshot::LineSplit {
                    line_split_id,
                    name,
                } => crate::app::state::PaneManualEntryKey::LineSplit {
                    id: *line_split_id,
                    name: name.clone(),
                },
            })
            .collect();
        let rebuilt = PaneSectionOrder::from_entry_keys(&keys, &state.workspaces);

        // Position preserved: line-split is the middle entry with its id/name.
        assert!(matches!(
            &rebuilt.order[1],
            PaneManualEntry::LineSplit { id, name } if *id == split_id && name == "scheduled"
        ));
        assert!(matches!(rebuilt.order[0], PaneManualEntry::Pane(_)));
        assert!(matches!(rebuilt.order[2], PaneManualEntry::Pane(_)));
        // Next id sits above the restored maximum.
        assert!(rebuilt.next_line_split_id > split_id.0);
    }

    #[test]
    fn v7_pane_section_order_without_splits_still_loads() {
        // A version-7 snapshot stored bare pane objects (no line-splits). The
        // untagged entry enum must still parse those as `Pane` entries.
        let json = serde_json::json!({
            "version": 7,
            "workspaces": [],
            "active": null,
            "selected": 0,
            "pane_section_order": {
                "entries": [
                    { "workspace_id": "w1", "pane_number": 2 },
                    { "workspace_id": "w2", "pane_number": 1 }
                ]
            }
        })
        .to_string();

        let restored = parse_snapshot(&json).unwrap();
        assert_eq!(restored.version, 7);
        let entries = restored
            .pane_section_order
            .expect("v7 pane section order parses")
            .entries;
        assert_eq!(entries.len(), 2);
        assert!(matches!(
            &entries[0],
            PaneSectionEntrySnapshot::Pane { workspace_id, pane_number }
                if workspace_id == "w1" && *pane_number == 2
        ));
        assert!(matches!(&entries[1], PaneSectionEntrySnapshot::Pane { .. }));
    }

    #[test]
    fn round_trip_layout_snapshot() {
        let layout = LayoutSnapshot::Split {
            direction: DirectionSnapshot::Horizontal,
            ratio: 0.6,
            first: Box::new(LayoutSnapshot::Pane(0)),
            second: Box::new(LayoutSnapshot::Split {
                direction: DirectionSnapshot::Vertical,
                ratio: 0.5,
                first: Box::new(LayoutSnapshot::Pane(1)),
                second: Box::new(LayoutSnapshot::Pane(2)),
            }),
        };
        let json = serde_json::to_string(&layout).unwrap();
        let restored: LayoutSnapshot = serde_json::from_str(&json).unwrap();

        match restored {
            LayoutSnapshot::Split { ratio, .. } => assert!((ratio - 0.6).abs() < 0.01),
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn round_trip_full_workspace_snapshot() {
        let mut panes = HashMap::new();
        panes.insert(
            0,
            PaneSnapshot {
                cwd: PathBuf::from("/home/can/Projects/herdr"),
                label: None,
                agent_name: None,
                agent_session: None,
                launch_argv: None,
                parent: None,
            },
        );
        panes.insert(
            1,
            PaneSnapshot {
                cwd: PathBuf::from("/home/can/Projects/website"),
                label: Some("website".into()),
                agent_name: None,
                agent_session: None,
                launch_argv: None,
                parent: None,
            },
        );

        let snap = SessionSnapshot {
            workspaces: vec![WorkspaceSnapshot {
                id: Some("wproj".to_string()),
                custom_name: Some("pi-mono".to_string()),
                identity_cwd: PathBuf::from("/home/can/Projects/herdr"),
                worktree_space: None,
                public_pane_numbers: HashMap::from([(0, 1), (1, 2)]),
                next_public_pane_number: 3,
                public_tab_numbers: vec![1],
                next_public_tab_number: 2,
                tabs: vec![TabSnapshot {
                    custom_name: Some("api".to_string()),
                    layout: LayoutSnapshot::Split {
                        direction: DirectionSnapshot::Horizontal,
                        ratio: 0.5,
                        first: Box::new(LayoutSnapshot::Pane(0)),
                        second: Box::new(LayoutSnapshot::Pane(1)),
                    },
                    panes,
                    zoomed: false,
                    focused: Some(0),
                    root_pane: Some(0),
                }],
                active_tab: 0,
                home_tab: 0,
            }],
            active: Some(0),
            selected: 0,
            sidebar_width: Some(26),
            sidebar_section_split: Some(0.5),
            collapsed_space_keys: std::collections::HashSet::new(),
            collapsed_agent_keys: std::collections::HashSet::new(),
            collapsed_line_split_keys: std::collections::HashSet::new(),
            agent_manual_order: None,
            sidebar_pane_section_split: None,
            pane_section_order: None,
            version: SNAPSHOT_VERSION,
        };

        let json = serde_json::to_string_pretty(&snap).unwrap();
        let restored = parse_snapshot(&json).unwrap();

        assert_eq!(restored.workspaces.len(), 1);
        assert_eq!(restored.workspaces[0].id.as_deref(), Some("wproj"));
        assert_eq!(
            restored.workspaces[0].custom_name.as_deref(),
            Some("pi-mono")
        );
        assert_eq!(restored.workspaces[0].tabs.len(), 1);
        assert_eq!(restored.workspaces[0].tabs[0].panes.len(), 2);
        assert_eq!(
            restored.workspaces[0].tabs[0].panes[&0].cwd,
            PathBuf::from("/home/can/Projects/herdr")
        );
        assert_eq!(
            restored.workspaces[0].tabs[0].panes[&1].label.as_deref(),
            Some("website")
        );
        assert_eq!(restored.sidebar_width, Some(26));
        assert_eq!(restored.sidebar_section_split, Some(0.5));
    }

    #[test]
    fn current_session_fixture_parses() {
        let snap = parse_snapshot(session_fixture("current-herdr")).unwrap();

        assert_eq!(snap.version, 3);
        assert_eq!(snap.workspaces.len(), 2);
        assert_eq!(snap.active, Some(0));
        assert_eq!(snap.selected, 0);
        assert_eq!(snap.sidebar_width, None);
        assert_eq!(snap.sidebar_section_split, None);
        assert_eq!(snap.workspaces[0].tabs.len(), 2);
        assert_eq!(
            snap.workspaces[1].identity_cwd,
            PathBuf::from("/home/test/projects/project-b")
        );
    }

    #[test]
    fn current_dev_session_fixture_parses_additive_fields() {
        let snap = parse_snapshot(session_fixture("current-herdr-dev")).unwrap();

        assert_eq!(snap.version, 3);
        assert_eq!(snap.workspaces.len(), 2);
        assert_eq!(snap.sidebar_section_split, Some(0.4));
        assert_eq!(snap.workspaces[0].active_tab, 1);
        assert_eq!(snap.workspaces[1].tabs[0].panes.len(), 2);
    }

    #[test]
    fn old_snapshot_defaults_sidebar_fields() {
        let json = serde_json::json!({
            "version": SNAPSHOT_VERSION,
            "workspaces": [],
            "active": null,
            "selected": 0
        })
        .to_string();

        let restored = parse_snapshot(&json).unwrap();

        assert_eq!(restored.sidebar_width, None);
        assert_eq!(restored.sidebar_section_split, None);
    }

    #[test]
    fn old_tab_based_snapshot_loads_and_drops_stale_section_order() {
        // A pre-panes snapshot keyed the section by tabs, under the old field
        // names `tab_section_order` / `sidebar_tabs_section_split`. Those names no
        // longer deserialize, so the section order is simply dropped and rebuilt
        // from the current panes on the next reconcile.
        let json = serde_json::json!({
            "version": 6,
            "workspaces": [],
            "active": null,
            "selected": 0,
            "sidebar_tabs_section_split": 0.6,
            "tab_section_order": {
                "entries": [{ "workspace_id": "w1", "tab_number": 1 }]
            }
        })
        .to_string();

        let restored = parse_snapshot(&json).unwrap();

        assert_eq!(restored.version, 6);
        // Old field names are ignored; the pane-based fields default to absent.
        assert_eq!(restored.sidebar_pane_section_split, None);
        assert!(restored.pane_section_order.is_none());
    }

    #[test]
    fn old_pane_snapshot_with_embedded_history_is_ignored() {
        let json = serde_json::json!({
            "version": SNAPSHOT_VERSION,
            "workspaces": [{
                "id": "wtest",
                "identity_cwd": "/tmp",
                "tabs": [{
                    "layout": { "Pane": 0 },
                    "panes": {
                        "0": {
                            "cwd": "/tmp",
                            "history": {
                                "ansi": "legacy-secret",
                                "lines": 1
                            }
                        }
                    },
                    "zoomed": false,
                    "focused": 0,
                    "root_pane": 0
                }],
                "active_tab": 0
            }],
            "active": 0,
            "selected": 0
        })
        .to_string();

        let restored = parse_snapshot(&json).unwrap();

        let encoded = serde_json::to_string(&restored).unwrap();
        assert!(!encoded.contains("legacy-secret"));
        assert!(!encoded.contains("\"history\""));
    }

    #[test]
    fn legacy_workspace_snapshot_migrates_to_single_tab() {
        let snap = parse_snapshot(session_fixture("legacy-pre-tabs-v2")).unwrap();
        let ws = &snap.workspaces[0];

        assert_eq!(snap.version, 2);
        assert_eq!(snap.workspaces.len(), 1);
        assert_eq!(ws.custom_name.as_deref(), Some("legacy"));
        assert_eq!(ws.identity_cwd, PathBuf::from("/tmp/pion"));
        assert_eq!(ws.active_tab, 0);
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.tabs[0].focused, Some(1));
        assert_eq!(ws.tabs[0].root_pane, Some(0));
        assert_eq!(ws.tabs[0].panes[&0].cwd, PathBuf::from("/tmp/pion"));
        assert_eq!(ws.tabs[0].panes[&1].cwd, PathBuf::from("/tmp/herdr"));
    }

    #[test]
    fn capture_contract_tracks_workspace_order_active_and_selected() {
        let mut state = state_with_workspaces(&["a", "b", "c"]);
        state.active = Some(1);
        state.selected = 2;

        state.move_workspace(1, 0);

        let snapshot = capture_from_state(&state);
        let ids: Vec<_> = state.workspaces.iter().map(|ws| ws.id.clone()).collect();
        let captured_ids: Vec<_> = snapshot
            .workspaces
            .iter()
            .map(|ws| ws.id.clone().unwrap())
            .collect();
        assert_eq!(captured_ids, ids);
        assert_eq!(snapshot.active, state.active);
        assert_eq!(snapshot.selected, state.selected);
    }

    #[test]
    fn capture_contract_tracks_workspace_and_tab_names_and_active_tab() {
        let mut state = state_with_workspaces(&["one"]);
        state.workspaces[0].set_custom_name("renamed-workspace".into());
        let second_tab = state.workspaces[0].test_add_tab(Some("logs"));
        state.workspaces[0].switch_tab_sticky(second_tab);
        state.workspaces[0].tabs[0].set_custom_name("main".into());

        let snapshot = capture_from_state(&state);
        let workspace = &snapshot.workspaces[0];
        assert_eq!(workspace.custom_name.as_deref(), Some("renamed-workspace"));
        assert_eq!(workspace.active_tab, second_tab);
        assert_eq!(workspace.home_tab, second_tab);
        assert_eq!(workspace.tabs[0].custom_name.as_deref(), Some("main"));
        assert_eq!(workspace.tabs[1].custom_name.as_deref(), Some("logs"));
    }

    #[test]
    fn capture_contract_tracks_workspace_closure() {
        let mut state = state_with_workspaces(&["one", "two"]);
        state.selected = 1;
        state.active = Some(1);

        state.close_selected_workspace();

        let snapshot = capture_from_state(&state);
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(snapshot.workspaces[0].custom_name.as_deref(), Some("one"));
        assert_eq!(snapshot.active, Some(0));
        assert_eq!(snapshot.selected, 0);
    }

    #[test]
    fn capture_contract_tracks_sidebar_state() {
        let mut state = state_with_workspaces(&["one"]);
        state.sidebar_width = 31;
        state.sidebar_section_split = 0.4;
        state.collapsed_space_keys.insert("repo-key".into());

        let snapshot = capture_from_state(&state);
        assert_eq!(snapshot.sidebar_width, Some(31));
        assert_eq!(snapshot.sidebar_section_split, Some(0.4));
        assert!(snapshot.collapsed_space_keys.contains("repo-key"));
    }

    #[test]
    fn capture_roundtrip_preserves_collapsed_line_split_keys() {
        let mut state = state_with_workspaces(&["one"]);
        state.collapsed_line_split_keys.insert("agents:2".into());
        state.collapsed_line_split_keys.insert("panes:5".into());

        let snapshot = capture_from_state(&state);
        assert!(snapshot.collapsed_line_split_keys.contains("agents:2"));
        assert!(snapshot.collapsed_line_split_keys.contains("panes:5"));

        // Survives a JSON serialize/deserialize round trip.
        let json = serde_json::to_string(&snapshot).unwrap();
        let restored: SessionSnapshot = serde_json::from_str(&json).unwrap();
        assert!(restored.collapsed_line_split_keys.contains("agents:2"));
        assert!(restored.collapsed_line_split_keys.contains("panes:5"));
    }

    #[test]
    fn legacy_snapshot_without_collapsed_line_split_keys_defaults_empty() {
        let json = r#"{"version":9,"workspaces":[],"active":null,"selected":0}"#;
        let snapshot: SessionSnapshot = serde_json::from_str(json).unwrap();
        assert!(snapshot.collapsed_line_split_keys.is_empty());
    }

    #[test]
    fn capture_contract_tracks_worktree_space_membership() {
        let mut state = state_with_workspaces(&["main"]);
        state.workspaces[0].worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
            key: "repo-key".into(),
            label: "herdr".into(),
            repo_root: PathBuf::from("/repo/herdr"),
            checkout_path: PathBuf::from("/repo/herdr/worktree-a"),
            is_linked_worktree: true,
        });

        let snapshot = capture_from_state(&state);

        assert_eq!(
            snapshot.workspaces[0].worktree_space,
            state.workspaces[0].worktree_space
        );
    }

    #[test]
    fn capture_contract_tracks_layout_focus_zoom_and_root_pane() {
        let mut state = state_with_workspaces(&["one"]);
        let root = state.workspaces[0].tabs[0].root_pane;
        let second = state.workspaces[0].test_split(Direction::Horizontal);
        state.workspaces[0].tabs[0].layout.focus_pane(second);
        state.toggle_zoom();

        let snapshot = capture_from_state(&state);
        let tab = &snapshot.workspaces[0].tabs[0];
        assert!(matches!(tab.layout, LayoutSnapshot::Split { .. }));
        assert_eq!(tab.focused, Some(second.raw()));
        assert_eq!(tab.root_pane, Some(root.raw()));
        assert!(tab.zoomed);
        assert_eq!(tab.panes.len(), 2);
    }

    #[test]
    fn capture_contract_tracks_focus_navigation() {
        let mut state = state_with_workspaces(&["one"]);
        let root = state.workspaces[0].tabs[0].root_pane;
        let second = state.workspaces[0].test_split(Direction::Horizontal);
        crate::ui::compute_view(&mut state, Rect::new(0, 0, 106, 20));

        state.navigate_pane(NavDirection::Right);

        let snapshot = capture_from_state(&state);
        assert_eq!(snapshot.workspaces[0].tabs[0].focused, Some(second.raw()));
        assert_ne!(snapshot.workspaces[0].tabs[0].focused, Some(root.raw()));
    }

    #[test]
    fn capture_contract_tracks_resize_ratio_changes() {
        let mut state = state_with_workspaces(&["one"]);
        let root = state.workspaces[0].tabs[0].root_pane;
        state.workspaces[0].test_split(Direction::Horizontal);
        state.workspaces[0].layout.focus_pane(root);
        crate::ui::compute_view(&mut state, Rect::new(0, 0, 106, 20));
        let before = capture_from_state(&state);

        state.resize_pane(NavDirection::Right);

        let after = capture_from_state(&state);
        let before_ratio = root_split_ratio(&before.workspaces[0].tabs[0]).unwrap();
        let after_ratio = root_split_ratio(&after.workspaces[0].tabs[0]).unwrap();
        assert_ne!(before_ratio, after_ratio);
    }

    #[test]
    fn capture_contract_tracks_tab_closure() {
        let mut state = state_with_workspaces(&["one"]);
        let second_tab = state.workspaces[0].test_add_tab(Some("logs"));
        state.switch_tab(second_tab);

        state.close_tab();

        let snapshot = capture_from_state(&state);
        let workspace = &snapshot.workspaces[0];
        assert_eq!(workspace.tabs.len(), 1);
        assert_eq!(workspace.active_tab, 0);
        assert!(workspace.tabs[0].custom_name.is_none());
    }

    #[test]
    fn capture_contract_tracks_pane_closure() {
        let mut state = state_with_workspaces(&["one"]);
        state.workspaces[0].test_split(Direction::Horizontal);

        state.close_pane();

        let snapshot = capture_from_state(&state);
        let tab = &snapshot.workspaces[0].tabs[0];
        assert_eq!(tab.panes.len(), 1);
        assert!(matches!(tab.layout, LayoutSnapshot::Pane(_)));
        assert!(!tab.zoomed);
    }

    #[test]
    fn capture_contract_tracks_public_id_counters() {
        let mut state = state_with_workspaces(&["one"]);
        let second = state.workspaces[0].test_split(Direction::Horizontal);
        let third = state.workspaces[0].test_split(Direction::Vertical);
        let second_tab = state.workspaces[0].test_add_tab(None);

        state.workspaces[0].close_pane(second);

        let snapshot = capture_from_state(&state);
        let workspace = &snapshot.workspaces[0];
        assert_eq!(
            workspace.public_pane_numbers,
            HashMap::from([
                (state.workspaces[0].tabs[0].root_pane.raw(), 1),
                (third.raw(), 3),
                (state.workspaces[0].tabs[second_tab].root_pane.raw(), 4),
            ])
        );
        assert_eq!(workspace.next_public_pane_number, 5);
        assert_eq!(workspace.public_tab_numbers, vec![1, 2]);
        assert_eq!(workspace.next_public_tab_number, 3);
    }

    #[test]
    fn capture_contract_tracks_workspace_identity_and_pane_cwds() {
        let mut state = state_with_workspaces(&["one"]);
        let root = state.workspaces[0].tabs[0].root_pane;
        state.workspaces[0].identity_cwd = PathBuf::from("/tmp/pion");
        let second = state.workspaces[0].test_split(Direction::Horizontal);
        state.ensure_test_terminals();
        let root_terminal_id = state.workspaces[0].tabs[0].panes[&root]
            .attached_terminal_id
            .clone();
        state.terminals.get_mut(&root_terminal_id).unwrap().cwd = PathBuf::from("/tmp/pion");
        let second_terminal_id = state.workspaces[0].tabs[0].panes[&second]
            .attached_terminal_id
            .clone();
        state.terminals.get_mut(&second_terminal_id).unwrap().cwd = PathBuf::from("/tmp/herdr");

        let snapshot = capture_from_state(&state);
        let workspace = &snapshot.workspaces[0];
        let tab = &workspace.tabs[0];
        assert_eq!(workspace.identity_cwd, PathBuf::from("/tmp/pion"));
        assert_eq!(tab.panes[&root.raw()].cwd, PathBuf::from("/tmp/pion"));
        assert_eq!(tab.panes[&second.raw()].cwd, PathBuf::from("/tmp/herdr"));
    }

    #[tokio::test]
    async fn capture_contract_tracks_pane_history_from_runtime() {
        let state = state_with_workspaces(&["one"]);
        let root = state.workspaces[0].tabs[0].root_pane;
        let terminal_id = state.workspaces[0].tabs[0].panes[&root]
            .attached_terminal_id
            .clone();
        let mut terminal_runtimes = TerminalRuntimeRegistry::new();
        terminal_runtimes.insert(
            terminal_id,
            crate::terminal::TerminalRuntime::test_with_scrollback_bytes(
                20,
                3,
                4096,
                b"alpha\r\nbeta\r\ngamma\r\n",
            ),
        );

        let snapshot = capture_from_state_with_runtimes(&state, &terminal_runtimes);
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(!encoded.contains("alpha"));
        assert!(!encoded.contains("\"history\""));

        let history_snapshot = capture_history_from_state_with_runtimes(&state, &terminal_runtimes);
        let history = &history_snapshot.workspaces[0].tabs[0].panes[&root.raw()];

        assert!(history.ansi.contains("alpha"));
        assert!(history.ansi.contains("gamma"));
        assert!(history.lines >= 3);
    }

    #[tokio::test]
    async fn capture_contract_tracks_history_for_each_pane() {
        let mut state = state_with_workspaces(&["one"]);
        let first = state.workspaces[0].tabs[0].root_pane;
        let second = state.workspaces[0].test_split(Direction::Horizontal);
        let first_terminal_id = state.workspaces[0].tabs[0].panes[&first]
            .attached_terminal_id
            .clone();
        let second_terminal_id = state.workspaces[0].tabs[0].panes[&second]
            .attached_terminal_id
            .clone();
        let mut terminal_runtimes = TerminalRuntimeRegistry::new();
        terminal_runtimes.insert(
            first_terminal_id,
            crate::terminal::TerminalRuntime::test_with_scrollback_bytes(
                20,
                3,
                4096,
                b"first-pane-history\r\n",
            ),
        );
        terminal_runtimes.insert(
            second_terminal_id,
            crate::terminal::TerminalRuntime::test_with_scrollback_bytes(
                20,
                3,
                4096,
                b"second-pane-history\r\n",
            ),
        );

        let snapshot = capture_from_state_with_runtimes(&state, &terminal_runtimes);
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(!encoded.contains("first-pane-history"));
        assert!(!encoded.contains("second-pane-history"));

        let history_snapshot = capture_history_from_state_with_runtimes(&state, &terminal_runtimes);
        let tab = &history_snapshot.workspaces[0].tabs[0];
        let first_history = &tab.panes[&first.raw()];
        let second_history = &tab.panes[&second.raw()];

        assert!(first_history.ansi.contains("first-pane-history"));
        assert!(second_history.ansi.contains("second-pane-history"));
    }

    #[test]
    fn capture_contract_tracks_hook_authority_agent_session() {
        let mut state = state_with_workspaces(&["one"]);
        let session_path = test_session_path("pi-session.jsonl");
        let root = state.workspaces[0].tabs[0].root_pane;
        state.ensure_test_terminals();
        let terminal_id = state.workspaces[0].tabs[0].panes[&root]
            .attached_terminal_id
            .clone();
        state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .set_hook_authority_with_session_ref(
                "herdr:pi".into(),
                "pi".into(),
                crate::detect::AgentState::Working,
                None,
                None,
                crate::agent_resume::AgentSessionRef::path(session_path.clone()),
                Some(20),
            );

        let snapshot = capture_from_state(&state);
        let agent_session = snapshot.workspaces[0].tabs[0].panes[&root.raw()]
            .agent_session
            .as_ref()
            .expect("agent session should be captured");

        assert_eq!(agent_session.source, "herdr:pi");
        assert_eq!(agent_session.agent, "pi");
        assert_eq!(
            agent_session.kind,
            crate::agent_resume::AgentSessionRefKind::Path
        );
        assert_eq!(agent_session.value, session_path);
    }

    #[test]
    fn capture_contract_preserves_restored_agent_session() {
        let mut state = state_with_workspaces(&["one"]);
        let root = state.workspaces[0].tabs[0].root_pane;
        state.ensure_test_terminals();
        let terminal_id = state.workspaces[0].tabs[0].panes[&root]
            .attached_terminal_id
            .clone();
        state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
                source: "herdr:opencode".into(),
                agent: "opencode".into(),
                session_ref: crate::agent_resume::AgentSessionRef::id("opencode-session").unwrap(),
            });

        let snapshot = capture_from_state(&state);
        let agent_session = snapshot.workspaces[0].tabs[0].panes[&root.raw()]
            .agent_session
            .as_ref()
            .expect("persisted agent session should be captured");

        assert_eq!(agent_session.source, "herdr:opencode");
        assert_eq!(agent_session.agent, "opencode");
        assert_eq!(
            agent_session.kind,
            crate::agent_resume::AgentSessionRefKind::Id
        );
        assert_eq!(agent_session.value, "opencode-session");
    }

    #[test]
    fn old_unversioned_snapshot_loads_as_version_0() {
        let json = r#"{"workspaces":[],"active":null,"selected":0}"#;
        let snap = parse_snapshot(json).unwrap();
        assert_eq!(snap.version, 0);
    }

    #[test]
    fn future_version_is_rejected() {
        let json = r#"{"version":999,"workspaces":[],"active":null,"selected":0}"#;
        assert!(parse_snapshot(json).is_err());
    }

    #[test]
    fn active_tab_default_is_zero() {
        let json = r#"{"custom_name":"test","identity_cwd":"/tmp","tabs":[]}"#;
        let ws: WorkspaceSnapshot = serde_json::from_str(json).unwrap();
        assert_eq!(ws.active_tab, 0);
    }

    #[test]
    fn home_tab_default_is_zero() {
        // Snapshots written before home_tab existed must still load.
        let json = r#"{"custom_name":"test","identity_cwd":"/tmp","tabs":[],"active_tab":2}"#;
        let ws: WorkspaceSnapshot = serde_json::from_str(json).unwrap();
        assert_eq!(ws.home_tab, 0);
    }

    #[test]
    fn restore_falls_back_to_home_when_cwd_missing() {
        let mut panes = HashMap::new();
        panes.insert(
            0,
            PaneSnapshot {
                cwd: PathBuf::from("/tmp/this-directory-does-not-exist-for-herdr-test"),
                label: None,
                agent_name: None,
                agent_session: None,
                launch_argv: None,
                parent: None,
            },
        );
        panes.insert(
            1,
            PaneSnapshot {
                cwd: std::env::var("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from("/tmp")),
                label: None,
                agent_name: None,
                agent_session: None,
                launch_argv: None,
                parent: None,
            },
        );

        let snap = SessionSnapshot {
            version: SNAPSHOT_VERSION,
            workspaces: vec![WorkspaceSnapshot {
                id: Some("test-ws".to_string()),
                custom_name: Some("fallback test".to_string()),
                identity_cwd: PathBuf::from("/tmp"),
                worktree_space: None,
                public_pane_numbers: HashMap::new(),
                next_public_pane_number: 0,
                public_tab_numbers: Vec::new(),
                next_public_tab_number: 0,
                tabs: vec![TabSnapshot {
                    custom_name: None,
                    layout: LayoutSnapshot::Split {
                        direction: DirectionSnapshot::Horizontal,
                        ratio: 0.5,
                        first: Box::new(LayoutSnapshot::Pane(0)),
                        second: Box::new(LayoutSnapshot::Pane(1)),
                    },
                    panes,
                    zoomed: false,
                    focused: Some(0),
                    root_pane: Some(0),
                }],
                active_tab: 0,
                home_tab: 0,
            }],
            active: Some(0),
            selected: 0,
            sidebar_width: Some(26),
            sidebar_section_split: Some(0.5),
            collapsed_space_keys: std::collections::HashSet::new(),
            collapsed_agent_keys: std::collections::HashSet::new(),
            collapsed_line_split_keys: std::collections::HashSet::new(),
            agent_manual_order: None,
            sidebar_pane_section_split: None,
            pane_section_order: None,
        };

        let json = serde_json::to_string(&snap).unwrap();
        let restored = parse_snapshot(&json).unwrap();
        assert_eq!(restored.workspaces.len(), 1);
        assert_eq!(
            restored.workspaces[0].tabs[0].panes[&0].cwd,
            PathBuf::from("/tmp/this-directory-does-not-exist-for-herdr-test")
        );
    }
}
