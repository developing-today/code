//! Cross-target canonical order hash — DST-06
//! (developing-today/code#9456; design parent #9200, program #9196).
//!
//! Pure offline contract slice of the PM orchestration design
//! (`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
//! `components/9200-priority-distribution.md`, "Cross-target order
//! and reconciliation"): given one complete eligible lane snapshot,
//! compute the deterministic `order_hash` every target must agree
//! on. No I/O, no queue, no broker, no persistence — a partial
//! candidate set cannot claim full-order conformance and is a typed
//! error, never a hash.
//!
//! Framing is normative from the architecture digest/serialization
//! contract and shared with [`crate::work::contracts::digest`]:
//! fixed ASCII purpose (`fleet.selection-order/v1`) + exactly one
//! NUL byte + RFC 8785 canonical UTF-8 of the DTO, SHA-256, then
//! `sha256:` plus 64 lowercase hex characters. The hash covers the
//! snapshot/policy/graph inputs AND every ordered item's comparator
//! keys (authority-qualified identity, effective urgency, exact peer
//! rank, creation time) — never a top-page display subset, and never
//! target-local references. Numeric work-ID ties preserve `9 < 10`
//! despite the string wire encoding
//! ([`cmp_work_id`]/[`sort_by_canonical_identity`]).

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use super::contracts::{digest, ContractError, DIGEST_DOMAIN_SELECTION_ORDER};

/// One ordered candidate in comparator order (effective urgency
/// ascending, then exact peer rank, creation time, normalized
/// canonical identity, per the #9200 component). The hash is
/// order-sensitive: the same set in a different order is a different
/// observed order and hashes differently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderedItem {
    /// Canonical authority scope (bytewise identity).
    pub authority_id: String,
    /// Canonical work ID as a decimal string (never coerced through
    /// float; `9 < 10` numerically despite the string encoding).
    pub work_id: String,
    /// Effective urgency from the pinned policy/snapshot inputs.
    /// Lower claims sooner.
    pub effective_urgency: i64,
    /// Exact peer rank within equal effective urgency.
    pub peer_rank: String,
    /// Creation instant (UTC, pinned string).
    pub created_at: String,
}

/// The snapshot inputs the order was calculated from. Every field is
/// part of the hash: the same items under a different
/// policy/profile/graph snapshot are a different observed order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderSnapshot {
    /// Priority profile version (e.g. `3`).
    pub profile_version: String,
    /// Priority taxonomy mapping version (e.g. `2`).
    pub taxonomy_version: String,
    /// Effective-policy version the pressure was calculated from.
    pub policy_version: String,
    /// Queue high-water mark identifying the exact candidate set.
    pub queue_rev: String,
    /// Dependency-graph snapshot revision.
    pub graph_rev: String,
    /// Eligible lane this slice was selected for.
    pub lane: String,
    /// Read snapshot / as-of instant (UTC, pinned string).
    pub as_of: String,
}

/// Input to [`selection_order_hash`]. `complete` must be true: only
/// a full-population eligible slice may claim
/// `fleet.selection-order/v1` conformance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionOrderInput {
    /// Snapshot/policy inputs shared by every item.
    pub snapshot: OrderSnapshot,
    /// Candidates in comparator order.
    pub items: Vec<OrderedItem>,
    /// True only for a full-population eligible slice. A partial
    /// candidate set (top page, truncated fetch) hashes to a typed
    /// error, never to an order claim.
    pub complete: bool,
}

/// Why a candidate set cannot claim full-order conformance: the
/// producer holds one page of a paged population, a truncated
/// fetch, or a population whose completeness it cannot state
/// (DST-06 record §3.1). A partial population stays displayable
/// but is never hashable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartialReason {
    /// Only one page of a paged population is held.
    Paginated,
    /// The fetch was cut off before the population was complete.
    Truncated,
    /// The producer cannot state whether the set is complete.
    UnknownCompleteness,
}

