CREATE TABLE teams (
 id uuid PRIMARY KEY,
 owner_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 name text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 60),
 created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX teams_owner ON teams(owner_id, created_at);

CREATE TABLE team_memberships (
 id uuid PRIMARY KEY,
 team_id uuid NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
 user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 status text NOT NULL CHECK (status IN ('pending', 'active')),
 UNIQUE (team_id, user_id)
);
CREATE INDEX team_memberships_user ON team_memberships(user_id, team_id);

ALTER TABLE profile_shares ADD COLUMN team_id uuid REFERENCES teams(id) ON DELETE CASCADE;
ALTER TABLE profile_shares ADD CONSTRAINT team_shares_configuration_only
 CHECK (team_id IS NULL OR NOT includes_session);
CREATE UNIQUE INDEX team_shares_live_assignment ON profile_shares(team_id, recipient_id, source_profile_id)
 WHERE team_id IS NOT NULL AND status IN ('pending', 'accepted');
