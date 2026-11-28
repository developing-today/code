//! Append-list submission vocabulary for A-GRAPH-9127-02 (#10237).
//!
//! This leaf owns only the signed-submission shape and state identifiers.
//! Admission, membership and revocation rechecks live in `id-events` so the
//! graph model cannot copy or weaken the authorization interface.

/// Stable identifier for a pending submission.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PendingId(String);

impl PendingId {
    /// Borrow the stable identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for PendingId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for PendingId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// A signed append submission after its signature verifier has run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttributeSubmission {
    /// Stable submission identity.
    pub id: String,
    /// Caller-chosen idempotency key.
    pub idempotency_key: String,
    /// Exact signed bytes, retained for idempotency conflict checks.
    pub bytes: Vec<u8>,
    /// Signer identity bound by the verified evidence.
    pub signer: String,
    /// Result of the signature/evidence verifier.
    pub signature_valid: bool,
}

/// Result of submitting a valid signed append.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdmissionOutcome {
    /// The submission waits for approval.
    Pending(PendingId),
    /// The submission was directly accepted.
    Accepted,
}
