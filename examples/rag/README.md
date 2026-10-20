# RAG example: Wikipedia into a vector index

A real ingestion pipeline for retrieval-augmented generation, on the distributed runtime: real
articles, a real embedding model, a real vector database.

```
Wikipedia ──▶ articles ──▶ chunk ──▶ embed ──────────▶ store ──▶ Qdrant ◀── query
(fetched)     native        WASM      native batch      native              "How do neural
              stream        per        all-MiniLM-L6-v2  upsert              networks learn?"
                            article    on candle         by chunk id
```

```bash
./scripts/rag.sh                    # 300 articles of Category:Machine learning, then three questions
./scripts/rag.sh "what is overfitting?" "how are images classified?"   # your own questions
ARTICLES=50 ./scripts/rag.sh        # fewer
CHECK=1 ./scripts/rag.sh            # kill a worker mid-run: the index ends up the same, nothing doubled
```

The script downloads, once, into `data/rag`: the articles (`scripts/rag-fetch.py`, plain text
from the MediaWiki API, CC BY-SA 4.0, each with its URL and revision), the model
([sentence-transformers/all-MiniLM-L6-v2](https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2),
Apache-2.0, at a pinned revision), and a Qdrant release binary. Then it starts Qdrant and a local
cluster (Redis, the orchestrator, three workers), runs `index_wikipedia`, checks that Qdrant holds
exactly as many points as chunks were made, and asks the index three questions.

## The workflow

```rust
#[workflow(id = "index_wikipedia")]
pub fn index_wikipedia() -> IndexReport {
    let limit = 300;
    let words = 200;
    let collection = "wikipedia";
    let article = articles(limit);              // a stream: a branch per article, as it arrives
    let chunks = chunk(article, words);         // Vec<Chunk>
    let embedded = embed(chunks);               // a branch per chunk; batched across all of them
    let stored = store(embedded, collection);   // upsert by the chunk's own id
    let per_article = count_stored(stored);     // fan-in: chunks per article
    index_report(per_article)                   // fan-in: articles
}
```

| Task | Runtime | What it does |
|---|---|---|
| `articles` | native stream | reads the corpus file (`RAG_CORPUS`), an article at a time |
| `chunk` | WASM | splits an article into chunks of about 200 words, each overlapping the last by a quarter, titled |
| `embed` | native batch (32) | all-MiniLM-L6-v2 on candle: mean-pooled, unit-length 384-dimensional vectors; Accelerate on macOS, Metal with `NAMU_DEVICE=metal` |
| `store` | native | upserts into Qdrant (`QDRANT_URL`), with the chunk's text, title and URL as payload |
| `count_stored`, `index_report` | WASM | the two fan-ins |

`model/` is the model, shared by `embed` and by `query/`, which asks the index a question, so a
question and the chunks it is compared with are embedded alike.

## What it shows

- **Streaming ingestion**: chunking and embedding start with the first article, not after the
  last.
- **Batching across branches**: chunks from every article in flight fill the model's batches.
- **Recovery without duplicates**: with `CHECK=1` a worker is killed mid-run. Its successor runs
  again what it had not finished; chunks are upserted by their own ids (article, index), so the
  collection holds each once, and the report is the same as without the crash.

## Numbers (Apple M-series, 14 cores, CPU)

| | |
|---|---|
| Corpus | 282 articles (stubs dropped), 452k words |
| Index | 3,053 chunks in 36 s on 3 workers on one machine: about 86 chunks/s |
| Model alone, one process | 64 chunks/s (Accelerate), 33 without |

The model is the bottleneck; on one machine the three workers share its cores. Across machines
(or with a GPU each), throughput grows with the workers.
