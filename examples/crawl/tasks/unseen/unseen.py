"""The next frontier: links found on this level's pages that no earlier level has seen."""

import namu


@namu.task
def unseen(found: list[list[str]], seen: list[str]) -> list[str]:
    known = set(seen)
    return sorted({url for links in found for url in links} - known)
