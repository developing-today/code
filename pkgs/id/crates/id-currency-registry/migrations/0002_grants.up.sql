-- Migration 0002: the grant table (design
-- 2026-10-29T12-10-00Z_design_refundable_grants_pending_transfers,
-- section 2.3; slice G1 of the grant service, issue #2294).
--
-- The control-plane record TigerBeetle cannot hold: which Steam
-- key (by hash — the key itself is never stored) a pending
-- transfer's value came from, and whether it is still revocable.
-- Ids and transfer ids are TigerBeetle u128s, stored as decimal
-- text like migration 0001's account_id; amounts are u128,
-- stored as numeric(39,0). Times are unix seconds (bigint),
-- matching the in-memory store's record exactly.

CREATE TABLE grant_record (
    id                      text PRIMARY KEY,
    steam_key_hash          bytea NOT NULL CHECK (octet_length(steam_key_hash) = 32),
    user_id                 text NOT NULL,
    ledger                  bigint NOT NULL CHECK (ledger BETWEEN 1 AND 4294967295),
    amount                  numeric(39, 0) NOT NULL CHECK (amount > 0),
    state                   text NOT NULL DEFAULT 'pending'
                            CHECK (state IN ('pending', 'posted', 'voided', 'expired',
                                             'clawback_pending', 'clawback_settled', 'debt')),
    pending_transfer_id     text NOT NULL UNIQUE,
    resolution_transfer_id  text UNIQUE,
    window_started_at       bigint NOT NULL,
    window_ends_at          bigint NOT NULL CHECK (window_ends_at >= window_started_at),
    created_at              bigint NOT NULL,
    updated_at              bigint NOT NULL,
    attempt_count           integer NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    last_error              text,
    correction_of           text REFERENCES grant_record (id)
);

-- A key redeems once, ever: at most one live (non-voided) grant
-- per steam_key_hash. A voided grant's hash may recur only on its
-- own correction chain (enforced by the store seam; this index is
-- the schema half of the rule).
CREATE UNIQUE INDEX grant_record_live_key_hash
    ON grant_record (steam_key_hash)
    WHERE state <> 'voided';

-- The GET /users/{id}/grants read.
CREATE INDEX grant_record_user ON grant_record (user_id, id);

-- The reconciler's sweep (design section 6.3): pending grants
-- whose window has closed.
CREATE INDEX grant_record_pending_window
    ON grant_record (window_ends_at)
    WHERE state = 'pending';
