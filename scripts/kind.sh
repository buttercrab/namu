#!/usr/bin/env bash
# NAMU on Kubernetes, checked end to end on a local kind cluster: the images, the Helm chart
# (secured: HTTPS, tokens, mutual TLS), and what scripts/lib/cluster-checks.sh checks: workflows
# run through it, and a worker drained by deleting its pod in the middle of a run.
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
WORKERS=3
# shellcheck source=lib/cluster-checks.sh
source "$ROOT_DIR/scripts/lib/cluster-checks.sh"
cluster_checks
echo "kind passed"
