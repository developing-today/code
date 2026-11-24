//! Signed user-attribute definitions and policy (slice A-GRAPH-9127-01,
//! issue #10222; design part A §6.1 and §12, statements S5.13–S5.19,
//! S5.22, S5.23, S5.24, S5.29).
//!
//! Oracle: **reference** — canonical attribute definitions round-trip;
//! malformed scope, unauthorized policy edits and cross-tenant
//! references reject under the independent schema and tenancy model.
//! The independent schema model is [`reference_validate`] below: a
//! validator written in this file that re-checks the payload's field
//! set, vocabularies, bounds and scope form without calling the
//! module's decoder. The tenancy model is [`FixtureAuthority`], a
//! fixture implementation of the #9083 tenant-authority boundary: an
//! owner and a group admin may define and edit policy in their
//! tenant, a member may do neither (S5.22, S5.23), and a reference is
//! cross-tenant when the fixture homes it in another tenant, denied
//! unless a sharing pair is recorded.
//!
//! `graph/mod.rs` wiring is declared by the slice itself; the test
//! still includes the modules by path, as the sibling graph tests do,
//! so it exercises the exact source files.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/canonical_cbor.rs"]
#[allow(dead_code)]
mod canonical_cbor;

#[path = "../src/id_shape.rs"]
#[allow(dead_code)]
mod id_shape;

#[path = "../src/hash_profile.rs"]
#[allow(dead_code)]
mod hash_profile;

#[path = "../src/signed_envelope.rs"]
#[allow(dead_code)]
mod signed_envelope;

#[path = "../src/graph/term.rs"]
#[allow(dead_code)]
mod term;

#[path = "../src/graph/space_binding.rs"]
#[allow(dead_code)]
mod space_binding;

// `attribute.rs` reaches `term.rs` and `space_binding.rs` as
// `super::term` / `super::space_binding`, which resolves the same way
// here (all at this crate's root) as under `graph/` in id-core.
#[path = "../src/graph/attribute.rs"]
mod attribute;

use attribute::{
    AdmissionRule, AppendMode, AttributeDefinition, AttributeError, AttributeScope, Audience,
    AuthorityOp, TenantAuthority, TenantId, authorize_definition, check_policy_revision, open,
};
use canonical_cbor::{Value, decode_canonical, encode_canonical};
use ed25519_dalek::{SigningKey, VerifyingKey};
use hash_profile::ProfileDispatch;
use id_shape::{ContentId, HashProfile};
use signed_envelope::{Body, Envelope, PayloadId, SUITE_ED25519};
use space_binding::SpaceAddress;
use term::{ObjectRef, Term};

const RAW: HashProfile = HashProfile::RawBlake3_256;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn tenant_a() -> TenantId {
    TenantId::parse("0123456789abcdef0123456789abcdef").unwrap()
}

fn tenant_b() -> TenantId {
    TenantId::parse("fedcba9876543210fedcba9876543210").unwrap()
}

fn content_ref(seed: &[u8]) -> ObjectRef {
    ObjectRef::Content(ContentId::of_bytes(RAW, seed).unwrap())
}

fn dispatch() -> ProfileDispatch {
    ProfileDispatch::default()
}

/// A definition of the `comments` append-list attribute on a user
/// object, in tenant A, controlled by `owner`'s space.
fn definition(owner: &SigningKey, append_mode: AppendMode) -> AttributeDefinition {
    AttributeDefinition {
        scope: AttributeScope {
            tenant: tenant_a(),
            owner_space: SpaceAddress::for_key(owner.verifying_key().to_bytes()).unwrap(),
            target: content_ref(b"user object for geordi.png"),
            name: "comments".to_owned(),
        },
        kind: "append-list".to_owned(),
        schema_ref: None,
        audience: Audience::Members,
        append_mode,
        admission_rule: AdmissionRule::AdminsAndMembers,
        retention_policy_id: "retention/default-30d".to_owned(),
        revision: 1,
    }
}

