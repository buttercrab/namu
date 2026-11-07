"""How many URLs a crawl found."""

import namu


@namu.task
def size(urls: list[str]) -> int:
    return len(urls)
