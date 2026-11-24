//! Signed user-attribute definitions and policy (slice A-GRAPH-9127-01,
//! #10222; design part A §6.1 and §12, statements S5.13–S5.19, S5.22,
//! S5.23, S5.24, S5.29).
//!
//! A user attribute is metadata on a user's own object — not Atlas
//! metadata and not content-location metadata (S5.13, S5.14). The
//! attribute is itself an object (S5.16): an [`AttributeDefinition`] is
//! a signed owner/group object addressed by its scope
//! `(tenant, owner_space, target, name)` and carrying a kind, an
//! optional schema reference, an audience, an explicit append mode
//! (`closed | open | moderated`), an admission rule, a
//! retention/deletion policy ID and a revision. `name` on a user's
//! path and `comments` on a path are examples (S5.17, S5.18), never
//! reserved global properties; an attribute is an append-list
//! attribute when its kind says so (S5.19), not by a special case here.
//!
//! Opened versus closed is an explicit per-attribute state (S5.29):
//! the append mode is a required field of every definition, never a
//! default applied later. Submitters cannot set or edit the definition
//! (S5.22); the owner and group admins control it (S5.23), and an
//! explicitly opened attribute accepts append submissions despite that
//! control (S5.24). Submission and admission themselves belong to
//! slice A-GRAPH-9127-02; this module defines the signed definition
//! and policy those checks read.
//!
//! # Payload
//!
//! `canonical_cbor({"v": 1, "tenant": tstr, "owner_space": bstr,
//! "target": term, "name": tstr, "kind": tstr, "schema"?: term,
//! "audience": tstr, "append_mode": tstr, "admission": tstr,
//! "retention_policy": tstr, "revision": uint})`, carried as the
//! payload of a deterministic-CBOR [`Envelope`] under domain
//! [`ATTRIBUTE_DEFINITION_DOMAIN`]. The envelope context is the tenant
//! ID, binding the signature to the tenant scope; a payload naming a
//! different tenant than its envelope context rejects. Every field is
//! critical: an unknown key, a missing required key, another schema
//! version or a non-canonical encoding rejects.
//!
//! # Tenant authority
//!
//! Tenancy is #9083's owned boundary; this module consumes it only,
//! through [`TenantAuthority`]. A definition's signer must be
//! authorized in its tenant for the operation at hand
//! ([`authorize_definition`]); a policy edit must keep the scope, name
//! the definition it revises as its envelope predecessor and carry
//! exactly the next revision ([`check_policy_revision`]); and a
//! reference whose home tenant differs from the definition's tenant
//! rejects unless the authority records a deliberate sharing decision
//! (design §12: cross-tenant references are denied by default, #9083
//! controls any deliberate sharing).

use std::fmt;

use ed25519_dalek::{SigningKey, VerifyingKey};

use super::space_binding::{SpaceAddress, SpaceError};
use super::term::{ObjectRef, Term, TermError};
use crate::canonical_cbor::{CborError, Value, decode_canonical, encode_canonical};
use crate::hash_profile::ProfileDispatch;
use crate::id_shape::{ContentId, HashProfile};
use crate::signed_envelope::{Body, Envelope, EnvelopeError, EnvelopeId, PayloadId, SUITE_ED25519};

/// The payload schema version this reader writes and accepts.
pub const SCHEMA_VERSION: u64 = 1;

/// The signing domain of an attribute definition.
pub const ATTRIBUTE_DEFINITION_DOMAIN: &str = "id/attribute-definition";

/// The profile definition identities are computed under.
pub const DEFINITION_ID_PROFILE: HashProfile = HashProfile::RawBlake3_256;

/// Upper bound on an attribute name, in bytes.
pub const MAX_NAME_BYTES: usize = 128;

/// Upper bound on an attribute kind, in bytes.
pub const MAX_KIND_BYTES: usize = 64;

/// Upper bound on a retention/deletion policy ID, in bytes.
pub const MAX_POLICY_ID_BYTES: usize = 128;

/// A tenant's length in text form: the tenancy registry's
/// `binding_id`, 32 lowercase hex characters (#9083).
const TENANT_ID_LEN: usize = 32;

