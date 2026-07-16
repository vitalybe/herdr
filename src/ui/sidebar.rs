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
use crate::app::state::{
    AgentPanelSort, LineSplitId, ManualEntry, Palette, PaneManualEntry as ManualPaneEntry,
    SidebarSectionCollapse,
};
use crate::app::{AppState, Mode};
use crate::detect::AgentState;
use crate::terminal::TerminalRuntimeRegistry;

const WORKSPACE_SECTION_HEADER_ROWS: u16 = 2;
const AGENT_PANEL_HEADER_ROWS: u16 = 3;
/// Header rows above the Panes-section body: a divider rule plus the title.
const PANE_SECTION_HEADER_ROWS: u16 = 2;
/// Rows a collapsed sidebar band occupies: just its title/toggle row.
const COLLAPSED_SECTION_ROWS: u16 = 1;
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
    /// Tree nesting depth. Each level adds [`AGENT_TREE_INDENT`] leading columns.
    pub depth: usize,
    /// Collapse glyph state: `None` for leaves (a plain leading space),
    /// `Some(true)` for an expanded parent (`▾`), `Some(false)` for a collapsed
    /// parent (`▸`).
    pub expanded: Option<bool>,
    /// Aggregate of direct children by status key, ordered. Rendered on a third
    /// line for parents; empty for leaves.
    pub child_summary: &'a [(&'static str, usize)],
}

/// Columns of indentation added per tree depth level in the agents panel.
pub(crate) const AGENT_TREE_INDENT: usize = 2;

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

    /// Leading indentation span for this row's tree depth, or `None` at depth 0
    /// so root rows keep the exact historical span layout.
    fn indent_span(&self) -> Option<Span<'static>> {
        (self.depth > 0)
            .then(|| Span::styled(" ".repeat(self.depth * AGENT_TREE_INDENT), Style::default()))
    }

    /// The collapse/expand glyph column: `▾`/`▸` for a parent, else a plain
    /// leading space so leaves keep the historical single-space layout.
    fn glyph_span(&self, p: &Palette) -> Span<'static> {
        match self.expanded {
            Some(true) => Span::styled("▾".to_string(), Style::default().fg(p.accent)),
            Some(false) => Span::styled("▸".to_string(), Style::default().fg(p.accent)),
            None => Span::styled(" ".to_string(), Style::default()),
        }
    }

    /// First line: indent, collapse glyph (or space), status icon, then the tab
    /// name.
    fn line_one(&self, p: &Palette, max_width: usize) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        spans.extend(self.indent_span());
        spans.push(self.glyph_span(p));
        spans.push(Span::styled(self.icon.to_string(), self.icon_style));
        if let Some(tab) = self.tab.filter(|tab| !tab.is_empty()) {
            spans.push(Span::styled(format!(" {tab}"), self.name_style(p)));
        }
        truncate_agent_row_spans(spans, max_width)
    }

    /// Second line: indent, leading space, dark-grey space name, then ` · status`.
    fn line_two(&self, max_width: usize) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        spans.extend(self.indent_span());
        spans.push(Span::styled(" ".to_string(), Style::default()));
        spans.push(Span::styled(
            self.space.to_string(),
            Style::default().fg(agent_row_space_color()),
        ));
        if !self.status.is_empty() {
            spans.push(Span::styled(
                format!(" · {}", self.status),
                self.status_style(),
            ));
        }
        truncate_agent_row_spans(spans, max_width)
    }

    /// Third line (parents only): indent, leading space, then a muted
    /// `"Subagents - <count> <state> ..."` summary of direct children.
    fn line_three(&self, p: &Palette, max_width: usize) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        spans.extend(self.indent_span());
        spans.push(Span::styled(
            format!(" {}", format_subagent_summary(self.child_summary)),
            Style::default().fg(p.overlay0).add_modifier(Modifier::DIM),
        ));
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
    /// Live pane id of this agent's parent, resolved from its stable parent
    /// link. `None` for roots or when the parent no longer exists. Used to build
    /// the agents-panel tree.
    pub parent_pane: Option<crate::layout::PaneId>,
    /// Tree nesting depth (0 for roots), assigned during tree flattening.
    pub depth: usize,
    /// True when this agent has at least one direct child in the panel.
    pub has_children: bool,
    /// True when this parent is collapsed (its descendants are hidden).
    pub collapsed: bool,
    /// Aggregate of direct children by status key, ordered for display. Only
    /// populated for parents (`has_children`).
    pub child_summary: Vec<(&'static str, usize)>,
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
            // Parent agents (those with children) render a third summary line.
            AgentPanelRow::Agent(entry) if entry.has_children => 3,
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
    collapse: SidebarSectionCollapse,
) -> (Rect, Rect, Rect) {
    // Panes collapse only matters when the Panes band is present at all.
    let pane_collapsed = collapse.panes && show_pane_section;
    if !collapse.spaces && !pane_collapsed && !collapse.agents {
        return expanded_sidebar_sections3_uncollapsed(
            area,
            spaces_ratio,
            pane_section_ratio,
            show_pane_section,
        );
    }
    collapsible_sidebar_sections3(
        area,
        spaces_ratio,
        pane_section_ratio,
        show_pane_section,
        collapse,
    )
}