/// The #9083 tenant-authority boundary as a fixture: one tenant, an
/// owner and an admin authorized to define and edit, a member
/// authorized for neither, and a reference home table.
struct FixtureAuthority {
    tenant: TenantId,
    owner: VerifyingKey,
    admin: VerifyingKey,
    homes: Vec<(ObjectRef, TenantId)>,
    shared: Vec<(TenantId, TenantId)>,
}

impl FixtureAuthority {
    fn new(owner: &SigningKey, admin: &SigningKey) -> Self {
        Self {
            tenant: tenant_a(),
            owner: owner.verifying_key(),
            admin: admin.verifying_key(),
            homes: Vec::new(),
            shared: Vec::new(),
        }
    }
}

impl TenantAuthority for FixtureAuthority {
    fn authorize(&self, tenant: &TenantId, principal: &VerifyingKey, op: AuthorityOp) -> bool {
        if *tenant != self.tenant {
            return false;
        }
        match op {
            AuthorityOp::Define | AuthorityOp::EditPolicy => {
                *principal == self.owner || *principal == self.admin
            }
        }
    }

    fn tenant_of_reference(&self, reference: &ObjectRef) -> Option<TenantId> {
        self.homes
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, tenant)| tenant.clone())
    }

    fn allows_shared_reference(&self, from: &TenantId, to: &TenantId) -> bool {
        self.shared
            .iter()
            .any(|(candidate_from, candidate_to)| candidate_from == from && candidate_to == to)
    }
}

/// The independent reference schema validator: re-checks a payload's
/// field set, vocabularies, bounds and scope form from the design,
/// without the module's decoder.
fn reference_validate(bytes: &[u8]) -> Result<(), String> {
    let value = decode_canonical(bytes).map_err(|error| error.to_string())?;
    let Value::Map(entries) = &value else {
        return Err("payload is not a map".to_owned());
    };
    let get = |name: &str| {
        entries
            .iter()
            .find(|(key, _)| matches!(key, Value::Text(key) if key == name))
            .map(|(_, value)| value)
    };
    const KNOWN: [&str; 12] = [
        "v",
        "tenant",
        "owner_space",
        "target",
        "name",
        "kind",
        "schema",
        "audience",
        "append_mode",
        "admission",
        "retention_policy",
        "revision",
    ];
    for (key, _) in entries {
        let Value::Text(name) = key else {
            return Err("non-text key".to_owned());
        };
        if !KNOWN.contains(&name.as_str()) {
            return Err(format!("unknown field {name}"));
        }
    }
    if get("v") != Some(&Value::UInt(1)) {
        return Err("schema version is not 1".to_owned());
    }
    let Some(Value::Text(tenant)) = get("tenant") else {
        return Err("tenant is not text".to_owned());
    };
    if tenant.len() != 32
        || !tenant
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("tenant is not 32 lowercase hex".to_owned());
    }
    let Some(Value::Bytes(space)) = get("owner_space") else {
        return Err("owner_space is not bytes".to_owned());
    };
    SpaceAddress::from_bytes(space).map_err(|error| format!("{error:?}"))?;
    for field in ["target", "schema"] {
        if let Some(term_value) = get(field) {
            match Term::from_value(term_value) {
                Ok(Term::Object(_)) => {}
                _ => return Err(format!("{field} is not an object reference")),
            }
        } else if field == "target" {
            return Err("target is missing".to_owned());
        }
    }
    let text = |field: &str| -> Result<&str, String> {
        match get(field) {
            Some(Value::Text(text)) => Ok(text),
            _ => Err(format!("{field} is not text")),
        }
    };
    let name = text("name")?;
    if name.is_empty() || name.len() > 128 || name.contains('/') {
        return Err("name is not one bounded path segment".to_owned());
    }
    let kind = text("kind")?;
    if kind.is_empty() || kind.len() > 64 {
        return Err("kind is not bounded text".to_owned());
    }
    if !["private", "members", "public"].contains(&text("audience")?) {
        return Err("audience outside its vocabulary".to_owned());
    }
    if !["closed", "open", "moderated"].contains(&text("append_mode")?) {
        return Err("append_mode outside its vocabulary".to_owned());
    }
    if !["owner_only", "admins", "members", "admins_and_members"].contains(&text("admission")?) {
        return Err("admission outside its vocabulary".to_owned());
    }
    let policy = text("retention_policy")?;
    if policy.is_empty() || policy.len() > 128 {
        return Err("retention_policy is not bounded text".to_owned());
    }
    match get("revision") {
        Some(Value::UInt(revision)) if *revision >= 1 => {}
        _ => return Err("revision is not an unsigned integer >= 1".to_owned()),
    }
    Ok(())
}

