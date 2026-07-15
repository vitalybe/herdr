use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::scrollbar::{render_scrollbar, should_show_scrollbar};
use super::status::{agent_icon, state_dot, state_label, state_label_color};
use super::text::{display_width, display_width_u16, truncate_end};
use crate::app::state::{AgentPanelSort, LineSplitId, ManualEntry, Palette};
use crate::app::{AppState, Mode};
use crate::detect::AgentState;
use crate::terminal::TerminalRuntimeRegistry;

const WORKSPACE_SECTION_HEADER_ROWS: u16 = 2;
const AGENT_PANEL_HEADER_ROWS: u16 = 3;
/// Header rows above the Panes-section body: a divider rule plus the title.
const PANE_SECTION_HEADER_ROWS: u16 = 2;
/// Content height of a single Panes-section row: pane name line + space name line.
const PANE_SECTION_ROW_HEIGHT: u16 = 2;

/// Fixed dark grey used for the space (workspace) name on an agent row's second
/// line. Resolved through `parse_color_checked` so it matches the `darkgrey`
/// config color exactly, independent of theme or active state.
fn agent_row_space_color() -> Color {
    crate::config::parse_color_checked("darkgrey").unwrap_or(Color::DarkGray)
}

/// Runtime values and state-derived styles for one agent-panel entry. The
/// layout is fixed at two lines:
///
/// - Line 1: `" {icon} {tab}"` - a leading space, the status icon (keeping its
///   intrinsic state color), then the tab name. The tab and its separating
///   space are dropped together when the pane has no tab label.
/// - Line 2: `" {space} · {status}"` - a leading space, the space (workspace)
///   name in fixed dark grey, then ` · ` and the status text. The ` · `
///   separator and status are dropped together when the status is empty.
pub(crate) struct AgentRowContext<'a> {
    pub icon: &'a str,
    pub icon_style: Style,
    pub space: &'a str,
    pub tab: Option<&'a str>,
    pub status: &'a str,
    pub status_color: Color,
    pub is_active: bool,
}

impl AgentRowContext<'_> {
    /// Bold "name" style for the tab label: brighter foreground when the row is
    /// the active pane, dimmer otherwise.
    fn name_style(&self, p: &Palette) -> Style {
        let fg = if self.is_active { p.text } else { p.subtext0 };
        Style::default().fg(fg).add_modifier(Modifier::BOLD)
    }

    /// Status style: the live state color, dimmed when the row is inactive.
    fn status_style(&self) -> Style {
        let style = Style::default().fg(self.status_color);
        if self.is_active {
            style
        } else {
            style.add_modifier(Modifier::DIM)
        }
    }

    /// First line: leading space, status icon, then the tab name.
    fn line_one(&self, p: &Palette, max_width: usize) -> Vec<Span<'static>> {
        let mut spans = vec![
            Span::styled(" ".to_string(), Style::default()),
            Span::styled(self.icon.to_string(), self.icon_style),
        ];
        if let Some(tab) = self.tab.filter(|tab| !tab.is_empty()) {
            spans.push(Span::styled(format!(" {tab}"), self.name_style(p)));
        }
        truncate_agent_row_spans(spans, max_width)
    }

    /// Second line: leading space, dark-grey space name, then ` · status`.
    fn line_two(&self, max_width: usize) -> Vec<Span<'static>> {
        let mut spans = vec![
            Span::styled(" ".to_string(), Style::default()),
            Span::styled(
                self.space.to_string(),
                Style::default().fg(agent_row_space_color()),
            ),
        ];
        if !self.status.is_empty() {
            spans.push(Span::styled(
                format!(" · {}", self.status),
                self.status_style(),
            ));
        }
        truncate_agent_row_spans(spans, max_width)
    }
}

/// Trim a span list so its combined display width does not exceed `max_width`,
/// eliding the boundary span with an ellipsis when it overflows.
fn truncate_agent_row_spans(spans: Vec<Span<'static>>, max_width: usize) -> Vec<Span<'static>> {
    let mut out = Vec::with_capacity(spans.len());
    let mut used = 0usize;
    for span in spans {
        let width = display_width(&span.content);
        if used + width <= max_width {
            used += width;
            out.push(span);
            continue;
        }
        let remaining = max_width.saturating_sub(used);
        if remaining > 0 {
            let truncated = truncate_end(&span.content, remaining);
            out.push(Span::styled(truncated, span.style));
        }
        break;
    }
    out
}

pub(crate) struct AgentPanelEntry {
    pub ws_idx: usize,
    pub tab_idx: usize,
    pub pane_id: crate::layout::PaneId,
    pub primary_label: String,
    pub primary_tab_label: Option<String>,
    pub agent_label: Option<String>,
    pub state: AgentState,
    pub seen: bool,
    pub last_agent_state_change_seq: Option<u64>,
    pub custom_status: Option<String>,
    pub state_labels: std::collections::HashMap<String, String>,
}

/// A single visible row in the agents panel: either an agent entry or a named
/// line-split divider. Line-splits only appear in manual sort mode. Client-only
/// presentation state.
pub(crate) enum AgentPanelRow {
    Agent(AgentPanelEntry),
    LineSplit { id: LineSplitId, name: String },
}

impl AgentPanelRow {
    /// Content-row height of this row (excluding the trailing gap). Agent rows
    /// render the two configured template lines; line-splits are a single rule.
    fn content_height(&self) -> u16 {
        match self {
            AgentPanelRow::Agent(_) => 2,
            AgentPanelRow::LineSplit { .. } => 1,
        }
    }
}

/// Screen placement for one visible agent-panel row.
pub(crate) struct AgentPanelRowArea {
    pub row_idx: usize,
    pub y: u16,
    pub height: u16,
}

fn sidebar_section_heights(total_h: u16, split_ratio: f32) -> (u16, u16) {
    if total_h == 0 {
        return (0, 0);
    }

    if total_h < 6 {
        let ws_h = total_h.div_ceil(2);
        return (ws_h, total_h.saturating_sub(ws_h));
    }

    let ratio = split_ratio.clamp(0.1, 0.9);
    let ws_h = ((total_h as f32) * ratio).round() as u16;
    let ws_h = ws_h.clamp(3, total_h.saturating_sub(3));
    let detail_h = total_h.saturating_sub(ws_h);
    (ws_h, detail_h)
}

pub(crate) fn expanded_sidebar_sections(area: Rect, split_ratio: f32) -> (Rect, Rect) {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.width == 0 || content.height == 0 {
        return (Rect::default(), Rect::default());
    }

    let (ws_h, detail_h) = sidebar_section_heights(content.height, split_ratio);
    let ws_area = Rect::new(content.x, content.y, content.width, ws_h);
    let detail_area = Rect::new(content.x, content.y + ws_h, content.width, detail_h);
    (ws_area, detail_area)
}

pub(crate) fn sidebar_section_divider_rect(area: Rect, split_ratio: f32) -> Rect {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.width == 0 || content.height < 6 {
        return Rect::default();
    }

    let (ws_h, _) = sidebar_section_heights(content.height, split_ratio);
    Rect::new(content.x, content.y + ws_h, content.width, 1)
}

/// Partition the sidebar content height into three stacked bands
/// (Spaces / Tabs / Agents). `spaces_ratio` allocates the Spaces band out of the
/// total height; `pane_section_ratio` then allocates the Panes band out of the remaining
/// height, leaving the rest for Agents. Reuses the two-band split so the Spaces
/// band matches the historical geometry exactly.
///
/// When `show_pane_section` is false (no non-agent panes exist) the Panes band collapses
/// to zero height and the Agents band takes the whole region below Spaces, so
/// agent-only sidebars keep the historical two-band geometry.
pub(crate) fn expanded_sidebar_sections3(
    area: Rect,
    spaces_ratio: f32,
    pane_section_ratio: f32,
    show_pane_section: bool,
) -> (Rect, Rect, Rect) {
    let (spaces_area, rest) = expanded_sidebar_sections(area, spaces_ratio);
    if rest.width == 0 || rest.height == 0 {
        return (spaces_area, Rect::default(), rest);
    }
    if !show_pane_section {
        let pane_section_area = Rect::new(rest.x, rest.y, rest.width, 0);
        return (spaces_area, pane_section_area, rest);
    }
    let (pane_section_h, agents_h) = sidebar_section_heights(rest.height, pane_section_ratio);
    let pane_section_area = Rect::new(rest.x, rest.y, rest.width, pane_section_h);
    let agents_area = Rect::new(rest.x, rest.y + pane_section_h, rest.width, agents_h);
    (spaces_area, pane_section_area, agents_area)
}

/// The draggable divider between the Panes and Agents bands (divider index 1).
/// Empty when the Panes band is collapsed.
pub(crate) fn sidebar_pane_section_divider_rect(
    area: Rect,
    spaces_ratio: f32,
    pane_section_ratio: f32,
    show_pane_section: bool,
) -> Rect {
    if !show_pane_section {
        return Rect::default();
    }
    let (_, rest) = expanded_sidebar_sections(area, spaces_ratio);
    if rest.width == 0 || rest.height < 6 {
        return Rect::default();
    }
    let (pane_section_h, _) = sidebar_section_heights(rest.height, pane_section_ratio);
    Rect::new(rest.x, rest.y + pane_section_h, rest.width, 1)
}

/// The Agents (detail) band as the third of three stacked sidebar sections.
pub(crate) fn agents_detail_rect(
    area: Rect,
    spaces_ratio: f32,
    pane_section_ratio: f32,
    show_pane_section: bool,
) -> Rect {
    let (_, _, agents_area) =
        expanded_sidebar_sections3(area, spaces_ratio, pane_section_ratio, show_pane_section);
    agents_area
}

/// The Panes band as the middle of three stacked sidebar sections.
pub(crate) fn pane_section_rect(
    area: Rect,
    spaces_ratio: f32,
    pane_section_ratio: f32,
    show_pane_section: bool,
) -> Rect {
    let (_, pane_section_area, _) =
        expanded_sidebar_sections3(area, spaces_ratio, pane_section_ratio, show_pane_section);
    pane_section_area
}

/// Whether the sidebar Panes section has any entries to show. When false, the
/// Panes band collapses and the Agents band keeps the historical geometry.
pub(crate) fn sidebar_shows_pane_section(app: &AppState) -> bool {
    !sidebar_pane_section_entries(app).is_empty()
}

/// Body (scrolling content) region of the Panes band, below its header rows.
/// Reserves the rightmost column for the scrollbar when `has_scrollbar`.
pub(crate) fn pane_section_body_rect(area: Rect, has_scrollbar: bool) -> Rect {
    if area.width == 0 || area.height <= PANE_SECTION_HEADER_ROWS {
        return Rect::default();
    }
    let body_y = area.y.saturating_add(PANE_SECTION_HEADER_ROWS);
    let body_height = (area.y + area.height).saturating_sub(body_y);
    let body_width = area.width.saturating_sub(u16::from(has_scrollbar));
    Rect::new(area.x, body_y, body_width, body_height)
}

/// A single non-agent pane surfaced in the Panes section, resolved from the
/// client-only [`crate::app::state::PaneSectionOrder`]. `tab_idx` is the pane's
/// containing tab (used for the display-name fallback) and `pane_id` addresses
/// the pane itself for focus and rename.
pub(crate) struct PaneSectionEntry {
    pub order_idx: usize,
    pub ws_idx: usize,
    pub tab_idx: usize,
    pub pane_id: crate::layout::PaneId,
}

/// All non-agent panes across every workspace, ordered by the client-only Panes
/// section ordering. Entries whose pane no longer resolves are skipped.
pub(crate) fn sidebar_pane_section_entries(app: &AppState) -> Vec<PaneSectionEntry> {
    let mut lookup: std::collections::HashMap<
        (&str, usize),
        (usize, usize, crate::layout::PaneId),
    > = std::collections::HashMap::new();
    for (ws_idx, ws) in app.workspaces.iter().enumerate() {
        for (tab_idx, pane_id, pane_number) in ws.non_agent_panes(&app.terminals) {
            lookup.insert((ws.id.as_str(), pane_number), (ws_idx, tab_idx, pane_id));
        }
    }
    app.pane_section_order
        .order
        .iter()
        .enumerate()
        .filter_map(|(order_idx, pane_ref)| {
            lookup
                .get(&(pane_ref.workspace_id.as_str(), pane_ref.pane_number))
                .map(|&(ws_idx, tab_idx, pane_id)| PaneSectionEntry {
                    order_idx,
                    ws_idx,
                    tab_idx,
                    pane_id,
                })
        })
        .collect()
}

/// Visible-row layout for the Panes section, walking entries from `scroll` and
/// laying out two-line rows (with a one-row gap) inside `body`. Pure; does not
/// consult scroll metrics, so it is safe to call from the metrics path.
fn pane_section_row_areas_in(
    app: &AppState,
    body: Rect,
    scroll: usize,
) -> Vec<crate::app::state::PaneSectionRowArea> {
    let mut areas = Vec::new();
    if body.width == 0 || body.height == 0 {
        return areas;
    }
    let body_bottom = body.y + body.height;
    let mut row_y = body.y;
    for entry in sidebar_pane_section_entries(app).into_iter().skip(scroll) {
        if row_y.saturating_add(PANE_SECTION_ROW_HEIGHT) > body_bottom {
            break;
        }
        areas.push(crate::app::state::PaneSectionRowArea {
            ws_idx: entry.ws_idx,
            tab_idx: entry.tab_idx,
            pane_id: entry.pane_id,
            order_idx: entry.order_idx,
            rect: Rect::new(body.x, row_y, body.width, PANE_SECTION_ROW_HEIGHT),
        });
        row_y = row_y.saturating_add(PANE_SECTION_ROW_HEIGHT);
        if row_y < body_bottom {
            row_y = row_y.saturating_add(1);
        }
    }
    areas
}

fn pane_section_visible_count(app: &AppState, area: Rect, scroll: usize) -> usize {
    let body = pane_section_body_rect(area, false);
    pane_section_row_areas_in(app, body, scroll).len()
}

pub(crate) fn pane_section_scroll_metrics(
    app: &AppState,
    area: Rect,
) -> crate::pane::ScrollMetrics {
    let total_rows = sidebar_pane_section_entries(app).len();
    let scroll = app.pane_section_scroll.min(total_rows.saturating_sub(1));
    let viewport_rows = pane_section_visible_count(app, area, scroll);
    let max_offset_from_bottom = total_rows.saturating_sub(viewport_rows);
    let offset_from_bottom = total_rows
        .saturating_sub(app.pane_section_scroll)
        .saturating_sub(viewport_rows);

    crate::pane::ScrollMetrics {
        offset_from_bottom,
        max_offset_from_bottom,
        viewport_rows,
    }
}

pub(crate) fn pane_section_scrollbar_rect(app: &AppState, area: Rect) -> Option<Rect> {
    let metrics = pane_section_scroll_metrics(app, area);
    let body = pane_section_body_rect(area, true);
    (should_show_scrollbar(metrics) && body.width > 0 && body.height > 0).then_some(Rect::new(
        area.x + area.width.saturating_sub(1),
        body.y,
        1,
        body.height,
    ))
}

/// Screen placement of the visible Panes-section rows, honoring the current scroll
/// offset and reserving space for the scrollbar when one is shown.
pub(crate) fn compute_pane_section_row_areas(
    app: &AppState,
    area: Rect,
) -> Vec<crate::app::state::PaneSectionRowArea> {
    let metrics = pane_section_scroll_metrics(app, area);
    let body = pane_section_body_rect(area, should_show_scrollbar(metrics));
    pane_section_row_areas_in(app, body, app.pane_section_scroll)
}

/// Row (y) of the drop indicator for a Panes-section reorder targeting flat
/// `insert_idx`, mirroring the agent-panel drop indicator.
pub(crate) fn pane_section_drop_indicator_row(
    areas: &[crate::app::state::PaneSectionRowArea],
    body: Rect,
    insert_idx: usize,
) -> Option<u16> {
    if body.height == 0 {
        return None;
    }
    let body_bottom = body.y + body.height;
    if let Some(area) = areas.iter().find(|area| area.order_idx == insert_idx) {
        let y = if area.rect.y == body.y {
            body.y
        } else {
            area.rect.y.saturating_sub(1)
        };
        return (y < body_bottom).then_some(y);
    }
    if let Some(last) = areas.last() {
        if insert_idx >= last.order_idx.saturating_add(1) {
            let y = last.rect.y.saturating_add(last.rect.height);
            return (y < body_bottom).then_some(y);
        }
    }
    None
}

fn agent_panel_sort_label(sort: AgentPanelSort) -> &'static str {
    match sort {
        AgentPanelSort::Spaces => "grouped",
        AgentPanelSort::Priority => "priority",
        AgentPanelSort::Manual => "manual",
    }
}

