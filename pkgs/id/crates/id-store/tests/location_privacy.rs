//! Model test for scope authorization applied before location
//! candidate counts and projection (slice A9115-privacy-filter,
//! issue #10271; design A-9115.1/A-9115.2 + the #9115 privacy rule).
//!
//! Oracle: **model** — expected `Authorization model:
//! output/candidate counts are functions only of the authorized
//! visible subset.` The reference model below folds the same
//! records into plain maps (assertions harvest fresh, or
//! unverified for `stored_unverified`; a delete tombstone wins
//! over every prior state; a stale tombstone never resurrects a
//! deleted location; absence erases nothing) and then applies a
//! visibility predicate written here, from the design text, not
//! shared with the module: public entries need the explicit public
//! grant or the scope grant; scope entries need the exact scope;
//! stored-unverified entries need the scope and the unverified
//! grant; tombstone-created entries are scope-private. After every
//! generated event, for a fixed panel of authorizations, every
//! module output — candidate counts, candidate lists, visible
//! totals, the materialized projection, single-entry lookups and
//! both acknowledgement forms — is compared against the model
//! restricted to the visible subset.
//!
//! Fixed tests pin the named rules: counts never leak unauthorized
//! existence (adding unauthorized locations changes no output of
//! an unauthorized caller), public enumeration requires the
//! explicit grant, the unverified grant is separate from scope
//! membership, tombstone-created entries are scope-private, and
//! acknowledgements coalesce externally.
//!
//! Why the modules are included by path: slice 9110-02 owns the
//! crate roots, so `sidecar_snapshot`, `location_index` and
//! `location_policy` are not exported from id-store yet.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/location_index.rs"]
mod location_index;
#[path = "../src/location_policy.rs"]
mod location_policy;
#[path = "../src/sidecar_snapshot.rs"]
mod sidecar_snapshot;

use std::collections::BTreeMap;

use ed25519_dalek::SigningKey;

use id_core::canonical_cbor::Value;
use id_core::hash_profile::ProfileDispatch;
use id_core::id_shape::{ContentId, HashProfile};
use id_core::location_ref::TypedLocator;

use location_index::{
    AccessClass, LocationIndex, LocationState, LocationTombstoneV1, TombstoneKind,
};
use location_policy::{AckResult, ExternalAck, ScopeAuthorization};
use sidecar_snapshot::{SidecarBodyV1, SidecarEnvelopeV1, encode_view};

const PROFILE: HashProfile = HashProfile::RawBlake3_256;
const NAMESPACE: &str = "ns/alpha";
const TENANT: &str = "tenant/a";
const DOMAIN: &str = "id.sidecar/v1";
const SIGNER: &str = "sidecar-writer";

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn content_id_value(id: &ContentId) -> Value {
    Value::Map(vec![
        (text("digest"), Value::Bytes(id.digest().to_vec())),
        (text("profile"), text(id.profile().token())),
    ])
}

fn content(seed: &str) -> ContentId {
    ContentId::of_bytes(PROFILE, seed.as_bytes()).unwrap()
}

/// One assertion as the test intends it, before encoding.
#[derive(Debug, Clone)]
struct TestAssertion {
    content: ContentId,
    locator: String,
    class: &'static str,
}

fn assertion(content: ContentId, locator: &str, class: &'static str) -> TestAssertion {
    TestAssertion {
        content,
        locator: locator.to_owned(),
        class,
    }
}

/// Hand-encoded assertion value (independent of the module's own
/// encoder, so a shared encoding bug cannot hide). Keys are put in
/// canonical order (length, then bytes) before encoding.
fn assertion_value(a: &TestAssertion) -> Value {
    let mut entries: Vec<(&str, Value)> = vec![
        ("access_class", text(a.class)),
        ("content_id", content_id_value(&a.content)),
        ("locator", text(&a.locator)),
        ("namespace", text(NAMESPACE)),
        ("observed_at", Value::UInt(1_000)),
        ("signer", text(SIGNER)),
        ("tenant_scope", text(TENANT)),
    ];
    entries.sort_by(|x, y| x.0.len().cmp(&y.0.len()).then_with(|| x.0.cmp(y.0)));
    Value::Map(
        entries
            .into_iter()
            .map(|(k, v)| (text(k), v))
            .collect::<Vec<_>>(),
    )
}