/// Rebuilds a payload with one map entry replaced or removed, so a
/// test can present exactly one malformed field at a time.
fn mutated_payload(definition: &AttributeDefinition, field: &str, value: Option<Value>) -> Vec<u8> {
    let Value::Map(entries) = definition.to_value().unwrap() else {
        panic!("definition payload is a map");
    };
    let mut entries: Vec<(Value, Value)> = entries
        .into_iter()
        .filter(|(key, _)| !matches!(key, Value::Text(name) if name == field))
        .collect();
    if let Some(value) = value {
        entries.push((Value::Text(field.to_owned()), value));
    }
    encode_canonical(&Value::Map(entries)).unwrap()
}

#[test]
fn canonical_definitions_round_trip() {
    let owner = key(1);
    let admin = key(2);
    let authority = FixtureAuthority::new(&owner, &admin);
    for mode in [AppendMode::Closed, AppendMode::Open, AppendMode::Moderated] {
        for schema_ref in [None, Some(content_ref(b"comment schema object"))] {
            for kind in ["append-list", "text"] {
                let mut candidate = definition(&owner, mode);
                candidate.schema_ref = schema_ref;
                candidate.kind = kind.to_owned();

                // Payload round-trip, agreed by the reference validator.
                let payload = candidate.payload_bytes().unwrap();
                reference_validate(&payload).unwrap();
                assert_eq!(
                    AttributeDefinition::from_payload(&payload).unwrap(),
                    candidate
                );

                // Signed round-trip through the envelope bytes.
                let opened = candidate.sign("owner", &owner, None).unwrap();
                let bytes = opened.envelope.canonical_bytes().unwrap();
                let reopened = open(Envelope::decode(&bytes).unwrap(), &dispatch()).unwrap();
                assert_eq!(reopened, opened);
                assert_eq!(reopened.definition, candidate);

                // Signing is deterministic: the same definition and
                // key produce the same envelope, byte for byte.
                let resigned = candidate.sign("owner", &owner, None).unwrap();
                assert_eq!(resigned.envelope.canonical_bytes().unwrap(), bytes);

                // The tenant authority admits the owner's definition.
                authorize_definition(&authority, &reopened).unwrap();
            }
        }
    }
}

#[test]
fn explicit_append_modes_are_distinct_states() {
    let owner = key(1);
    assert!(!AppendMode::Closed.accepts_submissions());
    assert!(AppendMode::Open.accepts_submissions());
    assert!(AppendMode::Moderated.accepts_submissions());
    let payloads: Vec<Vec<u8>> = [AppendMode::Closed, AppendMode::Open, AppendMode::Moderated]
        .iter()
        .map(|mode| definition(&owner, *mode).payload_bytes().unwrap())
        .collect();
    assert_ne!(payloads[0], payloads[1]);
    assert_ne!(payloads[1], payloads[2]);
    assert_ne!(payloads[0], payloads[2]);
    for (mode, payload) in [AppendMode::Closed, AppendMode::Open, AppendMode::Moderated]
        .iter()
        .zip(&payloads)
    {
        assert_eq!(
            AttributeDefinition::from_payload(payload).unwrap().append_mode,
            *mode
        );
    }
}

