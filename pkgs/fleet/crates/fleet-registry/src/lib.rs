//! fleet-registry — the full domain registry for the `fleet` multicall
//! binary (epic developing-today/code#5412, design note §2/§3).
//!
//! This crate is strings and tables only: std-only, no I/O. It is compiled
//! into **every** variant — including the tiny `fleet-connect` build — so
//! any artifact can name the whole fleet: render the full help surface with
//! `[not in this build]` marks, and answer an invocation of a domain it
//! does not contain with the capability stub (exit 69, §3) instead of a
//! parser error.
//!
//! Three different sets of capabilities are kept distinct here, because
//! the dispatch layer's correctness depends on never confusing them:
//!
//! * the **compiled** set — what a build's Cargo features put in the
//!   artifact (supplied by the bin crate, which knows its `cfg`s);
//! * a **profile** — the capability ceiling a hyphenated variant name
//!   (`fleet-serve`, `fleet-connect`) imposes at invocation time
//!   (Refinement A: a name can only narrow, never widen);
//! * the **effective** set — compiled ∩ profile — the surface a given
//!   invocation actually has.

use std::collections::BTreeSet;
use std::fmt;

/// One unit of surface a build can have or lack. Capabilities are the
/// atoms of feature selection (§2), of the stub's "Enabled capabilities"
/// list (§3), and of the proxy hello frame's capability list (§4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    /// `fleet proxy connect` / `fleet proxy init` — the proxy client half.
    ProxyConnect,
    /// `fleet proxy serve` — the session broker.
    ProxyServe,
    /// `fleet proxy splice` — the raw TCP stream relay.
    ProxyRawSplice,
    /// `fleet queue …` — the repo-queue client surface.
    Queue,
    /// `fleet serve queue-broker` — the queue broker daemon role.
    QueueBroker,
    /// `fleet serve secrets-broker` — the secrets broker daemon role.
    SecretsBroker,
    /// `fleet search …` — atlas-search standalone/client surface.
    Search,
    /// `fleet serve search` — the atlas search service role.
    SearchServe,
    /// `fleet dispatch …` — thin client over pm-stack dispatch.
    Dispatch,
    /// `fleet dispatch-test …` — explicit-dispatch CI test surface.
    DispatchTest,
    /// `fleet laptop-agent …` — client for the (Python) laptop-agent daemon.
    LaptopAgent,
    /// `fleet secrets …` — resolve secrets by name (step 6, #5554):
    /// the fleet-secrets chain as a verb — broker first, `bws` fallback.
    Secrets,
    /// `fleet gh …` — the real `gh` CLI as a passthrough under a
    /// rotating token (personal + the developing.today App's
    /// installation tokens), code#8887.
    Gh,
}

impl Capability {
    /// The hyphenated id used in stub text, `fleet version`, `fleet
    /// capabilities` JSON, and the proxy hello frame.
    pub fn id(self) -> &'static str {
        match self {
            Capability::ProxyConnect => "proxy-connect",
            Capability::ProxyServe => "proxy-serve",
            Capability::ProxyRawSplice => "proxy-raw-splice",
            Capability::Queue => "queue",
            Capability::QueueBroker => "queue-broker",
            Capability::SecretsBroker => "secrets-broker",
            Capability::Search => "search",
            Capability::SearchServe => "search-serve",
            Capability::Dispatch => "dispatch",
            Capability::DispatchTest => "dispatch-test",
            Capability::LaptopAgent => "laptop-agent",
            Capability::Secrets => "secrets",
            Capability::Gh => "gh",
        }
    }

    /// Every capability, in registry order (the order help and version
    /// output list them in).
    pub const ALL: &'static [Capability] = &[
        Capability::ProxyConnect,
        Capability::ProxyServe,
        Capability::ProxyRawSplice,
        Capability::Queue,
        Capability::QueueBroker,
        Capability::SecretsBroker,
        Capability::Search,
        Capability::SearchServe,
        Capability::Dispatch,
        Capability::DispatchTest,
        Capability::LaptopAgent,
        Capability::Secrets,
        Capability::Gh,
    ];
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// A set of capabilities. A `BTreeSet` so every rendering (stub text,
/// version, JSON) is deterministic.
pub type CapabilitySet = BTreeSet<Capability>;

pub fn capability_set(caps: &[Capability]) -> CapabilitySet {
    caps.iter().copied().collect()
}

/// The `fleet` (full) build: every domain compiled in (§2).
pub fn full_capabilities() -> CapabilitySet {
    Capability::ALL.iter().copied().collect()
}

/// The `fleet-serve` build: daemon roles + proxy, no working-seat clients
/// (§2: registry, config, secrets, proxy serve+connect, queue brokers,
/// search serve only).
pub fn serve_capabilities() -> CapabilitySet {
    capability_set(&[
        Capability::ProxyConnect,
        Capability::ProxyServe,
        Capability::ProxyRawSplice,
        Capability::QueueBroker,
        Capability::SecretsBroker,
        Capability::SearchServe,
        Capability::Secrets,
    ])
}

