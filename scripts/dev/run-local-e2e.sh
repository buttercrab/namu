#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

COMPOSE_BASE="dev/docker-compose.yml"
COMPOSE_APP="dev/docker-compose.app.yml"
ORCH_PORT="${ORCH_PORT:-8080}"

docker compose -f "$COMPOSE_BASE" -f "$COMPOSE_APP" up -d

echo "Waiting for master to be ready..."
for _ in $(seq 1 120); do
  if curl -fsS "http://localhost:${ORCH_PORT}/healthz" >/dev/null; then
    break
  fi
  sleep 2
done

if ! curl -fsS "http://localhost:${ORCH_PORT}/healthz" >/dev/null; then
  echo "Master did not become ready in time."
  exit 1
fi

echo "Clearing worker caches..."
rm -rf data/cache/worker-1 data/cache/worker-2
rm -rf e2e/tasks/*/target

echo "Building E2E tasks/workflows..."
docker compose -f "$COMPOSE_BASE" -f "$COMPOSE_APP" exec -T master \
  /usr/local/cargo/bin/cargo run -p namu-cli -- build --tasks-dir e2e/tasks --workflows-dir e2e/workflows --out-dir e2e/dist

echo "Publishing artifacts/workflows..."
docker compose -f "$COMPOSE_BASE" -f "$COMPOSE_APP" exec -T master \
  env NAMU_ORCH_URL="http://localhost:8080" \
  /usr/local/cargo/bin/cargo run -p namu-cli -- publish --out-dir e2e/dist

echo "Submitting run..."
RUN_ID="$(
  curl -sS -X POST "http://localhost:${ORCH_PORT}/runs" \
    -H "Content-Type: application/json" \
    -d '{"workflow_id":"add_workflow","version":"0.1.0"}' \
    | python3 -c 'import json, sys; print(json.load(sys.stdin)["run_id"])'
)"

echo "Run id: ${RUN_ID}"

echo "Waiting for completion..."
for _ in $(seq 1 60); do
  STATUS="$(curl -sS "http://localhost:${ORCH_PORT}/runs/${RUN_ID}" | python3 -c 'import json, sys; print(json.load(sys.stdin).get("status", "unknown"))')"
  if [[ "$STATUS" == "succeeded" || "$STATUS" == "partial_failed" ]]; then
    echo "Run completed with status: ${STATUS}"
    break
  fi
  sleep 2
done

echo "Run status:"
curl -sS "http://localhost:${ORCH_PORT}/runs/${RUN_ID}" | python3 -m json.tool

echo "Run values (context 0):"
curl -sS "http://localhost:${ORCH_PORT}/runs/${RUN_ID}/values" | python3 -m json.tool
