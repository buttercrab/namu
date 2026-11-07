"""A website to crawl, the same every time, that counts what it is asked.

    python3 -I scripts/crawl-site.py PORT [PAGES]

Page i (`/page/i`) links to three others, by arithmetic, so the graph is fixed. Some pages are
flaky: they answer 503 (or 429, with Retry-After) once or twice before answering. Some link to
pages that do not exist (404). `/_stats` says how often each path was asked for and how often it
was served; `/_expected?seed=/page/0&depth=D` says what a crawl of depth D from the seed finds
and fetches.
"""

import json
import sys
import threading
from collections import Counter
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

PAGES = int(sys.argv[2]) if len(sys.argv) > 2 else 300
lock = threading.Lock()
asked: Counter = Counter()
served: Counter = Counter()


def links(i: int) -> list[str]:
    out = [f"/page/{(i * 7 + 1) % PAGES}", f"/page/{(i * 13 + 5) % PAGES}", f"/page/{(i * 29 + 11) % PAGES}"]
    if i % 23 == 7:
        out.append(f"/page/missing-{i}")
    return out


def failures_before_success(i: int) -> int:
    """How many times page i fails before it answers: flaky pages, deterministically."""
    if i % 17 == 5:
        return 2
    if i % 11 == 3:
        return 1
    return 0


def expected(seed: str, depth: int) -> dict:
    """What a crawl finds: every URL within `depth` links of the seed; what it fetches: those
    within `depth - 1`, which are the pages whose links it follows."""
    level = [seed]
    seen = {seed}
    fetched = []
    for _ in range(depth):
        fetched += level
        nxt = []
        for url in level:
            if url.startswith("/page/missing"):
                continue
            for link in links(int(url.rsplit("/", 1)[1])):
                if link not in seen:
                    seen.add(link)
                    nxt.append(link)
        level = nxt
    return {
        "seen": sorted(seen),
        "fetched": sorted(u for u in fetched if not u.startswith("/page/missing")),
        "broken": sorted(u for u in fetched if u.startswith("/page/missing")),
    }


class Site(BaseHTTPRequestHandler):
    def log_message(self, *args) -> None:
        pass

    def send(self, status: int, body: str, kind: str = "text/html", headers: dict | None = None) -> None:
        data = body.encode()
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(data)))
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self) -> None:
        url = urlparse(self.path)
        if url.path == "/_stats":
            with lock:
                return self.send(200, json.dumps({"asked": asked, "served": served}), "application/json")
        if url.path == "/_expected":
            query = parse_qs(url.query)
            want = expected(query["seed"][0], int(query["depth"][0]))
            return self.send(200, json.dumps(want), "application/json")
        with lock:
            asked[url.path] += 1
            times = asked[url.path]
        name = url.path.rsplit("/", 1)[-1]
        if not url.path.startswith("/page/") or not name.isdigit() or int(name) >= PAGES:
            return self.send(404, "no such page")
        i = int(name)
        if times <= failures_before_success(i):
            if i % 2:
                return self.send(503, "try again")
            return self.send(429, "slow down", headers={"Retry-After": "0"})
        with lock:
            served[url.path] += 1
        anchors = "".join(f'<li><a href="{link}">{link}</a></li>' for link in links(i))
        self.send(200, f"<html><head><title>Page {i}</title></head><body><ul>{anchors}</ul></body></html>")


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Site).serve_forever()
