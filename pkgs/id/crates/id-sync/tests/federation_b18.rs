//! B18 invariant test: query-interest disclosure (#10264, statement
//! S1.63).
//!
//! Oracle: **invariant** — only configured, authorized remotes receive
//! query fields, and only the configured fields. A reference model in
//! this file computes, from the configuration alone, which remotes may
//! learn the query, which fields each learns and why every other remote
//! learns nothing. Random policies, queries and candidate lists are
//! compared with the slice's release, and random honest and tampered
//! sends (to an unreleased remote, with an extra or missing field, with
//! changed terms or retention, after expiry) go through the gate; the
//! gate's send log, which records the recipient set and the disclosed
//! fields, must equal the reference log exactly.
//!
//! The slice modules are loaded by path: the federation roots record
//! them without declaring them (slice 9110-02), and B18 reaches B20
//! through `super::`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b20_request_identity.rs"]
#[allow(dead_code)]
mod b20_request_identity;

#[path = "../src/federation/slices/b18_query_privacy.rs"]
mod b18_query_privacy;

use std::collections::{BTreeMap, BTreeSet};

use b18_query_privacy::{
    Decision, Disclose, Disclosure, Field, Gate, GateError, MAX_RETENTION_SECS, PolicyError,
    PrivacyPolicy, Query, ReleaseError, ScopePolicy, Sent, Withheld, release,
};
use b20_request_identity::{PeerKey, RequestId};
use proptest::prelude::*;

const fn peer(n: u8) -> PeerKey {
    [n; 32]
}

fn fields(list: &[Field]) -> BTreeSet<Field> {
    list.iter().copied().collect()
}

fn disclose(recipients: &[u8], list: &[Field], decision: Decision) -> Disclose {
    Disclose {
        recipients: recipients.iter().map(|n| peer(*n)).collect(),
        fields: fields(list),
        retention_secs: 600,
        relay: false,
        decision,
    }
}

fn policy(entries: Vec<(&[u8], ScopePolicy)>) -> PrivacyPolicy {
    PrivacyPolicy::new(entries.into_iter().map(|(s, p)| (s.to_vec(), p)).collect()).unwrap()
}

fn query(scope: &[u8]) -> Query {
    Query {
        request_id: RequestId([7; 32]),
        origin: [9; 32],
        scope: scope.to_vec(),
        terms: b"name = alice".to_vec(),
        limit: Some(20),
        expires_at: 10_000,
    }
}

const ALL: [Field; 5] = Field::ALL;
const NO_ORIGIN: [Field; 4] = [Field::Terms, Field::Scope, Field::Limit, Field::Deadline];

fn refused(scope: &[u8], d: Disclose) -> PolicyError {
    PrivacyPolicy::new(BTreeMap::from([(scope.to_vec(), ScopePolicy::Disclose(d))])).unwrap_err()
}

