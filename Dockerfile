# syntax=docker/dockerfile:1

# 1. Dashboard Vue: file statici in web/dist (controllo dei tipi compreso).
FROM node:24-trixie-slim AS web
RUN npm install -g pnpm@12.23.0
WORKDIR /src/web
COPY web/package.json web/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm build

# 2. Binario Rust statico (musl), con la dashboard incorporata (rust-embed legge ../web/dist).
#    Non servono librerie né certificati di sistema: TLS è rustls con i certificati
#    radice compilati dentro (webpki-roots).
FROM rust:1-alpine AS server
RUN apk add --no-cache musl-dev
WORKDIR /src/server
COPY server/ ./
COPY --from=web /src/web/dist /src/web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo build --release --locked \
    && cp target/release/ironheart /usr/local/bin/ironheart \
    && mkdir -p /rootfs/data

# 3. Immagine finale vuota: solo il binario e la cartella dei dati.
FROM scratch
COPY --from=server /usr/local/bin/ironheart /ironheart
COPY --from=server --chown=10001:10001 /rootfs/data /data

# Senza DATABASE_URL il database è SQLite in /data/ironheart.db (percorso relativo
# data/ironheart.db dalla cartella di lavoro /): va montato un volume su /data.
WORKDIR /
# mimalloc: memoria impegnata solo quando serve e restituita subito
# (senza, ~16 MiB a riposo invece di ~3, a parità di prestazioni).
ENV BIND_ADDR=0.0.0.0:8080 \
    RUST_LOG=info \
    MIMALLOC_ARENA_EAGER_COMMIT=0 \
    MIMALLOC_PURGE_DELAY=0
VOLUME ["/data"]
EXPOSE 8080
USER 10001:10001
ENTRYPOINT ["/ironheart"]
