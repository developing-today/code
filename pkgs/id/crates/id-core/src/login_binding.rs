//! Keyless-custody login binding and challenge verification
//! (#13710 slice 1; design of record: #5684 synthesis §§4, 5, 11 and the
//! #5885 feasibility record).
//!
//! The authorizer never holds the user's ID signing key. A login method
//! (OIDC or OPAQUE) only *authenticates an account*; it is an account
//! pointer, never key authority. What ties an ID public key to that
//! account is a domain-separated statement the client signs with its own,
//! client-held key:
//!
//! - [`BindingStatement`] commits the ID public key, the authentication
//!   account ([`AuthAccount`]: OIDC `(iss, sub)` with the subject hashed,
//!   or the OPAQUE account identifier), the custody profile, the recovery
//!   commitment hash, a generation counter and the hash of the previous
//!   statement. It is signed as `"id/login-binding/v1\0" ‖
//!   canonical_cbor(statement)`.
//! - [`BindingRegistry`] is the authorizer side. It stores the public key,
//!   the statement and the signatures — nothing else. Enrolment needs an
//!   authenticated session for the same account *and* a self-signature by
//!   the key being bound; any later change needs a signature from the
//!   currently bound key or a recovery-threshold proof (#5888). A fresh
//!   authenticated session never replaces a binding.
//! - [`ChallengeIssuer`] issues single-use, nonce/audience/expiry-bound
//!   challenges for ID-authorised operations (login acceptance and
//!   new-device enrolment); acceptance is a signature from the bound key.
//!   A device-enrolment challenge names the requested pseudo-login scope,
//!   so the signature covers it (composition with #5887 §4.3 and #5889).
//!   Verifying one does not mint a delegation: grants are #5889's artefact.
//!
//! The six #5885 invariants are the test oracles; tests named
//! `invariant_<n>_…` below check each one this slice covers. The OIDC
//! token validation and OPAQUE flows that produce an
//! [`AuthenticatedAccount`] are later slices of #13710.

use std::collections::BTreeMap;
use std::fmt;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use rand::RngExt as _;
use sha2::{Digest as _, Sha256};

use crate::canonical_cbor::{CborError, Value, decode_canonical, encode_canonical};

/// Signing domain of a login-binding statement.
pub const BINDING_DOMAIN: &str = "id/login-binding/v1";

/// Signing domain of a login challenge.
pub const CHALLENGE_DOMAIN: &str = "id/login-challenge/v1";

/// Domain that hashes an OIDC subject, so the authorizer's binding
/// records do not carry the provider's raw subject identifier.
const OIDC_SUBJECT_DOMAIN: &[u8] = b"id/login-binding/v1/oidc-subject\0";

/// The statement format version this module writes and reads.
pub const STATEMENT_VERSION: u64 = 1;

/// Upper bound on any text field (issuer, account id, audience, scope).
pub const MAX_TEXT_BYTES: usize = 256;

/// Most scope entries a device-enrolment challenge may name.
pub const MAX_SCOPE_ENTRIES: usize = 32;

/// Most challenges outstanding at once per issuer.
pub const MAX_OUTSTANDING_CHALLENGES: usize = 4096;

/// Maximum number of accepted binding changes restored from persistent state.
pub const MAX_BINDING_HISTORY_RECORDS: usize = 100_000;

/// Length of a challenge nonce.
pub const NONCE_LEN: usize = 32;

/// Typed errors for binding and challenge handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingError {
    /// A field is empty, too long, or otherwise malformed.
    Field {
        /// The offending field.
        field: &'static str,
        /// What is wrong with it.
        problem: &'static str,
    },
    /// The canonical encoding was rejected.
    Cbor(CborError),
    /// A signature did not verify against the key it must come from.
    BadSignature {
        /// Which signature failed.
        which: &'static str,
    },
    /// A public key is not a valid Ed25519 point.
    BadKey,
    /// The statement names a different account from the authenticated
    /// session.
    AccountMismatch,
    /// The account already has a binding; changing it needs the current
    /// key or the recovery threshold, never a new login.
    AlreadyBound,
    /// No binding exists for the account.
    NotBound,
    /// A binding update carries no key or recovery authority.
    MissingAuthority,
    /// The update's authority is not the currently bound key.
    WrongAuthorityKey,
    /// The recovery-threshold proof was rejected.
    RecoveryRejected,
    /// The statement's generation or predecessor hash does not follow
    /// the current binding.
    OutOfSequence,
    /// The challenge is unknown or was already used.
    UnknownChallenge,
    /// The challenge has expired.
    Expired,
    /// The issuer has too many outstanding challenges.
    TooManyOutstanding,
}

impl fmt::Display for BindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Field { field, problem } => write!(f, "field `{field}` {problem}"),
            Self::Cbor(err) => write!(f, "canonical encoding rejected: {err}"),
            Self::BadSignature { which } => write!(f, "{which} signature does not verify"),
            Self::BadKey => f.write_str("public key is not a valid Ed25519 key"),
            Self::AccountMismatch => {
                f.write_str("statement account does not match the authenticated account")
            }
            Self::AlreadyBound => f.write_str(
                "account is already bound; a change needs the current key or the recovery threshold",
            ),
            Self::NotBound => f.write_str("account has no login binding"),
            Self::MissingAuthority => f.write_str("binding update carries no authority"),
            Self::WrongAuthorityKey => f.write_str("authority key is not the bound key"),
            Self::RecoveryRejected => f.write_str("recovery-threshold proof rejected"),
            Self::OutOfSequence => f.write_str("statement does not follow the current binding"),
            Self::UnknownChallenge => f.write_str("challenge is unknown or already used"),
            Self::Expired => f.write_str("challenge has expired"),
            Self::TooManyOutstanding => f.write_str("too many outstanding challenges"),
        }
    }
}

impl std::error::Error for BindingError {}

impl From<CborError> for BindingError {
    fn from(err: CborError) -> Self {
        Self::Cbor(err)
    }
}

/// The authentication account a binding points at. It identifies an
/// account at a login provider; it confers no key authority.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuthAccount {
    /// An OIDC account: the issuer and the SHA-256 of the subject.
    Oidc {
        /// The ID Token `iss`.
        issuer: String,
        /// Domain-separated SHA-256 of the ID Token `sub`.
        subject_hash: [u8; 32],
    },
    /// An OPAQUE (RFC 9807) account identifier.
    Opaque {
        /// The OPAQUE credential identifier.
        account_id: String,
    },
}

