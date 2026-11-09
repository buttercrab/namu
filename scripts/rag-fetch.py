#!/usr/bin/env python3
"""Fetches Wikipedia articles into a JSONL corpus for the RAG example (examples/rag).

    scripts/rag-fetch.py [out.jsonl] [--limit N] [--category NAME]

Takes the articles of a category (default "Machine learning"), in the order Wikipedia lists them,
and writes one line per article: {"id", "title", "revision", "url", "text"}. The text is the plain
text of the current revision (CC BY-SA 4.0; the URL and revision id credit it). Articles are
fetched one by one, politely; a corpus already fetched with the same arguments is kept (its
`.done` file records them).
"""

import argparse
import http.client
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

API = "https://en.wikipedia.org/w/api.php"
AGENT = "namu-rag-example/0.1 (https://github.com/buttercrab/namu)"


def get(params, tries=8):
    """One API call. Rate limits, server errors and broken connections are waited out, as long as
    Wikipedia asks (`Retry-After`) or, if it does not say, twice as long each time, up to a
    minute; other client errors are not retried."""
    url = API + "?" + urllib.parse.urlencode({**params, "format": "json", "formatversion": 2})
    for attempt in range(tries):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": AGENT})
            with urllib.request.urlopen(request, timeout=30) as response:
                return json.load(response)
        except urllib.error.HTTPError as err:
            if err.code != 429 and err.code < 500:
                raise
            error, wait = err, retry_after(err)
        # Refused or dropped connections, timeouts, and bodies cut short (or not JSON for it).
        except (OSError, http.client.HTTPException, ValueError) as err:
            error, wait = err, None
        if attempt == tries - 1:
            raise error
        wait = min(2**attempt, 60) if wait is None else wait
        print(f"  retrying in {wait}s after {error}", file=sys.stderr)
        time.sleep(wait)


def retry_after(err):
    """The seconds a 429 or 503 asks to wait, if it says, capped at five minutes."""
    try:
        return min(int(err.headers.get("Retry-After", "")), 300)
    except ValueError:
        return None


def titles(category, limit):
    params = {
        "action": "query",
        "list": "categorymembers",
        "cmtitle": f"Category:{category}",
        "cmnamespace": 0,
        "cmlimit": "max",
    }
    found = []
    while len(found) < limit:
        page = get(params)
        found += [member["title"] for member in page["query"]["categorymembers"]]
        if "continue" not in page:
            break
        params.update(page["continue"])
    return found[:limit]


def article(title):
    page = get(
        {
            "action": "query",
            "prop": "extracts|info|revisions",
            "explaintext": 1,
            "inprop": "url",
            "rvprop": "ids",
            "titles": title,
        }
    )["query"]["pages"][0]
    return {
        "id": page["pageid"],
        "title": page["title"],
        "revision": page["revisions"][0]["revid"],
        "url": page["fullurl"],
        "text": page.get("extract", ""),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("out", nargs="?", default="data/rag/corpus.jsonl")
    parser.add_argument("--limit", type=int, default=300)
    parser.add_argument("--category", default="Machine learning")
    args = parser.parse_args()

    done = f"{args.category}\n{args.limit}\n"
    try:
        if open(args.out + ".done").read() == done and os.path.exists(args.out):
            print(f"{args.out}: already fetched")
            return
    except FileNotFoundError:
        pass

    os.makedirs(os.path.dirname(args.out) or ".", exist_ok=True)
    wanted = titles(args.category, args.limit)
    written = 0
    with open(args.out + ".part", "w") as out:
        for title in wanted:
            doc = article(title)
            if len(doc["text"].split()) < 50:  # Stubs and redirects carry nothing to search.
                continue
            out.write(json.dumps(doc, ensure_ascii=False) + "\n")
            written += 1
            time.sleep(0.05)
    os.replace(args.out + ".part", args.out)
    with open(args.out + ".done", "w") as marker:
        marker.write(done)
    print(f"{args.out}: {written} articles from Category:{args.category}")


if __name__ == "__main__":
    main()