fn snapshot(
    generation: u64,
    parent: Option<sidecar_snapshot::SnapshotId>,
    assertions: &[TestAssertion],
    signer_key: &SigningKey,
) -> SidecarEnvelopeV1 {
    let mut view: BTreeMap<String, Value> = BTreeMap::new();
    if !assertions.is_empty() {
        view.insert(
            "locations".to_owned(),
            Value::Array(assertions.iter().map(assertion_value).collect()),
        );
    }
    let payload = encode_view(&view).unwrap();
    let body = SidecarBodyV1::for_payload(
        content("subject-1"),
        NAMESPACE.to_owned(),
        TENANT.to_owned(),
        generation,
        parent,
        1_700_000_000_000 + generation,
        DOMAIN.to_owned(),
        SIGNER.to_owned(),
        signer_key.verifying_key().to_bytes(),
        PROFILE,
        &payload,
    )
    .unwrap();
    SidecarEnvelopeV1::sign(body, payload, signer_key).unwrap()
}

fn tombstone(
    basis: u64,
    sequence: u64,
    kind: TombstoneKind,
    target: &ContentId,
    locator: &str,
    signer_key: &SigningKey,
) -> LocationTombstoneV1 {
    let body = location_index::TombstoneBodyV1 {
        basis_generation: basis,
        content_id: *target,
        domain_context: location_index::TOMBSTONE_DOMAIN.to_owned(),
        kind,
        locator: TypedLocator::parse_tagged(locator).unwrap(),
        namespace_id: NAMESPACE.to_owned(),
        observed_at: 2_000 + sequence,
        sequence,
        signer_id: SIGNER.to_owned(),
        key: signer_key.verifying_key().to_bytes(),
        suite: "ed25519".to_owned(),
        tenant_scope: TENANT.to_owned(),
    };
    LocationTombstoneV1::sign(body, signer_key).unwrap()
}

fn empty_index() -> LocationIndex {
    LocationIndex::new(NAMESPACE, TENANT).unwrap()
}

const R2_A: &str = "r2/acct/bucket/key-1";
const R2_B: &str = "r2/acct/bucket/key-2";
const LAPTOP: &str = "laptop/host-1/home/user/file.txt";
const DRIVE: &str = "drive/acct/file-9/https://drive.example/9";

fn content_key(id: &ContentId) -> String {
    format!("{}/{}", id.profile().token(), hex::encode(id.digest()))
}

/// The model's view of one authorization, written from the design
/// text alongside — never derived from — the module object it is
/// paired with.
#[derive(Debug, Clone, Copy)]
struct ModelAuth {
    scope: bool,
    public: bool,
    unverified: bool,
}

/// The panel of authorizations every check runs: the module
/// object and its independent model twin.
fn auth_panel() -> Vec<(String, ScopeAuthorization, ModelAuth)> {
    vec![
        (
            "scope".to_owned(),
            ScopeAuthorization::for_scope(NAMESPACE, TENANT).unwrap(),
            ModelAuth {
                scope: true,
                public: false,
                unverified: true,
            },
        ),
        (
            "scope-no-unverified".to_owned(),
            ScopeAuthorization::for_scope(NAMESPACE, TENANT)
                .unwrap()
                .with_unverified_visible(false),
            ModelAuth {
                scope: true,
                public: false,
                unverified: false,
            },
        ),
        (
            "scope+public".to_owned(),
            ScopeAuthorization::for_scope(NAMESPACE, TENANT)
                .unwrap()
                .with_public_visible(true),
            ModelAuth {
                scope: true,
                public: true,
                unverified: true,
            },
        ),
        (
            "public-reader".to_owned(),
            ScopeAuthorization::anonymous().with_public_visible(true),
            ModelAuth {
                scope: false,
                public: true,
                unverified: false,
            },
        ),
        (
            "wrong-tenant".to_owned(),
            ScopeAuthorization::for_scope(NAMESPACE, "tenant/other").unwrap(),
            ModelAuth {
                scope: false,
                public: false,
                unverified: true,
            },
        ),
        (
            "wrong-namespace".to_owned(),
            ScopeAuthorization::for_scope("ns/other", TENANT).unwrap(),
            ModelAuth {
                scope: false,
                public: false,
                unverified: true,
            },
        ),
        (
            "anonymous".to_owned(),
            ScopeAuthorization::anonymous(),
            ModelAuth {
                scope: false,
                public: false,
                unverified: false,
            },
        ),
    ]
}

