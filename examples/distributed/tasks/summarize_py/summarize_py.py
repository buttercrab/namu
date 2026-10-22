"""`summarize`, in Python: the per-document stats that `analyze` (WASM) made, into one summary.
The same summary as the native task's, so a workflow may use either."""

from dataclasses import dataclass

import namu


@dataclass
class Stats:
    words: int
    long_words: int
    top_word: str
    top_count: int
    checksum: int


@dataclass
class Summary:
    docs: int
    words: int
    long_words: int
    top_words: dict[str, int]
    checksum: int


@namu.task
def summarize_py(stats: list[Stats]) -> Summary:
    top_words: dict[str, int] = {}
    checksum = 0
    for s in stats:
        top_words[s.top_word] = top_words.get(s.top_word, 0) + s.top_count
        # XOR does not depend on the order the documents came in.
        checksum ^= s.checksum
    return Summary(
        docs=len(stats),
        words=sum(s.words for s in stats),
        long_words=sum(s.long_words for s in stats),
        top_words=dict(sorted(top_words.items())),
        checksum=checksum,
    )
