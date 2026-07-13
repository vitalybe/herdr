use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use unicode_width::UnicodeWidthChar;

use crate::{
    app::{
        state::{
            CopyModeSelection, CopyModeState, CopySearchDirection, CopySearchMatch, CopySearchState,
        },
        App, AppState, Mode,
    },
    input::TerminalKey,
    layout::PaneId,
    selection::Selection,
    terminal::TerminalRuntimeRegistry,
};

impl App {
    pub(crate) fn handle_copy_mode_key(&mut self, key: TerminalKey) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        self.state.update_dismissed = true;
        self.state
            .handle_copy_mode_key(&self.terminal_runtimes, key);
        if let Some(content) = self.state.request_clipboard_write.take() {
            if self
                .event_tx
                .try_send(crate::events::AppEvent::ClipboardWrite { content })
                .is_err()
            {
                tracing::warn!("failed to queue clipboard write event");
            }
        }
    }
}

impl AppState {
    pub(crate) fn enter_copy_mode(&mut self, terminal_runtimes: &TerminalRuntimeRegistry) {
        let Some(ws_idx) = self.active else {
            return;
        };
        let Some(pane_id) = self
            .workspaces
            .get(ws_idx)
            .and_then(|ws| ws.focused_pane_id())
        else {
            return;
        };
        let Some(info) = self.pane_info_by_id(pane_id).cloned() else {
            return;
        };
        if info.inner_rect.width == 0 || info.inner_rect.height == 0 {
            return;
        }

        let cursor = self
            .runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, pane_id)
            .and_then(|rt| rt.cursor_state(info.inner_rect, true))
            .filter(|cursor| cursor.visible)
            .map(|cursor| {
                (
                    cursor.y.saturating_sub(info.inner_rect.y),
                    cursor.x.saturating_sub(info.inner_rect.x),
                )
            })
            .unwrap_or_else(|| (info.inner_rect.height.saturating_sub(1), 0));
        let entry_offset_from_bottom = self
            .pane_scroll_metrics(terminal_runtimes, pane_id)
            .map_or(0, |metrics| metrics.offset_from_bottom);