impl AuthAccount {
    /// The account pointer for an OIDC `(iss, sub)` pair.
    ///
    /// # Errors
    ///
    /// [`BindingError::Field`] for an empty or over-long issuer or
    /// subject.
    pub fn oidc(issuer: &str, subject: &str) -> Result<Self, BindingError> {
        check_text("issuer", issuer)?;
        check_text("subject", subject)?;
        let mut hasher = Sha256::new();
        hasher.update(OIDC_SUBJECT_DOMAIN);
        hasher.update(
            u64::try_from(issuer.len())
                .unwrap_or(u64::MAX)
                .to_be_bytes(),
        );
        hasher.update(issuer.as_bytes());
        hasher.update(subject.as_bytes());
        Ok(Self::Oidc {
            issuer: issuer.to_owned(),
            subject_hash: hasher.finalize().into(),
        })
    }

    /// The account pointer for an OPAQUE account identifier.
    ///
    /// # Errors
    ///
    /// [`BindingError::Field`] for an empty or over-long identifier.
    pub fn opaque(account_id: &str) -> Result<Self, BindingError> {
        check_text("account_id", account_id)?;
        Ok(Self::Opaque {
            account_id: account_id.to_owned(),
        })
    }

    fn to_value(&self) -> Value {
        match self {
            Self::Oidc {
                issuer,
                subject_hash,
            } => map(vec![
                ("method", text("oidc")),
                ("issuer", text(issuer)),
                ("subject_hash", Value::Bytes(subject_hash.to_vec())),
            ]),
            Self::Opaque { account_id } => map(vec![
                ("method", text("opaque")),
                ("account_id", text(account_id)),
            ]),
        }
    }

    fn from_value(value: &Value) -> Result<Self, BindingError> {
        let fields = Fields::new(value, &["method", "issuer", "subject_hash", "account_id"])?;
        match fields.text("method")? {
            "oidc" => {
                fields.only(&["method", "issuer", "subject_hash"])?;
                let issuer = fields.text("issuer")?;
                check_text("issuer", issuer)?;
                Ok(Self::Oidc {
                    issuer: issuer.to_owned(),
                    subject_hash: fields.bytes32("subject_hash")?,
                })
            }
            "opaque" => {
                fields.only(&["method", "account_id"])?;
                Self::opaque(fields.text("account_id")?)
            }
            _ => Err(BindingError::Field {
                field: "method",
                problem: "is not a known login method",
            }),
        }
    }
}

/// The custody variant the identity was set up under (#5684 §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CustodyProfile {
    /// (a) Service-held key — custodial, outside the keyless boundary.
    ServiceHeld,
    /// (b) User-held key.
    UserHeld,
    /// (c) Security-code recovery.
    SecurityCode,
    /// (d) Pseudo-login with scoped delegation.
    PseudoLogin,
}

impl CustodyProfile {
    /// The canonical token for this profile.
    pub const fn token(self) -> &'static str {
        match self {
            Self::ServiceHeld => "a-service-held",
            Self::UserHeld => "b-user-held",
            Self::SecurityCode => "c-security-code",
            Self::PseudoLogin => "d-pseudo-login",
        }
    }

    fn from_token(token: &str) -> Result<Self, BindingError> {
        match token {
            "a-service-held" => Ok(Self::ServiceHeld),
            "b-user-held" => Ok(Self::UserHeld),
            "c-security-code" => Ok(Self::SecurityCode),
            "d-pseudo-login" => Ok(Self::PseudoLogin),
            _ => Err(BindingError::Field {
                field: "custody",
                problem: "is not a known custody profile",
            }),
        }
    }
}

/// The `"id/login-binding/v1"` statement a client signs to bind its ID
/// public key to an authentication account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingStatement {
    /// The ID public key being bound.
    pub id_public_key: [u8; 32],
    /// The authentication account it is bound to.
    pub account: AuthAccount,
    /// The custody profile of the identity.
    pub custody: CustodyProfile,
    /// SHA-256 of the identity's current `RecoverySet` (#5884 §4).
    pub recovery_commitment: [u8; 32],
    /// 1 for the first binding of an account, +1 for each update.
    pub generation: u64,
    /// [`BindingStatement::hash`] of the binding this one replaces;
    /// `None` exactly when `generation` is 1.
    pub prev: Option<[u8; 32]>,
}

impl BindingStatement {
    fn to_value(&self) -> Value {
        let mut entries = vec![
            ("v", Value::UInt(STATEMENT_VERSION)),
            ("id_public_key", Value::Bytes(self.id_public_key.to_vec())),
            ("account", self.account.to_value()),
            ("custody", text(self.custody.token())),
            (
                "recovery_commitment",
                Value::Bytes(self.recovery_commitment.to_vec()),
            ),
            ("generation", Value::UInt(self.generation)),
        ];
        if let Some(prev) = &self.prev {
            entries.push(("prev", Value::Bytes(prev.to_vec())));
        }
        map(entries)
    }

