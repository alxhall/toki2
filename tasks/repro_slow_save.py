#!/usr/bin/env python3
"""Reproduce a slow save locally without logging in or contacting Kleer.

Linux/macOS PTY harness for the released TUI behavior. Run from repo root:
    python3 tasks/repro_slow_save.py target/release/toki-tui

A controlled HTTP server holds PUT /time-tracking/timer until the script releases it.
No production requests or real credentials are used. Windows must be tested separately.
"""

import datetime
import http.server
import os
import pty
import select
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
        self.server.release_save.wait(timeout=15)
        self.respond({"entry": {"registrationId": "r1"}, "timer": None})


def main(binary):
    with tempfile.TemporaryDirectory(prefix="toki-slow-save-") as tmp:
        os.makedirs(os.path.join(tmp, "toki-tui"))
        with open(os.path.join(tmp, "toki-tui", "session"), "w", encoding="utf-8") as session:
            session.write("dummy-local-session")

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Stub)
        server.save_seen = threading.Event()
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
            os.write(master, b"q")
            time.sleep(0.5)
            blocked = process.poll() is None
            print(f"Save request held; quit input processed within 0.5s: {not blocked}")
            server.release_save.set()
            process.wait(timeout=5)
            print(f"Exited after save was released; return code: {process.returncode}")
            if not blocked:
                print("No input stall observed (expected after an off-loop save fix).")
            else:
                print("Reproduced 0.4.0 input stall while awaiting the save response.")
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
    if len(sys.argv) != 2:
        sys.exit("usage: python3 tasks/repro_slow_save.py path/to/toki-tui")
    main(os.path.abspath(sys.argv[1]))
