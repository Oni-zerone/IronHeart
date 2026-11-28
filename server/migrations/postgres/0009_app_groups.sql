-- Apps owned by a user or by a group: user_id becomes nullable, group_id is
-- new, and exactly one of the two is set.

ALTER TABLE apps ALTER COLUMN user_id DROP NOT NULL;

-- A group that owns apps cannot be deleted: they are moved first.
ALTER TABLE apps ADD COLUMN group_id TEXT REFERENCES groups (id) ON DELETE RESTRICT;

ALTER TABLE apps ADD CONSTRAINT apps_one_owner CHECK ((user_id IS NULL) <> (group_id IS NULL));

CREATE INDEX apps_group_id ON apps (group_id);
