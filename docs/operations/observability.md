# Observability

Every NAMU process (the orchestrator, each worker) tells about itself in three ways, set by its
environment:

| | Where | Set with |
|---|---|---|
| Logs | stderr: text, or JSON lines | `NAMU_LOG_FORMAT=json`; `RUST_LOG` filters (default `info`) |
| Metrics | Prometheus text at `http://<addr>/metrics` | `NAMU_METRICS_ADDR=0.0.0.0:9100` |
| Traces | OTLP over HTTP to a collector | `OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318`; `NAMU_TRACE_RATIO` |

On AWS: JSON logs to CloudWatch Logs; metrics scraped by the ADOT collector or the CloudWatch
agent into Amazon Managed Prometheus or CloudWatch; traces to the ADOT collector and on to X-Ray.
The metrics endpoint is plain HTTP without a token: bind it to an address only the scraper reaches.

## A run is one trace

A run's trace id is its run id, so its spans from every worker land in one trace without anything
carried in messages. Each task call is a `task` span (task, version, run, call key), a child of
the run's root span, whose id comes from the run id too; the orchestrator sends the root span,
`run <workflow>`, when the run ends, from when it was asked for. `NAMU_TRACE_RATIO` (0 to 1,
default 1) is the share of runs traced, chosen by run id, so every worker traces the same runs. A
run of millions of calls is millions of spans: trace a share of large runs.

## Metrics

Metrics carry no run ids (a series per run would never stop growing); a task's id is a label.

### Workers

| Metric | Kind | What |
|---|---|---|
| `namu_task_calls_total{task, outcome}` | counter | Task calls; `outcome` is `ok`, `failed` (the task returned an error) or `error` (it could not be run) |
| `namu_task_retries_total{task}` | counter | Failed calls tried again, by the task's retry policy |
| `namu_task_call_seconds{task}` | histogram | How long calls took, slot wait included |
| `namu_handoffs_sent_total`, `namu_handoffs_received_total` | counter | Branches moved between workers |
| `namu_branches_ended_total` | counter | Branches ended in scopes this worker owns |
| `namu_node_fetches_total` | counter | Value-tree nodes fetched from other workers |
| `namu_reports_total{kind}` | counter | Reports recorded by runs' owners here (`result`, `failure`) |
| `namu_runs`, `namu_scopes_open` | gauge | Runs taken part in; scopes owned and open |
| `namu_branches_driving` | gauge | Branches being driven: the load the worker reports for routing |
| `namu_task_slots_busy` | gauge | Task slots in use, of `NAMU_TASK_SLOTS` |
| `namu_unacknowledged_messages` | gauge | Messages kept until acknowledged: grows when a peer or the orchestrator is slow or away |
| `namu_releases_pending` | gauge | Log records waiting on acknowledgements: what a drain waits for |
| `namu_log_write_seconds` | histogram | How long a log batch took to be durable in Redis |
| `namu_log_records_total`, `namu_log_write_failures_total` | counter | Log records written; failed writes (retried) |
| `namu_heartbeat_failures_total` | counter | Heartbeats that did not reach the orchestrator |

Gauges are sampled every second.

### Orchestrator

| Metric | Kind | What |
|---|---|---|
| `namu_runs_started_total`, `namu_runs_ended_total{status}` | counter | Runs started; runs ended, `succeeded` or `partial_failed` |
| `namu_reports_stored_total{kind}` | counter | Results and failures stored (each branch's once) |
| `namu_members`, `namu_runs_running` | gauge | Live workers; runs that have not ended |
| `namu_http_requests_total{route, status}` | counter | API requests, by route pattern and status |
| `namu_http_request_seconds{route}` | histogram | API latency |

### What to watch

- `namu_unacknowledged_messages` rising on every worker at once: the orchestrator is down or slow;
  runs go on, results wait.
- `namu_log_write_seconds` rising: Redis is the bottleneck; every handoff's release waits on it.
- `namu_task_slots_busy` at `NAMU_TASK_SLOTS` everywhere with `namu_branches_driving` growing: add
  workers (they join a running run).
- `rate(namu_task_calls_total{outcome="error"}[5m]) > 0`: tasks that cannot be run (a missing
  artifact, a runtime that fails to load).
