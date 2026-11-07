#!/usr/bin/env bash
# End-to-end check of the distributed runtime on a local cluster (Redis + orchestrator + 3
# workers, see scripts/poc-local.sh). Requires `redis-server` and the `wasm32-wasip2` target.
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT_DIR"

check() {
  local workflow=$1 expect=$2
  echo "== $workflow"
  POC_DIR="$ROOT_DIR/data/e2e/$workflow" WORKFLOW="$workflow" RUNS=2 \
    ./scripts/poc-local.sh | grep -E "^(run|worker)"
  python3 - "$ROOT_DIR/data/e2e/$workflow/run-2.json" "$expect" <<'PY'
import json, sys
text = open(sys.argv[1]).read()
status = json.loads(text[text.index("{"):])
expect = json.loads(sys.argv[2])
got = {"status": status["status"], "failures": len(status["failures"])}
result = status["results"][0] if status["results"] else None
if isinstance(result, dict):
    got.update({k: result[k] for k in expect if k in result})
elif "result" in expect:
    got["result"] = result
for key, want in expect.items():
    assert got.get(key) == want, f"{key}: expected {want!r}, got {got.get(key)!r} ({status})"
assert len(status["calls_by_worker"]) > 1, f"work did not spread: {status['calls_by_worker']}"
print(f"   ok: {got}")
PY
}

check doc_stats '{"status": "succeeded", "failures": 0, "docs": 48, "words": 30750, "checksum": 9482608591355693952}'
# The same, with what doc_stats hard-codes given as the run's inputs.
INPUTS="n=48 min_len=6 rounds=4000" \
  check doc_stats_of '{"status": "succeeded", "failures": 0, "docs": 48, "words": 30750, "checksum": 9482608591355693952}'
# The same, with the summary folded document by document (a reduce) rather than collected.
check doc_stats_reduced '{"status": "succeeded", "failures": 0, "docs": 48, "words": 30750, "checksum": 9482608591355693952}'
# The summary written in Python, run by a pool of Python processes on the workers.
check doc_stats_py '{"status": "succeeded", "failures": 0, "docs": 48, "words": 30750, "checksum": 9482608591355693952}'
check nested_stats '{"status": "succeeded", "failures": 0, "result": 30750}'
# The same documents as doc_stats, streamed in one at a time: the same result.
check stream_stats '{"status": "succeeded", "failures": 0, "docs": 48, "words": 30750, "checksum": 9482608591355693952}'
# Windows over the stream: every 8 documents are summarized together; the same words in all.
check stream_windows '{"status": "succeeded", "failures": 0, "result": 30750}'
# Two batch tasks (dynamic, then continuous) label each document; every label is counted.
check label_stats '{"status": "succeeded", "failures": 0, "TREE": 4, "STREAM": 4, "BRANCH": 4, "A": 1, "WORKER": 2}'
check doc_stats_partial '{"status": "partial_failed", "failures": 14, "docs": 34}'
# Tasks that crash (trap) fail their branch; the run carries on without it.
check doc_stats_poison '{"status": "partial_failed", "failures": 14, "docs": 34}'
# Errors as values: sends a flaky service turns down are matched on and retried in a loop, and
# no branch fails.
check retry_stats '{"status": "succeeded", "failures": 0, "docs": 48, "retries": 72, "most": 3}'

# Recovery: kill -9 a worker in the middle of a run. Once its membership expires, its successor
# takes over from the log, and the run must produce the result of an uninterrupted run.
echo "== doc_stats_slow with a worker killed mid-run"
crash_dir="$ROOT_DIR/data/e2e/crash"
# Worker 2 also runs native tasks, among them `deliver`, which appends to $deliveries.
deliveries="$ROOT_DIR/data/e2e/deliveries.log"
rm -f "$deliveries"
KEEP=1 POC_DIR="$crash_dir" NAMU_TASK_SLOTS=2 WORKFLOW=doc_stats_slow \
  WORKERS="wasm,native wasm,native wasm" NAMU_DELIVERIES="$deliveries" \
  ./scripts/poc-local.sh >"$crash_dir.log" 2>&1
read -r -a pids <<<"$(sed -n 's/^left running (pids: \(.*\))$/\1/p' "$crash_dir.log")"
# The cluster's address, CA and a client token.
# shellcheck source=/dev/null
source "$crash_dir/env"
teardown() {
  # Workers first, so they can drain while the orchestrator and Redis are up; then the rest.
  kill "${pids[@]:2}" >/dev/null 2>&1 || true
  for _ in $(seq 20); do
    running=0
    for pid in "${pids[@]:2}"; do
      kill -0 "$pid" 2>/dev/null && running=1
    done
    [[ $running == 0 ]] && break
    sleep 0.5
  done
  kill -9 "${pids[@]:2}" >/dev/null 2>&1 || true
  kill "${pids[@]:0:2}" >/dev/null 2>&1 || true
  redis-cli -p "${REDIS_PORT:-6390}" shutdown nosave >/dev/null 2>&1 || true
}
trap teardown EXIT
# pids: redis, orchestrator, worker-1, worker-2, worker-3