    /// `canonical_cbor(statement)`.
    ///
    /// # Errors
    ///
    /// [`BindingError::Cbor`] if the encoder rejects the value.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, BindingError> {
        Ok(encode_canonical(&self.to_value())?)
    }

    /// Decodes a statement, rejecting unknown fields, non-canonical
    /// encodings and an inconsistent generation/predecessor pair.
    ///
    /// # Errors
    ///
    /// [`BindingError`] describing the first problem found.
    pub fn decode(bytes: &[u8]) -> Result<Self, BindingError> {
        let value = decode_canonical(bytes)?;
        Self::from_value(&value)
    }

    fn from_value(value: &Value) -> Result<Self, BindingError> {
        let fields = Fields::new(
            value,
            &[
                "v",
                "id_public_key",
                "account",
                "custody",
                "recovery_commitment",
                "generation",
                "prev",
            ],
        )?;
        if fields.uint("v")? != STATEMENT_VERSION {
            return Err(BindingError::Field {
                field: "v",
                problem: "is not a supported version",
            });
        }
        let statement = Self {
            id_public_key: fields.bytes32("id_public_key")?,
            account: AuthAccount::from_value(fields.get("account")?)?,
            custody: CustodyProfile::from_token(fields.text("custody")?)?,
            recovery_commitment: fields.bytes32("recovery_commitment")?,
            generation: fields.uint("generation")?,
            prev: if fields.has("prev") {
                Some(fields.bytes32("prev")?)
            } else {
                None
            },
        };
        statement.check_shape()?;
        Ok(statement)
    }

    /// Domain-separated SHA-256 identity of the statement, used as the
    /// next statement's `prev`.
    ///
    /// # Errors
    ///
    /// As [`BindingStatement::canonical_bytes`].
    pub fn hash(&self) -> Result<[u8; 32], BindingError> {
        let mut hasher = Sha256::new();
        hasher.update(BINDING_DOMAIN.as_bytes());
        hasher.update([0]);
        hasher.update(self.canonical_bytes()?);
        Ok(hasher.finalize().into())
    }

    /// The bytes a key signs for this statement:
    /// `"id/login-binding/v1\0" ‖ canonical_cbor(statement)`.
    ///
    /// # Errors
    ///
    /// As [`BindingStatement::canonical_bytes`].
    pub fn signing_preimage(&self) -> Result<Vec<u8>, BindingError> {
        let mut out = Vec::with_capacity(BINDING_DOMAIN.len() + 160);
        out.extend_from_slice(BINDING_DOMAIN.as_bytes());
        out.push(0);
        out.extend_from_slice(&self.canonical_bytes()?);
        Ok(out)
    }

    const fn check_shape(&self) -> Result<(), BindingError> {
        match (self.generation, self.prev.is_some()) {
            (0, _) => Err(BindingError::Field {
                field: "generation",
                problem: "must be at least 1",
            }),
            (1, false) => Ok(()),
            (1, true) => Err(BindingError::Field {
                field: "prev",
                problem: "must be absent on the first binding",
            }),
            (_, false) => Err(BindingError::Field {
                field: "prev",
                problem: "is required after the first binding",
            }),
            (_, true) => Ok(()),
        }
    }
}

/// Who authorises a change to an existing binding (#5885 invariant 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingAuthority {
    /// A signature over the new statement by the currently bound key.
    CurrentKey {
        /// The signing key's public half.
        key: [u8; 32],
        /// Signature over [`BindingStatement::signing_preimage`].
        signature: [u8; 64],
    },
    /// A recovery-threshold proof (#5888), judged by a
    /// [`RecoveryThreshold`] verifier.
    Recovery(Vec<u8>),
}

impl BindingAuthority {
    fn to_value(&self) -> Value {
        match self {
            Self::CurrentKey { key, signature } => map(vec![
                ("kind", text("current-key")),
                ("key", Value::Bytes(key.to_vec())),
                ("signature", Value::Bytes(signature.to_vec())),
            ]),
            Self::Recovery(proof) => map(vec![
                ("kind", text("recovery")),
                ("proof", Value::Bytes(proof.clone())),
            ]),
        }
    }

    fn from_value(value: &Value) -> Result<Self, BindingError> {
        let fields = Fields::new(value, &["kind", "key", "signature", "proof"])?;
        match fields.text("kind")? {
            "current-key" => {
                fields.only(&["kind", "key", "signature"])?;
                let signature = bytes64(fields.get("signature")?, "signature")?;
                Ok(Self::CurrentKey {
                    key: fields.bytes32("key")?,
                    signature,
                })
            }
            "recovery" => {
                fields.only(&["kind", "proof"])?;
                let Value::Bytes(proof) = fields.get("proof")? else {
                    return Err(BindingError::Field {
                        field: "proof",
                        problem: "is not a byte string",
                    });
                };
                Ok(Self::Recovery(proof.clone()))
            }
            _ => Err(BindingError::Field {
                field: "kind",
                problem: "is not a known binding authority",
            }),
        }
    }
}

/// Verifies a #5888 recovery-threshold proof for a binding change.
pub trait RecoveryThreshold {
    /// True when `proof` authorises replacing the binding of `account`
    /// with the statement whose hash is `statement_hash`.
    fn authorises(&self, account: &AuthAccount, statement_hash: &[u8; 32], proof: &[u8]) -> bool;
}

/// A recovery verifier that rejects every proof, for deployments where
/// the #5888 threshold is not configured.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoRecoveryThreshold;

impl RecoveryThreshold for NoRecoveryThreshold {
    fn authorises(&self, _: &AuthAccount, _: &[u8; 32], _: &[u8]) -> bool {
        false
    }
}

/// A statement with the signatures that make it a binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedBinding {
    /// The statement.
    pub statement: BindingStatement,
    /// Signature by `statement.id_public_key`: proof the client holds
    /// the key it is binding.
    pub self_signature: [u8; 64],
    /// Authority for an update; `None` on the first binding.
    pub authority: Option<BindingAuthority>,
}

impl SignedBinding {
    fn to_value(&self) -> Value {
        let mut entries = vec![
            ("statement", self.statement.to_value()),
            ("self_signature", Value::Bytes(self.self_signature.to_vec())),
        ];
        if let Some(authority) = &self.authority {
            entries.push(("authority", authority.to_value()));
        }
        map(entries)
    }

    fn from_value(value: &Value) -> Result<Self, BindingError> {
        let fields = Fields::new(value, &["statement", "self_signature", "authority"])?;
        Ok(Self {
            statement: BindingStatement::from_value(fields.get("statement")?)?,
            self_signature: bytes64(fields.get("self_signature")?, "self_signature")?,
            authority: if fields.has("authority") {
                Some(BindingAuthority::from_value(fields.get("authority")?)?)
            } else {
                None
            },
        })
    }

    /// Client side: signs a first binding with the key it binds.
    ///
    /// # Errors
    ///
    /// [`BindingError`] if the statement is malformed or names another
    /// key.
    pub fn enrol(statement: BindingStatement, key: &SigningKey) -> Result<Self, BindingError> {
        statement.check_shape()?;
        if statement.id_public_key != key.verifying_key().to_bytes() {
            return Err(BindingError::Field {
                field: "id_public_key",
                problem: "is not the signing key",
            });
        }
        let self_signature = key.sign(&statement.signing_preimage()?).to_bytes();
        Ok(Self {
            statement,
            self_signature,
            authority: None,
        })
    }

    /// Client side: signs an update with the new key, authorised by the
    /// currently bound key.
    ///
    /// # Errors
    ///
    /// As [`SignedBinding::enrol`].
    pub fn update(
        statement: BindingStatement,
        new_key: &SigningKey,
        current_key: &SigningKey,
    ) -> Result<Self, BindingError> {
        let mut signed = Self::enrol(statement, new_key)?;
        let preimage = signed.statement.signing_preimage()?;
        signed.authority = Some(BindingAuthority::CurrentKey {
            key: current_key.verifying_key().to_bytes(),
            signature: current_key.sign(&preimage).to_bytes(),
        });
        Ok(signed)
    }

