//! fleet — one multicall binary for the fleet's tools
//! (epic developing-today/code#5412; design note
//! doc/2026-11-04T11-45-48Z_design_fleet_multicall_binary/).
//!
//! Step 1 (#5429) is the skeleton: the platform crates, this dispatch
//! layer, the capability-stub machinery, and the variant builds. No
//! domain is implemented yet — a compiled-in domain answers with the
//! "not implemented yet" form of the stub — but the *surface* is
//! complete and honest: every variant renders the full registry, and
//! every missing capability answers with a branchable stub, never a
//! parser error.
//!
//! Dispatch order (§3), extended by Refinement A (name-scoping):
//!
//!   argv0 alias → domain route
//!   argv0 variant name (`fleet-serve`/`fleet-connect`) → root dispatch
//!     with that profile as a capability ceiling (narrow, never widen)
//!   argv0 `fleet` → root dispatch
//!   domain → capability compiled in? in profile? not config-disabled?
//!     → domain (step 1: the not-implemented stub) : capability stub
//!   unknown name → usage error, exit 2
//!
//! Exit codes: 0 ok · 2 usage/unknown (clap's code) · **69**
//! `EX_UNAVAILABLE` for every capability stub (D3) — "not here", with
//! the text saying why: not in this build, narrowed out by this name,
//! disabled by config, or not implemented yet.
//!
//! The top-level dispatch is hand-rolled rather than clap: it must
//! behave identically in every variant — including the tiny connect
//! build, where the full registry and the stub machinery are the
//! product — and multicall routing happens before any per-domain
//! parsing exists. Domains adopt their own parsers as they land.

use fleet_registry::{
    Capability, CapabilitySet, Domain, Variant, alias_route, capability_set, domain,
    is_known_invocation_name,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const SOURCE_REV: &str = env!("FLEET_SOURCE_REV");

const EXIT_OK: i32 = 0;
const EXIT_USAGE: i32 = 2;
/// D3: sysexits.h `EX_UNAVAILABLE` — "a service is unavailable",
/// precisely the semantics of a capability that is not in this artifact.
const EXIT_UNAVAILABLE: i32 = 69;

/// The capabilities this artifact was compiled with (its Cargo feature
/// set). `proxy-connect` is unconditional: it is in every variant (§2).
fn compiled_capabilities() -> CapabilitySet {
    let mut caps = capability_set(&[Capability::ProxyConnect]);
    if cfg!(feature = "proxy-serve") {
        caps.insert(Capability::ProxyServe);
    }
    if cfg!(feature = "queue") {
        caps.insert(Capability::Queue);
    }
    if cfg!(feature = "queue-broker") {
        caps.insert(Capability::QueueBroker);
    }
    if cfg!(feature = "secrets-broker") {
        caps.insert(Capability::SecretsBroker);
    }
    if cfg!(feature = "search") {
        caps.insert(Capability::Search);
    }
    if cfg!(feature = "search-serve") {
        caps.insert(Capability::SearchServe);
    }
    if cfg!(feature = "dispatch") {
        caps.insert(Capability::Dispatch);
    }
    if cfg!(feature = "laptop-agent") {
        caps.insert(Capability::LaptopAgent);
    }
    caps
}

/// The shipped variant this artifact's feature set corresponds to.
fn build_variant(compiled: &CapabilitySet) -> Option<Variant> {
    if *compiled == Variant::Full.capabilities() {
        Some(Variant::Full)
    } else if *compiled == Variant::Serve.capabilities() {
        Some(Variant::Serve)
    } else if *compiled == Variant::Connect.capabilities() {
        Some(Variant::Connect)
    } else {
        None
    }
}

fn build_name(compiled: &CapabilitySet) -> String {
    match build_variant(compiled) {
        Some(v) => v.name().to_string(),
        None => "fleet-custom".to_string(),
    }
}

/// Why a capability is not usable by this invocation, in check order.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CapState {
    Available,
    NotInBuild,
    NarrowedByName,
    DisabledByConfig,
}

/// The invocation's whole surface: what was compiled, what the name
/// allows, what config disables.
struct Surface {
    compiled: CapabilitySet,
    profile: Option<Variant>,
    effective: CapabilitySet,
    config: fleet_config::Config,
}

impl Surface {
    fn new(profile: Option<Variant>, config: fleet_config::Config) -> Surface {
        let compiled = compiled_capabilities();
        let effective = match profile {
            Some(v) => compiled.intersection(&v.capabilities()).copied().collect(),
            None => compiled.clone(),
        };
        Surface {
            compiled,
            profile,
            effective,
            config,
        }
    }