impl PartialReason {
    /// Stable machine code for the reason, suitable for logs and
    /// refusal details.
    pub fn code(&self) -> &'static str {
        match self {
            PartialReason::Paginated => "paginated",
            PartialReason::Truncated => "truncated",
            PartialReason::UnknownCompleteness => "unknown_completeness",
        }
    }

    /// One-line human sentence carried in the refusal error.
    pub fn detail(&self) -> String {
        match self {
            PartialReason::Paginated => "population is one page of a paged fetch".to_string(),
            PartialReason::Truncated => "population is a truncated fetch".to_string(),
            PartialReason::UnknownCompleteness => {
                "population completeness cannot be stated".to_string()
            }
        }
    }
}

/// An order population known to be partial: a page, a truncated
/// fetch, or a population of unknown completeness. Carries the
/// same items for display, with the reason stated. Deliberately
/// exposes no hash: there is no function that hashes a
/// `PartialOrder`, so a page or truncated fetch cannot produce a
/// conformant `order_hash` by accident (DST-06 record §3.1, §8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialOrder {
    /// Candidates held, in display order (never hashed).
    pub items: Vec<OrderedItem>,
    /// Why the set cannot claim full-order conformance.
    pub reason: PartialReason,
}

impl SelectionOrderInput {
    /// Refuse completion of a partial population: attempting to
    /// complete a `PartialOrder` is a typed `invalid_schema` error
    /// naming the reason, never an order claim. Reuses the shared
    /// `ContractError` so no `contracts.rs` change is needed (the
    /// DST-07 record's G1 landing order leaves that file to DST-04).
    pub fn from_partial(partial: PartialOrder) -> Result<Self, ContractError> {
        Err(ContractError::InvalidSchema {
            detail: format!(
                "partial candidate set cannot claim full-order conformance: [{}] {}",
                partial.reason.code(),
                partial.reason.detail()
            ),
        })
    }
}

