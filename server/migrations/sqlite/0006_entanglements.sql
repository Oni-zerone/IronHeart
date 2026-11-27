-- Entanglement: questa istanza manda heartbeat ad altre istanze di IronHeart,
-- così più istanze si controllano a vicenda. Configurazione dell'istanza,
-- gestita solo dall'utente master.

CREATE TABLE entanglements (
    id                  TEXT    PRIMARY KEY,
    name                TEXT    NOT NULL,
    -- Origine dell'altra istanza (schema, host, porta), senza `/` finale.
    base_url            TEXT    NOT NULL,
    key_id              TEXT    NOT NULL,
    -- Segreto dell'app sull'altra istanza, cifrato come quelli delle app.
    secret_enc          BLOB    NOT NULL,
    secret_key_version  INTEGER NOT NULL,
    interval_secs       INTEGER NOT NULL CHECK (interval_secs BETWEEN 10 AND 86400),
    enabled             INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    next_ping_at        INTEGER,
    last_attempt_at     INTEGER,
    -- Ultimo 204 ricevuto: la richiesta è arrivata (la validità la vede solo l'altra istanza).
    last_success_at     INTEGER,
    last_error          TEXT,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
) STRICT;

CREATE INDEX entanglements_due ON entanglements (next_ping_at) WHERE enabled = 1;
