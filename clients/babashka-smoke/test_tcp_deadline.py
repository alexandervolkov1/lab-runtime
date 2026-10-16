"""Optional loopback regressions for the sole client; requires bb, not Runtime."""

from contextlib import contextmanager
import json
from pathlib import Path
import socket
import subprocess
import time
import unittest


CLIENT = Path(__file__).with_name("manual-output.clj")


@contextmanager
def connected_client():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen(2)
        listener.settimeout(5)
        port = listener.getsockname()[1]
        process = subprocess.Popen(
            ["bb", str(CLIENT), "--url", f"tcp://127.0.0.1:{port}",
             "--timeout-ms", "1000"],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        try:
            connection, _ = listener.accept()
            with connection:
                connection.settimeout(3)
                yield connection, process, listener
        finally:
            if process.poll() is None:
                process.kill()
            process.communicate(timeout=5)


def request(connection):
    data = bytearray()
    while not data.endswith(b"\n"):
        chunk = connection.recv(1)
        if not chunk:
            raise AssertionError("client disconnected before request")
        data.extend(chunk)
    return json.loads(data)


def frame(value):
    return (json.dumps(value, ensure_ascii=False) + "\n").encode("utf-8")


class TcpDeadlineTests(unittest.TestCase):
    def test_trickled_unterminated_line_expires_and_closes_without_retry(self):
        with connected_client() as (connection, process, listener):
            self.assertEqual(request(connection)["op"], "hello")
            started = time.monotonic()
            connection.settimeout(0.25)
            received = bytearray()
            closed = False
            while time.monotonic() - started < 3:
                try:
                    connection.sendall(b" ")
                    data = connection.recv(4096)
                    if not data:
                        closed = True
                        break
                    received.extend(data)
                except socket.timeout:
                    continue
                except (ConnectionResetError, BrokenPipeError):
                    closed = True
                    break
            elapsed = time.monotonic() - started
            self.assertTrue(closed, "client must close the socket at its deadline")
            self.assertGreaterEqual(elapsed, 0.7)
            self.assertLess(elapsed, 2.5, f"absolute 1 s deadline took {elapsed:.3f} s")
            stdout, stderr = process.communicate(timeout=2)
            self.assertEqual(process.returncode, 1, stderr.decode())
            self.assertIn(b"FAIL:", stderr)
            self.assertEqual(received, b"", "no request may be retried")
            sent = [json.loads(line) for line in stdout.splitlines()
                    if json.loads(line)["kind"] == "request"]
            self.assertEqual(len(sent), 1)
            listener.settimeout(0.05)
            with self.assertRaises(socket.timeout):
                listener.accept()
            print(f"TCP deadline: {elapsed:.3f}s, exit=1, closed, one request")

    def test_fragmented_utf8_and_coalesced_events_preserve_response_correlation(self):
        with connected_client() as (connection, process, _):
            hello = request(connection)
            label = "Печь °C 🔥"
            event = frame({"type": "event", "kind": "host", "data": label})
            # Break inside both two-byte and four-byte UTF-8 code points.
            cuts = [0, event.index("П".encode()) + 1, event.index("🔥".encode()) + 2]
            for start, end in zip(cuts, cuts[1:]):
                connection.sendall(event[start:end])
                time.sleep(0.02)
            connection.sendall(
                event[cuts[-1]:]
                + frame({"type": "subscription_progress"})
                + frame({"type": "result", "msg_id": hello["msg_id"],
                         "result": {"scope": "fixture", "next_seq": "1"}})
            )
            discovery = request(connection)
            self.assertEqual(discovery["op"], "discover")
            # End this transport fixture deliberately; it is not a second smoke client.
            connection.sendall(frame({"type": "error", "msg_id": discovery["msg_id"]}))
            stdout, stderr = process.communicate(timeout=2)
            self.assertEqual(process.returncode, 1)
            self.assertIn(b"discover failed", stderr)
            events = [json.loads(line)["data"] for line in stdout.splitlines()
                      if json.loads(line)["kind"] == "event"]
            self.assertEqual(events[0]["data"], label)
            self.assertEqual(events[1]["type"], "subscription_progress")


if __name__ == "__main__":
    unittest.main()