const FIELDS: [&str; 12] = [
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

/// A tenant scope identifier, in the tenancy registry's form: exactly
/// 32 lowercase hex characters (the `binding_id` a user tenant is
/// named by, #9083).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TenantId(String);

impl TenantId {
    /// Parses a tenant ID, rejecting any other shape.
    ///
    /// # Errors
    ///
    /// [`AttributeError::MalformedScope`] for a wrong length, uppercase
    /// or non-hex characters.
    pub fn parse(text: &str) -> Result<Self, AttributeError> {
        if text.len() != TENANT_ID_LEN
            || !text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(AttributeError::MalformedScope(
                "tenant id must be 32 lowercase hex characters",
            ));
        }
        Ok(Self(text.to_owned()))
    }

    /// The tenant ID as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether an attribute accepts append submissions, and under what
/// review. An explicit per-attribute state (S5.29), always present in
/// a definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppendMode {
    /// Append is rejected.
    Closed,
    /// An eligible signed submission may be submitted (S5.24); it
    /// still enters the pending gate of slice A-GRAPH-9127-02 unless
    /// the admission rule accepts the submitter directly.
    Open,
    /// Submissions enter pending and publish only on approval.
    Moderated,
}

impl AppendMode {
    /// The wire text of this mode.
    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Open => "open",
            Self::Moderated => "moderated",
        }
    }

    /// Reads a mode from its wire text; anything else is not a mode.
    #[must_use]
    pub fn from_wire(text: &str) -> Option<Self> {
        match text {
            "closed" => Some(Self::Closed),
            "open" => Some(Self::Open),
            "moderated" => Some(Self::Moderated),
            _ => None,
        }
    }

    /// Whether the attribute is explicitly opened: submissions may be
    /// submitted at all (S5.24).
    #[must_use]
    pub const fn accepts_submissions(self) -> bool {
        !matches!(self, Self::Closed)
    }
}

/// Who may read an attribute's accepted values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Audience {
    /// The owner/group only.
    Private,
    /// Members of the owning group.
    Members,
    /// Anyone.
    Public,
}

impl Audience {
    /// The wire text of this audience.
    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Members => "members",
            Self::Public => "public",
        }
    }

    /// Reads an audience from its wire text.
    #[must_use]
    pub fn from_wire(text: &str) -> Option<Self> {
        match text {
            "private" => Some(Self::Private),
            "members" => Some(Self::Members),
            "public" => Some(Self::Public),
            _ => None,
        }
    }
}

/// Which submitters the attribute's policy makes eligible for direct
/// acceptance; everyone else eligible to submit waits in pending
/// (design §6.2). This is the definition's configured member/admin
/// policy, read by the admission slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdmissionRule {
    /// Only the owner is directly accepted.
    OwnerOnly,
    /// Group admins are directly accepted.
    Admins,
    /// Group members are directly accepted.
    Members,
    /// Group admins and members are directly accepted.
    AdminsAndMembers,
}

impl AdmissionRule {
    /// The wire text of this rule.
    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::OwnerOnly => "owner_only",
            Self::Admins => "admins",
            Self::Members => "members",
            Self::AdminsAndMembers => "admins_and_members",
        }
    }

    /// Reads a rule from its wire text.
    #[must_use]
    pub fn from_wire(text: &str) -> Option<Self> {
        match text {
            "owner_only" => Some(Self::OwnerOnly),
            "admins" => Some(Self::Admins),
            "members" => Some(Self::Members),
            "admins_and_members" => Some(Self::AdminsAndMembers),
            _ => None,
        }
    }
}

/// The address of one attribute: the tenant it is administered in,
/// the owner/group space that controls it, the object it is an
/// attribute of, and its name on that object. Two definitions with
/// different scopes are different attributes; a policy edit never
/// changes the scope.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AttributeScope {
    /// The tenant the definition is administered in.
    pub tenant: TenantId,
    /// The owner/group space controlling the attribute.
    pub owner_space: SpaceAddress,
    /// The object the attribute is on (a user object, a path object).
    pub target: ObjectRef,
    /// The attribute's name on the target: one path segment, such as
    /// `name` or `comments`.
    pub name: String,
}

impl AttributeScope {
    /// Checks the scope is well-formed.
    ///
    /// # Errors
    ///
    /// [`AttributeError::MalformedScope`] for an empty, oversized or
    /// multi-segment name.
    pub fn validate(&self) -> Result<(), AttributeError> {
        if self.name.is_empty() || self.name.len() > MAX_NAME_BYTES {
            return Err(AttributeError::MalformedScope(
                "attribute name must be 1..=128 bytes",
            ));
        }
        if self.name.contains('/') {
            return Err(AttributeError::MalformedScope(
                "attribute name is one path segment and must not contain '/'",
            ));
        }
        Ok(())
    }
}

