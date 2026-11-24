//! Acceptance tests for the grant service (slice G1, issue
//! #2294), against the design
//! `doc/2026-10-29T12-10-00Z_design_refundable_grants_pending_transfers`.
//!
//! Every test names its oracle: the grant row's state (the
//! control-plane projection) and the stub ledger's balances (the
//! money truth — pending is held and unspendable, posted is
//! spendable). The brief's named cases are register-vest,
//! register-void, double-resolution 409, and partial refund
//! leaving no posted credit.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use id_currency_registry::grants::{
    steam_key_hash, Grant, GrantConfig, GrantError, GrantService, GrantState, GrantStore,
    GrantStoreError, InMemoryGrantStore, RegisterGrant, StubLedger,
};

const POOL: u128 = 1001;
const HOLDER: u128 = 2002;
const LEDGER: u32 = 7;
const WINDOW_SECONDS: u64 = 14 * 24 * 60 * 60;
const NOW: u64 = 1_000_000;

fn service() -> GrantService<InMemoryGrantStore, StubLedger> {
    GrantService::new(
        InMemoryGrantStore::new(),
        StubLedger::new(),
        GrantConfig::new(WINDOW_SECONDS),
    )
}

fn request(key: &str, user: &str, amount: u128) -> RegisterGrant {
    RegisterGrant {
        steam_key: key.to_string(),
        user_id: user.to_string(),
        ledger: LEDGER,
        amount,
        grant_pool_account_id: POOL,
        holder_account_id: HOLDER,
    }
}

#[test]
fn register_then_vest_posts_full_amount() {
    // Oracle: the stub ledger's balances + the grant row's state.
    let mut svc = service();
    let grant = svc
        .register(&request("key-vest", "user-1", 500), NOW)
        .expect("register");
    assert_eq!(grant.state, GrantState::Pending);
    assert_eq!(grant.window_ends_at, NOW + WINDOW_SECONDS);
    assert_eq!(svc.ledger().credits_pending(HOLDER), 500);
    assert_eq!(svc.ledger().credits_posted(HOLDER), 0);

    let vested = svc.vest(grant.id, NOW + 60).expect("vest");
    assert_eq!(vested.state, GrantState::Posted);
    assert!(vested.resolution_transfer_id.is_some());
    assert_eq!(svc.ledger().credits_pending(HOLDER), 0);
    assert_eq!(svc.ledger().credits_posted(HOLDER), 500);
}

#[test]
fn register_then_void_releases_hold_and_posts_nothing() {
    // Oracle: after a void, the holder has no posted credit and
    // no pending hold — the refund path of design section 2.1.
    let mut svc = service();
    let grant = svc
        .register(&request("key-void", "user-1", 500), NOW)
        .expect("register");
    let voided = svc.void(grant.id, NOW + 60).expect("void");
    assert_eq!(voided.state, GrantState::Voided);
    assert_eq!(svc.ledger().credits_pending(HOLDER), 0);
    assert_eq!(svc.ledger().credits_posted(HOLDER), 0);
}

#[test]
fn double_resolution_is_already_resolved() {
    // Oracle: the GrantError::AlreadyResolved verdict (the API's
    // 409) carrying the state the grant actually reached, for
    // every second resolution attempt.
    let mut svc = service();
    let voided = svc
        .register(&request("key-dr-void", "user-1", 10), NOW)
        .expect("register");
    svc.void(voided.id, NOW).expect("void");
    assert_eq!(
        svc.vest(voided.id, NOW),
        Err(GrantError::AlreadyResolved {
            grant_id: voided.id,
            state: GrantState::Voided,
        })
    );

    let posted = svc
        .register(&request("key-dr-vest", "user-1", 10), NOW)
        .expect("register");
    svc.vest(posted.id, NOW).expect("vest");
    assert_eq!(
        svc.void(posted.id, NOW),
        Err(GrantError::AlreadyResolved {
            grant_id: posted.id,
            state: GrantState::Posted,
        })
    );
    assert_eq!(
        svc.vest(posted.id, NOW),
        Err(GrantError::AlreadyResolved {
            grant_id: posted.id,
            state: GrantState::Posted,
        })
    );
}

