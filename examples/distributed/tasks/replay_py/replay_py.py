"""The documents given, yielded one at a time, as a feed would. The cursor is how many have been
yielded: reopened after a crash, the stream goes on from there instead of from the start."""

import namu


@namu.stream
def replay_py(docs: list[str], *, cursor: str | None = None):
    for i in range(int(cursor or 0), len(docs)):
        yield namu.Item(docs[i], cursor=str(i + 1))
