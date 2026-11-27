# syntax=docker/dockerfile:1

# 1. Dashboard Vue: file statici in web/dist (controllo dei tipi compreso).
FROM node:24-trixie-slim AS web
RUN npm install -g pnpm@12.23.0
WORKDIR /src/web
COPY web/package.json web/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm build

# 2. Binario Rust, con la dashboard incorporata (rust-embed legge ../web/dist).
FROM rust:1-trixie AS server
WORKDIR /src/server
COPY server/ ./
COPY --from=web /src/web/dist /src/web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo build --release --locked \
    && cp target/release/ironheart /usr/local/bin/ironheart

# 3. Immagine finale: solo il binario.
FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /nonexistent --shell /usr/sbin/nologin ironheart \
    && mkdir /data && chown ironheart:ironheart /data
COPY --from=server /usr/local/bin/ironheart /usr/local/bin/ironheart

# Senza DATABASE_URL il database è SQLite in /data/ironheart.db (percorso relativo
# data/ironheart.db dalla cartella di lavoro /): va montato un volume su /data.
WORKDIR /
ENV BIND_ADDR=0.0.0.0:8080 \
    RUST_LOG=info
VOLUME ["/data"]
EXPOSE 8080
USER ironheart
ENTRYPOINT ["/usr/local/bin/ironheart"]
