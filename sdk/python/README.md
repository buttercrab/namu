# namu (Python)

Write NAMU tasks in Python. No dependencies (`pip install namu[fast]` adds the C MessagePack
codec).

```python
import namu

@namu.task
def shout(text: str) -> str:
    return text.upper() + "!"
```

Streams (`@namu.stream`, a generator), dynamic batches (`@namu.batch`, a list in and a list
out) and continuous batches (`@namu.continuous`, a class with `step`) too.

Annotations are the task's signature: `namu build` reads them (by parsing the module, without
running it) and records what the task takes and gives, so a workflow that calls it with the wrong
types is refused when it is published. `python -m namu.signature task.py shout` prints it.

A task that fails with a type of its own declares it and raises it:

```python
@namu.task(error=FetchError)
def fetch(url: str) -> str:
    raise namu.Failed(FetchError.not_found)
```

A workflow that attempts the call gets the error value, and can match on it; anything else a task
raises is a message.

A worker runs a task's calls in a pool of Python processes (`python -m namu.worker task.py`), one
call at a time each, and an open stream or batch session in a process of its own; see
`docs/guides/tasks.md`. Tests: `python3 -m unittest test_namu`.