#[test]
fn partial_refund_leaves_no_posted_credit() {
    // Oracle: design section 4 — void-and-reissue. After a
    // partial refund of a pending grant, posted credit is zero,
    // the original is voided, and the remainder rides a new
    // pending grant linked by correction_of, in the same window.
    let mut svc = service();
    let grant = svc
        .register(&request("key-partial", "user-1", 100), NOW)
        .expect("register");
    let (voided, reissue) = svc
        .refund_partial(grant.id, 30, POOL, HOLDER, NOW + 60)
        .expect("partial refund");
    assert_eq!(voided.state, GrantState::Voided);
    assert_eq!(reissue.state, GrantState::Pending);
    assert_eq!(reissue.amount, 70);
    assert_eq!(reissue.correction_of, Some(grant.id));
    assert_eq!(reissue.window_ends_at, grant.window_ends_at);
    assert_eq!(svc.ledger().credits_posted(HOLDER), 0);
    assert_eq!(svc.ledger().credits_pending(HOLDER), 70);

    // The reissue still vests normally at window close.
    let vested = svc.vest(reissue.id, NOW + 120).expect("vest reissue");
    assert_eq!(vested.state, GrantState::Posted);
    assert_eq!(svc.ledger().credits_posted(HOLDER), 70);
    assert_eq!(svc.ledger().credits_pending(HOLDER), 0);
}

#[test]
fn register_is_idempotent_on_steam_key() {
    // Oracle: one key, one grant, one hold — a repeat
    // registration returns the existing grant and the pending
    // balance is the grant amount once, not twice.
    let mut svc = service();
    let first = svc
        .register(&request("key-same", "user-1", 250), NOW)
        .expect("register");
    let second = svc
        .register(&request("key-same", "user-1", 250), NOW + 10)
        .expect("re-register");
    assert_eq!(first.id, second.id);
    assert_eq!(svc.ledger().credits_pending(HOLDER), 250);
    assert_eq!(svc.grants_for_user("user-1").len(), 1);

    let other = svc
        .register(&request("key-other", "user-1", 5), NOW)
        .expect("other key");
    assert_ne!(other.id, first.id);
    assert_eq!(svc.grants_for_user("user-1").len(), 2);
}

#[test]
fn refund_amount_must_be_a_proper_partial() {
    // Oracle: InvalidAmount for a zero refund and for a full
    // refund (a full refund is a plain void, not a reissue).
    let mut svc = service();
    let grant = svc
        .register(&request("key-bounds", "user-1", 100), NOW)
        .expect("register");
    assert_eq!(
        svc.refund_partial(grant.id, 0, POOL, HOLDER, NOW),
        Err(GrantError::InvalidAmount {
            attempted: 0,
            grant_amount: 100,
        })
    );
    assert_eq!(
        svc.refund_partial(grant.id, 100, POOL, HOLDER, NOW),
        Err(GrantError::InvalidAmount {
            attempted: 100,
            grant_amount: 100,
        })
    );
    // Neither attempt resolved or reissued anything.
    assert_eq!(svc.get(grant.id).expect("grant").state, GrantState::Pending);
    assert_eq!(svc.ledger().credits_pending(HOLDER), 100);
}

#[test]
fn expired_hold_syncs_row_and_reports_already_resolved() {
    // Oracle: when the ledger's timeout fires first (section
    // 6.3), a vest attempt syncs the row to the ledger's truth
    // (Expired) and answers AlreadyResolved — never a late post.
    let mut svc = service();
    let grant = svc
        .register(&request("key-expire", "user-1", 40), NOW)
        .expect("register");
    svc.ledger_mut().expire_pending(grant.pending_transfer_id);
    assert_eq!(
        svc.vest(grant.id, NOW + WINDOW_SECONDS + 1),
        Err(GrantError::AlreadyResolved {
            grant_id: grant.id,
            state: GrantState::Expired,
        })
    );
    assert_eq!(svc.get(grant.id).expect("grant").state, GrantState::Expired);
    assert_eq!(svc.ledger().credits_posted(HOLDER), 0);
    assert_eq!(svc.ledger().credits_pending(HOLDER), 0);
}

#[test]
fn unknown_grant_is_unknown() {
    let mut svc = service();
    assert_eq!(svc.vest(999, NOW), Err(GrantError::UnknownGrant(999)));
    assert_eq!(svc.void(999, NOW), Err(GrantError::UnknownGrant(999)));
}

#[test]
fn key_hash_is_sha256() {
    // Oracle: the published sha256 test vector for "test".
    let hash = steam_key_hash("test");
    let expected_prefix: [u8; 8] = [0x9f, 0x86, 0xd0, 0x81, 0x88, 0x4c, 0x7d, 0x65];
    assert_eq!(hash[..8], expected_prefix);
}

