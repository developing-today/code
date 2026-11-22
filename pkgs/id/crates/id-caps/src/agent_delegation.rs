//! Agent identity, role binding and UCAN-style attenuated delegation
//! (spike 3/5 for #6300, issue #7915).
//!
//! Oracle: `doc/2026-11-09_design_id6300_agents_in_worlds_v1.md` §5
//! (identity, roles, delegation), §9.2 (lifetime: 1 h default, roles may
//! set 15 min – 4 h, renewal by the owner's client only) and §9.4
//! (hosted edge: OAuth scope strings name world + role; the role is the
//! authority of record, so a valid transport token with no matching
//! role binding grants nothing). Audit events follow the §7 shape:
//! agent key, owner, world, tool, capability proof reference, outcome —
//! refusals included, never sampled away.
//!
//! The model is deliberately offline-verifiable: a [`RoleBinding`] and a
//! [`Delegation`] are Ed25519-signed canonical byte strings, and
//! [`verify_join`] re-derives every check from the two signatures alone.
//! The host never trusts the bridge's word for the chain (§5.2).

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use sha2::{Digest as _, Sha256};

use crate::world_scopes::WorldScopes;

/// Default delegation lifetime: one hour (§5.3, §9.2).
pub const DEFAULT_LIFETIME_SECS: u64 = 3_600;
/// Shortest lifetime a role may set: 15 minutes (§9.2).
pub const MIN_LIFETIME_SECS: u64 = 900;
/// Longest lifetime a role may set: 4 hours (§9.2). Longer requires an
/// owner-side policy change, not a role edit.
pub const MAX_LIFETIME_SECS: u64 = 14_400;
/// Delegations are depth 1 in v1: owner → agent, never further (§5.2).
pub const DELEGATION_DEPTH: u8 = 1;

/// An agent's identity: its own Ed25519 public key (§5.1). On iroh the
/// endpoint ID *is* this key; the key alone grants nothing — authority
/// comes only from a binding plus a delegation chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentId(pub [u8; 32]);

impl AgentId {
    /// The agent identity carried by a verifying key.
    #[must_use]
    pub fn from_verifying_key(key: &VerifyingKey) -> Self {
        Self(key.to_bytes())
    }

    /// The verifying key for this identity.
    ///
    /// # Errors
    ///
    /// Returns [`DelegationRefusal::BadKey`] if the bytes are not a valid
    /// Ed25519 point.
    pub fn verifying_key(&self) -> Result<VerifyingKey, DelegationRefusal> {
        VerifyingKey::from_bytes(&self.0).map_err(|_| DelegationRefusal::BadKey)
    }

    /// Lowercase hex encoding, as journaled.
    #[must_use]
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// A role: a named bundle of a capability ceiling plus the worlds it
/// applies to (§5.2), with its delegation lifetime (§9.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Role {
    /// Stable role name, as carried in hosted-edge scope strings (§9.4).
    pub name: String,
    /// The most any delegation under this role may ever carry.
    pub ceiling: WorldScopes,
    /// Worlds this role applies to.
    pub worlds: Vec<String>,
    /// Delegation lifetime in seconds, within
    /// [`MIN_LIFETIME_SECS`]..=[`MAX_LIFETIME_SECS`].
    pub lifetime_secs: u64,
}

impl Role {
    /// A role with the default one-hour lifetime.
    ///
    /// # Errors
    ///
    /// Returns [`DelegationRefusal::LifetimeOutOfBounds`] if
    /// `lifetime_secs` is outside the §9.2 range.
    pub fn new(
        name: impl Into<String>,
        ceiling: WorldScopes,
        worlds: Vec<String>,
        lifetime_secs: u64,
    ) -> Result<Self, DelegationRefusal> {
        if !(MIN_LIFETIME_SECS..=MAX_LIFETIME_SECS).contains(&lifetime_secs) {
            return Err(DelegationRefusal::LifetimeOutOfBounds);
        }
        Ok(Self {
            name: name.into(),
            ceiling,
            worlds,
            lifetime_secs,
        })
    }

