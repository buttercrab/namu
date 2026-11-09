# What a NAMU cluster on Kubernetes is checked for, wherever it runs (scripts/kind.sh, scripts/eks.sh):
# workers joined over HTTPS with their tokens, the example workflows published and run across
# them, and a worker drained by deleting its pod in the middle of a run, as a scale-down or a
# reclaimed spot node would.
#
# Sourced with NS (the namespace), DIR (scratch), BIN (the `namu` CLI's directory) and WORKERS
# (how many there are) set, and the orchestrator's service port-forwarded, with NAMU_ORCH_URL,
# NAMU_TLS_CA and NAMU_TOKEN exported.

members() {
  curl -sf --cacert "$NAMU_TLS_CA" -H "Authorization: Bearer $NAMU_TOKEN" "$NAMU_ORCH_URL/workers" |
    python3 -c 'import json,sys; print(sum(1 for w in json.load(sys.stdin)["workers"] if w["id"]))'
}

cluster_checks() {
  for _ in $(seq 120); do
    curl -sf --cacert "$NAMU_TLS_CA" "$NAMU_ORCH_URL/healthz" >/dev/null && break
    sleep 1
  done
  for _ in $(seq 120); do
    [[ $(members) == "$WORKERS" ]] && break
    sleep 1
  done
  echo "   ok: $(members) workers joined over HTTPS with their token, on mutual TLS"

  echo "== publish"
  cargo run -q -p namu-distributed-workflows --bin export -- "$DIR/workflows" >/dev/null
  "$BIN/namu" build --tasks-dir examples/distributed/tasks --workflows-dir "$DIR/workflows" \
    --out-dir "$DIR/dist" >"$DIR/build.log" 2>&1
  "$BIN/namu" publish --out-dir "$DIR/dist" >"$DIR/publish.log"

  echo "== doc_stats_py: WASM and Python tasks across the cluster's workers"
  "$BIN/namu" run doc_stats_py 0.1.0 --wait >"$DIR/doc_stats_py.json"
  python3 - "$DIR/doc_stats_py.json" <<'PY'
import json, sys
text = open(sys.argv[1]).read()
status = json.loads(text[text.index("{"):])
result = status["results"][0]
assert status["status"] == "succeeded", status
assert (result["docs"], result["words"], result["checksum"]) == (48, 30750, 9482608591355693952), result
assert len(status["calls_by_worker"]) > 1, status["calls_by_worker"]
print(f"   ok: the same checksum as anywhere else; calls by pod {status['calls_by_worker']}")
PY

  echo "== a worker's pod deleted mid-run: it drains"
  local run victim
  run=$("$BIN/namu" run doc_ids_streamed 0.1.0 --input n=48 --input delay_ms=100 |
    sed -n 's/^Run created: //p')
  sleep 1.5
  victim=$(kubectl -n "$NS" get pods -l app.kubernetes.io/component=worker -o name | head -1)
  kubectl -n "$NS" delete "$victim" --wait=false >/dev/null
  echo "   deleted $victim"
  "$BIN/namu" results "$run" --follow >"$DIR/drained.jsonl"
  python3 - "$DIR/drained.jsonl" <<'PY'
import sys
ids = open(sys.argv[1]).read().split()
assert len(ids) == 48 and len(set(ids)) == 48, f"{len(ids)} results, {len(set(ids))} distinct"
PY
  # A worker that drains tells the orchestrator it leaves; one killed would be found dead instead.
  for _ in $(seq 60); do
    kubectl -n "$NS" logs deploy/namu-orchestrator | grep -q "worker left" && break
    sleep 1
  done
  if ! kubectl -n "$NS" logs deploy/namu-orchestrator | grep -q "worker left"; then
    echo "the deleted worker did not drain and leave" >&2
    return 1
  fi
  for _ in $(seq 120); do
    [[ $(members) == "$WORKERS" ]] && break
    sleep 1
  done
  echo "   ok: it drained and left; all 48 results, each once; its replacement joined ($(members) workers)"
}
