use ratatui::layout::Rect;

use crate::app::state::{AgentPanelSort, AppState, LineSplitId, ManualEntryRef, Mode, ViewLayout};

use super::ScrollbarClickTarget;

impl AppState {
    pub(super) fn workspace_list_rect(&self) -> Rect {
        let sidebar = self.view.sidebar_rect;
        if self.sidebar_collapsed || sidebar.width <= 1 || sidebar.height == 0 {
            return Rect::default();
        }
        crate::ui::workspace_list_rect(
            sidebar,
            self.sidebar_section_split,
            self.sidebar_pane_section_split,
            crate::ui::sidebar_shows_pane_section(self),
            self.sidebar_section_collapse(),
        )
    }

    pub(super) fn agent_panel_rect(&self) -> Rect {
        let sidebar = self.view.sidebar_rect;
        if self.sidebar_collapsed || sidebar.width <= 1 || sidebar.height == 0 {
            return Rect::default();
        }
        crate::ui::agents_detail_rect(
            sidebar,
            self.sidebar_section_split,
            self.sidebar_pane_section_split,
            crate::ui::sidebar_shows_pane_section(self),
            self.sidebar_section_collapse(),
        )
    }

    pub(super) fn pane_section_rect(&self) -> Rect {
        let sidebar = self.view.sidebar_rect;
        if self.sidebar_collapsed || sidebar.width <= 1 || sidebar.height == 0 {
            return Rect::default();
        }
        crate::ui::pane_section_rect(
            sidebar,
            self.sidebar_section_split,
            self.sidebar_pane_section_split,
            crate::ui::sidebar_shows_pane_section(self),
            self.sidebar_section_collapse(),
        )
    }

    /// Resolve a sidebar row to the Panes-section pane row it hits, returning the
    /// flat order index plus the `(ws_idx, pane_id)` it points at. Line-split rows
    /// resolve to `None` (they are not focusable panes).
    pub(super) fn pane_section_row_at(
        &self,
        row: u16,
    ) -> Option<(usize, usize, crate::layout::PaneId)> {
        use crate::app::state::PaneSectionRowContent;
        self.view.pane_section_row_areas.iter().find_map(|area| {
            if row < area.rect.y || row >= area.rect.y + area.rect.height {
                return None;
            }
            match area.content {
                PaneSectionRowContent::Pane {
                    ws_idx, pane_id, ..
                } => Some((area.order_idx, ws_idx, pane_id)),
                PaneSectionRowContent::LineSplit { .. } => None,
            }
        })
    }

    /// Manual-order entry (pane or line-split) under the given sidebar row, for
    /// drag pickup. Mirrors [`AppState::agent_panel_entry_ref_at_row`].
    pub(super) fn pane_section_entry_ref_at_row(
        &self,
        row: u16,
    ) -> Option<crate::app::state::PaneManualEntryRef> {
        use crate::app::state::{PaneManualEntryRef, PaneSectionRowContent};
        for area in &self.view.pane_section_row_areas {
            if row < area.rect.y || row >= area.rect.y + area.rect.height {
                continue;
            }
            return match area.content {
                PaneSectionRowContent::Pane {
                    ws_idx, pane_id, ..
                } => {
                    let ws = self.workspaces.get(ws_idx)?;
                    let pane_number = ws.public_pane_number(pane_id)?;
                    Some(PaneManualEntryRef::Pane(
                        crate::app::state::PaneSectionRef {
                            workspace_id: ws.id.clone(),
                            pane_number,
                        },
                    ))
                }
                PaneSectionRowContent::LineSplit { id } => Some(PaneManualEntryRef::LineSplit(id)),
            };
        }
        None
    }

    /// Line-split id under the given sidebar row in the Panes section, if any (for
    /// the right-click context menu). Mirrors `agent_panel_line_split_at_row`.
    pub(super) fn pane_section_line_split_at_row(
        &self,
        row: u16,
    ) -> Option<crate::app::state::LineSplitId> {
        use crate::app::state::PaneSectionRowContent;
        for area in &self.view.pane_section_row_areas {
            if row < area.rect.y || row >= area.rect.y + area.rect.height {
                continue;
            }
            return match area.content {
                PaneSectionRowContent::LineSplit { id } => Some(id),
                PaneSectionRowContent::Pane { .. } => None,
            };
        }
        None
    }

    /// Whether the given cell hits the Panes-section "+ split" affordance.
    pub(super) fn on_pane_section_split_button(&self, col: u16, row: u16) -> bool {
        if self.sidebar_collapsed || self.pane_section_collapsed {
            return false;
        }
        let area = self.pane_section_rect();
        let rect = crate::ui::pane_section_split_button_rect(area);
        rect.width > 0
            && col >= rect.x
            && col < rect.x + rect.width
            && row >= rect.y
            && row < rect.y + rect.height
    }

    /// Resolve a stable [`crate::app::state::PaneSectionRef`] back to its live
    /// `(ws_idx, pane_id)`, or `None` if the pane no longer exists.
    pub(super) fn resolve_pane_section_ref(
        &self,
        pane_ref: &crate::app::state::PaneSectionRef,
    ) -> Option<(usize, crate::layout::PaneId)> {
        let ws_idx = self
            .workspaces
            .iter()
            .position(|ws| ws.id == pane_ref.workspace_id)?;
        let pane_id = self.workspaces[ws_idx]
            .public_pane_numbers
            .iter()
            .find(|(_, number)| **number == pane_ref.pane_number)
            .map(|(pane_id, _)| *pane_id)?;
        Some((ws_idx, pane_id))
    }

    /// Flat drop index for a Panes-section reorder at sidebar `row`, mirroring the
    /// agent-panel drop-index geometry (upper half inserts before, lower half
    /// after; the gap and area below the last row insert at the end).
    pub(super) fn pane_section_drop_index_at_row(&self, row: u16) -> Option<usize> {
        let pane_section_area = self.pane_section_rect();
        let has_scrollbar = crate::ui::should_show_scrollbar(
            crate::ui::pane_section_scroll_metrics(self, pane_section_area),
        );
        let body = crate::ui::pane_section_body_rect(pane_section_area, has_scrollbar);
        if body.width == 0 || body.height == 0 {
            return None;
        }
        if row < body.y || row >= body.y + body.height {
            return None;
        }
        // Insert index is a slot in the flat order (panes + line-splits).
        let num_entries = self.pane_section_order.order.len();
        let areas = &self.view.pane_section_row_areas;
        for area in areas {
            let slot_bottom = area.rect.y.saturating_add(area.rect.height);
            if row < slot_bottom {
                let mid = area.rect.y.saturating_add(area.rect.height / 2);
                let idx = if row < mid {
                    area.order_idx
                } else {
                    area.order_idx.saturating_add(1)
                };
                return Some(idx.min(num_entries));
            }
            if row == slot_bottom {
                return Some(area.order_idx.saturating_add(1).min(num_entries));
            }
        }
        let idx = areas
            .last()
            .map(|area| area.order_idx.saturating_add(1))
            .unwrap_or(0);
        Some(idx.min(num_entries))
    }

    pub(super) fn workspace_list_scrollbar_target_at(
        &self,
        col: u16,
        row: u16,
    ) -> Option<ScrollbarClickTarget> {
        let area = self.workspace_list_rect();
        let metrics = crate::ui::workspace_list_scroll_metrics(self, area);
        let track = crate::ui::workspace_list_scrollbar_rect(self, area)?;
        if col < track.x
            || col >= track.x + track.width
            || row < track.y
            || row >= track.y + track.height
        {
            return None;
        }
        if let Some(grab_row_offset) = crate::ui::scrollbar_thumb_grab_offset(metrics, track, row) {
            Some(ScrollbarClickTarget::Thumb { grab_row_offset })
        } else {
            Some(ScrollbarClickTarget::Track {
                offset_from_bottom: crate::ui::scrollbar_offset_from_row(metrics, track, row),
            })
        }
    }

    pub(super) fn workspace_list_offset_for_drag_row(
        &self,
        row: u16,
        grab_row_offset: u16,
    ) -> Option<usize> {
        let area = self.workspace_list_rect();
        let metrics = crate::ui::workspace_list_scroll_metrics(self, area);
        let track = crate::ui::workspace_list_scrollbar_rect(self, area)?;
        Some(crate::ui::scrollbar_offset_from_drag_row(
            metrics,
            track,
            row,
            grab_row_offset,
        ))
    }

    pub(super) fn set_workspace_list_offset_from_bottom(&mut self, offset_from_bottom: usize) {
        let area = self.workspace_list_rect();
        let metrics = crate::ui::workspace_list_scroll_metrics(self, area);
        self.workspace_scroll = metrics
            .max_offset_from_bottom
            .saturating_sub(offset_from_bottom);
        self.workspace_scroll = crate::ui::normalized_workspace_scroll(
            self,
            self.view.sidebar_rect,
            self.workspace_scroll,
        );
    }

    pub(super) fn scroll_workspace_list(&mut self, delta: i16) {
        if delta.is_negative() {
            self.workspace_scroll = self
                .workspace_scroll
                .saturating_sub(delta.unsigned_abs() as usize);
            self.workspace_scroll = crate::ui::normalized_workspace_scroll(
                self,
                self.view.sidebar_rect,
                self.workspace_scroll,
            );
            return;
        }

        let area = self.workspace_list_rect();
        let metrics = crate::ui::workspace_list_scroll_metrics(self, area);
        self.workspace_scroll = self
            .workspace_scroll
            .saturating_add(delta as usize)
            .min(metrics.max_offset_from_bottom);
        self.workspace_scroll = crate::ui::normalized_workspace_scroll(
            self,
            self.view.sidebar_rect,
            self.workspace_scroll,
        );
    }