pub(crate) fn agent_panel_toggle_rect(area: Rect, sort: AgentPanelSort) -> Rect {
    if area.width == 0 || area.height < 2 {
        return Rect::default();
    }

    let label = agent_panel_sort_label(sort);
    let width = display_width_u16(label);
    Rect::new(
        area.x + area.width.saturating_sub(width),
        area.y + 1,
        width,
        1,
    )
}

const AGENT_PANEL_SPLIT_LABEL: &str = "+ split";

/// Mouse-first "+ split" affordance rect, placed just left of the sort toggle in
/// the agents header. Only meaningful in manual mode; returns the empty rect
/// otherwise or when there is no room.
pub(crate) fn agent_panel_split_button_rect(area: Rect, sort: AgentPanelSort) -> Rect {
    if !matches!(sort, AgentPanelSort::Manual) || area.width == 0 || area.height < 2 {
        return Rect::default();
    }
    let toggle = agent_panel_toggle_rect(area, sort);
    if toggle == Rect::default() {
        return Rect::default();
    }
    let width = display_width_u16(AGENT_PANEL_SPLIT_LABEL);
    // One-column gap between the affordance and the toggle.
    let right = toggle.x.saturating_sub(1);
    if right <= area.x || width == 0 {
        return Rect::default();
    }
    let x = right.saturating_sub(width);
    if x < area.x {
        return Rect::default();
    }
    Rect::new(x, area.y + 1, width, 1)
}

pub(crate) fn agent_panel_entries(app: &AppState) -> Vec<AgentPanelEntry> {
    agent_panel_entries_with_runtimes(app, None)
}

