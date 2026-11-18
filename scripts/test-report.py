#!/usr/bin/env python3
"""Collect Crow test and fuzz runs into one JSON report per invocation.

`run` starts the command without a shell, streams merged stdout/stderr to the
terminal and `run.log`, and writes `run.json` once the outcome is known. The
command's own exit status is preserved even when parsing or writing fails.

`summary` finds `run.json` files recursively, validates each one, and prints a
Markdown table. A malformed report or an unknown schema is skipped with a
warning; it never changes or suppresses the reports that do load.

Fuzz reporters opt in with `CROW_TEST_EVENTS=1` and print one JSON object per
line prefixed `CROW_EVENT `. `run` sets that for the fuzz modes. The report
contains only run metadata and relative artifacts; inherited environment values
are never copied.

Usage:
    scripts/test-report.py run --mode workspace --out DIR -- cargo test --workspace
    scripts/test-report.py run --mode fuzz --out DIR -- cargo test -p crow-app fuzz_long -- --ignored --nocapture
    scripts/test-report.py summary DIR [DIR ...]
"""

import argparse
import datetime
import json
import os
import platform
import re
import signal
import subprocess
import sys
import tempfile
import time

SCHEMA = 1
EVENT_PREFIX = "CROW_EVENT "
EVENT_MODES = {"fuzz", "fuzz-live"}
ALLOWED_EVENT_ENV = {
    "CROW_FUZZ_SEED",
    "CROW_FUZZ_TREE",
    "CROW_FUZZ_CHURN",
    "CROW_FUZZ_STEPS",
    "CROW_FUZZ_SHRINK_SECS",
    "CROW_FUZZ_MINUTES",
}
RESULTS = {"passed", "failed", "interrupted", "empty", "incomplete", "paused", "collector-error"}
ANSI_RE = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
RUNNING_RE = re.compile(r"^\s*Running\s+(.+?)\s+\(([^)]+)\)\s*$")
DOCTEST_RE = re.compile(r"^\s*Doc-tests\s+(\S+)\s*$")
TEST_RESULT_RE = re.compile(
    r"^\s*test result:\s*(ok|FAILED)\.\s+(\d+)\s+passed;\s+(\d+)\s+failed;\s+"
    r"(\d+)\s+ignored;\s+(\d+)\s+measured;\s+(\d+)\s+filtered out;\s+finished in\s+([0-9.]+)s"
)
FAILURE_HEADER_RE = re.compile(r"^----\s+(.+?)\s+stdout\s+----$")


def utc_now():
    return datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0)


def iso_utc(when):
    return when.strftime("%Y-%m-%dT%H:%M:%SZ")


def strip_ansi(text):
    return ANSI_RE.sub("", text)


def safe_component(text):
    return re.sub(r"[^A-Za-z0-9._-]+", "-", text).strip("-") or "run"


def git_metadata():
    """The actual commit under test, if this tree is a Git checkout."""
    for cwd in (os.getcwd(), os.path.dirname(os.path.dirname(os.path.abspath(__file__)))):
        try:
            head = subprocess.run(
                ["git", "-C", cwd, "rev-parse", "HEAD"],
                check=False,
                capture_output=True,
                text=True,
                timeout=5,
            )
            if head.returncode != 0:
                continue
            dirty = subprocess.run(
                ["git", "-C", cwd, "status", "--porcelain", "--untracked-files=no"],
                check=False,
                capture_output=True,
                text=True,
                timeout=5,
            )
            return head.stdout.strip() or None, bool(dirty.stdout.strip()) if dirty.returncode == 0 else None
        except (OSError, subprocess.SubprocessError):
            continue
    return None, None


def write_json_atomic(path, value):
    directory = os.path.dirname(path) or "."
    os.makedirs(directory, exist_ok=True)
    handle = tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=directory, prefix=".run-", delete=False)
    try:
        json.dump(value, handle, ensure_ascii=False, indent=2)
        handle.write("\n")
        handle.flush()
        os.fsync(handle.fileno())
    finally:
        handle.close()
    os.replace(handle.name, path)


def warn(message):
    sys.stderr.write("test-report: %s\n" % message)
    sys.stderr.flush()


def shell_status(returncode):
    """The status a shell reports for a child; negative means a signal."""
    if returncode is None:
        return None, None
    if returncode < 0:
        return 128 + (-returncode), -returncode
    return returncode, None