/// The historical (no-band-collapsed) three-band split. Kept as the fast path so
/// the default geometry is byte-for-byte identical to before section collapse.
fn expanded_sidebar_sections3_uncollapsed(
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

/// Three-band split when at least one band is collapsed. Each collapsed present
/// band takes a single header row; the remaining height is shared between the
/// still-expanded bands using the same ratio math as the uncollapsed layout.
fn collapsible_sidebar_sections3(
    area: Rect,
    spaces_ratio: f32,
    pane_section_ratio: f32,
    show_pane_section: bool,
    collapse: SidebarSectionCollapse,
) -> (Rect, Rect, Rect) {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.width == 0 || content.height == 0 {
        return (Rect::default(), Rect::default(), Rect::default());
    }

    let cs = collapse.spaces;
    let cp = collapse.panes && show_pane_section;
    let ca = collapse.agents;
    let h = COLLAPSED_SECTION_ROWS;

    let mut reserved = 0u16;
    if cs {
        reserved = reserved.saturating_add(h);
    }
    if cp {
        reserved = reserved.saturating_add(h);
    }
    if ca {
        reserved = reserved.saturating_add(h);
    }
    let remaining = content.height.saturating_sub(reserved);

    let s_exp = !cs;
    let p_exp = show_pane_section && !cp;
    let a_exp = !ca;

    let (mut hs, mut hp, mut ha) = (
        if cs { h } else { 0 },
        if cp { h } else { 0 },
        if ca { h } else { 0 },
    );
    match (s_exp, p_exp, a_exp) {
        // A single expanded band takes all the remaining height.
        (true, false, false) => hs = remaining,
        (false, true, false) => hp = remaining,
        (false, false, true) => ha = remaining,
        // Two expanded bands split the remaining height by the ratio that
        // separated them in the uncollapsed layout.
        (true, true, false) => {
            let (a, b) = sidebar_section_heights(remaining, spaces_ratio);
            hs = a;
            hp = b;
        }
        (true, false, true) => {
            let (a, b) = sidebar_section_heights(remaining, spaces_ratio);
            hs = a;
            ha = b;
        }
        (false, true, true) => {
            let (a, b) = sidebar_section_heights(remaining, pane_section_ratio);
            hp = a;
            ha = b;
        }
        // No band collapsed is handled by the fast path; all bands collapsed
        // leaves the freed height unused below the stacked headers.
        (true, true, true) | (false, false, false) => {}
    }

    let x = content.x;
    let w = content.width;
    let mut y = content.y;
    let spaces_area = Rect::new(x, y, w, hs);
    y = y.saturating_add(hs);
    let pane_section_area = if show_pane_section {
        let r = Rect::new(x, y, w, hp);
        y = y.saturating_add(hp);
        r
    } else {
        Rect::new(x, y, w, 0)
    };
    let agents_area = Rect::new(x, y, w, ha);
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
    collapse: SidebarSectionCollapse,
) -> Rect {
    let (_, _, agents_area) = expanded_sidebar_sections3(
        area,
        spaces_ratio,
        pane_section_ratio,
        show_pane_section,
        collapse,
    );
    agents_area
}

/// The Panes band as the middle of three stacked sidebar sections.
pub(crate) fn pane_section_rect(
    area: Rect,
    spaces_ratio: f32,
    pane_section_ratio: f32,
    show_pane_section: bool,
    collapse: SidebarSectionCollapse,
) -> Rect {
    let (_, pane_section_area, _) = expanded_sidebar_sections3(
        area,
        spaces_ratio,
        pane_section_ratio,
        show_pane_section,
        collapse,
    );
    pane_section_area
}

/// One of the three stacked sidebar bands, used to place and hit-test the
/// per-band collapse/expand toggle in the band header.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SidebarBand {
    Spaces,
    Panes,
    Agents,
}

impl SidebarBand {
    fn title(self) -> &'static str {
        match self {
            SidebarBand::Spaces => "spaces",
            SidebarBand::Panes => "panes",
            SidebarBand::Agents => "agents",
        }
    }
}

/// Expand/collapse glyph shown before a band title (`▾` expanded, `▸` collapsed).
fn section_toggle_glyph(collapsed: bool) -> &'static str {
    if collapsed {
        "▸"
    } else {
        "▾"
    }
}

/// The clickable region of a band's collapse/expand toggle: the glyph plus the
/// title word on the header's title row. Spaces titles sit on the band's first
/// row; the Panes/Agents titles sit one row below their divider rule (or on the
/// first row when the band is collapsed to a single row).
pub(crate) fn sidebar_section_header_toggle_rect(
    area: Rect,
    band: SidebarBand,
    collapsed: bool,
) -> Rect {
    if area.width == 0 || area.height == 0 {
        return Rect::default();
    }
    let title_y = if collapsed {
        area.y
    } else {
        match band {
            SidebarBand::Spaces => area.y,
            SidebarBand::Panes | SidebarBand::Agents => area.y.saturating_add(1),
        }
    };
    if title_y >= area.y.saturating_add(area.height) {
        return Rect::default();
    }
    // "▾ " (glyph + space) plus the title word.
    let width = (2 + band.title().chars().count() as u16).min(area.width);
    Rect::new(area.x, title_y, width, 1)
}

/// Render a collapsed band as a single header row: `▸ title ─────`, the trailing
/// rule doubling as the separator to the band below.
fn render_collapsed_section_header(frame: &mut Frame, area: Rect, band: SidebarBand, p: &Palette) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let prefix = format!("{} {} ", section_toggle_glyph(true), band.title());
    let used = display_width_u16(&prefix);
    let mut spans = vec![Span::styled(
        prefix,
        Style::default().fg(p.overlay0).add_modifier(Modifier::BOLD),
    )];
    if area.width > used {
        let rule = "─".repeat((area.width - used) as usize);
        spans.push(Span::styled(rule, Style::default().fg(p.surface_dim)));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(area.x, area.y, area.width, 1),
    );
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

/// A single visible row in the Panes section: either a non-agent pane entry or a
/// named line-split divider. Mirrors [`AgentPanelRow`]. Client-only presentation
/// state.
pub(crate) enum PaneSectionRow {
    Pane(PaneSectionEntry),
    LineSplit {
        order_idx: usize,
        id: LineSplitId,
        name: String,
    },
}

