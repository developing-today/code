-- Retirement marks for retained Atlas objects (#9140 design §5.3, build
-- slice 4). Applied after 0003_retained_objects.sql. A version its class
-- rule lets go is marked retired (the cold step) rather than deleted:
-- reports run before any delete, and deletion is not built yet.
ALTER TABLE atlas_retained_objects ADD COLUMN retired_at_ms INTEGER;
CREATE INDEX atlas_retained_objects_live ON atlas_retained_objects (object_id, retired_at_ms);