def parse_test_binary(source, artifact):
    name = os.path.basename(artifact.rstrip("/")) if artifact else source
    name = re.sub(r"-[0-9a-f]{16}$", "", name)
    return name


def parse_workspace_output(text):
    """Counts per cargo test binary plus the full output of each failed test."""
    binaries = []
    failures = []
    current = None
    failure = None
    for raw in text.splitlines():
        line = strip_ansi(raw)
        running = RUNNING_RE.match(line)
        if running:
            source, artifact = running.group(1), running.group(2)
            current = {
                "binary": parse_test_binary(source, artifact),
                "source": source,
                "artifact": artifact,
                "passed": None,
                "failed": None,
                "ignored": None,
                "measured": None,
                "filtered_out": None,
                "duration_ms": None,
                "result": None,
            }
            binaries.append(current)
            failure = None
            continue
        doctest = DOCTEST_RE.match(line)
        if doctest:
            crate = doctest.group(1)
            current = {
                "binary": "Doc-tests %s" % crate,
                "source": "Doc-tests %s" % crate,
                "artifact": None,
                "passed": None,
                "failed": None,
                "ignored": None,
                "measured": None,
                "filtered_out": None,
                "duration_ms": None,
                "result": None,
            }
            binaries.append(current)
            failure = None
            continue
        result = TEST_RESULT_RE.match(line)
        if result:
            if current is None:
                current = {
                    "binary": "unknown",
                    "source": None,
                    "artifact": None,
                    "passed": None,
                    "failed": None,
                    "ignored": None,
                    "measured": None,
                    "filtered_out": None,
                    "duration_ms": None,
                    "result": None,
                }
                binaries.append(current)
            current.update(
                {
                    "passed": int(result.group(2)),
                    "failed": int(result.group(3)),
                    "ignored": int(result.group(4)),
                    "measured": int(result.group(5)),
                    "filtered_out": int(result.group(6)),
                    "duration_ms": int(round(float(result.group(7)) * 1000)),
                    "result": "ok" if result.group(1) == "ok" else "failed",
                }
            )
            failure = None
            continue
        header = FAILURE_HEADER_RE.match(line)
        if header:
            failure = {
                "binary": current["binary"] if current else None,
                "test": header.group(1),
                "output": line + "\n",
            }
            failures.append(failure)
            continue
        if failure is not None:
            if line.startswith("failures:") or TEST_RESULT_RE.match(line):
                failure = None
                continue
            failure["output"] += raw + "\n"
    return binaries, failures


def totals_for(tests):
    totals = {"binaries": len(tests), "passed": 0, "failed": 0, "ignored": 0, "measured": 0, "filtered_out": 0}
    known = False
    for test in tests:
        for key in ("passed", "failed", "ignored", "measured", "filtered_out"):
            value = test.get(key)
            if isinstance(value, int):
                totals[key] += value
                known = True
    return totals if known else None


def event_object(line):
    """Decode one prefixed event line; return None when it is not an event."""
    stripped = strip_ansi(line).strip()
    if not stripped.startswith(EVENT_PREFIX):
        return None
    payload = stripped[len(EVENT_PREFIX) :].strip()
    try:
        value = json.loads(payload)
    except ValueError as error:
        raise ValueError("malformed event JSON: %s" % error)
    if not isinstance(value, dict):
        raise ValueError("event is not an object")
    if value.get("schema") != SCHEMA:
        raise ValueError("event schema %r is not %d" % (value.get("schema"), SCHEMA))
    if not isinstance(value.get("event"), str):
        raise ValueError("event has no name")
    return value


def optional_int(value):
    if isinstance(value, bool) or not isinstance(value, int):
        return None
    return value


def optional_number(value):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    return value


def clean_replay(replay, warnings):
    if not isinstance(replay, dict):
        return None
    command = replay.get("command")
    command = [entry for entry in command if isinstance(entry, str)] if isinstance(command, list) else None
    raw_env = replay.get("env")
    if isinstance(raw_env, dict):
        dropped = [key for key in raw_env if key not in ALLOWED_EVENT_ENV]
        if dropped:
            warnings.append("ignored replay environment outside CROW_FUZZ_*: %s" % ", ".join(sorted(dropped)))
        env = {
            key: str(value)
            for key, value in raw_env.items()
            if key in ALLOWED_EVENT_ENV and isinstance(value, (str, int, float)) and not isinstance(value, bool)
        }
    else:
        env = {}
    script = replay.get("script")
    if not isinstance(script, str):
        script = None
    script_format = replay.get("script_format")
    if script_format not in ("rust", "step-lines"):
        script_format = "step-lines"
    if command is None and not env and script is None:
        return None
    return {"command": command, "env": env, "script": script, "script_format": script_format}


