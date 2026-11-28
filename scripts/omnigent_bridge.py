#!/usr/bin/env python3
"""Bridge omnigent sessions into herdr (status, project -> space).

Polls the local omnigent API and, for every herdr pane that is attached to an
omnigent session, reports the session's state to herdr (`pane report-agent`,
source `omnigent-bridge`, agent `omnigent`) and keeps the pane in the herdr
space named after the session's omnigent project.

Usage (run anywhere herdr's CLI can reach the server):

    python3 scripts/omnigent_bridge.py [--api URL] [--interval SECS]
        [--default-space NAME] [--no-move] [--herdr PATH] [--once]
        [--recheck SECS] [--sessions-ttl SECS]

Env: OMNIGENT_API (default http://127.0.0.1:6767), OMNIGENT_BRIDGE_DEFAULT_SPACE
(default `omnigent`), HERDR_SOCKET_PATH is honoured by the herdr CLI.

How it works
- A pane maps to a session when its foreground argv is `omnigent attach <id>`
  or `tmux -S <sock> attach` (control-mode `-C` clients are ignored) and
  <sock> is the tmux_socket of one of the session's terminal resources
  (resolved in parallel and cached per session). process-info runs, in
  parallel, for a pane as soon as `pane list` shows it changed (a shell
  running a command retitles the pane), and otherwise as a fallback every
  --recheck secs for unmapped panes and every 2s for mapped ones (to notice
  the attach process leaving). The session list is
  cached for --sessions-ttl secs; a new `omnigent attach <id>` pane fetches
  just that session, and claims the pane with the session's state at once (an
  idle claim when the omnigent API does not answer), refining later.
- State: pending_elicitations_count > 0 -> blocked, running -> working,
  idle -> idle. Reports only on change plus a 30s heartbeat.
- The bridge releases its claim when the session ends or is archived, or when
  the attach process leaves the pane. It never creates panes (sessions
  started elsewhere simply have no pane).
- Lifecycle sync. Closing a tracked pane (or its tab) in herdr archives the
  session in omnigent (PATCH /v1/sessions/<id> {"archived": true}); archiving
  a session that was live while tracked closes its pane (and the tab, when it
  was the tab's only pane). A pane counts as closed only after it is missing
  from two consecutive polls of a non-empty pane list, and only when the
  session is still unarchived; a failed or empty list never archives. Neither
  direction triggers the other: the bridge forgets a pane before closing it,
  and an already archived session is never archived again. A pane whose attach
  process exits on its own and closes the pane within one poll looks like a
  user close and archives the session.
- Project -> space: the space label is the project name (/v1/projects), or
  --default-space when the session has no project. The pane's whole tab moves
  (`tab space`, keeping the tab and its other panes) when the pane is first
  seen or its project changed, into the existing space (case-insensitive
  label match) or a new space. --no-move disables moving.
- Tab name: the tab takes the omnigent session title (`tab rename`, which is
  advisory in herdr) and follows later title changes. A tab that already has a
  non-default name, or whose name changed since the bridge last set it, is
  treated as user-named and left alone.

Caveat: `experimental.hide_tabs_with_agents` hides agent-only spaces, so spaces
holding only omnigent panes may disappear from the sidebar spaces list.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor

SOURCE = "omnigent-bridge"
AGENT = "omnigent"
HEARTBEAT_SECS = 30
PROJECT_TTL_SECS = 30
MAX_SESSION_PAGES = 20
CLOSE_CONFIRM_TICKS = 2  # consecutive polls a tracked pane must be missing
MAX_WORKERS = 8
NEGATIVE_SOCKET_TTL_SECS = 10
NEW_SESSION_SECS = 30
UNRESOLVED_RECHECK_SECS = 5
SIGNATURE_FIELDS = ("terminal_title", "title", "foreground_cwd", "agent", "revision")


def map_status(session):
    if session.get("pending_elicitations_count", 0) > 0:
        return "blocked"
    return "working" if session.get("status") == "running" else "idle"


def session_ended(session):
    return session is None or bool(session.get("archived"))


def parse_attach(argv):
    """Return ("session", id), ("socket", path) or None for an argv."""
    if "attach" not in argv:
        return None
    rest = argv[argv.index("attach") + 1 :]
    if os.path.basename(argv[0]) == "tmux":
        if {"-C", "-CC"} & set(argv[: argv.index("attach")]):
            return None
        for i, token in enumerate(argv):
            if token == "-S" and i + 1 < len(argv):
                return ("socket", argv[i + 1])
            if token.startswith("-S") and len(token) > 2:
                return ("socket", token[2:])
        return None
    if "omnigent" in " ".join(argv[:3]):
        skip = False
        for token in rest:
            if skip:
                skip = False
            elif token == "--server":
                skip = True
            elif not token.startswith("-"):
                return ("session", token)
    return None


def attach_from_process_info(info):
    for proc in info.get("foreground_processes", []):
        found = parse_attach(proc.get("argv") or [])
        if found:
            return found
    return None


def target_space(session, project_names, default_space):
    return project_names.get(session.get("project_id")) or default_space


DEFAULT_TAB_LABEL = re.compile(r"\d+(:\d{2})?")  # position ("3") or creation time ("09:41")


def may_rename(label, last_set):
    """Whether the bridge may name a tab whose current label is `label`.

    First time: only a default label. Afterwards: only while the label is
    still the one the bridge set, otherwise a user changed it.
    """
    if last_set is not None:
        return label == last_set
    return DEFAULT_TAB_LABEL.fullmatch(label) is not None


class Seq:
    """Strictly increasing microsecond sequence numbers."""

    def __init__(self, clock_ns=time.time_ns):
        self.clock_ns = clock_ns
        self.last = 0

    def next(self):
        self.last = max(self.last + 1, self.clock_ns() // 1000)
        return self.last


class Herdr:
    def __init__(self, binary="herdr"):
        self.binary = binary

    def call(self, *args):
        out = subprocess.run(
            [self.binary, *args], capture_output=True, text=True, timeout=10
        )
        if out.returncode != 0:
            raise RuntimeError(out.stderr.strip() or out.stdout.strip())
        return json.loads(out.stdout)["result"] if out.stdout.strip() else {}

    def panes(self):
        return self.call("pane", "list")["panes"]

    def process_info(self, pane_id):
        return self.call("pane", "process-info", "--pane", pane_id)["process_info"]

    def report(self, pane_id, state, seq):
        self.call("pane", "report-agent", pane_id, "--source", SOURCE,
                  "--agent", AGENT, "--state", state, "--seq", str(seq))

    def release(self, pane_id, seq):
        self.call("pane", "release-agent", pane_id, "--source", SOURCE,
                  "--agent", AGENT, "--seq", str(seq))

    def close(self, pane_id):
        self.call("pane", "close", pane_id)

    def move_tab(self, tab_id, label):
        """Move the whole tab into space `label`; return the tab's new id."""
        result = self.call("tab", "space", label, "--tab", tab_id)
        return result.get("tab", {}).get("tab_id")

    def tab_label(self, tab_id):
        return self.call("tab", "get", tab_id)["tab"]["label"]

    def rename_tab(self, tab_id, label):
        self.call("tab", "rename", tab_id, label)


