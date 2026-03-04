# ── Stage 1: builder ────────────────────────────────────────────────────────
FROM rust:slim-bookworm AS builder
WORKDIR /app
RUN apt-get update -qq \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Cache dependencies separately (only re-runs when Cargo.toml/Cargo.lock change)
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main(){}' > src/main.rs \
    && cargo build --release 2>/dev/null || true \
    && rm -rf src

# Build the real source
COPY src ./src
RUN touch src/main.rs && cargo build --release --bin disintea-server

# ── Stage 2: minimal runtime ────────────────────────────────────────────────
FROM debian:12-slim

LABEL org.opencontainers.image.title="disintea-server" \
      org.opencontainers.image.description="Disintea WebSocket signaling server"

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