    fn verify_self_signature(&self) -> Result<Vec<u8>, BindingError> {
        let preimage = self.statement.signing_preimage()?;
        verify(
            &self.statement.id_public_key,
            &preimage,
            &self.self_signature,
            "self",
        )?;
        Ok(preimage)
    }
}

/// An account whose login flow (OIDC ID Token validation or OPAQUE
/// key exchange) has succeeded. It is evidence of *authentication only*:
/// it can start an enrolment for an unbound account and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedAccount(AuthAccount);

impl AuthenticatedAccount {
    /// Wraps an account the caller's login flow has verified. Call only
    /// after the flow succeeded.
    pub const fn from_verified_login(account: AuthAccount) -> Self {
        Self(account)
    }

    /// The authenticated account.
    pub const fn account(&self) -> &AuthAccount {
        &self.0
    }
}

/// What the authorizer stores per account: public material only
/// (#5885 invariant 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBinding {
    /// The current statement.
    pub statement: BindingStatement,
    /// Its hash.
    pub statement_hash: [u8; 32],
    /// The bound key's signature over it.
    pub self_signature: [u8; 64],
    /// The authority that admitted it, if it is an update.
    pub authority: Option<BindingAuthority>,
}

/// The authorizer's login-binding table.
#[derive(Debug, Default)]
pub struct BindingRegistry {
    bindings: BTreeMap<AuthAccount, StoredBinding>,
    history: Vec<SignedBinding>,
}

impl BindingRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// The current binding of `account`, if any.
    pub fn binding(&self, account: &AuthAccount) -> Option<&StoredBinding> {
        self.bindings.get(account)
    }

    /// The key currently bound to `account`.
    pub fn bound_key(&self, account: &AuthAccount) -> Option<[u8; 32]> {
        self.bindings
            .get(account)
            .map(|stored| stored.statement.id_public_key)
    }

    /// Encode the accepted signed binding history for persistent storage.
    /// Every operation is replayed and re-authorized when restored.
    ///
    /// # Errors
    ///
    /// [`BindingError::Cbor`] if canonical encoding fails.
    pub fn history_bytes(&self) -> Result<Vec<u8>, BindingError> {
        if self.history.len() > MAX_BINDING_HISTORY_RECORDS {
            return Err(BindingError::Field {
                field: "history",
                problem: "has too many records",
            });
        }
        Ok(encode_canonical(&Value::Array(
            self.history.iter().map(SignedBinding::to_value).collect(),
        ))?)
    }

    /// Restore a registry from its signed operation history. Each enrolment,
    /// current-key update, and recovery update is re-validated in order.
    ///
    /// # Errors
    ///
    /// [`BindingError`] when the history is malformed or any operation fails
    /// its original authorization checks.
    pub fn from_history_bytes(
        bytes: &[u8],
        recovery: &dyn RecoveryThreshold,
    ) -> Result<Self, BindingError> {
        let value = decode_canonical(bytes)?;
        let Value::Array(entries) = value else {
            return Err(BindingError::Field {
                field: "history",
                problem: "is not an array",
            });
        };
        if entries.len() > MAX_BINDING_HISTORY_RECORDS {
            return Err(BindingError::Field {
                field: "history",
                problem: "has too many records",
            });
        }
        let mut registry = Self::new();
        for entry in entries {
            let signed = SignedBinding::from_value(&entry)?;
            if signed.statement.generation == 1 {
                let session =
                    AuthenticatedAccount::from_verified_login(signed.statement.account.clone());
                registry.enrol(&session, signed)?;
            } else {
                registry.update(signed, recovery)?;
            }
        }
        Ok(registry)
    }

    /// First binding of an account, admitted on an authenticated session
    /// for that same account and a self-signature by the bound key.
    ///
    /// # Errors
    ///
    /// [`BindingError::AlreadyBound`] when the account has a binding —
    /// a new login never replaces one; otherwise the first failed check.
    pub fn enrol(
        &mut self,
        session: &AuthenticatedAccount,
        signed: SignedBinding,
    ) -> Result<&StoredBinding, BindingError> {
        let history_entry = signed.clone();
        let statement = &signed.statement;
        if &statement.account != session.account() {
            return Err(BindingError::AccountMismatch);
        }
        if self.bindings.contains_key(&statement.account) {
            return Err(BindingError::AlreadyBound);
        }
        if statement.generation != 1 || statement.prev.is_some() {
            return Err(BindingError::OutOfSequence);
        }
        if signed.authority.is_some() {
            return Err(BindingError::Field {
                field: "authority",
                problem: "must be absent on the first binding",
            });
        }
        signed.verify_self_signature()?;
        let account = statement.account.clone();
        let stored = StoredBinding {
            statement_hash: statement.hash()?,
            statement: signed.statement,
            self_signature: signed.self_signature,
            authority: None,
        };
        self.bindings.insert(account.clone(), stored);
        self.history.push(history_entry);
        self.bindings.get(&account).ok_or(BindingError::NotBound)
    }

    /// Replaces an account's binding. The change is authorised by the
    /// signature of the currently bound key or a recovery-threshold
    /// proof — never by an authenticated session, so this method does
    /// not take one.
    ///
    /// # Errors
    ///
    /// [`BindingError`] for the first failed check; the registry is
    /// unchanged on error.
    pub fn update(
        &mut self,
        signed: SignedBinding,
        recovery: &dyn RecoveryThreshold,
    ) -> Result<&StoredBinding, BindingError> {
        let history_entry = signed.clone();
        let statement = &signed.statement;
        let current = self
            .bindings
            .get(&statement.account)
            .ok_or(BindingError::NotBound)?;
        if statement.generation != current.statement.generation.saturating_add(1)
            || statement.prev != Some(current.statement_hash)
        {
            return Err(BindingError::OutOfSequence);
        }
        let preimage = signed.verify_self_signature()?;
        let statement_hash = statement.hash()?;
        match &signed.authority {
            None => return Err(BindingError::MissingAuthority),
            Some(BindingAuthority::CurrentKey { key, signature }) => {
                if key != &current.statement.id_public_key {
                    return Err(BindingError::WrongAuthorityKey);
                }
                verify(key, &preimage, signature, "authority")?;
            }
            Some(BindingAuthority::Recovery(proof)) => {
                if !recovery.authorises(&statement.account, &statement_hash, proof) {
                    return Err(BindingError::RecoveryRejected);
                }
            }
        }
        let account = statement.account.clone();
        let stored = StoredBinding {
            statement_hash,
            statement: signed.statement,
            self_signature: signed.self_signature,
            authority: signed.authority,
        };
        self.bindings.insert(account.clone(), stored);
        self.history.push(history_entry);
        self.bindings.get(&account).ok_or(BindingError::NotBound)
    }
}

