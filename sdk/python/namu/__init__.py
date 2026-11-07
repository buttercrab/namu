"""Write NAMU tasks in Python.

    import namu

    @namu.task
    def summarize(stats: list[Stats]) -> Summary:
        ...

    @namu.stream
    def lines(path: str, *, cursor: str | None = None) -> Iterator[str]:
        ...
        yield namu.Item(line, cursor=str(offset))

    @namu.batch
    def embed(texts: list[str]) -> list[Vector]:
        ...

    @namu.continuous
    class Decoder:
        def step(self, admit: list[tuple[int, Prompt]]) -> list[tuple[int, str]]:
            ...

A worker runs a module's tasks in Python processes: calls of a task in a pool of them, one call
at a time each; an open stream, or a batch session, in a process of its own for as long as it
lasts, so a model loaded once serves every step. Arguments and results cross as MessagePack, as
between any two tasks: a struct is a map of its fields, and a parameter annotated with a
dataclass gets one.

- A task returns its output, or raises: its branch fails with the error.
- A stream is a generator; each value it yields is an item. A stream that declares a keyword
  `cursor` can be reopened where it stopped: it yields `Item(value, cursor=...)`, and is given the
  last cursor when the engine opens it again after a crash.
- A batch task is given a list of inputs and returns a list of outputs, one for each, in order.
  An exception in place of an output fails that item alone; one raised fails all of them.
- A continuous batch task is a class whose `step` takes in `(id, input)` pairs and returns the
  `(id, output)` pairs that are done, possibly fewer, possibly from earlier steps: a session is
  one instance, kept across steps.

`namu.call_key()` names the call being run (or the stream's call), and `namu.call_keys()` the
calls a batch step serves, for side effects to be made once (see the NAMU docs on call keys).
"""

from __future__ import annotations

import contextvars
from dataclasses import dataclass
from typing import Any, Callable, Generic, TypeVar

__all__ = ["task", "stream", "batch", "continuous", "Item", "call_key", "call_keys"]

T = TypeVar("T")


@dataclass(frozen=True)
class Task:
    """A registered task: what it is (`single`, `stream`, `batch` or `continuous`) and its code."""

    kind: str
    code: Callable


#: Tasks by name, as the decorators register them.
TASKS: dict[str, Task] = {}

_CALL_KEY: contextvars.ContextVar[str | None] = contextvars.ContextVar("namu_call_key", default=None)
_CALL_KEYS: contextvars.ContextVar[list[str]] = contextvars.ContextVar("namu_call_keys", default=[])


def _register(kind: str, code: T) -> T:
    TASKS[code.__name__] = Task(kind, code)
    return code


def task(function: T) -> T:
    """Registers `function` as the task of its name."""
    return _register("single", function)


def stream(generator: T) -> T:
    """Registers a generator function as the stream task of its name."""
    return _register("stream", generator)


def batch(function: T) -> T:
    """Registers a function from a list of inputs to a list of outputs as a batch task."""
    return _register("batch", function)


def continuous(cls: T) -> T:
    """Registers a class with a `step` method as a continuous batch task."""
    return _register("continuous", cls)


@dataclass(frozen=True)
class Item(Generic[T]):
    """An item of a stream, with where the stream can be reopened to continue after it. A stream
    of them is annotated `Iterator[Item[T]]`."""

    value: T
    cursor: str | None = None


def call_key() -> str | None:
    """The key of the call being run: the same each time the engine runs it again after a crash,
    and different for every other call. `None` outside a call."""
    return _CALL_KEY.get()


def call_keys() -> list[str]:
    """The keys of the calls a batch step serves, in the order of its items. Empty outside a
    batch step."""
    return list(_CALL_KEYS.get())
