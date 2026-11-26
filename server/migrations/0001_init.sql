-- Regole di portabilità verso Postgres: vedi CLAUDE.md.
-- Tempi: INTEGER, millisecondi Unix UTC. ID: TEXT, UUID v7 generato in Rust.

CREATE TABLE users (
    id          TEXT    PRIMARY KEY,
    google_sub  TEXT    NOT NULL UNIQUE,
    email       TEXT    NOT NULL,
    name        TEXT,
    created_at  INTEGER NOT NULL
) STRICT;

CREATE TABLE apps (
    id                TEXT    PRIMARY KEY,
    user_id           TEXT    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name              TEXT    NOT NULL,
    api_key_prefix    TEXT    NOT NULL,
    api_key_hash      TEXT    NOT NULL UNIQUE,
    interval_secs     INTEGER NOT NULL CHECK (interval_secs > 0),
    grace_secs        INTEGER NOT NULL DEFAULT 0 CHECK (grace_secs >= 0),
    state             TEXT    NOT NULL DEFAULT 'new'
                      CHECK (state IN ('new', 'up', 'down', 'paused')),
    last_seen_at      INTEGER,
    next_deadline_at  INTEGER,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
) STRICT;

CREATE INDEX apps_user_id ON apps (user_id);
CREATE INDEX apps_due ON apps (next_deadline_at) WHERE state = 'up';