#[test]
fn a_policy_must_state_a_complete_truthful_disclosure() {
    let s = b"docs".to_vec();
    let mut d = disclose(&[], &ALL, Decision::Accepted);
    assert_eq!(
        refused(&s, d),
        PolicyError::NoRecipients { scope: s.clone() }
    );

    d = disclose(&[1], &[Field::Scope, Field::Limit], Decision::Mitigated);
    assert_eq!(
        refused(&s, d),
        PolicyError::MissingRequired {
            scope: s.clone(),
            field: Field::Terms
        }
    );
    d = disclose(&[1], &[Field::Terms], Decision::Mitigated);
    assert_eq!(
        refused(&s, d),
        PolicyError::MissingRequired {
            scope: s.clone(),
            field: Field::Scope
        }
    );

    d = disclose(&[1], &ALL, Decision::Accepted);
    d.retention_secs = 0;
    assert_eq!(
        refused(&s, d),
        PolicyError::ZeroRetention { scope: s.clone() }
    );
    d = disclose(&[1], &ALL, Decision::Accepted);
    d.retention_secs = MAX_RETENTION_SECS + 1;
    assert_eq!(
        refused(&s, d),
        PolicyError::RetentionAboveCeiling { scope: s.clone() }
    );
    d = disclose(&[1], &ALL, Decision::Accepted);
    d.retention_secs = MAX_RETENTION_SECS;
    assert!(PrivacyPolicy::new(BTreeMap::from([(s.clone(), ScopePolicy::Disclose(d))])).is_ok());

    d = disclose(&[1], &NO_ORIGIN, Decision::Mitigated);
    d.relay = true;
    assert_eq!(
        refused(&s, d),
        PolicyError::RelayNeedsOrigin { scope: s.clone() }
    );

    d = disclose(&[1], &NO_ORIGIN, Decision::Accepted);
    assert_eq!(
        refused(&s, d),
        PolicyError::DecisionMismatch { scope: s.clone() }
    );
    d = disclose(&[1], &ALL, Decision::Mitigated);
    assert_eq!(
        refused(&s, d),
        PolicyError::DecisionMismatch { scope: s.clone() }
    );

    // The first bad scope in scope order is the one reported, even
    // behind a good one.
    let bad = BTreeMap::from([
        (b"a".to_vec(), ScopePolicy::LocalOnly),
        (
            b"b".to_vec(),
            ScopePolicy::Disclose(disclose(&[1], &ALL, Decision::Accepted)),
        ),
        (
            b"c".to_vec(),
            ScopePolicy::Disclose(disclose(&[], &ALL, Decision::Accepted)),
        ),
        (
            b"d".to_vec(),
            ScopePolicy::Disclose(disclose(&[1], &ALL, Decision::Mitigated)),
        ),
    ]);
    assert_eq!(
        PrivacyPolicy::new(bad).unwrap_err(),
        PolicyError::NoRecipients {
            scope: b"c".to_vec()
        }
    );
}

#[test]
fn local_only_and_unconfigured_scopes_disclose_nothing() {
    let p = policy(vec![(b"medical", ScopePolicy::LocalOnly)]);
    let cands = [peer(1), peer(2)];

    let r = release(&p, &query(b"medical"), &cands, 0).unwrap();
    assert!(r.sends.is_empty());
    assert_eq!(r.decision, None);
    assert_eq!(
        r.withheld,
        BTreeMap::from([
            (peer(1), Withheld::LocalOnly),
            (peer(2), Withheld::LocalOnly)
        ])
    );

    let r = release(&p, &query(b"other"), &cands, 0).unwrap();
    assert!(r.sends.is_empty());
    assert_eq!(
        r.withheld,
        BTreeMap::from([
            (peer(1), Withheld::Unconfigured),
            (peer(2), Withheld::Unconfigured)
        ])
    );
}

#[test]
fn only_listed_remotes_receive_and_only_the_listed_fields() {
    let p = policy(vec![(
        b"docs",
        ScopePolicy::Disclose(disclose(&[1, 3], &NO_ORIGIN, Decision::Mitigated)),
    )]);
    let q = query(b"docs");
    let r = release(&p, &q, &[peer(1), peer(2), peer(3), peer(1)], 100).unwrap();

    assert_eq!(r.decision, Some(Decision::Mitigated));
    assert_eq!(r.recipients(), BTreeSet::from([peer(1), peer(3)]));
    assert_eq!(
        r.withheld,
        BTreeMap::from([(peer(2), Withheld::NotAuthorized)])
    );
    let expected = Disclosure {
        request_id: q.request_id,
        scope: b"docs".to_vec(),
        terms: q.terms,
        limit: Some(20),
        deadline: Some(10_000),
        origin: None,
        retain_until: 700,
        relay: false,
    };
    assert_eq!(r.sends[&peer(1)], expected);
    assert_eq!(r.sends[&peer(3)], expected);
    assert_eq!(expected.fields(), fields(&NO_ORIGIN));
}