/// The `fleet-connect` build: the proxy client and nothing else (§2).
pub fn connect_capabilities() -> CapabilitySet {
    capability_set(&[Capability::ProxyConnect, Capability::ProxyRawSplice])
}

/// A shipped build variant (§2). The variant is a property of the
/// artifact; the same names double as invocation profiles (Refinement A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Full,
    Serve,
    Connect,
}

impl Variant {
    /// The name the variant is installed under where several variants sit
    /// side by side (package/attr level — §2, Refinement A). Wherever a
    /// variant is actually used it is renamed plain `fleet`.
    pub fn name(self) -> &'static str {
        match self {
            Variant::Full => "fleet",
            Variant::Serve => "fleet-serve",
            Variant::Connect => "fleet-connect",
        }
    }

    /// The capabilities this variant compiles in — also the profile
    /// ceiling when the name is used at invocation time.
    pub fn capabilities(self) -> CapabilitySet {
        match self {
            Variant::Full => full_capabilities(),
            Variant::Serve => serve_capabilities(),
            Variant::Connect => connect_capabilities(),
        }
    }

    /// The variant a hyphenated invocation name selects, if any.
    /// Plain `fleet` selects no profile (the build's full surface).
    pub fn from_invocation_name(name: &str) -> Option<Variant> {
        match name {
            "fleet-serve" => Some(Variant::Serve),
            "fleet-connect" => Some(Variant::Connect),
            _ => None,
        }
    }
}

/// One fleet domain: the unit help lists, stubs name, and config disables.
#[derive(Debug, Clone, Copy)]
pub struct Domain {
    /// Canonical subcommand name (`fleet <name> …`).
    pub name: &'static str,
    /// One-line description for help and the registry JSON.
    pub summary: &'static str,
    /// Capabilities that make up this domain's surface. Headline
    /// availability (help marks, domain stubs) keys on `primary` below;
    /// individual subcommands and daemon roles gate on their specific
    /// capability from this list.
    pub capabilities: &'static [Capability],
    /// The capability the domain's headline surface needs. Stubs and
    /// `[not in this build]` marks key on this: on `fleet-serve`, the
    /// `search` domain is marked absent because its headline surface is
    /// the standalone client, even though `search-serve` exists under
    /// `serve`.
    pub primary: Capability,
    /// argv0 names that route to this domain (§3 argv0 map).
    pub aliases: &'static [&'static str],
    /// Subcommand names, for domain help. Descriptive only in step 1.
    pub subcommands: &'static [&'static str],
}

impl Domain {
    /// Is this domain's headline surface in the given capability set?
    pub fn available_in(&self, caps: &CapabilitySet) -> bool {
        caps.contains(&self.primary)
    }
}

/// The full registry, in help order. Every variant renders all of it.
pub const DOMAINS: &[Domain] = &[
    Domain {
        name: "queue",
        summary: "Repo work queue: enqueue, claim, and complete tasks through the queue broker.",
        capabilities: &[Capability::Queue],
        primary: Capability::Queue,
        aliases: &["repo-queue"],
        subcommands: &[
            "validate",
            "list",
            "enqueue",
            "claim",
            "lane",
            "bind",
            "heartbeat",
            "checkpoint",
            "park",
            "block",
            "cancel",
            "fail",
            "complete",
            "health",
            "signal",
            "fallback",
        ],
    },
    Domain {
        name: "serve",
        summary: "Run a fleet daemon role (queue-broker, secrets-broker, search, proxy) in the foreground.",
        capabilities: &[
            Capability::QueueBroker,
            Capability::SecretsBroker,
            Capability::SearchServe,
            Capability::ProxyServe,
        ],
        primary: Capability::QueueBroker,
        aliases: &["queue-broker", "secrets-broker"],
        subcommands: &["queue-broker", "secrets-broker", "search", "proxy"],
    },
    Domain {
        name: "proxy",
        summary: "The session proxy: serve the broker socket, connect or init through it, run recovery SSH, or splice raw TCP streams.",
        capabilities: &[
            Capability::ProxyServe,
            Capability::ProxyConnect,
            Capability::ProxyRawSplice,
        ],
        primary: Capability::ProxyConnect,
        aliases: &["laptop-agent-broker"],
        subcommands: &["serve", "connect", "init", "ssh", "splice"],
    },
    Domain {
        name: "dispatch",
        summary: "Thin client over pm-stack dispatch: status, launch, and enqueue verbs.",
        capabilities: &[Capability::Dispatch],
        primary: Capability::Dispatch,
        aliases: &[],
        subcommands: &["status", "launch", "enqueue"],
    },
    Domain {
        name: "dispatch-test",
        summary: "Explicit-dispatch CI test: scan fleet repos for free Actions capacity, submit a job, track its stored status.",
        capabilities: &[Capability::DispatchTest],
        primary: Capability::DispatchTest,
        aliases: &[],
        subcommands: &["scan", "submit", "status"],
    },
    Domain {
        name: "secrets",
        summary: "Resolve secrets by name: the laptop secrets-broker first, Bitwarden as fallback.",
        capabilities: &[Capability::Secrets],
        primary: Capability::Secrets,
        aliases: &[],
        subcommands: &["get", "list"],
    },
    Domain {
        name: "gh",
        summary: "The real `gh` CLI, passed through under a rotating fleet token (personal, then the developing.today App's installations); rate-limit and permission failures fail over to the next token.",
        capabilities: &[Capability::Gh],
        primary: Capability::Gh,
        aliases: &[],
        subcommands: &[],
    },
    Domain {
        name: "search",
        summary: "Atlas corpus search over the session history (Snowflake hybrid, via atlas-search).",
        capabilities: &[Capability::Search],
        primary: Capability::Search,
        aliases: &["atlas-search"],
        subcommands: &["search", "get-chunk"],
    },
    Domain {
        name: "laptop-agent",
        summary: "Client for the laptop-agent daemon: run and manage jobs on the laptop.",
        capabilities: &[Capability::LaptopAgent],
        primary: Capability::LaptopAgent,
        aliases: &["laptop-agent"],
        subcommands: &["run", "status", "poll", "kill"],
    },
];

