#!/usr/bin/env python3
"""Bridge omnigent sessions into herdr (status, project -> space).

Polls the local omnigent API and, for every herdr pane that is attached to an
omnigent session, reports the session's state to herdr (`pane report-agent`,
source `omnigent-bridge`, agent `omnigent`) and keeps the pane in the herdr
space named after the session's omnigent project.

Usage (run anywhere herdr's CLI can reach the server):

    python3 scripts/omnigent_bridge.py [--api URL] [--interval SECS]
        [--default-space NAME] [--no-move] [--herdr PATH] [--once]

Env: OMNIGENT_API (default http://127.0.0.1:6767), OMNIGENT_BRIDGE_DEFAULT_SPACE
(default `omnigent`), HERDR_SOCKET_PATH is honoured by the herdr CLI.

How it works
- A pane maps to a session when its foreground argv is `omnigent attach <id>`
  or `tmux -S <sock> attach` (control-mode `-C` clients are ignored) and
  <sock> is the tmux_socket of one of the session's terminal resources.
  process-info runs for panes not yet mapped (retried every --recheck secs)
  and for mapped panes each poll, to notice the attach process leaving.
- State: pending_elicitations_count > 0 -> blocked, running -> working,
  idle -> idle. Reports only on change plus a 30s heartbeat.
- The bridge releases its claim when the session ends or is archived, or when
  the attach process leaves the pane. It never creates panes (sessions
  started elsewhere simply have no pane).
- Project -> space: the space label is the project name (/v1/projects), or
  --default-space when the session has no project. A pane moves only when it
  is first seen or its project changed, into a new tab of the existing space
  (case-insensitive label match) or a new space. --no-move disables moving.

Caveat: `experimental.hide_tabs_with_agents` hides agent-only spaces, so spaces
holding only omnigent panes may disappear from the sidebar spaces list.
"""

import argparse
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request

SOURCE = "omnigent-bridge"
AGENT = "omnigent"
HEARTBEAT_SECS = 30
PROJECT_TTL_SECS = 30
MAX_SESSION_PAGES = 20


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


def find_workspace(workspaces, label):
    wanted = label.casefold()
    for ws in workspaces:
        if ws.get("label", "").casefold() == wanted:
            return ws["workspace_id"]
    return None


def move_args(pane_id, label, workspaces):
    """Return the `pane move` args that put the pane into space `label`."""
    base = ["pane", "move", pane_id]
    workspace_id = find_workspace(workspaces, label)
    if workspace_id:
        return base + ["--new-tab", "--workspace", workspace_id, "--no-focus"]
    return base + ["--new-workspace", "--label", label, "--no-focus"]


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

    def workspaces(self):
        return self.call("workspace", "list")["workspaces"]

    def process_info(self, pane_id):
        return self.call("pane", "process-info", "--pane", pane_id)["process_info"]

    def report(self, pane_id, state, seq):
        self.call("pane", "report-agent", pane_id, "--source", SOURCE,
                  "--agent", AGENT, "--state", state, "--seq", str(seq))

    def release(self, pane_id, seq):
        self.call("pane", "release-agent", pane_id, "--source", SOURCE,
                  "--agent", AGENT, "--seq", str(seq))

    def move(self, args):
        result = self.call(*args)
        return result.get("move_result", {}).get("pane", {}).get("pane_id")


