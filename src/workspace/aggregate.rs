use std::collections::HashMap;

use crate::detect::{Agent, AgentState};
use crate::layout::PaneId;
use crate::terminal::{TerminalId, TerminalState};

use super::{Tab, Workspace};

/// Detail info for a single pane, used by the agent detail panel.
pub struct PaneDetail {
    pub pane_id: PaneId,
    pub tab_idx: usize,
    pub tab_label: String,
    pub label: String,
    pub pane_label: Option<String>,
    pub terminal_title: Option<String>,
    pub terminal_title_stripped: Option<String>,
    pub agent_label: String,
    pub agent_kind_label: Option<String>,
    pub agent: Option<Agent>,
    pub state: AgentState,
    pub seen: bool,
    pub last_agent_state_change_seq: Option<u64>,
    pub state_labels: HashMap<String, String>,
    pub tokens: HashMap<String, String>,
}

impl Tab {
    /// True when this tab is an agent tab: it has at least one pane and every
    /// pane is attached to an agent terminal. A tab that mixes agent and
    /// non-agent panes is not an agent tab. The agent's current state (idle,
    /// working, or blocked) does not matter.
    pub fn is_agent_tab(&self, terminals: &HashMap<TerminalId, TerminalState>) -> bool {
        let mut saw_agent = false;
        for pane in self.panes.values() {
            match terminals.get(&pane.attached_terminal_id) {
                Some(terminal) if terminal.is_agent_terminal() => saw_agent = true,
                _ => return false,
            }
        }
        saw_agent
    }

    fn pane_details(
        &self,
        terminals: &HashMap<TerminalId, TerminalState>,
        tab_idx: usize,
        tab_label: &str,
    ) -> Vec<PaneDetail> {
        self.layout
            .pane_ids()
            .iter()
            .filter_map(|id| {
                let pane = self.panes.get(id)?;
                let terminal = terminals.get(&pane.attached_terminal_id)?;
                let agent_kind_label = terminal.effective_agent_label().map(str::to_string);
                let fallback_agent_label = terminal
                    .agent_name
                    .as_deref()
                    .or(agent_kind_label.as_deref())?
                    .to_string();
                let agent_label = terminal
                    .effective_display_agent()
                    .unwrap_or_else(|| fallback_agent_label.clone());
                let presentation = terminal.effective_presentation();
                Some(PaneDetail {
                    pane_id: *id,
                    tab_idx,
                    tab_label: tab_label.to_string(),
                    label: agent_label.clone(),
                    pane_label: terminal
                        .effective_title()
                        .or_else(|| terminal.manual_label.clone()),
                    terminal_title: terminal.terminal_title.clone(),
                    terminal_title_stripped: terminal.terminal_title_stripped(),
                    agent_label,
                    agent_kind_label,
                    agent: terminal.effective_known_agent(),
                    state: terminal.state,
                    seen: pane.seen,
                    last_agent_state_change_seq: terminal.last_agent_state_change_seq,
                    state_labels: presentation.state_labels,
                    tokens: terminal.metadata_tokens.values(),
                })
            })
            .collect()
    }
}

fn pane_attention_priority(state: AgentState, seen: bool) -> u8 {
    match (state, seen) {
        (AgentState::Blocked, _) => 4,
        (AgentState::Idle, false) => 3,
        (AgentState::Working, _) => 2,
        (AgentState::Idle, true) => 1,
        (AgentState::Unknown, _) => 0,
    }
}

impl Workspace {
    pub fn aggregate_state(
        &self,
        terminals: &HashMap<TerminalId, TerminalState>,
    ) -> (AgentState, bool) {
        self.tabs
            .iter()
            .flat_map(|tab| tab.panes.values())
            .filter_map(|pane| {
                terminals
                    .get(&pane.attached_terminal_id)
                    .map(|terminal| (terminal.state, pane.seen))
            })
            .max_by_key(|(state, seen)| pane_attention_priority(*state, *seen))
            .unwrap_or((AgentState::Unknown, true))
    }

    pub fn pane_details(&self, terminals: &HashMap<TerminalId, TerminalState>) -> Vec<PaneDetail> {
        let multi_tab = self.tabs.len() > 1;
        self.tabs
            .iter()
            .enumerate()
            .flat_map(|(tab_idx, tab)| {
                let tab_label = self
                    .tab_display_name(tab_idx)
                    .unwrap_or_else(|| (tab_idx + 1).to_string());
                tab.pane_details(terminals, tab_idx, &tab_label).into_iter()
            })
            .map(|mut detail| {
                if multi_tab {
                    detail.label = format!("{}·{}", detail.tab_label, detail.agent_label);
                }
                detail
            })
            .collect()
    }