    pub(super) fn agent_panel_scrollbar_target_at(
        &self,
        col: u16,
        row: u16,
    ) -> Option<ScrollbarClickTarget> {
        let area = self.agent_panel_rect();
        let metrics = crate::ui::agent_panel_scroll_metrics(self, area);
        let track = crate::ui::agent_panel_scrollbar_rect(self, area)?;
        if col < track.x
            || col >= track.x + track.width
            || row < track.y
            || row >= track.y + track.height
        {
            return None;
        }
        if let Some(grab_row_offset) = crate::ui::scrollbar_thumb_grab_offset(metrics, track, row) {
            Some(ScrollbarClickTarget::Thumb { grab_row_offset })
        } else {
            Some(ScrollbarClickTarget::Track {
                offset_from_bottom: crate::ui::scrollbar_offset_from_row(metrics, track, row),
            })
        }
    }

    pub(super) fn agent_panel_offset_for_drag_row(
        &self,
        row: u16,
        grab_row_offset: u16,
    ) -> Option<usize> {
        let area = self.agent_panel_rect();
        let metrics = crate::ui::agent_panel_scroll_metrics(self, area);
        let track = crate::ui::agent_panel_scrollbar_rect(self, area)?;
        Some(crate::ui::scrollbar_offset_from_drag_row(
            metrics,
            track,
            row,
            grab_row_offset,
        ))
    }

    pub(super) fn set_agent_panel_offset_from_bottom(&mut self, offset_from_bottom: usize) {
        let area = self.agent_panel_rect();
        let metrics = crate::ui::agent_panel_scroll_metrics(self, area);
        self.agent_panel_scroll = metrics
            .max_offset_from_bottom
            .saturating_sub(offset_from_bottom);
    }

    pub(super) fn scroll_agent_panel(&mut self, delta: i16) {
        let area = self.agent_panel_rect();
        let max_scroll = crate::ui::agent_panel_scroll_metrics(self, area).max_offset_from_bottom;
        if delta.is_negative() {
            self.agent_panel_scroll = self
                .agent_panel_scroll
                .saturating_sub(delta.unsigned_abs() as usize);
        } else {
            self.agent_panel_scroll = self
                .agent_panel_scroll
                .saturating_add(delta as usize)
                .min(max_scroll);
        }
    }

    /// Route a mouse-wheel notch (`delta` -1 up / +1 down) to whichever expanded
    /// sidebar band the cursor is over: Tabs, Agents, or the workspace list.
    pub(super) fn scroll_sidebar_band(&mut self, row: u16, delta: i16) {
        let over =
            |area: Rect| area != Rect::default() && row >= area.y && row < area.y + area.height;
        let tab_area = self.pane_section_rect();
        let agent_area = self.agent_panel_rect();
        if over(tab_area) {
            if crate::ui::should_show_scrollbar(crate::ui::pane_section_scroll_metrics(
                self, tab_area,
            )) {
                self.scroll_pane_section(delta);
            }
        } else if over(agent_area) {
            if crate::ui::should_show_scrollbar(crate::ui::agent_panel_scroll_metrics(
                self, agent_area,
            )) {
                self.scroll_agent_panel(delta);
            }
        } else if crate::ui::should_show_scrollbar(crate::ui::workspace_list_scroll_metrics(
            self,
            self.workspace_list_rect(),
        )) {
            self.scroll_workspace_list(delta);
        } else {
            self.move_selected_workspace_by_visible_delta(delta as isize);
        }
    }

    pub(super) fn pane_section_scrollbar_target_at(
        &self,
        col: u16,
        row: u16,
    ) -> Option<ScrollbarClickTarget> {
        let area = self.pane_section_rect();
        let metrics = crate::ui::pane_section_scroll_metrics(self, area);
        let track = crate::ui::pane_section_scrollbar_rect(self, area)?;
        if col < track.x
            || col >= track.x + track.width
            || row < track.y
            || row >= track.y + track.height
        {
            return None;
        }
        if let Some(grab_row_offset) = crate::ui::scrollbar_thumb_grab_offset(metrics, track, row) {
            Some(ScrollbarClickTarget::Thumb { grab_row_offset })
        } else {
            Some(ScrollbarClickTarget::Track {
                offset_from_bottom: crate::ui::scrollbar_offset_from_row(metrics, track, row),
            })
        }
    }

    pub(super) fn pane_section_offset_for_drag_row(
        &self,
        row: u16,
        grab_row_offset: u16,
    ) -> Option<usize> {
        let area = self.pane_section_rect();
        let metrics = crate::ui::pane_section_scroll_metrics(self, area);
        let track = crate::ui::pane_section_scrollbar_rect(self, area)?;
        Some(crate::ui::scrollbar_offset_from_drag_row(
            metrics,
            track,
            row,
            grab_row_offset,
        ))
    }

    pub(super) fn set_pane_section_offset_from_bottom(&mut self, offset_from_bottom: usize) {
        let area = self.pane_section_rect();
        let metrics = crate::ui::pane_section_scroll_metrics(self, area);
        self.pane_section_scroll = metrics
            .max_offset_from_bottom
            .saturating_sub(offset_from_bottom);
    }

    pub(super) fn scroll_pane_section(&mut self, delta: i16) {
        let area = self.pane_section_rect();
        let max_scroll = crate::ui::pane_section_scroll_metrics(self, area).max_offset_from_bottom;
        if delta.is_negative() {
            self.pane_section_scroll = self
                .pane_section_scroll
                .saturating_sub(delta.unsigned_abs() as usize);
        } else {
            self.pane_section_scroll = self
                .pane_section_scroll
                .saturating_add(delta as usize)
                .min(max_scroll);
        }
    }

    pub(crate) fn sidebar_footer_rect(&self) -> Rect {
        let ws_area = self.workspace_list_rect();
        if ws_area == Rect::default() {
            return Rect::default();
        }
        let y = ws_area.y + ws_area.height.saturating_sub(1);
        Rect::new(ws_area.x, y, ws_area.width, 1)
    }

    pub(crate) fn sidebar_new_button_rect(&self) -> Rect {
        let footer = self.sidebar_footer_rect();
        let width = 5u16.min(footer.width.max(1));
        Rect::new(footer.x, footer.y, width, footer.height)
    }

    pub(crate) fn global_launcher_rect(&self) -> Rect {
        if self.view.layout == ViewLayout::Mobile {
            return self.view.mobile_menu_hit_area;
        }

        let footer = self.sidebar_footer_rect();
        let width = if self.global_menu_attention_badge_visible() {
            8
        } else {
            6
        }
        .min(footer.width.max(1));
        let x = footer.x + footer.width.saturating_sub(width);
        Rect::new(x, footer.y, width, footer.height)
    }