#[test]
fn policy_partial_vest_posts_only_the_vested_fraction() {
    // Oracle: design section 4's policy row — post the vested
    // fraction with `post_pending_transfer` amount = N' < N; the
    // unposted remainder is auto-restored (no longer pending,
    // never posted). The stub ledger's balances are the money
    // truth.
    let mut svc = service();
    let grant = svc
        .register(&request("key-partial-vest", "user-1", 500), NOW)
        .expect("register");
    let vested = svc
        .vest_partial(grant.id, 200, NOW + 60)
        .expect("vest_partial");
    assert_eq!(vested.state, GrantState::Posted);
    assert!(vested.resolution_transfer_id.is_some());
    assert_eq!(svc.ledger().credits_posted(HOLDER), 200);
    assert_eq!(svc.ledger().credits_pending(HOLDER), 0);
}

#[test]
fn policy_partial_vest_amount_must_be_a_proper_partial() {
    // Oracle: InvalidAmount for a zero vest (nothing to post —
    // releasing a hold is the void path) and for the full amount
    // or more (a full vest is a plain vest). Rejections resolve
    // nothing and spend no resolution: a proper partial vest
    // still lands afterwards.
    let mut svc = service();
    let grant = svc
        .register(&request("key-partial-vest-bounds", "user-1", 500), NOW)
        .expect("register");
    for attempted in [0, 500, 501] {
        assert_eq!(
            svc.vest_partial(grant.id, attempted, NOW + 60),
            Err(GrantError::InvalidAmount {
                attempted,
                grant_amount: 500,
            })
        );
    }
    assert_eq!(svc.get(grant.id).expect("grant").state, GrantState::Pending);
    assert_eq!(svc.ledger().credits_pending(HOLDER), 500);
    assert_eq!(svc.ledger().credits_posted(HOLDER), 0);

    let vested = svc
        .vest_partial(grant.id, 125, NOW + 120)
        .expect("vest_partial after rejections");
    assert_eq!(vested.state, GrantState::Posted);
    assert_eq!(svc.ledger().credits_posted(HOLDER), 125);
    assert_eq!(svc.ledger().credits_pending(HOLDER), 0);
}

#[test]
fn policy_partial_vest_resolves_at_most_once() {
    // Oracle: at-most-once resolution (design section 2.4's
    // 409): after a partial vest, any further vest, partial
    // vest, or void is AlreadyResolved carrying the reached
    // state, and no more value moves.
    let mut svc = service();
    let grant = svc
        .register(&request("key-partial-vest-once", "user-1", 500), NOW)
        .expect("register");
    svc.vest_partial(grant.id, 200, NOW + 60)
        .expect("vest_partial");
    let already = Err(GrantError::AlreadyResolved {
        grant_id: grant.id,
        state: GrantState::Posted,
    });
    assert_eq!(svc.vest(grant.id, NOW + 120), already);
    assert_eq!(svc.vest_partial(grant.id, 100, NOW + 120), already);
    assert_eq!(svc.void(grant.id, NOW + 120), already);
    assert_eq!(svc.ledger().credits_posted(HOLDER), 200);
    assert_eq!(svc.ledger().credits_pending(HOLDER), 0);

    assert_eq!(
        svc.vest_partial(999, 100, NOW),
        Err(GrantError::UnknownGrant(999))
    );
}

#[test]
fn late_refund_with_balance_held_settles_clawback() {
    // Oracle: design sections 4 + 6.2 — a refund that arrives
    // after vest takes the clawback path. While the holder still
    // holds the balance, the clawback transfer settles: the
    // posted balance returns to the grant pool and the grant row
    // reads clawback_settled.
    let mut svc = service();
    let grant = svc
        .register(&request("key-late-held", "user-1", 500), NOW)
        .expect("register");
    svc.vest(grant.id, NOW + 60).expect("vest");
    assert_eq!(svc.ledger().credits_posted(HOLDER), 500);

    let settled = svc
        .refund_late(grant.id, 500, POOL, HOLDER, NOW + 120)
        .expect("late refund");
    assert_eq!(settled.state, GrantState::ClawbackSettled);
    assert_eq!(svc.ledger().credits_posted(HOLDER), 0);
    assert_eq!(svc.ledger().credits_posted(POOL), 500);
}

