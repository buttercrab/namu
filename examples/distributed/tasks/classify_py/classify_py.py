"""`classify`, in Python: labels each document with its most frequent word (ties go to the
alphabetically first). A batch task: each step labels all the documents given to it at once, as a
model would; an empty document fails alone."""

from collections import Counter

import namu


@namu.batch
def classify_py(docs: list[str]) -> list[str]:
    return [top_word(doc) for doc in docs]


def top_word(doc: str) -> str | Exception:
    counts = Counter(doc.split())
    if not counts:
        return ValueError("empty document")
    return min(counts, key=lambda word: (-counts[word], word))
