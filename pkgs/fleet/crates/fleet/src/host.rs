//! `fleet host <name>` — host resolution and selection over the
//! #13717 forms (developing-today/code#14716, transport & pooling
//! design §1.3, work item 1c).
//!
//! Resolving a host name yields candidate members: one per
//! (transport offer × invocation form) — the #13717 matrix as data.
//! Selection among candidates is a pool decision through the
//! fleet-pool primitive (#14721), never a hand-written per-host
//! ladder, and every result names the serving offer and form (the
//! pool's decision log, read back).
//!
//! The host records below mirror the reviewed registry artifact
//! `doc/fleet-machine-registry.yaml` (schema slice 1a, #14714):
//! registry `id`, human names, transport offers (endpoint data and
//! credential NAMES only — never values), and the enrollment table
//! saying which agent classes may use which offers. The live
//! overlay (1b, #14715) is not in this build yet, so load is
//! *unknown* end to end (#13611: unknown never sorts as zero) and
//! selection is the pool's ordered strategy over registry order;
//! when the overlay lands, its per-offer health/load feed this same
//! `resolve` as pool member state/load without a new ladder.
//!
//! `fleet laptop` is the N=1 legacy spelling: the name `laptop`
//! resolves to the `amd` record here, exactly as the tiered
//! `fleet laptop` walker serves that host today.

use fleet_pool::{NodeId, Ordered, Pool};

/// One transport offer on a host record (design §1.2): endpoint data
/// and credential names only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportOffer {
    pub name: &'static str,
    pub kind: &'static str,
}

/// A host record in the #2179 machine registry (design §1.2).
#[derive(Debug, Clone, Copy)]
pub struct HostRecord {
    /// Registry identity; every credential is bound to this id.
    pub id: &'static str,
    /// Human/DNS names — labels, not identity. The first is the
    /// canonical short name (`fleet host <name>` accepts any).
    pub names: &'static [&'static str],
    pub offers: &'static [TransportOffer],
    /// (agent class, offer names that class may use) — §1.2
    /// enrollment. A class with no row may use no offers.
    pub enrollment: &'static [(&'static str, &'static [&'static str])],
}

const AMD_OFFERS: &[TransportOffer] = &[
    TransportOffer {
        name: "commander-door",
        kind: "commander-door",
    },
    TransportOffer {
        name: "laptop-agent-cf",
        kind: "laptop-agent",
    },
    TransportOffer {
        name: "laptop-agent-tailnet",
        kind: "tsnet",
    },
    TransportOffer {
        name: "ssh",
        kind: "ssh",
    },
];

const AMD_FULL_LADDER: &[&str] = &[
    "laptop-agent-cf",
    "laptop-agent-tailnet",
    "ssh",
    "commander-door",
];

/// The registry as data. Mirrors `doc/fleet-machine-registry.yaml`;
/// a reviewed edit there is the source of truth, this table is its
/// compiled-in read path (the fleet-registry crate's pattern).
pub const HOSTS: &[HostRecord] = &[HostRecord {
    id: "amd-6",
    names: &["amd", "amd-6"],
    offers: AMD_OFFERS,
    enrollment: &[
        ("opencode-lanes", AMD_FULL_LADDER),
        ("muse-seats", AMD_FULL_LADDER),
        ("external-mcp-clients", &["commander-door"]),
    ],
}];

/// The agent class a bare `fleet host <name>` resolves for: this
/// binary's seats are muse seats unless a caller names a class.
pub const DEFAULT_AGENT_CLASS: &str = "muse-seats";

/// The invocation forms an offer kind yields, as #13717 form ids.
/// Every executable offer kind yields its method in both shapes
/// (fleet subcommand, raw command); the tiered T0 and the wrapper
/// T7 are walk shapes over the whole candidate set, not per-offer
/// candidates, so they never appear here.
fn forms_for_kind(kind: &str) -> &'static [&'static str] {
    match kind {
        // T1 commander/fleet, T4 commander/raw.
        "commander-door" => &["T1", "T4"],
        // T2 http/fleet, T5 http/raw — both laptop-agent legs are
        // the http method; the offer distinguishes the leg.
        "laptop-agent" | "tsnet" => &["T2", "T5"],
        // T3 ssh/fleet, T6 ssh/raw.
        "ssh" => &["T3", "T6"],
        _ => &[],
    }
}

