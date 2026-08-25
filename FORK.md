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

## Backup branch

`backup/fork-pre-upstream-rebase` — the fork's `master` as it stood before the first
rebase onto upstream (`b5dd479c`, 43 commits on the pre-rebase base `f54d8e8c` from
2026-06-30). Keep it until the rebased line has been running as the daily build long
enough to trust; it is the only place several fork implementations still exist (see
*Dropped*).

## Fork features

Landed on the upstream base:

| Feature | Notes |
| --- | --- |
| Agent resume recovery | Recovers from stale persisted session refs, and skips resume when the pane's saved cwd is gone. |
| Movable caret in rename dialogs | Caret moves by char/word, Home/End, insert and delete at the caret, CJK-safe. Merged with upstream's IME host-cursor anchoring: the frame carries the caret position only, so the host terminal's inverted cell *is* the caret. |
| Undo close | Reopens the most recently closed tab or workspace (`keys.undo_close`, `prefix+u`). |
| Tab naming from the agent session | A single-agent tab takes the agent's published session title, and a later session name supersedes a rename that came from tooling. Rides upstream's OSC-title tracker. |
| Agent parent/child links | `PaneState.parent`, persisted in the session snapshot and exposed as `parent` on the pane and agent JSON API. |
| `herdr agent set-parent <target> <parent>` | Reparents a running agent; rejects self-parenting and cycles. |
| `herdr agent children [target] [--recursive] [--json]` | Lists direct children or the whole descendant subtree in preorder. Target defaults to `$HERDR_PANE_ID`. |

## Deferred: the sidebar port

The fork's sidebar rework (19 commits) is **not** on the rebased line. It and upstream
both rewrote the same files — the fork added +3631/-977 lines to `src/ui/sidebar.rs`
where upstream added +1764/-376 — because each grew its own agents-panel model:

- Upstream owns row **content**: `resolved_agent_rows()` produces config-driven,
  possibly multi-line rows; `agent_entry_height_in_body()` / `agent_entry_gap()` derive
  geometry from that config; `apply_token_style()` styles each token; the same engine
  renders the spaces section (`src/ui/sidebar/tokens.rs`).
- The fork owns row **structure**: `AgentPanelRow::{Agent, LineSplit}` makes the panel a
  heterogeneous list rather than a pane list, and `compute_agent_panel_row_areas()`
  precomputes rects so hit-testing is a rect scan instead of re-walking heights.

Target shape: keep upstream's token/height engine, and re-introduce `AgentPanelRow` over
upstream's `AgentPanelEntry` with one shared row-area computation used by both render and
input.

Waiting on that port: named line-splits, manual agent order with drag-and-drop, the
agents-panel tree UI and its collapse state (`collapsed_agent_keys`), tree-order agent
cycling, the sidebar panes section, hide-non-agent-panes, collapsible sidebar bands, and
`home_tab` stickiness (`src/app/undo_close.rs` currently calls `switch_workspace_tab`,
not the fork's sticky variant).

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

### Tooling that depends on the dropped `agent start`

Not yet migrated, and required before the rebased line becomes the daily build:
`~/hq/bin/ag`, `~/hq/herdr-plugins/claude-resume/resume.sh`, the `task-herdr` skill, and
the `herdr agent start` example in `CLAUDE.md`.

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