/// What an ID-authorised challenge is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChallengePurpose {
    /// Accept a login where the ID key is required.
    LoginAcceptance,
    /// Enrol a new device, naming the pseudo-login scope it requests.
    DeviceEnrolment {
        /// The new device's (pseudo-login) public key.
        device_key: [u8; 32],
        /// The requested scope entries (worlds × capabilities, #5889).
        scope: Vec<String>,
    },
}

impl ChallengePurpose {
    fn to_value(&self) -> Value {
        match self {
            Self::LoginAcceptance => map(vec![("kind", text("login"))]),
            Self::DeviceEnrolment { device_key, scope } => map(vec![
                ("kind", text("device-enrolment")),
                ("device_key", Value::Bytes(device_key.to_vec())),
                (
                    "scope",
                    Value::Array(scope.iter().map(|entry| text(entry)).collect()),
                ),
            ]),
        }
    }

    fn check(&self) -> Result<(), BindingError> {
        if let Self::DeviceEnrolment { device_key, scope } = self {
            VerifyingKey::from_bytes(device_key).map_err(|_err| BindingError::BadKey)?;
            if scope.is_empty() {
                return Err(BindingError::Field {
                    field: "scope",
                    problem: "must name the requested pseudo-login scope",
                });
            }
            if scope.len() > MAX_SCOPE_ENTRIES {
                return Err(BindingError::Field {
                    field: "scope",
                    problem: "has too many entries",
                });
            }
            for entry in scope {
                check_text("scope", entry)?;
            }
        }
        Ok(())
    }
}

/// A fresh challenge for the client-held ID key to sign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge {
    /// Random single-use nonce.
    pub nonce: [u8; NONCE_LEN],
    /// The relying party the signature is for.
    pub audience: String,
    /// The account whose bound key must sign.
    pub account: AuthAccount,
    /// What the signature authorises.
    pub purpose: ChallengePurpose,
    /// Issue time, unix seconds.
    pub issued_at: u64,
    /// Expiry, unix seconds (exclusive).
    pub expires_at: u64,
}

impl Challenge {
    /// The bytes the ID key signs:
    /// `"id/login-challenge/v1\0" ‖ canonical_cbor(challenge)`.
    ///
    /// # Errors
    ///
    /// [`BindingError::Cbor`] if the encoder rejects the value.
    pub fn signing_preimage(&self) -> Result<Vec<u8>, BindingError> {
        let value = map(vec![
            ("v", Value::UInt(STATEMENT_VERSION)),
            ("nonce", Value::Bytes(self.nonce.to_vec())),
            ("audience", text(&self.audience)),
            ("account", self.account.to_value()),
            ("purpose", self.purpose.to_value()),
            ("issued_at", Value::UInt(self.issued_at)),
            ("expires_at", Value::UInt(self.expires_at)),
        ]);
        let mut out = Vec::new();
        out.extend_from_slice(CHALLENGE_DOMAIN.as_bytes());
        out.push(0);
        out.extend_from_slice(&encode_canonical(&value)?);
        Ok(out)
    }

    /// Client side: signs the challenge with the held ID key.
    ///
    /// # Errors
    ///
    /// As [`Challenge::signing_preimage`].
    pub fn sign(&self, key: &SigningKey) -> Result<ChallengeResponse, BindingError> {
        Ok(ChallengeResponse {
            nonce: self.nonce,
            signature: key.sign(&self.signing_preimage()?).to_bytes(),
        })
    }
}

/// The client's answer to a [`Challenge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChallengeResponse {
    /// The nonce of the challenge answered.
    pub nonce: [u8; NONCE_LEN],
    /// Signature over [`Challenge::signing_preimage`].
    pub signature: [u8; 64],
}

/// A challenge whose signature verified against the bound key. For a
/// device enrolment it records the *requested* scope; it is not a
/// delegation grant (#5889 mints those).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedChallenge {
    /// The challenge that was answered.
    pub challenge: Challenge,
    /// The key that signed it.
    pub signer: [u8; 32],
}

/// Issues and verifies single-use challenges for one audience.
#[derive(Debug)]
pub struct ChallengeIssuer {
    audience: String,
    ttl_secs: u64,
    outstanding: BTreeMap<[u8; NONCE_LEN], Challenge>,
}

impl ChallengeIssuer {
    /// An issuer for `audience` whose challenges live `ttl_secs`.
    ///
    /// # Errors
    ///
    /// [`BindingError::Field`] for a bad audience or a zero lifetime.
    pub fn new(audience: &str, ttl_secs: u64) -> Result<Self, BindingError> {
        check_text("audience", audience)?;
        if ttl_secs == 0 {
            return Err(BindingError::Field {
                field: "ttl_secs",
                problem: "must be positive",
            });
        }
        Ok(Self {
            audience: audience.to_owned(),
            ttl_secs,
            outstanding: BTreeMap::new(),
        })
    }

    /// Issues a challenge for `account` at time `now` (unix seconds).
    ///
    /// # Errors
    ///
    /// [`BindingError::NotBound`] when the account has no binding (an
    /// unbound account has no key to sign), a malformed purpose, or
    /// [`BindingError::TooManyOutstanding`].
    pub fn issue(
        &mut self,
        registry: &BindingRegistry,
        account: &AuthAccount,
        purpose: ChallengePurpose,
        now: u64,
    ) -> Result<Challenge, BindingError> {
        if registry.binding(account).is_none() {
            return Err(BindingError::NotBound);
        }
        purpose.check()?;
        self.outstanding
            .retain(|_, challenge| challenge.expires_at > now);
        if self.outstanding.len() >= MAX_OUTSTANDING_CHALLENGES {
            return Err(BindingError::TooManyOutstanding);
        }
        let mut nonce = [0u8; NONCE_LEN];
        rand::rng().fill(&mut nonce);
        let challenge = Challenge {
            nonce,
            audience: self.audience.clone(),
            account: account.clone(),
            purpose,
            issued_at: now,
            expires_at: now.saturating_add(self.ttl_secs),
        };
        self.outstanding.insert(nonce, challenge.clone());
        Ok(challenge)
    }