def write_replay(out_dir, run_id, seed, index, replay, warnings):
    if not replay or not replay.get("script"):
        return None
    extension = "rs" if replay["script_format"] == "rust" else "txt"
    name = "%s-seed-%s-step-%s.%s" % (safe_component(run_id), seed, index, extension)
    relative = os.path.join("replays", name)
    path = os.path.join(out_dir, relative)
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(replay["script"])
            if not replay["script"].endswith("\n"):
                handle.write("\n")
    except OSError as error:
        warnings.append("could not write %s: %s" % (relative, error))
        return None
    return relative.replace(os.sep, "/")


def rewrite_script_command(command, artifact):
    """Point a generated script at its report artifact; keep a caller's path."""
    if not command or artifact is None:
        return command
    rewritten = list(command)
    for index, value in enumerate(rewritten):
        if value == "--script" and index + 1 < len(rewritten):
            rewritten[index + 1] = artifact
            return rewritten
    return rewritten


def seed_entry(event, report_mode):
    seed = optional_int(event.get("seed"))
    if seed is None:
        raise ValueError("seed event has no integer seed")
    mode = event.get("mode") if isinstance(event.get("mode"), str) else report_mode
    churn = event.get("churn")
    return {
        "mode": mode,
        "seed": seed,
        "tree": event.get("tree") if isinstance(event.get("tree"), str) else None,
        "churn": churn if isinstance(churn, bool) else None,
        "requested_steps": optional_int(event.get("requested_steps")),
        "executed_steps": None,
        "duration_ms": None,
        "result": "started",
        "failing_index": None,
        "invariant": None,
        "detail": None,
        "replay": None,
        "frames": None,
    }


def find_seed(seeds, event, report_mode, open_only=True):
    seed = optional_int(event.get("seed"))
    if seed is None:
        raise ValueError("seed event has no integer seed")
    mode = event.get("mode") if isinstance(event.get("mode"), str) else report_mode
    for entry in reversed(seeds):
        if entry["seed"] == seed and entry["mode"] == mode and (not open_only or entry["result"] == "started"):
            return entry
    return None


def frames_entry(event):
    return {
        "heartbeats": optional_int(event.get("heartbeats")),
        "threshold_ms": optional_number(event.get("threshold_ms")),
        "over_budget": optional_int(event.get("over_budget")),
        "worst_frame_ms": optional_number(event.get("worst_frame_ms")),
        "freeze_threshold_ms": optional_number(event.get("freeze_threshold_ms")),
        "freeze": event.get("freeze") if isinstance(event.get("freeze"), bool) else None,
        "slow_steps": optional_int(event.get("slow_steps")),
        "steps": optional_int(event.get("steps")),
        "executed_steps": optional_int(event.get("executed_steps")),
        "duration_ms": optional_number(event.get("duration_ms")),
    }


