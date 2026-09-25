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
            if self.server.save_seen.is_set():
                self.server.refresh_seen.set()
                if self.server.mode == "slow-refresh":
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
        with self.server.request_lock:
            self.server.save_requests += 1
        self.server.save_seen.set()
        if self.server.mode != "slow-refresh":
            self.server.release_save.wait(timeout=15)
        if self.server.mode in ("lost-response", "deferred-history"):
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
        server.save_requests = 0
        server.request_lock = threading.Lock()
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

        def drain(fd, stop):
            while not stop.is_set():
                try:
                    ready, _, _ = select.select([fd], [], [], 0.1)
                    if ready:
                        os.read(fd, 65536)  # discard bounded output; do not retain frames
                except OSError:
                    break

        threading.Thread(target=drain, args=(master, stop_drain), daemon=True).start()

        def invoke_recovery(answer):
            recovery_master, recovery_slave = pty.openpty()
            child = subprocess.Popen([binary, "resolve-save"], stdin=recovery_slave,
                                     stdout=recovery_slave, stderr=recovery_slave,
                                     env=env, start_new_session=True)
            os.close(recovery_slave)
            try:
                output = bytearray()
                deadline = time.monotonic() + 5
                while b"CLEAR VERIFIED" not in output and time.monotonic() < deadline:
                    ready, _, _ = select.select([recovery_master], [], [], 0.1)
                    if ready:
                        output.extend(os.read(recovery_master, 8192))
                        del output[:-8192]
                    if child.poll() is not None:
                        break
                assert b"CLEAR VERIFIED" in output, "Recovery did not reach the verification prompt"
                os.write(recovery_master, answer)
                assert child.wait(timeout=5) == 0, "Recovery command failed"
            finally:
                if child.poll() is None:
                    child.terminate()
                    child.wait(timeout=2)
                os.close(recovery_master)

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
            if require_responsive:
                concurrent = subprocess.run([binary, "resolve-save"], env=env,
                                            stdin=subprocess.DEVNULL, capture_output=True,
                                            timeout=5, check=False)
                assert concurrent.returncode != 0, "Recovery ran while the TUI still held its lock"
            if mode == "deferred-history":
                os.write(master, b"h")
                time.sleep(0.2)
                assert not server.refresh_seen.is_set(), "Fetched history before resolving in-flight save"
                server.release_save.set()  # drop the response; the outcome remains unknown
                assert server.refresh_seen.wait(5), "Did not fetch history after uncertain save"
            else:
                os.write(master, b"\x13")  # repeated Ctrl+S while request/refresh is held
                os.write(master, b"1")
                time.sleep(0.2)
            os.write(master, b"q")
            time.sleep(0.5)
            blocked = process.poll() is None
            print(f"{mode} response held; quit input processed within 0.5s: {not blocked}")
            if require_responsive and not blocked:
                marker = os.path.join(tmp, "toki-tui", "pending-save.json")
                assert os.path.exists(marker) == (mode != "slow-refresh"), "Incorrect recovery record after quit"
                assert server.save_requests == 1, "Repeated key sent a duplicate save"
            server.release_save.set()
            process.wait(timeout=5)
            print(f"Exited after stub response was released; return code: {process.returncode}")
            if not blocked:
                print("No input stall observed (expected after an off-loop I/O fix).")
            else:
                print(f"Reproduced 0.4.0 input stall during {mode}.")
            if require_responsive and blocked:
                raise AssertionError("TUI ignored quit input while save was pending")
            if require_responsive and mode != "slow-refresh":
                # Launch again with the same isolated config: never replay an uncertain write.
                master2, slave2 = pty.openpty()
                fcntl.ioctl(slave2, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 100, 0, 0))
                process2 = subprocess.Popen([binary, "run"], stdin=slave2, stdout=slave2,
                                            stderr=slave2, env=env, start_new_session=True)
                os.close(slave2)
                stop2 = threading.Event()
                threading.Thread(target=drain, args=(master2, stop2), daemon=True).start()
                try:
                    time.sleep(1.2)
                    assert process2.poll() is None, "TUI failed to restart with pending record"
                    os.write(master2, b"\x13")
                    time.sleep(0.2)
                    os.write(master2, b"1")
                    time.sleep(0.2)
                    os.write(master2, b"q")
                    process2.wait(timeout=5)
                    assert server.save_requests == 1, "Restart replayed an unresolved save"
                    assert os.path.exists(marker), "Restart lost the recovery record"
                    print("Restart retained the unresolved record; no duplicate PUT")
                finally:
                    if process2.poll() is None:
                        process2.terminate()
                        process2.wait(timeout=2)
                    stop2.set()
                    os.close(master2)
            if require_responsive and mode == "lost-response":
                import json
                with open(marker, encoding="utf-8") as pending_file:
                    pending = json.load(pending_file)
                timestamp = int(datetime.datetime.fromisoformat(
                    pending["attempted_at"].replace("Z", "+00:00")).timestamp())
                phrase = f"CLEAR VERIFIED {pending['user_id']} {timestamp}\n".encode()
                invoke_recovery(b"no\n")
                assert os.path.exists(marker), "A refusal cleared the recovery record"
                invoke_recovery(phrase)
                assert not os.path.exists(marker), "Explicit verification failed to clear the guard"
                assert server.save_requests == 1, "Recovery replayed the save"
                print("Interactive recovery refused then cleared only the local guard")
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
    parser.add_argument("--mode", choices=("slow-save", "lost-response", "server-error", "slow-refresh", "deferred-history"), default="slow-save")
    parser.add_argument("--require-responsive", action="store_true")
    args = parser.parse_args()
    main(os.path.abspath(args.binary), mode=args.mode, require_responsive=args.require_responsive)
