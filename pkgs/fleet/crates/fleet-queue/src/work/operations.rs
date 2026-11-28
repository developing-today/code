//! Canonical Fleet command acceptance — DSP-01
//! (developing-today/code#9445; design parent #9203, program #9196).
//!
//! A signed GitHub webhook event can carry a `/fleet` command. The
//! projector that receives it is bounded transport: its delivery
//! table dedupes *projection application*, and that dedupe is not —
//! and must never be read as — command acceptance. This module is
//! the acceptance contract the queue-broker handler persists:
//!
//! * **Authenticated principal first.** An unauthenticated actor is
//!   refused before any grant is consulted; nothing is recorded.
//! * **Per-verb, per-resource authorization.** A grant names one
//!   principal, the verbs it may run, and the repositories it may
//!   run them on. Any mismatch refuses with a distinct reason and
//!   records nothing — a refusal is never a partial acceptance.
//! * **Durable idempotency, keyed semantically.** Acceptance is
//!   keyed by the command's idempotency key (repo + issue + comment
//!   + verb, minted by the forwarding projector). The GitHub
//!   delivery GUID rides along as provenance only: two deliveries
//!   of the same command — including a redelivery after the
//!   projector already applied the projection — produce exactly one
//!   effect and the second returns the stored receipt.
//!
//! This is a pure model, like its `work` siblings: it holds the
//! acceptance ledger in memory and does no I/O. The broker adapter
//! owns persisting the ledger; the projector's `pmp_deliveries`
//! keyspace is never consulted here.

/// REL-03 scoped ledger for the shared `fleet.command/v1` envelope
/// (developing-today/code#9513).
pub mod idempotency;

use std::collections::BTreeMap;

/// An actor identity as proven by the verified provider context.
/// `authenticated` is false for an actor the intake could not
/// authenticate; such a principal can never be authorized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// Stable actor login (e.g. the GitHub login from the signed
    /// webhook payload).
    pub login: String,
    /// Whether the intake authenticated this actor.
    pub authenticated: bool,
}

impl Principal {
    /// An actor the intake authenticated under `login`.
    pub fn authenticated(login: impl Into<String>) -> Self {
        Principal {
            login: login.into(),
            authenticated: true,
        }
    }

    /// An actor the intake could not authenticate.
    pub fn unauthenticated(login: impl Into<String>) -> Self {
        Principal {
            login: login.into(),
            authenticated: false,
        }
    }
}

/// One verb of the /fleet command grammar (#9203 §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Verb {
    Status,
    Triage,
    Start,
    Resume,
    Retry,
    Park,
    Unblock,
    Priority,
    DoNow,
    Merge,
    AgentStatus,
}

impl Verb {
    /// Parse the canonical token form used in command envelopes
    /// (`agent status` travels as `agent-status`).
    pub fn parse(token: &str) -> Option<Verb> {
        Some(match token {
            "status" => Verb::Status,
            "triage" => Verb::Triage,
            "start" => Verb::Start,
            "resume" => Verb::Resume,
            "retry" => Verb::Retry,
            "park" => Verb::Park,
            "unblock" => Verb::Unblock,
            "priority" => Verb::Priority,
            "do-now" => Verb::DoNow,
            "merge" => Verb::Merge,
            "agent-status" => Verb::AgentStatus,
            _ => return None,
        })
    }

    /// The canonical token form.
    pub fn token(self) -> &'static str {
        match self {
            Verb::Status => "status",
            Verb::Triage => "triage",
            Verb::Start => "start",
            Verb::Resume => "resume",
            Verb::Retry => "retry",
            Verb::Park => "park",
            Verb::Unblock => "unblock",
            Verb::Priority => "priority",
            Verb::DoNow => "do-now",
            Verb::Merge => "merge",
            Verb::AgentStatus => "agent-status",
        }
    }
}

/// The resource a command acts on: one issue/PR in one repository.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceRef {
    /// `owner/repo`.
    pub repo: String,
    /// Issue or PR number within the repository.
    pub number: u64,
}

impl ResourceRef {
    /// Parse the `owner/repo#number` form.
    pub fn parse(text: &str) -> Option<ResourceRef> {
        let (repo, number) = text.rsplit_once('#')?;
        if repo.is_empty() || !repo.contains('/') {
            return None;
        }
        Some(ResourceRef {
            repo: repo.to_string(),
            number: number.parse().ok()?,
        })
    }
}

/// One authorization grant: a principal, the verbs it may run, and
/// the repositories it may run them on (`"*"` matches any repo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// Principal login this grant belongs to.
    pub principal: String,
    /// Verbs the principal may run under this grant.
    pub verbs: Vec<Verb>,
    /// Repositories in scope: exact `owner/repo`, or `"*"`.
    pub repos: Vec<String>,
}

/// Why a command was refused. Every variant is non-mutating: no
/// receipt is stored and no effect is recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationError {
    /// The actor was not authenticated by the intake.
    Unauthenticated,
    /// No grant names this principal at all.
    NoGrant,
    /// The principal has grants, but none for this verb.
    VerbDenied,
    /// The principal may run this verb, but not on this repository.
    ResourceDenied,
}