    /// Whether this role applies to `world`.
    #[must_use]
    pub fn applies_to(&self, world: &str) -> bool {
        self.worlds.iter().any(|w| w == world)
    }
}

/// The owner's signed binding of an agent key to a role in one world
/// (§5.1, §5.2). Without this record a delegation chain anchors nowhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleBinding {
    /// World the binding applies to.
    pub world: String,
    /// Role name (see [`Role::name`]).
    pub role: String,
    /// The bound agent key.
    pub agent: AgentId,
    /// The owner's public key bytes.
    pub owner: [u8; 32],
    /// The role's capability ceiling at bind time.
    pub ceiling: WorldScopes,
    /// Owner signature over [`RoleBinding::canonical_bytes`].
    pub signature: [u8; 64],
}

impl RoleBinding {
    /// Bind `agent` to `role` in `world`, signed by the owner's key.
    ///
    /// # Errors
    ///
    /// Returns [`DelegationRefusal::RoleNotInWorld`] if the role does not
    /// apply to `world`.
    pub fn bind(
        owner_key: &SigningKey,
        world: &str,
        role: &Role,
        agent: AgentId,
    ) -> Result<Self, DelegationRefusal> {
        if !role.applies_to(world) {
            return Err(DelegationRefusal::RoleNotInWorld);
        }
        let mut binding = Self {
            world: world.to_owned(),
            role: role.name.clone(),
            agent,
            owner: owner_key.verifying_key().to_bytes(),
            ceiling: role.ceiling,
            signature: [0u8; 64],
        };
        binding.signature = owner_key.sign(&binding.canonical_bytes()).to_bytes();
        Ok(binding)
    }

    /// Canonical signed bytes: length-prefixed fields in a fixed order.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        push_field(&mut out, b"role-binding/v1");
        push_field(&mut out, self.world.as_bytes());
        push_field(&mut out, self.role.as_bytes());
        push_field(&mut out, &self.agent.0);
        push_field(&mut out, &self.owner);
        push_field(&mut out, &self.ceiling.bits().to_be_bytes());
        out
    }

    /// Verify the owner's signature over this binding.
    ///
    /// # Errors
    ///
    /// Returns [`DelegationRefusal::BadBindingSignature`] if the
    /// signature does not verify against the recorded owner key.
    pub fn verify_signature(&self) -> Result<(), DelegationRefusal> {
        let key = VerifyingKey::from_bytes(&self.owner).map_err(|_| DelegationRefusal::BadKey)?;
        key.verify(
            &self.canonical_bytes(),
            &Signature::from_bytes(&self.signature),
        )
        .map_err(|_| DelegationRefusal::BadBindingSignature)
    }
}

/// A UCAN-style delegation from the owner to the agent's key (§5.2):
/// attenuated (a subset of the role ceiling), expiring, depth 1, and
/// verifiable offline from the signature alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delegation {
    /// Issuer public key bytes — must equal the binding's owner.
    pub issuer: [u8; 32],
    /// The agent key the capabilities are delegated to.
    pub agent: AgentId,
    /// World the delegation applies to.
    pub world: String,
    /// Role name the delegation is issued under.
    pub role: String,
    /// Delegated scopes: a subset of the binding ceiling, never wider.
    pub scopes: WorldScopes,
    /// Issuance time, unix seconds.
    pub issued_at: u64,
    /// Expiry time, unix seconds. At or after this instant the token
    /// fails verification (§5.3).
    pub expires_at: u64,
    /// Delegation depth; v1 accepts exactly [`DELEGATION_DEPTH`].
    pub depth: u8,
    /// Issuer signature over [`Delegation::canonical_bytes`].
    pub signature: [u8; 64],
}

