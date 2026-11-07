"""Everything seen so far."""

import namu


@namu.task
def union(seen: list[str], new: list[str]) -> list[str]:
    return sorted(set(seen) | set(new))
