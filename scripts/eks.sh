#!/usr/bin/env bash
# NAMU on AWS, checked end to end: the Terraform in deploy/terraform/aws applied (EKS, MemoryDB,
# S3, the orchestrator's IAM role), the images pushed to ECR, the Helm chart installed, and what
# scripts/lib/cluster-checks.sh checks: workflows run across the workers, a worker drained mid-run.
# Then the orchestrator's pod is deleted, and its successor carries on from MemoryDB. Everything is
# destroyed at the end, unless KEEP=1.
#
#   ./scripts/eks.sh                  # make, check, destroy
#   KEEP=1 ./scripts/eks.sh           # leave it running
#   DESTROY_ONLY=1 ./scripts/eks.sh   # destroy what a kept run made
#
# It costs money, in the account and region the AWS CLI is signed in to: test-sized (two small
# spot workers, small on-demand nodes for the orchestrator, one small MemoryDB node), about
# $0.35 an hour, for the 30 minutes or so a run takes. Its state is in data/eks/$NAME.
#
# Requires aws, terraform, docker (with buildx), kubectl, helm, and the wasm32-wasip2 target. The
# nodes are Graviton (arm64); native example tasks are built for this machine, so only workflows
# of WASM and Python tasks run on the cluster.
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT_DIR"

NAME=${NAME:-namu-test}
REGION=${AWS_REGION:-$(aws configure get region)}
NS=namu
DIR="$ROOT_DIR/data/eks/$NAME"
STATE="$DIR/terraform.tfstate"
PORT=${PORT:-18443}
BIN="$ROOT_DIR/target/release"
TAG=${TAG:-$(git rev-parse --short HEAD)}
ACCOUNT=$(aws sts get-caller-identity --query Account --output text)
REGISTRY="$ACCOUNT.dkr.ecr.$REGION.amazonaws.com"
export KUBECONFIG="$DIR/kubeconfig"
mkdir -p "$DIR"

tf() { terraform -chdir=deploy/terraform/aws "$@"; }
VARS=(
  -var "name=$NAME" -var "region=$REGION" -var force_destroy=true
  -var 'orchestrator_instance_types=["t4g.medium"]'
  -var 'worker_instance_types=["c7g.large","c6g.large","m7g.large","m6g.large"]'
  -var 'worker_nodes={min=1,desired=2,max=3}'
  -var memorydb_node_type=db.t4g.small -var memorydb_replicas=0
)

destroy() {
  echo "== destroy"
  tf destroy -input=false -auto-approve -state="$STATE" "${VARS[@]}" >"$DIR/destroy.log" 2>&1 || {
    echo "terraform destroy failed: see $DIR/destroy.log, and run DESTROY_ONLY=1 $0 again" >&2
    return 1
  }
  for image in orchestrator worker; do
    aws ecr delete-repository --region "$REGION" --repository-name "$NAME/$image" --force \
      >/dev/null 2>&1 || true
  done
  rm -f "$KUBECONFIG"
  echo "   ok: destroyed"
}

if [[ ${DESTROY_ONLY:-0} == 1 ]]; then
  tf init -input=false >/dev/null
  destroy
  exit
fi

# What the cluster says about itself, kept in $DIR/diagnostics before it is destroyed.
diagnose() {
  [[ -f $KUBECONFIG ]] || return 0
  local out="$DIR/diagnostics"
  mkdir -p "$out"
  kubectl get nodes,pods -A -o wide >"$out/pods.txt" 2>&1 || true
  kubectl -n "$NS" describe pods >"$out/describe.txt" 2>&1 || true
  kubectl get events -A --sort-by=.lastTimestamp >"$out/events.txt" 2>&1 || true
  for pod in $(kubectl -n "$NS" get pods -o name 2>/dev/null); do
    kubectl -n "$NS" logs "$pod" --all-containers --tail=300 \
      >"$out/$(basename "$pod").log" 2>&1 || true
    kubectl -n "$NS" logs "$pod" --all-containers --previous --tail=300 \
      >"$out/$(basename "$pod").previous.log" 2>/dev/null || true
  done
  echo "   diagnostics in $out"
}

pf_pid=""
cleanup() {
  local status=$?
  [[ -n $pf_pid ]] && kill "$pf_pid" >/dev/null 2>&1 || true
  [[ $status == 0 ]] || diagnose
  if [[ ${KEEP:-0} != 1 ]]; then
    destroy
  else
    echo "kept: KUBECONFIG=$KUBECONFIG; destroy with DESTROY_ONLY=1 $0"
  fi
}
trap cleanup EXIT

echo "== images ($REGISTRY/$NAME, $TAG)"
aws ecr get-login-password --region "$REGION" |
  docker login --username AWS --password-stdin "$REGISTRY" >/dev/null
for image in orchestrator worker; do
  aws ecr describe-repositories --region "$REGION" --repository-names "$NAME/$image" >/dev/null 2>&1 ||
    aws ecr create-repository --region "$REGION" --repository-name "$NAME/$image" >/dev/null
  docker buildx build -q -f deploy/docker/Dockerfile --platform linux/arm64 --target "$image" \
    -t "$REGISTRY/$NAME/$image:$TAG" --push . >/dev/null
