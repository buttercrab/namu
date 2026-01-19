# Value Tree

Each workflow run produces immutable values. NAMU stores these values in a persistent tree so workers can resolve inputs efficiently across branching paths.

## Core idea
- A **context** is an immutable snapshot of values for one execution path.
- New values create child contexts instead of mutating parent contexts.
- Value lookup uses a persistent structure so each context can be queried without copying the full map.

## Storage model
- **Redis** stores context metadata and value deltas (source of truth).
- **Workers** cache frequently used values in memory to reduce round trips.
- **Object store** can hold large values when inline payloads are disabled or too large.

## Input delivery to workers
The orchestrator resolves inputs before enqueuing work and can attach:
- **inline values** (`input_values`) when small enough,
- **hashes** (`input_hashes`) for cache hits,
- **references** (`input_refs`) for object store fetches.

Workers resolve inputs in this order:
1. cache hit by hash
2. inline values
3. object store reference
4. Redis fallback

## Relevant knobs
- `NAMU_INLINE_INPUT_LIMIT_BYTES` (master): inline value size limit; `0` disables inline values.
- `NAMU_VALUE_CACHE_BYTES` (worker): total cache size for value hashes.
- Object store env vars in `docs/operations/configuration.md`.

## Notes
The value tree is immutable by design. Branch failures only affect their leaf contexts; other branches continue independently.