    fn cap_state(&self, cap: Capability, domain_name: &str) -> CapState {
        if !self.compiled.contains(&cap) {
            CapState::NotInBuild
        } else if !self.effective.contains(&cap) {
            CapState::NarrowedByName
        } else if !self.config.domain_enabled(domain_name) {
            CapState::DisabledByConfig
        } else {
            CapState::Available
        }
    }

    fn domain_state(&self, d: &Domain) -> CapState {
        self.cap_state(d.primary, d.name)
    }

    /// Capabilities this invocation can actually use: effective, minus
    /// the surfaces of config-disabled domains.
    fn usable(&self) -> CapabilitySet {
        let mut set = self.effective.clone();
        for d in fleet_registry::DOMAINS {
            if !self.config.domain_enabled(d.name) {
                for cap in d.capabilities {
                    set.remove(cap);
                }
            }
        }
        set
    }
}

fn format_caps(caps: &CapabilitySet) -> String {
    if caps.is_empty() {
        return "none".to_string();
    }
    caps.iter().map(|c| c.id()).collect::<Vec<_>>().join(", ")
}

/// The §3 stub block, one form per reason. Always to stderr, always
/// exit 69; the first line names the domain and the reason class, the
/// second lists what this invocation *can* do.
fn stub(surface: &Surface, domain_name: &str, state: &CapState, invoked_as: &str) -> i32 {
    let build = build_name(&surface.compiled);
    let caps = format_caps(&surface.usable());
    let mut out = String::new();
    match state {
        CapState::NotInBuild => {
            out.push_str(&format!(
                "fleet: this build does not include '{domain_name}'.\n"
            ));
            out.push_str(&format!("Enabled capabilities: {caps}.\n"));
            out.push_str(&format!("This is the {build} build ({VERSION})."));
            if build != "fleet" {
                out.push_str(" The full build is 'fleet'.");
            }
            out.push('\n');
        }
        CapState::NarrowedByName => {
            let profile = surface.profile.map(|v| v.name()).unwrap_or("fleet-connect");
            out.push_str(&format!(
                "fleet: invoked as '{invoked_as}', this build does not include '{domain_name}'.\n"
            ));
            out.push_str(&format!("Enabled capabilities: {caps}.\n"));
            out.push_str(&format!(
                "This is the {build} build ({VERSION}), name-scoped to the {profile} profile. \
                 Invoked as 'fleet' it carries the full surface.\n"
            ));
        }
        CapState::DisabledByConfig => {
            out.push_str(&format!(
                "fleet: '{domain_name}' is disabled by config on this host.\n"
            ));
            out.push_str(&format!("Enabled capabilities: {caps}.\n"));
            out.push_str(&format!(
                "This is the {build} build ({VERSION}). Re-enable '{domain_name}' under [domains] in {}.\n",
                fleet_config::Config::file_path().display()
            ));
        }
        CapState::Available => {
            // The step-1 form: compiled in, nothing behind it yet.
            out.push_str(&format!(
                "fleet: '{domain_name}' is included in this build but not implemented yet.\n"
            ));
            out.push_str(&format!("Enabled capabilities: {caps}.\n"));
            out.push_str(&format!(
                "This is the {build} build ({VERSION}) — the step-1 skeleton of epic #5412; \
                 the '{domain_name}' domain lands in a later step.\n"
            ));
        }
    }
    eprint!("{out}");
    EXIT_UNAVAILABLE
}

/// `fleet version` — the surface the census found missing (§1):
/// workspace version + enabled capability list + source rev.
fn print_version(surface: &Surface, invoked_as: &str) {
    let build = build_name(&surface.compiled);
    println!("fleet {VERSION}");
    match surface.profile {
        Some(v) => println!(
            "Build: {build} (invoked as {invoked_as}; surface narrowed to the {} profile)",
            v.name()
        ),
        None => println!("Build: {build}"),
    }
    println!("Source rev: {SOURCE_REV}");
    println!("Enabled capabilities: {}", format_caps(&surface.usable()));
}

