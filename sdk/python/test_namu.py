"""Tests of the SDK: the codec against the format, and a worker process end to end."""

import struct
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

from namu import _msgpack

HERE = Path(__file__).parent


class Codec(unittest.TestCase):
    def test_round_trip(self):
        for value in [None, True, False, 0, 127, 128, -1, -33, 2**40, -(2**40), 1.5, "", "naïve",
                      b"\x00\xff", [1, [2, 3]], {"a": {"b": [1.0]}}, "x" * 300, list(range(70))]:
            self.assertEqual(_msgpack.loads(_msgpack.dumps(value)), value)

    def test_known_encodings(self):
        # From the MessagePack spec.
        self.assertEqual(_msgpack.dumps({"compact": True, "schema": 0}).hex(),
                         "82a7636f6d70616374c3a6736368656d6100")
        self.assertEqual(_msgpack.dumps(-129).hex(), "d1ff7f")


class Worker(unittest.TestCase):
    def test_a_worker_process_runs_calls(self):
        module = textwrap.dedent('''
            from dataclasses import dataclass
            import namu

            @dataclass
            class Point:
                x: int
                y: int

            @namu.task
            def shift(p: Point, by: int) -> Point:
                print("shifting")  # to stderr, not the protocol
                return Point(p.x + by, p.y + by)

            @namu.task
            def whoami() -> str:
                return namu.call_key()

            @namu.task
            def fails(x: int) -> int:
                raise ValueError(f"bad {x}")

            @namu.batch
            def lengths(words: list[str]) -> list[int]:
                return [len(w) for w in words]
        ''')
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "task.py"
            path.write_text(module)
            process = subprocess.Popen([sys.executable, "-m", "namu.worker", str(path)],
                                       stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                       stderr=subprocess.DEVNULL, cwd=HERE)

            def ask(*request):
                frame = _msgpack.dumps(list(request))
                process.stdin.write(struct.pack(">I", len(frame)) + frame)
                process.stdin.flush()
                (length,) = struct.unpack(">I", process.stdout.read(4))
                return _msgpack.loads(process.stdout.read(length))

            def call(task, key, value):
                ok, payload = ask("call", task, key, _msgpack.dumps(value))
                return ok, _msgpack.loads(payload) if ok else payload

            self.assertEqual(call("shift", "k1", [{"x": 1, "y": 2}, 10]), (True, {"x": 11, "y": 12}))
            self.assertEqual(call("whoami", "run/7/0/3", None), (True, "run/7/0/3"))
            self.assertEqual(call("fails", "k2", 4), (False, "ValueError: bad 4"))
            self.assertEqual(call("missing", "k3", None)[0], False)
            self.assertEqual(ask("step", []), [False, "RuntimeError: no batch session is open"])
            # From here on, the process serves a batch session.
            self.assertEqual(ask("session", "lengths"), [True, None])
            ok, finished = ask("step", [[1, "k4", _msgpack.dumps("abc")], [2, "k5", _msgpack.dumps("")]])
            self.assertTrue(ok)
            self.assertEqual([(id, _msgpack.loads(out)) for id, (_, out) in finished], [(1, 3), (2, 0)])
            process.stdin.close()
            self.assertEqual(process.wait(timeout=5), 0)


if __name__ == "__main__":
    unittest.main()
