-- Destinatari opzionali degli avvisi di un'app.
-- Se un'app non ne ha, gli avvisi vanno all'email dell'account proprietario.
-- Le email si salvano in minuscolo (normalizzate in Rust).

CREATE TABLE app_recipients (
    app_id      TEXT    NOT NULL REFERENCES apps (id) ON DELETE CASCADE,
    email       TEXT    NOT NULL,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (app_id, email)
) STRICT;
