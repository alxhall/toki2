#!/usr/bin/env python3
"""Assert a stopped, unchanged TUI emits no idle redraw bytes after startup.

Run on Linux/macOS against a local binary: python3 tasks/repro_idle_redraw.py target/debug/toki-tui
Uses dev mode and an isolated config; never accesses a real account or API.
"""
import fcntl
import os
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time


def drain_for(fd, seconds):
    end = time.monotonic() + seconds
    total = 0
    while time.monotonic() < end:
        ready, _, _ = select.select([fd], [], [], min(0.1, max(0, end - time.monotonic())))
        if ready:
            try:
                total += len(os.read(fd, 65536))
            except OSError:
                break
    return total


def main(binary):
    with tempfile.TemporaryDirectory(prefix="toki-idle-redraw-") as config:
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 100, 0, 0))
        env = dict(os.environ, XDG_CONFIG_HOME=config, TERM="xterm-256color")
        process = subprocess.Popen([os.path.abspath(binary), "dev"], stdin=slave,
                                   stdout=slave, stderr=slave, env=env, start_new_session=True)
        os.close(slave)
        try:
            drain_for(master, 4)  # startup throbber and first stable frame
            if process.poll() is not None:
                raise AssertionError("TUI exited before idle measurement")
            output_bytes = drain_for(master, 5)
            print(f"Unchanged, stopped TUI emitted {output_bytes} bytes in 5 seconds")
            assert output_bytes < 100, "Idle event loop still sends ANSI redraws"
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 34, 105, 0, 0))
            os.kill(process.pid, signal.SIGWINCH)
            resize_bytes = drain_for(master, 1)
            print(f"Resize emitted {resize_bytes} bytes")
            assert resize_bytes > 100, "Resize must redraw the screen"
            os.write(master, b" ")  # start a fake dev-mode timer
            drain_for(master, 1)  # discard the one-time layout change
            running_bytes = drain_for(master, 3)
            print(f"Steady running timer emitted {running_bytes} bytes in 3 seconds")
            assert 10 < running_bytes < 500, "Running clock should update, but not redraw at 10 Hz"
            os.write(master, b"q")
            process.wait(timeout=3)
            assert process.returncode == 0, "TUI did not quit cleanly"
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=3)
            os.close(master)


if __name__ == "__main__":
    main(sys.argv[1])
