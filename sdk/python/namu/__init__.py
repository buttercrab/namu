"""Write NAMU tasks in Python.

    import namu

    @namu.task
    def summarize(stats: list[Stats]) -> Summary:
        ...

A worker runs the module in a pool of Python processes, one call at a time each. Arguments and
results cross as MessagePack, as between any two tasks: a struct is a map of its fields, and a
parameter annotated with a dataclass gets one. `namu.call_key()` names the call being run, for
side effects to be made once (see the NAMU docs on call keys).
"""

from __future__ import annotations

import contextvars
from typing import Callable, TypeVar

__all__ = ["task", "call_key"]

F = TypeVar("F", bound=Callable)

#: Tasks by name, as `@task` registers them.
TASKS: dict[str, Callable] = {}

_CALL_KEY: contextvars.ContextVar[str | None] = contextvars.ContextVar("namu_call_key", default=None)


def task(function: F) -> F:
    """Registers `function` as the task of its name."""
    TASKS[function.__name__] = function
    return function


def call_key() -> str | None:
    """The key of the call being run: the same each time the engine runs it again after a crash,
    and different for every other call. `None` outside a call."""
    return _CALL_KEY.get()