pub(crate) fn agent_panel_entries_from(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> Vec<AgentPanelEntry> {
    agent_panel_entries_with_runtimes(app, Some(terminal_runtimes))
}

fn agent_panel_entries_with_runtimes(
    app: &AppState,
    terminal_runtimes: Option<&TerminalRuntimeRegistry>,
) -> Vec<AgentPanelEntry> {
    let empty_runtimes;
    let terminal_runtimes = match terminal_runtimes {
        Some(terminal_runtimes) => terminal_runtimes,
        None => {
            empty_runtimes = TerminalRuntimeRegistry::new();
            &empty_runtimes
        }
    };

    let mut entries: Vec<_> = app
        .workspaces
        .iter()
        .enumerate()
        .flat_map(|(ws_idx, ws)| {
            let workspace_label = ws.display_name_from(&app.terminals, terminal_runtimes);
            ws.pane_details(&app.terminals)
                .into_iter()
                .map(move |detail| AgentPanelEntry {
                    ws_idx,
                    tab_idx: detail.tab_idx,
                    pane_id: detail.pane_id,
                    primary_label: workspace_label.clone(),
                    primary_tab_label: Some(detail.tab_label),
                    agent_label: Some(detail.agent_label),
                    state: detail.state,
                    seen: detail.seen,
                    last_agent_state_change_seq: detail.last_agent_state_change_seq,
                    custom_status: detail.custom_status,
                    state_labels: detail.state_labels,
                })
        })
        .collect();

    match app.agent_panel_sort {
        AgentPanelSort::Spaces => {}
        AgentPanelSort::Priority => {
            entries.sort_by_key(|entry| {
                (
                    std::cmp::Reverse(workspace_attention_priority(entry.state, entry.seen)),
                    std::cmp::Reverse(entry.last_agent_state_change_seq),
                )
            });
        }
        AgentPanelSort::Manual => {
            // Reorder the flat list to follow the manual order. Entries present
            // in `order` sort by their position; any not-yet-reconciled entries
            // keep their natural relative order and land at the end (defensive
            // fallback - reconcile normally keeps `order` in sync before render).
            let order_pos: std::collections::HashMap<crate::layout::PaneId, usize> = app
                .agent_manual_order
                .order
                .iter()
                .enumerate()
                .filter_map(|(idx, entry)| match entry {
                    ManualEntry::Pane(pane_id) => Some((*pane_id, idx)),
                    ManualEntry::LineSplit { .. } => None,
                })
                .collect();
            entries
                .sort_by_key(|entry| order_pos.get(&entry.pane_id).copied().unwrap_or(usize::MAX));
        }
    }

    entries
}

pub(crate) fn agent_panel_rows(app: &AppState) -> Vec<AgentPanelRow> {
    agent_panel_rows_with_runtimes(app, None)
}

pub(crate) fn agent_panel_rows_from(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> Vec<AgentPanelRow> {
    agent_panel_rows_with_runtimes(app, Some(terminal_runtimes))
}

/// Full ordered list of visible agent-panel rows. In Spaces/Priority the rows
/// are the sorted agent entries. In Manual, agents and line-splits are
/// interleaved by walking `agent_manual_order.order`; agents not yet placed in
/// the order fall back to the end. Pure.
fn agent_panel_rows_with_runtimes(
    app: &AppState,
    terminal_runtimes: Option<&TerminalRuntimeRegistry>,
) -> Vec<AgentPanelRow> {
    let entries = agent_panel_entries_with_runtimes(app, terminal_runtimes);
    if !matches!(app.agent_panel_sort, AgentPanelSort::Manual) {
        return entries.into_iter().map(AgentPanelRow::Agent).collect();
    }

    // Index agents by pane so the flat order can pull them in position.
    let mut by_pane: std::collections::HashMap<crate::layout::PaneId, AgentPanelEntry> = entries
        .into_iter()
        .map(|entry| (entry.pane_id, entry))
        .collect();

    let mut rows = Vec::new();
    for entry in &app.agent_manual_order.order {
        match entry {
            ManualEntry::Pane(pane_id) => {
                if let Some(agent) = by_pane.remove(pane_id) {
                    rows.push(AgentPanelRow::Agent(agent));
                }
            }
            ManualEntry::LineSplit { id, name } => {
                rows.push(AgentPanelRow::LineSplit {
                    id: *id,
                    name: name.clone(),
                });
            }
        }
    }
    // Any agents not present in the manual order (not yet reconciled) land last,
    // in natural order.
    for entry in agent_panel_entries_with_runtimes(app, terminal_runtimes) {
        if let Some(agent) = by_pane.remove(&entry.pane_id) {
            rows.push(AgentPanelRow::Agent(agent));
        }
    }

    rows
}

/// Row index (in the full [`agent_panel_rows`] list) of the agent row for
/// `pane_id`, if present.
pub(crate) fn agent_panel_row_index_of_pane(
    app: &AppState,
    pane_id: crate::layout::PaneId,
) -> Option<usize> {
    agent_panel_rows(app)
        .iter()
        .position(|row| matches!(row, AgentPanelRow::Agent(entry) if entry.pane_id == pane_id))
}

/// Visible-row layout for the agent panel: walks the ordered rows from `scroll`,
/// assigning each a screen `y` and content height, stopping when the next row no
/// longer fits. Mirrors the variable-height workspace-list layout.
pub(crate) fn compute_agent_panel_row_areas(
    rows: &[AgentPanelRow],
    body: Rect,
    scroll: usize,
) -> Vec<AgentPanelRowArea> {
    let mut areas = Vec::new();
    if body.width == 0 || body.height == 0 {
        return areas;
    }
    let body_bottom = body.y + body.height;
    let mut row_y = body.y;
    for (row_idx, row) in rows.iter().enumerate().skip(scroll) {
        let height = row.content_height();
        if row_y.saturating_add(height) > body_bottom {
            break;
        }
        areas.push(AgentPanelRowArea {
            row_idx,
            y: row_y,
            height,
        });
        row_y = row_y.saturating_add(height);
        if row_y < body_bottom {
            row_y = row_y.saturating_add(1);
        }
    }
    areas
}

pub(super) fn agent_panel_status_key(state: AgentState, seen: bool) -> &'static str {
    match (state, seen) {
        (AgentState::Idle, false) => "done",
        (AgentState::Idle, true) => "idle",
        (AgentState::Working, _) => "working",
        (AgentState::Blocked, _) => "blocked",
        (AgentState::Unknown, _) => "unknown",
    }
}

fn workspace_row_height(ws: &crate::workspace::Workspace) -> u16 {
    if ws.branch().is_some() {
        2
    } else {
        1
    }
}

fn workspace_attention_priority(state: AgentState, seen: bool) -> u8 {
    match (state, seen) {
        (AgentState::Blocked, _) => 4,
        (AgentState::Idle, false) => 3,
        (AgentState::Working, _) => 2,
        (AgentState::Idle, true) => 1,
        (AgentState::Unknown, _) => 0,
    }
}

fn space_aggregate_state(app: &AppState, key: &str) -> (AgentState, bool) {
    app.workspaces
        .iter()
        .filter(|ws| ws.worktree_space().is_some_and(|space| space.key == key))
        .map(|ws| ws.aggregate_state(&app.terminals))
        .max_by_key(|(state, seen)| workspace_attention_priority(*state, *seen))
        .unwrap_or((AgentState::Unknown, true))
}

pub(crate) fn workspace_parent_group_state(
    app: &AppState,
    ws_idx: usize,
) -> Option<(String, bool)> {
    let space = app.workspaces.get(ws_idx)?.worktree_space()?;
    if space.is_linked_worktree {
        return None;
    }
    let member_count = app
        .workspaces
        .iter()
        .filter(|ws| {
            ws.worktree_space()
                .is_some_and(|member| member.key == space.key)
        })
        .count();
    (member_count >= 2).then(|| {
        (
            space.key.clone(),
            app.collapsed_space_keys.contains(&space.key),
        )
    })
}

pub(crate) fn grouped_child_display_label(
    label: &str,
    branch: Option<&str>,
    has_custom_name: bool,
) -> String {
    if has_custom_name {
        return label.to_string();
    }
    let Some(branch) = branch else {
        return label.to_string();
    };
    branch
        .strip_prefix("worktree/")
        .unwrap_or(branch)
        .to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceListEntry {
    Workspace { ws_idx: usize, indented: bool },
}

pub(crate) fn next_entry_is_indented_workspace(entries: &[WorkspaceListEntry], idx: usize) -> bool {
    matches!(
        entries.get(idx.saturating_add(1)),
        Some(WorkspaceListEntry::Workspace { indented: true, .. })
    )
}

pub(crate) fn normalized_workspace_scroll(app: &AppState, area: Rect, requested: usize) -> usize {
    let ws_area = workspace_list_rect(area, app.sidebar_section_split);
    let body = workspace_list_body_rect(ws_area, false);
    if body.height == 0 {
        return requested;
    }

    let entry_count = workspace_list_entries(app).len();
    if entry_count == 0 {
        0
    } else {
        requested.min(entry_count.saturating_sub(1))
    }
}

pub(crate) fn workspace_list_entries(app: &AppState) -> Vec<WorkspaceListEntry> {
    workspace_list_entries_inner(app, false)
}

/// Like [`workspace_list_entries`] but always expands worktree groups, ignoring
/// `collapsed_space_keys`. The mobile switcher has no collapse affordance and
/// always shows the full worktree tree.
pub(crate) fn workspace_list_entries_expanded(app: &AppState) -> Vec<WorkspaceListEntry> {
    workspace_list_entries_inner(app, true)
}

fn workspace_list_entries_inner(app: &AppState, force_expanded: bool) -> Vec<WorkspaceListEntry> {
    let mut members_by_key = std::collections::HashMap::<String, Vec<usize>>::new();
    for (ws_idx, ws) in app.workspaces.iter().enumerate() {
        if let Some(space) = ws.worktree_space() {
            members_by_key
                .entry(space.key.clone())
                .or_default()
                .push(ws_idx);
        }
    }
    let grouped_keys = members_by_key
        .iter()
        .filter(|(_, members)| {
            members.len() >= 2
                && members.iter().any(|idx| {
                    app.workspaces
                        .get(*idx)
                        .and_then(|ws| ws.worktree_space())
                        .is_some_and(|space| !space.is_linked_worktree)
                })
        })
        .map(|(key, _)| key.clone())
        .collect::<std::collections::HashSet<_>>();

    let visible_group_idx = if matches!(app.mode, Mode::Navigate) {
        Some(app.selected)
    } else {
        app.active
    };
    let active_group = visible_group_idx.and_then(|idx| {
        app.workspaces
            .get(idx)
            .and_then(|ws| ws.worktree_space())
            .map(|space| space.key.clone())
    });

    let mut emitted_groups = std::collections::HashSet::<String>::new();
    let mut entries = Vec::new();
    for (ws_idx, ws) in app.workspaces.iter().enumerate() {
        let Some(space) = ws
            .worktree_space()
            .filter(|space| grouped_keys.contains(&space.key))
        else {
            entries.push(WorkspaceListEntry::Workspace {
                ws_idx,
                indented: false,
            });
            continue;
        };

        if !emitted_groups.insert(space.key.clone()) {
            continue;
        }

        let Some(members) = members_by_key.get(&space.key) else {
            continue;
        };
        let Some(parent_idx) = members.iter().copied().find(|idx| {
            app.workspaces
                .get(*idx)
                .and_then(|member| member.worktree_space())
                .is_some_and(|member_space| !member_space.is_linked_worktree)
        }) else {
            entries.push(WorkspaceListEntry::Workspace {
                ws_idx,
                indented: false,
            });
            continue;
        };
        let collapsed = !force_expanded && app.collapsed_space_keys.contains(&space.key);
        entries.push(WorkspaceListEntry::Workspace {
            ws_idx: parent_idx,
            indented: false,
        });

        if collapsed {
            if let Some(active_idx) = visible_group_idx
                .filter(|idx| *idx != parent_idx)
                .filter(|_| active_group.as_deref() == Some(space.key.as_str()))
            {
                entries.push(WorkspaceListEntry::Workspace {
                    ws_idx: active_idx,
                    indented: true,
                });
            }
        } else {
            for member_idx in members {
                if *member_idx == parent_idx {
                    continue;
                }
                entries.push(WorkspaceListEntry::Workspace {
                    ws_idx: *member_idx,
                    indented: true,
                });
            }
        }
    }

    entries
}

pub(crate) fn workspace_list_rect(area: Rect, split_ratio: f32) -> Rect {
    let (ws_area, _) = expanded_sidebar_sections(area, split_ratio);
    ws_area
}

pub(crate) fn workspace_list_body_rect(area: Rect, has_scrollbar: bool) -> Rect {
    if area.width == 0 || area.height <= WORKSPACE_SECTION_HEADER_ROWS {
        return Rect::default();
    }

    let body_y = area.y.saturating_add(WORKSPACE_SECTION_HEADER_ROWS);
    let footer_y = area.y + area.height.saturating_sub(1);
    let body_height = footer_y.saturating_sub(body_y);
    let body_width = area.width.saturating_sub(u16::from(has_scrollbar));
    Rect::new(area.x, body_y, body_width, body_height)
}

fn workspace_list_visible_count(app: &AppState, area: Rect, scroll: usize) -> usize {
    let body = workspace_list_body_rect(area, false);
    if body.width == 0 || body.height == 0 {
        return 0;
    }

    let mut used_rows = 0u16;
    let mut visible = 0usize;
    let entries = workspace_list_entries(app);
    for (entry_idx, entry) in entries.iter().enumerate().skip(scroll) {
        let needed = match entry {
            WorkspaceListEntry::Workspace { ws_idx, indented } => {
                let Some(ws) = app.workspaces.get(*ws_idx) else {
                    continue;
                };
                let row_height = if *indented {
                    1
                } else {
                    workspace_row_height(ws)
                };
                let gap = u16::from(
                    !(*indented && next_entry_is_indented_workspace(&entries, entry_idx)),
                );
                row_height.saturating_add(gap)
            }
        };
        if used_rows.saturating_add(needed) > body.height {
            break;
        }
        used_rows = used_rows.saturating_add(needed);
        visible += 1;
    }
    visible
}

pub(crate) fn workspace_list_scroll_metrics(
    app: &AppState,
    area: Rect,
) -> crate::pane::ScrollMetrics {
    let entries = workspace_list_entries(app);
    let total_rows = entries.len();
    let scroll = app.workspace_scroll.min(total_rows.saturating_sub(1));
    let viewport_rows = workspace_list_visible_count(app, area, scroll);
    let max_offset_from_bottom = total_rows.saturating_sub(viewport_rows);
    let offset_from_bottom = total_rows
        .saturating_sub(scroll)
        .saturating_sub(viewport_rows);

    crate::pane::ScrollMetrics {
        offset_from_bottom,
        max_offset_from_bottom,
        viewport_rows,
    }
}

pub(crate) fn workspace_list_scrollbar_rect(app: &AppState, area: Rect) -> Option<Rect> {
    let metrics = workspace_list_scroll_metrics(app, area);
    let body = workspace_list_body_rect(area, true);
    (should_show_scrollbar(metrics) && body.width > 0 && body.height > 0).then_some(Rect::new(
        area.x + area.width.saturating_sub(1),
        body.y,
        1,
        body.height,
    ))
}

pub(crate) fn agent_panel_body_rect(area: Rect, has_scrollbar: bool) -> Rect {
    if area.width == 0 || area.height <= AGENT_PANEL_HEADER_ROWS {
        return Rect::default();
    }

    let body_y = area.y.saturating_add(AGENT_PANEL_HEADER_ROWS);
    let body_height = (area.y + area.height).saturating_sub(body_y);
    let body_width = area.width.saturating_sub(u16::from(has_scrollbar));
    Rect::new(area.x, body_y, body_width, body_height)
}

fn agent_panel_visible_count(app: &AppState, area: Rect, scroll: usize) -> usize {
    let body = agent_panel_body_rect(area, false);
    if body.width == 0 || body.height < 2 {
        return 0;
    }
    compute_agent_panel_row_areas(&agent_panel_rows(app), body, scroll).len()
}

pub(crate) fn agent_panel_scroll_metrics(app: &AppState, area: Rect) -> crate::pane::ScrollMetrics {
    let total_rows = agent_panel_rows(app).len();
    let scroll = app.agent_panel_scroll.min(total_rows.saturating_sub(1));
    let viewport_rows = agent_panel_visible_count(app, area, scroll);
    let max_offset_from_bottom = total_rows.saturating_sub(viewport_rows);
    let offset_from_bottom = total_rows
        .saturating_sub(app.agent_panel_scroll)
        .saturating_sub(viewport_rows);

    crate::pane::ScrollMetrics {
        offset_from_bottom,
        max_offset_from_bottom,
        viewport_rows,
    }
}

pub(crate) fn agent_panel_scrollbar_rect(app: &AppState, area: Rect) -> Option<Rect> {
    let metrics = agent_panel_scroll_metrics(app, area);
    let body = agent_panel_body_rect(area, true);
    (should_show_scrollbar(metrics) && body.width > 0 && body.height > 0).then_some(Rect::new(
        area.x + area.width.saturating_sub(1),
        body.y,
        1,
        body.height,
    ))
}

pub(crate) fn compute_workspace_list_areas(
    app: &AppState,
    area: Rect,
) -> (Vec<crate::app::state::WorkspaceCardArea>, Vec<()>) {
    let ws_area = workspace_list_rect(area, app.sidebar_section_split);
    if ws_area == Rect::default() {
        return (Vec::new(), Vec::new());
    }

    let metrics = workspace_list_scroll_metrics(app, ws_area);
    let body = workspace_list_body_rect(ws_area, should_show_scrollbar(metrics));
    if body.width == 0 || body.height == 0 {
        return (Vec::new(), Vec::new());
    }

    let scroll = app.workspace_scroll;
    let mut row_y = body.y;
    let body_bottom = body.y + body.height;
    let mut cards = Vec::new();
    let headers = Vec::new();

    let entries = workspace_list_entries(app);
    for (entry_idx, entry) in entries.iter().enumerate().skip(scroll) {
        match entry {
            WorkspaceListEntry::Workspace { ws_idx, indented } => {
                let Some(ws) = app.workspaces.get(*ws_idx) else {
                    continue;
                };
                let row_height = if *indented {
                    1
                } else {
                    workspace_row_height(ws)
                };
                let gap = u16::from(
                    !(*indented && next_entry_is_indented_workspace(&entries, entry_idx)),
                );
                if row_y.saturating_add(row_height).saturating_add(gap) > body_bottom {
                    break;
                }
                cards.push(crate::app::state::WorkspaceCardArea {
                    ws_idx: *ws_idx,
                    rect: Rect::new(body.x, row_y, body.width, row_height),
                    indented: *indented,
                });
                row_y = row_y.saturating_add(row_height + gap);
            }
        }
    }

    (cards, headers)
}

pub(crate) fn compute_workspace_card_areas(
    app: &AppState,
    area: Rect,
) -> Vec<crate::app::state::WorkspaceCardArea> {
    compute_workspace_list_areas(app, area).0
}

/// Auto-scale sidebar width based on workspace identity + agent summary.
pub(crate) fn collapsed_sidebar_sections(area: Rect) -> (Rect, Option<u16>, Rect) {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.width == 0 || content.height == 0 {
        return (Rect::default(), None, Rect::default());
    }

    if content.height < 7 {
        return (content, None, Rect::default());
    }

    let total_h = content.height as usize;
    let ws_h = total_h.div_ceil(2);
    let detail_h = total_h.saturating_sub(ws_h + 1);
    if ws_h == 0 || detail_h == 0 {
        return (content, None, Rect::default());
    }

    let divider_y = content.y + ws_h as u16;
    let ws_area = Rect::new(content.x, content.y, content.width, ws_h as u16);
    let detail_area = Rect::new(content.x, divider_y + 1, content.width, detail_h as u16);
    (ws_area, Some(divider_y), detail_area)
}

/// Collapsed sidebar: workspace glance on top, compact agent list below.
pub(super) fn render_sidebar_collapsed(app: &AppState, frame: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let is_navigating = matches!(app.mode, Mode::Navigate);

    let p = &app.palette;
    let sep_style = if is_navigating {
        Style::default().fg(p.accent)
    } else {
        Style::default().fg(p.surface_dim)
    };
    let sep_x = area.x + area.width.saturating_sub(1);
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        buf[(sep_x, y)].set_symbol("│");
        buf[(sep_x, y)].set_style(sep_style);
    }

    let (ws_area, divider_y, detail_area) = collapsed_sidebar_sections(area);
    if ws_area == Rect::default() {
        render_sidebar_toggle(app, frame, area, true, p);
        return;
    }

    let mut row: u16 = 0;
    for (visible_idx, ws) in app.workspaces.iter().enumerate() {
        let y = ws_area.y + row;
        if y >= ws_area.y + ws_area.height {
            break;
        }
        row = row.saturating_add(1);
        let (agg_state, agg_seen) = ws.aggregate_state(&app.terminals);
        let (icon, icon_style) = state_dot(agg_state, agg_seen, p);
        let is_selected = visible_idx == app.selected && is_navigating;
        let is_active = Some(visible_idx) == app.active;
        let row_style = if is_selected {
            Style::default().bg(p.surface0)
        } else if is_active {
            Style::default().bg(p.surface_dim)
        } else {
            Style::default()
        };
        let num_style = if is_selected {
            Style::default().fg(p.overlay1).bg(p.surface0)
        } else if is_active {
            Style::default().fg(p.text).bg(p.surface_dim)
        } else {
            Style::default().fg(p.overlay0)
        };

        if is_selected || is_active {
            let buf = frame.buffer_mut();
            for x in ws_area.x..ws_area.x + ws_area.width {
                buf[(x, y)].set_style(row_style);
            }
        }

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("{}", visible_idx + 1), num_style),
                Span::styled(" ", row_style),
                Span::styled(icon, icon_style),
            ])),
            Rect::new(ws_area.x, y, ws_area.width, 1),
        );
    }

    if let Some(divider_y) = divider_y {
        let buf = frame.buffer_mut();
        for x in ws_area.x..ws_area.x + ws_area.width {
            buf[(x, divider_y)].set_symbol("─");
            buf[(x, divider_y)].set_style(Style::default().fg(p.surface_dim));
        }
    }

    let detail_ws_idx = if is_navigating {
        Some(app.selected)
    } else {
        app.active
    };
    let detail_content_area = Rect::new(
        detail_area.x,
        detail_area.y,
        detail_area.width,
        detail_area.height.saturating_sub(1),
    );
    if detail_content_area != Rect::default() {
        if let Some(ws_idx) = detail_ws_idx {
            if let Some(ws) = app.workspaces.get(ws_idx) {
                for (detail_idx, detail) in ws.pane_details(&app.terminals).iter().enumerate() {
                    let y = detail_content_area.y + detail_idx as u16;
                    if y >= detail_content_area.y + detail_content_area.height {
                        break;
                    }
                    let pane_num = ws
                        .public_pane_number(detail.pane_id)
                        .unwrap_or(detail_idx + 1);
                    let pane_style = Style::default().fg(p.overlay0);
                    let (icon, icon_style) =
                        agent_icon(detail.state, detail.seen, app.spinner_tick, p);
                    frame.render_widget(
                        Paragraph::new(Line::from(vec![
                            Span::styled(format!("{pane_num}"), pane_style),
                            Span::styled(" ", pane_style),
                            Span::styled(icon, icon_style),
                        ])),
                        Rect::new(detail_content_area.x, y, detail_content_area.width, 1),
                    );
                }
            }
        }
    }

    render_sidebar_toggle(app, frame, area, true, p);
}

