import importlib.util
import unittest
from pathlib import Path

SCRIPT = Path(__file__).parent / "omnigent_bridge.py"
spec = importlib.util.spec_from_file_location("omnigent_bridge", SCRIPT)
bridge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bridge)


class FakeHerdr:
    def __init__(self, panes, argv_by_pane):
        self.pane_list, self.argv = panes, argv_by_pane
        self.calls = []
        self.tab_labels = {}  # tab id -> label; absent means the default "1"
        self.moved_tab_id = None

    def panes(self):
        return self.pane_list

    def process_info(self, pane_id):
        return {"foreground_processes": [{"argv": self.argv[pane_id]}]}

    def report(self, pane_id, state, seq):
        self.calls.append(("report", pane_id, state, seq))

    def release(self, pane_id, seq):
        self.calls.append(("release", pane_id, seq))

    def close(self, pane_id):
        self.calls.append(("close", pane_id))
        if getattr(self, "close_fails", False):
            raise RuntimeError("boom")

    def move_tab(self, tab_id, label):
        self.calls.append(("move_tab", tab_id, label))
        return self.moved_tab_id or tab_id

    def tab_label(self, tab_id):
        return self.tab_labels.get(tab_id, "1")

    def rename_tab(self, tab_id, label):
        self.calls.append(("rename", tab_id, label))
        self.tab_labels[tab_id] = label


class FakeOmnigent:
    def __init__(self, sessions):
        self.session_list = sessions

    def sessions(self):
        self.sessions_calls = getattr(self, "sessions_calls", 0) + 1
        if getattr(self, "session_fail", False):
            raise RuntimeError("api down")
        return self.session_list

    def session(self, session_id):
        if getattr(self, "session_fail", False):
            raise RuntimeError("api down")
        return next((s for s in self.session_list if s["id"] == session_id), None)

    def archive(self, session_id):
        self.archived = getattr(self, "archived", []) + [session_id]

    def projects(self):
        self.project_calls = getattr(self, "project_calls", 0) + 1
        if getattr(self, "projects_fail", False):
            raise RuntimeError("boom")
        return {"p1": "Alpha"}

    def tmux_sockets(self, session_id):
        self.socket_calls = getattr(self, "socket_calls", 0) + 1
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

    def test_may_rename(self):
        self.assertTrue(bridge.may_rename("3", None))
        self.assertTrue(bridge.may_rename("09:41", None))
        self.assertFalse(bridge.may_rename("my tab", None))
        self.assertTrue(bridge.may_rename("t1", "t1"))
        self.assertFalse(bridge.may_rename("mine", "t1"))

    def test_seq_strictly_increasing(self):
        seq = bridge.Seq(clock_ns=lambda: 5_000_000)
        self.assertEqual([seq.next(), seq.next(), seq.next()], [5000, 5001, 5002])


class TickBase(unittest.TestCase):
    def make(self, sessions, argv, workspace="w9"):
        pane = {"pane_id": "w9:p1", "terminal_id": "t1", "workspace_id": workspace,
                "tab_id": "w9:t1"}
        self.herdr = FakeHerdr([pane], {"w9:p1": argv})
        self.omni = FakeOmnigent(sessions)
        self.now = [0.0]
        self.bridge = bridge.Bridge(self.herdr, self.omni, clock=lambda: self.now[0],
                                    sessions_ttl_secs=0)

    def set_argv(self, argv):
        """Change the pane's foreground command the way a shell would: retitle it."""
        self.herdr.argv["w9:p1"] = argv
        self.herdr.pane_list[0]["terminal_title"] = " ".join(argv)

    def kinds(self):
        return [c[0] for c in self.herdr.calls]


