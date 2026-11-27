-- Login Google (flusso lato server con PKCE) e sessioni.

-- Un tentativo di login in corso: vive dal redirect verso Google al callback.
CREATE TABLE oauth_flows (
    state          TEXT    PRIMARY KEY,
    nonce          TEXT    NOT NULL,
    pkce_verifier  TEXT    NOT NULL,
    created_at     INTEGER NOT NULL
) STRICT;

-- Il cookie contiene il token; qui c'è solo il suo SHA-256.
CREATE TABLE sessions (
    token_hash  TEXT    PRIMARY KEY,
    user_id     TEXT    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL
) STRICT;

CREATE INDEX sessions_user_id ON sessions (user_id);
