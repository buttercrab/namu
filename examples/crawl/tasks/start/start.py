"""The first frontier: the seed alone."""

import namu


@namu.task
def start(seed: str) -> list[str]:
    return [seed]
