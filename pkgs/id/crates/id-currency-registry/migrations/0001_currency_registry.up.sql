-- 0001_currency_registry.up.sql
--
-- Currency control-plane registry, slice F1 of the #2062 design
-- (doc/2026-10-29T12-42-21Z_design_currency_control_plane, sections 2 and 6),
-- filed as issue #2317.
--
-- Decisions applied (ledger currency-registry-decisions-2317, answered
-- 2026-11-16, recorded on issue #2317 by triage task 15189):
--   1. Registry store: Postgres (this migration).
--   2. Ledger u32 is stored as bigint with a 0..4294967295 check constraint;
--      the design section 6 range split is ratified (see the per-kind range
--      check below).
--   3. currency_id is globally unique; symbol is unique per issuer.
--   4. currency_issuer / account columns, which the design names but does
--      not tabulate, are drafted here from design sections 2.1-2.2, 2.4,
--      3 and 4.2 for ratification.
--
-- Ledger range split (design section 6, ratified):
--   0x00000001-0x000FFFFF  tenancy / system
--   0x00100000-0xFFFFFFFF  currencies
-- Ledger 0 is invalid in TigerBeetle and is never allocated: both allocator
-- sequences start at or above 1 and the per-kind range check excludes 0
-- from every allocation row.

BEGIN;

-- ---------------------------------------------------------------------------
-- ledger_allocation: the u32 allocator's ledger (design section 2.1)
-- ---------------------------------------------------------------------------
CREATE TABLE ledger_allocation (
    ledger_id   bigint PRIMARY KEY
                CHECK (ledger_id BETWEEN 0 AND 4294967295),
    kind        text NOT NULL
                CHECK (kind IN ('currency', 'system', 'tenancy')),
    owner_ref   text NOT NULL,
    allocated_at timestamptz NOT NULL DEFAULT now(),
    -- Ratified range split: a row may only exist inside its kind's range.
    CHECK (
        (kind = 'currency' AND ledger_id >= 1048576)
        OR (kind IN ('system', 'tenancy')
            AND ledger_id BETWEEN 1 AND 1048575)
    )
);

COMMENT ON TABLE ledger_allocation IS
    'The only place a TigerBeetle ledger u32 is assigned (design #2062 s.2.1). Rows are never deleted and ids are never recycled (s.6).';

-- Monotonic allocator sequences. Sequences never go backwards and are
-- never reset by this schema, which is what makes allocation
-- never-recycled at the storage layer.
CREATE SEQUENCE ledger_system_seq
    START WITH 1 INCREMENT BY 1 MINVALUE 1 MAXVALUE 1048575 NO CYCLE;
CREATE SEQUENCE ledger_currency_seq
    START WITH 1048576 INCREMENT BY 1 MINVALUE 1048576
    MAXVALUE 4294967295 NO CYCLE;

-- allocate_ledger(kind, owner_ref) -> the newly allocated ledger id.
-- Hands out non-zero ids monotonically inside the kind's ratified range
-- and records the allocation in the same statement's transaction.
CREATE FUNCTION allocate_ledger(p_kind text, p_owner_ref text)
RETURNS bigint
LANGUAGE plpgsql
AS $$
DECLARE
    v_id bigint;
BEGIN
    IF p_kind = 'currency' THEN
        v_id := nextval('ledger_currency_seq');
    ELSIF p_kind IN ('system', 'tenancy') THEN
        v_id := nextval('ledger_system_seq');
    ELSE
        RAISE EXCEPTION 'unknown ledger kind: %', p_kind
            USING ERRCODE = 'check_violation';
    END IF;
    INSERT INTO ledger_allocation (ledger_id, kind, owner_ref)
    VALUES (v_id, p_kind, p_owner_ref);
    RETURN v_id;
END;
$$;

-- Never recycled, enforced: allocation rows cannot be deleted at all.
CREATE FUNCTION ledger_allocation_no_delete()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'ledger_allocation rows are never deleted (design #2062 s.6: ledger ids are never recycled)'
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER ledger_allocation_no_delete
    BEFORE DELETE ON ledger_allocation
    FOR EACH ROW EXECUTE FUNCTION ledger_allocation_no_delete();

-- ---------------------------------------------------------------------------
-- currency: the currency record (design section 2.2)
-- ---------------------------------------------------------------------------
CREATE TABLE currency (
    ledger_id       bigint PRIMARY KEY
                    REFERENCES ledger_allocation (ledger_id),
    -- Semantic identity (design s.2.3): minted by the registry at
    -- registration, globally unique, never changes.
    currency_id     text NOT NULL UNIQUE,
    name            text NOT NULL,
    -- Display label: unique per issuer, not globally (design s.2.3), so
    -- two issuers may both register the symbol ABC.
    symbol          text NOT NULL,
    -- Decimal places. Registry-only; immutable once balances exist (s.0).
    scale           smallint NOT NULL CHECK (scale >= 0),
    issuer_id       text NOT NULL,
    supply_policy   jsonb NOT NULL
                    DEFAULT '{"mintable": false, "cap": null, "policy_uri": null}'::jsonb,
    status          text NOT NULL DEFAULT 'active'
                    CHECK (status IN ('active', 'frozen', 'sunset')),
    -- Linkage to the signed graph object that is the currency's canonical
    -- record (design s.2.4); this row is a derived index over it.
    graph_object_id text,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (issuer_id, symbol)
);

COMMENT ON TABLE currency IS
    'Currency metadata TigerBeetle does not hold (design #2062 s.2.2). currency_id globally unique; (issuer_id, symbol) unique.';

-- ---------------------------------------------------------------------------
-- currency_issuer: who may act for a currency (drafted, decision 4)
-- Drafted from design s.4.2: register_currency records the issuer here and
-- grant_minter writes rows with role = 'minter'. The registry, not the
-- signature, decides authority (s.4.2).
-- ---------------------------------------------------------------------------
CREATE TABLE currency_issuer (
    ledger_id   bigint NOT NULL REFERENCES currency (ledger_id),
    account_id  text NOT NULL,
    role        text NOT NULL CHECK (role IN ('issuer', 'minter')),
    granted_by  text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (ledger_id, account_id, role)
);

COMMENT ON TABLE currency_issuer IS
    'Issuer/minter grants per currency (drafted from design #2062 s.4.2 for ratification, decision 4 on issue #2317).';

-- ---------------------------------------------------------------------------
-- account: the registry index of factory accounts (drafted, decision 4)
-- Drafted from design s.3: account ids are derived, not allocated
-- (s.3.1), so this table is an index of the accounts the factory has
-- ensured, keyed by the derived 128-bit account id (hex text), with the
-- (ledger, owner, kind, salt) tuple that derivation is a function of.
-- ---------------------------------------------------------------------------
CREATE TABLE account (
    account_id  text PRIMARY KEY,
    ledger_id   bigint NOT NULL REFERENCES currency (ledger_id),
    owner_id    text NOT NULL,
    kind        text NOT NULL
                CHECK (kind IN ('issuer', 'liquidity', 'fee', 'burn', 'holder')),
    salt        text NOT NULL DEFAULT '',
    created_at  timestamptz NOT NULL DEFAULT now(),
    UNIQUE (ledger_id, owner_id, kind, salt)
);

COMMENT ON TABLE account IS
    'Index of deterministic factory accounts (drafted from design #2062 s.3 for ratification, decision 4 on issue #2317). Derivation itself is slice F2.';

COMMIT;
