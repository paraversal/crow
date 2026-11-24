#!/usr/bin/env python3
"""Preview each file in ignore/icons/extract on the local Crow website."""

import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]
SITE_ADDRESS = ("127.0.0.1", 8765)
SITE_URL = f"http://{SITE_ADDRESS[0]}:{SITE_ADDRESS[1]}/"


def site_is_listening():
    try:
        with socket.create_connection(SITE_ADDRESS, timeout=0.2):
            return True
    except ConnectionRefusedError:
        return False


def wait_for_site(server):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        status = server.poll()
        if status is not None:
            raise RuntimeError(f"scripts/site.sh exited with status {status}.")
        if site_is_listening():
            return
        time.sleep(0.1)
    raise RuntimeError(f"The preview site did not start at {SITE_URL} within 30 seconds.")


def stop_site(server):
    if server.poll() is not None:
        return
    # During the build, site.sh can have rsync or sips running as a child.
    try:
        os.killpg(server.pid, signal.SIGTERM)
        server.wait(timeout=5)
    except ProcessLookupError:
        server.wait()
    except subprocess.TimeoutExpired:
        os.killpg(server.pid, signal.SIGKILL)
        server.wait()


def show_progress(reviewed, total):
    width = 24
    filled = reviewed * width // total
    bar = "#" * filled + "-" * (width - filled)
    print(f"[{bar}] {reviewed}/{total} reviewed · {total - reviewed} left", flush=True)


def preview_icons():
    source = ROOT / "ignore/icons/extract"
    icon = ROOT / "assets/app-icon.png"
    candidates = sorted(path for path in source.iterdir() if path.is_file())
    total = len(candidates)
    if not total:
        print(f"No files found in {source.relative_to(ROOT)}.")
        return
    if site_is_listening():
        raise RuntimeError(f"{SITE_URL} is already in use. Stop that server before previewing icons.")

    # Stage beside the icon so that each swap and the final restore are atomic.
    with tempfile.TemporaryDirectory(prefix=".icon-preview-", dir=icon.parent) as temp:
        original = Path(temp) / "original.png"
        staged = Path(temp) / "candidate.png"
        had_original = icon.exists() or icon.is_symlink()
        if had_original:
            shutil.copy2(icon, original, follow_symlinks=False)
        try:
            for reviewed, candidate in enumerate(candidates):
                show_progress(reviewed, total)
                print(f"Preview {reviewed + 1}/{total} · {candidate.name}", flush=True)
                shutil.copy2(candidate, staged)
                staged.replace(icon)

                server = subprocess.Popen(
                    ["scripts/site.sh", "--serve"], cwd=ROOT, start_new_session=True
                )
                try:
                    wait_for_site(server)
                    print(f"Open or refresh {SITE_URL}")
                    while input("Type n and press Enter to advance: ").strip() != "n":
                        pass
                finally:
                    stop_site(server)
            show_progress(total, total)
        finally:
            if had_original:
                original.replace(icon)
            else:
                icon.unlink(missing_ok=True)


def terminate(_signal, _frame):
    raise SystemExit(143)


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, terminate)
    try:
        preview_icons()
    except EOFError:
        print("\nPreview ended.")
    except KeyboardInterrupt:
        print("\nPreview interrupted.")
        sys.exit(130)
    except (OSError, RuntimeError) as exc:
        print(f"Icon preview: {exc}", file=sys.stderr)
        sys.exit(1)
