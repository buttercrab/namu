#!/usr/bin/env bash
# Local distribution demo: Redis + orchestrator + several workers on this machine.
#
#   ./scripts/poc-local.sh                      # run the doc_stats workflow
#   WORKFLOW=bench_local ./scripts/poc-local.sh
#   WORKERS="wasm native" WORKFLOW=bench_handoff ./scripts/poc-local.sh
#
# WORKERS is a space-separated list, one entry per worker, each a comma-separated list of
# runtimes. TASKS_DIR and WORKFLOWS (the workflow crate) pick another example, e.g.
# TASKS_DIR=examples/rag/tasks WORKFLOWS=namu-rag-workflows. RUNS=N runs the workflow N times (the first run includes cold start: artifact download
# and WASM compilation). KEEP=1 leaves everything running afterwards.
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT_DIR"

REDIS_PORT=${REDIS_PORT:-6390}
ORCH_PORT=${ORCH_PORT:-8090}
WORKERS=${WORKERS:-"wasm,native wasm wasm"}
WORKFLOW=${WORKFLOW:-doc_stats}
TASKS_DIR=${TASKS_DIR:-examples/distributed/tasks}
WORKFLOWS=${WORKFLOWS:-namu-distributed-workflows}
RUNS=${RUNS:-1}
POC_DIR=${POC_DIR:-$ROOT_DIR/data/poc}
BIN="$ROOT_DIR/target/release"

PIDS=()

# Stops workers: SIGTERM drains each one; one still running after 10 seconds is killed.
stop_workers() {
  kill "$@" >/dev/null 2>&1 || true
  for _ in $(seq 20); do
    local running=0
    for pid in "$@"; do
      kill -0 "$pid" 2>/dev/null && running=1
    done
    [[ $running == 0 ]] && return 0
    sleep 0.5
  done
  kill -9 "$@" >/dev/null 2>&1 || true
}

cleanup() {
  if [[ "${KEEP:-0}" == "1" ]]; then
    echo "left running (pids: ${PIDS[*]})"
    return
  fi
  # Workers first: they drain and leave while the orchestrator and Redis are still up.
  stop_workers "${PIDS[@]:2}"
  for pid in "${PIDS[@]:0:2}"; do kill "$pid" >/dev/null 2>&1 || true; done
  redis-cli -p "$REDIS_PORT" shutdown nosave >/dev/null 2>&1 || true
}
trap cleanup EXIT

rm -rf "$POC_DIR" && mkdir -p "$POC_DIR/logs"

cargo build --release -q -p namu-master -p namu-worker -p namu-cli
cargo run -q -p "$WORKFLOWS" --bin export -- "$POC_DIR/workflows" >/dev/null

if redis-cli -p "$REDIS_PORT" ping >/dev/null 2>&1; then
  echo "port $REDIS_PORT is already serving Redis; stop it or set REDIS_PORT" >&2
  exit 1
fi
redis-server --port "$REDIS_PORT" --save "" --appendonly no >"$POC_DIR/logs/redis.log" 2>&1 &
PIDS+=($!)
until redis-cli -p "$REDIS_PORT" ping >/dev/null 2>&1; do sleep 0.1; done

export REDIS_URL="redis://127.0.0.1:${REDIS_PORT}/"
export NAMU_ORCH_URL="http://127.0.0.1:${ORCH_PORT}"

BIND_ADDR="127.0.0.1:${ORCH_PORT}" ARTIFACTS_DIR="$POC_DIR/artifacts" \
  "$BIN/namu-master" >"$POC_DIR/logs/master.log" 2>&1 &
PIDS+=($!)
until curl -sf "$NAMU_ORCH_URL/healthz" >/dev/null; do sleep 0.1; done

i=0
for runtimes in $WORKERS; do
  i=$((i + 1))
  RUST_LOG=${RUST_LOG:-info} WORKER_ID="worker-$i" NAMU_RUNTIMES="$runtimes" ARTIFACT_CACHE="$POC_DIR/cache" \
    "$BIN/namu-worker" >"$POC_DIR/logs/worker-$i.log" 2>&1 &
  PIDS+=($!)
  echo "worker-$i: $runtimes"
done

"$BIN/namu" build --tasks-dir "$TASKS_DIR" \
  --workflows-dir "$POC_DIR/workflows" --out-dir "$POC_DIR/dist" >"$POC_DIR/logs/build.log" 2>&1
"$BIN/namu" publish --out-dir "$POC_DIR/dist" >"$POC_DIR/logs/publish.log"

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
      f"calls={sum(calls.values())} {dict(calls)}  remote_fetches={status['remote_node_fetches_by_worker']}")
PY
done
cat "$POC_DIR/run-$RUNS.json"
