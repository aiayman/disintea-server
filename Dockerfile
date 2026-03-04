# ── Stage 1: chef installer ─────────────────────────────────────────────────
FROM rust:1.82-slim-bookworm AS chef
WORKDIR /app
RUN apt-get update -qq && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --locked

# ── Stage 2: planner — generate recipe from manifests ───────────────────────
FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo chef prepare --recipe-path recipe.json

# ── Stage 3: cacher — build all dependencies (cached unless manifests change) 
FROM chef AS cacher
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# ── Stage 4: builder — compile only our code ────────────────────────────────
FROM chef AS builder
COPY --from=cacher /app/target target
COPY --from=cacher /usr/local/cargo /usr/local/cargo
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --bin disintea-server

# ── Stage 5: minimal runtime ────────────────────────────────────────────────
# debian:12-slim — smaller than full debian, supports healthcheck via wget,
# while docker-compose enforces no-new-privileges + cap_drop + read_only.
FROM debian:12-slim

LABEL org.opencontainers.image.title="disintea-server" \
      org.opencontainers.image.description="Disintea WebSocket signaling server"

# Only runtime dependencies: libssl + ca-certs + wget (for health check)
RUN apt-get update -qq \
    && apt-get install -y --no-install-recommends libssl3 ca-certificates wget \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -r disintea \
    && useradd -r -g disintea -s /sbin/nologin disintea

COPY --from=builder --chown=disintea:disintea \
     /app/target/release/disintea-server /usr/local/bin/disintea-server

USER disintea
EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/disintea-server"]


# ── Stage 3: distroless runtime ─────────────────────────────────────────────
# No shell, no package manager, runs as nonroot (UID 65532)
FROM gcr.io/distroless/cc-debian12:nonroot

LABEL org.opencontainers.image.title="disintea-server" \
      org.opencontainers.image.description="Disintea WebSocket signaling server" \
      security.no-shell="true" \
      security.distroless="true"

COPY --from=builder --chown=nonroot:nonroot \
     /app/target/release/disintea-server /usr/local/bin/disintea-server

USER nonroot
EXPOSE 8080

# Health check is defined in docker-compose (distroless has no shell/curl).

ENTRYPOINT ["/usr/local/bin/disintea-server"]
