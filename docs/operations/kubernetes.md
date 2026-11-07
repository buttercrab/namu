# NAMU on Kubernetes (EKS)

One orchestrator and any number of workers, from a Helm chart (`deploy/helm/namu`), on a cluster
the Terraform in `deploy/terraform/aws` makes: EKS with on-demand nodes for the orchestrator and
spot nodes for workers, MemoryDB for Redis, and an S3 bucket for task packages.
`scripts/kind.sh` checks it all on a local kind cluster, secured, with a worker drained mid-run.

## 1. AWS: the cluster and what it needs

```bash
cd deploy/terraform/aws
terraform init
terraform apply -var name=namu -var region=us-east-1
$(terraform output -raw kubeconfig_command)
```

It makes:

| | |
|---|---|
| VPC | three AZs, private subnets for nodes and MemoryDB, one NAT gateway |
| EKS | node groups `system` (on-demand, the orchestrator) and `workers` (spot, several instance types, tainted `namu.dev/pool=workers:NoSchedule`); Graviton (arm64) by default |
| MemoryDB | one shard with a replica, TLS, an ACL user; its URL, password included, in Secrets Manager (`terraform output redis_url_secret`) |
| S3 | a private, versioned, encrypted bucket for task packages (`terraform output artifacts_url`) |
| IAM | a role the orchestrator's service account assumes (IRSA): read and write the bucket, nothing else |

Keep Terraform's state in a remote backend (S3 with locking) for anything shared; the state holds
the MemoryDB password.

## 2. Images

```bash
docker buildx build -f deploy/docker/Dockerfile --platform linux/amd64,linux/arm64 \
  --target orchestrator -t <account>.dkr.ecr.<region>.amazonaws.com/namu-orchestrator:0.1.0 --push .
docker buildx build -f deploy/docker/Dockerfile --platform linux/amd64,linux/arm64 \
  --target worker -t <account>.dkr.ecr.<region>.amazonaws.com/namu-worker:0.1.0 --push .
```

Workers run WASM tasks anywhere, native tasks built for their platform (`linux-aarch64` on
Graviton), and Python tasks in the image's Python: tasks needing packages run on an image built
`FROM` the worker image with them installed (set `image.worker`).

## 3. Secrets

The chart reads three Secrets in its namespace; it never makes secrets itself.

```bash
kubectl create namespace namu

# Redis: the URL Terraform stored.
kubectl -n namu create secret generic namu-redis --from-literal=url="$(aws secretsmanager \
  get-secret-value --secret-id "$(terraform output -raw redis_url_secret)" --query SecretString --output text)"

# Tokens: the orchestrator's file of hashes, and a worker token. Keep the client token.
namu token --role worker --tokens tokens > worker.token
namu token --role client --tokens tokens > client.token
kubectl -n namu create secret generic namu-tokens --from-file=tokens=tokens --from-file=worker=worker.token

# TLS: a certificate naming `namu.mesh` and the orchestrator's service, signed by the cluster's CA.
kubectl -n namu create secret generic namu-tls \
  --from-file=ca.crt=ca.pem --from-file=tls.crt=member.pem --from-file=tls.key=member.key
```

For TLS, cert-manager is the better way: create an `Issuer` from the cluster's CA (or AWS Private
CA, with its issuer) and set `tls.certManager.enabled=true`; the chart's `Certificate` names
everything needed, and cert-manager renews it. Members take up a renewed certificate (and a
changed tokens file) within 10 seconds, without a rollout. `namu certs` makes a CA and
certificate for trying it out.

## 4. Install

```bash
helm install namu deploy/helm/namu -n namu \
  --set image.orchestrator.repository=<account>.dkr.ecr.<region>.amazonaws.com/namu-orchestrator \
  --set image.worker.repository=<account>.dkr.ecr.<region>.amazonaws.com/namu-worker \
  --set artifacts.url="$(terraform output -raw artifacts_url)" \
  --set orchestrator.serviceAccount.annotations."eks\.amazonaws\.com/role-arn"="$(terraform output -raw orchestrator_role_arn)" \
  --set orchestrator.nodeSelector."namu\.dev/pool"=system \
  --set worker.nodeSelector."namu\.dev/pool"=workers \
  --set 'worker.tolerations[0].key=namu.dev/pool' --set 'worker.tolerations[0].value=workers' \
  --set 'worker.tolerations[0].effect=NoSchedule' \
  --set worker.autoscaling.enabled=true
```

Then, from your machine:

```bash
kubectl -n namu port-forward svc/namu-orchestrator 8080:8080 &
export NAMU_ORCH_URL=https://localhost:8080 NAMU_TLS_CA=ca.pem NAMU_TOKEN=$(cat client.token)
namu workers
```

For clients outside the cluster, put a load balancer in front of the orchestrator's service
(`orchestrator.service.type=LoadBalancer`, with an internal NLB's annotations), and name it in the
certificate (`tls.certManager.extraDnsNames`).

## How it behaves

- **The orchestrator** is one pod, replaced on failure (`Recreate`: two would be two mesh peer
  0s). It keeps no state: a new one takes up from MemoryDB, and runs go on while it is away, their
  results waiting on the workers.
- **Workers** join a running run as they come. One stopped (a scale-down, a node drained, a rollout,
  a spot node reclaimed) gets SIGTERM and drains: it takes no new work, hands on what it holds, and
  leaves once everything is durable, within `worker.terminationGracePeriodSeconds` (110 seconds by
  default, inside a spot instance's two minutes). One killed outright is taken over by a successor
  from the log, a lease (5 seconds) after.
- **Spot interruptions**: EKS managed node groups drain a node on a rebalance recommendation. To
  drain on the two-minute interruption notice itself, run the AWS Node Termination Handler (queue
  mode) or use Karpenter, which handles it; either drains through the pods' grace periods.
- **Autoscaling**: the HPA scales workers on CPU, and scales down one worker a minute (each drains).
  Scaling on NAMU's own metrics (`namu_task_slots_busy`, `namu_branches_driving`) needs a metrics
  adapter (KEDA, or the Prometheus adapter). Nodes follow pods with the Cluster Autoscaler or
  Karpenter.
- **Disruptions**: a PodDisruptionBudget lets one worker drain at a time.
- **Metrics** are served on port 9100 of every pod (`docs/operations/observability.md`); traces go
  to `telemetry.otlpEndpoint`, e.g. an ADOT collector.

## Not covered yet

- Applying the Terraform and installing on EKS has not been done from this repository: the chart
  is checked on kind (`scripts/kind.sh`), the Terraform with `terraform validate`.
- No KEDA scaler on NAMU's own metrics ships with the chart.
- A worker's token is an environment variable: rotating it needs a rollout of the workers.