done
cargo build --release -q -p namu-cli

echo "== terraform apply ($NAME in $REGION)"
tf init -input=false >/dev/null
tf apply -input=false -auto-approve -state="$STATE" "${VARS[@]}" >"$DIR/apply.log" 2>&1 || {
  tail -30 "$DIR/apply.log" >&2
  exit 1
}
output() { tf output -state="$STATE" -raw "$1"; }
aws eks update-kubeconfig --region "$REGION" --name "$(output cluster_name)" >/dev/null
kubectl get nodes -L namu.dev/pool,node.kubernetes.io/instance-type | sed 's/^/   /'

echo "== secrets"
kubectl create namespace "$NS" --dry-run=client -o yaml | kubectl apply -f - >/dev/null
svc="namu-orchestrator"
"$BIN/namu" certs --out "$DIR/secrets" --host localhost \
  --host "$svc" --host "$svc.$NS.svc" --host "$svc.$NS.svc.cluster.local" >/dev/null
client=$("$BIN/namu" token --role client --tokens "$DIR/secrets/tokens")
worker=$("$BIN/namu" token --role worker --tokens "$DIR/secrets/tokens")
kubectl -n "$NS" create secret generic namu-tls --from-file=ca.crt="$DIR/secrets/ca.pem" \
  --from-file=tls.crt="$DIR/secrets/member.pem" --from-file=tls.key="$DIR/secrets/member.key" >/dev/null
kubectl -n "$NS" create secret generic namu-tokens --from-file=tokens="$DIR/secrets/tokens" \
  --from-literal=worker="$worker" >/dev/null
redis_url=$(aws secretsmanager get-secret-value --region "$REGION" \
  --secret-id "$(output redis_url_secret)" --query SecretString --output text)
kubectl -n "$NS" create secret generic namu-redis --from-literal=url="$redis_url" >/dev/null

echo "== helm install"
WORKERS=3
helm install namu deploy/helm/namu -n "$NS" --wait --timeout 10m \
  --set image.orchestrator.repository="$REGISTRY/$NAME/orchestrator" \
  --set image.worker.repository="$REGISTRY/$NAME/worker" \
  --set image.orchestrator.tag="$TAG" --set image.worker.tag="$TAG" \
  --set artifacts.url="$(output artifacts_url)" \
  --set orchestrator.serviceAccount.annotations."eks\.amazonaws\.com/role-arn"="$(output orchestrator_role_arn)" \
  --set orchestrator.nodeSelector."namu\.dev/pool"=system \
  --set worker.nodeSelector."namu\.dev/pool"=workers \
  --set 'worker.tolerations[0].key=namu.dev/pool' --set 'worker.tolerations[0].value=workers' \
  --set 'worker.tolerations[0].effect=NoSchedule' \
  --set worker.replicas="$WORKERS" --set worker.taskSlots=2 \
  --set worker.resources.requests.cpu=500m --set worker.resources.requests.memory=512Mi \
  >/dev/null
kubectl -n "$NS" get pods -o wide | sed 's/^/   /'

kubectl -n "$NS" port-forward svc/$svc "$PORT:8080" >"$DIR/port-forward.log" 2>&1 &
pf_pid=$!
export NAMU_ORCH_URL="https://localhost:$PORT" NAMU_TLS_CA="$DIR/secrets/ca.pem" NAMU_TOKEN="$client"
# shellcheck source=lib/cluster-checks.sh
source "$ROOT_DIR/scripts/lib/cluster-checks.sh"
cluster_checks

echo "== task packages in S3, through the orchestrator's IAM role"
bucket=$(output artifacts_url | sed -E 's#s3://([^/]+)/.*#\1#')
kept=$(aws s3 ls "s3://$bucket/" --recursive --region "$REGION" | wc -l | tr -d ' ')
[[ $kept -gt 0 ]] || { echo "no task packages in $bucket" >&2; exit 1; }
echo "   ok: $kept task packages in $bucket"

echo "== the orchestrator's pod deleted: its successor carries on from MemoryDB"
run=$("$BIN/namu" run doc_ids_streamed 0.1.0 --input n=48 --input delay_ms=100 |
  sed -n 's/^Run created: //p')
sleep 1.5
kubectl -n "$NS" delete pod -l app.kubernetes.io/component=orchestrator --wait=false >/dev/null
kubectl -n "$NS" rollout status deploy/namu-orchestrator --timeout 5m >/dev/null
kill "$pf_pid" >/dev/null 2>&1 || true
kubectl -n "$NS" port-forward svc/$svc "$PORT:8080" >"$DIR/port-forward.log" 2>&1 &
pf_pid=$!
for _ in $(seq 120); do
  curl -sf --cacert "$NAMU_TLS_CA" "$NAMU_ORCH_URL/healthz" >/dev/null && break
  sleep 1
done
"$BIN/namu" results "$run" --follow >"$DIR/recovered.jsonl"
python3 - "$DIR/recovered.jsonl" <<'PY'
import sys
ids = open(sys.argv[1]).read().split()
assert len(ids) == 48 and len(set(ids)) == 48, f"{len(ids)} results, {len(set(ids))} distinct"
PY
echo "   ok: all 48 results, each once, across the orchestrator's restart"
echo "eks passed"
