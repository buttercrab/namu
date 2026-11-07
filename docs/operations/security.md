# Security

Task code is trusted (design §3): there is no sandbox. So who may **publish** a task, and who may
**join** the cluster, is what security comes down to: publishing a task is shipping code every
worker runs, and a member of the mesh sees the data of every run that reaches it.

## What is protected, and how

| Surface | Protection |
|---|---|
| Orchestrator API (publish, run, results; worker membership) | HTTPS, and a bearer token with a role on every request but `/healthz` |
| Mesh (workers and the orchestrator, peer to peer) | Mutual TLS: each side shows a certificate signed by the cluster's CA, or no connection |
| Redis (control-plane state, the durable log) | Its own password and TLS: `REDIS_URL=rediss://:<password>@host:6380/` |
| Python task processes | Local pipes on the worker; nothing listens |

Without the settings below, the orchestrator and workers refuse to start. `NAMU_INSECURE=1` runs
them with an open API and a plaintext mesh, for a machine nobody else reaches
(`scripts/poc-cross.sh` does, to show platforms mixing with nothing in the way).

## Tokens

Two roles:

| Role | May |
|---|---|
| `client` | publish tasks and workflows, start runs, read status and results, list workers, read tasks |
| `worker` | register, heartbeat, drain and leave; read tasks and download their artifacts |

The orchestrator keeps only hashes, in the file `NAMU_TOKENS` names, one per line:
`<role> sha256:<hex>` (`#` starts a comment). `namu token` makes a token, prints it, and appends its
line:

```bash
namu token --role client --tokens /etc/namu/tokens    # give the printed token to a client
namu token --role worker --tokens /etc/namu/tokens    # and this one to workers (NAMU_TOKEN)
```

Clients send `Authorization: Bearer <token>`; the CLI sends `NAMU_TOKEN`. A missing or unknown
token is `401`, a token whose role may not do something `403`. Several tokens per role may be
listed, so one is rotated by adding the new one, moving its holders over, and removing the old;
the orchestrator reads the file when it starts.

## Certificates

Every member (the orchestrator and each worker) has a certificate and key, signed by the cluster's
CA. Members reach each other by address, so every certificate carries the DNS name `namu.mesh`,
which every mesh connection checks. The orchestrator's certificate serves its HTTPS too, so it also
names the hosts the API is reached by (unless a load balancer terminates HTTPS in front of it).

- `NAMU_TLS_CA`: the cluster's CA certificate (PEM). Clients that reach the API with it set trust
  it; otherwise they use the system's roots.
- `NAMU_TLS_CERT`, `NAMU_TLS_KEY`: this member's certificate (with its chain) and key (PEM).

In production, issue them from your PKI (on AWS, AWS Private CA; keep keys in Secrets Manager).
For development, `namu certs` makes a CA and one member certificate:

```bash
namu certs --out ./secrets --host 127.0.0.1 --host localhost
```

`scripts/poc-local.sh` does all of this for its cluster, which runs secured as in production, and
writes `$POC_DIR/env` with what a client needs to reach it.

## Not covered yet

- Tokens are read once, at start; rotating one means restarting the orchestrator.
- Certificates are read once, at start; renewing them means restarting the member.
- Every member is equally trusted; a certificate does not bind a member to its worker id.
- No per-run or per-workflow permissions: a `client` token may do every client thing.
