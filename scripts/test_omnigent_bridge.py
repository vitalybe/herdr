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


class FakeOmnigent:
    def __init__(self, sessions):
        self.session_list = sessions

    def sessions(self):
        return self.session_list

    def projects(self):
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


class TickTests(unittest.TestCase):
    def make(self, sessions, argv, workspace="w9"):
        pane = {"pane_id": "w9:p1", "terminal_id": "t1", "workspace_id": workspace}
        self.herdr = FakeHerdr([pane], [{"workspace_id": "w9", "label": "Other"}], {"w9:p1": argv})
        self.omni = FakeOmnigent(sessions)
        self.now = [0.0]
        self.bridge = bridge.Bridge(self.herdr, self.omni, clock=lambda: self.now[0])

    def kinds(self):
        return [c[0] for c in self.herdr.calls]

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
        self.assertEqual(self.herdr.calls[-1][0], "release")
        self.bridge.tick()
        self.assertEqual(self.kinds().count("release"), 1)
        seqs = [c[3] if c[0] == "report" else c[2] for c in self.herdr.calls if c[0] != "move"]
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


if __name__ == "__main__":
    unittest.main()
