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
# and WASM compilation). KEEP=1 leaves everything running afterwards. INPUTS gives the workflow's
# inputs, space-separated `name=value` pairs (as `namu run --input` takes them).
#
# Metrics are served at 127.0.0.1:ORCH_PORT+100 (the orchestrator) and +100+i (worker i).
#
# The cluster runs as a production one does: its API over HTTPS with tokens, its mesh over mutual
# TLS, with a CA and tokens made for it in $POC_DIR/secrets. $POC_DIR/env sets what a client needs
# to reach it (`source` it, then use `namu` or curl).
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT_DIR"

REDIS_PORT=${REDIS_PORT:-6390}
ORCH_PORT=${ORCH_PORT:-8090}
WORKERS=${WORKERS:-"wasm,native,python wasm,python wasm"}
WORKFLOW=${WORKFLOW:-doc_stats}
TASKS_DIR=${TASKS_DIR:-examples/distributed/tasks}
WORKFLOWS=${WORKFLOWS:-namu-distributed-workflows}
RUNS=${RUNS:-1}
POC_DIR=${POC_DIR:-$ROOT_DIR/data/poc}
BIN="$ROOT_DIR/target/release"
# What `namu run` is given after the workflow: never empty, which bash 3 refuses under `set -u`.
run_args=(--wait)
for input in ${INPUTS:-}; do
  run_args+=(--input "$input")
done

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
# How Redis starts: with REDIS_PERSIST=1, durable (an append-only file, synced every write), so
# it can be stopped and started again with nothing lost, as MemoryDB is. In cluster mode, as
# MemoryDB always is: one node holding every slot, which refuses a transaction across slots, as
# MemoryDB does. Prints its pid.
if [[ ${REDIS_PERSIST:-0} == 1 ]]; then
  redis_opts="--appendonly yes --appendfsync always"
else
  redis_opts="--save '' --appendonly no"
fi
redis_opts="$redis_opts --dir $POC_DIR --cluster-enabled yes --cluster-config-file redis-nodes.conf"
cat >"$POC_DIR/start-redis" <<EOF
#!/usr/bin/env bash
set -euo pipefail
redis-server --port "$REDIS_PORT" $redis_opts >>"$POC_DIR/logs/redis.log" 2>&1 &
pid=\$!
until redis-cli -p "$REDIS_PORT" ping >/dev/null 2>&1; do sleep 0.1; done
# The first start takes every slot; a restart finds them in its cluster config.
redis-cli -p "$REDIS_PORT" cluster info | grep -q cluster_slots_assigned:16384 ||
  redis-cli -p "$REDIS_PORT" cluster addslotsrange 0 16383 >/dev/null
until redis-cli -p "$REDIS_PORT" cluster info | grep -q cluster_state:ok; do sleep 0.1; done
echo "\$pid"
EOF
chmod +x "$POC_DIR/start-redis"
PIDS+=("$("$POC_DIR/start-redis")")

# The cluster's CA, the certificate its members share, and a token for clients and one for workers.
secrets="$POC_DIR/secrets"
"$BIN/namu" certs --out "$secrets" --host 127.0.0.1 --host localhost >/dev/null
client_token=$("$BIN/namu" token --role client --tokens "$secrets/tokens")
worker_token=$("$BIN/namu" token --role worker --tokens "$secrets/tokens")
cat >"$POC_DIR/env" <<EOF
export NAMU_ORCH_URL="https://127.0.0.1:${ORCH_PORT}"
export NAMU_TLS_CA="$secrets/ca.pem"
export NAMU_TOKEN="$client_token"
EOF
# shellcheck source=/dev/null
source "$POC_DIR/env"
export NAMU_TLS_CERT="$secrets/member.pem" NAMU_TLS_KEY="$secrets/member.key"
export REDIS_URL="redis://127.0.0.1:${REDIS_PORT}/"

# How the orchestrator starts, kept so it can be started again the same way (it keeps no state of
# its own: a restart takes up from Redis). Prints its pid once it answers.
cat >"$POC_DIR/start-master" <<EOF
#!/usr/bin/env bash
set -euo pipefail
source "$POC_DIR/env"
BIND_ADDR="127.0.0.1:${ORCH_PORT}" ARTIFACTS_DIR="$POC_DIR/artifacts" NAMU_TOKENS="$secrets/tokens" \\
  NAMU_TLS_CERT="$secrets/member.pem" NAMU_TLS_KEY="$secrets/member.key" REDIS_URL="redis://127.0.0.1:${REDIS_PORT}/" \\
  NAMU_METRICS_ADDR="127.0.0.1:$((ORCH_PORT + 100))" \\
  "$BIN/namu-master" >>"$POC_DIR/logs/master.log" 2>&1 &
pid=\$!
until curl -sf --cacert "\$NAMU_TLS_CA" "\$NAMU_ORCH_URL/healthz" >/dev/null; do sleep 0.1; done
echo "\$pid"
EOF
chmod +x "$POC_DIR/start-master"
PIDS+=("$("$POC_DIR/start-master")")

# How worker i starts, with the runtimes it serves: `start-worker 5 wasm,python` adds one to a
# cluster left running. Other settings (NAMU_TASK_SLOTS, ...) come from its caller's environment.
# Prints its pid.
cat >"$POC_DIR/start-worker" <<EOF
#!/usr/bin/env bash
set -euo pipefail
source "$POC_DIR/env"
i=\$1
RUST_LOG=${RUST_LOG:-info} WORKER_ID="worker-\$i" NAMU_RUNTIMES="\$2" ARTIFACT_CACHE="$POC_DIR/cache" \\
  NAMU_TOKEN="$worker_token" NAMU_METRICS_ADDR="127.0.0.1:\$(($ORCH_PORT + 100 + i))" \\
  NAMU_TLS_CERT="$secrets/member.pem" NAMU_TLS_KEY="$secrets/member.key" \\
  REDIS_URL="redis://127.0.0.1:${REDIS_PORT}/" "$BIN/namu-worker" >"$POC_DIR/logs/worker-\$i.log" 2>&1 &
echo "\$!"
EOF
chmod +x "$POC_DIR/start-worker"

i=0
for runtimes in $WORKERS; do
  i=$((i + 1))
  PIDS+=("$("$POC_DIR/start-worker" "$i" "$runtimes")")
  echo "worker-$i: $runtimes"
done

"$BIN/namu" build --tasks-dir "$TASKS_DIR" \
  --workflows-dir "$POC_DIR/workflows" --out-dir "$POC_DIR/dist" >"$POC_DIR/logs/build.log" 2>&1
"$BIN/namu" publish --out-dir "$POC_DIR/dist" >"$POC_DIR/logs/publish.log"

# Every worker has joined before the first run starts.
expected=$(wc -w <<<"$WORKERS" | tr -d ' ')
joined() {
  curl -sf --cacert "$NAMU_TLS_CA" -H "Authorization: Bearer $NAMU_TOKEN" "$NAMU_ORCH_URL/workers" |
    python3 -c 'import json,sys; print(sum(1 for w in json.load(sys.stdin)["workers"] if w["id"]))'
}
for _ in $(seq 100); do
  [[ $(joined) -ge $expected ]] && break
  sleep 0.1
done

for run in $(seq 1 "$RUNS"); do
  set +e
  "$BIN/namu" run "$WORKFLOW" 0.1.0 "${run_args[@]}" >"$POC_DIR/run-$run.json"
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