/// One signed user-attribute definition and its policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeDefinition {
    /// The attribute's address.
    pub scope: AttributeScope,
    /// The attribute kind, such as `text` or `append-list` (S5.19).
    pub kind: String,
    /// The schema object values must conform to, if the attribute
    /// names one.
    pub schema_ref: Option<ObjectRef>,
    /// Who may read accepted values.
    pub audience: Audience,
    /// The explicit append policy (S5.29).
    pub append_mode: AppendMode,
    /// Who is eligible for direct acceptance.
    pub admission_rule: AdmissionRule,
    /// The retention/deletion policy governing this attribute's
    /// objects, by ID.
    pub retention_policy_id: String,
    /// The definition revision: 1 for the first definition of a
    /// scope, exactly one more per accepted policy edit.
    pub revision: u64,
}

impl AttributeDefinition {
    /// Every object the definition references: its target and its
    /// schema, in signed order.
    pub fn references(&self) -> impl Iterator<Item = ObjectRef> + '_ {
        std::iter::once(self.scope.target).chain(self.schema_ref)
    }

    /// Checks the definition is well-formed, independent of any
    /// signature: scope, bounded texts and a revision of at least 1.
    ///
    /// # Errors
    ///
    /// [`AttributeError::MalformedScope`] or
    /// [`AttributeError::Field`].
    pub fn validate(&self) -> Result<(), AttributeError> {
        self.scope.validate()?;
        if self.kind.is_empty() || self.kind.len() > MAX_KIND_BYTES {
            return Err(field("kind", "must be 1..=64 bytes"));
        }
        if self.retention_policy_id.is_empty()
            || self.retention_policy_id.len() > MAX_POLICY_ID_BYTES
        {
            return Err(field("retention_policy", "must be 1..=128 bytes"));
        }
        if self.revision == 0 {
            return Err(field("revision", "must be at least 1"));
        }
        Ok(())
    }

    /// The payload as a canonical CBOR value.
    ///
    /// # Errors
    ///
    /// A malformed definition, or a term rejection.
    pub fn to_value(&self) -> Result<Value, AttributeError> {
        self.validate()?;
        let mut entries = vec![
            (Value::Text("v".to_owned()), Value::UInt(SCHEMA_VERSION)),
            (
                Value::Text("tenant".to_owned()),
                Value::Text(self.scope.tenant.as_str().to_owned()),
            ),
            (
                Value::Text("owner_space".to_owned()),
                Value::Bytes(self.scope.owner_space.to_bytes()),
            ),
            (
                Value::Text("target".to_owned()),
                Term::Object(self.scope.target).to_value()?,
            ),
            (
                Value::Text("name".to_owned()),
                Value::Text(self.scope.name.clone()),
            ),
            (
                Value::Text("kind".to_owned()),
                Value::Text(self.kind.clone()),
            ),
        ];
        if let Some(schema) = self.schema_ref {
            entries.push((
                Value::Text("schema".to_owned()),
                Term::Object(schema).to_value()?,
            ));
        }
        entries.extend([
            (
                Value::Text("audience".to_owned()),
                Value::Text(self.audience.as_wire().to_owned()),
            ),
            (
                Value::Text("append_mode".to_owned()),
                Value::Text(self.append_mode.as_wire().to_owned()),
            ),
            (
                Value::Text("admission".to_owned()),
                Value::Text(self.admission_rule.as_wire().to_owned()),
            ),
            (
                Value::Text("retention_policy".to_owned()),
                Value::Text(self.retention_policy_id.clone()),
            ),
            (
                Value::Text("revision".to_owned()),
                Value::UInt(self.revision),
            ),
        ]);
        Ok(Value::Map(entries))
    }

    /// The exact payload bytes the envelope carries.
    ///
    /// # Errors
    ///
    /// As [`AttributeDefinition::to_value`], or an encoding error.
    pub fn payload_bytes(&self) -> Result<Vec<u8>, AttributeError> {
        Ok(encode_canonical(&self.to_value()?)?)
    }

    /// Reads a definition from its exact payload bytes.
    ///
    /// # Errors
    ///
    /// [`AttributeError`] for non-canonical bytes, an unknown or
    /// missing field, another schema version, a malformed scope or a
    /// policy value outside its vocabulary.
    pub fn from_payload(bytes: &[u8]) -> Result<Self, AttributeError> {
        let value = decode_canonical(bytes)?;
        let Value::Map(entries) = &value else {
            return Err(field("definition", "must be a map"));
        };
        for (key, _) in entries {
            let Value::Text(name) = key else {
                return Err(field("definition", "keys must be text"));
            };
            if !FIELDS.contains(&name.as_str()) {
                return Err(AttributeError::Cbor(CborError::UnknownCriticalField {
                    field: name.clone(),
                }));
            }
        }
        match lookup(entries, "v") {
            Some(Value::UInt(SCHEMA_VERSION)) => {}
            Some(Value::UInt(other)) => return Err(AttributeError::UnsupportedSchema(*other)),
            Some(_) => return Err(field("v", "must be an unsigned integer")),
            None => return Err(field("v", "is missing")),
        }
        let tenant = match lookup(entries, "tenant") {
            Some(Value::Text(text)) => TenantId::parse(text)?,
            Some(_) => return Err(field("tenant", "must be text")),
            None => return Err(field("tenant", "is missing")),
        };
        let owner_space = match lookup(entries, "owner_space") {
            Some(Value::Bytes(bytes)) => SpaceAddress::from_bytes(bytes)?,
            Some(_) => return Err(field("owner_space", "must be bytes")),
            None => return Err(field("owner_space", "is missing")),
        };
        let target = match lookup(entries, "target") {
            Some(value) => match Term::from_value(value)? {
                Term::Object(reference) => reference,
                Term::Typed { .. } => {
                    return Err(field("target", "must be an object reference"));
                }
            },
            None => return Err(field("target", "is missing")),
        };
        let name = text_field(entries, "name")?;
        let kind = text_field(entries, "kind")?;
        let schema_ref = match lookup(entries, "schema") {
            None => None,
            Some(value) => match Term::from_value(value)? {
                Term::Object(reference) => Some(reference),
                Term::Typed { .. } => {
                    return Err(field("schema", "must be an object reference"));
                }
            },
        };
        let audience = match lookup(entries, "audience") {
            Some(Value::Text(text)) => Audience::from_wire(text)
                .ok_or_else(|| field("audience", "is not a known audience"))?,
            Some(_) => return Err(field("audience", "must be text")),
            None => return Err(field("audience", "is missing")),
        };
        let append_mode = match lookup(entries, "append_mode") {
            Some(Value::Text(text)) => AppendMode::from_wire(text)
                .ok_or_else(|| field("append_mode", "is not a known append mode"))?,
            Some(_) => return Err(field("append_mode", "must be text")),
            None => return Err(field("append_mode", "is missing")),
        };
        let admission_rule = match lookup(entries, "admission") {
            Some(Value::Text(text)) => AdmissionRule::from_wire(text)
                .ok_or_else(|| field("admission", "is not a known admission rule"))?,
            Some(_) => return Err(field("admission", "must be text")),
            None => return Err(field("admission", "is missing")),
        };
        let retention_policy_id = text_field(entries, "retention_policy")?;
        let revision = match lookup(entries, "revision") {
            Some(Value::UInt(revision)) => *revision,
            Some(_) => return Err(field("revision", "must be an unsigned integer")),
            None => return Err(field("revision", "is missing")),
        };
        let definition = Self {
            scope: AttributeScope {
                tenant,
                owner_space,
                target,
                name,
            },
            kind,
            schema_ref,
            audience,
            append_mode,
            admission_rule,
            retention_policy_id,
            revision,
        };
        definition.validate()?;
        Ok(definition)
    }

    /// Rejects a definition that names `own` envelope or the payload
    /// it is carried as. An ID never appears in its own preimage.
    ///
    /// # Errors
    ///
    /// [`AttributeError::SelfReference`].
    pub fn check_not_self_referential(
        &self,
        own: &EnvelopeId,
        payload: &[u8],
    ) -> Result<(), AttributeError> {
        let own_payload: Vec<ContentId> = HashProfile::ALL
            .iter()
            .filter_map(|profile| ContentId::of_bytes(*profile, payload).ok())
            .collect();
        for reference in self.references() {
            let id = match reference {
                ObjectRef::Envelope(id) => *id.content_id(),
                ObjectRef::Content(id) => id,
            };
            if id == *own.content_id() || own_payload.contains(&id) {
                return Err(AttributeError::SelfReference);
            }
        }
        Ok(())
    }

    /// Signs the definition as a new envelope under
    /// [`ATTRIBUTE_DEFINITION_DOMAIN`], with the tenant ID as the
    /// envelope context. `predecessor` names the definition this one
    /// revises, for a policy edit; the first definition of a scope
    /// passes `None`.
    ///
    /// # Errors
    ///
    /// A payload or envelope rejection, or
    /// [`AttributeError::SelfReference`].
    pub fn sign(
        &self,
        signer_id: &str,
        key: &SigningKey,
        predecessor: Option<EnvelopeId>,
    ) -> Result<OpenedDefinition, AttributeError> {
        let payload = self.payload_bytes()?;
        let body = Body {
            domain: ATTRIBUTE_DEFINITION_DOMAIN.to_owned(),
            context: self.scope.tenant.as_str().to_owned(),
            subject: None,
            graph: None,
            predecessor,
            signer_id: signer_id.to_owned(),
            key: key.verifying_key().to_bytes(),
            suite: SUITE_ED25519.to_owned(),
            payload_id: PayloadId::of(DEFINITION_ID_PROFILE, &payload)
                .map_err(EnvelopeError::from)?,
            payload_length: payload.len() as u64,
        };
        let envelope = Envelope::sign(body, payload, key)?;
        let id = envelope.id(DEFINITION_ID_PROFILE)?;
        self.check_not_self_referential(&id, &envelope.payload)?;
        Ok(OpenedDefinition {
            id,
            definition: self.clone(),
            envelope,
        })
    }
}