/// The model's visibility predicate, from the design's privacy
/// rule: default visibility is private within the caller-authorized
/// namespace/tenant; public enumeration is prohibited unless
/// explicitly configured; a stored-unverified observation is its
/// own grant; a tombstone-created entry is scope-private.
fn model_visible(auth: &ModelAuth, class: Option<AccessClass>) -> bool {
    match class {
        Some(AccessClass::Public) => auth.public || auth.scope,
        Some(AccessClass::Scope) => auth.scope,
        Some(AccessClass::StoredUnverified) => auth.scope && auth.unverified,
        None => auth.scope,
    }
}

fn model_ack(visible: bool, state: Option<LocationState>) -> AckResult {
    match (visible, state) {
        (true, Some(LocationState::Fresh)) => AckResult::VerifiedPresent,
        (true, Some(LocationState::Unverified)) => AckResult::Indeterminate,
        _ => AckResult::NotPresent,
    }
}

/// The reference model: an independent fold over plain maps,
/// tracking only what the authorization outputs expose — the
/// access class and state of each (content, locator).
#[derive(Debug, Default)]
struct Model {
    entries: BTreeMap<(String, String), (Option<AccessClass>, LocationState)>,
    head_generation: Option<u64>,
}

impl Model {
    fn apply_snapshot(&mut self, generation: u64, assertions: &[TestAssertion]) {
        for a in assertions {
            let class = AccessClass::from_token(a.class).unwrap();
            let state = if a.class == "stored_unverified" {
                LocationState::Unverified
            } else {
                LocationState::Fresh
            };
            self.entries.insert(
                (content_key(&a.content), a.locator.clone()),
                (Some(class), state),
            );
        }
        self.head_generation = Some(generation);
    }

    fn apply_tombstone(&mut self, kind: TombstoneKind, target: &ContentId, locator: &str) {
        let entry_key = (content_key(target), locator.to_owned());
        match self.entries.get_mut(&entry_key) {
            Some((_, state)) => {
                if !(*state == LocationState::Deleted && kind == TombstoneKind::Stale) {
                    *state = match kind {
                        TombstoneKind::Stale => LocationState::Stale,
                        TombstoneKind::Delete => LocationState::Deleted,
                    };
                }
            }
            None => {
                self.entries.insert(
                    entry_key,
                    (
                        None,
                        match kind {
                            TombstoneKind::Stale => LocationState::Stale,
                            TombstoneKind::Delete => LocationState::Deleted,
                        },
                    ),
                );
            }
        }
    }

    /// The model's authorized outputs for one content: the visible
    /// (locator, class, state) rows in locator order.
    fn visible_for(
        &self,
        auth: &ModelAuth,
        content: &ContentId,
    ) -> Vec<(String, Option<AccessClass>, LocationState)> {
        let prefix = content_key(content);
        let mut rows: Vec<_> = self
            .entries
            .iter()
            .filter(|((c, _), (class, _))| *c == prefix && model_visible(auth, *class))
            .map(|((_, locator), (class, state))| (locator.clone(), *class, *state))
            .collect();
        // Locators are unique per content in the model, so ordering
        // by locator alone is total here.
        rows.sort_by(|x, y| x.0.cmp(&y.0));
        rows
    }

    fn visible_total(&self, auth: &ModelAuth) -> usize {
        self.entries
            .values()
            .filter(|(class, _)| model_visible(auth, *class))
            .count()
    }
}

/// A total sort key for (locator, class, state) rows: the enums
/// carry no `Ord`, so their design tokens stand in.
fn row_key(
    row: &(String, Option<AccessClass>, LocationState),
) -> (String, &'static str, &'static str) {
    (
        row.0.clone(),
        row.1.map_or("", |class| class.token()),
        row.2.token(),
    )
}