impl Delegation {
    /// Issue a delegation under `binding`, signed by the owner's key.
    /// `scopes` must attenuate the binding ceiling; the lifetime is the
    /// role lifetime passed in `lifetime_secs` (§9.2).
    ///
    /// # Errors
    ///
    /// Returns [`DelegationRefusal::NotOwner`] if `owner_key` is not the
    /// binding's owner, [`DelegationRefusal::WidenedBeyondCeiling`] if
    /// `scopes` exceeds the ceiling, or
    /// [`DelegationRefusal::LifetimeOutOfBounds`] for a lifetime outside
    /// the §9.2 range.
    pub fn issue(
        owner_key: &SigningKey,
        binding: &RoleBinding,
        scopes: WorldScopes,
        now: u64,
        lifetime_secs: u64,
    ) -> Result<Self, DelegationRefusal> {
        if owner_key.verifying_key().to_bytes() != binding.owner {
            return Err(DelegationRefusal::NotOwner);
        }
        if !binding.ceiling.implied().contains(scopes) {
            return Err(DelegationRefusal::WidenedBeyondCeiling);
        }
        if !(MIN_LIFETIME_SECS..=MAX_LIFETIME_SECS).contains(&lifetime_secs) {
            return Err(DelegationRefusal::LifetimeOutOfBounds);
        }
        let mut delegation = Self {
            issuer: binding.owner,
            agent: binding.agent,
            world: binding.world.clone(),
            role: binding.role.clone(),
            scopes,
            issued_at: now,
            expires_at: now.saturating_add(lifetime_secs),
            depth: DELEGATION_DEPTH,
            signature: [0u8; 64],
        };
        delegation.signature = owner_key.sign(&delegation.canonical_bytes()).to_bytes();
        Ok(delegation)
    }

    /// Renew this delegation. Renewal is the **owner's client's** act
    /// (§5.3, §9.2): the host must not be able to renew access to itself,
    /// and the agent cannot renew its own token — stopping renewal *is*
    /// revocation, with latency bounded by the token lifetime.
    ///
    /// # Errors
    ///
    /// Returns [`DelegationRefusal::NotOwner`] unless `renewing_key` is
    /// the binding's owner key; the other [`Delegation::issue`] checks
    /// apply unchanged.
    pub fn renew(
        &self,
        binding: &RoleBinding,
        renewing_key: &SigningKey,
        now: u64,
        lifetime_secs: u64,
    ) -> Result<Self, DelegationRefusal> {
        if renewing_key.verifying_key().to_bytes() != binding.owner
            || renewing_key.verifying_key().to_bytes() != self.issuer
        {
            return Err(DelegationRefusal::NotOwner);
        }
        Self::issue(renewing_key, binding, self.scopes, now, lifetime_secs)
    }

    /// Canonical signed bytes: length-prefixed fields in a fixed order.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        push_field(&mut out, b"delegation/v1");
        push_field(&mut out, &self.issuer);
        push_field(&mut out, &self.agent.0);
        push_field(&mut out, self.world.as_bytes());
        push_field(&mut out, self.role.as_bytes());
        push_field(&mut out, &self.scopes.bits().to_be_bytes());
        push_field(&mut out, &self.issued_at.to_be_bytes());
        push_field(&mut out, &self.expires_at.to_be_bytes());
        push_field(&mut out, &self.depth.to_be_bytes());
        out
    }

    /// The §7 capability proof reference: a digest of the signed token a
    /// journal entry can cite without carrying the token itself.
    #[must_use]
    pub fn proof_ref(&self) -> String {
        let digest = Sha256::digest(self.canonical_bytes());
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Authority derived at join: the agent may act in `world` under `role`
/// with exactly `scopes` until `expires_at`. Derived from the role
/// binding and the delegation chain — never from a transport token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinGrant {
    /// The joining agent.
    pub agent: AgentId,
    /// The owner the authority derives from.
    pub owner: [u8; 32],
    /// World joined.
    pub world: String,
    /// Role in force.
    pub role: String,
    /// Effective scopes (the delegation's scopes).
    pub scopes: WorldScopes,
    /// When the grant lapses (the delegation's expiry).
    pub expires_at: u64,
    /// §7 capability proof reference of the delegation used.
    pub proof_ref: String,
}

