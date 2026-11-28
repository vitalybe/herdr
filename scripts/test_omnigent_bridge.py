import importlib.util
import unittest
from pathlib import Path

SCRIPT = Path(__file__).parent / "omnigent_bridge.py"
spec = importlib.util.spec_from_file_location("omnigent_bridge", SCRIPT)
bridge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bridge)


class FakeHerdr:
    def __init__(self, panes, workspaces, argv_by_pane):
        self.pane_list, self.workspace_list, self.argv = panes, workspaces, argv_by_pane
        self.calls = []

    def panes(self):
        return self.pane_list

    def workspaces(self):
        return self.workspace_list

    def process_info(self, pane_id):
        return {"foreground_processes": [{"argv": self.argv[pane_id]}]}

    def report(self, pane_id, state, seq):
        self.calls.append(("report", pane_id, state, seq))

    def release(self, pane_id, seq):
        self.calls.append(("release", pane_id, seq))

    def move(self, args):
        self.calls.append(("move", args))
        return args[2]

    def close(self, pane_id):
        self.calls.append(("close", pane_id))
        if getattr(self, "close_fails", False):
            raise RuntimeError("boom")


class FakeOmnigent:
    def __init__(self, sessions):
        self.session_list = sessions

    def sessions(self):
        return self.session_list

    def session(self, session_id):
        return next((s for s in self.session_list if s["id"] == session_id), None)

    def archive(self, session_id):
        self.archived = getattr(self, "archived", []) + [session_id]

    def projects(self):
        self.project_calls = getattr(self, "project_calls", 0) + 1
        if getattr(self, "projects_fail", False):
            raise RuntimeError("boom")
        return {"p1": "Alpha"}

    def tmux_sockets(self, session_id):
        return ["/tmp/sock-" + session_id]


def session(**fields):
    return {"id": "s1", "status": "idle", "pending_elicitations_count": 0,
            "archived": False, "project_id": "p1", **fields}


class ParseTests(unittest.TestCase):
    def test_attach_shapes(self):
        parse = bridge.parse_attach
        self.assertEqual(parse(["tmux", "-S", "/tmp/a.sock", "attach", "-t", "main"]),
                         ("socket", "/tmp/a.sock"))
        self.assertIsNone(parse(["tmux", "-S", "/tmp/a.sock", "-f", "/dev/null", "-C", "attach"]))
        self.assertEqual(parse(["/x/python", "-m", "omnigent", "attach", "abc", "--server", "u"]),
                         ("session", "abc"))
        self.assertIsNone(parse(["-fish"]))
        self.assertIsNone(parse(["tmux", "-S", "/tmp/a.sock", "-CC", "attach"]))
        self.assertEqual(parse(["omnigent", "attach", "--server", "http://u", "abc"]),
                         ("session", "abc"))

    def test_status_map(self):
        self.assertEqual(bridge.map_status(session(status="running")), "working")
        self.assertEqual(bridge.map_status(session(pending_elicitations_count=2, status="running")), "blocked")
        self.assertEqual(bridge.map_status(session()), "idle")

    def test_move_args(self):
        spaces = [{"workspace_id": "w1", "label": "Alpha"}]
        self.assertEqual(bridge.move_args("w1:p1", "alpha", spaces),
                         ["pane", "move", "w1:p1", "--new-tab", "--workspace", "w1", "--no-focus"])
        self.assertEqual(bridge.move_args("w1:p1", "Beta", spaces),
                         ["pane", "move", "w1:p1", "--new-workspace", "--label", "Beta", "--no-focus"])

    def test_seq_strictly_increasing(self):
        seq = bridge.Seq(clock_ns=lambda: 5_000_000)
        self.assertEqual([seq.next(), seq.next(), seq.next()], [5000, 5001, 5002])


class TickBase(unittest.TestCase):
    def make(self, sessions, argv, workspace="w9"):
        pane = {"pane_id": "w9:p1", "terminal_id": "t1", "workspace_id": workspace}
        self.herdr = FakeHerdr([pane], [{"workspace_id": "w9", "label": "Other"}], {"w9:p1": argv})
        self.omni = FakeOmnigent(sessions)
        self.now = [0.0]
        self.bridge = bridge.Bridge(self.herdr, self.omni, clock=lambda: self.now[0])

    def kinds(self):
        return [c[0] for c in self.herdr.calls]