/// Compares every module output for every panel authorization
/// against the model restricted to the visible subset.
fn check_all(
    index: &LocationIndex,
    model: &Model,
    contents: &[ContentId],
    locators: &[&str],
    where_: &str,
) {
    for (name, auth, model_auth) in auth_panel() {
        let view = auth.view(index);
        assert_eq!(
            view.visible_len(),
            model.visible_total(&model_auth),
            "{where_}: visible_len diverged for {name}"
        );
        let projected: Vec<(String, Option<AccessClass>, LocationState)> = view
            .projection()
            .iter()
            .map(|e| (e.locator.to_tagged_string(), e.access_class, e.state))
            .collect();
        let mut expected_projection: Vec<(String, Option<AccessClass>, LocationState)> = model
            .entries
            .iter()
            .filter(|(_, (class, _))| model_visible(&model_auth, *class))
            .map(|((_, locator), (class, state))| (locator.clone(), *class, *state))
            .collect();
        expected_projection.sort_by(|x, y| row_key(x).cmp(&row_key(y)));
        let mut projected_sorted = projected.clone();
        projected_sorted.sort_by(|x, y| row_key(x).cmp(&row_key(y)));
        assert_eq!(
            projected_sorted, expected_projection,
            "{where_}: projection diverged for {name}"
        );

        for content in contents {
            let expected = model.visible_for(&model_auth, content);
            assert_eq!(
                view.candidate_count(content),
                expected.len(),
                "{where_}: candidate_count diverged for {name}"
            );
            let got: Vec<(String, Option<AccessClass>, LocationState)> = view
                .candidates(content)
                .iter()
                .map(|e| (e.locator.to_tagged_string(), e.access_class, e.state))
                .collect();
            assert_eq!(got, expected, "{where_}: candidates diverged for {name}");

            for locator in locators {
                let parsed = TypedLocator::parse_tagged(locator).unwrap();
                let model_entry = model
                    .entries
                    .get(&(content_key(content), (*locator).to_owned()));
                let (visible, state) = match model_entry {
                    Some((class, state)) => (model_visible(&model_auth, *class), Some(*state)),
                    None => (false, None),
                };
                assert_eq!(
                    view.entry(content, &parsed).map(|e| e.state),
                    if visible { state } else { None },
                    "{where_}: entry lookup diverged for {name}"
                );
                let expected_ack = model_ack(visible, state);
                assert_eq!(
                    view.acknowledge(content, &parsed),
                    expected_ack,
                    "{where_}: acknowledge diverged for {name}"
                );
                assert_eq!(
                    view.acknowledge_external(content, &parsed),
                    match expected_ack {
                        AckResult::VerifiedPresent => ExternalAck::VerifiedPresent,
                        _ => ExternalAck::NotVerifiedPresent,
                    },
                    "{where_}: acknowledge_external diverged for {name}"
                );
            }
        }
    }
}

#[test]
fn scope_caller_sees_its_scope_and_nothing_else_is_counted() {
    let mut index = empty_index();
    let a = content("content-a");
    let assertions = vec![
        assertion(a, R2_A, "public"),
        assertion(a, LAPTOP, "scope"),
        assertion(a, DRIVE, "scope"),
        assertion(a, R2_B, "stored_unverified"),
    ];
    let env = snapshot(0, None, &assertions, &key());
    index
        .apply_snapshot(&env, &ProfileDispatch::default())
        .unwrap();
    assert_eq!(index.len(), 4);

    let scope = ScopeAuthorization::for_scope(NAMESPACE, TENANT).unwrap();
    assert_eq!(scope.namespace_id(), NAMESPACE);
    assert_eq!(scope.tenant_scope(), TENANT);
    assert!(!scope.public_visible());
    assert!(scope.unverified_visible());
    assert_eq!(scope.view(&index).candidate_count(&a), 4);
    assert_eq!(scope.view(&index).visible_len(), 4);

    // The public reader sees the public location only; the three
    // scope-private locations do not move its count at all.
    let public = ScopeAuthorization::anonymous().with_public_visible(true);
    let view = public.view(&index);
    assert_eq!(view.candidate_count(&a), 1);
    assert_eq!(view.candidates(&a)[0].locator.to_tagged_string(), R2_A);
    assert_eq!(view.visible_len(), 1);

    // Without the explicit public grant, even public locations are
    // not enumerable.
    let anonymous = ScopeAuthorization::anonymous();
    assert_eq!(anonymous.view(&index).candidate_count(&a), 0);
    assert!(anonymous.view(&index).is_visibly_empty());

    // A caller authorized for another tenant or another namespace
    // sees nothing of this scope.
    let wrong_tenant = ScopeAuthorization::for_scope(NAMESPACE, "tenant/other").unwrap();
    assert_eq!(wrong_tenant.view(&index).candidate_count(&a), 0);
    let wrong_ns = ScopeAuthorization::for_scope("ns/other", TENANT).unwrap();
    assert_eq!(wrong_ns.view(&index).candidate_count(&a), 0);
}

