"""A worker's Python process: runs the tasks of one module.

    python -m namu.worker path/to/task.py

It reads requests from stdin and writes answers to stdout, each a MessagePack document after its
length (4 bytes, big-endian). A request is an array, its verb first; an answer is `[true, value]`,
or `[false, error]` for the task's own failure. Inputs and outputs are MessagePack themselves, as
the task ABI packs them (an input is nil for no arguments, the argument for one, an array for
several), carried as binary.

    ["call", task, key, input]       -> output
    ["open", task, key, input, cursor]  -> nil        a process serves one stream from then on
    ["next"]                         -> nil at the end, or [output, cursor]
    ["session", task]                -> nil        a process serves one batch session from then on
    ["step", [[id, key, input], ...]]   -> [[id, [true, output] or [false, error]], ...]

What a task prints goes to stderr, out of the protocol's way.
"""

from __future__ import annotations

import contextlib
import contextvars
import dataclasses
import importlib.util
import inspect
import struct
import sys
import typing
from typing import Any, Callable, Iterator

from . import _CALL_KEY, _CALL_KEYS, TASKS, Item, Task
from ._msgpack import dumps, loads


def main() -> None:
    path = sys.argv[1]
    protocol_in, protocol_out = sys.stdin.buffer, sys.stdout.buffer
    sys.stdout = sys.stderr
    spec = importlib.util.spec_from_file_location("namu_task", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    process = Process()
    while True:
        header = protocol_in.read(4)
        if len(header) < 4:
            return
        (length,) = struct.unpack(">I", header)
        verb, *arguments = loads(protocol_in.read(length))
        try:
            if verb not in Process.VERBS:
                raise LookupError(f"no request {verb!r}")
            answer = [True, getattr(process, verb)(*arguments)]
        except Exception as error:  # The task's own failure: its message goes back.
            answer = [False, failure(error)]
        payload = dumps(answer)
        protocol_out.write(struct.pack(">I", len(payload)) + payload)
        protocol_out.flush()


class Process:
    """What a process serves: calls, or the one stream or batch session it was opened for."""

    VERBS = frozenset({"call", "open", "next", "session", "step"})

    def __init__(self) -> None:
        self.items: Iterator | None = None
        self.key: str | None = None
        self.stepper: Callable[[list], list] | None = None

    def call(self, task: str, key: str, data: bytes) -> bytes:
        function = lookup(task, "single").code
        arguments = bind(function, loads(data))
        with keyed(_CALL_KEY, key):
            return dumps(to_value(function(*arguments)))

    def open(self, task: str, key: str, data: bytes, cursor: str | None) -> None:
        function = lookup(task, "stream").code
        resumes = "cursor" in inspect.signature(function).parameters
        arguments = bind(function, loads(data), skip={"cursor"})
        with keyed(_CALL_KEY, key):
            items = function(*arguments, cursor=cursor) if resumes else function(*arguments)
        self.items, self.key = iter(items), key

    def next(self) -> list | None:
        if self.items is None:
            raise RuntimeError("no stream is open")
        with keyed(_CALL_KEY, self.key):
            item = next(self.items, _END)
        if item is _END:
            return None
        if not isinstance(item, Item):
            item = Item(item)
        return [dumps(to_value(item.value)), item.cursor]

    def session(self, task: str) -> None:
        found = lookup(task, "batch", "continuous")
        if found.kind == "batch":
            self.stepper = dynamic(found.code)
        else:
            self.stepper = continuing(found.code())

    def step(self, admitted: list) -> list:
        if self.stepper is None:
            raise RuntimeError("no batch session is open")
        return self.stepper(admitted)


_END = object()


def dynamic(function: Callable[[list], list]) -> Callable[[list], list]:
    """A dynamic batch session: each step runs `function` over the items given, all of which it
    finishes."""
    (parameter,) = inspect.signature(function).parameters.values()
    hint = element(typing.get_type_hints(function).get(parameter.name))

    def step(admitted: list) -> list:
        finished, ids, keys, inputs = [], [], [], []
        for id, key, data in admitted:
            try:
                inputs.append(from_value(hint, loads(data)))
            except Exception as error:  # An input that does not fit fails alone.
                finished.append([id, [False, f"bad input: {failure(error)}"]])
                continue
            ids.append(id)
            keys.append(key)
        if not inputs:
            return finished
        try:
            with keyed(_CALL_KEYS, keys):
                outputs = list(function(inputs))
        except Exception as error:  # Raised, it fails every item; the session stays.
            return finished + [[id, [False, failure(error)]] for id in ids]
        if len(outputs) != len(ids):
            raise ValueError(f"a batch of {len(ids)} gave {len(outputs)} results")
        return finished + [[id, outcome(output)] for id, output in zip(ids, outputs)]

    return step


def continuing(session: Any) -> Callable[[list], list]:
    """A continuous batch session: `session.step` takes in `(id, input)` pairs and gives back the
    `(id, output)` pairs that are done. Raised, an error fails the session and every item in it."""
    hints = typing.get_type_hints(session.step)
    (parameter,) = inspect.signature(session.step).parameters.values()
    pair = element(hints.get(parameter.name))
    hint = typing.get_args(pair)[1] if len(typing.get_args(pair)) == 2 else None

    def step(admitted: list) -> list:
        finished, admit, keys = [], [], []
        for id, key, data in admitted:
            try:
                admit.append((id, from_value(hint, loads(data))))
            except Exception as error:
                finished.append([id, [False, f"bad input: {failure(error)}"]])
                continue
            keys.append(key)
        with keyed(_CALL_KEYS, keys):
            done = session.step(admit)
        return finished + [[id, outcome(output)] for id, output in done]

    return step


def lookup(task: str, *kinds: str) -> Task:
    found = TASKS.get(task)
    if found is None:
        raise LookupError(f"no task {task!r} in this module (is it decorated, e.g. @namu.task?)")
    if found.kind not in kinds:
        raise TypeError(f"{task} is a {found.kind} task, not a {' or '.join(kinds)} one")
    return found


def bind(function: Callable, value: Any, skip: set[str] = frozenset()) -> list:
    """A call's arguments, as `function`'s parameters (but `skip`) take them, from its input."""
    parameters = [p for p in inspect.signature(function).parameters.values() if p.name not in skip]
    hints = typing.get_type_hints(function)
    arguments = {0: [], 1: [value]}.get(len(parameters), value)
    if len(arguments) != len(parameters):
        raise TypeError(f"{function.__name__} takes {len(parameters)} arguments, not {len(arguments)}")
    return [from_value(hints.get(p.name), a) for p, a in zip(parameters, arguments)]


@contextlib.contextmanager
def keyed(variable: contextvars.ContextVar, value: Any) -> Iterator[None]:
    """`variable` is `value` for the duration of a `with` block."""
    token = variable.set(value)
    try:
        yield
    finally:
        variable.reset(token)


def outcome(output: Any) -> list:
    """An item's output as a step answers it: an exception in its place fails the item alone."""
    if isinstance(output, BaseException):
        return [False, failure(output)]
    return [True, dumps(to_value(output))]


def failure(error: BaseException) -> str:
    return f"{type(error).__name__}: {error}"


def element(hint: Any) -> Any:
    """`X` of `list[X]`, or nothing to go by."""
    args = typing.get_args(hint)
    return args[0] if typing.get_origin(hint) in (list, typing.List) and args else None


def from_value(hint: Any, value: Any) -> Any:
    """`value` as `hint` says: dataclasses from maps, element by element through lists, tuples
    and dicts."""
    if hint is None or value is None:
        return value
    if dataclasses.is_dataclass(hint) and isinstance(value, dict):
        hints = typing.get_type_hints(hint)
        fields = {f.name: from_value(hints.get(f.name), value.get(f.name)) for f in dataclasses.fields(hint)}
        return hint(**fields)
    origin, args = typing.get_origin(hint), typing.get_args(hint)
    if origin in (list, typing.List) and args and isinstance(value, list):
        return [from_value(args[0], item) for item in value]
    if origin in (tuple, typing.Tuple) and args and isinstance(value, list):
        if len(args) == 2 and args[1] is Ellipsis:
            return tuple(from_value(args[0], item) for item in value)
        return tuple(from_value(arg, item) for arg, item in zip(args, value))
    if origin in (dict, typing.Dict) and len(args) == 2 and isinstance(value, dict):
        return {key: from_value(args[1], item) for key, item in value.items()}
    return value


def to_value(output: Any) -> Any:
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
