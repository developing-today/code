-- +goose Up
ALTER TABLE nodes
    DROP CONSTRAINT IF EXISTS nodes_team_id_fkey, -- Drop existing foreign key constraint
    ADD COLUMN IF NOT EXISTS team_id UUID,       -- Add the column if it doesn't exist
    ADD CONSTRAINT nodes_team_id_fkey FOREIGN KEY (team_id) REFERENCES teams(id); -- Add foreign key constraint

-- +goose Down
ALTER TABLE nodes
    DROP CONSTRAINT IF EXISTS nodes_team_id_fkey; -- Drop the foreign key constraint
