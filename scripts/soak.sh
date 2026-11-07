#!/usr/bin/env bash
# Soak: a run that goes on and on, a stream of documents each its own result, while what the
# cluster holds is sampled. Runs are meant to last days; what a worker holds in memory, and what
# the log keeps in Redis, must stay as large as the work in flight, not grow with the run's length.
#
#   ./scripts/soak.sh                  # two minutes
#   DURATION=3600 ./scripts/soak.sh    # an hour (nightly)
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT_DIR"

DURATION=${DURATION:-120}
EVERY=${EVERY:-5}
DIR="$ROOT_DIR/data/soak"
REDIS_PORT=${REDIS_PORT:-6390}
ORCH_PORT=${ORCH_PORT:-8090}
rm -rf "$DIR"

echo "== a stream with no end in sight, on 3 workers, for ${DURATION}s"
# The first run is short; the soak is the next one, started below.
KEEP=1 POC_DIR="$DIR" WORKFLOW=doc_ids_streamed WORKERS="wasm,python wasm,python wasm,python" \
  INPUTS="n=10 delay_ms=0" ./scripts/poc-local.sh >"$DIR.log" 2>&1
read -r -a pids <<<"$(sed -n 's/^left running (pids: \(.*\))$/\1/p' "$DIR.log")"
teardown() {
  kill -9 "${pids[@]:2}" >/dev/null 2>&1 || true
  kill "${pids[1]}" >/dev/null 2>&1 || true
  redis-cli -p "$REDIS_PORT" shutdown nosave >/dev/null 2>&1 || true
}
trap teardown EXIT
# shellcheck source=/dev/null
source "$DIR/env"

"$ROOT_DIR/target/release/namu" run doc_ids_streamed 0.1.0 --input n=100000000 --input delay_ms=1 \
  >"$DIR/run.txt"
python3 - "$DURATION" "$EVERY" "$REDIS_PORT" $((ORCH_PORT + 101)) $((ORCH_PORT + 102)) $((ORCH_PORT + 103)) \
  "${pids[2]}" "${pids[3]}" "${pids[4]}" <<'PY'
import subprocess, sys, time, urllib.request

duration, every, redis_port = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
ports, pids = sys.argv[4:7], sys.argv[7:10]

def rss_mb(pid):
    return int(subprocess.run(["ps", "-o", "rss=", "-p", pid], capture_output=True, text=True).stdout or 0) / 1024

def gauge(port, name):
    text = urllib.request.urlopen(f"http://127.0.0.1:{port}/metrics", timeout=5).read().decode()
    return sum(float(line.split()[-1]) for line in text.splitlines() if line.startswith(name + " ") or line.startswith(name + "{"))

def redis(*args):
    return subprocess.run(["redis-cli", "-p", redis_port, *args], capture_output=True, text=True).stdout.split()

def log_records():
    """Records in the workers' log, every run's: branches, reports, nodes, ends and the rest."""
    total = 0
    for key in redis("--scan", "--pattern", "namu:log:*"):
        kind = redis("type", key)[0]
        size = {"hash": "hlen", "list": "llen", "set": "scard", "zset": "zcard"}.get(kind)
        total += int(redis(size, key)[0]) if size else 1
    return total

samples = []
start = time.time()
while time.time() - start < duration:
    time.sleep(every)
    calls = sum(gauge(p, "namu_task_calls_total") for p in ports)
    sample = {
        "t": round(time.time() - start),
        "calls": int(calls),
        "rss_mb": round(max(rss_mb(p) for p in pids), 1),
        "log": log_records(),
        "unacked": int(sum(gauge(p, "namu_unacknowledged_messages") for p in ports)),
    }
    samples.append(sample)
    print(f"   {sample}", flush=True)

# After warming up, the worst a worker holds and what the log keeps must not grow with the run.
middle, last = samples[len(samples) // 3], samples[-1]
worked = last["calls"] - middle["calls"]
assert worked > 1000, f"the run did little ({worked} calls): the soak proves nothing"
assert last["rss_mb"] <= middle["rss_mb"] * 1.5 + 20, f"worker memory grew: {middle['rss_mb']} -> {last['rss_mb']} MB"
# Flat, not merely slower: a log that grows with the run grows without end over days.
peak_log = max(s["log"] for s in samples[len(samples) // 3:])
assert peak_log <= middle["log"] * 1.25 + 2000, f"the log grew: {middle['log']} -> {peak_log} records"
print(f"   ok: {last['calls']} calls; worker memory {middle['rss_mb']} -> {last['rss_mb']} MB, "
      f"the log {middle['log']} -> {last['log']} records, while the run went on")
PY
echo "soak passed"