/// One candidate member: one (transport offer × invocation form).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCandidate {
    /// Pool member id: `<host-id>/<offer>/<form>`.
    pub id: String,
    pub offer: &'static str,
    pub kind: &'static str,
    pub form: &'static str,
}

impl HostCandidate {
    /// The label every result names: offer and form together
    /// (design §1.3: every result names the serving form/offer).
    pub fn label(&self) -> String {
        format!("{} via {} ({})", self.form, self.offer, self.kind)
    }
}

/// A completed resolution: the selected candidate plus the pool's
/// decision-log entry that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub host_id: &'static str,
    pub candidates: Vec<HostCandidate>,
    pub selected: HostCandidate,
    /// The fleet-pool decision log, read back (member, strategy,
    /// reason) — a pool decision you cannot read back is a bug.
    pub decision: String,
}

/// Find a host record by any of its names, its registry id, or the
/// legacy `laptop` alias for the `amd` record.
pub fn host_record(name: &str) -> Option<&'static HostRecord> {
    let name = if name == "laptop" { "amd" } else { name };
    HOSTS
        .iter()
        .find(|h| h.id == name || h.names.contains(&name))
}

/// The candidate members for a record and agent class: one per
/// (offer the class is enrolled for × invocation form of the
/// offer's kind), in registry order.
pub fn candidates(record: &HostRecord, agent_class: &str) -> Vec<HostCandidate> {
    let allowed: &[&str] = record
        .enrollment
        .iter()
        .find(|(class, _)| *class == agent_class)
        .map(|(_, offers)| *offers)
        .unwrap_or(&[]);
    let mut out = Vec::new();
    for offer in record.offers {
        if !allowed.contains(&offer.name) {
            continue;
        }
        for form in forms_for_kind(offer.kind) {
            out.push(HostCandidate {
                id: format!("{}/{}/{}", record.id, offer.name, form),
                offer: offer.name,
                kind: offer.kind,
                form,
            });
        }
    }
    out
}

/// Resolve a host name into candidates and select through the
/// fleet-pool primitive. `now` is logical seconds for the pool's
/// decision log. `None` for an unknown host or a class enrolled
/// for no offers.
pub fn resolve(name: &str, agent_class: &str, now: u64) -> Option<Resolution> {
    let record = host_record(name)?;
    let candidates = candidates(record, agent_class);
    if candidates.is_empty() {
        return None;
    }
    let mut pool = Pool::new();
    pool.add_group(NodeId::new(record.id), None);
    for candidate in &candidates {
        pool.add_member(
            NodeId::new(candidate.id.clone()),
            Some(&NodeId::new(record.id)),
        );
    }
    // No live overlay in this build (1b, #14715): load is unknown
    // for every member, never zero (#13611).
    let selected_id = pool.select(&mut Ordered, &|_| None, now)?;
    let selected = candidates
        .iter()
        .find(|c| c.id == selected_id.as_str())?
        .clone();
    let decision = pool
        .decisions()
        .last()
        .map(|d| {
            format!(
                "member={} strategy={} reason={}",
                d.member.as_ref().map(|m| m.as_str()).unwrap_or("none"),
                d.strategy,
                d.reason
            )
        })
        .unwrap_or_default();
    Some(Resolution {
        host_id: record.id,
        candidates,
        selected,
        decision,
    })
}

const HOST_USAGE: &str = "\
Usage: fleet host <name> [--class <agent-class>]

Resolve a host name through the machine registry into candidate
members — one per (transport offer × invocation form), the #13717
matrix as data — and select through the fleet-pool primitive.
Every result names the serving offer and form. The name `laptop`
resolves to the `amd` record. Agent classes: muse-seats (default),
opencode-lanes, external-mcp-clients.";