/// Compare two canonical work IDs numerically when both are decimal
/// strings (`9 < 10`), falling back to bytewise order otherwise.
/// Integer-like IDs are never coerced through floating point.
pub fn cmp_work_id(a: &str, b: &str) -> Ordering {
    match (parse_decimal_id(a), parse_decimal_id(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => a.cmp(b),
    }
}

/// Sort items by normalized canonical identity: authority bytewise,
/// then work ID by integer value. This is the final tie-break of
/// the comparator, not a replacement for it.
pub fn sort_by_canonical_identity(items: &mut [OrderedItem]) {
    items.sort_by(|a, b| {
        a.authority_id
            .cmp(&b.authority_id)
            .then_with(|| cmp_work_id(&a.work_id, &b.work_id))
    });
}

fn parse_decimal_id(value: &str) -> Option<u128> {
    if value.is_empty() {
        return None;
    }
    if value.as_bytes()[0] == b'0' && value.len() > 1 {
        return None;
    }
    if !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse::<u128>().ok()
}

/// Build the canonical DTO hashed by [`selection_order_hash`].
/// Exposed so cross-target fixtures and the framing oracle can
/// reconstruct and independently verify the exact bytes.
pub fn selection_order_dto(input: &SelectionOrderInput) -> serde_json::Value {
    serde_json::json!({
        "purpose": DIGEST_DOMAIN_SELECTION_ORDER,
        "snapshot": {
            "profile_version": input.snapshot.profile_version,
            "taxonomy_version": input.snapshot.taxonomy_version,
            "policy_version": input.snapshot.policy_version,
            "queue_rev": input.snapshot.queue_rev,
            "graph_rev": input.snapshot.graph_rev,
            "lane": input.snapshot.lane,
            "as_of": input.snapshot.as_of,
        },
        "items": input.items.iter().map(|item| serde_json::json!({
            "authority_id": item.authority_id,
            "work_id": item.work_id,
            "effective_urgency": item.effective_urgency,
            "peer_rank": item.peer_rank,
            "created_at": item.created_at,
        })).collect::<Vec<_>>(),
    })
}

/// Compute the canonical cross-target order hash for one complete
/// eligible lane snapshot. Deterministic: the same
/// authority/profile/snapshot/items yield byte-identical output on
/// every target. Any included-input change (snapshot field, item
/// field, item order) changes the hash. Version/target divergence
/// is observed as hash inequality, never coerced.
pub fn selection_order_hash(input: &SelectionOrderInput) -> Result<String, ContractError> {
    if !input.complete {
        return Err(ContractError::InvalidSchema {
            detail: "partial candidate set cannot claim full-order conformance".to_string(),
        });
    }
    digest(DIGEST_DOMAIN_SELECTION_ORDER, &selection_order_dto(input))
}

#[cfg(test)]
mod tests {
    use super::super::contracts::is_digest_string;
    use super::*;

    fn fixture_snapshot() -> OrderSnapshot {
        OrderSnapshot {
            profile_version: "3".to_string(),
            taxonomy_version: "2".to_string(),
            policy_version: "policy:v7".to_string(),
            queue_rev: "queue:rev:118".to_string(),
            graph_rev: "graph:rev:57".to_string(),
            lane: "fleet-default".to_string(),
            as_of: "2026-11-13T03:30:00Z".to_string(),
        }
    }

    fn fixture_items() -> Vec<OrderedItem> {
        vec![
            OrderedItem {
                authority_id: "fleet-local".to_string(),
                work_id: "842".to_string(),
                effective_urgency: 15,
                peer_rank: "a".to_string(),
                created_at: "2026-11-12T10:00:00Z".to_string(),
            },
            OrderedItem {
                authority_id: "fleet-local".to_string(),
                work_id: "843".to_string(),
                effective_urgency: 20,
                peer_rank: "a".to_string(),
                created_at: "2026-11-12T10:01:00Z".to_string(),
            },
        ]
    }

    fn fixture_input() -> SelectionOrderInput {
        SelectionOrderInput {
            snapshot: fixture_snapshot(),
            items: fixture_items(),
            complete: true,
        }
    }

    /// DST-06 oracle (#9456): same authority/profile/snapshot yields
    /// byte-identical canonical IDs and hash on every target;
    /// version/target divergence fails (hash inequality); framing
    /// matches the normative domain/NUL/RFC-8785/SHA-256 bytes;
    /// numeric work-ID ties preserve 9 < 10; a partial set is a
    /// typed error, never an order claim. Oracle source: the #9200
    /// component's "Cross-target order and reconciliation" section
    /// and the architecture digest/serialization contract — the
    /// independent framing recomputation below copies those
    /// documents, not this module's implementation.
    #[test]
    fn pmo_distribution_host_order_hash() {
        // Same logical input built twice on "two targets" hashes
        // byte-identically.
        let first = selection_order_hash(&fixture_input()).unwrap();
        let second = selection_order_hash(&fixture_input()).unwrap();
        assert_eq!(first, second);
        assert!(is_digest_string(&first));

        // Framing vector: independently recompute the exact wire
        // bytes — ASCII domain, one NUL, canonical JSON — and hash
        // with SHA-256 directly, without touching this module's
        // digest path.
        let dto = selection_order_dto(&fixture_input());
        let canonical = super::super::contracts::canonical_json(&dto).unwrap();
        let mut hasher = sha2::Sha256::new();
        use sha2::Digest as _;
        hasher.update(DIGEST_DOMAIN_SELECTION_ORDER.as_bytes());
        hasher.update([0u8]);
        hasher.update(canonical.as_bytes());
        let bytes: [u8; 32] = hasher.finalize().into();
        let mut expected = String::from("sha256:");
        for byte in bytes {
            expected.push_str(&format!("{byte:02x}"));
        }
        assert_eq!(first, expected);

        // Included-input changes diverge: policy version bump,
        // item-field change, and item reorder each change the hash.
        let mut bumped = fixture_input();
        bumped.snapshot.policy_version = "policy:v8".to_string();
        assert_ne!(selection_order_hash(&bumped).unwrap(), first);

        let mut rescored = fixture_input();
        rescored.items[0].effective_urgency = 16;
        assert_ne!(selection_order_hash(&rescored).unwrap(), first);

        let mut reordered = fixture_input();
        reordered.items.reverse();
        assert_ne!(selection_order_hash(&reordered).unwrap(), first);

        // Target divergence on taxonomy version fails equality too.
        let mut retaxed = fixture_input();
        retaxed.snapshot.taxonomy_version = "3".to_string();
        assert_ne!(selection_order_hash(&retaxed).unwrap(), first);

        // Numeric work-ID ties: 9 < 10 despite string encoding, and
        // the canonical sort applies it after authority order.
        assert_eq!(cmp_work_id("9", "10"), Ordering::Less);
        assert_eq!(cmp_work_id("10", "9"), Ordering::Greater);
        assert_eq!(cmp_work_id("9", "9"), Ordering::Equal);
        let mut ties = vec![
            OrderedItem {
                authority_id: "fleet-local".to_string(),
                work_id: "10".to_string(),
                effective_urgency: 15,
                peer_rank: "a".to_string(),
                created_at: "2026-11-12T10:00:00Z".to_string(),
            },
            OrderedItem {
                authority_id: "fleet-local".to_string(),
                work_id: "9".to_string(),
                effective_urgency: 15,
                peer_rank: "a".to_string(),
                created_at: "2026-11-12T10:00:00Z".to_string(),
            },
        ];
        sort_by_canonical_identity(&mut ties);
        assert_eq!(
            ties.iter()
                .map(|item| item.work_id.as_str())
                .collect::<Vec<_>>(),
            vec!["9", "10"]
        );

        // A partial candidate set cannot claim full-order
        // conformance: typed invalid_schema, no hash.
        let mut partial = fixture_input();
        partial.complete = false;
        let err = selection_order_hash(&partial).unwrap_err();
        assert_eq!(err.code(), "invalid_schema");
        assert!(format!("{err:?}").contains("full-order conformance"));
    }

    /// DST-06 oracle test 2 (#9456): a producer holding only a
    /// page, a truncated fetch, or a population of unknown
    /// completeness builds a `PartialOrder` stating the reason.
    /// `PartialOrder` exposes no hash function by construction, and
    /// completing one is a typed `invalid_schema` refusal naming the
    /// reason — never an order claim. Oracle source: the DST-06
    /// design record §3.1 ("only one can be hashed") and §8
    /// ("Producer has only a page: PartialOrder, no hash").
    #[test]
    fn pmo_distribution_partial_order_has_no_hash() {
        // Reason codes are stable: logs and refusal details can
        // match on them.
        assert_eq!(PartialReason::Paginated.code(), "paginated");
        assert_eq!(PartialReason::Truncated.code(), "truncated");
        assert_eq!(
            PartialReason::UnknownCompleteness.code(),
            "unknown_completeness"
        );

        // Every stated reason refuses completion with its name in
        // the typed error, and yields no input that could hash.
        for reason in [
            PartialReason::Paginated,
            PartialReason::Truncated,
            PartialReason::UnknownCompleteness,
        ] {
            let partial = PartialOrder {
                items: fixture_items(),
                reason,
            };
            let err = SelectionOrderInput::from_partial(partial).unwrap_err();
            assert_eq!(err.code(), "invalid_schema");
            let rendered = format!("{err:?}");
            assert!(
                rendered.contains(reason.code()),
                "refusal names the reason: {rendered}"
            );
            assert!(
                rendered.contains("full-order conformance"),
                "refusal states the rule: {rendered}"
            );
        }
    }
}
