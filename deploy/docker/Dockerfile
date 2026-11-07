# syntax=docker/dockerfile:1.7
#
# NAMU's images, one target each:
#
#   docker buildx build -f deploy/docker/Dockerfile --target orchestrator -t namu-orchestrator .
#   docker buildx build -f deploy/docker/Dockerfile --target worker       -t namu-worker .
#   docker buildx build -f deploy/docker/Dockerfile --target cli          -t namu-cli .
#
# Add `--platform linux/amd64,linux/arm64` for both architectures (Graviton nodes are arm64). A
# worker image runs WASM tasks anywhere, native tasks built for its platform, and Python tasks in
# its Python: tasks that need Python packages run on an image `FROM` this one, with them installed.

ARG RUST_IMAGE=rust:1-bookworm
ARG BASE_IMAGE=debian:bookworm-slim

FROM ${RUST_IMAGE} AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked -p namu-master -p namu-worker -p namu-cli \
    && mkdir -p /out \
    && cp target/release/namu-master target/release/namu-worker target/release/namu /out/

FROM ${BASE_IMAGE} AS base
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home --home-dir /var/lib/namu namu
WORKDIR /var/lib/namu
USER 10001

FROM base AS orchestrator
COPY --from=build /out/namu-master /usr/local/bin/namu-master
ENV BIND_ADDR=0.0.0.0:8080 \
    NAMU_MESH_ADDR=0.0.0.0:7100 \
    ARTIFACTS_DIR=/var/lib/namu/artifacts
EXPOSE 8080 7100
ENTRYPOINT ["namu-master"]

FROM base AS worker
USER root
RUN apt-get update \
    && apt-get install -y --no-install-recommends python3 \
    && rm -rf /var/lib/apt/lists/*
USER 10001
COPY --from=build /out/namu-worker /usr/local/bin/namu-worker
ENV NAMU_MESH_ADDR=0.0.0.0:7100 \
    NAMU_RUNTIMES=wasm,native,python \
    ARTIFACT_CACHE=/var/lib/namu/cache
EXPOSE 7100
ENTRYPOINT ["namu-worker"]

FROM base AS cli
COPY --from=build /out/namu /usr/local/bin/namu
ENTRYPOINT ["namu"]