    /// Verifies a response at time `now`. The nonce is consumed whether
    /// or not verification succeeds, so a challenge admits one attempt.
    ///
    /// # Errors
    ///
    /// [`BindingError::UnknownChallenge`], [`BindingError::Expired`],
    /// [`BindingError::NotBound`] or [`BindingError::BadSignature`].
    pub fn verify(
        &mut self,
        registry: &BindingRegistry,
        response: &ChallengeResponse,
        now: u64,
    ) -> Result<VerifiedChallenge, BindingError> {
        let challenge = self
            .outstanding
            .remove(&response.nonce)
            .ok_or(BindingError::UnknownChallenge)?;
        if now >= challenge.expires_at {
            return Err(BindingError::Expired);
        }
        let signer = registry
            .bound_key(&challenge.account)
            .ok_or(BindingError::NotBound)?;
        verify(
            &signer,
            &challenge.signing_preimage()?,
            &response.signature,
            "challenge",
        )?;
        Ok(VerifiedChallenge { challenge, signer })
    }

    /// How many challenges are outstanding.
    pub fn outstanding(&self) -> usize {
        self.outstanding.len()
    }
}

fn verify(
    key: &[u8; 32],
    message: &[u8],
    signature: &[u8; 64],
    which: &'static str,
) -> Result<(), BindingError> {
    let key = VerifyingKey::from_bytes(key).map_err(|_err| BindingError::BadKey)?;
    key.verify_strict(message, &Signature::from_bytes(signature))
        .map_err(|_err| BindingError::BadSignature { which })
}