#[test]
fn an_accepted_disclosure_carries_everything_the_query_has() {
    let mut d = disclose(&[1], &ALL, Decision::Accepted);
    d.relay = true;
    let p = policy(vec![(b"docs", ScopePolicy::Disclose(d))]);
    let mut q = query(b"docs");
    let sent = &release(&p, &q, &[peer(1)], 0).unwrap().sends[&peer(1)];
    assert_eq!(sent.origin, Some(q.origin));
    assert!(sent.relay);
    assert_eq!(sent.fields(), fields(&ALL));

    // A query with no limit has no limit to disclose.
    q.limit = None;
    let sent = &release(&p, &q, &[peer(1)], 0).unwrap().sends[&peer(1)];
    assert_eq!(sent.limit, None);
    assert!(!sent.fields().contains(&Field::Limit));
}

#[test]
fn withheld_optional_fields_stay_withheld_one_by_one() {
    for withhold in [Field::Limit, Field::Deadline, Field::Origin] {
        let list: Vec<Field> = ALL.into_iter().filter(|f| *f != withhold).collect();
        let p = policy(vec![(
            b"docs",
            ScopePolicy::Disclose(disclose(&[1], &list, Decision::Mitigated)),
        )]);
        let sent = &release(&p, &query(b"docs"), &[peer(1)], 0).unwrap().sends[&peer(1)];
        assert_eq!(sent.fields(), fields(&list), "withholding {withhold:?}");
    }
}

#[test]
fn retention_never_outlives_the_query_and_expired_queries_release_nothing() {
    let p = policy(vec![(
        b"docs",
        ScopePolicy::Disclose(disclose(&[1], &ALL, Decision::Accepted)),
    )]);
    let q = query(b"docs");
    assert_eq!(
        release(&p, &q, &[peer(1)], 9_500).unwrap().sends[&peer(1)].retain_until,
        10_000
    );
    assert_eq!(
        release(&p, &q, &[peer(1)], 9_399).unwrap().sends[&peer(1)].retain_until,
        9_999
    );
    assert_eq!(
        release(&p, &q, &[peer(1)], 10_000),
        Err(ReleaseError::Expired)
    );
    assert_eq!(
        release(&p, &q, &[peer(1)], u64::MAX),
        Err(ReleaseError::Expired)
    );
    assert!(release(&p, &q, &[peer(1)], 9_999).is_ok());
}

#[test]
fn the_gate_sends_exactly_what_was_released_to_whom_it_was_released() {
    let p = policy(vec![(
        b"docs",
        ScopePolicy::Disclose(disclose(&[1], &NO_ORIGIN, Decision::Mitigated)),
    )]);
    let q = query(b"docs");
    let r = release(&p, &q, &[peer(1), peer(2)], 0).unwrap();
    let mut gate = Gate::new();
    let honest = r.sends[&peer(1)].clone();

    // Nothing goes out before the release is admitted.
    assert_eq!(gate.send(peer(1), &honest, 0), Err(GateError::NotReleased));
    gate.admit(&r);
    assert_eq!(gate.held(), 1);

    assert_eq!(gate.send(peer(2), &honest, 0), Err(GateError::NotReleased));
    let mut leaky = honest.clone();
    leaky.origin = Some(q.origin);
    assert_eq!(gate.send(peer(1), &leaky, 0), Err(GateError::Mismatch));
    let mut other = honest.clone();
    other.terms = b"name = bob".to_vec();
    assert_eq!(gate.send(peer(1), &other, 0), Err(GateError::Mismatch));
    let mut longer = honest.clone();
    longer.retain_until += 1;
    assert_eq!(gate.send(peer(1), &longer, 0), Err(GateError::Mismatch));
    let mut foreign = honest.clone();
    foreign.request_id = RequestId([8; 32]);
    assert_eq!(gate.send(peer(1), &foreign, 0), Err(GateError::NotReleased));

    assert_eq!(gate.send(peer(1), &honest, 9_999), Ok(()));
    assert_eq!(gate.send(peer(1), &honest, 10_000), Err(GateError::Expired));
    assert_eq!(
        gate.sent(),
        [Sent {
            peer: peer(1),
            request_id: q.request_id,
            fields: fields(&NO_ORIGIN)
        }]
    );

    gate.prune(9_999);
    assert_eq!(gate.held(), 1);
    gate.prune(10_000);
    assert_eq!(gate.held(), 0);
    assert_eq!(gate.send(peer(1), &honest, 0), Err(GateError::NotReleased));
    assert_eq!(gate.sent().len(), 1, "pruning keeps the send log");
}

