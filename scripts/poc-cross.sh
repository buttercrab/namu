#!/usr/bin/env bash
# Cross-platform distribution demo: Redis, the orchestrator and 2 Linux workers run in Docker;
# workers on this machine (macOS) join them. The same .wasm artifacts run on both platforms;
# native tasks only run on the platform they were built for.
#
#   ./scripts/poc-cross.sh
#   HOST_WORKERS="native" WORKFLOW=bench_handoff RUNS=5 ./scripts/poc-cross.sh
#
# HOST_WORKERS: space-separated list of host workers, each a comma-separated runtime list.
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT_DIR"

export REDIS_PORT=${REDIS_PORT:-6390}
export ORCH_PORT=${ORCH_PORT:-8090}
HOST_WORKERS=${HOST_WORKERS:-"wasm,native"}
WORKFLOW=${WORKFLOW:-doc_stats}
RUNS=${RUNS:-3}
POC_DIR=${POC_DIR:-$ROOT_DIR/data/poc-cross}
BIN="$ROOT_DIR/target/release"
COMPOSE=(docker compose -f dev/docker-compose.poc.yml)

PIDS=()
cleanup() {
  if [[ "${KEEP:-0}" == "1" ]]; then
    echo "left running (host pids: ${PIDS[*]})"
    return
  fi
  for pid in "${PIDS[@]}"; do kill "$pid" >/dev/null 2>&1 || true; done
  "${COMPOSE[@]}" down >/dev/null 2>&1 || true
}
trap cleanup EXIT

rm -rf "$POC_DIR" && mkdir -p "$POC_DIR/logs"

cargo build --release -q -p namu-worker -p namu-cli
cargo run -q -p namu-distributed-workflows --bin export -- "$POC_DIR/workflows" >/dev/null

"${COMPOSE[@]}" up --build -d >"$POC_DIR/logs/compose.log" 2>&1
export REDIS_URL="redis://127.0.0.1:${REDIS_PORT}/"
export NAMU_ORCH_URL="http://127.0.0.1:${ORCH_PORT}"
until curl -sf "$NAMU_ORCH_URL/healthz" >/dev/null; do sleep 0.2; done

i=0
for runtimes in $HOST_WORKERS; do
  i=$((i + 1))
  WORKER_ID="host-$i" NAMU_RUNTIMES="$runtimes" ARTIFACT_CACHE="$POC_DIR/cache" \
    "$BIN/namu-worker" >"$POC_DIR/logs/host-$i.log" 2>&1 &
  PIDS+=($!)
done

"$BIN/namu" build --tasks-dir examples/distributed/tasks \
  --workflows-dir "$POC_DIR/workflows" --out-dir "$POC_DIR/dist" >"$POC_DIR/logs/build.log" 2>&1
"$BIN/namu" publish --out-dir "$POC_DIR/dist" >"$POC_DIR/logs/publish.log"

# Wait until every worker has registered.
expected=$((i + 2))
until [[ $(curl -sf "$NAMU_ORCH_URL/workers" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["workers"]))') -ge $expected ]]; do
  sleep 0.2
done
curl -sf "$NAMU_ORCH_URL/workers" | python3 -c '
import json, sys
for w in json.load(sys.stdin)["workers"]:
    print("  %-22s %-15s %s" % (w["worker_id"], w["platform"], ",".join(sorted(w["runtimes"]))))'

for run in $(seq 1 "$RUNS"); do
  set +e
  "$BIN/namu" run "$WORKFLOW" 0.1.0 --wait >"$POC_DIR/run-$run.json"
  set -e
  python3 - "$POC_DIR/run-$run.json" "$run" <<'PY'
import json, sys
text = open(sys.argv[1]).read()
status = json.loads(text[text.index("{"):])
calls = status["calls_by_worker"]
print(f"run {sys.argv[2]}: {status['status']:<14} {status['elapsed_ms']:>6} ms  "
      f"calls={sum(calls.values())} {dict(calls)}")
PY
done
cat "$POC_DIR/run-$RUNS.json"