impl PaneSectionRow {
    /// Content-row height of this row (excluding the trailing gap). Pane rows
    /// render two lines; line-splits are a single rule.
    fn content_height(&self) -> u16 {
        match self {
            PaneSectionRow::Pane(_) => PANE_SECTION_ROW_HEIGHT,
            PaneSectionRow::LineSplit { .. } => 1,
        }
    }

    /// Flat index of this row into `PaneSectionOrder::order`.
    fn order_idx(&self) -> usize {
        match self {
            PaneSectionRow::Pane(entry) => entry.order_idx,
            PaneSectionRow::LineSplit { order_idx, .. } => *order_idx,
        }
    }
}

/// Full ordered list of visible Panes-section rows, walking the client-only
/// order and interleaving panes and line-splits. Pane entries whose pane no
/// longer resolves are skipped; line-splits are always kept.
pub(crate) fn sidebar_pane_section_rows(app: &AppState) -> Vec<PaneSectionRow> {
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
        .filter_map(|(order_idx, entry)| match entry {
            ManualPaneEntry::Pane(pane_ref) => lookup
                .get(&(pane_ref.workspace_id.as_str(), pane_ref.pane_number))
                .map(|&(ws_idx, tab_idx, pane_id)| {
                    PaneSectionRow::Pane(PaneSectionEntry {
                        order_idx,
                        ws_idx,
                        tab_idx,
                        pane_id,
                    })
                }),
            ManualPaneEntry::LineSplit { id, name } => Some(PaneSectionRow::LineSplit {
                order_idx,
                id: *id,
                name: name.clone(),
            }),
        })
        .collect()
}

/// All non-agent panes across every workspace, ordered by the client-only Panes
/// section ordering. Line-splits are excluded, so this is the pane-only view used
/// for focus/enumeration (keyboard navigation and scroll targeting skip splits).
pub(crate) fn sidebar_pane_section_entries(app: &AppState) -> Vec<PaneSectionEntry> {
    sidebar_pane_section_rows(app)
        .into_iter()
        .filter_map(|row| match row {
            PaneSectionRow::Pane(entry) => Some(entry),
            PaneSectionRow::LineSplit { .. } => None,
        })
        .collect()
}

/// Row index (in the full [`sidebar_pane_section_rows`] list) of the pane row for
/// `pane_id`, if present. Mirrors [`agent_panel_row_index_of_pane`].
pub(crate) fn pane_section_row_index_of_pane(
    app: &AppState,
    pane_id: crate::layout::PaneId,
) -> Option<usize> {
    sidebar_pane_section_rows(app)
        .iter()
        .position(|row| matches!(row, PaneSectionRow::Pane(entry) if entry.pane_id == pane_id))
}

