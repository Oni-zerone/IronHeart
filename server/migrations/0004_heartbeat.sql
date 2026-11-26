-- App con chiave firmata (HMAC), stato del controllo e tentativi falliti.
-- La tabella apps non è mai stata usata: la ricreo invece di modificarla,
-- perché SQLite non può togliere una colonna UNIQUE.

DROP INDEX apps_due;
DROP INDEX apps_user_id;

CREATE TABLE apps_new (
    id                   TEXT    PRIMARY KEY,
    user_id              TEXT    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name                 TEXT    NOT NULL,
    -- Identificativo pubblico della chiave, in chiaro.
    key_id               TEXT    NOT NULL UNIQUE,
    -- Segreto HMAC cifrato con AES-256-GCM: nonce (12 byte) + testo cifrato.
    secret_enc           BLOB    NOT NULL,
    secret_key_version   INTEGER NOT NULL,
    interval_secs        INTEGER NOT NULL CHECK (interval_secs > 0),
    grace_secs           INTEGER NOT NULL DEFAULT 0 CHECK (grace_secs >= 0),
    state                TEXT    NOT NULL DEFAULT 'new'
                         CHECK (state IN ('new', 'up', 'down', 'paused')),
    last_seen_at         INTEGER,
    -- Ultimo timestamp firmato accettato (secondi): deve crescere a ogni ping.
    last_signed_ts       INTEGER,
    next_deadline_at     INTEGER,
    down_since           INTEGER,
    -- 1 quando l'app torna attiva dopo essere stata down: la mail la manda il controllo.
    recovery_pending     INTEGER NOT NULL DEFAULT 0 CHECK (recovery_pending IN (0, 1)),
    -- Avvisi sui tentativi falliti: livello 0–5 (5 → 120 minuti) e finestra aperta.
    fail_level           INTEGER NOT NULL DEFAULT 0 CHECK (fail_level BETWEEN 0 AND 5),
    fail_window_end      INTEGER,
    fail_last_report_at  INTEGER,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL
) STRICT;

DROP TABLE apps;
ALTER TABLE apps_new RENAME TO apps;

CREATE INDEX apps_user_id ON apps (user_id);
CREATE INDEX apps_due ON apps (next_deadline_at) WHERE state = 'up';

-- Tentativi falliti, sommati in memoria e scritti a ogni giro del controllo.
CREATE TABLE failure_buckets (
    id             TEXT    PRIMARY KEY,
    app_id         TEXT    NOT NULL REFERENCES apps (id) ON DELETE CASCADE,
    period_start   INTEGER NOT NULL,
    flushed_at     INTEGER NOT NULL,
    bad_signature  INTEGER NOT NULL,
    replay         INTEGER NOT NULL,
    clock_skew     INTEGER NOT NULL,
    -- JSON {"ip": conteggio}, al massimo 20 IP; gli altri finiscono in other_ips.
    ips            TEXT    NOT NULL,
    other_ips      INTEGER NOT NULL
) STRICT;

CREATE INDEX failure_buckets_app ON failure_buckets (app_id, flushed_at);
CREATE INDEX failure_buckets_flushed ON failure_buckets (flushed_at);
