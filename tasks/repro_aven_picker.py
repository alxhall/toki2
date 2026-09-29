#!/usr/bin/env python3
"""Exercise the Aven picker using a fake CLI and dev-mode TUI; no real tasks/API."""
import fcntl
import os
import pty
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time
from pathlib import Path


def wait_for(fd, needle, seconds=5):
    output = bytearray()
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        ready, _, _ = select.select([fd], [], [], 0.1)
        if ready:
            try:
                output.extend(os.read(fd, 65536))
            except OSError:
                break
            if needle in output:
                return bytes(output)
    raise AssertionError(f"TUI did not show expected fake picker state within {seconds}s")


def wait_for_note(fd, seconds=5):
    # Ratatui's differential renderer may write 'Fixture' and 'Two' with
    # cursor/style escapes between them; inspect only this fixture's output.
    import re
    output = bytearray()
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        ready, _, _ = select.select([fd], [], [], 0.1)
        if ready:
            try:
                output.extend(os.read(fd, 65536))
            except OSError:
                break
            visible = re.sub(rb"\x1b\[[0-9;?]*[ -/]*[@-~]", b"", bytes(output))
            if re.search(rb"Fixture.{0,40}Two", visible):
                return bytes(output)
    raise AssertionError("TUI did not show selected fixture title")


def drain_for(fd, seconds):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        ready, _, _ = select.select([fd], [], [], 0.05)
        if ready:
            try:
                os.read(fd, 65536)
            except OSError:
                break


def main(binary):
    with tempfile.TemporaryDirectory(prefix="toki-aven-picker-") as root:
        config_dir = Path(root) / "toki-tui"
        config_dir.mkdir()
        (config_dir / "config.toml").write_text('task_manager = "aven"\n')
        fake = Path(root) / "aven"
        fake.write_text(
            "#!/bin/sh\n"
            "[ \"$1 $2 $3\" = 'list --open --json' ] || exit 9\n"
            "printf '[{\"title\":\"Fixture One\",\"ref\":\"CHL-X30D\"},"
            "{\"title\":\"Fixture Two\",\"ref\":\"CHL-X30E\"}]'\n"
        )
        fake.chmod(0o700)
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 120, 0, 0))
        env = dict(os.environ, XDG_CONFIG_HOME=root, TERM="xterm-256color",
                   PATH=f"{root}:{os.environ.get('PATH', '')}")
        proc = subprocess.Popen([str(Path(binary).resolve()), "dev"],
                                cwd=root, stdin=slave, stdout=slave, stderr=slave,
                                env=env, start_new_session=True)
        os.close(slave)
        try:
            wait_for(master, b"Toki Timer TUI", 8)
            os.write(master, b"n")  # Note editor
            wait_for(master, b"Ctrl+A")
            os.write(master, b"\x01")  # Ctrl+A: only the fake Aven executable is on PATH first
            output = wait_for(master, b"CHL-X30D")
            assert b"Fixture" in output, "Picker did not show a fixture task title"
            os.write(master, b"j")  # Select second title; no task mutation
            drain_for(master, 0.3)  # Discard the picker redraw before checking note-only output.
            os.write(master, b"\r")
            note_output = wait_for_note(master)
            assert b"CHL-X30E" not in note_output, "Reference was inserted into the note"
            os.write(master, b"\r")  # Confirm note, not a time-entry save
            wait_for_note(master)
            os.write(master, b"q")
            proc.wait(timeout=4)
            assert proc.returncode == 0, "TUI did not quit cleanly"
            print("Fake Aven picker displayed ref, inserted title only; no production API or Aven writes")
        finally:
            if proc.poll() is None:
                proc.terminate()
                proc.wait(timeout=4)
            os.close(master)


if __name__ == "__main__":
    main(sys.argv[1])