/// The set of grants the authority decides against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthorizationPolicy {
    /// All grants in force.
    pub grants: Vec<Grant>,
}

impl AuthorizationPolicy {
    /// Decide whether `principal` may run `verb` on `resource`.
    /// Authentication is checked first; then a single grant must
    /// cover both the verb and the repository.
    pub fn authorize(
        &self,
        principal: &Principal,
        verb: Verb,
        resource: &ResourceRef,
    ) -> Result<(), AuthorizationError> {
        if !principal.authenticated {
            return Err(AuthorizationError::Unauthenticated);
        }
        let mine: Vec<&Grant> = self
            .grants
            .iter()
            .filter(|g| g.principal == principal.login)
            .collect();
        if mine.is_empty() {
            return Err(AuthorizationError::NoGrant);
        }
        if !mine.iter().any(|g| g.verbs.contains(&verb)) {
            return Err(AuthorizationError::VerbDenied);
        }
        let covered = mine.iter().any(|g| {
            g.verbs.contains(&verb) && g.repos.iter().any(|r| r == "*" || r == &resource.repo)
        });
        if !covered {
            return Err(AuthorizationError::ResourceDenied);
        }
        Ok(())
    }
}

/// One command presented for acceptance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// Semantic idempotency key minted by the forwarder
    /// (`ghcmd:v1:<repo>#<issue>:comment:<id>:<verb>`). This — never
    /// the delivery GUID — is the acceptance identity.
    pub idempotency_key: String,
    /// The authenticated actor asking for the command.
    pub principal: Principal,
    /// The verb to run.
    pub verb: Verb,
    /// The resource the verb acts on.
    pub resource: ResourceRef,
    /// Provenance only: the GitHub delivery that carried this
    /// attempt. Recorded on the effect for audit; never part of
    /// acceptance identity, so projector delivery dedupe state can
    /// neither create nor suppress an acceptance.
    pub delivery_id: String,
}

/// The stored proof that a command was accepted exactly once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    /// The command's idempotency key.
    pub idempotency_key: String,
    /// Monotonic acceptance sequence within the ledger.
    pub sequence: u64,
    /// The verb that was accepted.
    pub verb: Verb,
    /// The resource the verb acts on.
    pub resource: ResourceRef,
}

/// One applied command effect, appended on first acceptance only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effect {
    /// The accepted command's idempotency key.
    pub idempotency_key: String,
    /// Who ran it.
    pub principal: String,
    /// The verb that ran.
    pub verb: Verb,
    /// The resource it acted on.
    pub resource: ResourceRef,
    /// Provenance: the delivery that carried the accepted attempt.
    pub delivery_id: String,
}

/// The outcome of an acceptance attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptOutcome {
    /// The stored receipt for this command.
    pub receipt: Receipt,
    /// True when the receipt already existed: this attempt changed
    /// nothing and the caller is being handed back the original
    /// acceptance.
    pub duplicate: bool,
}

/// The canonical acceptance ledger. First acceptance of an
/// idempotency key appends exactly one effect and stores its
/// receipt; every later presentation of the same key returns the
/// stored receipt. Authorization is evaluated before the ledger is
/// touched, and a refusal leaves the ledger byte-identical.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AcceptanceLedger {
    receipts: BTreeMap<String, Receipt>,
    effects: Vec<Effect>,
    next_sequence: u64,
}

impl AcceptanceLedger {
    /// An empty ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// The effects applied so far, in acceptance order.
    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    /// The stored receipt for an idempotency key, if accepted.
    pub fn receipt(&self, idempotency_key: &str) -> Option<&Receipt> {
        self.receipts.get(idempotency_key)
    }

