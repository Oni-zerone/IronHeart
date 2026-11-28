-- Requests to hand an app over to another user, accepted or rejected by them.

CREATE TABLE app_transfers (
    id             TEXT    PRIMARY KEY,
    app_id         TEXT    NOT NULL REFERENCES apps (id) ON DELETE CASCADE,
    -- Owner when the request was made: accepting fails if it changed meanwhile.
    from_user_id   TEXT,
    from_group_id  TEXT,
    requested_by   TEXT    REFERENCES users (id) ON DELETE SET NULL,
    -- Lowercase; stored even when no account has it, so the answer does not tell.
    to_email       TEXT    NOT NULL,
    state          TEXT    NOT NULL
                   CHECK (state IN ('pending', 'accepted', 'rejected', 'cancelled', 'expired')),
    created_at     INTEGER NOT NULL,
    expires_at     INTEGER NOT NULL,
    closed_at      INTEGER
) STRICT;

-- At most one pending request per app.
CREATE UNIQUE INDEX app_transfers_pending ON app_transfers (app_id) WHERE state = 'pending';
CREATE INDEX app_transfers_to_email ON app_transfers (to_email) WHERE state = 'pending';
CREATE INDEX app_transfers_requested_by ON app_transfers (requested_by, created_at);