class Omnigent:
    def __init__(self, base):
        self.base = base.rstrip("/")

    def get(self, path):
        with urllib.request.urlopen(self.base + path, timeout=5) as resp:
            return json.load(resp)

    def archive(self, session_id):
        req = urllib.request.Request(
            f"{self.base}/v1/sessions/{session_id}", data=b'{"archived": true}',
            method="PATCH", headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=5) as resp:
            resp.read()

    def sessions(self):
        """All sessions, following cursor pages (the API defaults to 20)."""
        out, path = [], "/v1/sessions?limit=1000"
        for _ in range(MAX_SESSION_PAGES):
            page = self.get(path)
            out += page["data"]
            if not page.get("has_more") or not page.get("last_id"):
                break
            path = f"/v1/sessions?limit=1000&after={page['last_id']}"
        return out

    def session(self, session_id):
        try:
            return self.get(f"/v1/sessions/{session_id}")
        except urllib.error.HTTPError as err:
            if err.code == 404:
                return None
            raise

    def projects(self):
        return {p["id"]: p["name"] for p in self.get("/v1/projects")["data"]}

    def tmux_sockets(self, session_id):
        data = self.get(f"/v1/sessions/{session_id}/resources/terminals")["data"]
        return [d["metadata"]["tmux_socket"] for d in data
                if d.get("metadata", {}).get("tmux_socket")]


