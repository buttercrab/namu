# namu (Python)

Write NAMU tasks in Python. No dependencies (`pip install namu[fast]` adds the C MessagePack
codec).

```python
import namu

@namu.task
def shout(text: str) -> str:
    return text.upper() + "!"
```

A worker runs the module in a pool of Python processes (`python -m namu.worker task.py`), one
call at a time each; see `docs/guides/tasks.md`. Tests: `python3 -m unittest test_namu`.
