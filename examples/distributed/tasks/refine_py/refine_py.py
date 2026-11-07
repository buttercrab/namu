"""`refine`, in Python: a label, refined one letter per step, as a decoder makes a token per step.
A continuous batch task: words join and leave the session between steps, so a short word never
waits for a long one."""

import namu


@namu.continuous
class refine_py:
    def __init__(self) -> None:
        # Each word in the session, with the letters it has left.
        self.active: dict[int, list] = {}

    def step(self, admit: list[tuple[int, str]]) -> list[tuple[int, str]]:
        for id, word in admit:
            self.active[id] = [word, len(word)]
        done = []
        for id, entry in list(self.active.items()):
            entry[1] -= 1
            if entry[1] <= 0:
                done.append((id, entry[0].upper()))
                del self.active[id]
        return done