fn usage_error(message: &str) -> i32 {
    eprintln!("fleet host: {message}\n\n{HOST_USAGE}");
    2
}

/// `fleet host …` — argument handling is strict: a name, an optional
/// `--class`, `--help`, and nothing else.
pub fn run(args: &[String]) -> i32 {
    let mut name: Option<&str> = None;
    let mut agent_class = DEFAULT_AGENT_CLASS;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" | "help" => {
                println!("{HOST_USAGE}");
                return 0;
            }
            "--class" => {
                let Some(value) = args.get(i + 1) else {
                    return usage_error("--class needs an agent class");
                };
                agent_class = value;
                i += 2;
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown argument {other:?}"));
            }
            other => {
                if name.is_some() {
                    return usage_error(&format!("unexpected extra argument {other:?}"));
                }
                name = Some(other);
                i += 1;
            }
        }
    }
    let Some(name) = name else {
        return usage_error("a host name is required");
    };
    match resolve(name, agent_class, 0) {
        Some(resolution) => {
            println!("host: {} ({})", name, resolution.host_id);
            for candidate in &resolution.candidates {
                let marker = if candidate.id == resolution.selected.id {
                    "*"
                } else {
                    " "
                };
                println!("{marker} {}", candidate.label());
            }
            println!(
                "selected: {}",
                resolution.selected.label()
            );
            println!("decision: {}", resolution.decision);
            0
        }
        None => {
            eprintln!(
                "fleet host: no candidates for {name:?} as {agent_class} \
                 (unknown host, or the class is enrolled for no offers)"
            );
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Oracle (design §1.3): resolving a host yields one candidate
    // per (transport offer × invocation form).
    #[test]
    fn candidates_are_one_per_offer_times_form() {
        let record = host_record("amd").expect("amd record");
        let got = candidates(record, "muse-seats");
        assert_eq!(got.len(), 8, "4 offers × 2 forms: {got:?}");
        let mut ids: Vec<&str> = got.iter().map(|c| c.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 8, "candidate ids are unique: {got:?}");
    }

    // Oracle (design §1.2): enrollment gates offers per agent
    // class — hosted MCP clients speak HTTPS only and get the
    // public door alone.
    #[test]
    fn enrollment_limits_external_mcp_clients_to_the_door() {
        let record = host_record("amd").expect("amd record");
        let got = candidates(record, "external-mcp-clients");
        assert_eq!(got.len(), 2, "commander-door × T1/T4: {got:?}");
        assert!(got.iter().all(|c| c.offer == "commander-door"));
        assert!(candidates(record, "no-such-class").is_empty());
    }

    // Oracle (acceptance #14716): `fleet laptop` keeps working as
    // an alias for the `amd` record.
    #[test]
    fn laptop_alias_resolves_to_the_amd_record() {
        assert_eq!(host_record("laptop").map(|h| h.id), Some("amd-6"));
        let via_alias = resolve("laptop", "muse-seats", 0).expect("alias resolves");
        let via_name = resolve("amd", "muse-seats", 0).expect("name resolves");
        assert_eq!(via_alias, via_name);
    }

    // Oracle (acceptance #14716): selection is a pool decision and
    // every result names the serving form/offer.
    #[test]
    fn resolution_names_the_serving_offer_and_form() {
        let resolution = resolve("amd", "muse-seats", 7).expect("resolves");
        assert!(resolution.candidates.contains(&resolution.selected));
        let label = resolution.selected.label();
        assert!(
            label.contains(resolution.selected.offer)
                && label.contains(resolution.selected.form),
            "label names offer and form: {label}"
        );
        assert!(
            resolution.decision.contains(&resolution.selected.id)
                && resolution.decision.contains("strategy=ordered"),
            "decision log names the selected member: {}",
            resolution.decision
        );
    }

    #[test]
    fn unknown_host_resolves_to_nothing() {
        assert!(host_record("no-such-host").is_none());
        assert!(resolve("no-such-host", "muse-seats", 0).is_none());
    }
}