/// A definition whose envelope verified and whose payload decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedDefinition {
    /// The definition's identity: its envelope ID under
    /// [`DEFINITION_ID_PROFILE`].
    pub id: EnvelopeId,
    /// The decoded payload.
    pub definition: AttributeDefinition,
    /// The verified envelope, kept byte for byte.
    pub envelope: Envelope,
}

/// Verifies `envelope` as an attribute definition and decodes its
/// payload.
///
/// # Errors
///
/// [`AttributeError`]: a wrong domain, an envelope that does not
/// verify, a bad payload, an envelope context naming a different
/// tenant than the payload, or a self-reference.
pub fn open(
    envelope: Envelope,
    dispatch: &ProfileDispatch,
) -> Result<OpenedDefinition, AttributeError> {
    if envelope.body.domain != ATTRIBUTE_DEFINITION_DOMAIN {
        return Err(AttributeError::WrongDomain(envelope.body.domain));
    }
    envelope.verify(dispatch)?;
    let definition = AttributeDefinition::from_payload(&envelope.payload)?;
    if envelope.body.context != definition.scope.tenant.as_str() {
        return Err(AttributeError::TenantMismatch);
    }
    let id = envelope.id(DEFINITION_ID_PROFILE)?;
    definition.check_not_self_referential(&id, &envelope.payload)?;
    Ok(OpenedDefinition {
        id,
        definition,
        envelope,
    })
}