pub(crate) fn workspace_drop_indicator_row(
    cards: &[crate::app::state::WorkspaceCardArea],
    area: Rect,
    insert_idx: usize,
) -> Option<u16> {
    if area.height == 0 {
        return None;
    }
    let list_bottom = area.y + area.height.saturating_sub(1);

    let first = cards.first()?;
    if insert_idx == first.ws_idx {
        return first.rect.y.checked_sub(1).filter(|y| *y < list_bottom);
    }

    if let Some(row) = cards
        .last()
        .filter(|card| insert_idx == card.ws_idx.saturating_add(1))
        .map(|card| card.rect.y.saturating_add(card.rect.height))
        .filter(|y| *y < list_bottom)
    {
        return Some(row);
    }

    if let Some(card) = cards.iter().find(|card| card.ws_idx == insert_idx) {
        return card.rect.y.checked_sub(1).filter(|y| *y < list_bottom);
    }

    None
}

/// Screen row for the manual-agent drop indicator at flat `insert_idx`, computed
/// from the variable-height visible row areas. The indicator sits at the top of
/// the target slot (the gap row above the entry, or the panel top for the first
/// slot); inserting past the last visible row draws in the gap below it.
pub(crate) fn agent_panel_drop_indicator_row(
    areas: &[AgentPanelRowArea],
    body: Rect,
    insert_idx: usize,
) -> Option<u16> {
    if body.height == 0 {
        return None;
    }
    let body_bottom = body.y + body.height;
    if let Some(area) = areas.iter().find(|area| area.row_idx == insert_idx) {
        let y = if area.y == body.y {
            body.y
        } else {
            area.y.saturating_sub(1)
        };
        return (y < body_bottom).then_some(y);
    }
    if let Some(last) = areas.last() {
        if insert_idx >= last.row_idx.saturating_add(1) {
            let y = last.y.saturating_add(last.height);
            return (y < body_bottom).then_some(y);
        }
    }
    None
}