// ---------------------------------------------------------------------
// Reference model.

const SCOPES: [&[u8]; 3] = [b"a", b"b", b"c"];
const OPTIONAL: [Field; 3] = [Field::Limit, Field::Deadline, Field::Origin];

#[derive(Debug, Clone)]
enum ArbScope {
    Missing,
    Local,
    Open {
        recipients: u8,
        withhold: u8,
        retention: u64,
        relay: bool,
    },
}

fn arb_scope() -> impl Strategy<Value = ArbScope> {
    prop_oneof![
        1 => Just(ArbScope::Missing),
        1 => Just(ArbScope::Local),
        4 => (1u8..32, 0u8..8, 1u64..2_000, any::<bool>()).prop_map(
            // Relaying needs the origin, so a scope that withholds the
            // origin never relays.
            |(recipients, withhold, retention, relay)| ArbScope::Open {
                recipients,
                withhold,
                retention,
                relay: relay && withhold & 4 == 0,
            }
        ),
    ]
}

/// The reference: `(fields, retain_until, relay)` for each recipient, or
/// the reason it gets nothing.
type Expect = BTreeMap<PeerKey, Result<(BTreeSet<Field>, u64, bool), Withheld>>;

fn reference(scope: &ArbScope, has_limit: bool, expires_at: u64, cands: &[u8], now: u64) -> Expect {
    let mut out = Expect::new();
    for c in cands {
        let verdict = match scope {
            ArbScope::Missing => Err(Withheld::Unconfigured),
            ArbScope::Local => Err(Withheld::LocalOnly),
            ArbScope::Open {
                recipients,
                withhold,
                retention,
                relay,
            } => {
                if recipients & (1 << c) == 0 {
                    Err(Withheld::NotAuthorized)
                } else {
                    let mut f = BTreeSet::from([Field::Terms, Field::Scope]);
                    for (i, opt) in OPTIONAL.iter().enumerate() {
                        let shown = withhold & (1 << i) == 0;
                        if shown && (*opt != Field::Limit || has_limit) {
                            f.insert(*opt);
                        }
                    }
                    Ok((f, (now + retention).min(expires_at), *relay))
                }
            }
        };
        out.insert(peer(*c), verdict);
    }
    out
}

fn build(scope: &ArbScope) -> Option<ScopePolicy> {
    match scope {
        ArbScope::Missing => None,
        ArbScope::Local => Some(ScopePolicy::LocalOnly),
        ArbScope::Open {
            recipients,
            withhold,
            retention,
            relay,
        } => {
            let mut f = BTreeSet::from([Field::Terms, Field::Scope]);
            for (i, opt) in OPTIONAL.iter().enumerate() {
                if withhold & (1 << i) == 0 {
                    f.insert(*opt);
                }
            }
            Some(ScopePolicy::Disclose(Disclose {
                recipients: (0..5)
                    .filter(|n| recipients & (1 << n) != 0)
                    .map(peer)
                    .collect(),
                fields: f,
                retention_secs: *retention,
                relay: *relay,
                decision: if withhold.trailing_zeros() >= 3 {
                    Decision::Accepted
                } else {
                    Decision::Mitigated
                },
            }))
        }
    }
}

#[derive(Debug, Clone)]
enum Tamper {
    Honest,
    ToOther(u8),
    AddOrigin,
    DropDeadline,
    Terms,
    Retain,
    Relay,
    Late,
}