/// The operations a tenant authority decides for attribute
/// definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityOp {
    /// Create the first definition of a scope.
    Define,
    /// Replace a definition's policy with its next revision.
    EditPolicy,
}

/// The tenant-scope authorization boundary owned by #9083, consumed
/// here and nowhere implemented: tenancy decides who may act in a
/// tenant and which references are deliberately shared across
/// tenants.
pub trait TenantAuthority {
    /// Whether `principal` may perform `op` in `tenant`.
    fn authorize(&self, tenant: &TenantId, principal: &VerifyingKey, op: AuthorityOp) -> bool;

    /// The home tenant of a referenced object, when the authority
    /// knows it. `None` leaves the reference unplaced; a known home
    /// in another tenant is a cross-tenant reference.
    fn tenant_of_reference(&self, reference: &ObjectRef) -> Option<TenantId>;

    /// Whether a reference from `from` into `to` is a deliberate
    /// sharing decision recorded by the tenancy authority. The
    /// default is deny: cross-tenant references are denied unless
    /// #9083's authority says otherwise.
    fn allows_shared_reference(&self, from: &TenantId, to: &TenantId) -> bool {
        let _ = (from, to);
        false
    }
}

/// Checks an opened definition against the tenant authority: its
/// signer must be authorized in its tenant for the operation its
/// revision implies (define for revision 1, edit for any later
/// revision seen without its predecessor), and none of its references
/// may reach into another tenant without a recorded sharing decision.
///
/// # Errors
///
/// [`AttributeError::TenantDenied`],
/// [`AttributeError::UnauthorizedPolicyEdit`] or
/// [`AttributeError::CrossTenantReference`].
pub fn authorize_definition(
    authority: &dyn TenantAuthority,
    opened: &OpenedDefinition,
) -> Result<(), AttributeError> {
    let definition = &opened.definition;
    let op = if definition.revision == 1 && opened.envelope.body.predecessor.is_none() {
        AuthorityOp::Define
    } else {
        AuthorityOp::EditPolicy
    };
    let signer = signer_key(opened)?;
    if !authority.authorize(&definition.scope.tenant, &signer, op) {
        return Err(match op {
            AuthorityOp::Define => AttributeError::TenantDenied,
            AuthorityOp::EditPolicy => AttributeError::UnauthorizedPolicyEdit,
        });
    }
    check_references(authority, definition)
}