/// One §7 audit event for a join attempt, granted or refused. Refusals
/// are the highest-signal events and are always recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditEvent {
    /// Agent key attempting the join (hex).
    pub agent: String,
    /// Owner key the authority would derive from (hex).
    pub owner: String,
    /// World attempted.
    pub world: String,
    /// Tool surface: always `world.join` for this spike.
    pub tool: &'static str,
    /// Capability proof reference, when a delegation was presented.
    pub proof_ref: Option<String>,
    /// `granted`, or `refused:<code>`.
    pub outcome: String,
}

/// Why a delegation, binding or join was refused. Each variant maps to a
/// stable machine-readable [`DelegationRefusal::code`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DelegationRefusal {
    /// Key bytes are not a valid Ed25519 point.
    BadKey,
    /// The binding's signature does not verify.
    BadBindingSignature,
    /// The delegation's signature does not verify.
    BadDelegationSignature,
    /// The signing key is not the binding's owner (renewal and issuance
    /// are owner-only acts, §9.2).
    NotOwner,
    /// The delegation's issuer is not the binding's owner.
    IssuerNotOwner,
    /// The role does not apply to the world.
    RoleNotInWorld,
    /// Binding and delegation name different worlds.
    WorldMismatch,
    /// Binding and delegation name different roles.
    RoleMismatch,
    /// Binding and delegation name different agents.
    AgentMismatch,
    /// No delegation was presented at all (§5.2: join without a valid
    /// chain is refused).
    NoDelegation,
    /// The delegation has expired (§5.3).
    Expired,
    /// The delegation's scopes exceed the role ceiling (§5.2, §4.5.1:
    /// a token can never carry a capability the role does not hold).
    WidenedBeyondCeiling,
    /// Depth other than 1: an agent attempted to re-delegate (§5.2).
    Redelegated,
    /// Lifetime outside the §9.2 range of 15 min – 4 h.
    LifetimeOutOfBounds,
    /// Hosted edge: transport token valid, but no role binding exists
    /// (§5.4, §9.4 — transport success never mints authority).
    NoRoleBinding,
    /// Hosted edge: scope string is not `world:<id> role:<name>` (§9.4).
    BadEdgeScopes,
}

impl DelegationRefusal {
    /// Stable machine-readable refusal code, as journaled in §7 events.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::BadKey => "bad_key",
            Self::BadBindingSignature => "bad_binding_signature",
            Self::BadDelegationSignature => "bad_delegation_signature",
            Self::NotOwner => "not_owner",
            Self::IssuerNotOwner => "issuer_not_owner",
            Self::RoleNotInWorld => "role_not_in_world",
            Self::WorldMismatch => "world_mismatch",
            Self::RoleMismatch => "role_mismatch",
            Self::AgentMismatch => "agent_mismatch",
            Self::NoDelegation => "no_delegation",
            Self::Expired => "expired",
            Self::WidenedBeyondCeiling => "widened_beyond_ceiling",
            Self::Redelegated => "redelegated",
            Self::LifetimeOutOfBounds => "lifetime_out_of_bounds",
            Self::NoRoleBinding => "no_role_binding",
            Self::BadEdgeScopes => "bad_edge_scopes",
        }
    }
}

