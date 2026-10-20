#!/usr/bin/env bash
# The RAG example, end to end (examples/rag): Wikipedia articles chunked, embedded with
# all-MiniLM-L6-v2 and indexed in Qdrant by a local cluster, then searched.
#
#   ./scripts/rag.sh                          # index 300 articles, then ask the questions below
#   ./scripts/rag.sh "what is overfitting?"   # ask your own
#   ARTICLES=50 ./scripts/rag.sh
#   CHECK=1 ./scripts/rag.sh                  # kill a worker mid-run; check nothing is lost or doubled
#
# Downloads, once, into data/rag: the articles (scripts/rag-fetch.py), the model (from Hugging
# Face) and a Qdrant binary (from its GitHub release). Requires `redis-server`, and the
# `wasm32-wasip2` target for the chunking task.
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT_DIR"

ARTICLES=${ARTICLES:-300}
QDRANT_VERSION=v1.19.2
QDRANT_PORT=${QDRANT_PORT:-6333}
RAG_DIR="$ROOT_DIR/data/rag"
RUN_DIR="$RAG_DIR/run"
export QDRANT_URL="http://127.0.0.1:${QDRANT_PORT}"
export RAG_CORPUS="$RAG_DIR/corpus-$ARTICLES.jsonl"
export NAMU_MODEL_CACHE="$RAG_DIR/model"
mkdir -p "$RAG_DIR/bin"

echo "== articles"
python3 scripts/rag-fetch.py "$RAG_CORPUS" --limit "$ARTICLES"

echo "== model"
(cd examples/rag/query && cargo build -q --release)
QUERY="$ROOT_DIR/examples/rag/query/target/release/namu-rag-query"
"$QUERY" --warm
echo "   sentence-transformers/all-MiniLM-L6-v2 in $NAMU_MODEL_CACHE"

echo "== qdrant $QDRANT_VERSION"
qdrant="$RAG_DIR/bin/qdrant-$QDRANT_VERSION"
if [[ ! -x $qdrant ]]; then
  case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) target=aarch64-apple-darwin ;;
    Darwin-x86_64) target=x86_64-apple-darwin ;;
    Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
    Linux-aarch64) target=aarch64-unknown-linux-musl ;;
    *) echo "no Qdrant release for $(uname -sm)" >&2 && exit 1 ;;
  esac
  curl -sfL "https://github.com/qdrant/qdrant/releases/download/$QDRANT_VERSION/qdrant-$target.tar.gz" |
    tar -xz -C "$RAG_DIR/bin"
  mv "$RAG_DIR/bin/qdrant" "$qdrant"
fi
rm -rf "$RAG_DIR/qdrant" && mkdir -p "$RAG_DIR/qdrant"
# Started in its own directory, where it also leaves its files.
(cd "$RAG_DIR/qdrant" && QDRANT__SERVICE__HTTP_PORT=$QDRANT_PORT \
  QDRANT__SERVICE__GRPC_PORT=$((QDRANT_PORT + 1)) QDRANT__TELEMETRY_DISABLED=true \
  exec "$qdrant" >qdrant.log 2>&1) &
qdrant_pid=$!
cluster_pids=()
cleanup() {
  if ((${#cluster_pids[@]})); then
    kill "${cluster_pids[@]:2}" >/dev/null 2>&1 || true
    sleep 1
    kill -9 "${cluster_pids[@]:2}" >/dev/null 2>&1 || true
    kill "${cluster_pids[@]:0:2}" >/dev/null 2>&1 || true
    redis-cli -p "${REDIS_PORT:-6390}" shutdown nosave >/dev/null 2>&1 || true
  fi
  kill "$qdrant_pid" >/dev/null 2>&1 || true
}
trap cleanup EXIT
until curl -sf "$QDRANT_URL/healthz" >/dev/null; do sleep 0.2; done
curl -sf -X PUT "$QDRANT_URL/collections/wikipedia" -H 'Content-Type: application/json' \
  -d '{"vectors": {"size": 384, "distance": "Cosine"}}' >/dev/null
echo "   collection wikipedia (384 dimensions, cosine) at $QDRANT_URL"

# Every worker runs native tasks (the stream, the model, the store) and WASM ones (chunking).
cluster() {
  POC_DIR="$RUN_DIR" TASKS_DIR=examples/rag/tasks WORKFLOWS=namu-rag-workflows \
    WORKFLOW=index_wikipedia WORKERS="wasm,native wasm,native wasm,native" "$@"
}

echo "== index_wikipedia on 3 workers"
if [[ ${CHECK:-0} == 1 ]]; then
  # Two runs: one to compare with, and one during which a worker is killed. Recovery may embed
  # and store chunks again; stored by their own ids, they are not doubled.
  KEEP=1 RUNS=1 cluster ./scripts/poc-local.sh >"$RUN_DIR.log" 2>&1
  read -r -a cluster_pids <<<"$(sed -n 's/^left running (pids: \(.*\))$/\1/p' "$RUN_DIR.log")"
  NAMU_ORCH_URL="http://127.0.0.1:${ORCH_PORT:-8090}" "$ROOT_DIR/target/release/namu" \
    run index_wikipedia 0.1.0 --wait >"$RUN_DIR/crashed.json" &
  run_pid=$!
  sleep 4
  kill -9 "${cluster_pids[3]}"
  echo "   killed worker-2"
  wait "$run_pid"
  results=("$RUN_DIR/run-1.json" "$RUN_DIR/crashed.json")
else
  RUNS=${RUNS:-1} cluster ./scripts/poc-local.sh | grep -E "^(run|worker)"
  results=("$RUN_DIR/run-1.json")
fi

python3 - "$QDRANT_URL" "${results[@]}" <<'PY'
import json, sys, urllib.request
def load(path):
    text = open(path).read()
    return json.loads(text[text.index("{"):])
runs = [load(path) for path in sys.argv[2:]]
for run in runs:
    assert run["status"] == "succeeded", (run["status"], run["failures"][:3])
report = runs[0]["results"][0]
for run in runs[1:]:
    assert run["results"][0] == report, (run["results"], report)
request = urllib.request.Request(
    sys.argv[1] + "/collections/wikipedia/points/count",
    data=json.dumps({"exact": True}).encode(), headers={"Content-Type": "application/json"})
points = json.load(urllib.request.urlopen(request))["result"]["count"]
assert points == report["chunks"], f"{points} points for {report['chunks']} chunks"
calls = runs[-1]["calls_by_worker"]
print(f"   {report['articles']} articles, {report['chunks']} chunks, {points} points in Qdrant, "
      f"{runs[-1]['elapsed_ms'] / 1000:.1f} s; calls by worker {dict(calls)}")
PY

echo "== search"
if (($# == 0)); then
  set -- "How do neural networks learn from data?" \
    "Which methods find structure in data without labels?" \
    "Why can a model that fits its training data badly fail on new data?"
fi
for question in "$@"; do
  "$QUERY" --top 3 "$question"
done