pub(super) fn render_sidebar(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    area: Rect,
) {
    let p = &app.palette;
    let is_navigating = matches!(app.mode, Mode::Navigate);
    let sep_style = if is_navigating {
        Style::default().fg(p.accent)
    } else {
        Style::default().fg(p.surface_dim)
    };

    let sep_x = area.x + area.width.saturating_sub(1);
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        buf[(sep_x, y)].set_symbol("│");
        buf[(sep_x, y)].set_style(sep_style);
    }

    let (ws_area, pane_section_area, detail_area) = expanded_sidebar_sections3(
        area,
        app.sidebar_section_split,
        app.sidebar_pane_section_split,
        sidebar_shows_pane_section(app),
    );

    render_workspace_list(app, terminal_runtimes, frame, ws_area, is_navigating);
    render_pane_section(app, terminal_runtimes, frame, pane_section_area);
    render_agent_detail(app, terminal_runtimes, frame, detail_area);
    render_sidebar_toggle(app, frame, area, false, p);
}

/// The display name shown for a Panes-section row: the pane's own effective name
/// (manual label / terminal title) when it has one, otherwise the containing
/// tab's name, otherwise a positional fallback.
fn pane_section_row_name(
    app: &AppState,
    ws: &crate::workspace::Workspace,
    row_pane_id: crate::layout::PaneId,
    tab_idx: usize,
) -> String {
    let pane_name = ws.pane_state(row_pane_id).and_then(|pane| {
        app.terminals
            .get(&pane.attached_terminal_id)
            .and_then(|terminal| terminal.border_label(false))
    });
    pane_name
        .or_else(|| ws.tab_display_name(tab_idx))
        .unwrap_or_else(|| (tab_idx + 1).to_string())
}

/// Render the Panes section: every non-agent pane across all spaces as a
/// two-line row (pane name over its space name), ordered by the client-only
/// Panes-section order, with a drop indicator during a reorder drag.
fn render_pane_section(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    area: Rect,
) {
    let p = &app.palette;
    if area.height < 3 {
        return;
    }

    let sep_line = "─".repeat(area.width as usize);
    frame.render_widget(
        Paragraph::new(Span::styled(&sep_line, Style::default().fg(p.surface_dim))),
        Rect::new(area.x, area.y, area.width, 1),
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            " panes",
            Style::default().fg(p.overlay0).add_modifier(Modifier::BOLD),
        )])),
        Rect::new(area.x, area.y + 1, area.width, 1),
    );

    let metrics = pane_section_scroll_metrics(app, area);
    let scrollbar_rect = pane_section_scrollbar_rect(app, area);
    let body = pane_section_body_rect(area, should_show_scrollbar(metrics));
    if body == Rect::default() {
        return;
    }

    let dragged = match app.drag.as_ref().map(|drag| &drag.target) {
        Some(crate::app::state::DragTarget::PaneSectionReorder { source, .. }) => {
            Some(source.clone())
        }
        _ => None,
    };

    let areas = &app.view.pane_section_row_areas;
    let max_width = body.width as usize;
    for row in areas {
        let ws = &app.workspaces[row.ws_idx];
        let is_active = Some(row.ws_idx) == app.active
            && ws.active_tab == row.tab_idx
            && ws.focused_pane_id() == Some(row.pane_id);
        let is_dragged = dragged.as_ref().is_some_and(|source| {
            source.workspace_id == ws.id
                && ws.public_pane_number(row.pane_id) == Some(source.pane_number)
        });

        if is_active || is_dragged {
            let bg = if is_dragged {
                p.surface1
            } else {
                p.surface_dim
            };
            let buf = frame.buffer_mut();
            for y in row.rect.y..row.rect.y + row.rect.height {
                for x in row.rect.x..row.rect.x + row.rect.width {
                    buf[(x, y)].set_style(Style::default().bg(bg));
                }
            }
        }

        let name_style = if is_active || is_dragged {
            Style::default().fg(p.text).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.text)
        };
        let pane_name = pane_section_row_name(app, ws, row.pane_id, row.tab_idx);
        let space_name = ws.display_name_from(&app.terminals, terminal_runtimes);
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!(" {}", truncate_end(&pane_name, max_width.saturating_sub(1))),
                name_style,
            )])),
            Rect::new(body.x, row.rect.y, body.width, 1),
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!(
                    " {}",
                    truncate_end(&space_name, max_width.saturating_sub(1))
                ),
                Style::default().fg(p.overlay0),
            )])),
            Rect::new(body.x, row.rect.y + 1, body.width, 1),
        );
    }

    if let Some(insert_idx) = match app.drag.as_ref().map(|drag| &drag.target) {
        Some(crate::app::state::DragTarget::PaneSectionReorder {
            insert_idx: Some(insert_idx),
            ..
        }) => Some(*insert_idx),
        _ => None,
    } {
        if let Some(y) = pane_section_drop_indicator_row(areas, body, insert_idx) {
            let indicator_right = scrollbar_rect
                .map(|rect| rect.x)
                .unwrap_or(body.x + body.width);
            let buf = frame.buffer_mut();
            for x in body.x..indicator_right {
                buf[(x, y)].set_symbol("─");
                buf[(x, y)].set_style(Style::default().fg(p.accent));
            }
        }
    }

    if let Some(track) = scrollbar_rect {
        render_scrollbar(frame, metrics, track, p.surface_dim, p.overlay0, "▕");
    }
}

