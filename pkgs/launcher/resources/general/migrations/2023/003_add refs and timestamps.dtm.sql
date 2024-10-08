-- +goose Up
ALTER TABLE nodes
ADD COLUMN IF NOT EXISTS universal TEXT,
ADD COLUMN IF NOT EXISTS graph_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS class_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS match_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS merge_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS name_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS kind_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS sort_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS tier_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS mode_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS code_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS hash_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS item_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS part_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS slot_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS lead_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS peer_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS link_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS root_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS twig_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS leaf_id UUID REFERENCES nodes(id),
ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ DEFAULT now(),
ADD COLUMN IF NOT EXISTS created_by JSON,
ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ DEFAULT now(),
ADD COLUMN IF NOT EXISTS updated_by JSON,
ADD COLUMN IF NOT EXISTS expired_in_seconds INTEGER,
ADD COLUMN IF NOT EXISTS expired_at TIMESTAMPTZ,
ADD COLUMN IF NOT EXISTS expired_by JSON;


-- +goose Down
ALTER TABLE nodes
DROP COLUMN IF EXISTS universal,
DROP COLUMN IF EXISTS graph_id,
DROP COLUMN IF EXISTS class_id,
DROP COLUMN IF EXISTS match_id,
DROP COLUMN IF EXISTS merge_id,
DROP COLUMN IF EXISTS name_id,
DROP COLUMN IF EXISTS kind_id,
DROP COLUMN IF EXISTS sort_id,
DROP COLUMN IF EXISTS tier_id,
DROP COLUMN IF EXISTS mode_id,
DROP COLUMN IF EXISTS code_id,
DROP COLUMN IF EXISTS hash_id,
DROP COLUMN IF EXISTS item_id,
DROP COLUMN IF EXISTS part_id,
DROP COLUMN IF EXISTS slot_id,
DROP COLUMN IF EXISTS lead_id,
DROP COLUMN IF EXISTS peer_id,
DROP COLUMN IF EXISTS link_id,
DROP COLUMN IF EXISTS root_id,
DROP COLUMN IF EXISTS twig_id,
DROP COLUMN IF EXISTS leaf_id,
DROP COLUMN IF EXISTS created_at,
DROP COLUMN IF EXISTS created_by,
DROP COLUMN IF EXISTS updated_at,
DROP COLUMN IF EXISTS updated_by,
DROP COLUMN IF EXISTS expired_at,
DROP COLUMN IF EXISTS expired_in_seconds,
DROP COLUMN IF EXISTS expired_by;