class Bridge:
    def __init__(self, herdr, omnigent, default_space="omnigent", move=True,
                 recheck_secs=1.5, clock=time.monotonic, seq=None, sessions_ttl_secs=2.0,
                 tracked_recheck_secs=2.0):
        self.herdr, self.omnigent = herdr, omnigent
        self.default_space, self.move = default_space, move
        self.recheck_secs, self.clock = recheck_secs, clock
        self.sessions_ttl_secs = sessions_ttl_secs
        self.tracked_recheck_secs = tracked_recheck_secs
        self.pane_sigs = {}  # terminal_id -> what `pane list` showed at the last check
        self.seq = seq or Seq()
        self.tracked = {}  # terminal_id -> dict(session, project, state, sent, pane, ...)
        self.skipped = {}  # terminal_id -> time before which it is not rechecked
        self.missing = {}  # terminal_id -> consecutive polls a tracked pane was absent
        self.project_names, self.projects_at = {}, None
        self.sessions_cache, self.sessions_at = [], None
        self.session_sockets = {}  # session id -> (fetched_at, [realpath sockets])

    def log(self, msg):
        print(msg, file=sys.stderr, flush=True)

    def pmap(self, fn, items):
        items = list(items)
        if len(items) < 2:
            return [fn(i) for i in items]
        with ThreadPoolExecutor(max_workers=min(len(items), MAX_WORKERS)) as pool:
            return list(pool.map(fn, items))

    def refresh_projects(self):
        now = self.clock()
        if self.projects_at is not None and now - self.projects_at <= PROJECT_TTL_SECS:
            return
        self.projects_at = now  # failures and unknown ids also wait for the TTL
        try:
            self.project_names = self.omnigent.projects()
        except Exception as err:
            self.log(f"projects: {err}")

    def sessions(self, force=False):
        now = self.clock()
        if force or self.sessions_at is None or now - self.sessions_at >= self.sessions_ttl_secs:
            self.sessions_at = now  # a failing API is retried after the TTL too
            try:
                self.sessions_cache = self.omnigent.sessions()
            except Exception as err:
                self.log(f"sessions: {err}")
        return self.sessions_cache

    def sockets_of(self, session, now):
        """Real paths of the session's tmux sockets, cached per session."""
        cached = self.session_sockets.get(session["id"])
        is_new = time.time() - session.get("created_at", 0) < NEW_SESSION_SECS
        if cached and not is_new and (cached[1] or now - cached[0] < NEGATIVE_SOCKET_TTL_SECS):
            return cached[1]
        try:
            sockets = [os.path.realpath(s) for s in self.omnigent.tmux_sockets(session["id"])]
        except Exception:
            sockets = []
        self.session_sockets[session["id"]] = (now, sockets)
        return sockets

    def resolve_session(self, attach, now):
        kind, value = attach
        if kind == "session":
            return value
        wanted = os.path.realpath(value)
        for force in (False, True):  # a just-created session may not be listed yet
            live = [s for s in self.sessions(force) if not session_ended(s)]
            hits = self.pmap(lambda s: wanted in self.sockets_of(s, now), live)
            for session, hit in zip(live, hits):
                if hit:
                    return session["id"]
        return None

    def scan(self, pane):
        """Return (terminal_id, attach); attach is False when process-info failed."""
        try:
            return pane["terminal_id"], attach_from_process_info(
                self.herdr.process_info(pane["pane_id"]))
        except Exception as err:
            self.log(f"process-info {pane['pane_id']}: {err}")
            return pane["terminal_id"], False

    def tick(self):
        now = self.clock()
        by_id = {s["id"]: s for s in self.sessions()}
        self.refresh_projects()
        panes = {p["terminal_id"]: p for p in self.herdr.panes()}

        self.reap_closed(panes)
        for terminal_id in [t for t in self.skipped if t not in panes]:
            del self.skipped[terminal_id]

        for terminal_id in [t for t in self.pane_sigs if t not in panes]:
            del self.pane_sigs[terminal_id]

        # process-info is a subprocess plus a process-tree walk, so panes are
        # only checked when `pane list` shows something changed (a shell
        # running a command usually retitles the pane) or a slow fallback is due.
        due = []
        for terminal_id, pane in panes.items():
            sig = tuple(pane.get(k) for k in SIGNATURE_FIELDS)
            entry = self.tracked.get(terminal_id)
            if sig != self.pane_sigs.get(terminal_id):
                due.append(pane)
            elif entry is not None:
                if now - entry["checked"] >= self.tracked_recheck_secs:
                    due.append(pane)
            elif now >= self.skipped.get(terminal_id, -1e9):
                due.append(pane)
            self.pane_sigs[terminal_id] = sig
        attaches = dict(self.pmap(self.scan, due))

        for terminal_id, pane in panes.items():
            entry = self.tracked.get(terminal_id)
            if terminal_id in attaches:
                attach = attaches[terminal_id]
                if attach is False:  # process-info failed, already logged
                    continue
                if entry is not None:
                    entry["checked"] = now
            elif entry is not None:
                attach = entry["attach"]  # not due: no change since the last check
            else:
                continue
            if entry is not None and attach is not None and attach != entry["attach"]:
                if self.resolve_session(attach, now) != entry["session"]:
                    self.release(terminal_id, entry, pane["pane_id"])
                    del self.tracked[terminal_id]
                    entry = None  # re-attached elsewhere: map as new
            if entry is None:
                session_id = self.resolve_session(attach, now) if attach else None
                if session_id is None:
                    # an attach that maps to no session is retried less eagerly
                    wait = UNRESOLVED_RECHECK_SECS if attach else self.recheck_secs
                    self.skipped[terminal_id] = now + wait
                    if pane.get("agent") == AGENT:  # claim left by an earlier run
                        self.release(terminal_id, {"state": AGENT, "released": False,
                                                   "session": None}, pane["pane_id"])
                    continue
                entry = self.tracked[terminal_id] = {
                    "session": session_id, "attach": attach, "project": object(), "state": None,
                    "sent": 0, "pane": pane["pane_id"], "released": False,
                    "tab_label": None, "title_locked": False, "checked": now,
                    "live": False}
                self.skipped.pop(terminal_id, None)
            elif attach is None:
                self.release(terminal_id, entry, pane["pane_id"])
                del self.tracked[terminal_id]
                self.skipped[terminal_id] = now + self.recheck_secs
                continue
            entry["attach"] = attach
            session = by_id.get(entry["session"])
            if session is None:  # absent from the list is not proof it ended
                try:
                    session = self.omnigent.session(entry["session"])
                except Exception as err:
                    self.log(f"session {entry['session']}: {err}")
                    if entry["state"] is not None:
                        continue
                    # first sight and the API is not answering: claim now from
                    # what argv told us, refine when the API is back
                    session = {"id": entry["session"], "optimistic": True}
            self.sync(terminal_id, entry, pane, session, now)

    def reap_closed(self, panes):
        """Archive the session of each tracked pane the user closed in herdr."""
        if not panes:  # server stopping or restoring: absence proves nothing
            self.missing.clear()
            return
        for terminal_id in list(self.tracked):
            if terminal_id in panes:
                self.missing.pop(terminal_id, None)
                continue
            self.missing[terminal_id] = self.missing.get(terminal_id, 0) + 1
            if self.missing[terminal_id] >= CLOSE_CONFIRM_TICKS:
                del self.missing[terminal_id]
                self.archive_closed(self.tracked.pop(terminal_id))

    def archive_closed(self, entry):
        try:
            session = self.omnigent.session(entry["session"])
            if session_ended(session):  # already archived or gone: nothing to do
                return
            self.omnigent.archive(entry["session"])
            self.log(f"archived session {entry['session']} (pane closed)")
        except Exception as err:
            self.log(f"archive {entry['session']}: {err}")

    def close_archived(self, terminal_id, entry, pane_id):
        """Close the pane of a session archived in omnigent. The pane is
        forgotten first so its disappearance is not taken for a user close."""
        del self.tracked[terminal_id]
        self.skipped[terminal_id] = self.clock() + self.recheck_secs
        try:
            self.herdr.close(pane_id)
            self.log(f"closed {pane_id} (session {entry['session']} archived)")
        except Exception as err:
            self.log(f"close {pane_id}: {err}")

    def release(self, terminal_id, entry, pane_id):
        if entry["state"] is None or entry["released"]:
            return
        try:
            self.herdr.release(pane_id, self.seq.next())
            self.log(f"released {pane_id} (session {entry['session']})")
        except Exception as err:
            self.log(f"release {pane_id}: {err}")
        entry["released"], entry["state"] = True, None

    def sync(self, terminal_id, entry, pane, session, now):
        pane_id = pane["pane_id"]
        if session_ended(session):
            self.release(terminal_id, entry, pane_id)
            if entry["live"] and session is not None:  # archived (not deleted) while tracked
                self.close_archived(terminal_id, entry, pane_id)
            return
        entry["live"], entry["released"] = True, False
        # Claim first: the tab move and rename below take extra round trips and
        # must not delay the agent showing up.
        state = map_status(session)
        if state != entry["state"] or now - entry["sent"] >= HEARTBEAT_SECS:
            try:
                self.herdr.report(pane_id, state, self.seq.next())
                entry["state"], entry["sent"] = state, now
            except Exception as err:
                self.log(f"report {pane_id}: {err}")
        if session.get("optimistic"):
            return  # no project or title known yet
        tab_id = pane["tab_id"]
        project = session.get("project_id")
        if self.move and project != entry["project"]:
            tab_id = self.move_tab(pane, session) or tab_id
            entry["project"] = project
        self.sync_title(entry, tab_id, session)

    def move_tab(self, pane, session):
        """Move the pane's whole tab into its project's space; return the tab id."""
        label = target_space(session, self.project_names, self.default_space)
        try:
            new_id = self.herdr.move_tab(pane["tab_id"], label)
            if new_id and new_id != pane["tab_id"]:
                self.log(f"moved tab {pane['tab_id']} -> {new_id} space {label!r}")
            return new_id
        except Exception as err:
            self.log(f"move {pane['tab_id']}: {err}")
            return None

    def sync_title(self, entry, tab_id, session):
        """Name the tab after the session title unless the user named it."""
        title = (session.get("title") or "").strip()
        if not title or entry["title_locked"] or title == entry["tab_label"]:
            return
        try:
            label = self.herdr.tab_label(tab_id)
            if label != title:
                if not may_rename(label, entry["tab_label"]):
                    entry["title_locked"] = True
                    return
                self.herdr.rename_tab(tab_id, title)
            entry["tab_label"] = title
        except Exception as err:
            self.log(f"rename {tab_id}: {err}")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--api", default=os.environ.get("OMNIGENT_API", "http://127.0.0.1:6767"))
    parser.add_argument("--interval", type=float, default=0.5)
    parser.add_argument("--recheck", type=float, default=1.5,
                        help="fallback seconds between process-info checks of unchanged unmapped panes")
    parser.add_argument("--sessions-ttl", type=float, default=2.0,
                        help="seconds the omnigent session list is cached")
    parser.add_argument("--default-space",
                        default=os.environ.get("OMNIGENT_BRIDGE_DEFAULT_SPACE", "omnigent"))
    parser.add_argument("--no-move", action="store_true")
    parser.add_argument("--herdr", default="herdr")
    parser.add_argument("--once", action="store_true", help="run a single poll and exit")
    args = parser.parse_args(argv)
    bridge = Bridge(Herdr(args.herdr), Omnigent(args.api), args.default_space,
                    not args.no_move, args.recheck, sessions_ttl_secs=args.sessions_ttl)
    while True:
        try:
            bridge.tick()
        except Exception as err:
            bridge.log(f"tick failed: {err}")
        if args.once:
            return 0
        time.sleep(args.interval)


if __name__ == "__main__":
    sys.exit(main())
