#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd)

cleanup() {
  if [[ -n "${MASTER_PID:-}" ]]; then
    kill "$MASTER_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "${WORKER_PID:-}" ]]; then
    kill "$WORKER_PID" >/dev/null 2>&1 || true
  fi
  docker compose -f "$ROOT_DIR/dev/docker-compose.yml" down >/dev/null 2>&1 || true
}
trap cleanup EXIT

cd "$ROOT_DIR"

is_port_free() {
  if command -v lsof >/dev/null; then
    ! lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1
    return
  fi
  python3 - <<'PY' "$1"
import socket, sys
port = int(sys.argv[1])
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
try:
    s.bind(("127.0.0.1", port))
    free = True
except OSError:
    free = False
s.close()
sys.exit(0 if free else 1)
PY
}

choose_port() {
  local preferred=$1
  local fallback=$2
  if is_port_free "$preferred"; then
    echo "$preferred"
    return 0
  fi
  if is_port_free "$fallback"; then
    echo "$fallback"
    return 0
  fi
  return 1
}

if ! command -v docker >/dev/null; then
  echo "docker is required for e2e" >&2
  exit 1
fi

if ! command -v cargo >/dev/null; then
  echo "cargo is required for e2e" >&2
  exit 1
fi

PG_PORT=${PG_PORT:-}
if [[ -z "$PG_PORT" ]]; then
  PG_PORT=$(choose_port 5432 5433) || {
    echo "postgres ports 5432 and 5433 are in use; set PG_PORT to a free port" >&2
    exit 1
  }
fi

REDIS_PORT=${REDIS_PORT:-}
if [[ -z "$REDIS_PORT" ]]; then
  REDIS_PORT=$(choose_port 6379 6380) || {
    echo "redis ports 6379 and 6380 are in use; set REDIS_PORT to a free port" >&2
    exit 1
  }
fi

BIND_PORT=${BIND_PORT:-}
if [[ -z "$BIND_PORT" ]]; then
  BIND_PORT=$(choose_port 8080 8081) || {
    echo "orchestrator ports 8080 and 8081 are in use; set BIND_PORT to a free port" >&2
    exit 1
  }
fi

export PG_PORT
export REDIS_PORT
export BIND_PORT

docker compose -f dev/docker-compose.yml up -d

export DATABASE_URL="postgres://namu:namu@127.0.0.1:${PG_PORT}/namu"
export REDIS_URL="redis://127.0.0.1:${REDIS_PORT}/"
export ARTIFACTS_DIR="$ROOT_DIR/data/artifacts"
export BIND_ADDR="127.0.0.1:${BIND_PORT}"
export NAMU_ORCH_URL="http://127.0.0.1:${BIND_PORT}"

mkdir -p "$ROOT_DIR/data/artifacts" "$ROOT_DIR/data/cache"

cargo run -p namu-master >/tmp/namu-master.log 2>&1 &
MASTER_PID=$!

for _ in {1..30}; do
  if curl -sf "$NAMU_ORCH_URL/healthz" >/dev/null; then
    break
  fi
  sleep 1
done

cargo run -p namu-worker >/tmp/namu-worker.log 2>&1 &
WORKER_PID=$!

cargo run -p namu-cli -- build --tasks-dir e2e/tasks --workflows-dir e2e/workflows --out-dir e2e/dist
cargo run -p namu-cli -- publish --out-dir e2e/dist

RUN_ID=$(curl -sf -X POST "$NAMU_ORCH_URL/runs" \
  -H 'content-type: application/json' \
  -d '{"workflow_id":"add_workflow","version":"0.1.0"}' \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["run_id"])')

echo "Run ID: $RUN_ID"

STATUS="running"
for _ in {1..30}; do
  STATUS=$(curl -sf "$NAMU_ORCH_URL/runs/$RUN_ID" | python3 -c 'import sys,json; print(json.load(sys.stdin)["status"])')
  if [[ "$STATUS" != "running" ]]; then
    break
  fi
  sleep 1
done

echo "Run status: $STATUS"

curl -sf "$NAMU_ORCH_URL/runs/$RUN_ID/values" | python3 -c 'import sys,json; print(json.dumps(json.load(sys.stdin), indent=2))'