    /// Authorize and accept `command`. A repeat of an already
    /// accepted key returns the stored receipt with
    /// `duplicate: true` and appends no effect — regardless of which
    /// delivery carried the repeat.
    pub fn accept(
        &mut self,
        policy: &AuthorizationPolicy,
        command: &Command,
    ) -> Result<AcceptOutcome, AuthorizationError> {
        policy.authorize(&command.principal, command.verb, &command.resource)?;
        if let Some(receipt) = self.receipts.get(&command.idempotency_key) {
            return Ok(AcceptOutcome {
                receipt: receipt.clone(),
                duplicate: true,
            });
        }
        self.next_sequence += 1;
        let receipt = Receipt {
            idempotency_key: command.idempotency_key.clone(),
            sequence: self.next_sequence,
            verb: command.verb,
            resource: command.resource.clone(),
        };
        self.receipts
            .insert(command.idempotency_key.clone(), receipt.clone());
        self.effects.push(Effect {
            idempotency_key: command.idempotency_key.clone(),
            principal: command.principal.login.clone(),
            verb: command.verb,
            resource: command.resource.clone(),
            delivery_id: command.delivery_id.clone(),
        });
        Ok(AcceptOutcome {
            receipt,
            duplicate: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> AuthorizationPolicy {
        AuthorizationPolicy {
            grants: vec![
                Grant {
                    principal: "dezren39".to_string(),
                    verbs: vec![Verb::DoNow, Verb::Park, Verb::Status],
                    repos: vec!["developing-today/code".to_string()],
                },
                Grant {
                    principal: "joyce".to_string(),
                    verbs: vec![Verb::Status],
                    repos: vec!["*".to_string()],
                },
            ],
        }
    }

    fn command(key: &str, login: &str, verb: Verb, repo: &str, delivery: &str) -> Command {
        Command {
            idempotency_key: key.to_string(),
            principal: Principal::authenticated(login),
            verb,
            resource: ResourceRef::parse(&format!("{repo}#754")).unwrap(),
            delivery_id: delivery.to_string(),
        }
    }

    /// DSP-01 acceptance: authenticated principal + per-verb /
    /// per-resource authorization, and durable command idempotency
    /// that owes nothing to projector delivery dedupe — the same
    /// command under a *different* delivery GUID still yields one
    /// effect and the stored receipt.
    #[test]
    fn pmo_dsp_01_command_acceptance_is_durable_separate_from_projection_delivery() {
        let policy = policy();
        let mut ledger = AcceptanceLedger::new();
        let key = "ghcmd:v1:developing-today/code#754:comment:92001:do-now";

        // Unauthenticated actor: refused before grants, nothing stored.
        let mut anon = command(key, "dezren39", Verb::DoNow, "developing-today/code", "d-0");
        anon.principal = Principal::unauthenticated("dezren39");
        assert_eq!(
            ledger.accept(&policy, &anon),
            Err(AuthorizationError::Unauthenticated)
        );
        assert!(ledger.effects().is_empty());
        assert!(ledger.receipt(key).is_none());

        // Authenticated but unknown principal: no grant.
        let stranger = command(key, "mallory", Verb::DoNow, "developing-today/code", "d-0");
        assert_eq!(
            ledger.accept(&policy, &stranger),
            Err(AuthorizationError::NoGrant)
        );

        // Known principal, verb outside every grant.
        let wrong_verb = command(key, "joyce", Verb::DoNow, "developing-today/code", "d-0");
        assert_eq!(
            ledger.accept(&policy, &wrong_verb),
            Err(AuthorizationError::VerbDenied)
        );

        // Known principal and verb, repository out of scope.
        let wrong_repo = command(
            key,
            "dezren39",
            Verb::DoNow,
            "developing-today/other",
            "d-0",
        );
        assert_eq!(
            ledger.accept(&policy, &wrong_repo),
            Err(AuthorizationError::ResourceDenied)
        );
        assert!(ledger.effects().is_empty());

        // First authorized presentation — carried by delivery d-1,
        // whose projection the projector has already applied. The
        // ledger neither knows nor cares: acceptance is recorded.
        let first = command(key, "dezren39", Verb::DoNow, "developing-today/code", "d-1");
        let accepted = ledger.accept(&policy, &first).unwrap();
        assert!(!accepted.duplicate);
        assert_eq!(accepted.receipt.sequence, 1);
        assert_eq!(ledger.effects().len(), 1);
        assert_eq!(ledger.effects()[0].delivery_id, "d-1");

        // Redelivery under a *different* delivery GUID (the
        // projector's own dedupe keyed on d-1 can neither confirm
        // nor deny this): same stored receipt, no second effect.
        let redelivered = command(key, "dezren39", Verb::DoNow, "developing-today/code", "d-2");
        let again = ledger.accept(&policy, &redelivered).unwrap();
        assert!(again.duplicate);
        assert_eq!(again.receipt, accepted.receipt);
        assert_eq!(ledger.effects().len(), 1);

        // A genuinely different command from the same principal is a
        // new acceptance with its own effect and sequence.
        let other = command(
            "ghcmd:v1:developing-today/code#754:comment:92002:park",
            "dezren39",
            Verb::Park,
            "developing-today/code",
            "d-3",
        );
        let second = ledger.accept(&policy, &other).unwrap();
        assert!(!second.duplicate);
        assert_eq!(second.receipt.sequence, 2);
        assert_eq!(ledger.effects().len(), 2);
    }

    #[test]
    fn pmo_dsp_01_verb_and_resource_forms_parse() {
        assert_eq!(Verb::parse("do-now"), Some(Verb::DoNow));
        assert_eq!(Verb::parse("agent-status"), Some(Verb::AgentStatus));
        assert_eq!(Verb::parse("frobnicate"), None);
        assert_eq!(Verb::DoNow.token(), "do-now");
        assert_eq!(
            ResourceRef::parse("developing-today/code#754"),
            Some(ResourceRef {
                repo: "developing-today/code".to_string(),
                number: 754,
            })
        );
        assert_eq!(ResourceRef::parse("no-number"), None);
        assert_eq!(ResourceRef::parse("repo#12"), None);
    }
}