#[test]
fn malformed_scope_and_policy_reject() {
    let owner = key(1);
    let base = definition(&owner, AppendMode::Open);

    // Malformed scopes built through the typed API.
    for bad_name in ["", "comments/extra", &"n".repeat(129)] {
        let mut candidate = base.clone();
        candidate.scope.name = bad_name.to_owned();
        assert_eq!(
            candidate.payload_bytes().unwrap_err().class(),
            "malformed",
            "name {bad_name:?}"
        );
    }
    for bad_tenant in ["", "0123456789abcdef", "0123456789ABCDEF0123456789abcdef", "zz3456789abcdef0123456789abcdef"] {
        assert!(TenantId::parse(bad_tenant).is_err(), "tenant {bad_tenant:?}");
    }

    // Malformed payloads, one field at a time; the module and the
    // reference validator must both reject every one.
    let cases: Vec<(&str, Option<Value>)> = vec![
        ("name", Some(Value::Text(String::new()))),
        ("name", Some(Value::Text("a/b".to_owned()))),
        ("name", Some(Value::Text("n".repeat(129)))),
        ("tenant", Some(Value::Text("0123456789ABCDEF0123456789abcdef".to_owned()))),
        ("tenant", Some(Value::Text("short".to_owned()))),
        ("owner_space", Some(Value::Bytes(vec![1, 2, 3]))),
        ("target", Some(Term::typed(7, b"not a reference".to_vec()).to_value().unwrap())),
        ("kind", Some(Value::Text(String::new()))),
        ("audience", Some(Value::Text("everyone".to_owned()))),
        ("append_mode", Some(Value::Text("ajar".to_owned()))),
        ("append_mode", None),
        ("admission", Some(Value::Text("anyone".to_owned()))),
        ("retention_policy", Some(Value::Text(String::new()))),
        ("revision", Some(Value::UInt(0))),
        ("v", Some(Value::UInt(2))),
        ("surprise", Some(Value::UInt(1))),
    ];
    for (field, value) in cases {
        let payload = mutated_payload(&base, field, value);
        assert!(
            AttributeDefinition::from_payload(&payload).is_err(),
            "module accepted a payload with bad {field}"
        );
        assert!(
            reference_validate(&payload).is_err(),
            "reference validator accepted a payload with bad {field}"
        );
    }

    // A non-canonical encoding of a good payload rejects too.
    let mut payload = base.payload_bytes().unwrap();
    payload.push(0);
    assert!(AttributeDefinition::from_payload(&payload).is_err());
}

#[test]
fn unauthorized_policy_edits_reject() {
    let owner = key(1);
    let admin = key(2);
    let member = key(3);
    let authority = FixtureAuthority::new(&owner, &admin);

    let current = definition(&owner, AppendMode::Open)
        .sign("owner", &owner, None)
        .unwrap();
    authorize_definition(&authority, &current).unwrap();

    // The admin's edit: same scope, next revision, predecessor named.
    let mut edit = current.definition.clone();
    edit.append_mode = AppendMode::Moderated;
    edit.revision = 2;
    let admin_edit = edit.sign("admin", &admin, Some(current.id)).unwrap();
    check_policy_revision(&authority, &current, &admin_edit).unwrap();
    authorize_definition(&authority, &admin_edit).unwrap();

    // A member's edit of the same policy: rejected (S5.22).
    let member_edit = edit.sign("member", &member, Some(current.id)).unwrap();
    assert_eq!(
        check_policy_revision(&authority, &current, &member_edit).unwrap_err(),
        AttributeError::UnauthorizedPolicyEdit
    );
    // And a member may not define an attribute in the first place.
    let member_definition = definition(&owner, AppendMode::Open)
        .sign("member", &member, None)
        .unwrap();
    assert_eq!(
        authorize_definition(&authority, &member_definition).unwrap_err(),
        AttributeError::TenantDenied
    );

    // A skipped revision, a moved scope and a missing predecessor
    // each reject, whoever signs.
    let mut skipped = edit.clone();
    skipped.revision = 3;
    let skipped = skipped.sign("admin", &admin, Some(current.id)).unwrap();
    assert_eq!(
        check_policy_revision(&authority, &current, &skipped).unwrap_err(),
        AttributeError::RevisionMismatch
    );
    let mut moved = edit.clone();
    moved.scope.name = "renamed".to_owned();
    let moved = moved.sign("admin", &admin, Some(current.id)).unwrap();
    assert_eq!(
        check_policy_revision(&authority, &current, &moved).unwrap_err(),
        AttributeError::ScopeChanged
    );
    let unchained = edit.sign("admin", &admin, None).unwrap();
    assert_eq!(
        check_policy_revision(&authority, &current, &unchained).unwrap_err(),
        AttributeError::MissingPredecessor
    );
}

