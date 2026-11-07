#!/usr/bin/env bash
# A crawl, checked against the site it crawls: scripts/crawl-site.py serves a fixed graph of pages,
# some flaky (503, 429) and some links dead (404), and counts every request. The crawl must find
# exactly the URLs within DEPTH links of the seed, fetch each reachable page exactly once (and
# none beyond), retry the flaky ones, and fail on the dead ones alone.
#
#   ./scripts/crawl.sh                 # 300 pages, depth 4
#   PAGES=2000 DEPTH=6 ./scripts/crawl.sh
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT_DIR"

PAGES=${PAGES:-300}
DEPTH=${DEPTH:-4}
SITE_PORT=${SITE_PORT:-18300}
RUN_DIR="$ROOT_DIR/data/crawl"
rm -rf "$RUN_DIR" && mkdir -p "$RUN_DIR"

python3 -I scripts/crawl-site.py "$SITE_PORT" "$PAGES" &
site=$!
trap 'kill $site >/dev/null 2>&1 || true' EXIT
until curl -sf "http://127.0.0.1:$SITE_PORT/_stats" >/dev/null; do sleep 0.1; done

echo "== crawl $PAGES pages, depth $DEPTH, on 3 workers"
# Workers find the site here; the tasks are Python, so every worker serves Python.
export NAMU_CRAWL_BASE="http://127.0.0.1:$SITE_PORT"
POC_DIR="$RUN_DIR/cluster" TASKS_DIR=examples/crawl/tasks WORKFLOWS=namu-crawl-workflows \
  WORKFLOW=crawl WORKERS="python python python" INPUTS="seed=/page/0 depth=$DEPTH" \
  ./scripts/poc-local.sh | grep -E "^(run|worker)"

curl -sf "http://127.0.0.1:$SITE_PORT/_stats" >"$RUN_DIR/stats.json"
curl -sf "http://127.0.0.1:$SITE_PORT/_expected?seed=/page/0&depth=$DEPTH" >"$RUN_DIR/expected.json"
python3 - "$RUN_DIR/cluster/run-1.json" "$RUN_DIR/stats.json" "$RUN_DIR/expected.json" <<'PY'
import json, sys
text = open(sys.argv[1]).read()
run = json.loads(text[text.index("{"):])
stats, want = json.load(open(sys.argv[2])), json.load(open(sys.argv[3]))
asked, served = stats["asked"], stats["served"]

assert run["results"] == [len(want["seen"])], (run["results"], len(want["seen"]))
# Every page the crawl had to fetch was fetched, once: links seen before were not followed again.
fetched = sorted(served)
assert fetched == want["fetched"], f"fetched {len(fetched)}, expected {len(want['fetched'])}"
twice = {url: n for url, n in served.items() if n != 1}
assert not twice, f"fetched more than once: {twice}"
# Nothing beyond the depth was asked for; flaky pages were asked again until they answered.
beyond = set(asked) - set(want["fetched"]) - set(want["broken"])
assert not beyond, f"asked beyond the depth: {sorted(beyond)[:5]}"
retried = sum(1 for url, n in asked.items() if n > 1 and url in served)
assert retried > 0, "no flaky page was met: the check would prove nothing"
# Dead links failed their own branches, and only those.
assert run["status"] == ("partial_failed" if want["broken"] else "succeeded"), run["status"]
assert len(run["failures"]) == len(want["broken"]), (run["failures"], want["broken"])
assert all("HTTP 404" in failure for failure in run["failures"]), run["failures"]
workers = run["calls_by_worker"]
assert len(workers) > 1, f"work did not spread: {workers}"
print(f"   ok: {len(want['seen'])} URLs found, {len(fetched)} pages fetched once each "
      f"({retried} after a 503 or 429), {len(want['broken'])} dead links dropped; "
      f"calls by worker {workers}")
PY
echo "crawl passed"
