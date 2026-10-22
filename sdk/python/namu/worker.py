"""A worker's Python process: runs the tasks of one module, one call at a time.

    python -m namu.worker path/to/task.py

It reads calls from stdin and writes results to stdout, each a MessagePack document after its
length (4 bytes, big-endian): a call is `[task, key, input]`, with the input as the task ABI packs
it (nil for no arguments, the argument for one, an array for several); a result is `[true,
output]` or `[false, error]`. What the task prints goes to stderr, out of the protocol's way.
"""

from __future__ import annotations

import dataclasses
import importlib.util
import inspect
import struct
import sys
import typing

from . import TASKS, _CALL_KEY
from ._msgpack import dumps, loads


def main() -> None:
    path = sys.argv[1]
    protocol_in, protocol_out = sys.stdin.buffer, sys.stdout.buffer
    sys.stdout = sys.stderr
    spec = importlib.util.spec_from_file_location("namu_task", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    while True:
        header = protocol_in.read(4)
        if len(header) < 4:
            return
        (length,) = struct.unpack(">I", header)
        task, key, data = loads(protocol_in.read(length))
        try:
            result = [True, call(task, key, data)]
        except Exception as error:  # The task's own failure: its message goes back.
            result = [False, f"{type(error).__name__}: {error}"]
        payload = dumps(result)
        protocol_out.write(struct.pack(">I", len(payload)) + payload)
        protocol_out.flush()


def call(task: str, key: str, data: bytes) -> bytes:
    function = TASKS.get(task)
    if function is None:
        raise LookupError(f"no task {task!r} in this module (is it decorated with @namu.task?)")
    parameters = list(inspect.signature(function).parameters.values())
    hints = typing.get_type_hints(function)
    value = loads(data)
    arguments = {0: [], 1: [value]}.get(len(parameters), value)
    if len(arguments) != len(parameters):
        raise TypeError(f"{task} takes {len(parameters)} arguments, not {len(arguments)}")
    arguments = [from_value(hints.get(p.name), a) for p, a in zip(parameters, arguments)]
    token = _CALL_KEY.set(key)
    try:
        output = function(*arguments)
    finally:
        _CALL_KEY.reset(token)
    return dumps(to_value(output))


def from_value(hint, value):
    """`value` as `hint` says: dataclasses from maps, element by element through lists."""
    if hint is None or value is None:
        return value
    if dataclasses.is_dataclass(hint) and isinstance(value, dict):
        hints = typing.get_type_hints(hint)
        fields = {f.name: from_value(hints.get(f.name), value.get(f.name)) for f in dataclasses.fields(hint)}
        return hint(**fields)
    origin, args = typing.get_origin(hint), typing.get_args(hint)
    if origin in (list, typing.List) and args and isinstance(value, list):
        return [from_value(args[0], item) for item in value]
    if origin in (dict, typing.Dict) and len(args) == 2 and isinstance(value, dict):
        return {key: from_value(args[1], item) for key, item in value.items()}
    return value


def to_value(output):
    """`output` as MessagePack takes it: dataclasses as maps of their fields."""
    if dataclasses.is_dataclass(output) and not isinstance(output, type):
        return {f.name: to_value(getattr(output, f.name)) for f in dataclasses.fields(output)}
    if isinstance(output, (list, tuple)):
        return [to_value(item) for item in output]
    if isinstance(output, dict):
        return {key: to_value(item) for key, item in output.items()}
    return output


if __name__ == "__main__":
    main()