#[test]
fn adding_unauthorized_locations_changes_no_unauthorized_output() {
    let a = content("content-a");
    let public_only = vec![assertion(a, R2_A, "public")];
    let mut index_a = empty_index();
    let env = snapshot(0, None, &public_only, &key());
    index_a
        .apply_snapshot(&env, &ProfileDispatch::default())
        .unwrap();

    // A second index whose sidecar additionally carries scope and
    // stored-unverified locations for the same content.
    let mut index_b = empty_index();
    let fuller = vec![
        assertion(a, R2_A, "public"),
        assertion(a, LAPTOP, "scope"),
        assertion(a, R2_B, "stored_unverified"),
    ];
    let env = snapshot(0, None, &fuller, &key());
    index_b
        .apply_snapshot(&env, &ProfileDispatch::default())
        .unwrap();
    assert!(index_b.len() > index_a.len());

    // For every authorization that cannot see the added locations,
    // every output is identical on both indexes: the additions are
    // indistinguishable from absence. (Projections are compared by
    // their query-visible rows — locator, class, state — not by
    // provenance: the two indexes were folded from different
    // snapshots, so the asserting snapshot IDs legitimately differ.)
    for auth in [
        ScopeAuthorization::anonymous().with_public_visible(true),
        ScopeAuthorization::anonymous(),
        ScopeAuthorization::for_scope(NAMESPACE, "tenant/other").unwrap(),
        ScopeAuthorization::for_scope("ns/other", TENANT).unwrap(),
    ] {
        let va = auth.view(&index_a);
        let vb = auth.view(&index_b);
        assert_eq!(va.candidate_count(&a), vb.candidate_count(&a));
        assert_eq!(va.visible_len(), vb.visible_len());
        let rows = |view: &location_policy::AuthorizedLocations<'_>| {
            view.projection()
                .iter()
                .map(|e| (e.locator.to_tagged_string(), e.access_class, e.state))
                .collect::<Vec<_>>()
        };
        assert_eq!(rows(&va), rows(&vb));
        for locator in [R2_A, R2_B, LAPTOP, DRIVE] {
            let parsed = TypedLocator::parse_tagged(locator).unwrap();
            assert_eq!(va.acknowledge(&a, &parsed), vb.acknowledge(&a, &parsed));
            assert_eq!(
                va.acknowledge_external(&a, &parsed),
                vb.acknowledge_external(&a, &parsed)
            );
        }
    }
}

#[test]
fn unverified_grant_is_separate_from_scope_membership() {
    let mut index = empty_index();
    let a = content("content-a");
    let env = snapshot(
        0,
        None,
        &[
            assertion(a, LAPTOP, "scope"),
            assertion(a, R2_B, "stored_unverified"),
        ],
        &key(),
    );
    index
        .apply_snapshot(&env, &ProfileDispatch::default())
        .unwrap();

    let scope = ScopeAuthorization::for_scope(NAMESPACE, TENANT).unwrap();
    assert_eq!(scope.view(&index).candidate_count(&a), 2);

    let no_unverified = scope.clone().with_unverified_visible(false);
    let view = no_unverified.view(&index);
    assert_eq!(view.candidate_count(&a), 1);
    assert_eq!(view.candidates(&a)[0].locator.to_tagged_string(), LAPTOP);
    // The unverified location acknowledges exactly like an absent
    // one once its grant is withheld.
    let parsed = TypedLocator::parse_tagged(R2_B).unwrap();
    assert_eq!(view.acknowledge(&a, &parsed), AckResult::NotPresent);
}

#[test]
fn tombstone_created_entries_are_scope_private_and_states_pass_through() {
    let mut index = empty_index();
    let a = content("content-a");
    let env0 = snapshot(0, None, &[assertion(a, R2_A, "scope")], &key());
    index
        .apply_snapshot(&env0, &ProfileDispatch::default())
        .unwrap();
    // Delete the asserted location, and create a tombstone-only
    // entry for a location that was never asserted.
    index
        .apply_tombstone(&tombstone(0, 0, TombstoneKind::Delete, &a, R2_A, &key()))
        .unwrap();
    index
        .apply_tombstone(&tombstone(0, 1, TombstoneKind::Stale, &a, DRIVE, &key()))
        .unwrap();

    let scope = ScopeAuthorization::for_scope(NAMESPACE, TENANT).unwrap();
    let view = scope.view(&index);
    // Authorization filters visibility, not states: the deleted
    // and stale entries remain candidates, with states intact.
    assert_eq!(view.candidate_count(&a), 2);
    let deleted = TypedLocator::parse_tagged(R2_A).unwrap();
    assert_eq!(
        view.entry(&a, &deleted).unwrap().state,
        LocationState::Deleted
    );
    assert_eq!(view.acknowledge(&a, &deleted), AckResult::NotPresent);
    let stale = TypedLocator::parse_tagged(DRIVE).unwrap();
    let stale_entry = view.entry(&a, &stale).unwrap();
    assert_eq!(stale_entry.state, LocationState::Stale);
    assert_eq!(stale_entry.access_class, None);

    // The tombstone-only entry is scope-private: a public reader
    // cannot learn that anything was retired here.
    let public = ScopeAuthorization::anonymous().with_public_visible(true);
    assert_eq!(public.view(&index).candidate_count(&a), 0);
    assert_eq!(
        public.view(&index).acknowledge(&a, &stale),
        AckResult::NotPresent
    );
}