fn arb_tamper() -> impl Strategy<Value = Tamper> {
    prop_oneof![
        4 => Just(Tamper::Honest),
        1 => (0u8..5).prop_map(Tamper::ToOther),
        1 => Just(Tamper::AddOrigin),
        1 => Just(Tamper::DropDeadline),
        1 => Just(Tamper::Terms),
        1 => Just(Tamper::Retain),
        1 => Just(Tamper::Relay),
        1 => Just(Tamper::Late),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn releases_and_sends_match_the_reference(
        scopes in prop::collection::vec(arb_scope(), 3),
        query_scope in 0usize..3,
        has_limit in any::<bool>(),
        expires_at in 1u64..3_000,
        now in 0u64..3_000,
        cands in prop::collection::vec(0u8..5, 0..8),
        sends in prop::collection::vec((0usize..8, arb_tamper()), 0..12),
    ) {
        let mut map = BTreeMap::new();
        for (s, a) in SCOPES.iter().zip(&scopes) {
            if let Some(p) = build(a) {
                map.insert(s.to_vec(), p);
            }
        }
        let p = PrivacyPolicy::new(map).unwrap();
        let q = Query {
            request_id: RequestId([u8::try_from(query_scope).unwrap(); 32]),
            origin: [9; 32],
            scope: SCOPES[query_scope].to_vec(),
            terms: b"interest".to_vec(),
            limit: has_limit.then_some(5),
            expires_at,
        };
        let cand_keys: Vec<PeerKey> = cands.iter().map(|c| peer(*c)).collect();

        let r = release(&p, &q, &cand_keys, now);
        if now >= expires_at {
            prop_assert_eq!(r, Err(ReleaseError::Expired));
            return Ok(());
        }
        let r = r.unwrap();
        let expect = reference(&scopes[query_scope], has_limit, expires_at, &cands, now);

        let mut got = Expect::new();
        for (k, d) in &r.sends {
            prop_assert_eq!(&d.terms, &q.terms);
            prop_assert_eq!(&d.scope, &q.scope);
            prop_assert_eq!(d.request_id, q.request_id);
            prop_assert_eq!(d.origin.is_some_and(|o| o != q.origin), false);
            prop_assert_eq!(d.deadline.is_some_and(|t| t != q.expires_at), false);
            prop_assert_eq!(d.limit.is_some_and(|l| Some(l) != q.limit), false);
            got.insert(*k, Ok((d.fields(), d.retain_until, d.relay)));
        }
        for (k, w) in &r.withheld {
            prop_assert!(got.insert(*k, Err(*w)).is_none(), "a remote both sent and withheld");
        }
        prop_assert_eq!(&got, &expect);

        // Every send goes through the gate; the log must be exactly the
        // honest sends to released remotes before expiry.
        let mut gate = Gate::new();
        gate.admit(&r);
        let mut log = Vec::new();
        for (i, tamper) in &sends {
            let Some(c) = cands.get(*i) else { continue };
            let target = peer(*c);
            let base = r.sends.get(&target).cloned().unwrap_or_else(|| Disclosure {
                request_id: q.request_id,
                scope: q.scope.clone(),
                terms: q.terms.clone(),
                limit: q.limit,
                deadline: Some(q.expires_at),
                origin: Some(q.origin),
                retain_until: now,
                relay: false,
            });
            let mut d = base.clone();
            let mut to = target;
            let mut at = now;
            match tamper {
                Tamper::Honest => {}
                Tamper::ToOther(n) => to = peer(*n),
                Tamper::AddOrigin => d.origin = Some(q.origin),
                Tamper::DropDeadline => d.deadline = None,
                Tamper::Terms => d.terms.push(b'!'),
                Tamper::Retain => d.retain_until += 1,
                Tamper::Relay => d.relay = !d.relay,
                Tamper::Late => at = expires_at,
            }
            let released = expect.get(&to).and_then(|v| v.as_ref().ok());
            let want = match released {
                None => Err(GateError::NotReleased),
                Some(_) if at >= expires_at => Err(GateError::Expired),
                Some(_) if r.sends[&to] != d => Err(GateError::Mismatch),
                Some((f, _, _)) => {
                    log.push(Sent { peer: to, request_id: q.request_id, fields: f.clone() });
                    Ok(())
                }
            };
            prop_assert_eq!(gate.send(to, &d, at), want);
        }
        prop_assert_eq!(gate.sent(), log.as_slice());
        for s in gate.sent() {
            prop_assert!(r.recipients().contains(&s.peer));
            prop_assert!(Field::REQUIRED.iter().all(|f| s.fields.contains(f)));
        }
    }
}