/// Checks `next` as the policy edit replacing `current`: same scope,
/// exactly the next revision, `current` named as the envelope
/// predecessor, a signer the tenant authority authorizes for policy
/// edits, and no unshared cross-tenant reference. This is the check
/// S5.22 and S5.23 name: submitters cannot edit the policy; the owner
/// and group admins can.
///
/// # Errors
///
/// [`AttributeError::ScopeChanged`],
/// [`AttributeError::RevisionMismatch`],
/// [`AttributeError::MissingPredecessor`],
/// [`AttributeError::UnauthorizedPolicyEdit`] or
/// [`AttributeError::CrossTenantReference`].
pub fn check_policy_revision(
    authority: &dyn TenantAuthority,
    current: &OpenedDefinition,
    next: &OpenedDefinition,
) -> Result<(), AttributeError> {
    if next.definition.scope != current.definition.scope {
        return Err(AttributeError::ScopeChanged);
    }
    let expected = current
        .definition
        .revision
        .checked_add(1)
        .ok_or(AttributeError::RevisionMismatch)?;
    if next.definition.revision != expected {
        return Err(AttributeError::RevisionMismatch);
    }
    if next.envelope.body.predecessor != Some(current.id) {
        return Err(AttributeError::MissingPredecessor);
    }
    let signer = signer_key(next)?;
    if !authority.authorize(
        &next.definition.scope.tenant,
        &signer,
        AuthorityOp::EditPolicy,
    ) {
        return Err(AttributeError::UnauthorizedPolicyEdit);
    }
    check_references(authority, &next.definition)
}

fn check_references(
    authority: &dyn TenantAuthority,
    definition: &AttributeDefinition,
) -> Result<(), AttributeError> {
    for reference in definition.references() {
        if let Some(home) = authority.tenant_of_reference(&reference) {
            if home != definition.scope.tenant
                && !authority.allows_shared_reference(&definition.scope.tenant, &home)
            {
                return Err(AttributeError::CrossTenantReference);
            }
        }
    }
    Ok(())
}

fn signer_key(opened: &OpenedDefinition) -> Result<VerifyingKey, AttributeError> {
    VerifyingKey::from_bytes(&opened.envelope.body.key)
        .map_err(|_error| AttributeError::InvalidSignerKey)
}

