"""Fetches a page. A server that says to wait (429) or is briefly down (502, 503, 504) is asked
again, a few times, backing off; anything else that is not a page fails the call, and with it the
page's branch, which drops it from the crawl. `NAMU_CRAWL_BASE` is where relative URLs point."""

import os
import time
import urllib.error
import urllib.request
from dataclasses import dataclass

import namu

RETRIED = {429, 502, 503, 504}


@dataclass
class Page:
    url: str
    title: str
    html: str


@namu.task
def fetch(url: str) -> Page:
    full = url if "://" in url else os.environ.get("NAMU_CRAWL_BASE", "http://127.0.0.1:8000") + url
    for attempt in range(4):
        try:
            with urllib.request.urlopen(full, timeout=10) as response:
                html = response.read().decode("utf-8", "replace")
                start, end = html.find("<title>"), html.find("</title>")
                title = html[start + 7 : end] if start >= 0 and end > start else ""
                return Page(url=url, title=title, html=html)
        except urllib.error.HTTPError as err:
            if err.code not in RETRIED or attempt == 3:
                raise RuntimeError(f"{url}: HTTP {err.code}") from None
            wait = float(err.headers.get("Retry-After") or 0) or 0.05 * 2**attempt
            time.sleep(wait)
    raise RuntimeError(f"{url}: gave up")
