-- no-transaction
-- Apps owned by a user or by a group: user_id becomes nullable, group_id is
-- new, and exactly one of the two is set.
--
-- SQLite cannot drop NOT NULL or add a CHECK to an existing table, so the table
-- is rebuilt. app_recipients and failure_buckets reference apps with ON DELETE
-- CASCADE: dropping the old table with foreign keys on would delete them. The
-- foreign keys can be turned off only outside a transaction, hence
-- `no-transaction` and the explicit BEGIN/COMMIT around the rebuild.

PRAGMA foreign_keys = OFF;

BEGIN;

CREATE TABLE apps_new (
    id                   TEXT    PRIMARY KEY,
    -- Owner: a user or a group, never both.
    user_id              TEXT    REFERENCES users (id) ON DELETE CASCADE,
    -- A group that owns apps cannot be deleted: they are moved first.
    group_id             TEXT    REFERENCES groups (id) ON DELETE RESTRICT,
    name                 TEXT    NOT NULL,
    key_id               TEXT    NOT NULL UNIQUE,
    secret_enc           BLOB    NOT NULL,
    secret_key_version   INTEGER NOT NULL,
    interval_secs        INTEGER NOT NULL CHECK (interval_secs > 0),
    grace_secs           INTEGER NOT NULL DEFAULT 0 CHECK (grace_secs >= 0),
    state                TEXT    NOT NULL DEFAULT 'new'
                         CHECK (state IN ('new', 'up', 'down', 'paused')),
    last_seen_at         INTEGER,
    last_signed_ts       INTEGER,
    next_deadline_at     INTEGER,
    down_since           INTEGER,
    recovery_pending     INTEGER NOT NULL DEFAULT 0 CHECK (recovery_pending IN (0, 1)),
    fail_level           INTEGER NOT NULL DEFAULT 0 CHECK (fail_level BETWEEN 0 AND 5),
    fail_window_end      INTEGER,
    fail_last_report_at  INTEGER,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    CHECK ((user_id IS NULL) <> (group_id IS NULL))
) STRICT;

INSERT INTO apps_new (id, user_id, name, key_id, secret_enc, secret_key_version,
                      interval_secs, grace_secs, state, last_seen_at, last_signed_ts,
                      next_deadline_at, down_since, recovery_pending, fail_level,
                      fail_window_end, fail_last_report_at, created_at, updated_at)
SELECT id, user_id, name, key_id, secret_enc, secret_key_version,
       interval_secs, grace_secs, state, last_seen_at, last_signed_ts,
       next_deadline_at, down_since, recovery_pending, fail_level,
       fail_window_end, fail_last_report_at, created_at, updated_at
FROM apps;

DROP TABLE apps;
ALTER TABLE apps_new RENAME TO apps;

CREATE INDEX apps_user_id ON apps (user_id);
CREATE INDEX apps_group_id ON apps (group_id);
CREATE INDEX apps_due ON apps (next_deadline_at) WHERE state = 'up';

COMMIT;

PRAGMA foreign_keys = ON;