class Omnigent:
    def __init__(self, base):
        self.base = base.rstrip("/")

    def get(self, path):
        with urllib.request.urlopen(self.base + path, timeout=5) as resp:
            return json.load(resp)

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
                 recheck_secs=10, clock=time.monotonic, seq=None):
        self.herdr, self.omnigent = herdr, omnigent
        self.default_space, self.move = default_space, move
        self.recheck_secs, self.clock = recheck_secs, clock
        self.seq = seq or Seq()
        self.tracked = {}  # terminal_id -> dict(session, project, state, sent, pane)
        self.skipped = {}  # terminal_id -> time of last negative process-info
        self.project_names, self.projects_at = {}, None

    def log(self, msg):
        print(msg, file=sys.stderr, flush=True)

    def refresh_projects(self, sessions):
        now = self.clock()
        if self.projects_at is not None and now - self.projects_at <= PROJECT_TTL_SECS:
            return
        self.projects_at = now  # failures and unknown ids also wait for the TTL
        try:
            self.project_names = self.omnigent.projects()
        except Exception as err:
            self.log(f"projects: {err}")

    def resolve_session(self, attach, sessions):
        kind, value = attach
        if kind == "session":
            return value
        wanted = os.path.realpath(value)
        for session in sessions:
            if session_ended(session):
                continue
            try:
                sockets = self.omnigent.tmux_sockets(session["id"])
            except Exception:
                continue
            if any(os.path.realpath(s) == wanted for s in sockets):
                return session["id"]
        return None

    def tick(self):
        sessions = self.omnigent.sessions()
        by_id = {s["id"]: s for s in sessions}
        self.refresh_projects(sessions)
        panes = {p["terminal_id"]: p for p in self.herdr.panes()}
        now = self.clock()

        for terminal_id in [t for t in self.tracked if t not in panes]:
            del self.tracked[terminal_id]  # pane closed: herdr already dropped it
        for terminal_id in [t for t in self.skipped if t not in panes]:
            del self.skipped[terminal_id]

        for terminal_id, pane in panes.items():
            entry = self.tracked.get(terminal_id)
            if entry is None and now - self.skipped.get(terminal_id, -1e9) < self.recheck_secs:
                continue
            try:
                attach = attach_from_process_info(self.herdr.process_info(pane["pane_id"]))
            except Exception as err:
                self.log(f"process-info {pane['pane_id']}: {err}")
                continue
            if entry is not None and attach is not None and attach != entry["attach"]:
                if self.resolve_session(attach, sessions) != entry["session"]:
                    self.release(terminal_id, entry, pane["pane_id"])
                    del self.tracked[terminal_id]
                    entry = None  # re-attached elsewhere: map as new
            if entry is None:
                session_id = self.resolve_session(attach, sessions) if attach else None
                if session_id is None:
                    self.skipped[terminal_id] = now
                    if pane.get("agent") == AGENT:  # claim left by an earlier run
                        self.release(terminal_id, {"state": AGENT, "released": False,
                                                   "session": None}, pane["pane_id"])
                    continue
                entry = self.tracked[terminal_id] = {
                    "session": session_id, "attach": attach, "project": object(), "state": None,
                    "sent": 0, "pane": pane["pane_id"], "released": False}
                self.skipped.pop(terminal_id, None)
            elif attach is None:
                self.release(terminal_id, entry, pane["pane_id"])
                del self.tracked[terminal_id]
                self.skipped[terminal_id] = now
                continue
            entry["attach"] = attach
            session = by_id.get(entry["session"])
            if session is None:  # absent from the list is not proof it ended
                try:
                    session = self.omnigent.session(entry["session"])
                except Exception as err:
                    self.log(f"session {entry['session']}: {err}")
                    continue
            self.sync(terminal_id, entry, pane, session, now)

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
            return
        entry["released"] = False
        project = session.get("project_id")
        if self.move and project != entry["project"]:
            pane_id = self.move_pane(pane, session) or pane_id
            entry["project"] = project
        state = map_status(session)
        if state == entry["state"] and now - entry["sent"] < HEARTBEAT_SECS:
            return
        try:
            self.herdr.report(pane_id, state, self.seq.next())
        except Exception as err:
            self.log(f"report {pane_id}: {err}")
            return
        entry["state"], entry["sent"] = state, now

    def move_pane(self, pane, session):
        label = target_space(session, self.project_names, self.default_space)
        try:
            workspaces = self.herdr.workspaces()
            current = next((w for w in workspaces
                            if w["workspace_id"] == pane["workspace_id"]), None)
            if current and current.get("label", "").casefold() == label.casefold():
                return None
            new_id = self.herdr.move(move_args(pane["pane_id"], label, workspaces))
            self.log(f"moved {pane['pane_id']} -> {new_id} space {label!r}")
            return new_id
        except Exception as err:
            self.log(f"move {pane['pane_id']}: {err}")
            return None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--api", default=os.environ.get("OMNIGENT_API", "http://127.0.0.1:6767"))
    parser.add_argument("--interval", type=float, default=2.0)
    parser.add_argument("--recheck", type=float, default=10.0,
                        help="seconds between process-info checks of unmapped panes")
    parser.add_argument("--default-space",
                        default=os.environ.get("OMNIGENT_BRIDGE_DEFAULT_SPACE", "omnigent"))
    parser.add_argument("--no-move", action="store_true")
    parser.add_argument("--herdr", default="herdr")
    parser.add_argument("--once", action="store_true", help="run a single poll and exit")
    args = parser.parse_args(argv)
    bridge = Bridge(Herdr(args.herdr), Omnigent(args.api), args.default_space,
                    not args.no_move, args.recheck)
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
