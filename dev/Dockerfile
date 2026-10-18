# Orchestrator + worker image for the distribution proof of concept (Linux).
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p namu-master -p namu-worker \
    && cp target/release/namu-master target/release/namu-worker /usr/local/bin/

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /usr/local/bin/namu-master /usr/local/bin/namu-worker /usr/local/bin/