/// Verify a join attempt from first principles (§5.2): the binding must
/// be owner-signed, the delegation must be owner-issued for the same
/// binding, unexpired, attenuated to the ceiling, and depth 1.
///
/// # Errors
///
/// Returns the specific [`DelegationRefusal`] for the first failed
/// check. Every failure is a refusal, never a partial grant.
pub fn verify_join(
    binding: &RoleBinding,
    delegation: Option<&Delegation>,
    world: &str,
    now: u64,
) -> Result<JoinGrant, DelegationRefusal> {
    binding.verify_signature()?;
    if binding.world != world {
        return Err(DelegationRefusal::WorldMismatch);
    }
    let delegation = delegation.ok_or(DelegationRefusal::NoDelegation)?;
    if delegation.issuer != binding.owner {
        return Err(DelegationRefusal::IssuerNotOwner);
    }
    if delegation.depth != DELEGATION_DEPTH {
        return Err(DelegationRefusal::Redelegated);
    }
    if delegation.world != binding.world {
        return Err(DelegationRefusal::WorldMismatch);
    }
    if delegation.role != binding.role {
        return Err(DelegationRefusal::RoleMismatch);
    }
    if delegation.agent != binding.agent {
        return Err(DelegationRefusal::AgentMismatch);
    }
    let issuer_key =
        VerifyingKey::from_bytes(&delegation.issuer).map_err(|_| DelegationRefusal::BadKey)?;
    issuer_key
        .verify(
            &delegation.canonical_bytes(),
            &Signature::from_bytes(&delegation.signature),
        )
        .map_err(|_| DelegationRefusal::BadDelegationSignature)?;
    if !binding.ceiling.implied().contains(delegation.scopes) {
        return Err(DelegationRefusal::WidenedBeyondCeiling);
    }
    if now >= delegation.expires_at {
        return Err(DelegationRefusal::Expired);
    }
    Ok(JoinGrant {
        agent: delegation.agent,
        owner: binding.owner,
        world: binding.world.clone(),
        role: binding.role.clone(),
        scopes: delegation.scopes,
        expires_at: delegation.expires_at,
        proof_ref: delegation.proof_ref(),
    })
}

/// [`verify_join`] plus the §7 audit event for the attempt — the shape
/// the host's policy point calls, so a refusal is recorded with the same
/// fidelity as a grant.
#[must_use]
pub fn verify_join_audited(
    binding: &RoleBinding,
    delegation: Option<&Delegation>,
    world: &str,
    now: u64,
) -> (Result<JoinGrant, DelegationRefusal>, AuditEvent) {
    let result = verify_join(binding, delegation, world, now);
    let outcome = match &result {
        Ok(_) => "granted".to_owned(),
        Err(refusal) => format!("refused:{}", refusal.code()),
    };
    let event = AuditEvent {
        agent: binding.agent.to_hex(),
        owner: binding.owner.iter().map(|b| format!("{b:02x}")).collect(),
        world: world.to_owned(),
        tool: "world.join",
        proof_ref: delegation.map(Delegation::proof_ref),
        outcome,
    };
    (result, event)
}

/// Hosted-edge transport scopes (§5.4, §9.4): the OAuth scope string
/// `world:<id> role:<name>`. This is a *transport representation* only —
/// see [`authorize_edge`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdgeScopes {
    /// World named by the transport token.
    pub world: String,
    /// Role named by the transport token.
    pub role: String,
}

impl EdgeScopes {
    /// Parse a `world:<id> role:<name>` scope string.
    ///
    /// # Errors
    ///
    /// Returns [`DelegationRefusal::BadEdgeScopes`] for any other shape.
    pub fn parse(scopes: &str) -> Result<Self, DelegationRefusal> {
        let mut world = None;
        let mut role = None;
        for part in scopes.split_whitespace() {
            if let Some(value) = part.strip_prefix("world:") {
                world = Some(value.to_owned());
            } else if let Some(value) = part.strip_prefix("role:") {
                role = Some(value.to_owned());
            } else {
                return Err(DelegationRefusal::BadEdgeScopes);
            }
        }
        match (world, role) {
            (Some(world), Some(role)) if !world.is_empty() && !role.is_empty() => {
                Ok(Self { world, role })
            }
            _ => Err(DelegationRefusal::BadEdgeScopes),
        }
    }
}