#[test]
fn acknowledgements_distinguish_internally_and_coalesce_externally() {
    let mut index = empty_index();
    let a = content("content-a");
    let env = snapshot(
        0,
        None,
        &[
            assertion(a, R2_A, "public"),
            assertion(a, R2_B, "stored_unverified"),
        ],
        &key(),
    );
    index
        .apply_snapshot(&env, &ProfileDispatch::default())
        .unwrap();
    let scope = ScopeAuthorization::for_scope(NAMESPACE, TENANT).unwrap();
    let view = scope.view(&index);

    let fresh = TypedLocator::parse_tagged(R2_A).unwrap();
    assert_eq!(view.acknowledge(&a, &fresh), AckResult::VerifiedPresent);
    assert_eq!(
        view.acknowledge_external(&a, &fresh),
        ExternalAck::VerifiedPresent
    );

    // Unverified storage is an observation, never proof of
    // presence: distinct internally, coalesced externally.
    let unverified = TypedLocator::parse_tagged(R2_B).unwrap();
    assert_eq!(view.acknowledge(&a, &unverified), AckResult::Indeterminate);
    assert_eq!(
        view.acknowledge_external(&a, &unverified),
        ExternalAck::NotVerifiedPresent
    );

    let absent = TypedLocator::parse_tagged(DRIVE).unwrap();
    assert_eq!(view.acknowledge(&a, &absent), AckResult::NotPresent);
    assert_eq!(
        view.acknowledge_external(&a, &absent),
        ExternalAck::NotVerifiedPresent
    );
}

#[test]
fn authorization_labels_are_validated_like_index_scopes() {
    let err = ScopeAuthorization::for_scope("", TENANT).unwrap_err();
    assert_eq!(err.class(), "field");
    let err = ScopeAuthorization::for_scope(NAMESPACE, "").unwrap_err();
    assert_eq!(err.class(), "field");
    let err = ScopeAuthorization::for_scope("x".repeat(257), TENANT).unwrap_err();
    assert_eq!(err.class(), "field");
}

/// A tiny deterministic RNG (xorshift64*) so the generated stream
/// is identical on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[test]
fn generated_operations_match_the_authorization_model() {
    let dispatch = ProfileDispatch::default();
    let contents = [
        content("content-a"),
        content("content-b"),
        content("content-c"),
    ];
    let locators = [R2_A, R2_B, LAPTOP, DRIVE];
    let classes = ["public", "scope", "stored_unverified"];

    for seed in [1u64, 42, 0x5eed] {
        let mut rng = Rng(seed);
        let mut index = empty_index();
        let mut model = Model::default();
        let mut generation: u64 = 0;
        let mut parent = None;
        let mut tomb_sequence: u64 = 0;

        for step in 0..120 {
            let roll = rng.below(10);
            if roll < 6 || model.head_generation.is_none() {
                let count = rng.below(5);
                let mut assertions = Vec::new();
                for _ in 0..count {
                    assertions.push(assertion(
                        contents[rng.below(3) as usize],
                        locators[rng.below(4) as usize],
                        classes[rng.below(3) as usize],
                    ));
                }
                let env = snapshot(generation, parent, &assertions, &key());
                index.apply_snapshot(&env, &dispatch).unwrap();
                model.apply_snapshot(generation, &assertions);
                parent = Some(env.snapshot_id().unwrap());
                generation += 1;
                tomb_sequence = 0;
            } else {
                let basis = generation - 1;
                let target = contents[rng.below(3) as usize];
                let locator = locators[rng.below(4) as usize];
                let kind = if rng.below(2) == 0 {
                    TombstoneKind::Stale
                } else {
                    TombstoneKind::Delete
                };
                let t = tombstone(basis, tomb_sequence, kind, &target, locator, &key());
                index.apply_tombstone(&t).unwrap();
                model.apply_tombstone(kind, &target, locator);
                tomb_sequence += 1;
            }
            check_all(
                &index,
                &model,
                &contents,
                &locators,
                &format!("after step {step} (seed {seed})"),
            );
        }
    }
}
