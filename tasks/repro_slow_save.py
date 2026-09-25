#!/usr/bin/env python3
"""Reproduce a slow save locally without logging in or contacting Kleer.

Linux/macOS PTY harness for the released TUI behavior. Run from repo root:
    python3 tasks/repro_slow_save.py target/release/toki-tui

A controlled HTTP server can hold the save or the subsequent history refresh,
return 500, or drop the save response after receiving the request. No production
requests or real credentials are used. Windows must be tested separately.
"""

import datetime
import http.server
import os
import pty
import select
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time


class Stub(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def respond(self, payload):
        import json

        body = json.dumps(payload).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except BrokenPipeError:
            pass

    def do_GET(self):
        now = datetime.datetime.now(datetime.timezone.utc).isoformat()
        if self.path == "/me":
            self.respond({"id": 1, "email": "test@example.invalid", "fullName": "Test User"})
        elif self.path == "/time-tracking/timer":
            self.respond({"timer": {
                "startTime": now, "projectId": "p1", "projectName": "Test project",
                "activityId": "a1", "activityName": "Test activity", "note": "",
                "hours": 0, "minutes": 0, "seconds": 0,
            }})
        elif self.path.startswith("/time-tracking/time-entries"):
            if self.server.mode == "slow-refresh" and self.server.save_seen.is_set():
                self.server.refresh_seen.set()
                self.server.release_save.wait(timeout=15)
            self.respond([])
        elif self.path == "/time-tracking/projects":
            self.respond([{"projectId": "p1", "projectName": "Test project"}])
        elif self.path.startswith("/time-tracking/time-info"):
            self.respond({
                "workedHours": 0, "scheduledHours": 40, "remainingHours": 40,
                "absenceHours": 0, "coveredHours": 0, "periodFlexHours": 0,
            })
        else:
            self.send_error(404)

    def do_PUT(self):
        if self.path != "/time-tracking/timer":
            self.send_error(404)
            return
        self.rfile.read(int(self.headers.get("Content-Length", "0")))
        self.server.save_seen.set()
        if self.server.mode != "slow-refresh":
            self.server.release_save.wait(timeout=15)
        if self.server.mode == "lost-response":
            self.connection.shutdown(socket.SHUT_RDWR)
            self.connection.close()
        elif self.server.mode == "server-error":
            self.send_error(500)
        else:
            self.respond({"entry": {"registrationId": "r1"}, "timer": None})


def main(binary, mode="slow-save", require_responsive=False):
    with tempfile.TemporaryDirectory(prefix="toki-slow-save-") as tmp:
        os.makedirs(os.path.join(tmp, "toki-tui"))
        with open(os.path.join(tmp, "toki-tui", "session"), "w", encoding="utf-8") as session:
            session.write("dummy-local-session")

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Stub)
        server.mode = mode
        server.save_seen = threading.Event()
        server.refresh_seen = threading.Event()
        server.release_save = threading.Event()
        threading.Thread(target=server.serve_forever, daemon=True).start()
        master, slave = pty.openpty()
        import fcntl
        import termios

        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 100, 0, 0))
        env = dict(os.environ, XDG_CONFIG_HOME=tmp, TERM="xterm-256color",
                   TOKI_TUI_API_URL=f"http://127.0.0.1:{server.server_port}")
        process = subprocess.Popen([binary, "run"], stdin=slave, stdout=slave,
                                   stderr=slave, env=env, start_new_session=True)
        os.close(slave)
        stop_drain = threading.Event()

        def drain():
            while not stop_drain.is_set():
                try:
                    ready, _, _ = select.select([master], [], [], 0.1)
                    if ready:
                        os.read(master, 65536)  # discard bounded output; do not retain frames
                except OSError:
                    break

        threading.Thread(target=drain, daemon=True).start()
        try:
            time.sleep(1.5)  # bootstrap finishes before issuing save
            if process.poll() is not None:
                raise RuntimeError("TUI exited before save; check stub response shapes")
            os.write(master, b"\x13")  # Ctrl+S -> save dialog
            time.sleep(0.3)
            os.write(master, b"1")  # save and stop
            if not server.save_seen.wait(5):
                raise RuntimeError("No PUT /time-tracking/timer after Ctrl+S, 1")
            if mode == "slow-refresh" and not server.refresh_seen.wait(5):
                raise RuntimeError("No history refresh after successful save")
            os.write(master, b"q")
            time.sleep(0.5)
            blocked = process.poll() is None
            print(f"{mode} response held; quit input processed within 0.5s: {not blocked}")
            server.release_save.set()
            process.wait(timeout=5)
            print(f"Exited after stub response was released; return code: {process.returncode}")
            if not blocked:
                print("No input stall observed (expected after an off-loop I/O fix).")
            else:
                print(f"Reproduced 0.4.0 input stall during {mode}.")
            if require_responsive and blocked:
                raise AssertionError("TUI ignored quit input while save was pending")
        finally:
            server.release_save.set()
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            stop_drain.set()
            os.close(master)
            server.shutdown()
            server.server_close()


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary")
    parser.add_argument("--mode", choices=("slow-save", "lost-response", "server-error", "slow-refresh"), default="slow-save")
    parser.add_argument("--require-responsive", action="store_true")
    args = parser.parse_args()
    main(os.path.abspath(args.binary), mode=args.mode, require_responsive=args.require_responsive)
