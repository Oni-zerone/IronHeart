-- Telegram: chats linked to an account, and the one-time tokens that link them.

-- A chat belongs to one account; an account can have several chats.
CREATE TABLE telegram_links (
    id           TEXT    PRIMARY KEY,
    user_id      TEXT    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    chat_id      INTEGER NOT NULL UNIQUE,
    -- "@username" or first name, shown in the dashboard.
    label        TEXT    NOT NULL,
    created_at   INTEGER NOT NULL,
    -- Set when Telegram refuses delivery (bot blocked); cleared by the next message from the chat.
    disabled_at  INTEGER
) STRICT;

CREATE INDEX telegram_links_user_id ON telegram_links (user_id);

-- The deep link `t.me/<bot>?start=<token>` carries the token; here there is only its SHA-256.
CREATE TABLE telegram_link_tokens (
    token_hash  TEXT    PRIMARY KEY,
    user_id     TEXT    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    expires_at  INTEGER NOT NULL
) STRICT;

CREATE INDEX telegram_link_tokens_user_id ON telegram_link_tokens (user_id);
