-- 0001_currency_registry.down.sql
-- Reverses 0001_currency_registry.up.sql. Dropping the schema is an
-- operator action on an empty/decommissioned registry only; the
-- never-recycle rule binds allocation while the registry lives, and a
-- down migration must never be used to re-issue ledger ids.

BEGIN;

DROP TABLE IF EXISTS account;
DROP TABLE IF EXISTS currency_issuer;
DROP TABLE IF EXISTS currency;
DROP TRIGGER IF EXISTS ledger_allocation_no_delete ON ledger_allocation;
DROP FUNCTION IF EXISTS ledger_allocation_no_delete();
DROP FUNCTION IF EXISTS allocate_ledger(text, text);
DROP TABLE IF EXISTS ledger_allocation;
DROP SEQUENCE IF EXISTS ledger_currency_seq;
DROP SEQUENCE IF EXISTS ledger_system_seq;

COMMIT;
