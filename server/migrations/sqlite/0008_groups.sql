-- Groups of users, with group roles, and the invitations into a group.

CREATE TABLE groups (
    id          TEXT    PRIMARY KEY,
    name        TEXT    NOT NULL,
    -- Lowercase name, computed in Rust: group names are unique ignoring case.
    name_key    TEXT    NOT NULL UNIQUE,
    created_by  TEXT    REFERENCES users (id) ON DELETE SET NULL,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
) STRICT;

CREATE TABLE group_members (
    group_id    TEXT    NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    user_id     TEXT    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role        TEXT    NOT NULL CHECK (role IN ('admin', 'member')),
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (group_id, user_id)
) STRICT;

CREATE INDEX group_members_user_id ON group_members (user_id);

-- Pending invitations into a group, only for existing accounts. Only the invitee sees them.
CREATE TABLE group_invites (
    group_id    TEXT    NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    user_id     TEXT    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role        TEXT    NOT NULL CHECK (role IN ('admin', 'member')),
    invited_by  TEXT    REFERENCES users (id) ON DELETE SET NULL,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL,
    PRIMARY KEY (group_id, user_id)
) STRICT;

CREATE INDEX group_invites_user_id ON group_invites (user_id);

-- Groups attached to an instance invite (made by an instance admin who is also
-- admin of the group): the membership is added at the first sign-in.
CREATE TABLE user_invite_groups (
    email       TEXT    NOT NULL REFERENCES user_invites (email) ON DELETE CASCADE,
    group_id    TEXT    NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    role        TEXT    NOT NULL CHECK (role IN ('admin', 'member')),
    invited_by  TEXT    REFERENCES users (id) ON DELETE SET NULL,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (email, group_id)
) STRICT;