    /// True when this space has at least one tab and every tab is an agent tab.
    /// Such spaces are hidden from the spaces list, the collapsed rail, and
    /// space navigation under `[experimental] hide_tabs_with_agents`.
    pub fn is_agent_only(&self, terminals: &HashMap<TerminalId, TerminalState>) -> bool {
        !self.tabs.is_empty() && self.tabs.iter().all(|tab| tab.is_agent_tab(terminals))
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Direction;

    use super::*;
    use crate::detect::Agent;

    fn terminal_for_pane(ws: &Workspace, pane_id: PaneId) -> TerminalState {
        TerminalState::new(ws.terminal_id(pane_id).unwrap().clone(), "/tmp".into())
    }

    #[test]
    fn aggregate_state_all_unknown() {
        let ws = Workspace::test_new("test");
        let mut terminals = HashMap::new();
        let root = ws.tabs[0].root_pane;
        let terminal = terminal_for_pane(&ws, root);
        terminals.insert(terminal.id.clone(), terminal);
        let (state, seen) = ws.aggregate_state(&terminals);
        assert_eq!(state, AgentState::Unknown);
        assert!(seen);
    }

    #[test]
    fn aggregate_state_priority() {
        let mut ws = Workspace::test_new("test");
        let id2 = ws.test_split(Direction::Horizontal);
        let root_id = ws.tabs[0]
            .panes
            .keys()
            .find(|id| **id != id2)
            .copied()
            .unwrap();
        let mut terminals = HashMap::new();
        let mut root_terminal = terminal_for_pane(&ws, root_id);
        root_terminal.state = AgentState::Idle;
        terminals.insert(root_terminal.id.clone(), root_terminal);
        let mut second_terminal = terminal_for_pane(&ws, id2);
        second_terminal.state = AgentState::Working;
        terminals.insert(second_terminal.id.clone(), second_terminal);

        let (state, seen) = ws.aggregate_state(&terminals);

        assert_eq!(state, AgentState::Working);
        assert!(seen);
    }

    #[test]
    fn aggregate_state_done_unseen_beats_working() {
        let mut ws = Workspace::test_new("test");
        let id2 = ws.test_split(Direction::Horizontal);
        let root_id = ws.tabs[0]
            .panes
            .keys()
            .find(|id| **id != id2)
            .copied()
            .unwrap();
        let mut terminals = HashMap::new();
        let mut root_terminal = terminal_for_pane(&ws, root_id);
        root_terminal.state = AgentState::Idle;
        terminals.insert(root_terminal.id.clone(), root_terminal);
        let mut second_terminal = terminal_for_pane(&ws, id2);
        second_terminal.state = AgentState::Working;
        terminals.insert(second_terminal.id.clone(), second_terminal);
        let root = ws.tabs[0].panes.get_mut(&root_id).unwrap();
        root.seen = false;

        let (state, seen) = ws.aggregate_state(&terminals);

        assert_eq!(state, AgentState::Idle);
        assert!(!seen);
    }

    #[test]
    fn pane_details_prefers_agent_name_over_detected_agent_label() {
        let ws = Workspace::test_new("test");
        let root_pane = ws.tabs[0].root_pane;
        let mut terminals = HashMap::new();
        let mut terminal = terminal_for_pane(&ws, root_pane);
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Working);
        terminal.set_agent_name("planner".into());
        terminals.insert(terminal.id.clone(), terminal);

        let labels: Vec<_> = ws
            .pane_details(&terminals)
            .into_iter()
            .map(|detail| (detail.label, detail.agent_label, detail.agent))
            .collect();

        assert_eq!(
            labels,
            vec![("planner".into(), "planner".into(), Some(Agent::Pi))]
        );
    }