#[test]
fn late_refund_after_spending_records_debt_and_moves_nothing() {
    // Oracle: design section 6.2's bounded exposure — the holder
    // spent part of the posted balance, so the clawback transfer
    // fails exceeds_credits: nothing moves, the balance the
    // holder still has stays untouched and non-negative, and
    // the grant is marked debt (the recorded receivable). Debt
    // is an outcome of the refund, not an error.
    let mut svc = service();
    let grant = svc
        .register(&request("key-late-spent", "user-1", 500), NOW)
        .expect("register");
    svc.vest(grant.id, NOW + 60).expect("vest");
    svc.ledger_mut().spend_posted(HOLDER, 200);
    assert_eq!(svc.ledger().credits_posted(HOLDER), 300);

    let debt = svc
        .refund_late(grant.id, 500, POOL, HOLDER, NOW + 120)
        .expect("late refund reaching debt is Ok");
    assert_eq!(debt.state, GrantState::Debt);
    assert!(debt.last_error.is_some());
    assert_eq!(svc.ledger().credits_posted(HOLDER), 300);
    assert_eq!(svc.ledger().credits_posted(POOL), 0);
}

#[test]
fn partial_late_refund_claws_back_only_the_refunded_part() {
    // Oracle: a late refund of part of a posted grant claws
    // back exactly that part; the rest stays the holder's
    // posted balance.
    let mut svc = service();
    let grant = svc
        .register(&request("key-late-partial", "user-1", 500), NOW)
        .expect("register");
    svc.vest(grant.id, NOW + 60).expect("vest");

    let settled = svc
        .refund_late(grant.id, 200, POOL, HOLDER, NOW + 120)
        .expect("partial late refund");
    assert_eq!(settled.state, GrantState::ClawbackSettled);
    assert_eq!(svc.ledger().credits_posted(HOLDER), 300);
    assert_eq!(svc.ledger().credits_posted(POOL), 200);
}

#[test]
fn late_refund_requires_a_posted_grant_and_resolves_at_most_once() {
    // Oracle: a refund of a pending grant is the void path,
    // never a clawback (NotPosted; nothing resolves, and a void
    // still lands afterwards). A clawback that already finished
    // — settled or debt — and any other resolved grant answer
    // AlreadyResolved (the 409), and no more value moves.
    let mut svc = service();
    let pending = svc
        .register(&request("key-late-pending", "user-1", 100), NOW)
        .expect("register");
    assert_eq!(
        svc.refund_late(pending.id, 100, POOL, HOLDER, NOW),
        Err(GrantError::NotPosted {
            grant_id: pending.id,
            state: GrantState::Pending,
        })
    );
    assert_eq!(
        svc.get(pending.id).expect("grant").state,
        GrantState::Pending
    );
    svc.void(pending.id, NOW + 60).expect("void still lands");
    assert_eq!(
        svc.refund_late(pending.id, 100, POOL, HOLDER, NOW + 120),
        Err(GrantError::AlreadyResolved {
            grant_id: pending.id,
            state: GrantState::Voided,
        })
    );

    let settled = svc
        .register(&request("key-late-once", "user-1", 100), NOW)
        .expect("register");
    svc.vest(settled.id, NOW).expect("vest");
    svc.refund_late(settled.id, 100, POOL, HOLDER, NOW + 60)
        .expect("clawback");
    assert_eq!(
        svc.refund_late(settled.id, 100, POOL, HOLDER, NOW + 120),
        Err(GrantError::AlreadyResolved {
            grant_id: settled.id,
            state: GrantState::ClawbackSettled,
        })
    );
    assert_eq!(svc.ledger().credits_posted(POOL), 100);

    let spent = svc
        .register(&request("key-late-debt-once", "user-1", 100), NOW)
        .expect("register");
    svc.vest(spent.id, NOW).expect("vest");
    svc.ledger_mut().spend_posted(HOLDER, 100);
    svc.refund_late(spent.id, 100, POOL, HOLDER, NOW + 60)
        .expect("clawback reaching debt");
    assert_eq!(
        svc.refund_late(spent.id, 100, POOL, HOLDER, NOW + 120),
        Err(GrantError::AlreadyResolved {
            grant_id: spent.id,
            state: GrantState::Debt,
        })
    );

    assert_eq!(
        svc.refund_late(999, 100, POOL, HOLDER, NOW),
        Err(GrantError::UnknownGrant(999))
    );
}