class TickTests(TickBase):
    def test_report_dedupe_heartbeat_release(self):
        self.make([session()], ["tmux", "-S", "/tmp/sock-s1", "attach"])
        self.bridge.tick()
        self.assertEqual(self.kinds(), ["report", "move_tab"])
        self.now[0] = 5
        self.bridge.tick()
        self.assertEqual(self.kinds(), ["report", "move_tab"])  # unchanged: nothing sent
        self.now[0] = 31
        self.bridge.tick()
        self.assertEqual(self.kinds(), ["report", "move_tab", "report"])  # heartbeat
        self.omni.session_list = [session(status="running")]
        self.bridge.tick()
        self.assertEqual(self.herdr.calls[-1][:3], ("report", "w9:p1", "working"))
        self.omni.session_list = [session(archived=True)]
        self.bridge.tick()
        self.assertEqual(self.kinds()[-2:], ["release", "close"])  # archived: pane closed
        self.bridge.tick()
        self.assertEqual(self.kinds().count("release"), 1)
        seqs = [c[3] if c[0] == "report" else c[2] for c in self.herdr.calls if c[0] not in ("move_tab", "rename", "close")]
        self.assertEqual(seqs, sorted(set(seqs)))

    def test_release_when_attach_leaves(self):
        self.make([session()], ["omnigent", "attach", "s1"])
        self.bridge.tick()
        self.set_argv(["-fish"])
        self.bridge.tick()
        self.assertEqual(self.kinds()[-1], "release")

    def test_moves_whole_tab_once_and_on_project_change(self):
        self.make([session()], ["omnigent", "attach", "s1"], workspace="w9")
        self.bridge.tick()
        self.assertEqual(self.herdr.calls[1], ("move_tab", "w9:t1", "Alpha"))
        self.bridge.tick()
        self.assertEqual(self.kinds().count("move_tab"), 1)
        self.omni.session_list = [session(project_id=None)]
        self.bridge.tick()
        self.assertEqual(self.herdr.calls[-1], ("move_tab", "w9:t1", "omnigent"))

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
        self.set_argv(["omnigent", "attach", "s2"])
        self.bridge.tick()
        self.assertEqual(self.bridge.tracked["t1"]["session"], "s2")
        self.assertIn("release", self.kinds())
        self.assertEqual(self.kinds().count("move_tab"), 2)

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


    def test_claim_precedes_move_and_new_attach_is_seen_within_a_recheck(self):
        self.make([session(status="running")], ["-fish"])
        self.bridge.tick()
        self.assertEqual(self.herdr.calls, [])
        self.set_argv(["omnigent", "attach", "s1"])
        self.now[0] = 0.5  # the retitle triggers a check right away
        self.bridge.tick()
        self.assertEqual(self.herdr.calls[0][:3], ("report", "w9:p1", "working"))
        self.assertEqual(self.kinds(), ["report", "move_tab"])

    def test_unchanged_panes_are_not_scanned_every_tick(self):
        self.make([session()], ["-fish"])
        scans = []
        real = self.herdr.process_info
        self.herdr.process_info = lambda pane_id: scans.append(pane_id) or real(pane_id)
        self.bridge.tick()
        self.now[0] = 0.5
        self.bridge.tick()
        self.assertEqual(len(scans), 1)  # nothing changed, fallback not yet due
        self.now[0] = 2
        self.bridge.tick()
        self.assertEqual(len(scans), 2)  # slow fallback
        self.set_argv(["omnigent", "attach", "s1"])
        self.now[0] = 2.1
        self.bridge.tick()
        self.assertEqual(len(scans), 3)
        self.assertEqual(self.kinds()[0], "report")

    def test_claims_optimistically_when_omnigent_api_is_down(self):
        self.make([session()], ["omnigent", "attach", "s1"])
        self.omni.session_fail = True
        self.bridge.tick()
        self.assertEqual(self.herdr.calls[0][:3], ("report", "w9:p1", "idle"))
        self.assertEqual(self.kinds(), ["report"])  # no move without project knowledge
        self.omni.session_fail = False
        self.omni.session_list = [session(status="running")]
        self.bridge.tick()
        self.assertEqual(self.kinds(), ["report", "report", "move_tab"])
        self.assertEqual(self.herdr.calls[1][:3], ("report", "w9:p1", "working"))

    def test_socket_resolution_is_cached_and_sessions_list_ttl(self):
        self.make([session()], ["tmux", "-S", "/tmp/sock-s1", "attach"])
        self.bridge.sessions_ttl_secs = 2.0
        self.bridge.tick()
        self.bridge.tick()
        self.assertEqual(self.omni.sessions_calls, 1)
        self.omni.session_list = [session(), session(id="s2")]
        self.set_argv(["tmux", "-S", "/tmp/sock-s2", "attach"])
        self.bridge.tick()  # new socket: list refreshed, s1 socket lookup cached
        self.assertEqual(self.omni.socket_calls, 2)
        self.assertEqual(self.bridge.tracked["t1"]["session"], "s2")

    def test_tab_named_from_title_and_follows_changes(self):
        self.make([session(title="First")], ["omnigent", "attach", "s1"])
        self.bridge.tick()
        self.assertEqual(self.herdr.tab_labels["w9:t1"], "First")
        self.bridge.tick()
        self.assertEqual(self.kinds().count("rename"), 1)
        self.omni.session_list = [session(title="Second")]
        self.bridge.tick()
        self.assertEqual(self.herdr.calls[-1], ("rename", "w9:t1", "Second"))

    def test_user_named_tab_is_not_renamed(self):
        self.make([session(title="First")], ["omnigent", "attach", "s1"])
        self.herdr.tab_labels["w9:t1"] = "mine"  # named before the bridge saw it
        self.bridge.tick()
        self.assertNotIn("rename", self.kinds())
        self.omni.session_list = [session(title="Second")]
        self.herdr.tab_labels["w9:t1"] = "First"  # even if it later looks ours
        self.bridge.tick()
        self.assertNotIn("rename", self.kinds())

    def test_user_rename_after_bridge_rename_stops_syncing(self):
        self.make([session(title="First")], ["omnigent", "attach", "s1"])
        self.bridge.tick()
        self.herdr.tab_labels["w9:t1"] = "typed by user"
        self.omni.session_list = [session(title="Second")]
        self.bridge.tick()
        self.omni.session_list = [session(title="Third")]
        self.bridge.tick()
        self.assertEqual(self.kinds().count("rename"), 1)
        self.assertEqual(self.herdr.tab_labels["w9:t1"], "typed by user")

    def test_rename_uses_tab_id_returned_by_the_move(self):
        self.make([session(title="First")], ["omnigent", "attach", "s1"])
        self.herdr.moved_tab_id = "w1:t4"
        self.bridge.tick()
        self.assertEqual(self.herdr.calls[-1], ("rename", "w1:t4", "First"))


class LifecycleTests(TickBase):
    """Closing a pane archives the session and archiving closes the pane."""

    def other(self):
        self.herdr.pane_list = [{"pane_id": "w9:p2", "terminal_id": "t2", "workspace_id": "w9",
                                 "tab_id": "w9:t2"}]
        self.herdr.argv["w9:p2"] = ["-fish"]

    def attached(self):
        self.make([session()], ["omnigent", "attach", "s1"])
        self.bridge.tick()
        self.herdr.calls.clear()

    def test_user_close_archives_after_two_polls(self):
        self.attached()
        self.other()
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
        self.other()
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
        self.set_argv(["-fish"])  # attach left: released, untracked
        self.bridge.tick()
        self.other()
        for _ in range(3):
            self.bridge.tick()
        self.assertFalse(hasattr(self.omni, "archived"))

    def test_already_archived_session_is_not_archived_again(self):
        self.attached()
        self.omni.session_list = [session(archived=True)]
        self.other()
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
        self.other()  # the pane is gone now
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