        self.clear_selection();
        self.copy_search = None;
        self.copy_mode = Some(CopyModeState {
            pane_id,
            cursor_row: cursor.0.min(info.inner_rect.height.saturating_sub(1)),
            cursor_col: cursor.1.min(info.inner_rect.width.saturating_sub(1)),
            entry_offset_from_bottom,
            selection: None,
        });
        self.mode = Mode::Copy;
    }

    /// Enter copy mode (if not already) and open the scrollback search prompt.
    /// Driven by the `find` action for a direct jump into search, and by `/`
    /// and `?` from within copy mode.
    pub(crate) fn enter_copy_search(
        &mut self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        direction: CopySearchDirection,
    ) {
        let entered_via_find = self.copy_mode.is_none();
        if entered_via_find {
            self.enter_copy_mode(terminal_runtimes);
        }
        let Some(copy_mode) = self.copy_mode else {
            return;
        };
        let metrics = self.pane_scroll_metrics(terminal_runtimes, copy_mode.pane_id);
        let origin_offset_from_bottom = metrics.map_or(0, |m| m.offset_from_bottom);
        let origin_row = Selection::absolute_row_for_viewport(copy_mode.cursor_row, metrics);
        self.copy_search = Some(CopySearchState {
            query: String::new(),
            direction,
            editing: true,
            matches: Vec::new(),
            current: None,
            origin_offset_from_bottom,
            origin_row,
            origin_col: copy_mode.cursor_col,
            entered_via_find,
        });
    }

    fn handle_copy_search_key(
        &mut self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        key: TerminalKey,
    ) {
        match key.code {
            KeyCode::Esc => {
                self.cancel_copy_search(terminal_runtimes);
                return;
            }
            KeyCode::Enter => {
                self.commit_copy_search(terminal_runtimes);
                return;
            }
            KeyCode::Backspace => {
                if let Some(search) = self.copy_search.as_mut() {
                    search.query.pop();
                }
                self.recompute_copy_search(terminal_runtimes);
                return;
            }
            _ => {}
        }

        if let Some(ch) = copy_mode_command_char(key) {
            if let Some(search) = self.copy_search.as_mut() {
                search.query.push(ch);
            }
            self.recompute_copy_search(terminal_runtimes);
        }
    }

    fn recompute_copy_search(&mut self, terminal_runtimes: &TerminalRuntimeRegistry) {
        let Some(search) = self.copy_search.as_ref() else {
            return;
        };
        let Some(copy_mode) = self.copy_mode else {
            self.copy_search = None;
            return;
        };
        let query = search.query.clone();
        let direction = search.direction;
        let (origin_row, origin_col) = (search.origin_row, search.origin_col);
        let Some(ws_idx) = self.active else {
            return;
        };
        let lines = self
            .runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, copy_mode.pane_id)
            .map(|rt| rt.scrollback_lines())
            .unwrap_or_default();
        let matches = compute_search_matches(&lines, &query);
        let current = pick_match_index(&matches, origin_row, origin_col, direction);
        if let Some(search) = self.copy_search.as_mut() {
            search.matches = matches;
            search.current = current;
        }
        self.jump_to_current_search_match(terminal_runtimes);
    }

    fn commit_copy_search(&mut self, terminal_runtimes: &TerminalRuntimeRegistry) {
        let empty = self
            .copy_search
            .as_ref()
            .is_none_or(|search| search.query.is_empty());
        if empty {
            self.cancel_copy_search(terminal_runtimes);
            return;
        }
        if let Some(search) = self.copy_search.as_mut() {
            search.editing = false;
        }
    }

    fn cancel_copy_search(&mut self, terminal_runtimes: &TerminalRuntimeRegistry) {
        let Some(search) = self.copy_search.take() else {
            return;
        };
        if search.entered_via_find {
            self.exit_copy_mode(terminal_runtimes, false);
        } else if let Some(copy_mode) = self.copy_mode {
            self.set_pane_scroll_offset(
                terminal_runtimes,
                copy_mode.pane_id,
                search.origin_offset_from_bottom,
            );
        }
    }

    /// Step to the next match. `same_direction` follows the active search
    /// direction (`n`); otherwise steps against it (`N`).
    fn step_copy_search(
        &mut self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        same_direction: bool,
    ) {
        let Some(search) = self.copy_search.as_ref() else {
            return;
        };
        if search.editing || search.matches.is_empty() {
            return;
        }
        let len = search.matches.len();
        let forward = matches!(search.direction, CopySearchDirection::Forward) == same_direction;
        let current = search.current.unwrap_or(0);
        let next = if forward {
            (current + 1) % len
        } else {
            (current + len - 1) % len
        };
        if let Some(search) = self.copy_search.as_mut() {
            search.current = Some(next);
        }
        self.jump_to_current_search_match(terminal_runtimes);
    }

    fn jump_to_current_search_match(&mut self, terminal_runtimes: &TerminalRuntimeRegistry) {
        let Some(search) = self.copy_search.as_ref() else {
            return;
        };
        let Some(idx) = search.current else {
            return;
        };
        let Some(hit) = search.matches.get(idx).copied() else {
            return;
        };
        let Some(copy_mode) = self.copy_mode else {
            return;
        };
        let pane_id = copy_mode.pane_id;
        let Some(viewport_row) =
            self.scroll_search_row_into_view(terminal_runtimes, pane_id, hit.row)
        else {
            return;
        };
        let width = self
            .pane_info_by_id(pane_id)
            .map_or(0, |info| info.inner_rect.width);
        if let Some(copy_mode) = self.copy_mode.as_mut() {
            copy_mode.cursor_row = viewport_row;
            copy_mode.cursor_col = hit.col.min(width.saturating_sub(1));
        }
    }

    /// Scroll `row` (absolute) into the viewport, leaving roughly a third of the
    /// viewport above it, and return its resulting viewport row.
    fn scroll_search_row_into_view(
        &mut self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        pane_id: PaneId,
        row: u32,
    ) -> Option<u16> {
        let metrics = self.pane_scroll_metrics(terminal_runtimes, pane_id)?;
        let viewport_rows = (metrics.viewport_rows as u32).max(1);
        let max_offset = metrics.max_offset_from_bottom as u32;
        let margin = viewport_rows / 3;
        let target_top = row.saturating_sub(margin).min(max_offset);
        let offset = max_offset.saturating_sub(target_top);
        self.set_pane_scroll_offset(terminal_runtimes, pane_id, offset as usize);
        let viewport_row = row.saturating_sub(target_top);
        if viewport_row >= viewport_rows {
            return None;
        }
        Some(viewport_row as u16)
    }

    pub(crate) fn handle_copy_mode_key(
        &mut self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        key: TerminalKey,
    ) {
        if self
            .copy_search
            .as_ref()
            .is_some_and(|search| search.editing)
        {
            self.handle_copy_search_key(terminal_runtimes, key);
            return;
        }

        match key.code {
            KeyCode::Esc => {
                self.exit_copy_mode(terminal_runtimes, false);
                return;
            }
            KeyCode::Enter => {
                self.exit_copy_mode(terminal_runtimes, true);
                return;
            }
            KeyCode::Left => {
                self.move_copy_cursor(terminal_runtimes, 0, -1);
                return;
            }
            KeyCode::Down => {
                self.move_copy_cursor(terminal_runtimes, 1, 0);
                return;
            }
            KeyCode::Up => {
                self.move_copy_cursor(terminal_runtimes, -1, 0);
                return;
            }
            KeyCode::Right => {
                self.move_copy_cursor(terminal_runtimes, 0, 1);
                return;
            }
            KeyCode::PageUp => {
                self.scroll_copy_mode_page(terminal_runtimes, -1, false);
                return;
            }
            KeyCode::PageDown => {
                self.scroll_copy_mode_page(terminal_runtimes, 1, false);
                return;
            }
            KeyCode::Home => {
                self.copy_mode_line_edge(terminal_runtimes, false);
                return;
            }
            KeyCode::End => {
                self.copy_mode_line_edge(terminal_runtimes, true);
                return;
            }
            _ => {}
        }

        match (key.code, key.modifiers) {
            (KeyCode::Char('b'), mods) if mods.contains(KeyModifiers::CONTROL) => {
                self.scroll_copy_mode_page(terminal_runtimes, -1, false)
            }
            (KeyCode::Char('f'), mods) if mods.contains(KeyModifiers::CONTROL) => {
                self.scroll_copy_mode_page(terminal_runtimes, 1, false)
            }
            (KeyCode::Char('u'), mods) if mods.contains(KeyModifiers::CONTROL) => {
                self.scroll_copy_mode_page(terminal_runtimes, -1, true)
            }
            (KeyCode::Char('d'), mods) if mods.contains(KeyModifiers::CONTROL) => {
                self.scroll_copy_mode_page(terminal_runtimes, 1, true)
            }
            _ => {}
        }

        let Some(ch) = copy_mode_command_char(key) else {
            return;
        };
        match ch {
            'q' => self.exit_copy_mode(terminal_runtimes, false),
            'y' => self.exit_copy_mode(terminal_runtimes, true),
            'v' | ' ' => self.begin_copy_mode_selection(terminal_runtimes),
            'V' => self.select_copy_mode_line(terminal_runtimes),
            'h' => self.move_copy_cursor(terminal_runtimes, 0, -1),
            'j' => self.move_copy_cursor(terminal_runtimes, 1, 0),
            'k' => self.move_copy_cursor(terminal_runtimes, -1, 0),
            'l' => self.move_copy_cursor(terminal_runtimes, 0, 1),
            'g' => self.copy_mode_history_top(terminal_runtimes),
            'G' => self.copy_mode_history_bottom(terminal_runtimes),
            '0' => self.copy_mode_line_edge(terminal_runtimes, false),
            '$' => self.copy_mode_line_edge(terminal_runtimes, true),
            '^' => self.copy_mode_first_non_blank(terminal_runtimes),
            'w' => self.copy_mode_word_motion(terminal_runtimes, WordMotion::NextStart),
            'b' => self.copy_mode_word_motion(terminal_runtimes, WordMotion::PreviousStart),
            'e' => self.copy_mode_word_motion(terminal_runtimes, WordMotion::NextEnd),
            '{' => self.copy_mode_paragraph(terminal_runtimes, -1),
            '}' => self.copy_mode_paragraph(terminal_runtimes, 1),
            '/' => self.enter_copy_search(terminal_runtimes, CopySearchDirection::Forward),
            '?' => self.enter_copy_search(terminal_runtimes, CopySearchDirection::Backward),
            'n' => self.step_copy_search(terminal_runtimes, true),
            'N' => self.step_copy_search(terminal_runtimes, false),
            _ => {}
        }
    }

    fn exit_copy_mode(&mut self, terminal_runtimes: &TerminalRuntimeRegistry, copy: bool) {
        let restore_scroll = self
            .copy_mode
            .map(|copy_mode| (copy_mode.pane_id, copy_mode.entry_offset_from_bottom));
        if copy {
            self.copy_selection(terminal_runtimes);
        } else {
            self.clear_selection();
        }
        if let Some((pane_id, offset_from_bottom)) = restore_scroll {
            self.set_pane_scroll_offset(terminal_runtimes, pane_id, offset_from_bottom);
        }
        self.copy_search = None;
        self.copy_mode = None;
        self.mode = if self.active.is_some() {
            Mode::Terminal
        } else {
            Mode::Navigate
        };
    }

    fn begin_copy_mode_selection(&mut self, terminal_runtimes: &TerminalRuntimeRegistry) {
        let Some(copy_mode) = self.copy_mode else {
            return;
        };
        let Some(info) = self.pane_info_by_id(copy_mode.pane_id).cloned() else {
            return;
        };
        if copy_mode.cursor_row >= info.inner_rect.height
            || copy_mode.cursor_col >= info.inner_rect.width
        {
            return;
        }

        let metrics = self.pane_scroll_metrics(terminal_runtimes, copy_mode.pane_id);
        self.selection = Some(Selection::anchor(
            copy_mode.pane_id,
            copy_mode.cursor_row,
            copy_mode.cursor_col,
            metrics,
        ));
        if let Some(copy_mode) = self.copy_mode.as_mut() {
            copy_mode.selection = Some(CopyModeSelection::Character);
        }
    }

    fn select_copy_mode_line(&mut self, terminal_runtimes: &TerminalRuntimeRegistry) {
        let Some(mut copy_mode) = self.copy_mode else {
            return;
        };
        let Some(info) = self.pane_info_by_id(copy_mode.pane_id) else {
            return;
        };
        let end_col = info.inner_rect.width.saturating_sub(1);
        let metrics = self.pane_scroll_metrics(terminal_runtimes, copy_mode.pane_id);
        let anchor_row = Selection::absolute_row_for_viewport(copy_mode.cursor_row, metrics);
        self.selection = Some(Selection::line_range(
            copy_mode.pane_id,
            anchor_row,
            anchor_row,
            end_col,
        ));
        copy_mode.selection = Some(CopyModeSelection::Linewise { anchor_row });
        self.copy_mode = Some(copy_mode);
    }

    fn move_copy_cursor(
        &mut self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        row_delta: i16,
        col_delta: i16,
    ) {
        let Some(mut copy_mode) = self.copy_mode else {
            return;
        };
        let Some(info) = self.pane_info_by_id(copy_mode.pane_id).cloned() else {
            self.exit_copy_mode(terminal_runtimes, false);
            return;
        };

        if col_delta < 0 {
            copy_mode.cursor_col = copy_mode
                .cursor_col
                .saturating_sub(col_delta.unsigned_abs());
        } else if col_delta > 0 {
            copy_mode.cursor_col = copy_mode
                .cursor_col
                .saturating_add(col_delta as u16)
                .min(info.inner_rect.width.saturating_sub(1));
        }

        if row_delta < 0 {
            let delta = row_delta.unsigned_abs();
            if copy_mode.cursor_row >= delta {
                copy_mode.cursor_row -= delta;
            } else {
                self.scroll_pane_up(terminal_runtimes, copy_mode.pane_id, usize::from(delta));
                copy_mode.cursor_row = 0;
            }
        } else if row_delta > 0 {
            let delta = row_delta as u16;
            let bottom = info.inner_rect.height.saturating_sub(1);
            if copy_mode.cursor_row.saturating_add(delta) <= bottom {
                copy_mode.cursor_row += delta;
            } else {
                self.scroll_pane_down(terminal_runtimes, copy_mode.pane_id, usize::from(delta));
                copy_mode.cursor_row = bottom;
            }
        }

        self.copy_mode = Some(copy_mode);
        self.sync_copy_mode_selection(terminal_runtimes);
    }

    fn scroll_copy_mode_page(
        &mut self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        direction: i16,
        half_page: bool,
    ) {
        let Some(mut copy_mode) = self.copy_mode else {
            return;
        };
        let Some(info) = self.pane_info_by_id(copy_mode.pane_id).cloned() else {
            self.exit_copy_mode(terminal_runtimes, false);
            return;
        };
        let lines = copy_mode_page_lines(info.inner_rect.height, half_page);
        if let Some(metrics) = self.pane_scroll_metrics(terminal_runtimes, copy_mode.pane_id) {
            if direction < 0 {
                let next_offset = metrics.offset_from_bottom.saturating_add(lines);
                if next_offset > metrics.max_offset_from_bottom {
                    let scrolled_lines = metrics
                        .max_offset_from_bottom
                        .saturating_sub(metrics.offset_from_bottom);
                    let cursor_lines = lines.saturating_sub(scrolled_lines);
                    self.set_pane_scroll_offset(
                        terminal_runtimes,
                        copy_mode.pane_id,
                        metrics.max_offset_from_bottom,
                    );
                    copy_mode.cursor_row = copy_mode
                        .cursor_row
                        .saturating_sub(cursor_lines.min(u16::MAX as usize) as u16);
                } else {
                    self.set_pane_scroll_offset(terminal_runtimes, copy_mode.pane_id, next_offset);
                }
            } else if metrics.offset_from_bottom < lines {
                let cursor_lines = lines.saturating_sub(metrics.offset_from_bottom);
                self.set_pane_scroll_offset(terminal_runtimes, copy_mode.pane_id, 0);
                copy_mode.cursor_row = copy_mode
                    .cursor_row
                    .saturating_add(cursor_lines.min(u16::MAX as usize) as u16)
                    .min(info.inner_rect.height.saturating_sub(1));
            } else {
                self.set_pane_scroll_offset(
                    terminal_runtimes,
                    copy_mode.pane_id,
                    metrics.offset_from_bottom - lines,
                );
            }
        } else if direction < 0 {
            self.scroll_pane_up(terminal_runtimes, copy_mode.pane_id, lines);
        } else {
            self.scroll_pane_down(terminal_runtimes, copy_mode.pane_id, lines);
        }
        self.copy_mode = Some(copy_mode);
        self.sync_copy_mode_selection(terminal_runtimes);
    }

    fn copy_mode_history_top(&mut self, terminal_runtimes: &TerminalRuntimeRegistry) {
        let Some(mut copy_mode) = self.copy_mode else {
            return;
        };
        let Some(metrics) = self.pane_scroll_metrics(terminal_runtimes, copy_mode.pane_id) else {
            return;
        };
        self.set_pane_scroll_offset(
            terminal_runtimes,
            copy_mode.pane_id,
            metrics.max_offset_from_bottom,
        );
        copy_mode.cursor_row = 0;
        self.copy_mode = Some(copy_mode);
        self.sync_copy_mode_selection(terminal_runtimes);
    }

    fn copy_mode_history_bottom(&mut self, terminal_runtimes: &TerminalRuntimeRegistry) {
        let Some(mut copy_mode) = self.copy_mode else {
            return;
        };
        let Some(info) = self.pane_info_by_id(copy_mode.pane_id) else {
            self.exit_copy_mode(terminal_runtimes, false);
            return;
        };
        self.set_pane_scroll_offset(terminal_runtimes, copy_mode.pane_id, 0);
        copy_mode.cursor_row = info.inner_rect.height.saturating_sub(1);
        self.copy_mode = Some(copy_mode);
        self.sync_copy_mode_selection(terminal_runtimes);
    }

    fn copy_mode_line_edge(&mut self, terminal_runtimes: &TerminalRuntimeRegistry, end: bool) {
        let Some(mut copy_mode) = self.copy_mode else {
            return;
        };
        let Some(info) = self.pane_info_by_id(copy_mode.pane_id) else {
            self.exit_copy_mode(terminal_runtimes, false);
            return;
        };
        copy_mode.cursor_col = if end {
            info.inner_rect.width.saturating_sub(1)
        } else {
            0
        };
        self.copy_mode = Some(copy_mode);
        self.sync_copy_mode_selection(terminal_runtimes);
    }

    fn copy_mode_first_non_blank(&mut self, terminal_runtimes: &TerminalRuntimeRegistry) {
        let Some(mut copy_mode) = self.copy_mode else {
            return;
        };
        let Some(text) = self.copy_mode_visible_row_text(terminal_runtimes, copy_mode.cursor_row)
        else {
            return;
        };
        copy_mode.cursor_col = first_non_blank_col(&text).unwrap_or(0);
        self.copy_mode = Some(copy_mode);
        self.sync_copy_mode_selection(terminal_runtimes);
    }

    fn copy_mode_word_motion(
        &mut self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        motion: WordMotion,
    ) {
        let Some(mut copy_mode) = self.copy_mode else {
            return;
        };
        let Some(info) = self.pane_info_by_id(copy_mode.pane_id) else {
            self.exit_copy_mode(terminal_runtimes, false);
            return;
        };
        let Some(text) = self.copy_mode_visible_row_text(terminal_runtimes, copy_mode.cursor_row)
        else {
            return;
        };
        let Some(col) = word_motion_target(&text, copy_mode.cursor_col, motion) else {
            return;
        };
        copy_mode.cursor_col = col.min(info.inner_rect.width.saturating_sub(1));
        self.copy_mode = Some(copy_mode);
        self.sync_copy_mode_selection(terminal_runtimes);
    }

    fn copy_mode_paragraph(&mut self, terminal_runtimes: &TerminalRuntimeRegistry, direction: i16) {
        let Some(copy_mode) = self.copy_mode else {
            return;
        };
        let Some(pane_height) = self
            .pane_info_by_id(copy_mode.pane_id)
            .map(|info| info.inner_rect.height)
        else {
            self.exit_copy_mode(terminal_runtimes, false);
            return;
        };
        let limit = self
            .pane_scroll_metrics(terminal_runtimes, copy_mode.pane_id)
            .map(|metrics| metrics.max_offset_from_bottom + metrics.viewport_rows)
            .unwrap_or(pane_height as usize)
            .clamp(1, 1000);

        for _ in 0..limit {
            let before = self.copy_mode;
            let before_offset = self
                .pane_scroll_metrics(terminal_runtimes, copy_mode.pane_id)
                .map(|metrics| metrics.offset_from_bottom);

            self.move_copy_cursor(terminal_runtimes, direction, 0);

            let Some(after) = self.copy_mode else {
                return;
            };
            if self
                .copy_mode_visible_row_text(terminal_runtimes, after.cursor_row)
                .is_some_and(|text| text.trim().is_empty())
            {
                return;
            }

            let Some(after_metrics) = self.pane_scroll_metrics(terminal_runtimes, after.pane_id)
            else {
                continue;
            };
            let did_not_move =
                before == self.copy_mode && before_offset == Some(after_metrics.offset_from_bottom);
            let at_top = direction < 0
                && after.cursor_row == 0
                && after_metrics.offset_from_bottom == after_metrics.max_offset_from_bottom;
            let at_bottom = direction > 0
                && after.cursor_row + 1 >= pane_height
                && after_metrics.offset_from_bottom == 0;
            if did_not_move || at_top || at_bottom {
                return;
            }
        }
    }

    fn copy_mode_visible_row_text(
        &self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        viewport_row: u16,
    ) -> Option<String> {
        let copy_mode = self.copy_mode?;
        let ws_idx = self.active?;
        let info = self.pane_info_by_id(copy_mode.pane_id)?;
        if viewport_row >= info.inner_rect.height || info.inner_rect.width == 0 {
            return None;
        }
        let metrics = self.pane_scroll_metrics(terminal_runtimes, copy_mode.pane_id);
        let row_selection = Selection::range(
            copy_mode.pane_id,
            viewport_row,
            0,
            info.inner_rect.width.saturating_sub(1),
            metrics,
        );
        self.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, copy_mode.pane_id)?
            .extract_selection(&row_selection)
    }

    fn sync_copy_mode_selection(&mut self, terminal_runtimes: &TerminalRuntimeRegistry) {
        let Some(copy_mode) = self.copy_mode else {
            return;
        };
        let Some(selection) = copy_mode.selection else {
            return;
        };
        let Some(info) = self.pane_info_by_id(copy_mode.pane_id).cloned() else {
            return;
        };
        match selection {
            CopyModeSelection::Character => {
                let screen_col = info.inner_rect.x.saturating_add(copy_mode.cursor_col);
                let screen_row = info.inner_rect.y.saturating_add(copy_mode.cursor_row);
                self.update_selection_cursor(
                    terminal_runtimes,
                    copy_mode.pane_id,
                    screen_col,
                    screen_row,
                );
            }
            CopyModeSelection::Linewise { anchor_row } => {
                let metrics = self.pane_scroll_metrics(terminal_runtimes, copy_mode.pane_id);
                let cursor_row =
                    Selection::absolute_row_for_viewport(copy_mode.cursor_row, metrics);
                self.selection = Some(Selection::line_range(
                    copy_mode.pane_id,
                    anchor_row,
                    cursor_row,
                    info.inner_rect.width.saturating_sub(1),
                ));
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WordMotion {
    NextStart,
    PreviousStart,
    NextEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WordSpan {
    start: u16,
    end: u16,
}

fn first_non_blank_col(text: &str) -> Option<u16> {
    let mut col = 0u16;
    for ch in text.chars() {
        if !ch.is_whitespace() {
            return Some(col);
        }
        col = col.saturating_add(char_cell_width(ch));
    }
    None
}

fn word_motion_target(text: &str, cursor_col: u16, motion: WordMotion) -> Option<u16> {
    let spans = word_spans(text);
    match motion {
        WordMotion::NextStart => spans.iter().enumerate().find_map(|(idx, span)| {
            if cursor_col < span.start {
                Some(span.start)
            } else if cursor_col >= span.start && cursor_col <= span.end {
                spans.get(idx + 1).map(|next| next.start)
            } else {
                None
            }
        }),
        WordMotion::PreviousStart => spans
            .iter()
            .rev()
            .find(|span| span.start < cursor_col)
            .map(|span| span.start),
        WordMotion::NextEnd => spans.iter().find_map(|span| {
            if cursor_col < span.end {
                Some(span.end)
            } else {
                None
            }
        }),
    }
}

fn word_spans(text: &str) -> Vec<WordSpan> {
    let mut spans = Vec::new();
    let mut col = 0u16;
    let mut start = None;

    for ch in text.chars() {
        let width = char_cell_width(ch);
        if ch.is_whitespace() {
            if let Some(start_col) = start.take() {
                spans.push(WordSpan {
                    start: start_col,
                    end: col.saturating_sub(1),
                });
            }
        } else if start.is_none() {
            start = Some(col);
        }
        col = col.saturating_add(width);
    }

    if let Some(start_col) = start {
        spans.push(WordSpan {
            start: start_col,
            end: col.saturating_sub(1),
        });
    }
    spans
}

fn char_cell_width(ch: char) -> u16 {
    UnicodeWidthChar::width(ch).unwrap_or(1).max(1) as u16
}

fn char_cells(chars: &[char]) -> u16 {
    chars
        .iter()
        .fold(0u16, |acc, ch| acc.saturating_add(char_cell_width(*ch)))
}

/// Find every match of `query` across `lines` (indexed by absolute row),
/// returning hits ordered by row then column. Matching is case-insensitive for
/// ASCII; non-ASCII characters match exactly. Columns are cell columns so they
/// line up with the copy-mode cursor and the on-screen highlight.
fn compute_search_matches(lines: &[String], query: &str) -> Vec<CopySearchMatch> {
    let needle: Vec<char> = query.chars().map(|ch| ch.to_ascii_lowercase()).collect();
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    for (row, line) in lines.iter().enumerate() {
        find_line_matches(line, &needle, row as u32, &mut out);
    }
    out
}

fn find_line_matches(line: &str, needle: &[char], row: u32, out: &mut Vec<CopySearchMatch>) {
    let chars: Vec<char> = line.chars().collect();
    if needle.is_empty() || chars.len() < needle.len() {
        return;
    }
    let mut i = 0;
    while i + needle.len() <= chars.len() {
        let hit = (0..needle.len()).all(|k| chars[i + k].to_ascii_lowercase() == needle[k]);
        if hit {
            let col = char_cells(&chars[..i]);
            let width = char_cells(&chars[i..i + needle.len()]);
            out.push(CopySearchMatch { row, col, width });
            i += needle.len();
        } else {
            i += 1;
        }
    }
}

/// Pick the match to land on relative to an anchor `(row, col)`: the first hit
/// at or after the anchor for a forward search, or the last hit at or before it
/// for a backward search, wrapping around when none is on the chosen side.
fn pick_match_index(
    matches: &[CopySearchMatch],
    row: u32,
    col: u16,
    direction: CopySearchDirection,
) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    match direction {
        CopySearchDirection::Forward => matches
            .iter()
            .position(|m| (m.row, m.col) >= (row, col))
            .or(Some(0)),
        CopySearchDirection::Backward => matches
            .iter()
            .rposition(|m| (m.row, m.col) <= (row, col))
            .or(Some(matches.len() - 1)),
    }
}

fn copy_mode_page_lines(height: u16, half_page: bool) -> usize {
    if height <= 2 {
        1
    } else if half_page {
        usize::from(height / 2)
    } else {
        usize::from(height - 2)
    }
}

fn copy_mode_command_char(key: TerminalKey) -> Option<char> {
    if !key.modifiers.difference(KeyModifiers::SHIFT).is_empty() {
        return None;
    }

    if let Some(ch) = key.shifted_codepoint.and_then(char::from_u32) {
        return Some(ch);
    }

    let KeyCode::Char(ch) = key.code else {
        return None;
    };
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        Some(shifted_ascii_char(ch).unwrap_or(ch))
    } else {
        Some(ch)
    }
}

fn shifted_ascii_char(ch: char) -> Option<char> {
    match ch {
        'a'..='z' => Some(ch.to_ascii_uppercase()),
        '1' => Some('!'),
        '2' => Some('@'),
        '3' => Some('#'),
        '4' => Some('$'),
        '5' => Some('%'),
        '6' => Some('^'),
        '7' => Some('&'),
        '8' => Some('*'),
        '9' => Some('('),
        '0' => Some(')'),
        '-' => Some('_'),
        '=' => Some('+'),
        '[' => Some('{'),
        ']' => Some('}'),
        '\\' => Some('|'),
        ';' => Some(':'),
        '\'' => Some('"'),
        ',' => Some('<'),
        '.' => Some('>'),
        '/' => Some('?'),
        '`' => Some('~'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{app_for_mouse_test, numbered_lines_bytes};
    use super::*;
    use crate::{events::AppEvent, workspace::Workspace};
    use ratatui::layout::Rect;

    fn app_with_copy_runtime(
        runtime: impl FnOnce(u16, u16) -> crate::terminal::TerminalRuntime,
    ) -> (App, crate::layout::PaneId) {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        let pane_infos = ws.tabs[0].layout.panes(Rect::new(0, 0, 20, 5));
        let info = pane_infos[0].clone();
        ws.tabs[0].runtimes.insert(
            pane_id,
            runtime(info.inner_rect.width, info.inner_rect.height),
        );
        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.view.pane_infos = pane_infos;
        (app, pane_id)
    }

    fn app_with_copy_screen(bytes: &[u8]) -> (App, crate::layout::PaneId) {
        app_with_copy_runtime(|cols, rows| {
            crate::terminal::TerminalRuntime::test_with_screen_bytes(cols, rows, bytes)
        })
    }

    fn app_with_copy_scrollback(bytes: &[u8]) -> (App, crate::layout::PaneId) {
        app_with_copy_runtime(|cols, rows| {
            crate::terminal::TerminalRuntime::test_with_scrollback_bytes(
                cols,
                rows,
                16 * 1024,
                bytes,
            )
        })
    }

    fn copy_mode_clipboard_text(app: &mut App) -> String {
        match app.event_rx.try_recv().expect("clipboard event") {
            AppEvent::ClipboardWrite { content } => {
                String::from_utf8(content).expect("utf8 clipboard")
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    fn copy_mode_viewport_top_row(app: &App, pane_id: crate::layout::PaneId) -> usize {
        let metrics = app
            .state
            .runtime_for_pane_in_workspace(&app.terminal_runtimes, 0, pane_id)
            .and_then(crate::terminal::TerminalRuntime::scroll_metrics)
            .expect("copy mode scroll metrics");
        metrics
            .max_offset_from_bottom
            .saturating_sub(metrics.offset_from_bottom)
    }

    fn copy_mode_offset_from_bottom(app: &App, pane_id: crate::layout::PaneId) -> usize {
        app.state
            .runtime_for_pane_in_workspace(&app.terminal_runtimes, 0, pane_id)
            .and_then(crate::terminal::TerminalRuntime::scroll_metrics)
            .expect("copy mode scroll metrics")
            .offset_from_bottom
    }

    fn copy_mode_scroll_metrics(
        app: &App,
        pane_id: crate::layout::PaneId,
    ) -> crate::pane::ScrollMetrics {
        app.state
            .runtime_for_pane_in_workspace(&app.terminal_runtimes, 0, pane_id)
            .and_then(crate::terminal::TerminalRuntime::scroll_metrics)
            .expect("copy mode scroll metrics")
    }

    #[tokio::test]
    async fn enter_copy_mode_tracks_focused_pane() {
        let (mut app, pane_id) = app_with_copy_screen(b"alpha\nbeta\n");
        app.state.enter_copy_mode(&app.terminal_runtimes);
        assert_eq!(app.state.mode, Mode::Copy);
        assert_eq!(app.state.copy_mode.expect("copy mode").pane_id, pane_id);
    }

    #[tokio::test]
    async fn copy_mode_ctrl_b_uses_page_up() {
        let bytes = numbered_lines_bytes(64);
        let (mut app, pane_id) = app_with_copy_scrollback(&bytes);
        app.state.enter_copy_mode(&app.terminal_runtimes);
        let height = app.state.copy_mode.expect("copy mode").cursor_row + 1;
        let expected_lines = copy_mode_page_lines(height, false);

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('b'), KeyModifiers::CONTROL));

        assert_eq!(app.state.mode, Mode::Copy);
        assert_eq!(copy_mode_offset_from_bottom(&app, pane_id), expected_lines);
    }

    #[tokio::test]
    async fn copy_mode_ctrl_f_uses_page_down() {
        let bytes = numbered_lines_bytes(64);
        let (mut app, pane_id) = app_with_copy_scrollback(&bytes);
        app.state.enter_copy_mode(&app.terminal_runtimes);
        let height = app.state.copy_mode.expect("copy mode").cursor_row + 1;
        let page_lines = copy_mode_page_lines(height, false);
        app.state
            .set_pane_scroll_offset(&app.terminal_runtimes, pane_id, page_lines);

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('f'), KeyModifiers::CONTROL));

        assert_eq!(app.state.mode, Mode::Copy);
        assert_eq!(copy_mode_offset_from_bottom(&app, pane_id), 0);
    }

    #[tokio::test]
    async fn copy_mode_word_motions_use_visible_row_words() {
        let (mut app, _) = app_with_copy_screen(b"foo bar baz\n");
        app.state.enter_copy_mode(&app.terminal_runtimes);
        if let Some(copy_mode) = app.state.copy_mode.as_mut() {
            copy_mode.cursor_row = 0;
            copy_mode.cursor_col = 0;
        }

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('w'), KeyModifiers::empty()));
        assert_eq!(app.state.copy_mode.expect("copy mode").cursor_col, 4);

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('e'), KeyModifiers::empty()));
        assert_eq!(app.state.copy_mode.expect("copy mode").cursor_col, 6);

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('b'), KeyModifiers::empty()));
        assert_eq!(app.state.copy_mode.expect("copy mode").cursor_col, 4);
    }

    #[tokio::test]
    async fn copy_mode_shift_v_y_copies_visible_line() {
        let (mut app, _) = app_with_copy_screen(b"alpha\r\nbeta\r\n");
        app.state.enter_copy_mode(&app.terminal_runtimes);
        if let Some(copy_mode) = app.state.copy_mode.as_mut() {
            copy_mode.cursor_row = 1;
            copy_mode.cursor_col = 2;
        }

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('v'), KeyModifiers::SHIFT));
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('y'), KeyModifiers::empty()));

        assert_eq!(copy_mode_clipboard_text(&mut app), "beta");
        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[tokio::test]
    async fn copy_mode_shift_v_extends_linewise_down() {
        let (mut app, _) = app_with_copy_screen(b"alpha\r\nbeta\r\ngamma\r\n");
        app.state.enter_copy_mode(&app.terminal_runtimes);
        if let Some(copy_mode) = app.state.copy_mode.as_mut() {
            copy_mode.cursor_row = 0;
            copy_mode.cursor_col = 2;
        }

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('v'), KeyModifiers::SHIFT));
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('j'), KeyModifiers::empty()));
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('y'), KeyModifiers::empty()));

        assert_eq!(copy_mode_clipboard_text(&mut app), "alpha\nbeta");
    }

    #[tokio::test]
    async fn copy_mode_shift_v_extends_linewise_up() {
        let (mut app, _) = app_with_copy_screen(b"alpha\r\nbeta\r\ngamma\r\n");
        app.state.enter_copy_mode(&app.terminal_runtimes);
        if let Some(copy_mode) = app.state.copy_mode.as_mut() {
            copy_mode.cursor_row = 1;
            copy_mode.cursor_col = 2;
        }

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('v'), KeyModifiers::SHIFT));
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('k'), KeyModifiers::empty()));
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('y'), KeyModifiers::empty()));

        assert_eq!(copy_mode_clipboard_text(&mut app), "alpha\nbeta");
    }

    #[tokio::test]
    async fn copy_mode_shift_v_reverses_without_character_tail() {
        let (mut app, _) = app_with_copy_screen(b"alpha\r\nbeta\r\ngamma\r\n");
        app.state.enter_copy_mode(&app.terminal_runtimes);
        if let Some(copy_mode) = app.state.copy_mode.as_mut() {
            copy_mode.cursor_row = 1;
            copy_mode.cursor_col = 2;
        }

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('v'), KeyModifiers::SHIFT));
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('j'), KeyModifiers::empty()));
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('k'), KeyModifiers::empty()));
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('k'), KeyModifiers::empty()));
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('y'), KeyModifiers::empty()));

        assert_eq!(copy_mode_clipboard_text(&mut app), "alpha\nbeta");
    }

    #[tokio::test]
    async fn copy_mode_shift_v_horizontal_motion_keeps_linewise_selection() {
        let (mut app, _) = app_with_copy_screen(b"alpha\r\nbeta\r\n");
        app.state.enter_copy_mode(&app.terminal_runtimes);
        if let Some(copy_mode) = app.state.copy_mode.as_mut() {
            copy_mode.cursor_row = 1;
            copy_mode.cursor_col = 2;
        }

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('v'), KeyModifiers::SHIFT));
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('h'), KeyModifiers::empty()));
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('l'), KeyModifiers::empty()));
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('y'), KeyModifiers::empty()));

        assert_eq!(copy_mode_clipboard_text(&mut app), "beta");
    }

    #[tokio::test]
    async fn copy_mode_shift_v_page_up_keeps_linewise_scrollback_selection() {
        let bytes = numbered_lines_bytes(64);
        let (mut app, pane_id) = app_with_copy_scrollback(&bytes);
        app.state.enter_copy_mode(&app.terminal_runtimes);
        if let Some(copy_mode) = app.state.copy_mode.as_mut() {
            copy_mode.cursor_row = 0;
            copy_mode.cursor_col = 2;
        }

        let anchor_row = copy_mode_viewport_top_row(&app, pane_id);
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('v'), KeyModifiers::SHIFT));
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::PageUp, KeyModifiers::empty()));
        let cursor_row = copy_mode_viewport_top_row(&app, pane_id);
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('y'), KeyModifiers::empty()));

        assert!(cursor_row < anchor_row);
        let expected = (cursor_row..=anchor_row)
            .map(|row| format!("{row:06}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(copy_mode_clipboard_text(&mut app), expected);
        assert_eq!(copy_mode_offset_from_bottom(&app, pane_id), 0);
    }

    #[tokio::test]
    async fn copy_mode_page_up_uses_tmux_page_size() {
        let bytes = numbered_lines_bytes(64);
        let (mut app, pane_id) = app_with_copy_scrollback(&bytes);
        app.state.enter_copy_mode(&app.terminal_runtimes);
        let height = app.state.copy_mode.expect("copy mode").cursor_row + 1;
        let expected_lines = copy_mode_page_lines(height, false);

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::PageUp, KeyModifiers::empty()));

        assert_eq!(copy_mode_offset_from_bottom(&app, pane_id), expected_lines);
    }

    #[tokio::test]
    async fn copy_mode_ctrl_u_moves_cursor_when_history_top_clamps() {
        let bytes = numbered_lines_bytes(64);
        let (mut app, pane_id) = app_with_copy_scrollback(&bytes);
        app.state.enter_copy_mode(&app.terminal_runtimes);
        let bottom = app.state.copy_mode.expect("copy mode").cursor_row;
        let lines = copy_mode_page_lines(bottom + 1, true);
        let metrics = copy_mode_scroll_metrics(&app, pane_id);
        assert!(metrics.max_offset_from_bottom >= lines);
        app.state.set_pane_scroll_offset(
            &app.terminal_runtimes,
            pane_id,
            metrics.max_offset_from_bottom - lines + 1,
        );
        if let Some(copy_mode) = app.state.copy_mode.as_mut() {
            copy_mode.cursor_row = bottom;
        }

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('u'), KeyModifiers::CONTROL));

        let copy_mode = app.state.copy_mode.expect("copy mode");
        let expected_cursor_delta = 1;
        assert_eq!(
            copy_mode_offset_from_bottom(&app, pane_id),
            metrics.max_offset_from_bottom
        );
        assert_eq!(
            copy_mode.cursor_row,
            bottom.saturating_sub(expected_cursor_delta as u16)
        );
    }

    #[tokio::test]
    async fn copy_mode_ctrl_d_moves_cursor_when_live_bottom_clamps() {
        let bytes = numbered_lines_bytes(64);
        let (mut app, pane_id) = app_with_copy_scrollback(&bytes);
        app.state.enter_copy_mode(&app.terminal_runtimes);
        let bottom = app.state.copy_mode.expect("copy mode").cursor_row;
        let lines = copy_mode_page_lines(bottom + 1, true);
        assert!(lines > 1);
        app.state
            .set_pane_scroll_offset(&app.terminal_runtimes, pane_id, lines - 1);
        if let Some(copy_mode) = app.state.copy_mode.as_mut() {
            copy_mode.cursor_row = 0;
        }

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('d'), KeyModifiers::CONTROL));

        let copy_mode = app.state.copy_mode.expect("copy mode");
        assert_eq!(copy_mode_offset_from_bottom(&app, pane_id), 0);
        assert_eq!(copy_mode.cursor_row, 1);
    }

    #[tokio::test]
    async fn copy_mode_q_exits_and_returns_to_bottom_after_scrollback() {
        let bytes = numbered_lines_bytes(64);
        let (mut app, pane_id) = app_with_copy_scrollback(&bytes);
        app.state.enter_copy_mode(&app.terminal_runtimes);

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::PageUp, KeyModifiers::empty()));
        assert!(copy_mode_offset_from_bottom(&app, pane_id) > 0);

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('q'), KeyModifiers::empty()));

        assert_eq!(app.state.mode, Mode::Terminal);
        assert!(app.state.copy_mode.is_none());
        assert_eq!(copy_mode_offset_from_bottom(&app, pane_id), 0);
    }

    #[tokio::test]
    async fn copy_mode_q_restores_entry_scrollback_offset() {
        let bytes = numbered_lines_bytes(64);
        let (mut app, pane_id) = app_with_copy_scrollback(&bytes);
        let entry_offset = 3;
        app.state
            .set_pane_scroll_offset(&app.terminal_runtimes, pane_id, entry_offset);
        assert_eq!(copy_mode_offset_from_bottom(&app, pane_id), entry_offset);

        app.state.enter_copy_mode(&app.terminal_runtimes);
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::PageUp, KeyModifiers::empty()));
        assert!(copy_mode_offset_from_bottom(&app, pane_id) > entry_offset);

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('q'), KeyModifiers::empty()));

        assert_eq!(app.state.mode, Mode::Terminal);
        assert!(app.state.copy_mode.is_none());
        assert_eq!(copy_mode_offset_from_bottom(&app, pane_id), entry_offset);
    }

    #[tokio::test]
    async fn shifted_punctuation_keys_work_with_enhanced_key_reporting() {
        let (mut app, _) = app_with_copy_screen(b"foo\r\n\r\nbar\r\n");
        app.state.enter_copy_mode(&app.terminal_runtimes);
        if let Some(copy_mode) = app.state.copy_mode.as_mut() {
            copy_mode.cursor_row = 2;
            copy_mode.cursor_col = 2;
        }

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('6'), KeyModifiers::SHIFT));
        assert_eq!(app.state.copy_mode.expect("copy mode").cursor_col, 0);

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char(']'), KeyModifiers::SHIFT));
        assert_eq!(app.state.copy_mode.expect("copy mode").cursor_row, 3);

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('['), KeyModifiers::SHIFT));
        assert_eq!(app.state.copy_mode.expect("copy mode").cursor_row, 1);

        app.handle_copy_mode_key(
            TerminalKey::new(KeyCode::Char(']'), KeyModifiers::SHIFT)
                .with_shifted_codepoint('}' as u32),
        );
        assert_eq!(app.state.copy_mode.expect("copy mode").cursor_row, 3);
    }

    #[tokio::test]
    async fn copy_mode_v_y_copies_selection_and_exits() {
        let (mut app, _) = app_with_copy_screen(b"alpha\nbeta\n");
        app.state.enter_copy_mode(&app.terminal_runtimes);
        if let Some(copy_mode) = app.state.copy_mode.as_mut() {
            copy_mode.cursor_row = 0;
            copy_mode.cursor_col = 0;
        }
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('v'), KeyModifiers::empty()));
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('l'), KeyModifiers::empty()));
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('l'), KeyModifiers::empty()));
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('y'), KeyModifiers::empty()));

        match app.event_rx.try_recv().expect("clipboard event") {
            AppEvent::ClipboardWrite { content } => assert_eq!(content, b"alp"),
            other => panic!("unexpected event: {other:?}"),
        }
        assert_eq!(app.state.mode, Mode::Terminal);
        assert!(app.state.copy_mode.is_none());
    }

    fn type_query(app: &mut App, query: &str) {
        for ch in query.chars() {
            app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char(ch), KeyModifiers::empty()));
        }
    }

    #[test]
    fn compute_search_matches_uses_cell_columns_and_ascii_case() {
        let lines = vec![
            "  Error: boom".to_string(),
            "no hit".to_string(),
            "err err".to_string(),
        ];
        let matches = compute_search_matches(&lines, "err");
        assert_eq!(matches.len(), 3);
        assert_eq!(
            (matches[0].row, matches[0].col, matches[0].width),
            (0, 2, 3)
        );
        assert_eq!((matches[1].row, matches[1].col), (2, 0));
        assert_eq!((matches[2].row, matches[2].col), (2, 4));
    }

    #[test]
    fn compute_search_matches_empty_query_is_empty() {
        let lines = vec!["anything".to_string()];
        assert!(compute_search_matches(&lines, "").is_empty());
    }

    #[test]
    fn pick_match_index_forward_and_backward_wrap() {
        let matches = vec![
            CopySearchMatch {
                row: 1,
                col: 0,
                width: 1,
            },
            CopySearchMatch {
                row: 3,
                col: 2,
                width: 1,
            },
            CopySearchMatch {
                row: 5,
                col: 0,
                width: 1,
            },
        ];
        let fwd = CopySearchDirection::Forward;
        let back = CopySearchDirection::Backward;
        assert_eq!(pick_match_index(&matches, 0, 0, fwd), Some(0));
        assert_eq!(pick_match_index(&matches, 3, 0, fwd), Some(1));
        assert_eq!(pick_match_index(&matches, 6, 0, fwd), Some(0));
        assert_eq!(pick_match_index(&matches, 6, 0, back), Some(2));
        assert_eq!(pick_match_index(&matches, 0, 0, back), Some(2));
        assert_eq!(pick_match_index(&[], 0, 0, fwd), None);
    }

    #[tokio::test]
    async fn find_enters_copy_mode_and_matches_scrollback() {
        let bytes = numbered_lines_bytes(64);
        let (mut app, _) = app_with_copy_scrollback(&bytes);
        app.state
            .enter_copy_search(&app.terminal_runtimes, CopySearchDirection::Forward);
        assert_eq!(app.state.mode, Mode::Copy);
        assert!(app.state.copy_search.as_ref().expect("search").editing);

        type_query(&mut app, "000005");
        let search = app.state.copy_search.as_ref().expect("search");
        assert_eq!(search.matches.len(), 1);
        assert_eq!(search.current, Some(0));
        assert!(search.editing);
    }

    #[tokio::test]
    async fn find_commit_stops_editing_and_esc_exits_to_terminal() {
        let bytes = numbered_lines_bytes(64);
        let (mut app, _) = app_with_copy_scrollback(&bytes);
        app.state
            .enter_copy_search(&app.terminal_runtimes, CopySearchDirection::Forward);
        type_query(&mut app, "000005");
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Enter, KeyModifiers::empty()));
        assert!(!app.state.copy_search.as_ref().expect("search").editing);

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Esc, KeyModifiers::empty()));
        assert_eq!(app.state.mode, Mode::Terminal);
        assert!(app.state.copy_search.is_none());
        assert!(app.state.copy_mode.is_none());
    }

    #[tokio::test]
    async fn find_esc_while_editing_restores_and_exits() {
        let bytes = numbered_lines_bytes(64);
        let (mut app, pane_id) = app_with_copy_scrollback(&bytes);
        let origin = copy_mode_offset_from_bottom(&app, pane_id);
        app.state
            .enter_copy_search(&app.terminal_runtimes, CopySearchDirection::Forward);
        type_query(&mut app, "000000");
        assert!(copy_mode_offset_from_bottom(&app, pane_id) > origin);

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Esc, KeyModifiers::empty()));
        assert_eq!(app.state.mode, Mode::Terminal);
        assert!(app.state.copy_search.is_none());
        assert!(app.state.copy_mode.is_none());
        assert_eq!(copy_mode_offset_from_bottom(&app, pane_id), origin);
    }

    #[tokio::test]
    async fn slash_search_from_copy_mode_esc_stays_in_copy_mode() {
        let bytes = numbered_lines_bytes(64);
        let (mut app, _) = app_with_copy_scrollback(&bytes);
        app.state.enter_copy_mode(&app.terminal_runtimes);
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('/'), KeyModifiers::empty()));
        assert!(app.state.copy_search.as_ref().expect("search").editing);
        type_query(&mut app, "000010");

        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Esc, KeyModifiers::empty()));
        assert_eq!(app.state.mode, Mode::Copy);
        assert!(app.state.copy_search.is_none());
        assert!(app.state.copy_mode.is_some());
    }

    #[tokio::test]
    async fn find_n_and_shift_n_cycle_matches() {
        let (mut app, _) = app_with_copy_scrollback(b"alpha\r\nbeta\r\nalpha\r\n");
        app.state
            .enter_copy_search(&app.terminal_runtimes, CopySearchDirection::Forward);
        type_query(&mut app, "alpha");
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Enter, KeyModifiers::empty()));
        assert_eq!(
            app.state
                .copy_search
                .as_ref()
                .expect("search")
                .matches
                .len(),
            2
        );

        let idx0 = app.state.copy_search.as_ref().expect("search").current;
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('n'), KeyModifiers::empty()));
        let idx1 = app.state.copy_search.as_ref().expect("search").current;
        assert_ne!(idx0, idx1);
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('n'), KeyModifiers::empty()));
        assert_eq!(
            app.state.copy_search.as_ref().expect("search").current,
            idx0
        );
        app.handle_copy_mode_key(TerminalKey::new(KeyCode::Char('N'), KeyModifiers::SHIFT));
        assert_eq!(
            app.state.copy_search.as_ref().expect("search").current,
            idx1
        );
    }
}