const fn check_text(field: &'static str, value: &str) -> Result<(), BindingError> {
    if value.is_empty() {
        return Err(BindingError::Field {
            field,
            problem: "must not be empty",
        });
    }
    if value.len() > MAX_TEXT_BYTES {
        return Err(BindingError::Field {
            field,
            problem: "is too long",
        });
    }
    Ok(())
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn map(mut entries: Vec<(&'static str, Value)>) -> Value {
    entries.sort_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    Value::Map(entries.into_iter().map(|(k, v)| (text(k), v)).collect())
}

fn bytes64(value: &Value, field: &'static str) -> Result<[u8; 64], BindingError> {
    match value {
        Value::Bytes(bytes) => bytes
            .as_slice()
            .try_into()
            .map_err(|_err| BindingError::Field {
                field,
                problem: "is not 64 bytes",
            }),
        _ => Err(BindingError::Field {
            field,
            problem: "is not a byte string",
        }),
    }
}

/// Strict field access over a decoded map.
struct Fields<'a> {
    entries: &'a [(Value, Value)],
}

impl<'a> Fields<'a> {
    fn new(value: &'a Value, known: &[&str]) -> Result<Self, BindingError> {
        let Value::Map(entries) = value else {
            return Err(BindingError::Field {
                field: "record",
                problem: "is not a map",
            });
        };
        for (key, _) in entries {
            match key {
                Value::Text(name) if known.contains(&name.as_str()) => {}
                _ => {
                    return Err(BindingError::Field {
                        field: "record",
                        problem: "carries an unknown field",
                    });
                }
            }
        }
        Ok(Self { entries })
    }

    fn only(&self, allowed: &[&str]) -> Result<(), BindingError> {
        for (key, _) in self.entries {
            if let Value::Text(name) = key
                && !allowed.contains(&name.as_str())
            {
                return Err(BindingError::Field {
                    field: "account",
                    problem: "carries a field of another login method",
                });
            }
        }
        Ok(())
    }

    fn has(&self, name: &str) -> bool {
        self.entries
            .iter()
            .any(|(key, _)| matches!(key, Value::Text(k) if k == name))
    }

    fn get(&self, name: &'static str) -> Result<&'a Value, BindingError> {
        self.entries
            .iter()
            .find(|(key, _)| matches!(key, Value::Text(k) if k == name))
            .map(|(_, value)| value)
            .ok_or(BindingError::Field {
                field: name,
                problem: "is missing",
            })
    }

    fn text(&self, name: &'static str) -> Result<&'a str, BindingError> {
        match self.get(name)? {
            Value::Text(value) => Ok(value),
            _ => Err(BindingError::Field {
                field: name,
                problem: "is not text",
            }),
        }
    }

    fn uint(&self, name: &'static str) -> Result<u64, BindingError> {
        match self.get(name)? {
            Value::UInt(value) => Ok(*value),
            _ => Err(BindingError::Field {
                field: name,
                problem: "is not an unsigned integer",
            }),
        }
    }

    fn bytes32(&self, name: &'static str) -> Result<[u8; 32], BindingError> {
        match self.get(name)? {
            Value::Bytes(bytes) => {
                bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_err| BindingError::Field {
                        field: name,
                        problem: "is not 32 bytes",
                    })
            }
            _ => Err(BindingError::Field {
                field: name,
                problem: "is not a byte string",
            }),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn account() -> AuthAccount {
        AuthAccount::oidc("https://accounts.google.com", "1234567890").unwrap()
    }

    fn first(key: &SigningKey, account: &AuthAccount) -> BindingStatement {
        BindingStatement {
            id_public_key: key.verifying_key().to_bytes(),
            account: account.clone(),
            custody: CustodyProfile::UserHeld,
            recovery_commitment: [5; 32],
            generation: 1,
            prev: None,
        }
    }

    fn next(stored: &StoredBinding, key: &SigningKey) -> BindingStatement {
        BindingStatement {
            id_public_key: key.verifying_key().to_bytes(),
            generation: stored.statement.generation + 1,
            prev: Some(stored.statement_hash),
            ..stored.statement.clone()
        }
    }

    fn enrolled(seed: u8) -> (BindingRegistry, SigningKey) {
        let mut registry = BindingRegistry::new();
        let k = key(seed);
        let session = AuthenticatedAccount::from_verified_login(account());
        registry
            .enrol(
                &session,
                SignedBinding::enrol(first(&k, &account()), &k).unwrap(),
            )
            .unwrap();
        (registry, k)
    }

    #[test]
    fn test_statement_round_trips_canonically() {
        let k = key(1);
        let mut statement = first(&k, &account());
        let bytes = statement.canonical_bytes().unwrap();
        assert_eq!(BindingStatement::decode(&bytes).unwrap(), statement);
        statement.generation = 2;
        statement.prev = Some([9; 32]);
        statement.account = AuthAccount::opaque("alice").unwrap();
        let bytes = statement.canonical_bytes().unwrap();
        assert_eq!(BindingStatement::decode(&bytes).unwrap(), statement);
    }

    #[test]
    fn test_decode_rejects_unknown_field_and_bad_sequence() {
        let k = key(1);
        let Value::Map(mut entries) = first(&k, &account()).to_value() else {
            unreachable!()
        };
        entries.push((text("zzz_extra"), Value::Null));
        let bytes = encode_canonical(&Value::Map(entries)).unwrap();
        assert!(BindingStatement::decode(&bytes).is_err());

        let mut bad = first(&k, &account());
        bad.prev = Some([1; 32]);
        let bytes = bad.canonical_bytes().unwrap();
        assert!(matches!(
            BindingStatement::decode(&bytes),
            Err(BindingError::Field { field: "prev", .. })
        ));
    }

    #[test]
    fn test_oidc_subject_is_hashed_not_stored() {
        let AuthAccount::Oidc { subject_hash, .. } = account() else {
            unreachable!()
        };
        let bytes = first(&key(1), &account()).canonical_bytes().unwrap();
        assert!(!bytes.windows(10).any(|w| w == b"1234567890"));
        assert_ne!(subject_hash, [0; 32]);
        assert_ne!(
            account(),
            AuthAccount::oidc("https://accounts.google.com", "1234567891").unwrap()
        );
    }

    #[test]
    fn invariant_1_authorizer_stores_only_public_material() {
        let (registry, k) = enrolled(3);
        let stored = registry.binding(&account()).unwrap();
        let secret = k.to_bytes();
        let mut stored_bytes = stored.statement.canonical_bytes().unwrap();
        stored_bytes.extend_from_slice(&stored.self_signature);
        stored_bytes.extend_from_slice(&stored.statement_hash);
        assert!(!stored_bytes.windows(32).any(|w| w == secret));
        assert_eq!(
            registry.bound_key(&account()),
            Some(k.verifying_key().to_bytes())
        );
    }

    #[test]
    fn invariant_2_auth_success_alone_never_registers_or_replaces_a_key() {
        // Enrolment needs a self-signature by the key being bound.
        let mut registry = BindingRegistry::new();
        let session = AuthenticatedAccount::from_verified_login(account());
        let mut forged = SignedBinding::enrol(first(&key(1), &account()), &key(1)).unwrap();
        forged.self_signature = [0; 64];
        assert!(matches!(
            registry.enrol(&session, forged),
            Err(BindingError::BadSignature { which: "self" })
        ));
        assert!(registry.binding(&account()).is_none());

        // A fresh login for a bound account cannot rebind it.
        let (mut registry, original) = enrolled(1);
        let attacker = key(2);
        let attempt = SignedBinding::enrol(first(&attacker, &account()), &attacker).unwrap();
        assert_eq!(
            registry.enrol(&session, attempt),
            Err(BindingError::AlreadyBound)
        );
        assert_eq!(
            registry.bound_key(&account()),
            Some(original.verifying_key().to_bytes())
        );

        // A session for one account cannot enrol a statement for another.
        let other = AuthenticatedAccount::from_verified_login(AuthAccount::opaque("bob").unwrap());
        let mut registry = BindingRegistry::new();
        let attempt = SignedBinding::enrol(first(&attacker, &account()), &attacker).unwrap();
        assert_eq!(
            registry.enrol(&other, attempt),
            Err(BindingError::AccountMismatch)
        );
    }

    #[test]
    fn invariant_3_binding_mutation_needs_current_key_or_recovery_threshold() {
        let (mut registry, current) = enrolled(1);
        let stored = registry.binding(&account()).unwrap().clone();
        let new_key = key(2);

        // Self-signed by the new key only: no authority.
        let unauthorised = SignedBinding::enrol(next(&stored, &new_key), &new_key).unwrap();
        assert_eq!(
            registry.update(unauthorised, &NoRecoveryThreshold),
            Err(BindingError::MissingAuthority)
        );

        // Authority from a key that is not the bound one.
        let wrong = SignedBinding::update(next(&stored, &new_key), &new_key, &key(9)).unwrap();
        assert_eq!(
            registry.update(wrong, &NoRecoveryThreshold),
            Err(BindingError::WrongAuthorityKey)
        );

        // Recovery proof the threshold rejects.
        let mut recovery = SignedBinding::enrol(next(&stored, &new_key), &new_key).unwrap();
        recovery.authority = Some(BindingAuthority::Recovery(vec![1, 2, 3]));
        assert_eq!(
            registry.update(recovery, &NoRecoveryThreshold),
            Err(BindingError::RecoveryRejected)
        );
        assert_eq!(registry.binding(&account()), Some(&stored));

        // Current key authorises: accepted, generation advances.
        let good = SignedBinding::update(next(&stored, &new_key), &new_key, &current).unwrap();
        let updated = registry.update(good, &NoRecoveryThreshold).unwrap();
        assert_eq!(updated.statement.generation, 2);
        assert_eq!(
            registry.bound_key(&account()),
            Some(new_key.verifying_key().to_bytes())
        );

        // Replaying the same update is out of sequence.
        let replay = SignedBinding::update(next(&stored, &new_key), &new_key, &current).unwrap();
        assert_eq!(
            registry.update(replay, &NoRecoveryThreshold),
            Err(BindingError::OutOfSequence)
        );
    }

    #[test]
    fn invariant_3_recovery_threshold_authorises_rotation() {
        struct Accepts;
        impl RecoveryThreshold for Accepts {
            fn authorises(&self, _: &AuthAccount, _: &[u8; 32], proof: &[u8]) -> bool {
                proof == b"threshold-met"
            }
        }
        let (mut registry, _lost) = enrolled(1);
        let stored = registry.binding(&account()).unwrap().clone();
        let new_key = key(4);
        let mut signed = SignedBinding::enrol(next(&stored, &new_key), &new_key).unwrap();
        signed.authority = Some(BindingAuthority::Recovery(b"threshold-met".to_vec()));
        registry.update(signed, &Accepts).unwrap();
        assert_eq!(
            registry.bound_key(&account()),
            Some(new_key.verifying_key().to_bytes())
        );
    }

    #[test]
    fn invariant_5_password_reset_or_relogin_is_not_identity_recovery() {
        // A password reset yields a new authenticated session for the same
        // OPAQUE account; it must not rebind or recover the ID key.
        let opaque = AuthAccount::opaque("alice").unwrap();
        let mut registry = BindingRegistry::new();
        let k = key(1);
        let session = AuthenticatedAccount::from_verified_login(opaque.clone());
        registry
            .enrol(
                &session,
                SignedBinding::enrol(first(&k, &opaque), &k).unwrap(),
            )
            .unwrap();
        let after_reset = AuthenticatedAccount::from_verified_login(opaque.clone());
        let new_key = key(2);
        let attempt = SignedBinding::enrol(first(&new_key, &opaque), &new_key).unwrap();
        assert_eq!(
            registry.enrol(&after_reset, attempt),
            Err(BindingError::AlreadyBound)
        );
        assert_eq!(
            registry.bound_key(&opaque),
            Some(k.verifying_key().to_bytes())
        );
    }

    #[test]
    fn test_login_challenge_accepts_bound_key_once() {
        let (registry, k) = enrolled(1);
        let mut issuer = ChallengeIssuer::new("https://id.example", 60).unwrap();
        let challenge = issuer
            .issue(
                &registry,
                &account(),
                ChallengePurpose::LoginAcceptance,
                NOW,
            )
            .unwrap();
        let response = challenge.sign(&k).unwrap();
        let verified = issuer.verify(&registry, &response, NOW + 1).unwrap();
        assert_eq!(verified.signer, k.verifying_key().to_bytes());
        assert_eq!(
            issuer.verify(&registry, &response, NOW + 1),
            Err(BindingError::UnknownChallenge)
        );
    }

    #[test]
    fn test_challenge_rejects_other_key_expiry_and_unbound() {
        let (registry, k) = enrolled(1);
        let mut issuer = ChallengeIssuer::new("https://id.example", 60).unwrap();

        let challenge = issuer
            .issue(
                &registry,
                &account(),
                ChallengePurpose::LoginAcceptance,
                NOW,
            )
            .unwrap();
        assert_eq!(
            issuer.verify(&registry, &challenge.sign(&key(7)).unwrap(), NOW),
            Err(BindingError::BadSignature { which: "challenge" })
        );
        // One attempt per nonce: the right key now fails too.
        assert_eq!(
            issuer.verify(&registry, &challenge.sign(&k).unwrap(), NOW),
            Err(BindingError::UnknownChallenge)
        );

        let challenge = issuer
            .issue(
                &registry,
                &account(),
                ChallengePurpose::LoginAcceptance,
                NOW,
            )
            .unwrap();
        assert_eq!(
            issuer.verify(&registry, &challenge.sign(&k).unwrap(), NOW + 60),
            Err(BindingError::Expired)
        );

        let unbound = AuthAccount::opaque("nobody").unwrap();
        assert_eq!(
            issuer
                .issue(&registry, &unbound, ChallengePurpose::LoginAcceptance, NOW)
                .map(|_| ()),
            Err(BindingError::NotBound)
        );
    }

    #[test]
    fn test_challenge_signature_is_audience_bound() {
        let (registry, k) = enrolled(1);
        let mut a = ChallengeIssuer::new("https://a.example", 60).unwrap();
        let mut b = ChallengeIssuer::new("https://b.example", 60).unwrap();
        let ca = a
            .issue(
                &registry,
                &account(),
                ChallengePurpose::LoginAcceptance,
                NOW,
            )
            .unwrap();
        let cb = b
            .issue(
                &registry,
                &account(),
                ChallengePurpose::LoginAcceptance,
                NOW,
            )
            .unwrap();
        // A signature over a's challenge, presented under b's nonce, fails.
        let forged = ChallengeResponse {
            nonce: cb.nonce,
            signature: ca.sign(&k).unwrap().signature,
        };
        assert_eq!(
            b.verify(&registry, &forged, NOW),
            Err(BindingError::BadSignature { which: "challenge" })
        );
    }

    #[test]
    fn invariant_6_enrolment_challenge_names_scope_and_mints_no_delegation() {
        let (registry, k) = enrolled(1);
        let mut issuer = ChallengeIssuer::new("https://id.example", 60).unwrap();
        let device = key(8).verifying_key().to_bytes();

        let empty = ChallengePurpose::DeviceEnrolment {
            device_key: device,
            scope: Vec::new(),
        };
        assert!(matches!(
            issuer.issue(&registry, &account(), empty, NOW),
            Err(BindingError::Field { field: "scope", .. })
        ));

        let purpose = ChallengePurpose::DeviceEnrolment {
            device_key: device,
            scope: vec!["world:notes/read".to_owned()],
        };
        let challenge = issuer.issue(&registry, &account(), purpose, NOW).unwrap();
        // The scope is inside the signed bytes.
        let preimage = challenge.signing_preimage().unwrap();
        assert!(preimage.windows(16).any(|w| w == b"world:notes/read"));

        // Widening the scope after signing invalidates the signature.
        let response = challenge.sign(&k).unwrap();
        let mut widened = challenge.clone();
        widened.purpose = ChallengePurpose::DeviceEnrolment {
            device_key: device,
            scope: vec!["world:notes/write".to_owned()],
        };
        let widened_preimage = widened.signing_preimage().unwrap();
        assert!(
            k.verifying_key()
                .verify_strict(
                    &widened_preimage,
                    &Signature::from_bytes(&response.signature)
                )
                .is_err()
        );

        // Verification yields the requested scope only; the registry is
        // unchanged and no binding or grant exists for the device key.
        let verified = issuer.verify(&registry, &response, NOW).unwrap();
        assert_eq!(verified.challenge, challenge);
        assert_eq!(
            registry.bound_key(&account()),
            Some(k.verifying_key().to_bytes())
        );
    }

    #[test]
    fn test_outstanding_challenges_are_bounded_and_pruned() {
        let (registry, _) = enrolled(1);
        let mut issuer = ChallengeIssuer::new("https://id.example", 1).unwrap();
        for _ in 0..3 {
            issuer
                .issue(
                    &registry,
                    &account(),
                    ChallengePurpose::LoginAcceptance,
                    NOW,
                )
                .unwrap();
        }
        assert_eq!(issuer.outstanding(), 3);
        issuer
            .issue(
                &registry,
                &account(),
                ChallengePurpose::LoginAcceptance,
                NOW + 5,
            )
            .unwrap();
        assert_eq!(issuer.outstanding(), 1);
    }
}
