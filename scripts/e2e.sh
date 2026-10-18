#!/usr/bin/env bash
# End-to-end check of the distributed runtime on a local cluster (Redis + orchestrator + 3
# workers, see scripts/poc-local.sh). Requires `redis-server` and the `wasm32-wasip2` target.
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT_DIR"

check() {
  local workflow=$1 expect=$2
  echo "== $workflow"
  env ${EXTRA_ENV:-} POC_DIR="$ROOT_DIR/data/e2e/$workflow" WORKFLOW="$workflow" RUNS=2 \
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
check nested_stats '{"status": "succeeded", "failures": 0, "result": 30750}'
check doc_stats_partial '{"status": "partial_failed", "failures": 14, "docs": 34}'
# Tasks that crash every attempt are retried, then their branches fail.
EXTRA_ENV="NAMU_LEASE_MS=1000" check doc_stats_poison \
  '{"status": "partial_failed", "failures": 14, "docs": 34}'

# Recovery: kill -9 a worker in the middle of a run; the run must still produce the result of an
# uninterrupted run.
echo "== doc_stats_slow with a worker killed mid-run"
crash_dir="$ROOT_DIR/data/e2e/crash"
KEEP=1 POC_DIR="$crash_dir" NAMU_LEASE_MS=2000 NAMU_CONCURRENCY=2 WORKFLOW=doc_stats_slow \
  ./scripts/poc-local.sh >"$crash_dir.log" 2>&1
read -r -a pids <<<"$(sed -n 's/^left running (pids: \(.*\))$/\1/p' "$crash_dir.log")"
teardown() {
  kill "${pids[@]}" >/dev/null 2>&1 || true
  redis-cli -p "${REDIS_PORT:-6390}" shutdown nosave >/dev/null 2>&1 || true
}
trap teardown EXIT
# pids: redis, orchestrator, worker-1, worker-2, worker-3
NAMU_ORCH_URL="http://127.0.0.1:${ORCH_PORT:-8090}" "$ROOT_DIR/target/release/namu" \
  run doc_stats_slow 0.1.0 --wait >"$crash_dir/crashed.json" &
run_pid=$!
sleep 2
kill -9 "${pids[3]}"
echo "   killed worker-2"
wait "$run_pid"
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
echo "e2e passed"
