#!/usr/bin/env python3
"""Fixture tests for scripts/test-report.py.

Run with:
    python3 scripts/test_report_test.py
"""

import json
import os
import signal
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "test-report.py")
PYTHON = sys.executable


def event(**values):
    value = {"schema": 1}
    value.update(values)
    return "CROW_EVENT " + json.dumps(value, separators=(",", ":"))


def collect(mode, out, code, extra_env=None):
    env = os.environ.copy()
    if extra_env:
        env.update(extra_env)
    return subprocess.run(
        [PYTHON, SCRIPT, "run", "--mode", mode, "--out", out, "--", PYTHON, "-c", code],
        check=False,
        capture_output=True,
        text=True,
        env=env,
    )


def load(out):
    with open(os.path.join(out, "run.json"), "r", encoding="utf-8") as handle:
        return json.load(handle)


def write_report(directory, name, report):
    path = os.path.join(directory, name)
    os.makedirs(path, exist_ok=True)
    with open(os.path.join(path, "run.json"), "w", encoding="utf-8") as handle:
        json.dump(report, handle)
    with open(os.path.join(path, "run.log"), "w", encoding="utf-8") as handle:
        handle.write("log for %s\n" % name)
    return path


def base_report(**values):
    value = {
        "schema": 1,
        "run_id": "workspace-20260101T000000Z",
        "revision": "0123456789abcdef",
        "revision_dirty": False,
        "mode": "workspace",
        "started_utc": "2026-01-01T00:00:00Z",
        "duration_ms": 10,
        "host": "test-host",
        "arch": "arm64",
        "command": ["cargo", "test"],
        "command_status": "exited",
        "exit_code": 0,
        "signal": None,
        "result": "passed",
        "collector_error": None,
        "warnings": [],
        "tests": [],
        "totals": None,
        "failures": [],
        "failure_output": None,
        "seeds": [],
        "frame_summary": None,
        "log": "run.log",
    }
    value.update(values)
    return value


class CollectorTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.out = os.path.join(self.temp.name, "report")

    def test_pass_records_counts_per_binary_and_streams_log(self):
        code = "\n".join(
            [
                "import sys",
                "print('   Compiling foo')",
                "print('     Running unittests src/lib.rs (target/debug/deps/crow_core-0123456789abcdef)')",
                "print('running 2 tests')",
                "print('test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s')",
                "print('     Running tests/ops.rs (target/debug/deps/ops-fedcba9876543210)')",
                "print('test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s')",
                "print('plain stderr', file=sys.stderr)",
            ]
        )
        result = collect("workspace", self.out, code)
        self.assertEqual(result.returncode, 0, result.stderr)
        report = load(self.out)
        self.assertEqual(report["result"], "passed")
        self.assertEqual(report["exit_code"], 0)
        self.assertEqual([test["binary"] for test in report["tests"]], ["crow_core", "ops"])
        self.assertEqual(report["tests"][0]["passed"], 2)
        self.assertEqual(report["totals"]["passed"], 3)
        with open(os.path.join(self.out, "run.log"), encoding="utf-8") as handle:
            self.assertIn("plain stderr", handle.read())

    def test_failure_preserves_exit_and_full_failing_output(self):
        code = "\n".join(
            [
                "import sys",
                "print('     Running unittests src/lib.rs (target/debug/deps/crow_core-0123456789abcdef)')",
                "print('running 1 test')",
                "print('---- tests::boom stdout ----')",
                "print('thread tests::boom panicked at src/lib.rs:1')",
                "print('the whole failure output')",
                "print('failures:')",
                "print('    tests::boom')",
                "print('test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s')",
                "print('last stderr line', file=sys.stderr)",
                "sys.exit(7)",
            ]
        )
        result = collect("workspace", self.out, code)
        self.assertEqual(result.returncode, 7)
        report = load(self.out)
        self.assertEqual(report["result"], "failed")
        self.assertEqual(report["exit_code"], 7)
        self.assertEqual(report["failures"][0]["test"], "tests::boom")
        self.assertIn("the whole failure output", report["failures"][0]["output"])
        with open(os.path.join(self.out, "run.log"), encoding="utf-8") as handle:
            self.assertIn("last stderr line", handle.read())

    def test_zero_exit_without_evidence_is_empty_not_passed(self):
        result = collect("workspace", self.out, "print('hello')")
        self.assertEqual(result.returncode, 0)
        report = load(self.out)
        self.assertEqual(report["result"], "empty")
        self.assertEqual(report["tests"], [])

    def test_signaled_child_is_interrupted_and_keeps_status(self):
        result = collect("workspace", self.out, "import os, signal; os.kill(os.getpid(), signal.SIGKILL)")
        self.assertEqual(result.returncode, 137)
        report = load(self.out)
        self.assertEqual(report["result"], "interrupted")
        self.assertEqual(report["exit_code"], 137)
        self.assertEqual(report["signal"], 9)
        self.assertEqual(report["command_status"], "signaled")

    def test_fuzz_seed_and_live_frames_are_decoded(self):
        code = "\n".join(
            [
                "print(%r)" % event(event="seed-start", mode="fuzz", seed=42, tree="small", churn=True, requested_steps=10),
                "print('CROW_FUZZ_SEED=42')",
                "print(%r)" % event(event="seed-end", mode="fuzz", seed=42, executed_steps=10, duration_ms=250, result="passed"),
                "print(%r)" % event(event="seed-start", mode="fuzz-live", seed=7, tree="small", churn=False, requested_steps=None),
                "print(%r)" % event(event="seed-end", mode="fuzz-live", seed=7, executed_steps=10, duration_ms=1000, result="passed"),
                "print(%r)" % event(event="frame-summary", mode="fuzz-live", seed=7, heartbeats=100, threshold_ms=50, over_budget=2, worst_frame_ms=312.4, freeze_threshold_ms=250, freeze=False, slow_steps=3, steps=10, executed_steps=10, duration_ms=1000),
            ]
        )
        result = collect("fuzz", self.out, code)
        self.assertEqual(result.returncode, 0, result.stderr)
        report = load(self.out)
        self.assertEqual(report["result"], "passed")
        self.assertEqual(report["seeds"][0]["executed_steps"], 10)
        self.assertEqual(report["seeds"][1]["frames"]["over_budget"], 2)
        self.assertEqual(report["frame_summary"]["worst_frame_ms"], 312.4)

    def test_gpui_failure_after_seed_start_keeps_replay_artifact(self):
        script = "Keys cmd-k\nSettle\n"
        replay = {
            "command": ["cargo", "run", "--release", "-p", "crow-app", "--features", "fuzz", "--", "--fuzz", "--seed", "42", "--script", "/tmp/script.txt", "--until", "3"],
            "env": {},
            "script": script,
            "script_format": "step-lines",
        }
        code = "\n".join(
            [
                "print(%r)" % event(event="seed-start", mode="fuzz-live", seed=42, tree="small", churn=False, requested_steps=None),
                "print(%r)" % event(event="seed-failure", mode="fuzz-live", seed=42, failing_index=3, invariant="listing", detail="disk changed", executed_steps=3, duration_ms=500, replay=replay),
                "raise SystemExit(101)",
            ]
        )
        result = collect("fuzz-live", self.out, code)
        self.assertEqual(result.returncode, 101)
        report = load(self.out)
        seed = report["seeds"][0]
        self.assertEqual(report["result"], "failed")
        self.assertEqual(seed["result"], "failed")
        self.assertEqual(seed["failing_index"], 3)
        self.assertEqual(seed["invariant"], "listing")
        artifact = seed["replay"]["artifact"]
        self.assertTrue(artifact.startswith("replays/"))
        with open(os.path.join(self.out, artifact), encoding="utf-8") as handle:
            self.assertEqual(handle.read(), script)
        self.assertEqual(seed["replay"]["command"][seed["replay"]["command"].index("--script") + 1], artifact)

    def test_seed_start_without_end_is_incomplete(self):
        code = "print(%r)" % event(event="seed-start", mode="fuzz", seed=1, tree="small", churn=False, requested_steps=1)
        result = collect("fuzz", self.out, code)
        self.assertEqual(result.returncode, 0)
        report = load(self.out)
        self.assertEqual(report["result"], "incomplete")
        self.assertEqual(report["seeds"][0]["result"], "incomplete")

    def test_seed_failure_then_end_keeps_one_failed_seed(self):
        code = "\n".join(
            [
                "print(%r)" % event(event="seed-start", mode="fuzz-live", seed=4, tree="small", churn=False, requested_steps=None),
                "print(%r)" % event(event="seed-failure", mode="fuzz-live", seed=4, failing_index=2, invariant="listing", detail="bad", executed_steps=9, duration_ms=500, replay={"command": ["cargo"], "env": {}, "script": None}),
                "print(%r)" % event(event="seed-end", mode="fuzz-live", seed=4, executed_steps=9, duration_ms=500, result="failed"),
            ]
        )
        result = collect("fuzz-live", self.out, code)
        self.assertEqual(result.returncode, 0)
        report = load(self.out)
        self.assertEqual(len(report["seeds"]), 1)
        self.assertEqual(report["seeds"][0]["result"], "failed")
        self.assertEqual(report["seeds"][0]["failing_index"], 2)

    def test_start_without_end_but_failing_exit_is_failed(self):
        code = "print(%r); raise SystemExit(1)" % event(event="seed-start", mode="fuzz", seed=1, tree="small", churn=False, requested_steps=1)
        result = collect("fuzz", self.out, code)
        self.assertEqual(result.returncode, 1)
        report = load(self.out)
        self.assertEqual(report["result"], "failed")
        self.assertEqual(report["seeds"][0]["result"], "incomplete")

    def test_malformed_event_does_not_hide_the_report(self):
        result = collect("fuzz", self.out, "print('CROW_EVENT {bad json}'); print('CROW_EVENT {\"schema\":99,\"event\":\"seed-start\"}')")
        self.assertEqual(result.returncode, 0)
        report = load(self.out)
        self.assertEqual(report["result"], "empty")
        self.assertGreaterEqual(len(report["warnings"]), 2)

    def test_environment_values_are_not_copied(self):
        result = collect(
            "workspace",
            self.out,
            "print('     Running unittests src/lib.rs (target/debug/deps/crow_core-0123456789abcdef)'); print('test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s')",
            extra_env={"CROW_REPORT_SECRET": "do-not-copy-this"},
        )
        self.assertEqual(result.returncode, 0)
        with open(os.path.join(self.out, "run.json"), encoding="utf-8") as handle:
            self.assertNotIn("do-not-copy-this", handle.read())

    def test_summary_skips_bad_reports_and_sorts_two_revisions(self):
        root = self.temp.name
        first = base_report(
            run_id="workspace-20260101T000000Z",
            revision="1111111111111111111111111111111111111111",
            mode="workspace",
            started_utc="2026-01-01T00:00:00Z",
            log="run.log",
        )
        second = base_report(
            run_id="fuzz-20260201T000000Z",
            revision="2222222222222222222222222222222222222222",
            mode="fuzz",
            started_utc="2026-02-01T00:00:00Z",
            result="failed",
            seeds=[{"seed": 9, "result": "failed", "failing_index": 4, "replay": None, "frames": None}],
        )
        one = write_report(root, "one", first)
        two = write_report(root, "two", second)
        write_report(root, "malformed", "not a report")
        os.makedirs(os.path.join(root, "unknown"))
        with open(os.path.join(root, "unknown", "run.json"), "w", encoding="utf-8") as handle:
            json.dump({"schema": 99}, handle)
        result = subprocess.run(
            [PYTHON, SCRIPT, "summary", root],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("111111111111", result.stdout)
        self.assertIn("222222222222", result.stdout)
        self.assertLess(result.stdout.index("2026-01-01"), result.stdout.index("2026-02-01"))
        self.assertIn("skipping", result.stderr)
        self.assertIn("unknown schema", result.stderr)
        self.assertIn("malformed", result.stderr)
        self.assertTrue(os.path.exists(os.path.join(one, "run.json")))
        self.assertTrue(os.path.exists(os.path.join(two, "run.json")))

    def test_summary_keeps_live_frames_separate_and_marks_absent_as_dash(self):
        root = self.temp.name
        live = base_report(
            run_id="fuzz-live-20260101T000000Z",
            mode="fuzz-live",
            result="passed",
            seeds=[
                {"mode": "fuzz-live", "seed": 5, "result": "passed", "frames": None, "replay": None},
                {"mode": "fuzz-live", "seed": 6, "result": "passed", "frames": {"heartbeats": 10, "threshold_ms": 50, "over_budget": 0, "worst_frame_ms": 0.0}, "replay": None},
            ],
        )
        write_report(root, "live", live)
        result = subprocess.run([PYTHON, SCRIPT, "summary", root], check=False, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0)
        self.assertIn("Live frame budget", result.stdout)
        self.assertIn("| 5 | — | — | — | — |", result.stdout)
        self.assertIn("| 6 | 10 | 50 | 0 | 0.0 |", result.stdout)
        self.assertIn("—", result.stdout)


if __name__ == "__main__":
    unittest.main()