fn render_workspace_list(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    area: Rect,
    is_navigating: bool,
) {
    let p = &app.palette;
    let dragged_ws_idx = match app.drag.as_ref().map(|drag| &drag.target) {
        Some(crate::app::state::DragTarget::WorkspaceReorder { source_ws_idx, .. }) => {
            Some(*source_ws_idx)
        }
        _ => None,
    };
    let insertion_row = match app.drag.as_ref().map(|drag| &drag.target) {
        Some(crate::app::state::DragTarget::WorkspaceReorder {
            insert_idx: Some(insert_idx),
            ..
        }) => workspace_drop_indicator_row(&app.view.workspace_card_areas, area, *insert_idx),
        _ => None,
    };

    let list_bottom = area.y + area.height.saturating_sub(1);
    if area.height > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                " spaces",
                Style::default().fg(p.overlay0).add_modifier(Modifier::BOLD),
            )])),
            Rect::new(area.x, area.y, area.width, 1),
        );
    }

    let metrics = workspace_list_scroll_metrics(app, area);
    let scrollbar_rect = workspace_list_scrollbar_rect(app, area);
    let cards = &app.view.workspace_card_areas;

    for card in cards {
        let i = card.ws_idx;
        let ws = &app.workspaces[i];
        let row_y = card.rect.y;
        let row_height = card.rect.height;
        let selected = i == app.selected && is_navigating;
        let is_active = Some(i) == app.active;
        let is_dragged = dragged_ws_idx == Some(i);
        let highlighted = selected || is_active || is_dragged;
        let (agg_state, agg_seen) = ws.aggregate_state(&app.terminals);

        if highlighted {
            let bg = if selected {
                p.surface0
            } else if is_dragged {
                p.surface1
            } else {
                p.surface_dim
            };
            let buf = frame.buffer_mut();
            for y in row_y..row_y + row_height {
                if y >= list_bottom {
                    break;
                }
                for x in card.rect.x..card.rect.x + card.rect.width {
                    buf[(x, y)].set_style(Style::default().bg(bg));
                }
            }
        }

        let name_style = if selected || is_active || is_dragged {
            Style::default().fg(p.text).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.subtext0)
        };

        let (icon, icon_style) = state_dot(agg_state, agg_seen, p);
        let label = ws.display_name_from(&app.terminals, terminal_runtimes);
        let mut line1 = Vec::new();
        let mut show_workspace_icon = true;
        if card.indented {
            line1.push(Span::styled("   ", Style::default()));
        } else if let Some((key, collapsed)) = workspace_parent_group_state(app, i) {
            let icon = if collapsed { "▸" } else { "▾" };
            let (state_icon, state_style) = if collapsed {
                let (state, seen) = space_aggregate_state(app, &key);
                state_dot(state, seen, p)
            } else {
                (icon, Style::default().fg(p.accent))
            };
            line1.push(Span::styled(icon, Style::default().fg(p.accent)));
            if collapsed {
                line1.push(Span::styled(" ", Style::default()));
                line1.push(Span::styled(state_icon, state_style));
                show_workspace_icon = false;
            }
            line1.push(Span::styled(" ", Style::default()));
        } else {
            line1.push(Span::styled(" ", Style::default()));
        }
        if show_workspace_icon {
            line1.push(Span::styled(icon, icon_style));
            line1.push(Span::styled(" ", Style::default()));
        }
        if card.indented {
            let display_label = grouped_child_display_label(
                &label,
                ws.branch().as_deref(),
                ws.custom_name.is_some(),
            );
            line1.push(Span::styled(display_label, name_style));
        } else {
            line1.push(Span::styled(label, name_style));
        }

        frame.render_widget(
            Paragraph::new(Line::from(line1)),
            Rect::new(card.rect.x, row_y, card.rect.width, 1),
        );

        if row_height > 1 && row_y + 1 < list_bottom {
            if let Some(branch) = ws.branch() {
                let upstream_label = ws.git_ahead_behind().and_then(|(ahead, behind)| {
                    let mut parts = Vec::new();
                    if ahead > 0 {
                        parts.push((format!("↑{}", ahead), p.green));
                    }
                    if behind > 0 {
                        parts.push((format!("↓{}", behind), p.red));
                    }
                    (!parts.is_empty()).then_some(parts)
                });
                let reserved = upstream_label
                    .as_ref()
                    .map(|parts| {
                        parts.iter().map(|(label, _)| label.len()).sum::<usize>() + parts.len()
                    })
                    .unwrap_or(0);
                let max_branch_len = (card.rect.width as usize).saturating_sub(5 + reserved);
                let branch_display = truncate_end(&branch, max_branch_len);
                let branch_color = if selected || is_active {
                    p.mauve
                } else {
                    p.overlay0
                };
                let branch_indent = if card.indented { "     " } else { "   " };
                let mut spans = vec![
                    Span::styled(branch_indent, Style::default()),
                    Span::styled(branch_display, Style::default().fg(branch_color)),
                ];
                if let Some(parts) = upstream_label {
                    spans.push(Span::styled(" ", Style::default()));
                    for (idx, (label, color)) in parts.into_iter().enumerate() {
                        if idx > 0 {
                            spans.push(Span::styled(" ", Style::default()));
                        }
                        spans.push(Span::styled(label, Style::default().fg(color)));
                    }
                }
                frame.render_widget(
                    Paragraph::new(Line::from(spans)),
                    Rect::new(card.rect.x, row_y + 1, card.rect.width, 1),
                );
            }
        }
    }

    if let Some(y) = insertion_row.filter(|y| *y < list_bottom) {
        let indicator_right = scrollbar_rect
            .map(|rect| rect.x)
            .unwrap_or(area.x + area.width);
        let buf = frame.buffer_mut();
        for x in area.x..indicator_right {
            buf[(x, y)].set_symbol("─");
            buf[(x, y)].set_style(Style::default().fg(p.accent));
        }
    }

    if let Some(track) = scrollbar_rect {
        render_scrollbar(frame, metrics, track, p.surface_dim, p.overlay0, "▕");
    }

    if app.mouse_capture && list_bottom > area.y {
        let new_rect = app.sidebar_new_button_rect();
        frame.render_widget(
            Paragraph::new(Span::styled(" new", Style::default().fg(p.overlay0))),
            new_rect,
        );

        let menu_rect = app.global_launcher_rect();
        let menu_line = if app.global_menu_attention_badge_visible() {
            Line::from(vec![
                Span::styled(
                    "● ",
                    Style::default().fg(p.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled("menu", Style::default().fg(p.overlay0)),
            ])
        } else {
            Line::from(vec![Span::styled("menu", Style::default().fg(p.overlay0))])
        };
        frame.render_widget(
            Paragraph::new(menu_line).alignment(Alignment::Right),
            menu_rect,
        );
    }
}

/// Draw a named line-split divider as a full-width rule with the name embedded,
/// e.g. `── scheduled ──────`. An empty name renders a plain rule.
fn render_line_split_row(frame: &mut Frame, body: Rect, y: u16, name: &str, p: &Palette) {
    let width = body.width as usize;
    if width == 0 {
        return;
    }
    let dash_style = Style::default().fg(p.surface_dim);
    // The split name uses the same color as the workspace name in agent rows so
    // the label stays legible; the surrounding rule stays subtly dim.
    let name_style = Style::default().fg(agent_row_space_color());
    let trimmed = name.trim();
    let line = if trimmed.is_empty() {
        Line::from(Span::styled("─".repeat(width), dash_style))
    } else {
        let prefix = "── ";
        let label = format!("{trimmed} ");
        let used = display_width(prefix) + display_width(&label);
        if used >= width {
            Line::from(Span::styled(
                truncate_end(&format!("{prefix}{label}"), width),
                name_style,
            ))
        } else {
            Line::from(vec![
                Span::styled(prefix.to_string(), dash_style),
                Span::styled(label, name_style),
                Span::styled("─".repeat(width - used), dash_style),
            ])
        }
    };
    frame.render_widget(Paragraph::new(line), Rect::new(body.x, y, body.width, 1));
}

fn render_agent_detail(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    area: Rect,
) {
    let p = &app.palette;

    if area.height < 3 {
        return;
    }

    let sep_line = "─".repeat(area.width as usize);
    frame.render_widget(
        Paragraph::new(Span::styled(&sep_line, Style::default().fg(p.surface_dim))),
        Rect::new(area.x, area.y, area.width, 1),
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            " agents",
            Style::default().fg(p.overlay0).add_modifier(Modifier::BOLD),
        )])),
        Rect::new(area.x, area.y + 1, area.width, 1),
    );
    let toggle_rect = agent_panel_toggle_rect(area, app.agent_panel_sort);
    if toggle_rect != Rect::default() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                agent_panel_sort_label(app.agent_panel_sort),
                Style::default().fg(p.overlay0).add_modifier(Modifier::BOLD),
            ))
            .alignment(Alignment::Right),
            toggle_rect,
        );
    }
    if app.mouse_capture {
        let split_rect = agent_panel_split_button_rect(area, app.agent_panel_sort);
        if split_rect != Rect::default() {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    AGENT_PANEL_SPLIT_LABEL,
                    Style::default().fg(p.overlay0),
                )),
                split_rect,
            );
        }
    }

    let rows = agent_panel_rows_from(app, terminal_runtimes);
    let metrics = agent_panel_scroll_metrics(app, area);
    let scrollbar_rect = agent_panel_scrollbar_rect(app, area);
    let body = agent_panel_body_rect(area, should_show_scrollbar(metrics));
    if body == Rect::default() {
        return;
    }

    let areas = compute_agent_panel_row_areas(&rows, body, app.agent_panel_scroll);
    let max_width = body.width as usize;
    for area_row in &areas {
        let row = &rows[area_row.row_idx];
        match row {
            AgentPanelRow::Agent(detail) => {
                let is_active = app.is_active_pane(detail.ws_idx, detail.tab_idx, detail.pane_id);
                let (icon, icon_style) = agent_icon(detail.state, detail.seen, app.spinner_tick, p);
                let label_color = state_label_color(detail.state, detail.seen, p);
                let label = detail
                    .state_labels
                    .get(agent_panel_status_key(detail.state, detail.seen))
                    .map(String::as_str)
                    .unwrap_or_else(|| state_label(detail.state, detail.seen));

                let row_style = if is_active {
                    Style::default().bg(p.surface_dim)
                } else {
                    Style::default()
                };

                let row_ctx = AgentRowContext {
                    icon,
                    icon_style,
                    space: &detail.primary_label,
                    tab: detail.primary_tab_label.as_deref(),
                    status: label,
                    status_color: label_color,
                    is_active,
                };

                let lines = [row_ctx.line_one(p, max_width), row_ctx.line_two(max_width)];
                let mut line_y = area_row.y;
                for spans in lines {
                    frame.render_widget(
                        Paragraph::new(Line::from(spans)).style(row_style),
                        Rect::new(body.x, line_y, body.width, 1),
                    );
                    line_y += 1;
                }
            }
            AgentPanelRow::LineSplit { name, .. } => {
                render_line_split_row(frame, body, area_row.y, name, p);
            }
        }
    }

    if let Some(insert_idx) = match app.drag.as_ref().map(|drag| &drag.target) {
        Some(crate::app::state::DragTarget::AgentReorder {
            insert_idx: Some(insert_idx),
            ..
        }) => Some(*insert_idx),
        _ => None,
    } {
        if let Some(y) = agent_panel_drop_indicator_row(&areas, body, insert_idx) {
            let indicator_right = scrollbar_rect
                .map(|rect| rect.x)
                .unwrap_or(body.x + body.width);
            let buf = frame.buffer_mut();
            for x in body.x..indicator_right {
                buf[(x, y)].set_symbol("─");
                buf[(x, y)].set_style(Style::default().fg(p.accent));
            }
        }
    }

    if let Some(track) = scrollbar_rect {
        render_scrollbar(frame, metrics, track, p.surface_dim, p.overlay0, "▕");
    }
}

pub(crate) fn collapsed_sidebar_toggle_rect(area: Rect) -> Rect {
    let bottom_y = area.y + area.height.saturating_sub(1);
    let content_w = area.width.saturating_sub(1);
    if content_w == 0 || area.height == 0 {
        return Rect::default();
    }
    let x = area.x + content_w / 2;
    Rect::new(x, bottom_y, 1, 1)
}

pub(crate) fn expanded_sidebar_toggle_rect(area: Rect) -> Rect {
    if area.width <= 1 || area.height == 0 {
        return Rect::default();
    }
    Rect::new(
        area.x + area.width.saturating_sub(2),
        area.y + area.height.saturating_sub(1),
        1,
        1,
    )
}