/// Look a domain up by canonical name.
pub fn domain(name: &str) -> Option<&'static Domain> {
    DOMAINS.iter().find(|d| d.name == name)
}

/// Where an argv0 alias routes: the domain, plus a fixed first argument
/// the alias implies (e.g. `queue-broker` means `serve queue-broker`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AliasRoute {
    pub domain: &'static str,
    pub implied_arg: Option<&'static str>,
}

/// The §3 argv0 map. Aliases not listed here are not multicall names.
pub fn alias_route(argv0: &str) -> Option<AliasRoute> {
    match argv0 {
        "repo-queue" => Some(AliasRoute {
            domain: "queue",
            implied_arg: None,
        }),
        "queue-broker" => Some(AliasRoute {
            domain: "serve",
            implied_arg: Some("queue-broker"),
        }),
        "secrets-broker" => Some(AliasRoute {
            domain: "serve",
            implied_arg: Some("secrets-broker"),
        }),
        "laptop-agent-broker" => Some(AliasRoute {
            domain: "proxy",
            implied_arg: Some("serve"),
        }),
        "atlas-search" => Some(AliasRoute {
            domain: "search",
            implied_arg: None,
        }),
        "laptop-agent" => Some(AliasRoute {
            domain: "laptop-agent",
            implied_arg: None,
        }),
        _ => None,
    }
}

/// Every argv0 name the multicall dispatch recognizes, including `fleet`
/// itself and the hyphenated variant names.
pub fn is_known_invocation_name(name: &str) -> bool {
    name == "fleet" || Variant::from_invocation_name(name).is_some() || alias_route(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_ids_are_unique_and_stable() {
        let mut ids: Vec<&str> = Capability::ALL.iter().map(|c| c.id()).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "capability ids must be unique");
        assert!(ids.contains(&"proxy-connect"));
        assert!(ids.contains(&"proxy-raw-splice"));
        assert!(ids.contains(&"queue"));
    }

    #[test]
    fn variant_profiles_nest() {
        // connect ⊂ serve ⊂ full — names narrow, profiles must too.
        let full = full_capabilities();
        let serve = serve_capabilities();
        let connect = connect_capabilities();
        assert!(connect.is_subset(&serve));
        assert!(serve.is_subset(&full));
        assert_eq!(full.len(), Capability::ALL.len());
    }

    #[test]
    fn every_domain_primary_is_in_its_surface() {
        for d in DOMAINS {
            assert!(
                d.capabilities.contains(&d.primary),
                "domain {} primary {:?} not in its capabilities",
                d.name,
                d.primary
            );
        }
    }

    #[test]
    fn aliases_route_to_real_domains() {
        for name in [
            "repo-queue",
            "queue-broker",
            "secrets-broker",
            "laptop-agent-broker",
            "atlas-search",
            "laptop-agent",
        ] {
            let route = alias_route(name).unwrap_or_else(|| panic!("alias {name} must route"));
            assert!(
                domain(route.domain).is_some(),
                "alias {name} routes to unknown domain {}",
                route.domain
            );
        }
        assert!(alias_route("fleet").is_none());
        assert!(alias_route("bogus").is_none());
    }

    #[test]
    fn domain_aliases_match_the_argv0_map() {
        // The per-domain alias lists (rendered in help) and the routing
        // table are two views of one fact; keep them in lockstep.
        for d in DOMAINS {
            for alias in d.aliases {
                let route = alias_route(alias)
                    .unwrap_or_else(|| panic!("domain {} alias {alias} does not route", d.name));
                assert_eq!(route.domain, d.name, "alias {alias} routes elsewhere");
            }
        }
    }
}
