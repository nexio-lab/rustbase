# syntax=docker/dockerfile:1.7

# ----- build stage -----------------------------------------------------
FROM rust:1.85-slim-bookworm AS builder
WORKDIR /app

# Bun (for the embedded dashboard build that build.rs triggers).
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl unzip \
 && curl -fsSL https://bun.sh/install | bash \
 && ln -s /root/.bun/bin/bun /usr/local/bin/bun \
 && apt-get clean && rm -rf /var/lib/apt/lists/*

# Cache deps before copying source.
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY ui/package.json ui/bun.lock ui/
COPY ui/ ./ui/

# Build dashboard first so the include_dir! find it at compile time.
RUN cd ui && bun install --frozen-lockfile && bun run build

# Then the binary.
RUN cargo build --release --bin rustbase \
 && strip target/release/rustbase

# ----- runtime stage ---------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tini \
 && apt-get clean && rm -rf /var/lib/apt/lists/* \
 && useradd --create-home --uid 10001 rustbase

COPY --from=builder /app/target/release/rustbase /usr/local/bin/rustbase

USER rustbase
WORKDIR /home/rustbase
VOLUME ["/home/rustbase/data"]
EXPOSE 8080

# `tini` reaps zombies + forwards signals so SIGTERM cleanly stops the server.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/rustbase"]
