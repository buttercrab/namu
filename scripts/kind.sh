#!/usr/bin/env bash
# NAMU on Kubernetes, checked end to end on a local kind cluster: the images, the Helm chart
# (secured: HTTPS, tokens, mutual TLS), workflows run through it, and a worker drained by deleting
# its pod in the middle of a run, as a scale-down or a reclaimed spot node would.
#
#   ./scripts/kind.sh            # make the cluster, check, delete the cluster
#   KEEP=1 ./scripts/kind.sh     # leave it running (kind delete cluster --name namu-kind)
#
# Requires docker (with buildx), kind, helm, kubectl, redis-server's image pulls, and the
# wasm32-wasip2 target. Native example tasks are built for this machine, so only workflows of
# WASM and Python tasks run on the cluster's Linux workers.
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT_DIR"

CLUSTER=${CLUSTER:-namu-kind}
NS=namu
DIR="$ROOT_DIR/data/kind"
PORT=${PORT:-18080}
BIN="$ROOT_DIR/target/release"
rm -rf "$DIR" && mkdir -p "$DIR"

pf_pid=""
cleanup() {
  [[ -n $pf_pid ]] && kill "$pf_pid" >/dev/null 2>&1 || true
  if [[ ${KEEP:-0} != 1 ]]; then
    kind delete cluster --name "$CLUSTER" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

echo "== images"
for target in orchestrator worker; do
  docker buildx build -q -f deploy/docker/Dockerfile --target "$target" -t "namu-$target:dev" --load . >/dev/null
done
cargo build --release -q -p namu-cli

echo "== cluster"
kind get clusters 2>/dev/null | grep -qx "$CLUSTER" || kind create cluster --name "$CLUSTER" --wait 120s >/dev/null
kind load docker-image --name "$CLUSTER" namu-orchestrator:dev namu-worker:dev >/dev/null
kubectl config use-context "kind-$CLUSTER" >/dev/null
kubectl create namespace "$NS" --dry-run=client -o yaml | kubectl apply -f - >/dev/null

echo "== redis, certificates and tokens"
kubectl -n "$NS" run redis --image=redis:7-alpine --port=6379 --expose \
  --command -- redis-server --appendonly yes >/dev/null
svc="namu-orchestrator"
"$BIN/namu" certs --out "$DIR/secrets" --host localhost \
  --host "$svc" --host "$svc.$NS.svc" --host "$svc.$NS.svc.cluster.local" >/dev/null
client=$("$BIN/namu" token --role client --tokens "$DIR/secrets/tokens")
worker=$("$BIN/namu" token --role worker --tokens "$DIR/secrets/tokens")
kubectl -n "$NS" create secret generic namu-tls --from-file=ca.crt="$DIR/secrets/ca.pem" \
  --from-file=tls.crt="$DIR/secrets/member.pem" --from-file=tls.key="$DIR/secrets/member.key" >/dev/null
kubectl -n "$NS" create secret generic namu-tokens --from-file=tokens="$DIR/secrets/tokens" \
  --from-literal=worker="$worker" >/dev/null
kubectl -n "$NS" create secret generic namu-redis --from-literal=url="redis://redis:6379/" >/dev/null

echo "== helm install"
helm install namu deploy/helm/namu -n "$NS" --wait --timeout 5m \
  --set image.orchestrator.tag=dev --set image.worker.tag=dev \
  --set worker.replicas=3 --set worker.taskSlots=2 \
  --set worker.resources.requests.cpu=100m --set worker.resources.requests.memory=128Mi \
  --set orchestrator.resources.requests.cpu=50m --set telemetry.logFormat=text >/dev/null
kubectl -n "$NS" get pods -o wide | sed 's/^/   /'
# Workers may start before the orchestrator answers: they wait for it rather than crash.
restarts=$(kubectl -n "$NS" get pods -l app.kubernetes.io/name=namu \
  -o jsonpath='{range .items[*]}{.status.containerStatuses[0].restartCount}{"\n"}{end}' | awk '{s += $1} END {print s}')
if [[ $restarts != 0 ]]; then
  echo "pods restarted $restarts times" >&2
  exit 1
fi

kubectl -n "$NS" port-forward svc/$svc "$PORT:8080" >"$DIR/port-forward.log" 2>&1 &
pf_pid=$!
export NAMU_ORCH_URL="https://localhost:$PORT" NAMU_TLS_CA="$DIR/secrets/ca.pem" NAMU_TOKEN="$client"
for _ in $(seq 60); do
  curl -sf --cacert "$NAMU_TLS_CA" "$NAMU_ORCH_URL/healthz" >/dev/null && break
  sleep 1
done
members() {
  curl -sf --cacert "$NAMU_TLS_CA" -H "Authorization: Bearer $NAMU_TOKEN" "$NAMU_ORCH_URL/workers" |
    python3 -c 'import json,sys; print(sum(1 for w in json.load(sys.stdin)["workers"] if w["id"]))'
}
for _ in $(seq 60); do
  [[ $(members) == 3 ]] && break
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
  exit 1
fi
for _ in $(seq 60); do
  [[ $(members) == 3 ]] && break
  sleep 1
done
echo "   ok: it drained and left; all 48 results, each once; its replacement joined ($(members) workers)"
echo "kind passed"