    pub(crate) fn global_menu_labels(&self) -> Vec<&'static str> {
        let mut labels = vec!["settings", "keybinds", "reload config"];
        if self.update_available.is_some() {
            labels.push("update ready");
        } else if self.latest_release_notes_available {
            labels.push("what's new");
        }
        labels.push("detach");
        labels
    }

    pub(crate) fn global_menu_rect(&self) -> Rect {
        let screen = self.screen_rect();
        let launcher = self.global_launcher_rect();
        let labels = self.global_menu_labels();
        let content_width = labels
            .iter()
            .map(|label| {
                let badge_width = if self.global_menu_item_has_badge(label) {
                    2
                } else {
                    0
                };
                label.chars().count() as u16 + badge_width
            })
            .max()
            .unwrap_or(8)
            .saturating_add(2);
        let menu_w = content_width.saturating_add(2).min(screen.width.max(1));
        let menu_h = (labels.len() as u16 + 2).min(screen.height.max(1));
        let max_x = screen.x + screen.width.saturating_sub(menu_w);
        let desired_x = launcher.x + launcher.width.saturating_sub(menu_w);
        let x = desired_x.min(max_x);
        let y = launcher.y.saturating_sub(menu_h);
        Rect::new(x, y, menu_w, menu_h)
    }

    pub(super) fn on_sidebar_divider(&self, col: u16, row: u16) -> bool {
        if self.sidebar_collapsed {
            return false;
        }
        let sidebar = self.view.sidebar_rect;
        let toggle = crate::ui::expanded_sidebar_toggle_rect(sidebar);
        let on_toggle = toggle.width > 0
            && col >= toggle.x
            && col < toggle.x + toggle.width
            && row >= toggle.y
            && row < toggle.y + toggle.height;
        sidebar.width > 0
            && !on_toggle
            && col == sidebar.x + sidebar.width.saturating_sub(1)
            && row >= sidebar.y
            && row < sidebar.y + sidebar.height
    }

    pub(super) fn on_sidebar_toggle(&self, col: u16, row: u16) -> bool {
        let rect = if self.sidebar_collapsed {
            crate::ui::collapsed_sidebar_toggle_rect(self.view.sidebar_rect)
        } else {
            crate::ui::expanded_sidebar_toggle_rect(self.view.sidebar_rect)
        };
        rect.width > 0
            && col >= rect.x
            && col < rect.x + rect.width
            && row >= rect.y
            && row < rect.y + rect.height
    }

    /// Which stacked sidebar band's collapse/expand toggle sits under the cursor,
    /// if any. The toggle covers the glyph and title word on the band header.
    pub(super) fn sidebar_section_header_toggle_at(
        &self,
        col: u16,
        row: u16,
    ) -> Option<crate::ui::SidebarBand> {
        if self.sidebar_collapsed {
            return None;
        }
        let collapse = self.sidebar_section_collapse();
        let bands = [
            (
                crate::ui::SidebarBand::Spaces,
                self.workspace_list_rect(),
                collapse.spaces,
            ),
            (
                crate::ui::SidebarBand::Panes,
                self.pane_section_rect(),
                collapse.panes,
            ),
            (
                crate::ui::SidebarBand::Agents,
                self.agent_panel_rect(),
                collapse.agents,
            ),
        ];
        for (band, area, collapsed) in bands {
            let rect = crate::ui::sidebar_section_header_toggle_rect(area, band, collapsed);
            if rect.width > 0
                && col >= rect.x
                && col < rect.x + rect.width
                && row >= rect.y
                && row < rect.y + rect.height
            {
                return Some(band);
            }
        }
        None
    }

    /// Toggle the collapsed state of one stacked sidebar band.
    pub(super) fn toggle_sidebar_section(&mut self, band: crate::ui::SidebarBand) {
        match band {
            crate::ui::SidebarBand::Spaces => {
                self.spaces_section_collapsed = !self.spaces_section_collapsed;
            }
            crate::ui::SidebarBand::Panes => {
                self.pane_section_collapsed = !self.pane_section_collapsed;
            }
            crate::ui::SidebarBand::Agents => {
                self.agents_section_collapsed = !self.agents_section_collapsed;
            }
        }
    }

    pub(super) fn set_manual_sidebar_width(&mut self, divider_col: u16) {
        let sidebar = self.view.sidebar_rect;
        let width = divider_col.saturating_sub(sidebar.x).saturating_add(1);
        self.sidebar_width = width.clamp(self.sidebar_min_width, self.sidebar_max_width);
        self.sidebar_width_source = crate::app::state::SidebarWidthSource::Manual;
        self.mark_session_dirty();
    }

    /// Return which section divider (if any) sits under the cursor: 0 for the
    /// Spaces/Panes divider, 1 for the Panes/Agents divider.
    pub(super) fn on_sidebar_section_divider(&self, col: u16, row: u16) -> Option<usize> {
        if self.sidebar_collapsed {
            return None;
        }
        // The draggable band dividers are derived from the uncollapsed ratio
        // geometry, so disable them while any band is collapsed. Collapse toggles
        // manage the band sizes in that state instead.
        let collapse = self.sidebar_section_collapse();
        if collapse.spaces || collapse.panes || collapse.agents {
            return None;
        }
        let hits = |rect: Rect| {
            rect.width > 0
                && col >= rect.x
                && col < rect.x + rect.width
                && row >= rect.y
                && row < rect.y + rect.height
        };
        if hits(crate::ui::sidebar_section_divider_rect(
            self.view.sidebar_rect,
            self.sidebar_section_split,
        )) {
            return Some(0);
        }
        if hits(crate::ui::sidebar_pane_section_divider_rect(
            self.view.sidebar_rect,
            self.sidebar_section_split,
            self.sidebar_pane_section_split,
            crate::ui::sidebar_shows_pane_section(self),
        )) {
            return Some(1);
        }
        None
    }

    pub(super) fn set_sidebar_section_split(&mut self, index: usize, row: u16) {
        let sidebar = self.view.sidebar_rect;
        if sidebar.height < 6 {
            return;
        }
        match index {
            0 => {
                let relative_y = row.saturating_sub(sidebar.y);
                let ratio = (relative_y as f32) / (sidebar.height as f32);
                self.sidebar_section_split = ratio.clamp(0.1, 0.9);
                self.mark_session_dirty();
            }
            1 => {
                // The Panes/Agents divider lives inside the region below the Spaces
                // band; express its ratio relative to that region.
                let (_, rest) =
                    crate::ui::expanded_sidebar_sections(sidebar, self.sidebar_section_split);
                if rest.height < 6 {
                    return;
                }
                let relative_y = row.saturating_sub(rest.y);
                let ratio = (relative_y as f32) / (rest.height as f32);
                self.sidebar_pane_section_split = ratio.clamp(0.1, 0.9);
                self.mark_session_dirty();
            }
            _ => {}
        }
    }

    pub(super) fn workspace_at_row(&self, row: u16) -> Option<usize> {
        let footer = self.sidebar_footer_rect();
        if footer == Rect::default() {
            return None;
        }

        let cards = if self.view.workspace_card_areas.is_empty() {
            crate::ui::compute_workspace_card_areas(self, self.view.sidebar_rect)
        } else {
            self.view.workspace_card_areas.clone()
        };

        cards.iter().find_map(|card| {
            (row >= card.rect.y && row < card.rect.y + card.rect.height).then_some(card.ws_idx)
        })
    }

    pub(super) fn collapsed_workspace_at_row(&self, row: u16) -> Option<usize> {
        if !self.sidebar_collapsed {
            return None;
        }

        let (ws_area, _, _) = crate::ui::collapsed_sidebar_sections(self.view.sidebar_rect);
        if ws_area == Rect::default() || row < ws_area.y || row >= ws_area.y + ws_area.height {
            return None;
        }

        let idx = (row - ws_area.y) as usize;
        (idx < self.workspaces.len()).then_some(idx)
    }

    fn collapsed_detail_workspace_idx(&self) -> Option<usize> {
        if matches!(
            self.mode,
            Mode::Navigate
                | Mode::RenameWorkspace
                | Mode::Resize
                | Mode::ConfirmClose
                | Mode::ContextMenu
                | Mode::Settings
                | Mode::GlobalMenu
                | Mode::KeybindHelp
        ) {
            Some(self.selected)
        } else {
            self.active
        }
    }

    pub(super) fn collapsed_agent_detail_target_at(
        &self,
        row: u16,
    ) -> Option<(usize, usize, crate::layout::PaneId)> {
        if !self.sidebar_collapsed {
            return None;
        }

        let (_, _, detail_area) = crate::ui::collapsed_sidebar_sections(self.view.sidebar_rect);
        let detail_content_area = Rect::new(
            detail_area.x,
            detail_area.y,
            detail_area.width,
            detail_area.height.saturating_sub(1),
        );
        if detail_content_area == Rect::default()
            || row < detail_content_area.y
            || row >= detail_content_area.y + detail_content_area.height
        {
            return None;
        }

        let ws_idx = self.collapsed_detail_workspace_idx()?;
        let ws = self.workspaces.get(ws_idx)?;
        let detail_idx = (row - detail_content_area.y) as usize;
        let details = ws.pane_details(&self.terminals);
        let detail = details.get(detail_idx)?;
        Some((ws_idx, detail.tab_idx, detail.pane_id))
    }

    pub(super) fn workspace_drop_index_at_row(&self, row: u16) -> Option<usize> {
        let area = self.workspace_list_rect();
        let footer = self.sidebar_footer_rect();
        if area == Rect::default() || row < area.y || row >= footer.y {
            return None;
        }

        let cards = if self.view.workspace_card_areas.is_empty() {
            crate::ui::compute_workspace_card_areas(self, self.view.sidebar_rect)
        } else {
            self.view.workspace_card_areas.clone()
        };
        if cards.is_empty() {
            return Some(0);
        }

        let mut insert_indices = Vec::with_capacity(cards.len() + 1);
        for (idx, card) in cards.iter().enumerate() {
            let card_group = self
                .workspaces
                .get(card.ws_idx)
                .and_then(|ws| ws.worktree_space())
                .map(|space| space.key.as_str());
            let previous_group = idx.checked_sub(1).and_then(|prev_idx| {
                self.workspaces
                    .get(cards[prev_idx].ws_idx)
                    .and_then(|ws| ws.worktree_space())
                    .map(|space| space.key.as_str())
            });
            let inside_group_gap = card_group.is_some() && card_group == previous_group;
            if !inside_group_gap {
                insert_indices.push(card.ws_idx);
            }
        }
        insert_indices.push(cards.last().map(|card| card.ws_idx + 1).unwrap_or(0));

        let mut best: Option<(usize, u16)> = None;
        for insert_idx in insert_indices {
            let Some(slot_row) = crate::ui::workspace_drop_indicator_row(&cards, area, insert_idx)
            else {
                continue;
            };
            let distance = row.abs_diff(slot_row);
            match best {
                Some((best_idx, best_distance))
                    if distance > best_distance
                        || (distance == best_distance && insert_idx < best_idx) => {}
                _ => best = Some((insert_idx, distance)),
            }
        }

        best.map(|(insert_idx, _)| insert_idx)
    }

    pub(super) fn on_agent_panel_sort_toggle(&self, col: u16, row: u16) -> bool {
        if self.sidebar_collapsed || self.agents_section_collapsed {
            return false;
        }

        // Use the actual agents band (three-band layout) so the hit area matches
        // where the toggle is rendered, mirroring `on_agent_panel_split_button`.
        let detail_area = self.agent_panel_rect();
        let rect = crate::ui::agent_panel_toggle_rect(detail_area, self.agent_panel_sort);
        rect.width > 0
            && col >= rect.x
            && col < rect.x + rect.width
            && row >= rect.y
            && row < rect.y + rect.height
    }

    /// Body rect and visible row areas for the agent panel, shared by the
    /// pointer hit-testing helpers below. Returns `None` when the panel is
    /// collapsed or has no usable body.
    fn agent_panel_row_areas_at(&self) -> Option<(Rect, Vec<crate::ui::AgentPanelRowArea>)> {
        if self.sidebar_collapsed {
            return None;
        }
        let detail_area = self.agent_panel_rect();
        let metrics = crate::ui::agent_panel_scroll_metrics(self, detail_area);
        let body = crate::ui::agent_panel_body_rect(
            detail_area,
            crate::ui::should_show_scrollbar(metrics),
        );
        if body.width == 0 || body.height == 0 {
            return None;
        }
        let rows = crate::ui::agent_panel_rows(self);
        let areas = crate::ui::compute_agent_panel_row_areas(&rows, body, self.agent_panel_scroll);
        Some((body, areas))
    }

    /// The stable public pane id of a collapse/expand glyph under `(col, row)`,
    /// if the pointer is on a parent agent row's glyph. Applies in every sort
    /// mode. Returns `None` otherwise.
    pub(super) fn agent_panel_collapse_toggle_at(&self, col: u16, row: u16) -> Option<String> {
        if self.sidebar_collapsed {
            return None;
        }
        let (body, areas) = self.agent_panel_row_areas_at()?;
        let rows = crate::ui::agent_panel_rows(self);
        for area in &areas {
            // The glyph sits on the first line of the row.
            if row != area.y {
                continue;
            }
            if let crate::ui::AgentPanelRow::Agent(entry) = &rows[area.row_idx] {
                if !entry.has_children {
                    return None;
                }
                let glyph_col = body.x.saturating_add(
                    (entry.depth as u16).saturating_mul(crate::ui::AGENT_TREE_INDENT as u16),
                );
                if col == glyph_col {
                    let ws = self.workspaces.get(entry.ws_idx)?;
                    let number = ws.public_pane_number(entry.pane_id)?;
                    return Some(crate::workspace::public_pane_id_for_number(&ws.id, number));
                }
            }
            return None;
        }
        None
    }

    pub(super) fn agent_detail_target_at(
        &self,
        row: u16,
    ) -> Option<(usize, usize, crate::layout::PaneId)> {
        let (_, areas) = self.agent_panel_row_areas_at()?;
        let rows = crate::ui::agent_panel_rows(self);
        for area in &areas {
            if row >= area.y && row < area.y + area.height {
                // Left-click on a line-split row is a no-op (no pane focus).
                if let crate::ui::AgentPanelRow::Agent(detail) = &rows[area.row_idx] {
                    return Some((detail.ws_idx, detail.tab_idx, detail.pane_id));
                }
                return None;
            }
        }
        None
    }

    /// Manual-order entry (agent or line-split) under the given row, for drag
    /// pickup. Returns `None` outside manual mode or outside any row.
    pub(super) fn agent_panel_entry_ref_at_row(&self, row: u16) -> Option<ManualEntryRef> {
        if !matches!(self.agent_panel_sort, AgentPanelSort::Manual) {
            return None;
        }
        let (_, areas) = self.agent_panel_row_areas_at()?;
        let rows = crate::ui::agent_panel_rows(self);
        for area in &areas {
            if row >= area.y && row < area.y + area.height {
                return Some(match &rows[area.row_idx] {
                    crate::ui::AgentPanelRow::Agent(detail) => ManualEntryRef::Pane(detail.pane_id),
                    crate::ui::AgentPanelRow::LineSplit { id, .. } => {
                        ManualEntryRef::LineSplit(*id)
                    }
                });
            }
        }
        None
    }

    /// Line-split id under the given row, if any (for right-click menu).
    pub(super) fn agent_panel_line_split_at_row(&self, row: u16) -> Option<LineSplitId> {
        if !matches!(self.agent_panel_sort, AgentPanelSort::Manual) {
            return None;
        }
        let (_, areas) = self.agent_panel_row_areas_at()?;
        let rows = crate::ui::agent_panel_rows(self);
        for area in &areas {
            if row >= area.y && row < area.y + area.height {
                if let crate::ui::AgentPanelRow::LineSplit { id, .. } = &rows[area.row_idx] {
                    return Some(*id);
                }
                return None;
            }
        }
        None
    }

    pub(super) fn on_agent_panel_split_button(&self, col: u16, row: u16) -> bool {
        if self.sidebar_collapsed || self.agents_section_collapsed {
            return false;
        }
        let detail_area = self.agent_panel_rect();
        let rect = crate::ui::agent_panel_split_button_rect(detail_area, self.agent_panel_sort);
        rect.width > 0
            && col >= rect.x
            && col < rect.x + rect.width
            && row >= rect.y
            && row < rect.y + rect.height
    }

    /// Flat insert index into the manual agent order for a mouse row inside the
    /// agent panel body, honoring variable row heights. The upper half of a
    /// row's slot inserts before it, the lower half (including its trailing gap)
    /// after it. Returns `None` outside the body or when not in manual mode.
    pub(super) fn agent_panel_drop_index_at_row(&self, row: u16) -> Option<usize> {
        if !matches!(self.agent_panel_sort, AgentPanelSort::Manual) {
            return None;
        }
        let (body, areas) = self.agent_panel_row_areas_at()?;
        if row < body.y || row >= body.y + body.height {
            return None;
        }
        let num_entries = crate::ui::agent_panel_rows(self).len();
        for area in &areas {
            let slot_bottom = area.y.saturating_add(area.height);
            if row < slot_bottom {
                // Upper half inserts before, lower half after.
                let mid = area.y.saturating_add(area.height / 2);
                let idx = if row < mid {
                    area.row_idx
                } else {
                    area.row_idx.saturating_add(1)
                };
                return Some(idx.min(num_entries));
            }
            // Gap row directly below this slot inserts after it.
            if row == slot_bottom {
                return Some(area.row_idx.saturating_add(1).min(num_entries));
            }
        }
        // Below the last visible row: insert at the end of what is visible.
        let idx = areas
            .last()
            .map(|area| area.row_idx.saturating_add(1))
            .unwrap_or(self.agent_panel_scroll);
        Some(idx.min(num_entries))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crossterm::event::{MouseButton, MouseEventKind};
    use ratatui::layout::Rect;

    use super::super::{app_for_mouse_test, capture_snapshot, mouse, unique_temp_path};
    use crate::{
        app::state::{AgentPanelSort, DragTarget, Mode},
        app::App,
        config::SidebarCollapsedModeConfig,
        detect::Agent,
        workspace::Workspace,
    };

    #[test]
    fn clicking_launcher_opens_global_menu() {
        let mut app = app_for_mouse_test();
        let rect = app.state.global_launcher_rect();

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            rect.x + rect.width.saturating_sub(1),
            rect.y,
        ));

        assert_eq!(app.state.mode, Mode::GlobalMenu);
    }

    #[test]
    fn hovering_global_menu_updates_highlight() {
        let mut app = app_for_mouse_test();
        let launcher = app.state.global_launcher_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            launcher.x,
            launcher.y,
        ));

        let menu = app.state.global_menu_rect();
        app.handle_mouse(mouse(MouseEventKind::Moved, menu.x + 2, menu.y + 2));

        assert_eq!(app.state.global_menu.highlighted, 1);
    }

    #[test]
    fn clicking_keybinds_menu_item_opens_help() {
        let mut app = app_for_mouse_test();
        let launcher = app.state.global_launcher_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            launcher.x,
            launcher.y,
        ));

        let menu = app.state.global_menu_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            menu.x + 2,
            menu.y + 2,
        ));

        assert_eq!(app.state.mode, Mode::KeybindHelp);
    }

    #[test]
    fn clicking_settings_menu_item_opens_settings() {
        let mut app = app_for_mouse_test();
        let launcher = app.state.global_launcher_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            launcher.x,
            launcher.y,
        ));

        let menu = app.state.global_menu_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            menu.x + 2,
            menu.y + 1,
        ));

        assert_eq!(app.state.mode, Mode::Settings);
    }

    #[test]
    fn clicking_reload_config_menu_item_requests_reload() {
        let mut app = app_for_mouse_test();
        let launcher = app.state.global_launcher_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            launcher.x,
            launcher.y,
        ));

        let menu = app.state.global_menu_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            menu.x + 2,
            menu.y + 3,
        ));

        assert!(app.state.request_reload_config);
        assert_eq!(app.state.mode, Mode::Navigate);
    }

    #[test]
    fn update_pending_menu_surfaces_update_ready_entry() {
        let mut app = app_for_mouse_test();
        app.state.update_available = Some("0.3.2".into());
        app.state.latest_release_notes_available = true;

        let launcher = app.state.global_launcher_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            launcher.x,
            launcher.y,
        ));

        assert_eq!(
            app.state.global_menu_labels(),
            vec![
                "settings",
                "keybinds",
                "reload config",
                "update ready",
                "detach"
            ]
        );
        assert!(!app.state.should_quit);
    }

    #[test]
    fn persistence_mode_menu_surfaces_detach_action() {
        let mut app = app_for_mouse_test();
        app.state.detach_exits = false;

        let launcher = app.state.global_launcher_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            launcher.x,
            launcher.y,
        ));

        assert_eq!(
            app.state.global_menu_labels(),
            vec!["settings", "keybinds", "reload config", "detach"]
        );

        let menu = app.state.global_menu_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            menu.x + 2,
            menu.y + 4,
        ));

        assert!(app.state.detach_requested);
        assert!(!app.state.should_quit);
        assert_ne!(app.state.mode, Mode::GlobalMenu);
    }

    #[test]
    fn whats_new_remains_in_menu_for_latest_installed_release_notes() {
        let mut app = app_for_mouse_test();
        app.state.latest_release_notes_available = true;

        assert_eq!(
            app.state.global_menu_labels(),
            vec![
                "settings",
                "keybinds",
                "reload config",
                "what's new",
                "detach"
            ]
        );
    }

    #[test]
    fn clicking_agent_detail_row_switches_to_correct_tab_and_pane() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        ws.tabs[0].set_custom_name("main".into());
        let first_pane = ws.tabs[0].root_pane;
        let first_tab = ws.test_add_tab(Some("logs"));
        let second_pane = ws.tabs[first_tab].root_pane;
        app.state.workspaces = vec![ws];
        app.state.ensure_test_terminals();
        let first_terminal_id = app.state.workspaces[0].tabs[0].panes[&first_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Pi);
        let second_terminal_id = app.state.workspaces[0].tabs[first_tab].panes[&second_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&second_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Claude);
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, 16));

        assert_eq!(app.state.workspaces[0].active_tab, 1);
        assert_eq!(
            app.state.workspaces[0].tabs[1].layout.focused(),
            second_pane
        );
        assert_eq!(app.state.mode, Mode::Terminal);
        let snapshot = capture_snapshot(&app.state);
        assert_eq!(snapshot.workspaces[0].active_tab, first_tab);
        assert_eq!(
            snapshot.workspaces[0].tabs[first_tab].focused,
            Some(second_pane.raw())
        );
    }

    #[test]
    fn clicking_agent_panel_toggle_switches_sort() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("test")];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.agent_panel_scroll = 3;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let detail_area = app.state.agent_panel_rect();
        let toggle = crate::ui::agent_panel_toggle_rect(detail_area, app.state.agent_panel_sort);
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            toggle.x,
            toggle.y,
        ));

        assert_eq!(app.state.agent_panel_sort, AgentPanelSort::Priority);
        assert_eq!(app.state.agent_panel_scroll, 0);
    }

    #[test]
    fn clicking_all_workspaces_agent_row_switches_to_correct_workspace() {
        let mut app = app_for_mouse_test();
        let first = Workspace::test_new("one");
        let first_pane = first.tabs[0].root_pane;

        let second = Workspace::test_new("two");
        let second_pane = second.tabs[0].root_pane;

        app.state.workspaces = vec![first, second];
        app.state.ensure_test_terminals();
        let first_terminal_id = app.state.workspaces[0].tabs[0].panes[&first_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Pi);
        let second_terminal_id = app.state.workspaces[1].tabs[0].panes[&second_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&second_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Claude);
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;

        let (_, detail_area) = crate::ui::expanded_sidebar_sections(
            app.state.view.sidebar_rect,
            app.state.sidebar_section_split,
        );
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            detail_area.x + 2,
            detail_area.y + 6,
        ));

        assert_eq!(app.state.active, Some(1));
        assert_eq!(app.state.selected, 1);
        assert_eq!(app.state.workspaces[1].active_tab, 0);
        assert_eq!(
            app.state.workspaces[1].tabs[0].layout.focused(),
            second_pane
        );
    }

    #[test]
    fn scrolling_agent_panel_with_wheel_updates_agent_panel_scroll() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let first_pane = ws.tabs[0].root_pane;

        let mut tabs = Vec::new();
        for (tab_name, agent) in [
            ("logs", Agent::Claude),
            ("review", Agent::Codex),
            ("ops", Agent::Gemini),
        ] {
            let tab_idx = ws.test_add_tab(Some(tab_name));
            let pane_id = ws.tabs[tab_idx].root_pane;
            tabs.push((tab_idx, pane_id, agent));
        }

        app.state.workspaces = vec![ws];
        app.state.ensure_test_terminals();
        let first_terminal_id = app.state.workspaces[0].tabs[0].panes[&first_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Pi);
        for (tab_idx, pane_id, agent) in tabs {
            let terminal_id = app.state.workspaces[0].tabs[tab_idx].panes[&pane_id]
                .attached_terminal_id
                .clone();
            app.state
                .terminals
                .get_mut(&terminal_id)
                .unwrap()
                .detected_agent = Some(agent);
        }
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;

        let detail_area = app.state.agent_panel_rect();
        assert!(crate::ui::should_show_scrollbar(
            crate::ui::agent_panel_scroll_metrics(&app.state, detail_area)
        ));

        app.handle_mouse(mouse(
            MouseEventKind::ScrollDown,
            detail_area.x + 1,
            detail_area.y + 4,
        ));

        assert_eq!(app.state.agent_panel_scroll, 1);
        assert_eq!(app.state.selected, 0);
    }

    #[test]
    fn clicking_scrolled_agent_detail_row_switches_to_correct_tab_and_pane() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let first_pane = ws.tabs[0].root_pane;
        let second_tab = ws.test_add_tab(Some("logs"));
        let second_pane = ws.tabs[second_tab].root_pane;
        let mut extra_tabs = Vec::new();
        for (tab_name, agent) in [("review", Agent::Codex), ("ops", Agent::Gemini)] {
            let tab_idx = ws.test_add_tab(Some(tab_name));
            let pane_id = ws.tabs[tab_idx].root_pane;
            extra_tabs.push((tab_idx, pane_id, agent));
        }

        app.state.workspaces = vec![ws];
        app.state.ensure_test_terminals();
        let first_terminal_id = app.state.workspaces[0].tabs[0].panes[&first_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Pi);
        let second_terminal_id = app.state.workspaces[0].tabs[second_tab].panes[&second_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&second_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Claude);
        for (tab_idx, pane_id, agent) in extra_tabs {
            let terminal_id = app.state.workspaces[0].tabs[tab_idx].panes[&pane_id]
                .attached_terminal_id
                .clone();
            app.state
                .terminals
                .get_mut(&terminal_id)
                .unwrap()
                .detected_agent = Some(agent);
        }
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.agent_panel_scroll = 1;

        let detail_area = app.state.agent_panel_rect();
        let body = crate::ui::agent_panel_body_rect(detail_area, true);
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            body.x + 1,
            body.y,
        ));

        assert_eq!(app.state.workspaces[0].active_tab, second_tab);
        assert_eq!(
            app.state.workspaces[0].tabs[second_tab].layout.focused(),
            second_pane
        );
        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[test]
    fn clicking_collapsed_agent_row_switches_to_correct_tab_and_pane() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let first_pane = ws.tabs[0].root_pane;
        let second_tab = ws.test_add_tab(Some("logs"));
        let second_pane = ws.tabs[second_tab].root_pane;
        app.state.workspaces = vec![ws];
        app.state.ensure_test_terminals();
        let first_terminal_id = app.state.workspaces[0].tabs[0].panes[&first_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Pi);
        let second_terminal_id = app.state.workspaces[0].tabs[second_tab].panes[&second_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&second_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Claude);
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.sidebar_collapsed = true;
        app.state.view.sidebar_rect = Rect::new(0, 0, 4, 20);
        app.state.view.terminal_area = Rect::new(4, 0, 80, 20);

        let (_, _, detail_area) =
            crate::ui::collapsed_sidebar_sections(app.state.view.sidebar_rect);
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            detail_area.x,
            detail_area.y + 1,
        ));

        assert_eq!(app.state.workspaces[0].active_tab, 1);
        assert_eq!(
            app.state.workspaces[0].tabs[1].layout.focused(),
            second_pane
        );
        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[test]
    fn clicking_collapsed_sidebar_toggle_expands_sidebar() {
        let mut app = app_for_mouse_test();
        app.state.sidebar_collapsed = true;
        app.state.view.sidebar_rect = Rect::new(0, 0, 4, 20);
        app.state.view.terminal_area = Rect::new(4, 0, 80, 20);

        let toggle = crate::ui::collapsed_sidebar_toggle_rect(app.state.view.sidebar_rect);
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            toggle.x,
            toggle.y,
        ));

        assert!(!app.state.sidebar_collapsed);
    }

    #[test]
    fn hidden_collapsed_sidebar_has_no_mouse_expand_hotspot() {
        let mut app = app_for_mouse_test();
        app.state.sidebar_collapsed = true;
        app.state.sidebar_collapsed_mode = SidebarCollapsedModeConfig::Hidden;
        app.state.view.sidebar_rect = Rect::new(0, 0, 0, 20);
        app.state.view.terminal_area = Rect::new(0, 0, 80, 20);

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 0, 19));

        assert!(app.state.sidebar_collapsed);
    }

    #[test]
    fn clicking_expanded_sidebar_toggle_collapses_sidebar() {
        let mut app = app_for_mouse_test();
        app.state.sidebar_collapsed = false;
        app.state.view.sidebar_rect = Rect::new(0, 0, 26, 20);
        app.state.view.terminal_area = Rect::new(26, 0, 80, 20);

        let toggle = crate::ui::expanded_sidebar_toggle_rect(app.state.view.sidebar_rect);
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            toggle.x,
            toggle.y,
        ));

        assert!(app.state.sidebar_collapsed);
        assert!(app.state.drag.is_none());
    }

    #[test]
    fn clicking_workspace_switches_on_mouse_up() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("a"), Workspace::test_new("b")];
        app.state.active = Some(0);
        app.state.selected = 0;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let target_row = app.state.view.workspace_card_areas[1].rect.y;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            2,
            target_row,
        ));
        assert_eq!(app.state.active, Some(0));
        assert!(app.state.workspace_press.is_some());

        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 2, target_row));
        assert_eq!(app.state.active, Some(1));
        assert_eq!(app.state.selected, 1);
        assert!(app.state.workspace_press.is_none());
        let snapshot = capture_snapshot(&app.state);
        assert_eq!(snapshot.active, Some(1));
        assert_eq!(snapshot.selected, 1);
    }

    #[test]
    fn clicking_worktree_parent_row_focuses_workspace_without_toggling() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("main"), Workspace::test_new("issue")];
        for (idx, checkout_path) in ["/repo/herdr", "/repo/herdr-issue"].into_iter().enumerate() {
            app.state.workspaces[idx].worktree_space =
                Some(crate::workspace::WorktreeSpaceMembership {
                    key: "repo-key".into(),
                    label: "herdr".into(),
                    repo_root: "/repo/herdr".into(),
                    checkout_path: checkout_path.into(),
                    is_linked_worktree: idx > 0,
                });
        }
        app.state.active = None;
        app.state.mode = Mode::Terminal;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let parent = app.state.view.workspace_card_areas[0].rect;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            parent.x + 2,
            parent.y,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            parent.x + 2,
            parent.y,
        ));

        assert_eq!(app.state.active, Some(0));
        assert!(!app.state.collapsed_space_keys.contains("repo-key"));
    }

    #[test]
    fn clicking_worktree_parent_chevron_toggles_group_only() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("main"), Workspace::test_new("issue")];
        for (idx, checkout_path) in ["/repo/herdr", "/repo/herdr-issue"].into_iter().enumerate() {
            app.state.workspaces[idx].worktree_space =
                Some(crate::workspace::WorktreeSpaceMembership {
                    key: "repo-key".into(),
                    label: "herdr".into(),
                    repo_root: "/repo/herdr".into(),
                    checkout_path: checkout_path.into(),
                    is_linked_worktree: idx > 0,
                });
        }
        app.state.active = None;
        app.state.mode = Mode::Terminal;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let parent = app.state.view.workspace_card_areas[0].rect;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            parent.x,
            parent.y,
        ));

        assert_eq!(app.state.active, None);
        assert!(app.state.workspace_press.is_none());
        assert!(app.state.collapsed_space_keys.contains("repo-key"));

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            parent.x,
            parent.y,
        ));

        assert!(!app.state.collapsed_space_keys.contains("repo-key"));
    }

    #[test]
    fn wheel_workspace_selection_follows_grouped_visual_order_without_scrollbar() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![
            Workspace::test_new("main"),
            Workspace::test_new("normal"),
            Workspace::test_new("issue"),
        ];
        for (idx, checkout_path) in [(0, "/repo/herdr"), (2, "/repo/herdr-issue")] {
            app.state.workspaces[idx].worktree_space =
                Some(crate::workspace::WorktreeSpaceMembership {
                    key: "repo-key".into(),
                    label: "herdr".into(),
                    repo_root: "/repo/herdr".into(),
                    checkout_path: checkout_path.into(),
                    is_linked_worktree: idx != 0,
                });
        }
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Navigate;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 30));
        let list = app.state.workspace_list_rect();
        assert!(!crate::ui::should_show_scrollbar(
            crate::ui::workspace_list_scroll_metrics(&app.state, list)
        ));

        app.handle_mouse(mouse(MouseEventKind::ScrollDown, list.x + 1, list.y + 1));

        assert_eq!(app.state.selected, 2);
    }

    #[test]
    fn dragging_workspace_reorders_without_changing_identity() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![
            Workspace::test_new("a"),
            Workspace::test_new("b"),
            Workspace::test_new("c"),
        ];
        let active_id = app.state.workspaces[1].id.clone();
        let selected_id = app.state.workspaces[2].id.clone();
        app.state.active = Some(1);
        app.state.selected = 2;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let source_row = app.state.view.workspace_card_areas[1].rect.y;
        let target_row = crate::ui::workspace_drop_indicator_row(
            &app.state.view.workspace_card_areas,
            app.state.workspace_list_rect(),
            0,
        )
        .unwrap();

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            2,
            source_row,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            2,
            target_row,
        ));
        assert!(matches!(
            app.state.drag.as_ref().map(|drag| &drag.target),
            Some(DragTarget::WorkspaceReorder {
                source_ws_idx: 1,
                insert_idx: Some(0),
            })
        ));
        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 2, target_row));

        let names: Vec<_> = app
            .state
            .workspaces
            .iter()
            .map(|ws| ws.display_name())
            .collect();
        assert_eq!(names, vec!["b", "a", "c"]);
        assert_eq!(app.state.active, Some(0));
        assert_eq!(app.state.selected, 2);
        assert_eq!(app.state.workspaces[0].id, active_id);
        assert_eq!(app.state.workspaces[2].id, selected_id);
        let snapshot = capture_snapshot(&app.state);
        let captured_names: Vec<_> = snapshot
            .workspaces
            .iter()
            .map(|ws| ws.custom_name.clone().unwrap())
            .collect();
        assert_eq!(captured_names, vec!["b", "a", "c"]);
    }

    #[test]
    fn clicking_tab_scroll_button_reveals_hidden_tabs_without_renaming() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        ws.test_add_tab(Some("logs"));
        ws.test_add_tab(Some("review"));
        ws.test_add_tab(Some("ops"));
        ws.test_add_tab(Some("notes"));
        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 65, 20));

        let right = app.state.view.tab_scroll_right_hit_area;
        assert!(right.width > 0);

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            right.x + 1,
            right.y,
        ));

        assert_eq!(app.state.tab_scroll, 1);
        assert!(!app.state.tab_scroll_follow_active);
        assert_eq!(app.state.workspaces[0].active_tab, 0);
        assert_eq!(app.state.view.tab_hit_areas[0].width, 0);
        assert!(app.state.workspaces[0].tabs[0].custom_name.is_none());
        assert_eq!(
            app.state.workspaces[0].tabs[1].custom_name.as_deref(),
            Some("logs")
        );
    }

    #[test]
    fn clicking_last_visible_tab_at_right_edge_does_not_overscroll() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        for name in [
            "one", "two", "three", "four", "five", "six", "seven", "eight",
        ] {
            ws.test_add_tab(Some(name));
        }
        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.tab_scroll = usize::MAX;
        app.state.tab_scroll_follow_active = false;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 65, 20));

        let last_idx = app.state.workspaces[0].tabs.len() - 1;
        let target = app.state.view.tab_hit_areas[last_idx];
        let clamped_scroll = app.state.tab_scroll;
        assert!(target.width > 0, "last tab should already be visible");

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            target.x + 1,
            target.y,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            target.x + 1,
            target.y,
        ));

        assert_eq!(app.state.workspaces[0].active_tab, last_idx);
        assert_eq!(app.state.tab_scroll, clamped_scroll);
        assert!(app.state.view.tab_hit_areas[last_idx].width > 0);
    }

    #[test]
    fn dragging_tab_reorders_auto_and_custom_names_without_materializing_numbers() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        ws.test_add_tab(Some("foo"));
        ws.test_add_tab(None);
        let moved_root = ws.tabs[0].root_pane;
        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let source = app.state.view.tab_hit_areas[0];
        let last = app.state.view.tab_hit_areas[2];
        let drop_col = last.x + last.width;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            source.x + 1,
            source.y,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            drop_col,
            source.y,
        ));
        assert!(matches!(
            app.state.drag.as_ref().map(|drag| &drag.target),
            Some(DragTarget::TabReorder {
                ws_idx: 0,
                source_tab_idx: 0,
                insert_idx: Some(3),
            })
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            drop_col,
            source.y,
        ));

        let labels: Vec<_> = app.state.workspaces[0]
            .tabs
            .iter()
            .enumerate()
            .map(|(tab_idx, _)| app.state.workspaces[0].tab_display_name(tab_idx).unwrap())
            .collect();
        assert_eq!(labels, vec!["foo", "2", "3"]);
        assert_eq!(
            app.state.workspaces[0].tabs[0].custom_name.as_deref(),
            Some("foo")
        );
        assert!(app.state.workspaces[0].tabs[1].custom_name.is_none());
        assert!(app.state.workspaces[0].tabs[2].custom_name.is_none());
        assert_eq!(app.state.workspaces[0].tabs[0].number, 2);
        assert_eq!(app.state.workspaces[0].tabs[1].number, 3);
        assert_eq!(app.state.workspaces[0].tabs[2].number, 1);
        assert_eq!(app.state.workspaces[0].tabs[2].root_pane, moved_root);
        assert_eq!(app.state.workspaces[0].active_tab, 2);
    }

    fn temp_git_repo(branch: &str) -> std::path::PathBuf {
        let repo = unique_temp_path("sidebar-drop-slot-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(
            repo.join(".git/HEAD"),
            format!("ref: refs/heads/{branch}\n"),
        )
        .unwrap();
        repo
    }

    fn workspace_with_space(name: &str, key: &str) -> Workspace {
        let mut ws = Workspace::test_new(name);
        ws.worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
            key: key.into(),
            label: "herdr".into(),
            repo_root: "/repo/herdr".into(),
            checkout_path: format!("/repo/{name}").into(),
            is_linked_worktree: name != "main",
        });
        ws
    }

    #[test]
    fn top_drop_slot_is_distinct_from_gap_below_first_workspace() {
        let mut app = app_for_mouse_test();
        let first_repo = temp_git_repo("main");
        let second_repo = temp_git_repo("main");

        let mut first = Workspace::test_new("a");
        let first_root = first.tabs[0].root_pane;
        first.identity_cwd = first_repo.clone();
        first.refresh_git_ahead_behind();

        let mut second = Workspace::test_new("b");
        let second_root = second.tabs[0].root_pane;
        second.identity_cwd = second_repo.clone();
        second.refresh_git_ahead_behind();

        app.state.workspaces = vec![first, second];
        app.state.ensure_test_terminals();
        let first_terminal_id = app.state.workspaces[0].tabs[0].panes[&first_root]
            .attached_terminal_id
            .clone();
        app.state.terminals.get_mut(&first_terminal_id).unwrap().cwd = first_repo.clone();
        let second_terminal_id = app.state.workspaces[1].tabs[0].panes[&second_root]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&second_terminal_id)
            .unwrap()
            .cwd = second_repo.clone();
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        assert_eq!(app.state.workspace_drop_index_at_row(0), Some(0));
        assert_eq!(app.state.workspace_drop_index_at_row(1), Some(0));
        assert_eq!(app.state.workspace_drop_index_at_row(2), Some(0));
        assert_eq!(app.state.workspace_drop_index_at_row(3), Some(1));

        let _ = fs::remove_dir_all(first_repo);
        let _ = fs::remove_dir_all(second_repo);
    }

    #[test]
    fn bottom_drop_slot_stays_below_last_workspace_not_footer() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![
            Workspace::test_new("a"),
            Workspace::test_new("b"),
            Workspace::test_new("c"),
        ];
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let cards = &app.state.view.workspace_card_areas;
        let bottom_slot = crate::ui::workspace_drop_indicator_row(
            cards,
            app.state.workspace_list_rect(),
            cards.len(),
        )
        .unwrap();

        let last = cards.last().unwrap().rect;
        assert_eq!(bottom_slot, last.y + last.height);
        assert!(bottom_slot < app.state.sidebar_footer_rect().y.saturating_sub(1));
    }

    #[test]
    fn grouped_sidebar_drop_slots_do_not_land_inside_compact_group() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![
            workspace_with_space("main", "repo-key"),
            Workspace::test_new("normal"),
            workspace_with_space("issue", "repo-key"),
        ];
        app.state.active = Some(1);
        app.state.selected = 1;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 40));

        let cards = &app.state.view.workspace_card_areas;
        let order = cards.iter().map(|card| card.ws_idx).collect::<Vec<_>>();
        assert_eq!(order, vec![0, 2, 1]);
        let issue = cards.iter().find(|card| card.ws_idx == 2).unwrap();
        let normal = cards.iter().find(|card| card.ws_idx == 1).unwrap();

        assert_eq!(app.state.workspace_drop_index_at_row(issue.rect.y), Some(1));
        assert_eq!(
            crate::ui::workspace_drop_indicator_row(cards, app.state.workspace_list_rect(), 2),
            Some(normal.rect.y + normal.rect.height)
        );
    }

    #[test]
    fn dragging_worktree_space_member_does_not_reorder_workspaces() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![
            workspace_with_space("main", "repo-key"),
            Workspace::test_new("normal"),
            workspace_with_space("issue", "repo-key"),
        ];
        app.state.active = Some(0);
        app.state.selected = 0;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 40));

        let source = app
            .state
            .view
            .workspace_card_areas
            .iter()
            .find(|card| card.ws_idx == 2)
            .unwrap()
            .rect;
        let target_row = crate::ui::workspace_drop_indicator_row(
            &app.state.view.workspace_card_areas,
            app.state.workspace_list_rect(),
            0,
        )
        .unwrap();

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, source.y));
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            2,
            target_row,
        ));
        assert!(app.state.drag.is_none());
        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 2, target_row));

        let names = app
            .state
            .workspaces
            .iter()
            .map(|ws| ws.display_name())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["main", "normal", "issue"]);
    }

    #[test]
    fn dragging_sidebar_divider_sets_manual_width() {
        let mut app = app_for_mouse_test();

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 25, 5));
        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 30, 5));

        assert_eq!(app.state.sidebar_width, 31);
        let snapshot = capture_snapshot(&app.state);
        assert_eq!(snapshot.sidebar_width, Some(31));
    }

    #[test]
    fn dragging_sidebar_bottom_divider_still_sets_manual_width() {
        let mut app = app_for_mouse_test();
        let divider_col = app.state.view.sidebar_rect.x + app.state.view.sidebar_rect.width - 1;
        let bottom_row = app.state.view.sidebar_rect.y + app.state.view.sidebar_rect.height - 1;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            divider_col,
            bottom_row,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            divider_col + 5,
            bottom_row,
        ));

        assert_eq!(app.state.sidebar_width, 31);
    }

    #[test]
    fn dragging_past_max_clamps_to_configured_max() {
        let mut app = app_for_mouse_test();
        app.state.sidebar_max_width = 30;

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 25, 5));
        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 50, 5));

        assert_eq!(app.state.sidebar_width, 30);
    }

    #[test]
    fn dragging_below_min_clamps_to_configured_min() {
        let mut app = app_for_mouse_test();
        app.state.sidebar_min_width = 22;

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 25, 5));
        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 5, 5));

        assert_eq!(app.state.sidebar_width, 22);
    }

    #[test]
    fn dragging_sidebar_section_divider_sets_split_ratio() {
        let mut app = app_for_mouse_test();
        let divider = crate::ui::sidebar_section_divider_rect(
            app.state.view.sidebar_rect,
            app.state.sidebar_section_split,
        );

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            divider.x + 1,
            divider.y,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            divider.x + 1,
            divider.y + 4,
        ));

        assert!(app.state.sidebar_section_split > 0.5);
        let snapshot = capture_snapshot(&app.state);
        assert_eq!(
            snapshot.sidebar_section_split,
            Some(app.state.sidebar_section_split)
        );
    }

    #[test]
    fn double_clicking_sidebar_divider_resets_default_width() {
        let mut app = app_for_mouse_test();
        app.state.default_sidebar_width = 26;
        app.state.sidebar_width = 30;

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 25, 5));
        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 25, 5));
        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 25, 5));

        assert_eq!(app.state.sidebar_width, 26);
        assert!(app.state.drag.is_none());
        let snapshot = capture_snapshot(&app.state);
        assert_eq!(snapshot.sidebar_width, Some(26));
    }

    fn app_with_two_agents() -> App {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.agent_panel_sort = AgentPanelSort::Manual;
        for ws_idx in 0..2 {
            let pane = app.state.workspaces[ws_idx].tabs[0].root_pane;
            let terminal_id = app.state.workspaces[ws_idx].tabs[0].panes[&pane]
                .attached_terminal_id
                .clone();
            app.state
                .terminals
                .get_mut(&terminal_id)
                .unwrap()
                .detected_agent = Some(Agent::Pi);
        }
        app
    }

    fn manual_order_pane_ids(app: &App) -> Vec<crate::layout::PaneId> {
        crate::ui::agent_panel_entries(&app.state)
            .into_iter()
            .map(|entry| entry.pane_id)
            .collect()
    }

    fn agent_row_for(app: &App, pane_id: crate::layout::PaneId, want_last: bool) -> u16 {
        let sidebar = app.state.view.sidebar_rect;
        let rows = sidebar.y..sidebar.y + sidebar.height;
        let matching = rows.filter(|&r| {
            app.state.agent_detail_target_at(r).map(|(_, _, pane)| pane) == Some(pane_id)
        });
        if want_last {
            matching.max().expect("agent row present")
        } else {
            matching.min().expect("agent row present")
        }
    }

    #[test]
    fn dragging_agent_reorders_manual_order() {
        let mut app = app_with_two_agents();
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let order = manual_order_pane_ids(&app);
        assert_eq!(order.len(), 2);
        let source_pane = order[0];
        let last_pane = order[1];

        let source_row = agent_row_for(&app, source_pane, false);
        // The gap row just below the last entry inserts at the end of the order.
        let target_row = agent_row_for(&app, last_pane, true) + 1;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            2,
            source_row,
        ));
        assert!(app.state.agent_press.is_some());

        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            2,
            target_row,
        ));
        assert!(matches!(
            app.state.drag.as_ref().map(|drag| &drag.target),
            Some(DragTarget::AgentReorder {
                insert_idx: Some(2),
                ..
            })
        ));

        // A drag-to-reorder gesture must invalidate the pending double-click
        // candidate so a fast re-click after the drag focuses rather than
        // opening the rename dialog.
        assert!(app.last_agent_row_click.is_none());

        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 2, target_row));

        assert_eq!(manual_order_pane_ids(&app), vec![last_pane, source_pane]);
        assert_ne!(app.state.mode, Mode::RenameAgent);
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn double_clicking_agent_row_opens_tab_rename_in_manual_mode() {
        let mut app = app_with_two_agents();
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let order = manual_order_pane_ids(&app);
        let target_pane = order[0];
        let row = agent_row_for(&app, target_pane, false);

        // First click focuses; second quick click on the same row opens the tab
        // rename (agent double-click renames the agent's tab).
        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, row));
        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 2, row));
        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, row));

        assert_eq!(app.state.mode, Mode::RenameTab);
        assert_eq!(app.state.rename_tab_target, Some((0, 0)));
        assert!(app.state.agent_press.is_none());
    }

    #[test]
    fn clicking_agent_row_without_drag_focuses_pane() {
        let mut app = app_with_two_agents();
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let order = manual_order_pane_ids(&app);
        let target_pane = order[1];
        let row = agent_row_for(&app, target_pane, false);

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, row));
        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 2, row));

        assert_eq!(app.state.active, Some(1));
        assert_eq!(app.state.workspaces[1].focused_pane_id(), Some(target_pane));
        assert!(app.state.agent_press.is_none());
    }

    #[test]
    fn clicking_sort_toggle_cycles_spaces_priority_manual() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("one")];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.agent_panel_sort = AgentPanelSort::Spaces;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let click_toggle = |app: &mut App| {
            crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
            let detail_area = app.state.agent_panel_rect();
            let rect = crate::ui::agent_panel_toggle_rect(detail_area, app.state.agent_panel_sort);
            app.handle_mouse(mouse(
                MouseEventKind::Down(MouseButton::Left),
                rect.x,
                rect.y,
            ));
        };

        click_toggle(&mut app);
        assert_eq!(app.state.agent_panel_sort, AgentPanelSort::Priority);
        click_toggle(&mut app);
        assert_eq!(app.state.agent_panel_sort, AgentPanelSort::Manual);
        click_toggle(&mut app);
        assert_eq!(app.state.agent_panel_sort, AgentPanelSort::Spaces);
    }

    fn app_with_two_tab_rows() -> App {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app
    }

    #[test]
    fn pane_section_drop_index_geometry() {
        let mut app = app_with_two_tab_rows();
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 30));

        let areas = app.state.view.pane_section_row_areas.clone();
        assert_eq!(
            areas.len(),
            2,
            "both plain tabs render as Panes-section rows"
        );
        let first = areas[0].rect;
        let last = areas[1].rect;

        // Upper half of the first row inserts before it; lower half after it.
        assert_eq!(app.state.pane_section_drop_index_at_row(first.y), Some(0));
        assert_eq!(
            app.state
                .pane_section_drop_index_at_row(first.y + first.height - 1),
            Some(1)
        );
        // The gap row below the last entry inserts at the end.
        assert_eq!(
            app.state
                .pane_section_drop_index_at_row(last.y + last.height),
            Some(2)
        );
    }

    /// Pane id carried by a Panes-section row area, panicking on a line-split row.
    fn pane_area_pane_id(area: &crate::app::state::PaneSectionRowArea) -> crate::layout::PaneId {
        match area.content {
            crate::app::state::PaneSectionRowContent::Pane { pane_id, .. } => pane_id,
            crate::app::state::PaneSectionRowContent::LineSplit { .. } => {
                panic!("expected a pane row, found a line-split")
            }
        }
    }

    #[test]
    fn pane_row_double_click_opens_pane_rename() {
        let mut app = app_with_two_tab_rows();
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 30));
        let area = app.state.view.pane_section_row_areas[0];
        let row = area.rect.y;
        let pane_id = pane_area_pane_id(&area);
        let col = 2;

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), col, row));
        assert_eq!(app.state.mode, Mode::Terminal);
        assert!(app.last_pane_section_row_click.is_some());
        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), col, row));

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), col, row));
        assert_eq!(app.state.mode, Mode::RenamePane);
        assert_eq!(app.state.rename_pane_target, Some(pane_id));
        assert!(app.last_pane_section_row_click.is_none());
    }

    #[test]
    fn pane_section_split_button_click_creates_and_renames_split() {
        use crate::app::state::{LineSplitSection, PaneManualEntry};
        let mut app = app_with_two_tab_rows();
        app.state.mouse_capture = true;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 30));

        let pane_area = app.state.pane_section_rect();
        let rect = crate::ui::pane_section_split_button_rect(pane_area);
        assert_ne!(rect, Rect::default());
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            rect.x,
            rect.y,
        ));

        // A new empty line-split is inserted at the top and rename opens for it.
        assert!(matches!(
            app.state.pane_section_order.order.first(),
            Some(PaneManualEntry::LineSplit { name, .. }) if name.is_empty()
        ));
        assert_eq!(app.state.mode, Mode::RenameLineSplit);
        assert!(matches!(
            app.state.rename_line_split_target,
            Some((LineSplitSection::Panes, _))
        ));
    }

    #[test]
    fn pane_section_entry_ref_at_row_picks_up_split() {
        use crate::app::state::{PaneManualEntryRef, PaneSectionRowContent};
        let mut app = app_with_two_tab_rows();
        let split = app
            .state
            .pane_section_order
            .new_line_split("scheduled".to_string(), 0);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 30));

        let split_row = app
            .state
            .view
            .pane_section_row_areas
            .iter()
            .find(|area| matches!(area.content, PaneSectionRowContent::LineSplit { .. }))
            .expect("split row present");
        assert_eq!(
            app.state.pane_section_entry_ref_at_row(split_row.rect.y),
            Some(PaneManualEntryRef::LineSplit(split))
        );
        // A left-click on a split row is a focus no-op (no pane resolved).
        assert!(app.state.pane_section_row_at(split_row.rect.y).is_none());
    }

    #[test]
    fn pane_section_line_split_right_click_opens_menu() {
        use crate::app::state::{ContextMenuKind, LineSplitSection, PaneSectionRowContent};
        let mut app = app_with_two_tab_rows();
        let split = app
            .state
            .pane_section_order
            .new_line_split("scheduled".to_string(), 0);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 30));

        let split_row = app
            .state
            .view
            .pane_section_row_areas
            .iter()
            .find(|area| matches!(area.content, PaneSectionRowContent::LineSplit { .. }))
            .expect("split row present")
            .rect
            .y;
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Right),
            2,
            split_row,
        ));
        assert_eq!(app.state.mode, Mode::ContextMenu);
        assert!(matches!(
            app.state.context_menu.as_ref().map(|menu| &menu.kind),
            Some(ContextMenuKind::LineSplit {
                section: LineSplitSection::Panes,
                id,
            }) if *id == split
        ));
    }

    #[test]
    fn pane_section_drag_reorders_line_split() {
        use crate::app::state::{PaneManualEntry, PaneManualEntryRef, PaneSectionRowContent};
        // A single non-agent pane plus a split at the top, so both rows fit and
        // the end of the order is reachable by mouse.
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("one")];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        let split = app
            .state
            .pane_section_order
            .new_line_split("scheduled".to_string(), 0);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 30));

        let areas = app.state.view.pane_section_row_areas.clone();
        assert_eq!(areas.len(), 2, "split and pane both visible");
        let split_row = areas
            .iter()
            .find(|area| matches!(area.content, PaneSectionRowContent::LineSplit { .. }))
            .expect("split row present")
            .rect
            .y;
        let pane_area = areas
            .iter()
            .find(|area| matches!(area.content, PaneSectionRowContent::Pane { .. }))
            .expect("pane row present")
            .rect;
        // Lower half of the pane row inserts after it (end of the order).
        let target_row = pane_area.y + pane_area.height - 1;

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, split_row));
        assert!(app.state.pane_section_press.is_some());
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            2,
            target_row,
        ));
        assert!(matches!(
            app.state.drag.as_ref().map(|drag| &drag.target),
            Some(DragTarget::PaneSectionReorder {
                source: PaneManualEntryRef::LineSplit(id),
                insert_idx: Some(_),
            }) if *id == split
        ));
        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 2, target_row));

        // The split now sits at the end of the order (below the pane).
        assert!(matches!(
            app.state.pane_section_order.order.last(),
            Some(PaneManualEntry::LineSplit { id, .. }) if *id == split
        ));
        app.state.assert_invariants_for_test();
    }

    fn app_with_many_tab_rows(n: usize) -> App {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("one"); // tab 0
        for i in 1..n {
            ws.test_add_tab(Some(&format!("t{i}")));
        }
        app.state.workspaces = vec![ws];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app
    }

    #[test]
    fn pane_section_scroll_metrics_report_overflow() {
        let mut app = app_with_many_tab_rows(8);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let metrics =
            crate::ui::pane_section_scroll_metrics(&app.state, app.state.pane_section_rect());
        assert_eq!(crate::ui::sidebar_pane_section_entries(&app.state).len(), 8);
        assert!(metrics.viewport_rows >= 1 && metrics.viewport_rows < 8);
        assert!(metrics.max_offset_from_bottom > 0);
        assert!(crate::ui::should_show_scrollbar(metrics));
    }

    #[test]
    fn pane_section_hit_test_accounts_for_scroll() {
        let mut app = app_with_many_tab_rows(8);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        assert!(crate::ui::should_show_scrollbar(
            crate::ui::pane_section_scroll_metrics(&app.state, app.state.pane_section_rect())
        ));

        // At scroll 0 the top visible row maps to the first pane.
        let top_area = app.state.view.pane_section_row_areas[0];
        assert_eq!(
            app.state.pane_section_row_at(top_area.rect.y),
            Some((0, 0, pane_area_pane_id(&top_area)))
        );

        // After scrolling down, the same screen row maps to a later pane, and the
        // scroll-aware drop index follows the visible order.
        app.state.pane_section_scroll = 2;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let top = app.state.view.pane_section_row_areas[0];
        assert_eq!(top.order_idx, 2);
        assert_eq!(
            app.state.pane_section_row_at(top.rect.y),
            Some((2, 0, pane_area_pane_id(&top)))
        );
        assert_eq!(
            app.state.pane_section_drop_index_at_row(top.rect.y),
            Some(2)
        );
    }
}