class TickTests(TickBase):
    def test_report_dedupe_heartbeat_release(self):
        self.make([session()], ["tmux", "-S", "/tmp/sock-s1", "attach"])
        self.bridge.tick()
        self.assertEqual(self.kinds(), ["move", "report"])
        self.now[0] = 5
        self.bridge.tick()
        self.assertEqual(self.kinds(), ["move", "report"])  # unchanged: nothing sent
        self.now[0] = 31
        self.bridge.tick()
        self.assertEqual(self.kinds(), ["move", "report", "report"])  # heartbeat
        self.omni.session_list = [session(status="running")]
        self.bridge.tick()
        self.assertEqual(self.herdr.calls[-1][:3], ("report", "w9:p1", "working"))
        self.omni.session_list = [session(archived=True)]
        self.bridge.tick()
        self.assertEqual(self.kinds()[-2:], ["release", "close"])  # archived: pane closed
        self.bridge.tick()
        self.assertEqual(self.kinds().count("release"), 1)
        seqs = [c[3] if c[0] == "report" else c[2] for c in self.herdr.calls if c[0] not in ("move", "close")]
        self.assertEqual(seqs, sorted(set(seqs)))

    def test_release_when_attach_leaves(self):
        self.make([session()], ["omnigent", "attach", "s1"])
        self.bridge.tick()
        self.herdr.argv["w9:p1"] = ["-fish"]
        self.bridge.tick()
        self.assertEqual(self.kinds()[-1], "release")

    def test_moves_once_and_on_project_change(self):
        self.make([session()], ["omnigent", "attach", "s1"], workspace="w9")
        self.bridge.tick()
        self.assertEqual(self.herdr.calls[0][1],
                         ["pane", "move", "w9:p1", "--new-workspace", "--label", "Alpha", "--no-focus"])
        self.bridge.tick()
        self.assertEqual(self.kinds().count("move"), 1)
        self.omni.session_list = [session(project_id=None)]
        self.bridge.tick()
        self.assertEqual(self.herdr.calls[-1][1][-3:], ["--label", "omnigent", "--no-focus"])

    def test_stale_claim_released_after_restart(self):
        self.make([session()], ["-fish"])
        self.herdr.pane_list[0]["agent"] = "omnigent"
        self.bridge.tick()
        self.assertEqual(self.kinds(), ["release"])

    def test_unmapped_pane_ignored_and_no_move_flag(self):
        self.make([session()], ["-fish"])
        self.bridge.tick()
        self.assertEqual(self.herdr.calls, [])
        self.make([session()], ["omnigent", "attach", "s1"])
        self.bridge.move = False
        self.bridge.tick()
        self.assertEqual(self.kinds(), ["report"])

    def test_missing_from_list_is_not_ended(self):
        self.make([session()], ["omnigent", "attach", "s1"])
        self.bridge.tick()
        self.omni.session_list = [session()]
        self.omni.sessions = lambda: []  # truncated list
        self.omni.session = lambda sid: session(status="running")
        self.bridge.tick()
        self.assertEqual(self.herdr.calls[-1][:3], ("report", "w9:p1", "working"))
        self.assertNotIn("release", self.kinds())

    def test_reattach_to_other_session_remaps(self):
        self.make([session(), session(id="s2", project_id=None)], ["omnigent", "attach", "s1"])
        self.bridge.tick()
        self.herdr.argv["w9:p1"] = ["omnigent", "attach", "s2"]
        self.bridge.tick()
        self.assertEqual(self.bridge.tracked["t1"]["session"], "s2")
        self.assertIn("release", self.kinds())
        self.assertEqual(self.kinds().count("move"), 2)

    def test_projects_failure_and_ttl(self):
        self.make([session(project_id="unknown")], ["omnigent", "attach", "s1"])
        self.omni.projects_fail = True
        self.bridge.tick()
        self.now[0] = 5
        self.bridge.tick()
        self.assertEqual(self.omni.project_calls, 1)
        self.assertIn("report", self.kinds())
        self.now[0] = 40
        self.bridge.tick()
        self.assertEqual(self.omni.project_calls, 2)