/// Authorize a hosted-edge join (§5.4, §9.4): the edge has already
/// validated the transport token; authority is then re-derived from the
/// role binding and delegation chain. A valid transport token with no
/// matching role binding grants **nothing**.
///
/// # Errors
///
/// Returns [`DelegationRefusal::NoRoleBinding`] when no binding matches
/// the edge scopes, [`DelegationRefusal::BadEdgeScopes`] when the
/// binding names a different world or role than the token, or the
/// [`verify_join`] refusal for a bad chain.
pub fn authorize_edge(
    edge: &EdgeScopes,
    binding: Option<&RoleBinding>,
    delegation: Option<&Delegation>,
    now: u64,
) -> Result<JoinGrant, DelegationRefusal> {
    let binding = binding.ok_or(DelegationRefusal::NoRoleBinding)?;
    if binding.world != edge.world || binding.role != edge.role {
        return Err(DelegationRefusal::BadEdgeScopes);
    }
    verify_join(binding, delegation, &edge.world, now)
}

fn push_field(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u64).to_be_bytes());
    out.extend_from_slice(field);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const NOW: u64 = 1_700_000_000;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn fixture() -> (SigningKey, SigningKey, RoleBinding) {
        let owner = key(7);
        let agent = key(9);
        let role = Role::new(
            "archivist",
            WorldScopes::GUEST,
            vec!["atlas".to_owned()],
            DEFAULT_LIFETIME_SECS,
        )
        .expect("role within lifetime bounds");
        let binding = RoleBinding::bind(
            &owner,
            "atlas",
            &role,
            AgentId::from_verifying_key(&agent.verifying_key()),
        )
        .expect("role applies to world");
        (owner, agent, binding)
    }

    fn delegation(owner: &SigningKey, binding: &RoleBinding) -> Delegation {
        Delegation::issue(
            owner,
            binding,
            WorldScopes::JOIN.union(WorldScopes::CHAT),
            NOW,
            DEFAULT_LIFETIME_SECS,
        )
        .expect("attenuated issue")
    }

    /// Oracle §5.2 + §7: join without a delegation chain is refused, and
    /// the refusal is audited with the §7 event shape.
    #[test]
    fn join_without_delegation_is_refused_and_audited() {
        let (_owner, _agent, binding) = fixture();
        let (result, event) = verify_join_audited(&binding, None, "atlas", NOW);
        assert_eq!(result, Err(DelegationRefusal::NoDelegation));
        assert_eq!(event.outcome, "refused:no_delegation");
        assert_eq!(event.tool, "world.join");
        assert_eq!(event.world, "atlas");
        assert_eq!(event.proof_ref, None);
    }

    /// Oracle §5.2: the happy path — a valid, unexpired, attenuated,
    /// owner-issued chain grants exactly the delegated scopes.
    #[test]
    fn valid_chain_grants_delegated_scopes() {
        let (owner, _agent, binding) = fixture();
        let token = delegation(&owner, &binding);
        let grant = verify_join(&binding, Some(&token), "atlas", NOW).expect("valid chain grants");
        assert_eq!(grant.scopes, WorldScopes::JOIN.union(WorldScopes::CHAT));
        assert_eq!(grant.expires_at, NOW + DEFAULT_LIFETIME_SECS);
        assert_eq!(grant.proof_ref, token.proof_ref());
    }

    /// Oracle §5.2 + §4.5.1: a widened token fails verification even
    /// when the owner signed it — verification re-derives the ceiling
    /// check from the binding, and issuance refuses to mint it at all.
    #[test]
    fn widened_token_fails_verification() {
        let (owner, _agent, binding) = fixture();
        let wide = WorldScopes::GUEST.union(WorldScopes::DELEGATE);
        assert_eq!(
            Delegation::issue(&owner, &binding, wide, NOW, DEFAULT_LIFETIME_SECS),
            Err(DelegationRefusal::WidenedBeyondCeiling)
        );
        // Forge one by hand: owner signature, scopes beyond the ceiling.
        let mut forged = delegation(&owner, &binding);
        forged.scopes = wide;
        forged.signature = owner.sign(&forged.canonical_bytes()).to_bytes();
        assert_eq!(
            verify_join(&binding, Some(&forged), "atlas", NOW),
            Err(DelegationRefusal::WidenedBeyondCeiling)
        );
    }

    /// Oracle §5.2: depth is 1 — a token the agent re-delegates (signed
    /// by the agent key, depth 2) fails verification.
    #[test]
    fn redelegated_token_fails_verification() {
        let (owner, agent, binding) = fixture();
        let mut forged = delegation(&owner, &binding);
        forged.issuer = agent.verifying_key().to_bytes();
        forged.depth = 2;
        forged.signature = agent.sign(&forged.canonical_bytes()).to_bytes();
        assert_eq!(
            verify_join(&binding, Some(&forged), "atlas", NOW),
            Err(DelegationRefusal::IssuerNotOwner)
        );
        // Depth alone is disqualifying even under the owner's signature.
        let mut deep = delegation(&owner, &binding);
        deep.depth = 2;
        deep.signature = owner.sign(&deep.canonical_bytes()).to_bytes();
        assert_eq!(
            verify_join(&binding, Some(&deep), "atlas", NOW),
            Err(DelegationRefusal::Redelegated)
        );
    }

    /// Oracle §5.3: an expired token fails; the same token verifies one
    /// second before expiry.
    #[test]
    fn expired_token_fails() {
        let (owner, _agent, binding) = fixture();
        let token = delegation(&owner, &binding);
        assert!(verify_join(&binding, Some(&token), "atlas", token.expires_at - 1).is_ok());
        assert_eq!(
            verify_join(&binding, Some(&token), "atlas", token.expires_at),
            Err(DelegationRefusal::Expired)
        );
    }

    /// Oracle §5.3 + §9.2: renewal is owner-client-only. The agent
    /// cannot renew its own token; the owner's client can, and the
    /// renewed token verifies with a fresh expiry.
    #[test]
    fn renewal_is_owner_client_only() {
        let (owner, agent, binding) = fixture();
        let token = delegation(&owner, &binding);
        assert_eq!(
            token.renew(&binding, &agent, NOW + 100, DEFAULT_LIFETIME_SECS),
            Err(DelegationRefusal::NotOwner)
        );
        let renewed = token
            .renew(&binding, &owner, NOW + 100, DEFAULT_LIFETIME_SECS)
            .expect("owner renewal");
        assert_eq!(renewed.expires_at, NOW + 100 + DEFAULT_LIFETIME_SECS);
        assert!(verify_join(&binding, Some(&renewed), "atlas", NOW + 100).is_ok());
    }

    /// Oracle §9.2: lifetimes are bounded — roles cannot set 1 minute or
    /// 5 hours; the default is one hour.
    #[test]
    fn lifetime_bounds_are_enforced() {
        assert_eq!(
            Role::new("x", WorldScopes::GUEST, vec!["atlas".to_owned()], 60),
            Err(DelegationRefusal::LifetimeOutOfBounds)
        );
        assert_eq!(
            Role::new("x", WorldScopes::GUEST, vec!["atlas".to_owned()], 5 * 3_600),
            Err(DelegationRefusal::LifetimeOutOfBounds)
        );
        assert_eq!(DEFAULT_LIFETIME_SECS, 3_600);
    }

    /// Oracle §5.4 + §9.4: a valid transport token with no matching role
    /// binding grants nothing; with the binding and chain it grants
    /// exactly what the chain grants.
    #[test]
    fn hosted_edge_valid_token_without_role_binding_grants_nothing() {
        let (owner, _agent, binding) = fixture();
        let edge = EdgeScopes::parse("world:atlas role:archivist").expect("scope shape");
        assert_eq!(
            authorize_edge(&edge, None, None, NOW),
            Err(DelegationRefusal::NoRoleBinding)
        );
        let token = delegation(&owner, &binding);
        assert_eq!(
            authorize_edge(&edge, Some(&binding), None, NOW),
            Err(DelegationRefusal::NoDelegation)
        );
        let grant = authorize_edge(&edge, Some(&binding), Some(&token), NOW)
            .expect("binding + chain grants");
        assert_eq!(grant.role, "archivist");
    }
}