#[test]
fn cross_tenant_references_reject() {
    let owner = key(1);
    let admin = key(2);
    let schema = content_ref(b"schema object homed elsewhere");
    let mut authority = FixtureAuthority::new(&owner, &admin);
    authority.homes.push((schema, tenant_b()));

    let mut candidate = definition(&owner, AppendMode::Open);
    candidate.schema_ref = Some(schema);
    let opened = candidate.sign("owner", &owner, None).unwrap();
    assert_eq!(
        authorize_definition(&authority, &opened).unwrap_err(),
        AttributeError::CrossTenantReference
    );

    // A recorded sharing decision by the tenancy authority admits it.
    authority.shared.push((tenant_a(), tenant_b()));
    authorize_definition(&authority, &opened).unwrap();

    // A reference homed in the definition's own tenant is fine, and
    // so is the target when the fixture homes it locally.
    let mut local = FixtureAuthority::new(&owner, &admin);
    local
        .homes
        .push((candidate.scope.target, tenant_a()));
    let local_opened = definition(&owner, AppendMode::Open)
        .sign("owner", &owner, None)
        .unwrap();
    authorize_definition(&local, &local_opened).unwrap();

    // A definition administered in another tenant is outside this
    // authority entirely: its own owner's signature does not help.
    let mut foreign = definition(&owner, AppendMode::Open);
    foreign.scope.tenant = tenant_b();
    let foreign = foreign.sign("owner", &owner, None).unwrap();
    assert_eq!(
        authorize_definition(&local, &foreign).unwrap_err(),
        AttributeError::TenantDenied
    );
}

#[test]
fn envelope_tenant_binding_and_domain_reject() {
    let owner = key(1);
    let candidate = definition(&owner, AppendMode::Open);
    let payload = candidate.payload_bytes().unwrap();

    // A validly signed envelope whose context names tenant B while
    // the payload names tenant A: the signature does not bind.
    let body_for = |domain: &str, context: &str| Body {
        domain: domain.to_owned(),
        context: context.to_owned(),
        subject: None,
        graph: None,
        predecessor: None,
        signer_id: "owner".to_owned(),
        key: owner.verifying_key().to_bytes(),
        suite: SUITE_ED25519.to_owned(),
        payload_id: PayloadId::of(attribute::DEFINITION_ID_PROFILE, &payload).unwrap(),
        payload_length: payload.len() as u64,
    };
    let mismatched = Envelope::sign(
        body_for(
            attribute::ATTRIBUTE_DEFINITION_DOMAIN,
            tenant_b().as_str(),
        ),
        payload.clone(),
        &owner,
    )
    .unwrap();
    assert_eq!(
        open(mismatched, &dispatch()).unwrap_err(),
        AttributeError::TenantMismatch
    );

    // The same payload under another domain is not a definition.
    let wrong_domain = Envelope::sign(
        body_for("id/graph-record", tenant_a().as_str()),
        payload,
        &owner,
    )
    .unwrap();
    assert!(matches!(
        open(wrong_domain, &dispatch()).unwrap_err(),
        AttributeError::WrongDomain(_)
    ));

    // A tampered payload no longer verifies as the signed definition.
    let opened = candidate.sign("owner", &owner, None).unwrap();
    let mut bytes = opened.envelope.canonical_bytes().unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    assert!(open(Envelope::decode(&bytes).unwrap(), &dispatch()).is_err());
}
