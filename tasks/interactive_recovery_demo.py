#!/usr/bin/env python3
"""Hands-on lost-response TUI demo with fake localhost data and isolated config.

Run in an interactive WSL/Linux terminal with a Linux TUI binary:
  python3 tasks/interactive_recovery_demo.py /tmp/toki-tui-agent-target/debug/toki-tui
Never uses the real account, cookie, Toki API, or provider.
"""
import datetime
import fcntl
import http.server
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import tty

from repro_slow_save import Stub


def main(binary):
    if not sys.stdin.isatty() or not sys.stdout.isatty():
        raise SystemExit("Run this demo directly in an interactive WSL/Linux terminal.")
    executable = Path(binary).resolve(strict=True)
    with executable.open("rb") as file:
        if file.read(4) != b"\x7fELF":
            raise SystemExit("Use the Linux TUI binary, not a Windows .exe.")

    print("LOCAL DEMO ONLY: fake account, fake project, throwaway config, loopback API.")
    print("A fake timer is already running for over two minutes. Press Ctrl+S, then 1.")
    print("The fake server will commit an entry but drop the response. Press r to review;")
    print("check the fake entry and server timer, then c followed by y to clear the local guard.")
    print("Press q to exit the TUI, or Ctrl+C to stop this demo. No real data is touched.")
    try:
        input("Press Enter to launch... ")
    except (KeyboardInterrupt, EOFError):
        print("\nDemo cancelled before launch.")
        return

    with tempfile.TemporaryDirectory(prefix="toki-recovery-demo-") as config:
        root = Path(config) / "toki-tui"
        root.mkdir()
        (root / "session").write_text("fake-local-only-session", encoding="utf-8")

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Stub)
        server.mode = "interactive-demo"
        server.timer_started_at = datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(minutes=2)
        server.save_seen = threading.Event()
        server.refresh_seen = threading.Event()
        server.release_save = threading.Event()
        server.release_save.set()  # no artificial wait before dropping the response
        server.request_lock = threading.Lock()
        server.save_requests = 0
        server.history_reads = 0
        server_thread = threading.Thread(target=server.serve_forever, daemon=True)
        server_thread.start()

        master, slave = pty.openpty()
        stdout_fd = sys.stdout.fileno()
        stdin_fd = sys.stdin.fileno()
        try:
            size = fcntl.ioctl(stdout_fd, termios.TIOCGWINSZ, b"\0" * 8)
        except OSError:
            size = struct.pack("HHHH", 32, 100, 0, 0)
        fcntl.ioctl(slave, termios.TIOCSWINSZ, size)
        env = {k: v for k, v in os.environ.items() if not k.startswith("TOKI_TUI_")}
        env.update(XDG_CONFIG_HOME=config, TOKI_TUI_API_URL=f"http://127.0.0.1:{server.server_port}",
                   TERM=os.environ.get("TERM", "xterm-256color"), NO_PROXY="127.0.0.1,localhost",
                   no_proxy="127.0.0.1,localhost", HTTP_PROXY="", HTTPS_PROXY="", ALL_PROXY="",
                   http_proxy="", https_proxy="", all_proxy="")
        process = subprocess.Popen([str(executable), "run"], stdin=slave, stdout=slave,
                                   stderr=slave, env=env, start_new_session=True)
        os.close(slave)
        old_tty = termios.tcgetattr(stdin_fd)
        old_sigwinch = signal.getsignal(signal.SIGWINCH)
        old_sigterm = signal.getsignal(signal.SIGTERM)
        old_sighup = signal.getsignal(signal.SIGHUP)

        def abort(_signum, _frame):
            raise SystemExit(0)

        def resize(_signum, _frame):
            try:
                size = fcntl.ioctl(stdout_fd, termios.TIOCGWINSZ, b"\0" * 8)
                fcntl.ioctl(master, termios.TIOCSWINSZ, size)
                os.kill(process.pid, signal.SIGWINCH)
            except OSError:
                pass

        try:
            signal.signal(signal.SIGWINCH, resize)
            signal.signal(signal.SIGTERM, abort)
            signal.signal(signal.SIGHUP, abort)
            tty.setraw(stdin_fd)
            while True:
                ready, _, _ = select.select([stdin_fd, master], [], [], 0.1)
                if stdin_fd in ready:
                    keys = os.read(stdin_fd, 4096)
                    if not keys or b"\x03" in keys:  # Ctrl+C aborts only the demo
                        break
                    os.write(master, keys)
                if master in ready:
                    try:
                        output = os.read(master, 65536)
                    except OSError:
                        break  # PTY closes after TUI exit
                    if not output:
                        break
                    os.write(stdout_fd, output)
                # Drain the PTY until it closes so the TUI's final terminal
                # restoration sequences are forwarded before returning.
        finally:
            termios.tcsetattr(stdin_fd, termios.TCSADRAIN, old_tty)
            signal.signal(signal.SIGWINCH, old_sigwinch)
            signal.signal(signal.SIGTERM, old_sigterm)
            signal.signal(signal.SIGHUP, old_sighup)
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            os.close(master)
            server.shutdown()
            server.server_close()

        save_requests = server.save_requests

    print(f"\nDemo finished: fake save requests received: {save_requests}.")
    print("Fake account, recovery guard, and session were deleted with the temporary config.")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("Usage: python3 tasks/interactive_recovery_demo.py <Linux TUI binary>")
    main(sys.argv[1])