# Inputs that do not fit the workflow are refused before a run starts.
if refused=$("$ROOT_DIR/target/release/namu" \
  run doc_stats_of 0.1.0 --input n=lots --input min_len=6 2>&1); then
  echo "a run with wrong inputs started: $refused" >&2
  exit 1
fi
for problem in "400" "\`n\`: expected i32, got a string" "\`rounds\` (i32) is missing"; do
  if [[ $refused != *"$problem"* ]]; then
    echo "expected \"$problem\" in: $refused" >&2
    exit 1
  fi
done
echo "   ok: wrong inputs refused ($refused)"

# Nobody without a token reaches the API, a client's token opens no worker route, and nothing is
# served in the clear.
status() { curl -s -o /dev/null -w '%{http_code}' --cacert "$NAMU_TLS_CA" "$@"; }
anonymous=$(status -X POST -H 'Content-Type: application/json' -d '{}' "$NAMU_ORCH_URL/runs")
as_client=$(status -X POST -H "Authorization: Bearer $NAMU_TOKEN" -H 'Content-Type: application/json' \
  -d '{"name":"w","addr":"127.0.0.1:1","runtimes":[],"platform":"x"}' "$NAMU_ORCH_URL/workers/register")
forged=$(status -H "Authorization: Bearer namu_$(printf '0%.0s' $(seq 64))" "$NAMU_ORCH_URL/runs/$(uuidgen 2>/dev/null || echo 00000000-0000-0000-0000-000000000000)")
plain=$(curl -s -o /dev/null -w '%{http_code}' "${NAMU_ORCH_URL/https:/http:}/healthz" || true)
if [[ $anonymous != 401 || $as_client != 403 || $forged != 401 || $plain != 000 ]]; then
  echo "expected 401, 403, 401 and no plain HTTP; got $anonymous, $as_client, $forged, $plain" >&2
  exit 1
fi
echo "   ok: no token 401, a client's token on a worker route 403, a forged token 401, no plain HTTP"

