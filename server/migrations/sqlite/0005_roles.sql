-- Ruoli e accesso su invito.
-- L'utente master (ADMIN_EMAIL) diventa admin al primo accesso; gli altri
-- entrano solo con un invito, che si consuma al primo accesso con Google.

ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'user' CHECK (role IN ('admin', 'user'));

CREATE TABLE user_invites (
    email       TEXT    PRIMARY KEY,
    role        TEXT    NOT NULL CHECK (role IN ('admin', 'user')),
    invited_by  TEXT    REFERENCES users (id) ON DELETE SET NULL,
    created_at  INTEGER NOT NULL
) STRICT;