def decode_events(text, out_dir, run_id, report_mode):
    seeds = []
    frame_summary = None
    warnings = []
    for raw in text.splitlines():
        try:
            event = event_object(raw)
        except ValueError as error:
            warnings.append(str(error))
            continue
        if event is None:
            continue
        kind = event["event"]
        if kind == "seed-start":
            try:
                seeds.append(seed_entry(event, report_mode))
            except ValueError as error:
                warnings.append(str(error))
        elif kind in ("seed-end", "seed-failure"):
            try:
                entry = find_seed(seeds, event, report_mode, open_only=False)
                if entry is None:
                    entry = seed_entry(event, report_mode)
                    seeds.append(entry)
                    warnings.append("seed %s ended without a start" % entry["seed"])
                if kind == "seed-end":
                    result = event.get("result")
                    if entry["result"] != "failed":
                        entry["result"] = result if result in ("passed", "failed", "paused") else "passed"
                else:
                    entry["result"] = "failed"
                    entry["failing_index"] = optional_int(event.get("failing_index"))
                    entry["invariant"] = (
                        event.get("invariant") if isinstance(event.get("invariant"), str) else None
                    )
                    entry["detail"] = event.get("detail") if isinstance(event.get("detail"), str) else None
                entry["executed_steps"] = optional_int(event.get("executed_steps"))
                entry["duration_ms"] = optional_number(event.get("duration_ms"))
                replay = clean_replay(event.get("replay"), warnings)
                if replay:
                    artifact = write_replay(
                        out_dir, run_id, entry["seed"], entry["failing_index"] or 0, replay, warnings
                    )
                    replay["artifact"] = artifact
                    replay["command"] = rewrite_script_command(replay["command"], artifact)
                    replay.pop("script", None)
                    replay.pop("script_format", None)
                    entry["replay"] = replay
            except ValueError as error:
                warnings.append(str(error))
        elif kind == "frame-summary":
            try:
                entry = find_seed(seeds, event, report_mode, open_only=False)
                frames = frames_entry(event)
                if entry is not None:
                    entry["frames"] = frames
                else:
                    warnings.append("frame summary without a seed start")
                frame_summary = frames
            except ValueError as error:
                warnings.append(str(error))
        else:
            warnings.append("ignored unknown event %r" % kind)
    for entry in seeds:
        if entry["result"] == "started":
            entry["result"] = "incomplete"
    return seeds, frame_summary, warnings


def classify(exit_code, seeds, tests, failures, collector_error, mode):
    if exit_code is None:
        return "collector-error" if collector_error else "empty"
    signaled = exit_code < 0
    shell_exit, _ = shell_status(exit_code)
    if signaled:
        return "interrupted"
    if mode in EVENT_MODES:
        if any(seed["result"] == "failed" for seed in seeds):
            return "failed"
        if shell_exit != 0:
            return "failed"
        if any(seed["result"] == "paused" for seed in seeds):
            return "paused"
        if any(seed["result"] == "incomplete" for seed in seeds):
            return "incomplete"
        return "passed" if seeds else "empty"
    if tests:
        failed = any(type(test.get("failed")) is int and test["failed"] > 0 for test in tests) or bool(failures)
        if failed or shell_exit != 0:
            return "failed"
        known = any(
            type(test.get("passed")) is int and type(test.get("failed")) is int for test in tests
        )
        return "passed" if known else "empty"
    if shell_exit != 0:
        return "failed"
    return "empty"


