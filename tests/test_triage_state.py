import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "state", Path(__file__).resolve().parents[1] / "scripts/select_triage_state.py"
)
state = importlib.util.module_from_spec(spec)
spec.loader.exec_module(state)


def run(i, conclusion="success", status="completed"):
    return {"id": i, "conclusion": conclusion, "status": status}


def job(time):
    return {"completed_at": time, "steps": [
        {"started_at": "2026-11-13T00:00:00Z", "conclusion": "success"}
    ]}


class StateSelectionTests(unittest.TestCase):
    def select(self, runs, current=30, jobs=None):
        jobs = jobs or {}
        return state.select_run(runs, current, lambda i: jobs.get(i, []))

    def test_first_run(self):
        self.assertEqual(self.select([run(30)]), (None, False))

    def test_cancelled_pending_request_does_not_hide_state(self):
        self.assertEqual(self.select(
            [run(10), run(20, "cancelled"), run(30)],
            jobs={10: [job("2026-11-13T01:00:00Z")]}
        ), (10, False))

    def test_cancelled_started_run_remains_required(self):
        self.assertEqual(self.select(
            [run(10), run(20, "cancelled")],
            jobs={10: [job("2026-11-13T01:00:00Z")],
                  20: [job("2026-11-13T02:00:00Z")]}
        ), (20, False))

    def test_failed_run_remains_required(self):
        self.assertEqual(self.select(
            [run(20, "failure")],
            jobs={20: [job("2026-11-13T02:00:00Z")]}
        ), (20, False))

    def test_completion_order_not_dispatch_order(self):
        self.assertEqual(self.select(
            [run(10), run(20)],
            jobs={10: [job("2026-11-13T02:00:00Z")],
                  20: [job("2026-11-13T01:00:00Z")]}
        ), (10, False))

    def test_newer_run_makes_current_request_stale(self):
        self.assertEqual(self.select(
            [run(40)], jobs={40: [job("2026-11-13T02:00:00Z")]}
        ), (40, True))

    def test_queued_future_request_is_not_a_state_owner(self):
        self.assertEqual(self.select([run(40, None, "pending")]), (None, False))

    def test_unknown_failure_does_not_become_first_run(self):
        with self.assertRaises(RuntimeError):
            self.select([run(20, "failure")])

    def test_cancelled_rerun_cannot_hide_started_attempt(self):
        self.assertEqual(self.select(
            [run(20, "cancelled")],
            jobs={20: [job("2026-11-13T02:00:00Z"), {"steps": []}]}
        ), (20, False))

    def test_ambiguous_order_stops(self):
        with self.assertRaises(RuntimeError):
            self.select([run(10), run(20)], jobs={
                10: [job("2026-11-13T02:00:00Z")],
                20: [job("2026-11-13T02:00:00Z")]
            })


if __name__ == "__main__":
    unittest.main()