class LifecycleTests(TickBase):
    """Closing a pane archives the session and archiving closes the pane."""

    def attached(self):
        self.make([session()], ["omnigent", "attach", "s1"])
        self.bridge.tick()
        self.herdr.calls.clear()

    def test_user_close_archives_after_two_polls(self):
        self.attached()
        self.herdr.pane_list = [{"pane_id": "w9:p2", "terminal_id": "t2", "workspace_id": "w9"}]
        self.herdr.argv["w9:p2"] = ["-fish"]
        self.bridge.tick()
        self.assertFalse(hasattr(self.omni, "archived"))  # one missing poll is not enough
        self.bridge.tick()
        self.assertEqual(self.omni.archived, ["s1"])
        self.assertNotIn("t1", self.bridge.tracked)
        self.bridge.tick()
        self.assertEqual(self.omni.archived, ["s1"])  # no repeat
        self.assertNotIn("close", self.kinds())

    def test_pane_reappearing_resets_the_count(self):
        self.attached()
        pane = self.herdr.pane_list
        self.herdr.pane_list = [{"pane_id": "w9:p2", "terminal_id": "t2", "workspace_id": "w9"}]
        self.herdr.argv["w9:p2"] = ["-fish"]
        self.bridge.tick()
        self.herdr.pane_list = pane + self.herdr.pane_list
        self.bridge.tick()
        self.herdr.pane_list = self.herdr.pane_list[1:]
        self.bridge.tick()
        self.assertFalse(hasattr(self.omni, "archived"))

    def test_empty_or_failed_pane_list_never_archives(self):
        self.attached()
        self.herdr.pane_list = []
        for _ in range(5):
            self.bridge.tick()
        self.assertFalse(hasattr(self.omni, "archived"))
        self.assertIn("t1", self.bridge.tracked)

        def fail():
            raise RuntimeError("server gone")
        self.herdr.panes = fail
        for _ in range(5):
            with self.assertRaises(RuntimeError):
                self.bridge.tick()
        self.assertFalse(hasattr(self.omni, "archived"))

    def test_release_paths_do_not_archive(self):
        self.attached()
        self.herdr.argv["w9:p1"] = ["-fish"]  # attach left: released, untracked
        self.bridge.tick()
        self.herdr.pane_list = [{"pane_id": "w9:p2", "terminal_id": "t2", "workspace_id": "w9"}]
        self.herdr.argv["w9:p2"] = ["-fish"]
        for _ in range(3):
            self.bridge.tick()
        self.assertFalse(hasattr(self.omni, "archived"))

    def test_already_archived_session_is_not_archived_again(self):
        self.attached()
        self.omni.session_list = [session(archived=True)]
        self.herdr.pane_list = [{"pane_id": "w9:p2", "terminal_id": "t2", "workspace_id": "w9"}]
        self.herdr.argv["w9:p2"] = ["-fish"]
        self.bridge.tick()  # archived while tracked: closes (and forgets) the pane
        self.bridge.tick()
        self.bridge.tick()
        self.assertFalse(hasattr(self.omni, "archived"))

    def test_omnigent_archive_closes_pane_without_rearchiving(self):
        self.attached()
        self.omni.session_list = [session(archived=True)]
        self.bridge.tick()
        self.assertEqual([c for c in self.herdr.calls if c[0] == "close"], [("close", "w9:p1")])
        self.assertNotIn("t1", self.bridge.tracked)
        self.herdr.pane_list = [{"pane_id": "w9:p2", "terminal_id": "t2", "workspace_id": "w9"}]
        self.herdr.argv["w9:p2"] = ["-fish"]  # the pane is gone now
        for _ in range(3):
            self.bridge.tick()
        self.assertFalse(hasattr(self.omni, "archived"))
        self.assertEqual(self.kinds().count("close"), 1)

    def test_close_failure_is_not_retried_and_session_stays_archived(self):
        self.attached()
        self.herdr.close_fails = True
        self.omni.session_list = [session(archived=True)]
        self.bridge.tick()
        self.bridge.tick()
        self.assertEqual(self.kinds().count("close"), 1)

    def test_session_archived_before_tracking_is_not_closed(self):
        self.make([session(archived=True)], ["omnigent", "attach", "s1"])
        self.bridge.tick()
        self.assertNotIn("close", self.kinds())

    def test_deleted_session_releases_without_closing(self):
        self.attached()
        self.omni.session_list = []
        self.bridge.tick()
        self.assertNotIn("close", self.kinds())


class PagingTests(unittest.TestCase):
    def test_follows_has_more(self):
        pages = {
            "/v1/sessions?limit=1000": {"data": [{"id": "a"}], "has_more": True, "last_id": "a"},
            "/v1/sessions?limit=1000&after=a": {"data": [{"id": "b"}], "has_more": False},
        }
        omni = bridge.Omnigent("http://x")
        omni.get = lambda path: pages[path]
        self.assertEqual([s["id"] for s in omni.sessions()], ["a", "b"])


if __name__ == "__main__":
    unittest.main()