def run_command(args):
    mode = args.mode
    started = utc_now()
    out_dir = args.out
    if not out_dir:
        out_dir = os.path.join("target", "test-reports", "%s-%s" % (safe_component(mode), started.strftime("%Y%m%dT%H%M%SZ")))
    out_dir = os.path.abspath(out_dir)
    try:
        os.makedirs(out_dir, exist_ok=True)
    except OSError as error:
        warn("cannot create %s: %s" % (out_dir, error))
        return 2
    run_id = "%s-%s" % (safe_component(mode), started.strftime("%Y%m%dT%H%M%SZ"))
    suffix = 2
    while os.path.exists(os.path.join(out_dir, "run.json")):
        run_id = "%s-%s-%d" % (safe_component(mode), started.strftime("%Y%m%dT%H%M%SZ"), suffix)
        suffix += 1
        time.sleep(0.001)
        if suffix > 1000:
            break
    log_path = os.path.join(out_dir, "run.log")
    report_path = os.path.join(out_dir, "run.json")
    revision, dirty = git_metadata()
    command = list(args.command)
    child_env = os.environ.copy()
    if mode in EVENT_MODES:
        child_env["CROW_TEST_EVENTS"] = "1"
    else:
        child_env.pop("CROW_TEST_EVENTS", None)

    chunks = []
    collector_error = None
    interrupted = False
    started_monotonic = time.monotonic()
    returncode = None
    command_status = "spawn-error"

    try:
        with open(log_path, "wb") as log:
            try:
                process = subprocess.Popen(
                    command,
                    stdin=None,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    env=child_env,
                    bufsize=0,
                )
                command_status = "exited"
                while True:
                    chunk = process.stdout.read(65536)
                    if not chunk:
                        break
                    chunks.append(chunk)
                    log.write(chunk)
                    log.flush()
                    try:
                        sys.stdout.buffer.write(chunk)
                        sys.stdout.buffer.flush()
                    except AttributeError:
                        sys.stdout.write(chunk.decode("utf-8", "replace"))
                        sys.stdout.flush()
                returncode = process.wait()
                command_status = "signaled" if returncode < 0 else "exited"
            except KeyboardInterrupt:
                interrupted = True
                if "process" in locals():
                    try:
                        process.send_signal(signal.SIGINT)
                        returncode = process.wait(timeout=10)
                    except (OSError, subprocess.TimeoutExpired):
                        process.kill()
                        returncode = process.wait()
                        command_status = "signaled"
                else:
                    returncode = -signal.SIGINT
                command_status = "signaled" if returncode is not None and returncode < 0 else "exited"
            except OSError as error:
                collector_error = "could not run command: %s" % error
                returncode = None
                command_status = "unknown" if "process" in locals() else "spawn-error"
    except OSError as error:
        collector_error = "could not write %s: %s" % (log_path, error)
        returncode = None
        command_status = "spawn-error"

    duration_ms = int(round((time.monotonic() - started_monotonic) * 1000))
    raw = b"".join(chunks).decode("utf-8", "replace")
    text = strip_ansi(raw)
    tests = []
    failures = []
    seeds = []
    frame_summary = None
    parse_warnings = []
    try:
        tests, failures = parse_workspace_output(text)
    except Exception as error:  # the collector must still write its report
        collector_error = collector_error or "test output parse failed: %s" % error
    try:
        seeds, frame_summary, event_warnings = decode_events(text, out_dir, run_id, mode)
        parse_warnings.extend(event_warnings)
    except Exception as error:
        collector_error = collector_error or "event parse failed: %s" % error

    shell_exit, signal_number = shell_status(returncode)
    if interrupted:
        result = "interrupted"
    else:
        result = classify(returncode, seeds, tests, failures, collector_error, mode)
    failure_output = None
    if result == "failed" and not failures and shell_exit not in (None, 0):
        failure_output = text
    report = {
        "schema": SCHEMA,
        "run_id": run_id,
        "revision": revision,
        "revision_dirty": dirty,
        "mode": mode,
        "started_utc": iso_utc(started),
        "duration_ms": duration_ms,
        "host": platform.node() or None,
        "arch": platform.machine() or None,
        "command": command,
        "command_status": command_status,
        "exit_code": shell_exit,
        "signal": signal_number,
        "result": result,
        "collector_error": collector_error,
        "warnings": parse_warnings[:20] + (["..."] if len(parse_warnings) > 20 else []),
        "tests": tests,
        "totals": totals_for(tests),
        "failures": failures,
        "failure_output": failure_output,
        "seeds": seeds,
        "frame_summary": frame_summary,
        "log": "run.log",
    }
    try:
        write_json_atomic(report_path, report)
    except OSError as error:
        warn("could not write %s: %s" % (report_path, error))
        collector_error = collector_error or "report write failed: %s" % error
    if collector_error:
        warn(collector_error)
    if interrupted:
        return 130
    if shell_exit is not None:
        return shell_exit
    return 1


def report_paths(paths):
    found = []
    seen = set()
    for path in paths:
        if os.path.isfile(path):
            candidates = [path]
        elif os.path.isdir(path):
            candidates = []
            for root, _dirs, files in os.walk(path):
                if "run.json" in files:
                    candidates.append(os.path.join(root, "run.json"))
        else:
            warn("%s is not a file or directory" % path)
            continue
        for candidate in candidates:
            real = os.path.realpath(candidate)
            if real not in seen:
                seen.add(real)
                found.append(candidate)
    return sorted(found)


def parse_started(value):
    if not isinstance(value, str):
        raise ValueError("started_utc is not a string")
    try:
        return datetime.datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ")
    except ValueError:
        raise ValueError("started_utc is not UTC ISO-8601")