/// Visible-row layout for the Panes section, walking rows from `scroll` and
/// laying out variable-height rows (with a one-row gap) inside `body`. Pure; does
/// not consult scroll metrics, so it is safe to call from the metrics path.
fn pane_section_row_areas_in(
    app: &AppState,
    body: Rect,
    scroll: usize,
) -> Vec<crate::app::state::PaneSectionRowArea> {
    use crate::app::state::PaneSectionRowContent;
    let mut areas = Vec::new();
    if body.width == 0 || body.height == 0 {
        return areas;
    }
    let body_bottom = body.y + body.height;
    let mut row_y = body.y;
    for row in sidebar_pane_section_rows(app).into_iter().skip(scroll) {
        let height = row.content_height();
        if row_y.saturating_add(height) > body_bottom {
            break;
        }
        let content = match &row {
            PaneSectionRow::Pane(entry) => PaneSectionRowContent::Pane {
                ws_idx: entry.ws_idx,
                tab_idx: entry.tab_idx,
                pane_id: entry.pane_id,
            },
            PaneSectionRow::LineSplit { id, .. } => PaneSectionRowContent::LineSplit { id: *id },
        };
        areas.push(crate::app::state::PaneSectionRowArea {
            order_idx: row.order_idx(),
            content,
            rect: Rect::new(body.x, row_y, body.width, height),
        });
        row_y = row_y.saturating_add(height);
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
    let total_rows = sidebar_pane_section_rows(app).len();
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

const PANE_SECTION_SPLIT_LABEL: &str = "+ split";

/// Mouse-first "+ split" affordance rect for the Panes section, right-aligned on
/// the "panes" header title row (mirrors the agents-panel affordance). Returns
/// the empty rect when there is no room.
pub(crate) fn pane_section_split_button_rect(area: Rect) -> Rect {
    if area.width == 0 || area.height < 2 {
        return Rect::default();
    }
    let width = display_width_u16(PANE_SECTION_SPLIT_LABEL);
    if width == 0 || width >= area.width {
        return Rect::default();
    }
    let x = area.x + area.width.saturating_sub(width);
    Rect::new(x, area.y + 1, width, 1)
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
                .map(move |detail| {
                    let parent_pane = ws
                        .pane_state(detail.pane_id)
                        .and_then(|pane| pane.parent.as_ref())
                        .and_then(|parent| app.resolve_pane_parent(parent))
                        .map(|(_, parent_pane_id)| parent_pane_id);
                    AgentPanelEntry {
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
                        parent_pane,
                        depth: 0,
                        has_children: false,
                        collapsed: false,
                        child_summary: Vec::new(),
                    }
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
/// the order fall back to the end. The base ordering is then re-grouped into a
/// parent/child tree so children nest under their parent regardless of sort.
/// Pure.
fn agent_panel_rows_with_runtimes(
    app: &AppState,
    terminal_runtimes: Option<&TerminalRuntimeRegistry>,
) -> Vec<AgentPanelRow> {
    let base = base_agent_panel_rows(app, terminal_runtimes);
    flatten_agent_tree(app, base)
}

/// The flat, sort-driven base ordering of agent rows, before tree grouping.
fn base_agent_panel_rows(
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

/// Ordered status keys for the parent "Subagents" summary line. Groups appear in
/// this order and zero-count groups are omitted.
const SUBAGENT_SUMMARY_ORDER: [&str; 5] = ["working", "blocked", "idle", "done", "unknown"];

/// Aggregate direct-child agents by base status key, in [`SUBAGENT_SUMMARY_ORDER`],
/// omitting zero counts. Custom state labels collapse to their base status key
/// (the `(state, seen)` pair maps through [`agent_panel_status_key`]).
fn subagent_summary(
    children: impl IntoIterator<Item = (AgentState, bool)>,
) -> Vec<(&'static str, usize)> {
    let mut counts: std::collections::HashMap<&'static str, usize> =
        std::collections::HashMap::new();
    for (state, seen) in children {
        *counts
            .entry(agent_panel_status_key(state, seen))
            .or_insert(0) += 1;
    }
    SUBAGENT_SUMMARY_ORDER
        .iter()
        .filter_map(|key| counts.get(key).map(|count| (*key, *count)))
        .filter(|(_, count)| *count > 0)
        .collect()
}

/// Format the parent summary line body (without the leading indent), e.g.
/// `"Subagents - 2 idle 3 blocked"`.
fn format_subagent_summary(summary: &[(&'static str, usize)]) -> String {
    let groups = summary
        .iter()
        .map(|(key, count)| format!("{count} {key}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!("Subagents - {groups}")
}

/// Re-group the flat base rows into a parent/child tree. Children (agents whose
/// resolved parent is another agent in the list) are emitted nested under their
/// parent, one indent level per depth. Line-splits and root agents keep their
/// base order. Descendants of a collapsed parent are omitted. Cycles and
/// orphaned children are handled defensively so every agent appears exactly
/// once.
fn flatten_agent_tree(app: &AppState, base: Vec<AgentPanelRow>) -> Vec<AgentPanelRow> {
    let len = base.len();
    // pane_id -> base index for agent rows.
    let mut index_of: std::collections::HashMap<crate::layout::PaneId, usize> =
        std::collections::HashMap::new();
    for (i, row) in base.iter().enumerate() {
        if let AgentPanelRow::Agent(entry) = row {
            index_of.insert(entry.pane_id, i);
        }
    }

    // Resolve each agent's parent to a base index (when the parent is itself an
    // agent in the list), building the child lists in base order.
    let mut parent_index: Vec<Option<usize>> = vec![None; len];
    let mut children: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    // Cached (state, seen) per index for summary aggregation.
    let mut meta: Vec<Option<(AgentState, bool)>> = vec![None; len];
    // Collapsed flag per index, keyed by the agent's stable public pane id.
    let mut collapsed_flags: Vec<bool> = vec![false; len];
    for (i, row) in base.iter().enumerate() {
        if let AgentPanelRow::Agent(entry) = row {
            meta[i] = Some((entry.state, entry.seen));
            if let Some(ws) = app.workspaces.get(entry.ws_idx) {
                if let Some(number) = ws.public_pane_number(entry.pane_id) {
                    let key = crate::workspace::public_pane_id_for_number(&ws.id, number);
                    collapsed_flags[i] = app.collapsed_agent_keys.contains(&key);
                }
            }
            if let Some(parent_pane) = entry.parent_pane {
                if let Some(&pidx) = index_of.get(&parent_pane) {
                    if pidx != i {
                        parent_index[i] = Some(pidx);
                        children.entry(pidx).or_default().push(i);
                    }
                }
            }
        }
    }

    let mut slots: Vec<Option<AgentPanelRow>> = base.into_iter().map(Some).collect();
    let mut visited = vec![false; len];
    let mut out = Vec::with_capacity(len);

    for i in 0..len {
        if visited[i] {
            continue;
        }
        match &slots[i] {
            Some(AgentPanelRow::LineSplit { .. }) => {
                visited[i] = true;
                if let Some(row) = slots[i].take() {
                    out.push(row);
                }
            }
            Some(AgentPanelRow::Agent(_)) if parent_index[i].is_none() => {
                emit_agent_subtree(
                    i,
                    0,
                    &children,
                    &collapsed_flags,
                    &meta,
                    &mut slots,
                    &mut visited,
                    &mut out,
                );
            }
            _ => {}
        }
    }
    // Defensive: emit any agent not reachable from a root (e.g. a parent cycle).
    for i in 0..len {
        if !visited[i] && matches!(slots[i], Some(AgentPanelRow::Agent(_))) {
            emit_agent_subtree(
                i,
                0,
                &children,
                &collapsed_flags,
                &meta,
                &mut slots,
                &mut visited,
                &mut out,
            );
        }
    }

    out
}

#[allow(clippy::too_many_arguments)]
fn emit_agent_subtree(
    i: usize,
    depth: usize,
    children: &std::collections::HashMap<usize, Vec<usize>>,
    collapsed_flags: &[bool],
    meta: &[Option<(AgentState, bool)>],
    slots: &mut [Option<AgentPanelRow>],
    visited: &mut [bool],
    out: &mut Vec<AgentPanelRow>,
) {
    if visited[i] {
        return;
    }
    visited[i] = true;
    let kids = children.get(&i);
    let has_children = kids.is_some_and(|kids| !kids.is_empty());
    let collapsed = collapsed_flags[i];
    let summary = if has_children {
        let child_metas = kids
            .into_iter()
            .flatten()
            .filter_map(|&c| meta[c])
            .collect::<Vec<_>>();
        subagent_summary(child_metas)
    } else {
        Vec::new()
    };

    if let Some(AgentPanelRow::Agent(mut entry)) = slots[i].take() {
        entry.depth = depth;
        entry.has_children = has_children;
        entry.collapsed = collapsed;
        entry.child_summary = summary;
        out.push(AgentPanelRow::Agent(entry));
    }

    if let Some(kids) = kids {
        if collapsed {
            // Descendants stay hidden but must be marked visited so the
            // orphan-recovery pass does not re-surface them as roots.
            for &child in kids {
                mark_subtree_visited(child, children, visited);
            }
        } else {
            for &child in kids {
                emit_agent_subtree(
                    child,
                    depth + 1,
                    children,
                    collapsed_flags,
                    meta,
                    slots,
                    visited,
                    out,
                );
            }
        }
    }
}

/// Mark an agent subtree visited without emitting it (used for collapsed
/// parents). Guards against cycles via the shared `visited` flags.
fn mark_subtree_visited(
    i: usize,
    children: &std::collections::HashMap<usize, Vec<usize>>,
    visited: &mut [bool],
) {
    if visited[i] {
        return;
    }
    visited[i] = true;
    if let Some(kids) = children.get(&i) {
        for &child in kids {
            mark_subtree_visited(child, children, visited);
        }
    }
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
    let ws_area = workspace_list_rect(
        area,
        app.sidebar_section_split,
        app.sidebar_pane_section_split,
        sidebar_shows_pane_section(app),
        app.sidebar_section_collapse(),
    );
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

pub(crate) fn workspace_list_rect(
    area: Rect,
    spaces_ratio: f32,
    pane_section_ratio: f32,
    show_pane_section: bool,
    collapse: SidebarSectionCollapse,
) -> Rect {
    let (ws_area, _, _) = expanded_sidebar_sections3(
        area,
        spaces_ratio,
        pane_section_ratio,
        show_pane_section,
        collapse,
    );
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
    let ws_area = workspace_list_rect(
        area,
        app.sidebar_section_split,
        app.sidebar_pane_section_split,
        sidebar_shows_pane_section(app),
        app.sidebar_section_collapse(),
    );
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

    let collapse = app.sidebar_section_collapse();
    let show_pane_section = sidebar_shows_pane_section(app);
    let (ws_area, pane_section_area, detail_area) = expanded_sidebar_sections3(
        area,
        app.sidebar_section_split,
        app.sidebar_pane_section_split,
        show_pane_section,
        collapse,
    );

    if collapse.spaces {
        render_collapsed_section_header(frame, ws_area, SidebarBand::Spaces, p);
    } else {
        render_workspace_list(app, terminal_runtimes, frame, ws_area, is_navigating);
    }
    if show_pane_section {
        if collapse.panes {
            render_collapsed_section_header(frame, pane_section_area, SidebarBand::Panes, p);
        } else {
            render_pane_section(app, terminal_runtimes, frame, pane_section_area);
        }
    }
    if collapse.agents {
        render_collapsed_section_header(frame, detail_area, SidebarBand::Agents, p);
    } else {
        render_agent_detail(app, terminal_runtimes, frame, detail_area);
    }
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
    let tab_name = ws
        .tab_display_name(tab_idx)
        .unwrap_or_else(|| (tab_idx + 1).to_string());
    let pane_name = ws.pane_state(row_pane_id).and_then(|pane| {
        app.terminals
            .get(&pane.attached_terminal_id)
            .and_then(|terminal| terminal.border_label(false))
    });
    // When the pane has its own name, show both "<pane> • <tab>"; otherwise the
    // tab name stands alone.
    match pane_name {
        Some(pane_name) => format!("{pane_name} • {tab_name}"),
        None => tab_name,
    }
}

/// Render the Panes section: every non-agent pane across all spaces as a
/// two-line row (pane name over its space name) interleaved with named
/// line-split dividers, ordered by the client-only Panes-section order, with a
/// drop indicator during a reorder drag.
fn render_pane_section(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    area: Rect,
) {
    use crate::app::state::{PaneManualEntryRef, PaneSectionRowContent};
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
            format!("{} panes", section_toggle_glyph(false)),
            Style::default().fg(p.overlay0).add_modifier(Modifier::BOLD),
        )])),
        Rect::new(area.x, area.y + 1, area.width, 1),
    );
    if app.mouse_capture {
        let split_rect = pane_section_split_button_rect(area);
        if split_rect != Rect::default() {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    PANE_SECTION_SPLIT_LABEL,
                    Style::default().fg(p.overlay0),
                )),
                split_rect,
            );
        }
    }

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

    // Line-split names live in the flat order; index them by order slot so the
    // (name-less, Copy) row areas can render their label.
    let split_names: std::collections::HashMap<usize, String> = sidebar_pane_section_rows(app)
        .into_iter()
        .filter_map(|row| match row {
            PaneSectionRow::LineSplit {
                order_idx, name, ..
            } => Some((order_idx, name)),
            PaneSectionRow::Pane(_) => None,
        })
        .collect();

    let areas = &app.view.pane_section_row_areas;
    let max_width = body.width as usize;
    for row in areas {
        match row.content {
            PaneSectionRowContent::Pane {
                ws_idx,
                tab_idx,
                pane_id,
            } => {
                let ws = &app.workspaces[ws_idx];
                let is_active = Some(ws_idx) == app.active
                    && ws.active_tab == tab_idx
                    && ws.focused_pane_id() == Some(pane_id);
                let is_dragged = matches!(
                    &dragged,
                    Some(PaneManualEntryRef::Pane(source))
                        if source.workspace_id == ws.id
                            && ws.public_pane_number(pane_id) == Some(source.pane_number)
                );

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
                let pane_name = pane_section_row_name(app, ws, pane_id, tab_idx);
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
            PaneSectionRowContent::LineSplit { id } => {
                let is_dragged = matches!(&dragged, Some(PaneManualEntryRef::LineSplit(source)) if *source == id);
                if is_dragged {
                    let buf = frame.buffer_mut();
                    for x in row.rect.x..row.rect.x + row.rect.width {
                        buf[(x, row.rect.y)].set_style(Style::default().bg(p.surface1));
                    }
                }
                let name = split_names
                    .get(&row.order_idx)
                    .map(String::as_str)
                    .unwrap_or("");
                render_line_split_row(frame, body, row.rect.y, name, p);
            }
        }
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
                format!("{} spaces", section_toggle_glyph(false)),
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
            format!("{} agents", section_toggle_glyph(false)),
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
                    depth: detail.depth,
                    expanded: detail.has_children.then_some(!detail.collapsed),
                    child_summary: &detail.child_summary,
                };

                let mut lines = vec![row_ctx.line_one(p, max_width), row_ctx.line_two(max_width)];
                if detail.has_children {
                    lines.push(row_ctx.line_three(p, max_width));
                }
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
            depth: 0,
            expanded: None,
            child_summary: &[],
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

    fn set_pane_state(
        app: &mut AppState,
        pane: crate::layout::PaneId,
        state: AgentState,
        seen: bool,
    ) {
        let terminal_id = app.workspaces[0]
            .pane_state(pane)
            .expect("pane exists")
            .attached_terminal_id
            .clone();
        app.terminals.get_mut(&terminal_id).expect("terminal").state = state;
        app.workspaces[0].pane_state_mut(pane).expect("pane").seen = seen;
    }

    /// Link `child`'s pane to `parent` by the stable (workspace id + public pane
    /// number) reference, exactly as `agent start --parent` does.
    fn link_parent(
        app: &mut AppState,
        parent: crate::layout::PaneId,
        child: crate::layout::PaneId,
    ) {
        let ws = &mut app.workspaces[0];
        let number = ws.public_pane_number(parent).expect("parent has number");
        let workspace_id = ws.id.clone();
        ws.pane_state_mut(child).expect("child pane").parent = Some(crate::pane::PaneParentRef {
            workspace_id,
            pane_number: number,
        });
    }

    #[test]
    fn subagent_summary_orders_states_and_omits_zero_counts() {
        // working, blocked, idle, done, unknown ordering; zero-count groups drop.
        let children = vec![
            (AgentState::Blocked, true),
            (AgentState::Idle, true),  // idle
            (AgentState::Idle, false), // done
            (AgentState::Working, true),
            (AgentState::Idle, true), // idle
        ];
        let summary = subagent_summary(children);
        assert_eq!(
            summary,
            vec![("working", 1), ("blocked", 1), ("idle", 2), ("done", 1)]
        );
        assert_eq!(
            format_subagent_summary(&summary),
            "Subagents - 1 working 1 blocked 2 idle 1 done"
        );
    }

    #[test]
    fn subagent_summary_single_group() {
        let summary = subagent_summary(vec![
            (AgentState::Blocked, true),
            (AgentState::Blocked, true),
        ]);
        assert_eq!(summary, vec![("blocked", 2)]);
        assert_eq!(format_subagent_summary(&summary), "Subagents - 2 blocked");
    }

    /// One workspace, one tab, parent pane with two child agents linked to it.
    fn app_with_agent_tree() -> (
        AppState,
        crate::layout::PaneId,
        crate::layout::PaneId,
        crate::layout::PaneId,
    ) {
        let mut app = AppState::test_new();
        let mut ws = Workspace::test_new("one");
        ws.active_tab = 0;
        let parent = ws.tabs[0].root_pane;
        let child_a = ws.test_split(ratatui::layout::Direction::Horizontal);
        let child_b = ws.test_split(ratatui::layout::Direction::Vertical);
        app.workspaces = vec![ws];
        app.ensure_test_terminals();
        mark_pane_agent(&mut app, parent);
        mark_pane_agent(&mut app, child_a);
        mark_pane_agent(&mut app, child_b);
        link_parent(&mut app, parent, child_a);
        link_parent(&mut app, parent, child_b);
        set_pane_state(&mut app, parent, AgentState::Working, true);
        set_pane_state(&mut app, child_a, AgentState::Idle, true);
        set_pane_state(&mut app, child_b, AgentState::Blocked, true);
        app.active = Some(0);
        app.selected = 0;
        (app, parent, child_a, child_b)
    }

    fn parent_pane_key(app: &AppState, pane: crate::layout::PaneId) -> String {
        let ws = &app.workspaces[0];
        crate::workspace::public_pane_id_for_number(&ws.id, ws.public_pane_number(pane).unwrap())
    }

    #[test]
    fn agent_tree_nests_children_under_parent_with_depth_and_summary() {
        let (app, parent, child_a, child_b) = app_with_agent_tree();
        let rows = agent_panel_rows(&app);
        assert_eq!(rows.len(), 3);
        let agents: Vec<&AgentPanelEntry> = rows
            .iter()
            .filter_map(|row| match row {
                AgentPanelRow::Agent(entry) => Some(entry),
                _ => None,
            })
            .collect();
        // Parent first at depth 0, then its two children at depth 1.
        assert_eq!(agents[0].pane_id, parent);
        assert_eq!(agents[0].depth, 0);
        assert!(agents[0].has_children);
        assert!(!agents[0].collapsed);
        assert_eq!(agents[1].pane_id, child_a);
        assert_eq!(agents[1].depth, 1);
        assert!(!agents[1].has_children);
        assert_eq!(agents[2].pane_id, child_b);
        assert_eq!(agents[2].depth, 1);
        // Parent row spans three lines; leaves span two.
        assert_eq!(rows[0].content_height(), 3);
        assert_eq!(rows[1].content_height(), 2);
        // Direct-child aggregate: one idle, one blocked.
        assert_eq!(agents[0].child_summary, vec![("blocked", 1), ("idle", 1)]);
    }

    #[test]
    fn collapsing_parent_hides_descendants_but_keeps_parent_and_summary() {
        let (mut app, parent, _child_a, _child_b) = app_with_agent_tree();
        app.collapsed_agent_keys
            .insert(parent_pane_key(&app, parent));
        let rows = agent_panel_rows(&app);
        assert_eq!(rows.len(), 1, "descendants hidden when parent collapsed");
        let AgentPanelRow::Agent(entry) = &rows[0] else {
            panic!("expected agent row");
        };
        assert_eq!(entry.pane_id, parent);
        assert!(entry.collapsed);
        assert!(entry.has_children);
        // Third summary line still present while collapsed.
        assert_eq!(rows[0].content_height(), 3);
        assert_eq!(entry.child_summary, vec![("blocked", 1), ("idle", 1)]);
    }

    #[test]
    fn agent_with_unresolved_parent_renders_as_root() {
        // A child whose parent link no longer resolves (parent closed) becomes a
        // root rather than disappearing.
        let (mut app, _parent, child_a, _child_b) = app_with_agent_tree();
        app.workspaces[0].pane_state_mut(child_a).unwrap().parent =
            Some(crate::pane::PaneParentRef {
                workspace_id: app.workspaces[0].id.clone(),
                pane_number: 9999,
            });
        let rows = agent_panel_rows(&app);
        let child_row = rows
            .iter()
            .find_map(|row| match row {
                AgentPanelRow::Agent(entry) if entry.pane_id == child_a => Some(entry),
                _ => None,
            })
            .expect("child still present");
        assert_eq!(child_row.depth, 0, "orphaned child is a root");
    }

    #[test]
    fn resolve_pane_parent_uses_stable_number_not_pane_id() {
        let (app, parent, child_a, _child_b) = app_with_agent_tree();
        let parent_ref = app.workspaces[0]
            .pane_state(child_a)
            .unwrap()
            .parent
            .clone()
            .unwrap();
        let (ws_idx, resolved) = app.resolve_pane_parent(&parent_ref).expect("resolves");
        assert_eq!(ws_idx, 0);
        assert_eq!(resolved, parent);
        // A ref to a missing public number does not resolve.
        let missing = crate::pane::PaneParentRef {
            workspace_id: app.workspaces[0].id.clone(),
            pane_number: 4242,
        };
        assert!(app.resolve_pane_parent(&missing).is_none());
    }

    #[test]
    fn collapsed_parent_variable_height_geometry() {
        let (app, _parent, _child_a, _child_b) = app_with_agent_tree();
        let rows = agent_panel_rows(&app);
        let body = Rect::new(0, 0, 25, 30);
        let areas = compute_agent_panel_row_areas(&rows, body, 0);
        assert_eq!(areas.len(), 3);
        // Parent occupies 3 lines starting at body top; a one-row gap follows.
        assert_eq!(areas[0].y, 0);
        assert_eq!(areas[0].height, 3);
        assert_eq!(areas[1].y, 4); // 3 lines + 1 gap
        assert_eq!(areas[1].height, 2);
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
        let (spaces, tabs, agents) =
            expanded_sidebar_sections3(area, 0.5, 0.5, true, SidebarSectionCollapse::default());
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
        let (spaces2, tabs2, agents2) =
            expanded_sidebar_sections3(area, 0.5, 0.5, false, SidebarSectionCollapse::default());
        assert_eq!(tabs2.height, 0);
        assert_eq!(spaces2, spaces);
        assert_eq!(agents2, expanded_sidebar_sections(area, 0.5).1);
    }

    #[test]
    fn collapsing_a_band_reserves_one_row_and_grows_the_others() {
        let area = Rect::new(0, 0, 26, 40);
        let collapse = SidebarSectionCollapse {
            spaces: true,
            panes: false,
            agents: false,
        };
        let (spaces, panes, agents) = expanded_sidebar_sections3(area, 0.5, 0.5, true, collapse);
        assert_eq!(spaces.height, COLLAPSED_SECTION_ROWS);
        assert_eq!(spaces.y, 0);
        assert_eq!(panes.y, spaces.y + spaces.height);
        assert_eq!(agents.y, panes.y + panes.height);
        assert_eq!(agents.y + agents.height, 40, "bands cover the full height");
        assert!(
            panes.height > 3 && agents.height > 3,
            "freed rows go to the expanded bands"
        );
    }

    #[test]
    fn collapsing_every_band_stacks_single_header_rows() {
        let area = Rect::new(0, 0, 26, 40);
        let collapse = SidebarSectionCollapse {
            spaces: true,
            panes: true,
            agents: true,
        };
        let (spaces, panes, agents) = expanded_sidebar_sections3(area, 0.5, 0.5, true, collapse);
        assert_eq!((spaces.height, panes.height, agents.height), (1, 1, 1));
        assert_eq!(spaces.y, 0);
        assert_eq!(panes.y, 1);
        assert_eq!(agents.y, 2);
    }

    #[test]
    fn pane_collapse_is_ignored_without_a_pane_band() {
        let area = Rect::new(0, 0, 26, 40);
        let with_flag = expanded_sidebar_sections3(
            area,
            0.5,
            0.5,
            false,
            SidebarSectionCollapse {
                spaces: false,
                panes: true,
                agents: false,
            },
        );
        let baseline =
            expanded_sidebar_sections3(area, 0.5, 0.5, false, SidebarSectionCollapse::default());
        assert_eq!(with_flag, baseline);
    }

    #[test]
    fn section_header_toggle_rect_lands_on_the_title_row() {
        let area = Rect::new(0, 0, 20, 10);
        assert_eq!(
            sidebar_section_header_toggle_rect(area, SidebarBand::Spaces, false).y,
            area.y
        );
        assert_eq!(
            sidebar_section_header_toggle_rect(area, SidebarBand::Panes, false).y,
            area.y + 1
        );
        // A collapsed band renders its title on its single row.
        let collapsed = sidebar_section_header_toggle_rect(area, SidebarBand::Agents, true);
        assert_eq!(collapsed.y, area.y);
        assert!(collapsed.width >= 2);
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

    /// One workspace with two non-agent panes (two tabs), reconciled into the
    /// Panes section.
    fn pane_section_app_with_two_panes() -> AppState {
        let mut app = AppState::test_new();
        let mut ws = Workspace::test_new("one");
        ws.test_add_tab(Some("logs"));
        app.workspaces = vec![ws];
        app.ensure_test_terminals();
        app.active = Some(0);
        app.selected = 0;
        app.mode = crate::app::Mode::Terminal;
        app.reconcile_pane_section_order();
        app
    }

    #[test]
    fn pane_section_rows_interleave_line_split_between_panes() {
        let mut app = pane_section_app_with_two_panes();
        // Insert a line-split between the two panes (flat index 1).
        let id = app
            .pane_section_order
            .new_line_split("scheduled".to_string(), 1);

        let rows = sidebar_pane_section_rows(&app);
        assert_eq!(rows.len(), 3);
        assert!(matches!(rows[0], PaneSectionRow::Pane(_)));
        assert!(matches!(
            &rows[1],
            PaneSectionRow::LineSplit { id: row_id, name, .. } if *row_id == id && name == "scheduled"
        ));
        assert!(matches!(rows[2], PaneSectionRow::Pane(_)));
    }

    #[test]
    fn pane_section_row_areas_use_variable_heights_for_splits() {
        use crate::app::state::PaneSectionRowContent;
        let mut app = pane_section_app_with_two_panes();
        // Split first, then a pane below it.
        app.pane_section_order
            .new_line_split("scheduled".to_string(), 0);

        let body = Rect::new(0, 0, 20, 12);
        let areas = pane_section_row_areas_in(&app, body, 0);
        assert_eq!(areas.len(), 3);
        // Row 0 is the split: single line.
        assert!(matches!(
            areas[0].content,
            PaneSectionRowContent::LineSplit { .. }
        ));
        assert_eq!(areas[0].rect.height, 1);
        // Row 1 is a pane: two lines, placed after the split plus a one-row gap.
        assert!(matches!(
            areas[1].content,
            PaneSectionRowContent::Pane { .. }
        ));
        assert_eq!(areas[1].rect.height, 2);
        assert_eq!(areas[1].rect.y, areas[0].rect.y + 1 + 1);
    }

    fn render_pane_section_to_backend(app: &mut AppState) -> String {
        let area = Rect::new(0, 0, 106, 30);
        crate::ui::compute_view(app, area);
        let pane_area = pane_section_rect(
            app.view.sidebar_rect,
            app.sidebar_section_split,
            app.sidebar_pane_section_split,
            sidebar_shows_pane_section(app),
            app.sidebar_section_collapse(),
        );
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))
            .expect("test terminal should initialize");
        let runtimes = TerminalRuntimeRegistry::new();
        terminal
            .draw(|frame| render_pane_section(app, &runtimes, frame, pane_area))
            .expect("pane section should render");
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

    fn render_full_sidebar_to_lines(app: &mut AppState, area: Rect) -> Vec<String> {
        crate::ui::compute_view(app, area);
        let sidebar_rect = app.view.sidebar_rect;
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))
            .expect("test terminal should initialize");
        let runtimes = TerminalRuntimeRegistry::new();
        terminal
            .draw(|frame| render_sidebar(app, &runtimes, frame, sidebar_rect))
            .expect("sidebar should render");
        let buffer = terminal.backend().buffer().clone();
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn collapsed_band_renders_arrow_header_and_keeps_others_expanded() {
        let mut app = pane_section_app_with_two_panes();
        app.spaces_section_collapsed = true;
        let area = Rect::new(0, 0, 106, 30);
        let lines = render_full_sidebar_to_lines(&mut app, area);
        let joined = lines.join("\n");
        assert!(
            joined.contains("▸ spaces"),
            "collapsed spaces band shows a collapsed toggle header: {joined:?}"
        );
        assert!(
            joined.contains("▾ agents"),
            "an expanded band shows an expand toggle header: {joined:?}"
        );
    }

    #[test]
    fn pane_section_named_line_split_renders_rule_with_name() {
        let mut app = pane_section_app_with_two_panes();
        app.pane_section_order
            .new_line_split("scheduled".to_string(), 0);
        let rendered = render_pane_section_to_backend(&mut app);
        let split_line = rendered
            .lines()
            .find(|line| line.contains("scheduled"))
            .expect("named line-split row present");
        assert!(
            split_line.contains("─"),
            "line-split should be drawn as a rule: {split_line:?}"
        );
    }

    #[test]
    fn pane_section_split_button_sits_on_header_when_mouse_capture() {
        let area = Rect::new(0, 20, 26, 10);
        let rect = pane_section_split_button_rect(area);
        assert_ne!(rect, Rect::default());
        // Right-aligned on the title row (area.y + 1).
        assert_eq!(rect.y, area.y + 1);
        assert_eq!(rect.x + rect.width, area.x + area.width);
        assert_eq!(rect.width, display_width_u16(PANE_SECTION_SPLIT_LABEL));
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
