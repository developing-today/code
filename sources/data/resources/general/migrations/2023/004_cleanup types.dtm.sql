-- +goose Up
ALTER TABLE nodes
DROP COLUMN IF EXISTS expired_in_seconds,
ADD COLUMN IF NOT EXISTS expired_in_interval INTERVAL,
ADD COLUMN IF NOT EXISTS location GEOMETRY,
ADD COLUMN IF NOT EXISTS locations GEOMETRY,
ADD COLUMN IF NOT EXISTS shape GEOMETRY,
ADD COLUMN IF NOT EXISTS map GEOMETRY;
-- +goose Down
ALTER TABLE nodes
DROP COLUMN IF EXISTS expired_in_interval,
ADD COLUMN IF NOT EXISTS expired_in_seconds INT,
DROP COLUMN IF EXISTS location,
DROP COLUMN IF EXISTS locations,
DROP COLUMN IF EXISTS shape,
DROP COLUMN IF EXISTS map;