def validate_report(report):
    if not isinstance(report, dict):
        raise ValueError("report is not an object")
    if report.get("schema") != SCHEMA:
        raise ValueError("unknown schema %r (expected %d)" % (report.get("schema"), SCHEMA))
    for key in ("run_id", "mode", "started_utc", "result"):
        if not isinstance(report.get(key), str):
            raise ValueError("%s is missing or not a string" % key)
    if report["result"] not in RESULTS:
        raise ValueError("unknown result %r" % report["result"])
    if type(report.get("duration_ms")) is not int or report["duration_ms"] < 0:
        raise ValueError("duration_ms is not a non-negative integer")
    if report.get("revision") is not None and not isinstance(report["revision"], str):
        raise ValueError("revision is not a string or null")
    if report.get("revision_dirty") is not None and not isinstance(report["revision_dirty"], bool):
        raise ValueError("revision_dirty is not a boolean or null")
    if report.get("collector_error") is not None and not isinstance(report["collector_error"], str):
        raise ValueError("collector_error is not a string or null")
    tests = report.get("tests")
    if not isinstance(tests, list):
        raise ValueError("tests is not an array")
    if not all(isinstance(test, dict) for test in tests):
        raise ValueError("tests has a non-object entry")
    seeds = report.get("seeds")
    if not isinstance(seeds, list):
        raise ValueError("seeds is not an array")
    for seed in seeds:
        if not isinstance(seed, dict):
            raise ValueError("seeds has a non-object entry")
        if seed.get("frames") is not None and not isinstance(seed["frames"], dict):
            raise ValueError("seed frames is not an object or null")
        if seed.get("replay") is not None and not isinstance(seed["replay"], dict):
            raise ValueError("seed replay is not an object or null")
    if report.get("frame_summary") is not None and not isinstance(report["frame_summary"], dict):
        raise ValueError("frame_summary is not an object or null")
    totals = report.get("totals")
    if totals is not None:
        if not isinstance(totals, dict):
            raise ValueError("totals is not an object or null")
        for key in ("binaries", "passed", "failed", "ignored", "measured", "filtered_out"):
            value = totals.get(key)
            if value is not None and (type(value) is not int or value < 0):
                raise ValueError("totals.%s is not a non-negative integer" % key)
    parse_started(report["started_utc"])
    return report


def relative_link(path, cwd, label):
    relative = os.path.relpath(os.path.abspath(path), cwd).replace(os.sep, "/")
    return "[%s](<%s>)" % (label, relative)


def cell(value):
    if value is None:
        return "—"
    if isinstance(value, float):
        text = "%.1f" % value
    else:
        text = str(value)
    return text.replace("|", "\\|")


def short_revision(report):
    revision = report.get("revision")
    if not isinstance(revision, str) or not revision:
        return "—"
    return "`%s`" % revision[:12]


def test_counts(report):
    totals = report.get("totals")
    if not isinstance(totals, dict):
        return "—"
    parts = []
    if totals.get("binaries"):
        parts.append("%d binaries" % totals["binaries"])
    parts.append("%d passed" % totals.get("passed", 0))
    parts.append("%d failed" % totals.get("failed", 0))
    if totals.get("ignored"):
        parts.append("%d ignored" % totals["ignored"])
    return " · ".join(parts)


def seed_counts(report):
    seeds = report.get("seeds") or []
    if not seeds:
        return "—"
    passed = sum(1 for seed in seeds if seed.get("result") == "passed")
    failed = sum(1 for seed in seeds if seed.get("result") == "failed")
    incomplete = sum(1 for seed in seeds if seed.get("result") in ("incomplete", "paused"))
    parts = ["%d" % len(seeds)]
    if passed:
        parts.append("%d passed" % passed)
    if failed:
        parts.append("%d failed" % failed)
    if incomplete:
        parts.append("%d unfinished" % incomplete)
    throughputs = []
    for seed in seeds:
        steps = seed.get("executed_steps")
        duration = seed.get("duration_ms")
        if isinstance(steps, int) and isinstance(duration, (int, float)) and duration > 0:
            throughputs.append(steps / (duration / 1000.0))
    if throughputs:
        parts.append("%.0f steps/s" % (sum(throughputs) / len(throughputs)))
    failures = [
        "seed %s step %s" % (seed.get("seed"), seed.get("failing_index"))
        for seed in seeds
        if seed.get("result") == "failed" and seed.get("failing_index") is not None
    ]
    if failures:
        parts.append(failures[0] + (" +%d" % (len(failures) - 1) if len(failures) > 1 else ""))
    return " · ".join(parts)


def artifact_links(report, report_path):
    cwd = os.getcwd()
    base = os.path.dirname(os.path.abspath(report_path))
    links = [relative_link(report_path, cwd, "report")]
    log = report.get("log")
    if isinstance(log, str) and os.path.exists(os.path.join(base, log)):
        links.append(relative_link(os.path.join(base, log), cwd, "log"))
    for seed in report.get("seeds") or []:
        replay = seed.get("replay")
        if not isinstance(replay, dict):
            continue
        artifact = replay.get("artifact")
        if isinstance(artifact, str) and os.path.exists(os.path.join(base, artifact)):
            links.append(relative_link(os.path.join(base, artifact), cwd, "replay"))
    return " · ".join(links)