/// `fleet capabilities` — the registry as JSON, for agents.
fn print_capabilities(surface: &Surface, invoked_as: &str) {
    let build = build_name(&surface.compiled);
    let domains: Vec<serde_json::Value> = fleet_registry::DOMAINS
        .iter()
        .map(|d| {
            let state = match surface.domain_state(d) {
                CapState::Available => "available",
                CapState::NotInBuild => "not-in-build",
                CapState::NarrowedByName => "narrowed-by-name",
                CapState::DisabledByConfig => "disabled-by-config",
            };
            serde_json::json!({
                "name": d.name,
                "summary": d.summary,
                "aliases": d.aliases,
                "capabilities": d.capabilities.iter().map(|c| c.id()).collect::<Vec<_>>(),
                "available": surface.domain_state(d) == CapState::Available,
                "state": state,
            })
        })
        .collect();
    let doc = serde_json::json!({
        "build": {
            "name": build,
            "version": VERSION,
            "source_rev": SOURCE_REV,
            "invoked_as": invoked_as,
            "profile": surface.profile.map(|v| v.name()),
        },
        "capabilities": surface.usable().iter().map(|c| c.id()).collect::<Vec<_>>(),
        "compiled_capabilities": surface.compiled.iter().map(|c| c.id()).collect::<Vec<_>>(),
        "domains": domains,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".to_string())
    );
}

fn state_mark(state: &CapState) -> &'static str {
    match state {
        CapState::Available => "",
        CapState::NotInBuild => " [not in this build]",
        CapState::NarrowedByName => " [narrowed out by this name]",
        CapState::DisabledByConfig => " [disabled by config]",
    }
}

/// `--help` — the FULL registry on every variant (§3), absent domains
/// marked. This is registry data; it renders identically everywhere.
fn print_help(surface: &Surface) {
    print!("{}", help_text(surface));
}

fn help_text(surface: &Surface) -> String {
    let mut out = String::new();
    let build = build_name(&surface.compiled);
    out.push_str(&format!(
        "fleet {VERSION} — one multicall binary for the fleet's tools (build: {build})\n\n"
    ));
    out.push_str("Usage: fleet <domain> [args...]\n");
    out.push_str("       fleet <command>\n\n");
    out.push_str("Domains (the full registry — every build knows every domain):\n");
    let width = fleet_registry::DOMAINS
        .iter()
        .map(|d| d.name.len())
        .max()
        .unwrap_or(0);
    for d in fleet_registry::DOMAINS {
        let mark = state_mark(&surface.domain_state(d));
        let aliases = if d.aliases.is_empty() {
            String::new()
        } else {
            format!("  [aliases: {}]", d.aliases.join(", "))
        };
        out.push_str(&format!(
            "  {:<width$}  {}{}{}\n",
            d.name,
            d.summary,
            aliases,
            mark,
            width = width
        ));
    }
    out.push_str("\nBuilt-in commands:\n");
    out.push_str("  version        Print version, enabled capabilities, and source rev\n");
    out.push_str("  capabilities   Print the registry and this build's capabilities as JSON\n");
    out.push_str("  help [domain]  Show this help, or one domain's help\n\n");
    out.push_str("A domain this invocation cannot use answers with a stub and exit code 69.\n");
    out
}

/// One domain's help — also registry data, always available (§3 help
/// pointers), whatever the build.
fn print_domain_help(surface: &Surface, d: &Domain) {
    println!("fleet {} — {}", d.name, d.summary);
    println!();
    println!("Usage: fleet {} <command> [args...]", d.name);
    if !d.aliases.is_empty() {
        println!("Aliases (argv0): {}", d.aliases.join(", "));
    }
    if !d.subcommands.is_empty() {
        println!("Commands: {}", d.subcommands.join(", "));
    }
    let status = match surface.domain_state(d) {
        CapState::Available => {
            "included in this build (implementation lands in a later step of epic #5412)"
        }
        CapState::NotInBuild => {
            "not in this build — invoking it prints the capability stub (exit 69)"
        }
        CapState::NarrowedByName => {
            "narrowed out by this invocation name — invoking it prints the capability stub (exit 69)"
        }
        CapState::DisabledByConfig => "disabled by config on this host (exit 69 if invoked)",
    };
    println!("Status: {status}");
}

fn usage_error(message: &str) -> i32 {
    eprintln!("error: {message}");
    eprintln!();
    eprintln!("Usage: fleet <domain> [args...]");
    eprintln!();
    eprintln!("For the full domain registry, run: fleet --help");
    EXIT_USAGE
}

/// The capability a concrete domain invocation needs.
fn required_capability(d: &Domain, args: &[String]) -> Result<Capability, i32> {
    match d.name {
        "serve" => {
            let role = args.first().map(|s| s.as_str());
            match role {
                Some("queue-broker") => Ok(Capability::QueueBroker),
                Some("secrets-broker") => Ok(Capability::SecretsBroker),
                Some("search") => Ok(Capability::SearchServe),
                _ => Err(usage_error(
                    "'fleet serve' needs a role: queue-broker, secrets-broker, or search",
                )),
            }
        }
        "proxy" => {
            let sub = args.first().map(|s| s.as_str());
            match sub {
                Some("serve") => Ok(Capability::ProxyServe),
                Some("connect") | Some("init") => Ok(Capability::ProxyConnect),
                _ => Err(usage_error(
                    "'fleet proxy' needs a command: serve, connect, or init",
                )),
            }
        }
        _ => Ok(d.primary),
    }
}

