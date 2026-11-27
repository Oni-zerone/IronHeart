-- Schema completo per Postgres, equivalente alle migrazioni 0001–0006 di SQLite.
-- Il numero è 0006 così le migrazioni successive hanno lo stesso numero nelle
-- due cartelle (migrations/sqlite e migrations/postgres).
--
-- Stesse regole di SQLite (vedi CLAUDE.md): tempi BIGINT in millisecondi Unix UTC,
-- ID TEXT (UUID v7 generato in Rust), booleani BIGINT 0/1, enumerazioni TEXT con CHECK.
-- Tutti gli interi sono BIGINT: il driver `Any` di sqlx li legge come i64.

CREATE TABLE users (
    id          TEXT   PRIMARY KEY,
    google_sub  TEXT   NOT NULL UNIQUE,
    email       TEXT   NOT NULL,
    name        TEXT,
    created_at  BIGINT NOT NULL,
    role        TEXT   NOT NULL DEFAULT 'user' CHECK (role IN ('admin', 'user'))
);

CREATE TABLE user_invites (
    email       TEXT   PRIMARY KEY,
    role        TEXT   NOT NULL CHECK (role IN ('admin', 'user')),
    invited_by  TEXT   REFERENCES users (id) ON DELETE SET NULL,
    created_at  BIGINT NOT NULL
);

-- Un tentativo di login in corso: vive dal redirect verso Google al callback.
CREATE TABLE oauth_flows (
    state          TEXT   PRIMARY KEY,
    nonce          TEXT   NOT NULL,
    pkce_verifier  TEXT   NOT NULL,
    created_at     BIGINT NOT NULL
);

-- Il cookie contiene il token; qui c'è solo il suo SHA-256.
CREATE TABLE sessions (
    token_hash  TEXT   PRIMARY KEY,
    user_id     TEXT   NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at  BIGINT NOT NULL,
    expires_at  BIGINT NOT NULL
);

CREATE INDEX sessions_user_id ON sessions (user_id);

CREATE TABLE apps (
    id                   TEXT   PRIMARY KEY,
    user_id              TEXT   NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name                 TEXT   NOT NULL,
    -- Identificativo pubblico della chiave, in chiaro.
    key_id               TEXT   NOT NULL UNIQUE,
    -- Segreto HMAC cifrato con AES-256-GCM: nonce (12 byte) + testo cifrato.
    secret_enc           BYTEA  NOT NULL,
    secret_key_version   BIGINT NOT NULL,
    interval_secs        BIGINT NOT NULL CHECK (interval_secs > 0),
    grace_secs           BIGINT NOT NULL DEFAULT 0 CHECK (grace_secs >= 0),
    state                TEXT   NOT NULL DEFAULT 'new'
                         CHECK (state IN ('new', 'up', 'down', 'paused')),
    last_seen_at         BIGINT,
    -- Ultimo timestamp firmato accettato (secondi): deve crescere a ogni ping.
    last_signed_ts       BIGINT,
    next_deadline_at     BIGINT,
    down_since           BIGINT,
    -- 1 quando l'app torna attiva dopo essere stata down: la mail la manda il controllo.
    recovery_pending     BIGINT NOT NULL DEFAULT 0 CHECK (recovery_pending IN (0, 1)),
    -- Avvisi sui tentativi falliti: livello 0–5 (5 → 120 minuti) e finestra aperta.
    fail_level           BIGINT NOT NULL DEFAULT 0 CHECK (fail_level BETWEEN 0 AND 5),
    fail_window_end      BIGINT,
    fail_last_report_at  BIGINT,
    created_at           BIGINT NOT NULL,
    updated_at           BIGINT NOT NULL
);

CREATE INDEX apps_user_id ON apps (user_id);
CREATE INDEX apps_due ON apps (next_deadline_at) WHERE state = 'up';

-- Destinatari opzionali degli avvisi di un'app (email in minuscolo).
CREATE TABLE app_recipients (
    app_id      TEXT   NOT NULL REFERENCES apps (id) ON DELETE CASCADE,
    email       TEXT   NOT NULL,
    created_at  BIGINT NOT NULL,
    PRIMARY KEY (app_id, email)
);

-- Tentativi falliti, sommati in memoria e scritti a ogni giro del controllo.
CREATE TABLE failure_buckets (
    id             TEXT   PRIMARY KEY,
    app_id         TEXT   NOT NULL REFERENCES apps (id) ON DELETE CASCADE,
    period_start   BIGINT NOT NULL,
    flushed_at     BIGINT NOT NULL,
    bad_signature  BIGINT NOT NULL,
    replay         BIGINT NOT NULL,
    clock_skew     BIGINT NOT NULL,
    -- JSON {"ip": conteggio}, al massimo 20 IP; gli altri finiscono in other_ips.
    ips            TEXT   NOT NULL,
    other_ips      BIGINT NOT NULL
);

CREATE INDEX failure_buckets_app ON failure_buckets (app_id, flushed_at);
CREATE INDEX failure_buckets_flushed ON failure_buckets (flushed_at);

-- Entanglement: heartbeat verso altre istanze di IronHeart (solo master).
CREATE TABLE entanglements (
    id                  TEXT   PRIMARY KEY,
    name                TEXT   NOT NULL,
    base_url            TEXT   NOT NULL,
    key_id              TEXT   NOT NULL,
    secret_enc          BYTEA  NOT NULL,
    secret_key_version  BIGINT NOT NULL,
    interval_secs       BIGINT NOT NULL CHECK (interval_secs BETWEEN 10 AND 86400),
    enabled             BIGINT NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    next_ping_at        BIGINT,
    last_attempt_at     BIGINT,
    last_success_at     BIGINT,
    last_error          TEXT,
    created_at          BIGINT NOT NULL,
    updated_at          BIGINT NOT NULL
);

CREATE INDEX entanglements_due ON entanglements (next_ping_at) WHERE enabled = 1;
