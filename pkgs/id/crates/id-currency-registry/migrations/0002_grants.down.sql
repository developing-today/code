-- Reverse of migration 0002 (grant table, issue #2294).

DROP INDEX IF EXISTS grant_record_pending_window;
DROP INDEX IF EXISTS grant_record_user;
DROP INDEX IF EXISTS grant_record_live_key_hash;
DROP TABLE IF EXISTS grant_record;
