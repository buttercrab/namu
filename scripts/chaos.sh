#!/usr/bin/env bash
# Chaos: one run, and everything that can happen to a cluster while it goes. A stream of documents
# is processed on four workers while, one after another: a worker is killed (kill -9), another is
# drained (SIGTERM), the orchestrator is killed and started again, Redis is shut down and started
# again from its append-only file, and a new worker joins. The run must give exactly what an
# undisturbed run gives, every result once.
#
#   ./scripts/chaos.sh
#   DOCS=1000 DELAY_MS=20 ./scripts/chaos.sh
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT_DIR"

DOCS=${DOCS:-400}
DELAY_MS=${DELAY_MS:-40}
DIR="$ROOT_DIR/data/chaos"
rm -rf "$DIR"
mkdir -p "$(dirname "$DIR")"

echo "== an undisturbed run, then chaos, on 4 workers"
KEEP=1 POC_DIR="$DIR" REDIS_PERSIST=1 WORKFLOW=doc_ids_streamed \
  WORKERS="wasm,python wasm,python wasm,python wasm,python" \
  INPUTS="n=$DOCS delay_ms=$DELAY_MS" ./scripts/poc-local.sh >"$DIR.log" 2>&1
read -r -a pids <<<"$(sed -n 's/^left running (pids: \(.*\))$/\1/p' "$DIR.log")"
# pids: redis, orchestrator, worker-1 .. worker-4
teardown() {
  kill "${pids[@]:2}" >/dev/null 2>&1 || true
  sleep 2
  kill -9 "${pids[@]:2}" >/dev/null 2>&1 || true
  kill "${pids[1]}" >/dev/null 2>&1 || true
  redis-cli -p "${REDIS_PORT:-6390}" shutdown nosave >/dev/null 2>&1 || true
}
trap teardown EXIT
# shellcheck source=/dev/null
source "$DIR/env"
namu() { "$ROOT_DIR/target/release/namu" "$@"; }

run=$(namu run doc_ids_streamed 0.1.0 --input "n=$DOCS" --input "delay_ms=$DELAY_MS" |
  sed -n 's/^Run created: //p')
namu results "$run" --follow >"$DIR/results.jsonl" &
follow=$!
step() { sleep "$1"; echo "   $2"; }

step 2 "kill -9 worker-2"
kill -9 "${pids[3]}"
step 2 "drain worker-3"
kill -TERM "${pids[4]}"
step 2 "kill -9 the orchestrator"
kill -9 "${pids[1]}"
step 3 "start the orchestrator again"
pids[1]=$("$DIR/start-master")
step 2 "shut Redis down"
redis-cli -p "${REDIS_PORT:-6390}" shutdown >/dev/null 2>&1 || true
step 3 "start Redis again, from its append-only file"
pids[0]=$("$DIR/start-redis")
step 1 "a new worker joins"
pids+=("$("$DIR/start-worker" 5 wasm,python)")

wait "$follow"
python3 - "$DIR/run-1.json" "$DIR/results.jsonl" "$DOCS" <<'PY'
import json, sys
text = open(sys.argv[1]).read()
baseline = json.loads(text[text.index("{"):])
docs = int(sys.argv[3])
ids = [int(line) for line in open(sys.argv[2])]
assert baseline["status"] == "succeeded" and len(baseline["results"]) == docs, baseline["status"]
assert len(ids) == len(set(ids)), f"{len(ids) - len(set(ids))} results more than once"
assert sorted(ids) == sorted(baseline["results"]), (
    f"{len(set(baseline['results']) - set(ids))} results missing, "
    f"{len(set(ids) - set(baseline['results']))} not in the undisturbed run")
print(f"   ok: all {docs} results, each once, the same as the undisturbed run's")
PY
echo "chaos passed"