/// Why an attribute definition was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributeError {
    /// The payload is not canonical CBOR, or has an unknown field.
    Cbor(CborError),
    /// A term was rejected.
    Term(TermError),
    /// The envelope was rejected.
    Envelope(EnvelopeError),
    /// The owner space address was rejected.
    Space(SpaceError),
    /// A field is missing or has the wrong type or size.
    Field {
        /// The field name.
        field: &'static str,
        /// What was wrong.
        problem: &'static str,
    },
    /// The payload names a schema version this reader does not know.
    UnsupportedSchema(u64),
    /// The scope (tenant, space, target, name) is not well-formed.
    MalformedScope(&'static str),
    /// The envelope was signed for another domain.
    WrongDomain(String),
    /// The envelope context and the payload name different tenants.
    TenantMismatch,
    /// The signer is not authorized in the definition's tenant.
    TenantDenied,
    /// The signer is not authorized to edit this attribute's policy.
    UnauthorizedPolicyEdit,
    /// A reference reaches into another tenant without a recorded
    /// sharing decision.
    CrossTenantReference,
    /// A policy edit tried to move the attribute's scope.
    ScopeChanged,
    /// A policy edit does not carry exactly the next revision.
    RevisionMismatch,
    /// A policy edit does not name the definition it revises.
    MissingPredecessor,
    /// The definition references its own envelope or payload.
    SelfReference,
    /// The envelope's signer key is not a valid Ed25519 key.
    InvalidSignerKey,
}

impl AttributeError {
    /// A stable failure class: `malformed`, `unsupported`,
    /// `authorization`, `conflict`, `self_reference`, or a wrapped
    /// error's own class.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Cbor(_)
            | Self::Field { .. }
            | Self::WrongDomain(_)
            | Self::TenantMismatch
            | Self::MalformedScope(_)
            | Self::InvalidSignerKey => "malformed",
            Self::Term(error) => error.class(),
            Self::Envelope(error) => error.class(),
            Self::Space(error) => error.class(),
            Self::UnsupportedSchema(_) => "unsupported",
            Self::TenantDenied | Self::UnauthorizedPolicyEdit | Self::CrossTenantReference => {
                "authorization"
            }
            Self::ScopeChanged | Self::RevisionMismatch | Self::MissingPredecessor => "conflict",
            Self::SelfReference => "self_reference",
        }
    }
}

impl fmt::Display for AttributeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cbor(error) => write!(f, "attribute definition encoding: {error}"),
            Self::Term(error) => write!(f, "attribute definition term: {error}"),
            Self::Envelope(error) => write!(f, "attribute definition envelope: {error}"),
            Self::Space(error) => write!(f, "attribute definition space: {error}"),
            Self::Field { field, problem } => write!(f, "field {field}: {problem}"),
            Self::UnsupportedSchema(version) => {
                write!(f, "unsupported attribute definition schema {version}")
            }
            Self::MalformedScope(problem) => write!(f, "malformed scope: {problem}"),
            Self::WrongDomain(domain) => write!(f, "signed for domain {domain:?}"),
            Self::TenantMismatch => {
                write!(f, "envelope context and payload name different tenants")
            }
            Self::TenantDenied => write!(f, "signer not authorized in the tenant"),
            Self::UnauthorizedPolicyEdit => {
                write!(f, "signer not authorized to edit attribute policy")
            }
            Self::CrossTenantReference => {
                write!(f, "reference reaches into another tenant without sharing")
            }
            Self::ScopeChanged => write!(f, "policy edit changed the attribute scope"),
            Self::RevisionMismatch => write!(f, "policy edit is not the next revision"),
            Self::MissingPredecessor => {
                write!(f, "policy edit does not name the definition it revises")
            }
            Self::SelfReference => write!(f, "definition references its own identity"),
            Self::InvalidSignerKey => write!(f, "signer key is not a valid Ed25519 key"),
        }
    }
}

impl std::error::Error for AttributeError {}

impl From<CborError> for AttributeError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

impl From<TermError> for AttributeError {
    fn from(error: TermError) -> Self {
        Self::Term(error)
    }
}

impl From<EnvelopeError> for AttributeError {
    fn from(error: EnvelopeError) -> Self {
        Self::Envelope(error)
    }
}

impl From<SpaceError> for AttributeError {
    fn from(error: SpaceError) -> Self {
        Self::Space(error)
    }
}

const fn field(field: &'static str, problem: &'static str) -> AttributeError {
    AttributeError::Field { field, problem }
}

fn lookup<'a>(entries: &'a [(Value, Value)], name: &str) -> Option<&'a Value> {
    entries
        .iter()
        .find(|(key, _)| matches!(key, Value::Text(key) if key == name))
        .map(|(_, value)| value)
}

fn text_field(entries: &[(Value, Value)], name: &'static str) -> Result<String, AttributeError> {
    match lookup(entries, name) {
        Some(Value::Text(text)) => Ok(text.clone()),
        Some(_) => Err(field(name, "must be text")),
        None => Err(field(name, "is missing")),
    }
}