fn render_sidebar_toggle(
    app: &AppState,
    frame: &mut Frame,
    area: Rect,
    collapsed: bool,
    p: &Palette,
) {
    let toggle_area = if collapsed {
        collapsed_sidebar_toggle_rect(area)
    } else {
        expanded_sidebar_toggle_rect(area)
    };
    if toggle_area == Rect::default() {
        return;
    }
    let icon = if collapsed { "»" } else { "«" };
    let icon_style = if collapsed && app.global_menu_attention_badge_visible() {
        Style::default().fg(p.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(p.overlay0)
    };
    frame.render_widget(Paragraph::new(Span::styled(icon, icon_style)), toggle_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{detect::Agent, workspace::Workspace};
    use ratatui::{backend::TestBackend, Terminal};

    fn agent_row_palette() -> Palette {
        Palette::catppuccin()
    }

    fn agent_row_ctx() -> AgentRowContext<'static> {
        AgentRowContext {
            icon: "✓",
            icon_style: Style::default().fg(Color::Green),
            space: "herdr",
            tab: Some("main"),
            status: "idle",
            status_color: Color::Green,
            is_active: false,
        }
    }

    fn agent_row_text(spans: &[Span<'_>]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    #[test]
    fn agent_row_lines_match_hardcoded_layout() {
        let ctx = agent_row_ctx();
        let p = agent_row_palette();
        assert_eq!(agent_row_text(&ctx.line_one(&p, 80)), " ✓ main");
        assert_eq!(agent_row_text(&ctx.line_two(80)), " herdr · idle");
    }

    #[test]
    fn agent_row_empty_tab_drops_tab_and_separating_space() {
        let mut ctx = agent_row_ctx();
        ctx.tab = None;
        let p = agent_row_palette();
        assert_eq!(agent_row_text(&ctx.line_one(&p, 80)), " ✓");

        ctx.tab = Some("");
        assert_eq!(agent_row_text(&ctx.line_one(&p, 80)), " ✓");
    }

    #[test]
    fn agent_row_empty_status_drops_separator_and_status() {
        let mut ctx = agent_row_ctx();
        ctx.status = "";
        assert_eq!(agent_row_text(&ctx.line_two(80)), " herdr");
    }

    #[test]
    fn agent_row_space_uses_fixed_dark_grey() {
        // The space color is resolved from the `darkgrey` config color and is
        // independent of active state.
        let expected = crate::config::parse_color_checked("darkgrey").expect("darkgrey resolves");
        assert_eq!(agent_row_space_color(), expected);
        let mut ctx = agent_row_ctx();
        let space_span = |ctx: &AgentRowContext| ctx.line_two(80)[1].style.fg;
        assert_eq!(space_span(&ctx), Some(expected));
        ctx.is_active = true;
        assert_eq!(space_span(&ctx), Some(expected));
    }

    #[test]
    fn agent_row_tab_uses_active_name_style() {
        let p = agent_row_palette();
        let mut ctx = agent_row_ctx();

        ctx.is_active = true;
        let active = ctx.line_one(&p, 80);
        assert_eq!(active[2].style.fg, Some(p.text));
        assert!(active[2].style.add_modifier.contains(Modifier::BOLD));

        ctx.is_active = false;
        let inactive = ctx.line_one(&p, 80);
        assert_eq!(inactive[2].style.fg, Some(p.subtext0));
        assert!(inactive[2].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn agent_row_status_dims_when_inactive() {
        let mut ctx = agent_row_ctx();

        ctx.is_active = true;
        let active = ctx.line_two(80);
        assert_eq!(active[2].style.fg, Some(Color::Green));
        assert!(!active[2].style.add_modifier.contains(Modifier::DIM));

        ctx.is_active = false;
        let inactive = ctx.line_two(80);
        assert_eq!(inactive[2].style.fg, Some(Color::Green));
        assert!(inactive[2].style.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn agent_row_icon_keeps_intrinsic_style() {
        let ctx = agent_row_ctx();
        let spans = ctx.line_one(&agent_row_palette(), 80);
        assert_eq!(spans[1].style.fg, Some(Color::Green));
    }

    #[test]
    fn agent_row_lines_truncate_to_width() {
        let mut ctx = agent_row_ctx();
        ctx.space = "a-very-long-workspace-name";
        let text = agent_row_text(&ctx.line_two(6));
        assert_eq!(display_width(&text), 6);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn render_sidebar_toggle_draws_expanded_collapse_icon() {
        let app = crate::app::state::AppState::test_new();
        let area = Rect::new(0, 0, 26, 20);
        let mut terminal =
            Terminal::new(TestBackend::new(26, 20)).expect("test terminal should initialize");

        terminal
            .draw(|frame| render_sidebar_toggle(&app, frame, area, false, &app.palette))
            .expect("sidebar toggle should render");

        let toggle = expanded_sidebar_toggle_rect(area);
        assert_eq!(
            terminal.backend().buffer()[(toggle.x, toggle.y)].symbol(),
            "«"
        );
    }

    #[test]
    fn expanded_sidebar_toggle_sits_inside_sidebar_content() {
        let area = Rect::new(0, 0, 26, 20);
        let toggle = expanded_sidebar_toggle_rect(area);

        assert_eq!(toggle.x, area.x + area.width - 2);
        assert_eq!(toggle.y, area.y + area.height - 1);
    }

    #[test]
    fn all_workspaces_agent_panel_entries_use_workspace_and_optional_tab_labels() {
        let mut app = crate::app::state::AppState::test_new();
        let first = Workspace::test_new("one");
        let first_pane = first.tabs[0].root_pane;
        let mut second = Workspace::test_new("two");
        let second_tab = second.test_add_tab(Some("logs"));
        let second_pane = second.tabs[second_tab].root_pane;

        app.workspaces = vec![first, second];
        app.ensure_test_terminals();
        let first_terminal_id = app.workspaces[0].tabs[0].panes[&first_pane]
            .attached_terminal_id
            .clone();
        app.terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Pi);
        let second_terminal_id = app.workspaces[1].tabs[second_tab].panes[&second_pane]
            .attached_terminal_id
            .clone();
        app.terminals
            .get_mut(&second_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Claude);
        app.active = Some(0);
        app.selected = 0;

        let entries = agent_panel_entries(&app);
        assert_eq!(entries[0].primary_label, "one");
        assert_eq!(entries[0].primary_tab_label.as_deref(), Some("1"));
        assert_eq!(entries[0].agent_label.as_deref(), Some("pi"));
        assert_eq!(entries[1].primary_label, "two");
        assert_eq!(entries[1].primary_tab_label.as_deref(), Some("logs"));
        assert_eq!(entries[1].agent_label.as_deref(), Some("claude"));
    }

    fn mark_pane_agent(app: &mut AppState, pane: crate::layout::PaneId) {
        let terminal_id = app
            .workspaces
            .iter()
            .find_map(|ws| {
                ws.tabs
                    .iter()
                    .find_map(|tab| tab.panes.get(&pane))
                    .map(|p| p.attached_terminal_id.clone())
            })
            .expect("pane has a terminal");
        app.terminals
            .get_mut(&terminal_id)
            .expect("terminal exists")
            .detected_agent = Some(Agent::Pi);
    }

    #[test]
    fn sidebar_two_section_split_geometry_unchanged() {
        // Characterization: pin the historical two-band Spaces/Agents geometry so
        // the three-band generalization keeps the Spaces band identical.
        let area = Rect::new(0, 0, 26, 40);
        assert_eq!(
            expanded_sidebar_sections(area, 0.5),
            (Rect::new(0, 0, 25, 20), Rect::new(0, 20, 25, 20))
        );
        assert_eq!(
            expanded_sidebar_sections(area, 0.3),
            (Rect::new(0, 0, 25, 12), Rect::new(0, 12, 25, 28))
        );
        // Ratio clamps to [0.1, 0.9]; each band keeps at least three rows.
        assert_eq!(
            expanded_sidebar_sections(area, 0.99),
            (Rect::new(0, 0, 25, 36), Rect::new(0, 36, 25, 4))
        );
    }

    #[test]
    fn three_section_split_partitions_full_height_without_overlap() {
        let area = Rect::new(0, 0, 26, 40);
        let (spaces, tabs, agents) = expanded_sidebar_sections3(area, 0.5, 0.5, true);
        // Contiguous, non-overlapping, and covering the whole content height.
        assert_eq!(spaces.y, 0);
        assert_eq!(tabs.y, spaces.y + spaces.height);
        assert_eq!(agents.y, tabs.y + tabs.height);
        assert_eq!(agents.y + agents.height, 40);
        assert_eq!(
            spaces.height + tabs.height + agents.height,
            40,
            "bands cover the full content height"
        );
        assert!(tabs.height >= 3 && agents.height >= 3);

        // With no Panes entries the Panes band collapses and Agents keeps the
        // historical two-band geometry.
        let (spaces2, tabs2, agents2) = expanded_sidebar_sections3(area, 0.5, 0.5, false);
        assert_eq!(tabs2.height, 0);
        assert_eq!(spaces2, spaces);
        assert_eq!(agents2, expanded_sidebar_sections(area, 0.5).1);
    }

    #[test]
    fn pane_section_lists_only_non_agent_tabs() {
        let mut app = AppState::test_new();
        let mut ws = Workspace::test_new("one"); // tab 0: plain
        ws.test_add_tab(Some("agent")); // tab 1: pure agent
        ws.test_add_tab(Some("mixed")); // tab 2: mixed (agent + shell)
        ws.active_tab = 2;
        let mixed_extra = ws.test_split(ratatui::layout::Direction::Horizontal);
        let agent_pane = ws.tabs[1].root_pane;
        let mixed_agent_pane = ws.tabs[2].root_pane;
        app.workspaces = vec![ws];
        app.ensure_test_terminals();
        mark_pane_agent(&mut app, agent_pane);
        mark_pane_agent(&mut app, mixed_agent_pane);
        let _ = mixed_extra; // left as a non-agent shell so tab 2 stays mixed
        app.active = Some(0);
        app.selected = 0;

        app.reconcile_pane_section_order();
        let entries: Vec<usize> = sidebar_pane_section_entries(&app)
            .into_iter()
            .map(|entry| entry.tab_idx)
            .collect();
        // Plain (0) and mixed (2) tabs appear; the pure-agent tab (1) does not.
        assert_eq!(entries, vec![0, 2]);
    }

    #[test]
    fn priority_agent_panel_sort_uses_attention_then_space_order() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![
            Workspace::test_new("one"),
            Workspace::test_new("two"),
            Workspace::test_new("three"),
            Workspace::test_new("four"),
        ];
        app.ensure_test_terminals();
        app.active = Some(0);
        app.selected = 0;
        app.agent_panel_sort = crate::app::state::AgentPanelSort::Priority;

        let set_state = |app: &mut crate::app::state::AppState, ws_idx: usize, state| {
            let pane = app.workspaces[ws_idx].tabs[0].root_pane;
            let terminal_id = app.workspaces[ws_idx].tabs[0].panes[&pane]
                .attached_terminal_id
                .clone();
            let terminal = app.terminals.get_mut(&terminal_id).unwrap();
            terminal.detected_agent = Some(Agent::Claude);
            terminal.state = state;
        };
        set_state(&mut app, 0, AgentState::Working);
        set_state(&mut app, 1, AgentState::Idle);
        set_state(&mut app, 2, AgentState::Working);
        set_state(&mut app, 3, AgentState::Blocked);

        let done_pane = app.workspaces[1].tabs[0].root_pane;
        app.workspaces[1].tabs[0]
            .panes
            .get_mut(&done_pane)
            .unwrap()
            .seen = false;

        let labels: Vec<String> = agent_panel_entries(&app)
            .into_iter()
            .map(|entry| entry.primary_label)
            .collect();

        assert_eq!(labels, ["four", "two", "one", "three"]);
    }

    #[test]
    fn manual_agent_panel_sort_follows_manual_order() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![
            Workspace::test_new("one"),
            Workspace::test_new("two"),
            Workspace::test_new("three"),
        ];
        app.ensure_test_terminals();
        app.active = Some(0);
        app.selected = 0;
        app.agent_panel_sort = crate::app::state::AgentPanelSort::Manual;
        for ws_idx in 0..app.workspaces.len() {
            let pane = app.workspaces[ws_idx].tabs[0].root_pane;
            let terminal_id = app.workspaces[ws_idx].tabs[0].panes[&pane]
                .attached_terminal_id
                .clone();
            app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(Agent::Pi);
        }

        // Seed the natural order, then reverse it via a manual move.
        app.reconcile_agent_manual_order();
        let last = agent_panel_entries(&app)[2].pane_id;
        app.move_agent_entry(crate::app::state::ManualEntryRef::Pane(last), 0);

        let labels: Vec<String> = agent_panel_entries(&app)
            .into_iter()
            .map(|entry| entry.primary_label)
            .collect();
        assert_eq!(labels, ["three", "one", "two"]);
    }

    fn manual_app_with_two_agents() -> crate::app::state::AppState {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.ensure_test_terminals();
        app.active = Some(0);
        app.selected = 0;
        app.agent_panel_sort = crate::app::state::AgentPanelSort::Manual;
        for ws_idx in 0..app.workspaces.len() {
            let pane = app.workspaces[ws_idx].tabs[0].root_pane;
            let terminal_id = app.workspaces[ws_idx].tabs[0].panes[&pane]
                .attached_terminal_id
                .clone();
            app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(Agent::Pi);
        }
        app.reconcile_agent_manual_order();
        app
    }

    #[test]
    fn agent_panel_rows_interleave_line_split_between_agents() {
        let mut app = manual_app_with_two_agents();
        // Insert a line-split between the two agents (flat index 1).
        let id = app
            .agent_manual_order
            .new_line_split("scheduled".to_string(), 1);

        let rows = agent_panel_rows(&app);
        assert_eq!(rows.len(), 3);
        assert!(matches!(rows[0], AgentPanelRow::Agent(_)));
        assert!(matches!(
            &rows[1],
            AgentPanelRow::LineSplit { id: row_id, name } if *row_id == id && name == "scheduled"
        ));
        assert!(matches!(rows[2], AgentPanelRow::Agent(_)));
    }

    #[test]
    fn agent_panel_rows_hide_line_splits_outside_manual_mode() {
        let mut app = manual_app_with_two_agents();
        app.agent_manual_order
            .new_line_split("scheduled".to_string(), 1);
        app.agent_panel_sort = crate::app::state::AgentPanelSort::Spaces;

        let rows = agent_panel_rows(&app);
        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .all(|row| matches!(row, AgentPanelRow::Agent(_))));
    }

    fn render_agent_panel_to_backend(app: &AppState) -> String {
        let area = Rect::new(0, 0, 30, 18);
        let mut terminal =
            Terminal::new(TestBackend::new(30, 18)).expect("test terminal should initialize");
        let runtimes = TerminalRuntimeRegistry::new();
        terminal
            .draw(|frame| render_agent_detail(app, &runtimes, frame, area))
            .expect("agent detail should render");
        let buffer = terminal.backend().buffer().clone();
        let mut lines = Vec::new();
        for y in 0..area.height {
            let mut line = String::new();
            for x in 0..area.width {
                line.push_str(buffer[(x, y)].symbol());
            }
            lines.push(line);
        }
        lines.join("\n")
    }

    #[test]
    fn named_line_split_renders_rule_with_name() {
        let mut app = manual_app_with_two_agents();
        app.agent_manual_order
            .new_line_split("scheduled".to_string(), 0);
        let rendered = render_agent_panel_to_backend(&app);
        assert!(
            rendered.contains("scheduled"),
            "expected named line-split, got:\n{rendered}"
        );
        let split_line = rendered
            .lines()
            .find(|line| line.contains("scheduled"))
            .expect("line-split row present");
        assert!(
            split_line.contains("─"),
            "line-split should be drawn as a rule: {split_line:?}"
        );
    }

    #[test]
    fn empty_line_split_renders_plain_rule() {
        let mut app = manual_app_with_two_agents();
        app.agent_manual_order.new_line_split(String::new(), 0);
        let rendered = render_agent_panel_to_backend(&app);
        // The first body row is the empty line-split: a run of rule characters.
        assert!(
            rendered
                .lines()
                .any(|line| line.trim_end().chars().count() >= 10
                    && line.chars().all(|ch| ch == '─' || ch == ' ')
                    && line.contains("─")),
            "expected a plain rule row, got:\n{rendered}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn all_workspaces_agent_panel_entries_use_live_root_runtime_cwd_for_workspace_label() {
        let unique = format!(
            "herdr-agent-panel-runtime-cwd-{}-{}",
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

        let mut app = crate::app::state::AppState::test_new();
        let mut workspace = Workspace::test_new("stale-name");
        workspace.custom_name = None;
        workspace.identity_cwd = stale_cwd.clone();
        let pane = workspace.tabs[0].root_pane;

        app.workspaces = vec![workspace];
        app.ensure_test_terminals();
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane]
            .attached_terminal_id
            .clone();
        let terminal = app.terminals.get_mut(&terminal_id).unwrap();
        terminal.cwd = stale_cwd;
        terminal.detected_agent = Some(Agent::Pi);
        app.active = Some(0);
        app.selected = 0;

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

        let mut runtime_registry = TerminalRuntimeRegistry::new();
        runtime_registry.insert(terminal_id, runtime);
        let entries = agent_panel_entries_from(&app, &runtime_registry);
        let primary_label = entries[0].primary_label.clone();

        for (_, runtime) in runtime_registry.drain() {
            runtime.shutdown();
        }
        let _ = std::fs::remove_dir_all(root);

        assert_eq!(primary_label, "herdr");
    }

    #[test]
    fn all_workspaces_agent_panel_entries_prefer_agent_names_for_agent_identity() {
        let mut app = crate::app::state::AppState::test_new();
        let workspace = Workspace::test_new("bridge");
        let first_pane = workspace.tabs[0].root_pane;

        app.workspaces = vec![workspace];
        app.ensure_test_terminals();
        let first_terminal_id = app.workspaces[0].tabs[0].panes[&first_pane]
            .attached_terminal_id
            .clone();
        app.terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Pi);
        app.terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .set_agent_name("planner".into());
        app.active = Some(0);
        app.selected = 0;

        let entries = agent_panel_entries(&app);
        assert_eq!(entries[0].primary_label, "bridge");
        assert_eq!(entries[0].agent_label.as_deref(), Some("planner"));
    }

    #[test]
    fn expanded_sidebar_sections_handle_tiny_heights() {
        let (ws_area, detail_area) = expanded_sidebar_sections(Rect::new(0, 0, 20, 5), 0.9);

        assert_eq!(ws_area, Rect::new(0, 0, 19, 3));
        assert_eq!(detail_area, Rect::new(0, 3, 19, 2));
    }

    #[test]
    fn sidebar_section_divider_is_hidden_for_tiny_heights() {
        let divider = sidebar_section_divider_rect(Rect::new(0, 0, 20, 5), 0.5);

        assert_eq!(divider, Rect::default());
    }

    #[test]
    fn grouped_child_label_keeps_custom_workspace_name() {
        assert_eq!(
            grouped_child_display_label("renamed issue", Some("worktree/issue-137"), true),
            "renamed issue"
        );
    }

    #[test]
    fn grouped_child_label_uses_short_branch_for_auto_named_workspace() {
        assert_eq!(
            grouped_child_display_label("herdr-issue", Some("worktree/issue-137"), false),
            "issue-137"
        );
    }

    #[test]
    fn workspace_list_truncates_cjk_branch_without_panic() {
        let mut app = crate::app::state::AppState::test_new();
        let mut ws = Workspace::test_new("repo");
        ws.cached_git_branch = Some("feature/中文-分支-644".into());
        app.workspaces = vec![ws];
        app.active = Some(0);
        app.selected = 0;
        app.mode = Mode::Terminal;
        app.view.workspace_card_areas = vec![crate::app::state::WorkspaceCardArea {
            ws_idx: 0,
            rect: Rect::new(0, 1, 15, 2),
            indented: false,
        }];

        let mut terminal = Terminal::new(TestBackend::new(15, 6)).expect("test terminal");
        let runtimes = crate::terminal::TerminalRuntimeRegistry::new();

        terminal
            .draw(|frame| {
                render_workspace_list(&app, &runtimes, frame, Rect::new(0, 0, 15, 6), false)
            })
            .expect("workspace list should render");
    }

    fn workspace_with_worktree_space(
        name: &str,
        key: Option<&str>,
        checkout_key: &str,
    ) -> crate::workspace::Workspace {
        let mut ws = crate::workspace::Workspace::test_new(name);
        if let Some(key) = key {
            ws.worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
                key: key.into(),
                label: "herdr".into(),
                repo_root: std::path::PathBuf::from("/repo/herdr"),
                checkout_path: std::path::PathBuf::from(checkout_key),
                is_linked_worktree: name != "main",
            });
        }
        ws
    }

    fn workspace_with_git_space(name: &str, key: &str) -> crate::workspace::Workspace {
        let mut ws = crate::workspace::Workspace::test_new(name);
        ws.cached_git_space = Some(crate::workspace::GitSpaceMetadata {
            key: key.into(),
            checkout_key: format!("/repo/{name}"),
            label: "herdr".into(),
            repo_root: std::path::PathBuf::from(format!("/repo/{name}")),
            is_linked_worktree: false,
        });
        ws
    }

    #[test]
    fn parent_workspace_row_stays_clickable_when_grouped() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];

        let (cards, headers) = compute_workspace_list_areas(&app, Rect::new(0, 0, 30, 20));

        assert!(headers.is_empty());
        assert_eq!(cards[0].ws_idx, 0);
        assert!(!cards[0].indented);
        assert_eq!(cards[1].ws_idx, 1);
        assert!(cards[1].indented);
        assert_eq!(cards[1].rect.y, cards[0].rect.y + cards[0].rect.height + 1);
    }

    #[test]
    fn linked_only_worktree_members_do_not_form_parentless_group() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
            workspace_with_worktree_space("review", Some("repo-key"), "/repo/herdr-review"),
        ];

        let entries = workspace_list_entries(&app);

        assert_eq!(
            entries,
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false
                },
            ]
        );
    }

    #[test]
    fn compact_space_group_scroll_offset_can_start_inside_group() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("one", Some("repo-key"), "/repo/herdr-one"),
            workspace_with_worktree_space("two", Some("repo-key"), "/repo/herdr-two"),
        ];
        let area = Rect::new(0, 0, 30, 20);
        app.workspace_scroll = normalized_workspace_scroll(&app, area, 2);

        let (cards, headers) = compute_workspace_list_areas(&app, area);

        assert!(headers.is_empty());
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].ws_idx, 2);
    }

    #[test]
    fn workspace_scroll_metrics_count_display_entries_not_raw_workspaces() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
            Workspace::test_new("notes"),
        ];
        app.collapsed_space_keys.insert("repo-key".into());
        app.active = None;
        app.mode = Mode::Terminal;

        let ws_area = Rect::new(0, 0, 30, 6);
        let metrics = workspace_list_scroll_metrics(&app, ws_area);

        assert_eq!(metrics.viewport_rows, 1);
        assert_eq!(metrics.max_offset_from_bottom, 1);
        assert_eq!(metrics.offset_from_bottom, 1);
    }

    #[test]
    fn workspace_scroll_offset_applies_to_group_children() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
            Workspace::test_new("notes"),
        ];
        app.collapsed_space_keys.insert("repo-key".into());
        app.active = None;
        app.mode = Mode::Terminal;
        app.workspace_scroll = 1;

        let (cards, headers) = compute_workspace_list_areas(&app, Rect::new(0, 0, 30, 12));

        assert!(headers.is_empty());
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].ws_idx, 2);
    }

    #[test]
    fn workspace_list_entries_group_multiple_workspaces_in_same_git_space() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: true,
                },
            ]
        );
    }

    #[test]
    fn workspace_list_entries_group_non_contiguous_explicit_members() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_git_space("normal", "other-key"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 2,
                    indented: true,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false,
                },
            ]
        );
    }

    #[test]
    fn workspace_list_entries_do_not_group_normal_git_workspaces() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_git_space("one", "repo-key"),
            workspace_with_git_space("two", "repo-key"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false,
                },
            ]
        );
    }

    #[test]
    fn workspace_list_entries_do_not_auto_attach_normal_git_workspace_to_group() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_git_space("scratch", "repo-key"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 2,
                    indented: true,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false,
                },
            ]
        );
    }

    #[test]
    fn workspace_list_entries_leave_single_git_and_non_git_workspaces_flat() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_git_space("one", "repo-key"),
            workspace_with_worktree_space("notes", None, "/notes"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false,
                },
            ]
        );
    }

    #[test]
    fn collapsed_group_hides_inactive_children_but_keeps_active_visible() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];
        app.active = Some(1);
        app.mode = Mode::Terminal;
        app.collapsed_space_keys.insert("repo-key".into());

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: true,
                },
            ]
        );

        app.active = None;
        app.mode = Mode::Terminal;
        assert_eq!(
            workspace_list_entries(&app),
            vec![WorkspaceListEntry::Workspace {
                ws_idx: 0,
                indented: false,
            }]
        );
    }

    #[test]
    fn collapsed_group_keeps_selected_child_visible_in_navigate_mode() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];
        app.mode = Mode::Navigate;
        app.selected = 1;
        app.active = Some(1);
        app.collapsed_space_keys.insert("repo-key".into());

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: true,
                },
            ]
        );
    }
}
