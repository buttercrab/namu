# Testing

NAMU is tested in tiers, from the engine alone to real pipelines on a misbehaving cluster.

| Tier | What | Where it runs |
|---|---|---|
| Unit and engine | Every workflow on one worker, three, three over TCP and three over mutual TLS, which must agree; recovery at every moment of a run; the control plane's rules over a memory store | `cargo test`, every PR |
| End to end | A secured local cluster (Redis, the orchestrator, workers as processes): the distributed examples to their known checksums; a worker killed, a worker drained, the orchestrator killed and restarted; results followed as a run goes; metrics scraped | `scripts/e2e.sh`, every PR |
| Kubernetes | The images and the Helm chart on a kind cluster, secured; a workflow of WASM and Python tasks; a worker's pod deleted mid-run | `scripts/kind.sh`, every PR |
| Crawl | A crawler (Python tasks) on a fixture site with flaky pages (503, 429) and dead links: exactly the URLs within its depth, every page fetched once, dead links dropped | `scripts/crawl.sh`: small every PR, 3000 pages nightly |
| RAG | Wikipedia articles chunked, embedded (MiniLM) and indexed (Qdrant), a worker killed mid-run: the same index, nothing doubled; recall@5 on golden questions at least 0.8 | `scripts/rag.sh`: 60 articles every PR, 300 nightly |
| Chaos | One run while a worker is killed, another drained, the orchestrator killed and restarted, Redis shut down and restarted from its append-only file, and a worker joins: every result once, the same as an undisturbed run | `scripts/chaos.sh`, nightly |
| Soak | A stream that does not end, for 30 minutes: what a worker holds in memory and what the log keeps stay flat | `scripts/soak.sh`, nightly |

Each script runs on its own: see its header for its knobs (`PAGES`, `DEPTH`, `DOCS`, `DURATION`,
`ARTICLES`, ...), and `REDIS_PORT`/`ORCH_PORT` to run two at once.

## What the rough tiers found

- Chaos found that a report the orchestrator could not store while Redis restarted was lost: the
  orchestrator dropped it, and its owner never sent it again. Now the orchestrator tries until it
  is stored, and a restarted orchestrator is sent again what workers kept for it (its membership
  carries an epoch).
- Chaos found that a few seconds of Redis down buried every worker: their heartbeats could not be
  stored, their leases ran out, and the first sweep after counted them dead. A lease now runs out
  only while the orchestrator could have renewed it.
- Chaos found that `namu results --follow` gave up the moment the orchestrator did not answer.
- Soak found that the log kept every branch end of a scope until the scope was over: a stream
  with no end grew the log by a record per item, without end. A scope now settles: the ends before
  a point that are accounted for (it collects nothing, a fold has folded them, a window has gone
  out with them) leave the log, which holds a scope's ends in flight. (A first, looser check of
  the soak let this through; it now requires the log to stay flat.)
- The crawl found that the local cluster started a run before its workers had joined.
- Kubernetes found that workers started before the orchestrator exited instead of waiting for it.

## Not covered yet

- Labeling with a Python model, batched: Python tasks are single calls; Python batch and stream
  tasks are not supported yet.
- Runs of days, and clusters of more than a few machines.