    #[test]
    fn pane_details_includes_tab_context_for_multi_tab_workspace() {
        let mut ws = Workspace::test_new("test");
        ws.tabs[0].custom_name = Some("main".into());
        let root_pane = ws.tabs[0].root_pane;
        let second_tab = ws.test_add_tab(Some("review"));
        let review_pane = ws.tabs[second_tab].root_pane;
        let mut terminals = HashMap::new();
        let mut root_terminal = terminal_for_pane(&ws, root_pane);
        root_terminal.set_hook_authority(
            "test".into(),
            "pi".into(),
            AgentState::Working,
            None,
            None,
        );
        terminals.insert(root_terminal.id.clone(), root_terminal);
        let mut review_terminal = terminal_for_pane(&ws, review_pane);
        review_terminal.set_hook_authority(
            "test".into(),
            "claude".into(),
            AgentState::Idle,
            None,
            None,
        );
        terminals.insert(review_terminal.id.clone(), review_terminal);

        let labels: Vec<_> = ws
            .pane_details(&terminals)
            .into_iter()
            .map(|detail| (detail.label, detail.agent_label, detail.agent))
            .collect();

        assert_eq!(
            labels,
            vec![
                ("main·pi".into(), "pi".into(), Some(Agent::Pi)),
                ("review·claude".into(), "claude".into(), Some(Agent::Claude)),
            ]
        );
    }

    #[test]
    fn pane_details_use_tab_vector_index_not_stable_public_tab_number() {
        let mut ws = Workspace::test_new("test");
        let removed_tab = ws.test_add_tab(Some("removed"));
        let survivor_tab = ws.test_add_tab(Some("survivor"));
        let survivor_pane = ws.tabs[survivor_tab].root_pane;
        assert!(ws.close_tab(removed_tab));

        let mut terminals = HashMap::new();
        let mut terminal = terminal_for_pane(&ws, survivor_pane);
        terminal.detected_agent = Some(Agent::Codex);
        terminals.insert(terminal.id.clone(), terminal);

        let details = ws.pane_details(&terminals);
        let survivor = details
            .iter()
            .find(|detail| detail.pane_id == survivor_pane)
            .expect("surviving tab agent should be listed");

        assert_eq!(ws.tabs[1].number, 3);
        assert_eq!(survivor.tab_idx, 1);
    }

    /// A plain (non-agent) terminal for every pane in every tab of `ws`.
    fn terminals_for_all_panes(ws: &Workspace) -> HashMap<TerminalId, TerminalState> {
        let mut terminals = HashMap::new();
        for tab in &ws.tabs {
            for pane_id in tab.panes.keys() {
                let terminal = terminal_for_pane(ws, *pane_id);
                terminals.insert(terminal.id.clone(), terminal);
            }
        }
        terminals
    }

    fn mark_pane_agent(
        ws: &Workspace,
        terminals: &mut HashMap<TerminalId, TerminalState>,
        pane_id: PaneId,
    ) {
        let id = ws
            .terminal_id(pane_id)
            .expect("pane has a terminal")
            .clone();
        terminals
            .get_mut(&id)
            .expect("terminal registered")
            .set_detected_state(Some(Agent::Pi), AgentState::Idle);
    }

    #[test]
    fn is_agent_tab_requires_every_pane_to_be_an_agent() {
        let mut ws = Workspace::test_new("test");
        let second = ws.test_split(Direction::Horizontal);
        let root = ws.tabs[0].root_pane;
        let mut terminals = terminals_for_all_panes(&ws);

        // No agents at all.
        assert!(!ws.tabs[0].is_agent_tab(&terminals));

        // One agent pane, one plain shell pane: mixed, so not an agent tab.
        mark_pane_agent(&ws, &mut terminals, root);
        assert!(!ws.tabs[0].is_agent_tab(&terminals));

        // Both panes are agents.
        mark_pane_agent(&ws, &mut terminals, second);
        assert!(ws.tabs[0].is_agent_tab(&terminals));
    }

    #[test]
    fn is_agent_only_requires_every_tab_to_be_an_agent_tab() {
        let mut ws = Workspace::test_new("test");
        let plain_tab = ws.test_add_tab(Some("logs"));
        let agent_root = ws.tabs[0].root_pane;
        let plain_root = ws.tabs[plain_tab].root_pane;
        let mut terminals = terminals_for_all_panes(&ws);

        mark_pane_agent(&ws, &mut terminals, agent_root);
        // One agent tab plus one plain tab: the space is not agent-only.
        assert!(!ws.is_agent_only(&terminals));

        mark_pane_agent(&ws, &mut terminals, plain_root);
        assert!(ws.is_agent_only(&terminals));
    }

    #[test]
    fn is_agent_only_false_for_space_without_tabs() {
        let mut ws = Workspace::test_new("test");
        ws.tabs.clear();
        assert!(!ws.is_agent_only(&HashMap::new()));
    }
}