def summary_markdown(reports, skipped):
    lines = ["# Test reports", ""]
    if not reports:
        lines.append("No valid reports.")
        if skipped:
            lines.append("")
            lines.append("Skipped %d invalid report(s)." % len(skipped))
        return "\n".join(lines) + "\n"
    reports = sorted(
        reports,
        key=lambda item: (
            item["started_utc"],
            item.get("revision") or "",
            item["mode"],
            item["run_id"],
        ),
    )
    lines.append("Sorted by UTC, revision and mode.")
    lines.append("")
    lines.append("| UTC | Revision | Mode | Result | Tests | Seeds | Artifacts |")
    lines.append("| --- | --- | --- | --- | --- | --- | --- |")
    for report in reports:
        lines.append(
            "| %s | %s | %s | %s | %s | %s | %s |"
            % (
                cell(report["started_utc"]),
                short_revision(report),
                cell(report["mode"]),
                cell(report["result"]),
                test_counts(report),
                seed_counts(report),
                artifact_links(report, report["_path"]),
            )
        )
    live = [report for report in reports if report["mode"] == "fuzz-live"]
    if live:
        lines.append("")
        lines.append("## Live frame budget")
        lines.append("")
        lines.append("Kept apart from headless runs: the numbers depend on the host.")
        lines.append("")
        lines.append("| UTC | Revision | Host | Seed | Heartbeats | Threshold | Over budget | Worst frame | Result |")
        lines.append("| --- | --- | --- | --- | --- | --- | --- | --- | --- |")
        for report in live:
            seeds = report.get("seeds") or []
            if not seeds:
                seeds = [{"seed": None, "frames": None, "result": report["result"]}]
            for seed in seeds:
                frames = seed.get("frames") if isinstance(seed, dict) else None
                frames = frames if isinstance(frames, dict) else {}
                host = report.get("host") or "—"
                if report.get("arch"):
                    host = "%s (%s)" % (host, report["arch"])
                lines.append(
                    "| %s | %s | %s | %s | %s | %s | %s | %s | %s |"
                    % (
                        cell(report["started_utc"]),
                        short_revision(report),
                        cell(host),
                        cell(seed.get("seed") if isinstance(seed, dict) else None),
                        cell(frames.get("heartbeats")),
                        cell(frames.get("threshold_ms")),
                        cell(frames.get("over_budget")),
                        cell(frames.get("worst_frame_ms")),
                        cell(seed.get("result") if isinstance(seed, dict) else report["result"]),
                    )
                )
    if skipped:
        lines.append("")
        lines.append("Skipped invalid reports: %s." % ", ".join("`%s`" % path for path in skipped))
    return "\n".join(lines) + "\n"


def summarize(paths):
    reports = []
    skipped = []
    for path in report_paths(paths):
        try:
            with open(path, "r", encoding="utf-8") as handle:
                report = json.load(handle)
            report = validate_report(report)
            report["_path"] = path
            reports.append(report)
        except (OSError, ValueError) as error:
            warn("skipping %s: %s" % (path, error))
            skipped.append(path)
    sys.stdout.write(summary_markdown(reports, skipped))
    return 0 if reports else 1


def build_parser():
    parser = argparse.ArgumentParser(description="Collect and summarize Crow test runs.")
    subparsers = parser.add_subparsers(dest="action", required=True)
    run = subparsers.add_parser("run", help="run a command and write run.json/run.log")
    run.add_argument("--mode", required=True, help="workspace, fuzz, fuzz-live, or a local label")
    run.add_argument("--out", help="report directory (default: target/test-reports/<run-id>)")
    run.add_argument("command", nargs=argparse.REMAINDER, help="command after --")
    summary = subparsers.add_parser("summary", help="print a Markdown table for run.json files")
    summary.add_argument("paths", nargs="+", help="report files or directories")
    return parser


def main(argv):
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.action == "run":
        command = list(args.command)
        if command and command[0] == "--":
            command = command[1:]
        if not command:
            parser.error("run needs a command after --")
        args.command = command
        return run_command(args)
    return summarize(args.paths)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