#[test]
fn late_refund_amount_must_be_within_the_grant() {
    // Oracle: InvalidAmount for a zero refund (no refund at
    // all) and for more than the grant (never posted, so never
    // clawback-able). Rejections attempt nothing: the row never
    // enters clawback_pending and no value moves; a proper late
    // refund still lands afterwards.
    let mut svc = service();
    let grant = svc
        .register(&request("key-late-bounds", "user-1", 100), NOW)
        .expect("register");
    svc.vest(grant.id, NOW).expect("vest");
    for attempted in [0, 101] {
        assert_eq!(
            svc.refund_late(grant.id, attempted, POOL, HOLDER, NOW + 60),
            Err(GrantError::InvalidAmount {
                attempted,
                grant_amount: 100,
            })
        );
    }
    let row = svc.get(grant.id).expect("grant");
    assert_eq!(row.state, GrantState::Posted);
    assert_eq!(row.attempt_count, 2); // register + vest; rejections added none
    assert_eq!(svc.ledger().credits_posted(HOLDER), 100);
    assert_eq!(svc.ledger().credits_posted(POOL), 0);

    let settled = svc
        .refund_late(grant.id, 100, POOL, HOLDER, NOW + 120)
        .expect("late refund after rejections");
    assert_eq!(settled.state, GrantState::ClawbackSettled);
    assert_eq!(svc.ledger().credits_posted(HOLDER), 0);
    assert_eq!(svc.ledger().credits_posted(POOL), 100);
}

#[test]
fn chained_partial_refunds_void_each_predecessor() {
    // Oracle (#14294): design section 4's void-and-reissue must
    // compose. A second (and third) partial refund of the reissue
    // succeeds; every predecessor stays voided, each tip carries
    // the chain link, and the live hash resolves to the tip.
    let mut svc = service();
    let grant = svc
        .register(&request("key-chain", "user-1", 100), NOW)
        .expect("register");
    let (a, b) = svc
        .refund_partial(grant.id, 10, POOL, HOLDER, NOW + 60)
        .expect("first partial refund");
    assert_eq!(a.state, GrantState::Voided);
    assert_eq!(b.amount, 90);
    assert_eq!(b.correction_of, Some(a.id));

    let (b_row, c) = svc
        .refund_partial(b.id, 10, POOL, HOLDER, NOW + 120)
        .expect("second partial refund");
    assert_eq!(b_row.state, GrantState::Voided);
    assert_eq!(c.state, GrantState::Pending);
    assert_eq!(c.amount, 80);
    assert_eq!(c.correction_of, Some(b.id));
    assert_eq!(svc.get(a.id).expect("A row").state, GrantState::Voided);

    let (_, d) = svc
        .refund_partial(c.id, 10, POOL, HOLDER, NOW + 180)
        .expect("third partial refund");
    assert_eq!(d.amount, 70);
    assert_eq!(d.correction_of, Some(c.id));

    // The live hash resolves to the tip: re-registering the key
    // is idempotent on D, and no posted credit leaked.
    let same = svc
        .register(&request("key-chain", "user-1", 100), NOW + 240)
        .expect("re-register");
    assert_eq!(same.id, d.id);
    assert_eq!(svc.ledger().credits_posted(HOLDER), 0);
    assert_eq!(svc.ledger().credits_pending(HOLDER), 70);
}

#[test]
fn duplicate_key_hash_still_rejects_non_chain_reuse() {
    // Oracle (#14294): the chained-refund fix must not reopen
    // key reuse. A fresh registration over a voided grant (no
    // correction link) and a fork off a non-tip ancestor are
    // both rejected at the store seam.
    let hash = steam_key_hash("key-reuse");
    let row = |id: u128, state: GrantState, correction_of: Option<u128>| Grant {
        id,
        steam_key_hash: hash,
        user_id: "user-1".to_string(),
        ledger: LEDGER,
        amount: 100,
        state,
        pending_transfer_id: 10 + id,
        resolution_transfer_id: None,
        window_started_at: NOW,
        window_ends_at: NOW + WINDOW_SECONDS,
        created_at: NOW,
        updated_at: NOW,
        attempt_count: 0,
        last_error: None,
        correction_of,
    };

    let mut store = InMemoryGrantStore::new();
    store
        .insert_grant(&row(1, GrantState::Voided, None))
        .expect("seed voided");
    assert_eq!(
        store.insert_grant(&row(2, GrantState::Pending, None)),
        Err(GrantStoreError::DuplicateKeyHash),
        "fresh registration over a voided grant stays rejected"
    );

    // A legitimate single reissue is accepted ...
    store
        .insert_grant(&row(3, GrantState::Pending, Some(1)))
        .expect("reissue of the voided tip");
    // ... but a second correction of the same non-tip ancestor
    // (a fork while the tip is live) is rejected.
    assert_eq!(
        store.insert_grant(&row(4, GrantState::Pending, Some(1))),
        Err(GrantStoreError::DuplicateKeyHash),
        "fork off a non-tip ancestor stays rejected"
    );
}