/// Run one domain invocation (also the argv0-alias landing pad).
fn run_domain(surface: &Surface, d: &Domain, args: &[String], invoked_as: &str) -> i32 {
    // Domain help is registry data: always available, even where the
    // domain itself is not (§3).
    if matches!(
        args.first().map(|s| s.as_str()),
        Some("--help") | Some("-h") | Some("help")
    ) {
        print_domain_help(surface, d);
        return EXIT_OK;
    }
    let cap = match required_capability(d, args) {
        Ok(c) => c,
        Err(code) => return code,
    };
    match surface.cap_state(cap, d.name) {
        CapState::Available => {
            if args.is_empty() && d.name != "serve" && d.name != "proxy" {
                // A bare compiled-in domain is a usage error, like the
                // tools being replaced — but say what it will be.
                print_domain_help(surface, d);
                return EXIT_USAGE;
            }
            stub(surface, d.name, &CapState::Available, invoked_as)
        }
        state => stub(surface, d.name, &state, invoked_as),
    }
}

fn run_root(surface: &Surface, args: &[String], invoked_as: &str) -> i32 {
    let first = args.first().map(|s| s.as_str());
    match first {
        None => {
            // Bare `fleet`: clap-style missing-subcommand error —
            // message and help both to stderr, exit 2.
            eprintln!("error: 'fleet' requires a subcommand");
            eprintln!();
            eprint!("{}", help_text(surface));
            EXIT_USAGE
        }
        Some("--help") | Some("-h") | Some("help") if args.len() == 1 => {
            print_help(surface);
            EXIT_OK
        }
        Some("help") => match args.get(1).and_then(|name| domain(name)) {
            Some(d) => {
                print_domain_help(surface, d);
                EXIT_OK
            }
            None => usage_error(&format!(
                "no domain named '{}' — run 'fleet --help' for the registry",
                args.get(1).map(|s| s.as_str()).unwrap_or("")
            )),
        },
        Some("--version") | Some("-V") => {
            println!("fleet {VERSION}");
            EXIT_OK
        }
        Some("version") => {
            print_version(surface, invoked_as);
            EXIT_OK
        }
        Some("capabilities") => {
            print_capabilities(surface, invoked_as);
            EXIT_OK
        }
        Some(name) => match domain(name) {
            Some(d) => run_domain(surface, d, &args[1..], invoked_as),
            None => usage_error(&format!("unrecognized subcommand '{name}'")),
        },
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let invoked_as = argv
        .first()
        .map(|a| {
            std::path::Path::new(a)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| a.clone())
        })
        .unwrap_or_else(|| "fleet".to_string());
    let args: Vec<String> = argv.iter().skip(1).cloned().collect();

    // Config decides the disable table; a malformed config is a loud
    // error on every path (a silently ignored disable would defeat
    // the §7 incident lever).
    let config = match fleet_config::Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(EXIT_USAGE);
        }
    };

    let code = if invoked_as == "fleet" {
        let surface = Surface::new(None, config);
        run_root(&surface, &args, &invoked_as)
    } else if let Some(variant) = Variant::from_invocation_name(&invoked_as) {
        // Refinement A: the hyphenated name narrows (never widens).
        let surface = Surface::new(Some(variant), config);
        run_root(&surface, &args, &invoked_as)
    } else if let Some(route) = alias_route(&invoked_as) {
        let surface = Surface::new(None, config);
        let d = domain(route.domain).expect("registry aliases route to real domains");
        let mut routed: Vec<String> = Vec::new();
        if let Some(implied) = route.implied_arg {
            routed.push(implied.to_string());
        }
        routed.extend(args);
        run_domain(&surface, d, &routed, &invoked_as)
    } else if is_known_invocation_name(&invoked_as) {
        unreachable!("known names are handled above");
    } else {
        eprintln!(
            "fleet: invoked under unknown name '{invoked_as}'; expected 'fleet', a variant \
             name ('fleet-serve', 'fleet-connect'), or a known alias (repo-queue, queue-broker, \
             secrets-broker, laptop-agent, laptop-agent-broker, atlas-search)"
        );
        EXIT_USAGE
    };
    std::process::exit(code);
}

// Keep the platform crates linked into every build that compiles this
// binary: the proxy protocol types are the fleet's wire contract, and
// the secrets path is platform surface even while domains are stubs.
#[allow(dead_code)]
fn _platform_linkage() {
    let _ = fleet_proxy_proto::PROTO_MAJOR;
    let _ = std::mem::size_of::<fleet_proxy_proto::Hello>();
    #[cfg(feature = "queue")]
    let _ = std::mem::size_of::<fleet_secrets::Resolver>();
}