# Results arrive as the run goes: a stream of 48 documents, one every 100 ms, each its own result.
namu() { "$ROOT_DIR/target/release/namu" "$@"; }
api() { curl -sf --cacert "$NAMU_TLS_CA" -H "Authorization: Bearer $NAMU_TOKEN" "$NAMU_ORCH_URL$1"; }
streamed=$(namu run doc_ids_streamed 0.1.0 --input n=48 --input delay_ms=100 | sed -n 's/^Run created: //p')
early=0
for _ in $(seq 100); do
  early=$(api "/runs/$streamed" | python3 -c '
import json, sys
status = json.load(sys.stdin)
print(status["results"] if status["status"] == "running" else 0)')
  [[ $early != 0 ]] && break
  sleep 0.05
done
if [[ $early == 0 ]]; then
  echo "no result arrived while doc_ids_streamed went on" >&2
  exit 1
fi
namu results "$streamed" --follow >"$crash_dir/streamed.jsonl"
python3 - "$crash_dir/streamed.jsonl" "$early" <<'PY'
import sys
ids = open(sys.argv[1]).read().split()
assert len(ids) == 48 and len(set(ids)) == 48, f"{len(ids)} results, {len(set(ids))} distinct"
print(f"   ok: {sys.argv[2]} results while the run went on, then all 48 with --follow, each once")
PY
# Alongside it, a run that delivers a side effect per document, keyed by the call.
"$ROOT_DIR/target/release/namu" \
  run doc_stats_slow 0.1.0 --wait >"$crash_dir/crashed.json" &
run_pid=$!
"$ROOT_DIR/target/release/namu" \
  run deliveries_slow 0.1.0 --wait >"$crash_dir/deliveries.json" &
deliveries_pid=$!
# Killed as soon as it has delivered something: it dies waiting for the acknowledgement. However
# long that takes (on a small machine, the other run's work comes first), but not forever.
for _ in $(seq 2400); do
  grep -q " worker-2 " "$deliveries" 2>/dev/null && break
  sleep 0.05
done
if ! grep -q " worker-2 " "$deliveries" 2>/dev/null; then
  echo "worker-2 delivered nothing in 2 minutes" >&2
  exit 1
fi
kill -9 "${pids[3]}"
echo "   killed worker-2"
wait "$run_pid" "$deliveries_pid"
python3 - "$crash_dir/run-1.json" "$crash_dir/crashed.json" <<'PY'
import json, sys
def load(path):
    text = open(path).read()
    return json.loads(text[text.index("{"):])
baseline, crashed = load(sys.argv[1]), load(sys.argv[2])
assert crashed["status"] == "succeeded", crashed
assert not crashed["failures"], crashed["failures"]
assert crashed["results"] == baseline["results"], (crashed["results"], baseline["results"])
print(f"   ok: same result as the uninterrupted run ({crashed['elapsed_ms']} ms vs {baseline['elapsed_ms']} ms)")
PY
# Every document was delivered under one key: the deliveries the engine ran again (their worker
# died after delivering, before their branches moved on) carried the keys they had the first time.
python3 - "$crash_dir/deliveries.json" "$deliveries" <<'PY'
import json, sys
text = open(sys.argv[1]).read()
run = json.loads(text[text.index("{"):])
assert run["status"] == "succeeded" and run["results"] == [48], run
keys = [line.split()[1] for line in open(sys.argv[2])]
assert len(set(keys)) == 48, f"{len(set(keys))} keys for 48 documents"
assert len(keys) > 48, "no delivery was run again: the crash did not test the keys"
print(f"   ok: 48 documents delivered under 48 keys ({len(keys) - 48} deliveries run again)")
PY

# Drain: SIGTERM a worker in the middle of a run. It hands on what it holds and leaves, and the
# run gives the same result. (That nothing runs twice is checked by the engine's drain tests,
# which count every call; a worker that left takes its run statistics with it.)
echo "== doc_stats_slow with a worker drained mid-run"
"$ROOT_DIR/target/release/namu" \
  run doc_stats_slow 0.1.0 --wait >"$crash_dir/drained.json" &
run_pid=$!
sleep 2
kill -TERM "${pids[4]}"
echo "   draining worker-3"
wait "$run_pid"
for _ in $(seq 60); do
  kill -0 "${pids[4]}" 2>/dev/null || break
  sleep 0.5
done
if kill -0 "${pids[4]}" 2>/dev/null; then
  echo "worker-3 did not leave after draining" >&2
  exit 1
fi
python3 - "$crash_dir/run-1.json" "$crash_dir/drained.json" <<'PY'
import json, sys
def load(path):
    text = open(path).read()
    return json.loads(text[text.index("{"):])
baseline, drained = load(sys.argv[1]), load(sys.argv[2])
assert drained["status"] == "succeeded", drained
assert not drained["failures"], drained["failures"]
assert drained["results"] == baseline["results"], (drained["results"], baseline["results"])
print(f"   ok: same result as the uninterrupted run, and worker-3 left")
PY
# The orchestrator restarts: kill -9 it in the middle of a run, and start it again only after
# every lease has run out. The run goes on without it, peer to peer; its results wait on the
# workers until it is back. The workers stay members, and every result arrives once.
echo "== doc_ids_streamed with the orchestrator killed mid-run"
restarted=$(namu run doc_ids_streamed 0.1.0 --input n=48 --input delay_ms=100 | sed -n 's/^Run created: //p')
sleep 1.5
kill -9 "${pids[1]}"
echo "   killed the orchestrator"
sleep 7
pids[1]=$("$crash_dir/start-master")
echo "   started it again"
namu results "$restarted" --follow >"$crash_dir/restarted.jsonl"
api "/runs/$restarted" >"$crash_dir/restarted.json"
api "/workers" >"$crash_dir/workers.json"
python3 - "$crash_dir/restarted.jsonl" "$crash_dir/restarted.json" "$crash_dir/workers.json" <<'PY'
import json, sys
ids = open(sys.argv[1]).read().split()
status = json.load(open(sys.argv[2]))
workers = [w["name"] for w in json.load(open(sys.argv[3]))["workers"]]
assert status["status"] == "succeeded", status
assert len(ids) == 48 and len(set(ids)) == 48, f"{len(ids)} results, {len(set(ids))} distinct"
assert "worker-1" in workers, f"worker-1 is no longer a member: {workers}"
print(f"   ok: all 48 results, each once; the workers stayed ({', '.join(w for w in workers if w != 'orchestrator')})")
PY
# Metrics: the orchestrator's and a worker's, after all of the above.
echo "== metrics"
metrics() { curl -sf "http://127.0.0.1:$1/metrics"; }
orchestrator=$(metrics $((${ORCH_PORT:-8090} + 100)))
worker=$(metrics $((${ORCH_PORT:-8090} + 101)))
# (It restarted since it started a run: its counters began again.)
for series in namu_runs_ended_total namu_reports_stored_total namu_http_requests_total namu_members; do
  grep -q "^$series" <<<"$orchestrator" || { echo "the orchestrator has no $series" >&2; exit 1; }
done
for series in namu_task_calls_total namu_task_call_seconds namu_handoffs_sent_total \
  namu_log_write_seconds namu_runs namu_unacknowledged_messages; do
  grep -q "^$series" <<<"$worker" || { echo "worker-1 has no $series" >&2; exit 1; }
done
calls=$(awk '/^namu_task_calls_total/ {s += $2} END {print s}' <<<"$worker")
echo "   ok: the orchestrator's and worker-1's series are served; worker-1 made $calls task calls"

echo "e2e passed"
