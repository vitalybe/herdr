# Fork notes

This repository is a fork of [ogulcancelik/herdr](https://github.com/ogulcancelik/herdr).
It carries local features on top of upstream and periodically rebases onto upstream's
`master`.

## Remotes

| Remote | Points at | Role |
| --- | --- | --- |
| `origin` | `vitalybe/herdr` | This fork. `master` here is the fork's own line. |
| `source` | `ogulcancelik/herdr` | Upstream. Read-only; never push. |

```bash
git remote add source https://github.com/ogulcancelik/herdr.git   # if missing
git fetch source
```

## Backups

| What | Where |
| --- | --- |
| The fork's pre-rebase `master` | `backup/fork-pre-upstream-rebase` (`b5dd479c`, 43 commits on the pre-rebase base `f54d8e8c` from 2026-06-30) |
| The last herdr build the old tooling ran against | `~/.herdr/binaries/herdr-0.7.1-fork` |
| The tooling migrated to this line's CLI | branch `herdr-rebase-migration` in `~/hq` and in `~/hq/skills-marketplace` |

Keep the backup branch until the rebased line has been running as the daily build
long enough to trust; it is the only place several fork implementations still
exist (see *Dropped*). The binary copy matters because the installed `herdr` on
PATH is the build artifact in the main checkout, so a `cargo build` there
overwrites the binary currently serving the session.

## Fork features

Landed on the upstream base:

| Feature | Notes |
| --- | --- |
| Agent resume recovery | Recovers from stale persisted session refs, and skips resume when the pane's saved cwd is gone. |
| Movable caret in rename dialogs | Caret moves by char/word, Home/End, insert and delete at the caret, CJK-safe. Merged with upstream's IME host-cursor anchoring: the frame carries the caret position only, so the host terminal's inverted cell *is* the caret. |
| Undo close | Reopens the most recently closed tab or workspace (`keys.undo_close`, `prefix+u`), keeping it as the space's home tab. |
| Tab naming from the agent session | A single-agent tab takes the agent's published session title, and a later session name supersedes a rename that came from tooling. Rides upstream's OSC-title tracker. The tab bar styles a tab by whether it carries a name at all (`Tab::is_numbered`), not by whether the name was typed, so an agent-named tab is not dimmed like a numbered one; named tabs use `palette.text`. In the sidebar, a `tab` token takes the row's name style rather than upstream's muted secondary style, keeping the fork's role table where a tab names an agents row. |
| Session navigator opens in search mode | Typing filters immediately. Escape returns to browse and keeps the query, which is upstream's tested behaviour. |
| Workspace home tab | `Workspace::home_tab` is the tab a space returns to, so restore and space switching do not land on whatever agent tab was focused last. |
| Agents panel row model | `AgentPanelRow` over upstream's entries, with one `compute_agent_panel_row_areas` consumed by both render and hit-testing. Upstream's token/height engine is unchanged underneath. |
| Named line-splits | Divider rows a user can insert, rename, drag, and collapse. A collapsed divider hides its segment down to the next divider and shows the hidden row count. Both bands share one renderer, and collapse state is keyed by `LineSplitSection` so ids from the two counters cannot collide. |
| Manual agent order | A `Manual` sort alongside upstream's sort orders, with drag-to-reorder, persisted per space. |
| Double-click rename | Double-clicking an agent row renames its tab. |
| Sidebar tabs band | A third band listing every tab across spaces, one row per tab. A tab holding at least one agent pane is an agent tab and renders exactly as the agents band renders that agent, showing the most urgent status when the tab holds several (blocked, then done, then working, then idle). Every other tab takes a fixed neon-green `❯` prompt icon and no status. Rows carry `keys.previous_pane` / `keys.next_pane` cycling, drag-to-reorder, and double-click tab rename. A new tab takes the slot beside its natural neighbour rather than the top of the band, so the position a tab is created at - `tab create --index N`, or the tab bar's append - is the position its row takes. |
| Convert pane to tab | A `Convert to tab` item in the pane right-click menu moves the pane out of its split into a tab of its own, through upstream's `pane.move` with a `new_tab` destination. Offered only when the pane shares its tab and the tab is not zoomed. |
| Scratch terminal | A persistent session-modal shell on `keys.scratch_terminal` (`ctrl+backtick`). It rides upstream's popup pane, marked `scratch`: Escape or the toggle key parks it in `AppState::hidden_scratch_popup` with its PTY alive instead of closing it, so reopening keeps the history. Unlike command popups it keeps its pane identity and agent detection, so `keys.scratch_terminal_to_tab` (`ctrl+shift+backtick`) can hand its terminal to `Workspace::create_tab_from_existing_pane` as a normal pane. Command popups keep upstream's behaviour, Escape included. |
| Collapsible sidebar bands | Each band collapses to a header row; dividers stay draggable while a band is collapsed. |
| Hide agent-only spaces | `experimental.hide_tabs_with_agents` hides agent-only spaces from the spaces list, the collapsed rail, and space navigation, and suppresses the space highlight while an agent tab is focused. Config-file only: upstream removed the Settings > Experiments section. |

### Integration notes

Where the fork and upstream both grew a mechanism, upstream's is kept and the
fork's difference re-landed on top. Two collisions between the fork's own
sidebar bands needed a decision rather than a merge:

- Line-split identity. Each band hands out ids from its own counter, so
  `LineSplitId` is paired with a `LineSplitSection` wherever a divider is
  addressed: the rename target and the context menu carry the section, and one
  set of modal helpers dispatches on it.
- Line-split rename follows the pane rename convention. An existing name is
  edited in place; only a nameless split is replaced on the first keystroke.

## Dropped

Kept only in `backup/fork-pre-upstream-rebase`:

- **Copy-mode scrollback search** — upstream shipped its own (`CopyModeSearch*`, wired
  into its menus and plugin surfaces).
- **`herdr agent start <name> ... -- <argv>`** — upstream took that command name for a
  different operation. Upstream's `agent start <name> --kind KIND --pane ID` adopts a
  *supported* agent in an *existing* pane and waits for interactive readiness; the fork's
  created the pane (workspace/tab/split/cwd/env) and ran arbitrary argv. Pane creation
  now goes through `herdr pane split` / `herdr tab create`, and `--parent` was replaced
  by `agent set-parent`.
- **Agent-panel row template language** — the fork itself reverted it (`c9f1727b`), and
  upstream's configurable sidebar tokens cover the same ground.
- **Fork changelog entries** — `docs/next/CHANGELOG.md` follows upstream's releases.
- **Agent parent/child links** - the `PaneState.parent` stable reference, its snapshot and
  JSON API fields, `herdr agent set-parent`, `herdr agent children`, and the sidebar tree
  (indentation, collapse glyph, subagent summary line, `collapsed_agent_keys`, and
  drag-to-reparent with its confirm modal). Agent cycling and the manual order are flat.

### Tooling that depends on the dropped `agent start`

`~/hq/scripts/ag/ag.sh`, `~/hq/herdr-plugins/claude-resume/resume.sh`,
`~/hq/cron/lib/herdr-launch.sh`, and the `task-herdr` skill all launched agents
through the fork's `agent start`. They are migrated on the
`herdr-rebase-migration` branch of each repo and reverted on `main`, because the
migrated form needs a binary from this line: upstream also renamed
`herdr wait output` to `herdr pane wait-output`, which the installed 0.7.1 does
not have. Land the migration when this line becomes the installed build.

The migrated shape is `herdr tab create --cwd ...` followed by
`herdr agent start <name> --kind claude --pane <root_pane>`, which replaces the
old split-then-close-the-shell dance. A prompt is delivered with
`herdr pane send-text` plus a separate Enter rather than as an argv element,
because upstream's `agent start` types a shell command line into the pane.
Each launcher first polls `herdr pane process-info` until the shell owns the
foreground job; the CLI's own busy retry only covers an observation race, not a
shell still running its rc files.

## Syncing with upstream

1. `git fetch source`.
2. Branch off `source/master` and land fork work on top of it. Cherry-pick commit by
   commit rather than rebasing the whole fork line: upstream reworks the same subsystems,
   so most commits need adapting, not merging.
3. Enable `git config rerere.enabled true` in the working tree — the same conflicts recur.
4. When a fork feature meets an upstream feature that does the same job, prefer
   upstream's and re-land the fork's difference on top. Record what was dropped here.
5. Expect fork test fixtures to drift against upstream structs (added fields, renamed
   helpers, `RenderSignal` in place of `AtomicBool`, upstream's `persist::capture`
   signature). `cargo check --all-targets` catches these.
6. `just check` before landing. Note that `windows-lint` needs `rustup` and
   `integration-assets-test` needs `bun`.

The upstream test `live_handoff_keeps_unmanaged_agent_name_bound_to_saved_session`
currently fails on plain `source/master`; it is not a fork regression.
