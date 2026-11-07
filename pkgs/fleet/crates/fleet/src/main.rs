//! fleet — one multicall binary for the fleet's tools
//! (epic developing-today/code#5412; design note
//! doc/2026-11-04T11-45-48Z_design_fleet_multicall_binary/).
//!
//! Step 1 (#5429) built the skeleton: the platform crates, this
//! dispatch layer, the capability-stub machinery, and the variant
//! builds. Step 2 (#5430) landed the first real domain — queue: the
//! repo-queue client and the queue-broker / secrets-broker serve
//! roles run for real below; step 3 (#5431) the proxy domain; step 4
//! (#5432) the dispatch thin client; step 6 (#5554) the secrets verb;
//! step 5 (#5433) the search client domain, with its serve role
//! (`fleet serve search`, the Atlas service) following in #5604. A
//! compiled-in domain that has
//! not landed yet answers with the "not implemented yet" form of
//! the stub — but the *surface* is complete and honest: every
//! variant renders the full registry, and every missing capability
//! answers with a branchable stub, never a parser error.
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

#[cfg(feature = "secrets")]
mod secrets;

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
    let mut caps = capability_set(&[Capability::ProxyConnect, Capability::ProxyRawSplice]);
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
    if cfg!(feature = "dispatch-test") {
        caps.insert(Capability::DispatchTest);
    }
    if cfg!(feature = "laptop-agent") {
        caps.insert(Capability::LaptopAgent);
    }
    if cfg!(feature = "secrets") {
        caps.insert(Capability::Secrets);
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
    out.push_str(
        "  init           Autostart this host's roles per the [host] manifest (no-op when running)\n",
    );
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
        CapState::Available => match d.name {
            "queue" => "included in this build",
            "dispatch" => "included in this build",
            "dispatch-test" => "included in this build",
            "secrets" => "included in this build",
            "search" => "included in this build",
            "proxy" => "included in this build",
            "serve" => {
                "included in this build (queue-broker, secrets-broker, search and proxy roles)"
            }
            _ => "included in this build (implementation lands in a later step of epic #5412)",
        },
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
                Some("proxy") => Ok(Capability::ProxyServe),
                _ => Err(usage_error(
                    "'fleet serve' needs a role: queue-broker, secrets-broker, search, or proxy",
                )),
            }
        }
        // `search serve` (and so the standalone `atlas-search serve`
        // argv0 spelling) is the same service role as `serve search`
        // (#5604); every other search argument is the client surface.
        "search" if args.first().map(|s| s.as_str()) == Some("serve") => {
            Ok(Capability::SearchServe)
        }
        "proxy" => {
            let sub = args.first().map(|s| s.as_str());
            match sub {
                Some("serve") => Ok(Capability::ProxyServe),
                Some("connect") | Some("init") | Some("ssh") => Ok(Capability::ProxyConnect),
                Some("splice") => Ok(Capability::ProxyRawSplice),
                _ => Err(usage_error(
                    "'fleet proxy' needs a command: serve, connect, init, ssh, or splice",
                )),
            }
        }
        _ => Ok(d.primary),
    }
}

fn is_help_flag(arg: &str) -> bool {
    matches!(arg, "--help" | "-h" | "help")
}

/// Does this domain invocation ask for the domain help? The plain form
/// puts the flag first (`fleet queue --help`). The `serve` domain's
/// first argument is a role, so its help flag sits one position later
/// (`fleet serve queue-broker --help`) — and the argv0 aliases
/// (`queue-broker --help`) land in exactly that shape after the implied
/// arg is prepended. The broker entry points take no arguments, so a
/// help flag they never see would otherwise start the daemon (#5488).
fn wants_domain_help(d: &Domain, args: &[String]) -> bool {
    if args.first().is_some_and(|a| is_help_flag(a)) {
        return true;
    }
    d.name == "serve" && args.get(1).is_some_and(|a| is_help_flag(a))
}

/// A broker role is a complete invocation: the broker entry point has
/// no argument parser, so reject a trailing argument rather than silently
/// starting the daemon with it discarded.
fn broker_argument_error(d: &Domain, args: &[String]) -> Option<String> {
    if d.name != "serve" {
        return None;
    }
    let role = args.first()?.as_str();
    if !matches!(role, "queue-broker" | "secrets-broker") {
        return None;
    }
    args.get(1)
        .map(|extra| format!("unexpected argument '{extra}' for 'fleet serve {role}'"))
}
/// The arguments after the role name when this invocation is the
/// search service role, in either spelling: `serve search ...` or
/// `search serve ...` (the `atlas-search serve` argv0 route).
#[cfg(feature = "search-serve")]
fn search_serve_tail<'a>(d: &Domain, args: &'a [String]) -> Option<&'a [String]> {
    let first = args.first().map(|s| s.as_str());
    match (d.name, first) {
        ("serve", Some("search")) | ("search", Some("serve")) => Some(&args[1..]),
        _ => None,
    }
}

/// Run one domain invocation (also the argv0-alias landing pad).
fn run_domain(surface: &Surface, d: &Domain, args: &[String], invoked_as: &str) -> i32 {
    // The search role takes real flags, so its own usage answers
    // `--help` wherever the role is actually usable; elsewhere the
    // flag falls through to the registry's domain help below.
    #[cfg(feature = "search-serve")]
    if let Some(tail) = search_serve_tail(d, args)
        && tail.first().is_some_and(|a| is_help_flag(a))
        && surface.cap_state(Capability::SearchServe, d.name) == CapState::Available
    {
        println!("{}", fleet_search::serve::SERVE_USAGE);
        return EXIT_OK;
    }
    // Domain help is registry data: always available, even where the
    // domain itself is not (§3).
    if wants_domain_help(d, args) {
        print_domain_help(surface, d);
        return EXIT_OK;
    }
    if let Some(message) = broker_argument_error(d, args) {
        return usage_error(&message);
    }
    let cap = match required_capability(d, args) {
        Ok(c) => c,
        Err(code) => return code,
    };
    match surface.cap_state(cap, d.name) {
        CapState::Available => {
            // Step 2 (#5430): the queue domain runs for real — the
            // repo-queue client surface and the two broker roles.
            // `--version` through the argv0 route is the census fix:
            // repo-queue had no version surface at all before this.
            #[cfg(feature = "queue")]
            if d.name == "queue" {
                if matches!(
                    args.first().map(|s| s.as_str()),
                    Some("--version") | Some("-V")
                ) {
                    // The version shortcut takes no trailing arguments:
                    // like the root built-ins (#5572), a trailing argument
                    // is a usage error naming it, not silently discarded
                    // success (#5603).
                    if let Some(extra) = args.get(1) {
                        return usage_error(&format!(
                            "unexpected argument '{extra}' for 'fleet queue {}'",
                            args.first().map(|s| s.as_str()).unwrap_or("--version")
                        ));
                    }
                    println!(
                        "{} {VERSION}",
                        if invoked_as == "repo-queue" {
                            "repo-queue"
                        } else {
                            "fleet"
                        }
                    );
                    return EXIT_OK;
                }
                return fleet_queue::client::run(args);
            }
            #[cfg(feature = "queue-broker")]
            if d.name == "serve" && args.first().map(|s| s.as_str()) == Some("queue-broker") {
                return fleet_queue::queue_broker::run();
            }
            #[cfg(feature = "secrets-broker")]
            if d.name == "serve" && args.first().map(|s| s.as_str()) == Some("secrets-broker") {
                return fleet_queue::secrets_broker::run();
            }
            // #5604: the Atlas service role, rehomed from the
            // atlas-search bin crate. Runs in the foreground; the
            // atlas user unit's ExecStart is `fleet serve search`.
            #[cfg(feature = "search-serve")]
            if let Some(tail) = search_serve_tail(d, args) {
                return fleet_search::serve::run(tail);
            }
            // #6770 (design #5687 §5): the proxy daemon role under
            // its serve spelling — the same leaf as `fleet proxy
            // serve`. fleet-proxy's spawn_serve spawns this
            // spelling since the rename, so this route is what
            // every auto-started broker now runs as.
            if d.name == "serve" && args.first().map(|s| s.as_str()) == Some("proxy") {
                return run_serve_proxy(surface, &args[1..]);
            }
            // Step 4 (#5432): the dispatch domain runs for real — a
            // thin client over pm-stack's pm-dispatch service.
            #[cfg(feature = "dispatch")]
            if d.name == "dispatch" {
                return fleet_dispatch::run(&surface.config, args);
            }
            // The dispatch-test domain (developing-today/code#6580):
            // explicit-dispatch CI test over GitHub Actions + the
            // Turso jobs store (full build only).
            #[cfg(feature = "dispatch-test")]
            if d.name == "dispatch-test" {
                return fleet_dispatch_test::run(&surface.config, args);
            }
            // Step 6 (#5554): the secrets domain runs for real —
            // the fleet-secrets resolver chain as a verb.
            #[cfg(feature = "secrets")]
            if d.name == "secrets" {
                return secrets::run(&surface.config, args);
            }
            // Step 5 (#5433): the search domain runs for real —
            // the atlas-search client surface over the live Atlas
            // service (full build only; the serve build carries
            // search-serve, so the client stubs 69 there, §2).
            #[cfg(feature = "search")]
            if d.name == "search" {
                return fleet_search::run(&surface.config, args);
            }
            if d.name == "proxy" {
                return run_proxy(surface, args);
            }
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

const SERVE_PROXY_USAGE: &str = "\
Usage: fleet serve proxy

Run the session proxy (the broker) in the foreground, binding the
broker socket from fleet config (FLEET_BROKER_SOCKET / broker_socket).
The same leaf as 'fleet proxy serve'.

Takes no arguments.";

/// `fleet serve proxy` — the proxy daemon role under its serve
/// spelling (design #5687 §5, impl #6770). Argument handling is
/// the proxy serve leaf's own (`classify_proxy_serve_args`): the
/// leaf takes no arguments, so a trailing one is a usage error,
/// never something silently dropped before the socket is bound.
fn run_serve_proxy(surface: &Surface, tail: &[String]) -> i32 {
    match classify_proxy_serve_args(tail) {
        ProxyServeArgs::Help => {
            println!("{SERVE_PROXY_USAGE}");
            EXIT_OK
        }
        ProxyServeArgs::Reject(arg) => usage_error(&format!(
            "'fleet serve proxy' takes no arguments, got {arg:?}\n\n{SERVE_PROXY_USAGE}"
        )),
        ProxyServeArgs::Run => {
            #[cfg(feature = "proxy-serve")]
            {
                fleet_proxy::serve::run(&surface.config)
            }
            #[cfg(not(feature = "proxy-serve"))]
            {
                unreachable!("serve proxy dispatched without the proxy-serve capability")
            }
        }
    }
}

const PROXY_SERVE_USAGE: &str = "\
Usage: fleet proxy serve

Run the session proxy (the broker) in the foreground, binding the
broker socket from fleet config (FLEET_BROKER_SOCKET / broker_socket).

Takes no arguments.";

const PROXY_SSH_USAGE: &str = "\
Usage: fleet proxy ssh -- [ssh args...]

Run system OpenSSH through the fleet proxy as a recovery path. The destination,
identity, host-key policy, timeouts, keepalives, and ControlMaster settings are
pinned by fleet. SSH options go after `--`; an optional remote command may follow
the fixed destination. The fleet proxy credential stays in the environment and
is sent only by the CONNECT relay.

The inner SSH destination is fixed; do not pass an SSH `--` separator.";

const PROXY_SPLICE_USAGE: &str = "\
Usage: fleet proxy splice

Listen on 127.0.0.1:21118 and relay each accepted TCP stream through
the fleet broker to the laptop's RustDesk direct-access service.

Takes no arguments.";

/// What the arguments after `fleet proxy serve` ask for. The serve
/// leaf takes no arguments at all (`fleet_proxy::serve::run` is
/// config-only), so anything trailing is either a help request or a
/// mistake — never something to silently drop before the broker
/// socket is bound (#5575; the #5488/#5560 bug class on the proxy
/// leaf, reached directly and via the `laptop-agent-broker` argv0
/// alias, whose implied arg lands in exactly this position).
#[derive(Debug, PartialEq, Eq)]
enum ProxyServeArgs {
    Run,
    Help,
    Reject(String),
}

fn classify_proxy_serve_args(tail: &[String]) -> ProxyServeArgs {
    if tail.iter().any(|a| is_help_flag(a)) {
        ProxyServeArgs::Help
    } else if let Some(first) = tail.first() {
        ProxyServeArgs::Reject(first.clone())
    } else {
        ProxyServeArgs::Run
    }
}

/// The proxy domain's leaves (step 3, #5431). `required_capability`
/// has already mapped serve → ProxyServe and connect/init →
/// ProxyConnect, and cap_state has already stubbed at 69 where the
/// capability is unusable, so `serve` here implies the broker half
/// is compiled in.
fn run_proxy(surface: &Surface, args: &[String]) -> i32 {
    let config = &surface.config;
    let client = format!("fleet {} ({})", VERSION, build_name(&surface.compiled));
    let capabilities = fleet_proxy_proto::capability_ids(&surface.usable());
    match args.first().map(|s| s.as_str()) {
        Some("serve") => match classify_proxy_serve_args(&args[1..]) {
            ProxyServeArgs::Help => {
                println!("{PROXY_SERVE_USAGE}");
                EXIT_OK
            }
            ProxyServeArgs::Reject(arg) => usage_error(&format!(
                "'fleet proxy serve' takes no arguments, got {arg:?}\n\n{PROXY_SERVE_USAGE}"
            )),
            ProxyServeArgs::Run => {
                #[cfg(feature = "proxy-serve")]
                {
                    fleet_proxy::serve::run(config)
                }
                #[cfg(not(feature = "proxy-serve"))]
                {
                    unreachable!("proxy serve dispatched without the proxy-serve capability")
                }
            }
        },
        Some("connect") => {
            // A help flag in the tail asks for the leaf's usage:
            // print it to stdout and succeed, like the dispatch
            // verbs and proxy serve (#5560/#5575). ConnectArgs::parse
            // can only carry the text out as a usage error (exit 2),
            // so the help form is answered here (#5619).
            if args[1..]
                .iter()
                .any(|a| matches!(a.as_str(), "--help" | "-h"))
            {
                println!("{}", fleet_proxy::client::CONNECT_USAGE);
                return EXIT_OK;
            }
            let parsed = match fleet_proxy::client::ConnectArgs::parse(&args[1..]) {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("fleet: {e}");
                    return e.exit_code();
                }
            };
            match fleet_proxy::client::run_connect(&parsed, config, &client, &capabilities) {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("fleet: {e}");
                    e.exit_code()
                }
            }
        }
        Some("init") => run_init_call(surface, &args[1..]),
        Some("ssh") => run_proxy_ssh(&args[1..], config),
        Some("splice") => {
            if args[1..].iter().any(|a| is_help_flag(a)) {
                println!("{PROXY_SPLICE_USAGE}");
                return EXIT_OK;
            }
            if let Some(arg) = args.get(1) {
                return usage_error(&format!(
                    "'fleet proxy splice' takes no arguments, got {arg:?}\n\n{PROXY_SPLICE_USAGE}"
                ));
            }
            match fleet_proxy::raw_splice::run(config, &client, &capabilities) {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("fleet: {e}");
                    e.exit_code()
                }
            }
        }
        _ => unreachable!("required_capability rejects other proxy commands first"),
    }
}

/// Split SSH's option prefix from its optional remote command. The fleet
/// supplies the destination between these parts so callers cannot redirect
/// the recovery command to an arbitrary host. Options (including their
/// separate operands) remain byte-for-byte argument values, not shell text.
fn split_ssh_options_and_command(args: &[String]) -> Result<(Vec<String>, Vec<String>), String> {
    // Operand-bearing ssh option characters, per ssh(1) -- including
    // `P` (tag), which a whole-argument table lookup missed along
    // with every clustered form such as `-vL` (#6679).
    const OPTIONS_WITH_OPERANDS: &[char] = &[
        'B', 'b', 'c', 'D', 'E', 'e', 'F', 'I', 'i', 'J', 'L', 'l', 'm', 'O', 'o', 'P', 'p', 'Q',
        'R', 'S', 'W', 'w',
    ];

    let mut options = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--" {
            return Err("the SSH destination is fixed; do not pass a second '--'".to_string());
        }
        if !arg.starts_with('-') || arg == "-" {
            return Ok((options, args[index..].to_vec()));
        }
        options.push(arg.clone());
        index += 1;
        // Walk the option cluster character by character: a
        // non-operand character continues the cluster, and an
        // operand-bearing character consumes the rest of the
        // cluster as its operand -- or, when it is the cluster's
        // last character, the next argument. Everything after the
        // operand belongs to the next argument, never this cluster.
        let cluster = &arg[1..];
        let mut operand_char = None;
        for (offset, ch) in cluster.char_indices() {
            if OPTIONS_WITH_OPERANDS.contains(&ch) {
                if offset + ch.len_utf8() == cluster.len() {
                    operand_char = Some(ch);
                }
                break;
            }
        }
        if let Some(ch) = operand_char {
            let operand = args
                .get(index)
                .ok_or_else(|| format!("SSH option -{ch} in {arg} needs an argument"))?;
            if operand == "--" {
                return Err(format!(
                    "SSH option -{ch} in {arg} needs an argument before '--'"
                ));
            }
            options.push(operand.clone());
            index += 1;
        }
    }
    Ok((options, Vec::new()))
}

fn shell_quote_executable(path: &std::path::Path) -> Result<String, String> {
    let path = path.to_str().ok_or_else(|| {
        "fleet executable path is not valid UTF-8 for OpenSSH ProxyCommand".to_string()
    })?;
    Ok(shell_quote(path))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn build_proxy_ssh_argv(
    executable: &std::path::Path,
    config: &fleet_config::Config,
    ssh_args: &[String],
) -> Result<Vec<std::ffi::OsString>, String> {
    build_proxy_ssh_argv_with_home(
        executable,
        config,
        &fleet_config::Config::home_dir(),
        ssh_args,
    )
}

fn build_proxy_ssh_argv_with_home(
    executable: &std::path::Path,
    config: &fleet_config::Config,
    home: &std::path::Path,
    ssh_args: &[String],
) -> Result<Vec<std::ffi::OsString>, String> {
    // OpenSSH expands a literal `~` from the passwd database, not
    // from $HOME, so the recovery paths are resolved from Fleet's
    // $HOME-based home before the arguments are built (#6680) --
    // the same construction fleet-search's client uses.
    let known_hosts = home.join(".ssh/known_hosts_laptop");
    let identity = home.join(".ssh/id_ed25519");
    let control_path = home.join(".ssh/cm-%r@%h:%p");
    let (options, command) = split_ssh_options_and_command(ssh_args)?;
    let target_host = config.target_host();
    let proxy_command = format!(
        "ProxyCommand={} proxy connect --stdio --host {} --port %p",
        shell_quote_executable(executable)?,
        shell_quote(target_host)
    );
    let mut argv = vec![
        "-o".into(),
        "StrictHostKeyChecking=yes".into(),
        "-o".into(),
        format!("HostName={target_host}").into(),
        "-o".into(),
        "User=user".into(),
        "-o".into(),
        format!("UserKnownHostsFile={}", known_hosts.display()).into(),
        "-o".into(),
        "IdentitiesOnly=yes".into(),
        "-i".into(),
        identity.into(),
        "-o".into(),
        "ConnectTimeout=60".into(),
        "-o".into(),
        "ConnectionAttempts=3".into(),
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "ServerAliveCountMax=8".into(),
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        format!("ControlPath={}", control_path.display()).into(),
        "-o".into(),
        "ControlPersist=3600".into(),
        "-o".into(),
        proxy_command.into(),
    ];
    argv.extend(options.into_iter().map(std::ffi::OsString::from));
    argv.push(format!("user@{target_host}").into());
    argv.extend(command.into_iter().map(std::ffi::OsString::from));
    Ok(argv)
}

fn run_proxy_ssh(tail: &[String], config: &fleet_config::Config) -> i32 {
    if tail.first().is_some_and(|arg| is_help_flag(arg)) {
        println!("{PROXY_SSH_USAGE}");
        return EXIT_OK;
    }
    if tail.first().map(String::as_str) != Some("--") {
        return usage_error(&format!(
            "'fleet proxy ssh' expects `--` before SSH arguments\n\n{PROXY_SSH_USAGE}"
        ));
    }
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("fleet: cannot locate its executable for ProxyCommand: {error}");
            return EXIT_UNAVAILABLE;
        }
    };
    let argv = match build_proxy_ssh_argv(&executable, config, &tail[1..]) {
        Ok(argv) => argv,
        Err(error) => return usage_error(&format!("{error}\n\n{PROXY_SSH_USAGE}")),
    };

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = std::process::Command::new("ssh").args(argv).exec();
        eprintln!("fleet: cannot execute system ssh: {error}");
        EXIT_UNAVAILABLE
    }
    #[cfg(not(unix))]
    {
        let _ = argv;
        eprintln!("fleet: recovery SSH via system OpenSSH is supported on Unix only");
        EXIT_UNAVAILABLE
    }
}

fn run_init_call(surface: &Surface, argv: &[String]) -> i32 {
    let client = format!("fleet {} ({})", VERSION, build_name(&surface.compiled));
    let capabilities = fleet_proxy_proto::capability_ids(&surface.usable());
    match fleet_proxy::client::run_init(argv, &surface.config, &client, &capabilities) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("fleet: {e}");
            e.exit_code()
        }
    }
}

/// `fleet init` — the autostart door (design note §8): ensure the
/// session proxy is up, a strict no-op when it already answers.
/// (The verb went missing in step 2's dispatch rework; step 3
/// restores it — the note and #5431 both name it as the entry.)
/// The ladder mirrors the capability rules: a build whose usable
/// surface lacks proxy-serve can only stub (69).
fn cmd_init(surface: &Surface, args: &[String], invoked_as: &str) -> i32 {
    match surface.cap_state(Capability::ProxyServe, "proxy") {
        CapState::Available => run_init_call(surface, args),
        state => stub(surface, "proxy", &state, invoked_as),
    }
}

/// Root built-ins take no arguments: a trailing argument is a usage
/// error naming it (#5572), not silently discarded output.
fn reject_extra_args(args: &[String]) -> Option<i32> {
    args.get(1).map(|extra| {
        usage_error(&format!(
            "unexpected argument '{extra}' for 'fleet {}'",
            args.first().map(|s| s.as_str()).unwrap_or("")
        ))
    })
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
                // `help` takes exactly one argument — the domain name.
                // Anything after it is a usage error naming the first
                // extra, like the other root built-ins (#5572); this arm
                // was the one the #5572 sweep missed because its first
                // argument is legitimate (#5652).
                if let Some(extra) = args.get(2) {
                    return usage_error(&format!("unexpected argument '{extra}' for 'fleet help'"));
                }
                print_domain_help(surface, d);
                EXIT_OK
            }
            None => usage_error(&format!(
                "no domain named '{}' — run 'fleet --help' for the registry",
                args.get(1).map(|s| s.as_str()).unwrap_or("")
            )),
        },
        Some("--version") | Some("-V") => {
            if let Some(code) = reject_extra_args(args) {
                return code;
            }
            println!("fleet {VERSION}");
            EXIT_OK
        }
        Some("version") => {
            if let Some(code) = reject_extra_args(args) {
                return code;
            }
            print_version(surface, invoked_as);
            EXIT_OK
        }
        Some("capabilities") => {
            if let Some(code) = reject_extra_args(args) {
                return code;
            }
            print_capabilities(surface, invoked_as);
            EXIT_OK
        }
        Some("init") => cmd_init(surface, &args[1..], invoked_as),
        Some(name) => match domain(name) {
            Some(d) => run_domain(surface, d, &args[1..], invoked_as),
            None => usage_error(&format!("unrecognized subcommand '{name}'")),
        },
    }
}

/// Restore the default SIGPIPE disposition (#5569). The Rust
/// runtime ignores SIGPIPE, so a `println!` in any domain (queue,
/// search, secrets, ...) panics when a downstream pipe closes
/// early (`fleet secrets list | head`). With the default
/// disposition the process dies quietly by SIGPIPE, like a
/// well-behaved Unix filter, whichever verb is printing. Domains
/// that handle EPIPE explicitly (secrets) keep their quiet-exit
/// path; this covers the rest.
#[cfg(unix)]
fn restore_default_sigpipe() {
    // SAFETY: signal() with SIG_DFL for SIGPIPE is a plain
    // disposition reset, done before any other thread or I/O.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn restore_default_sigpipe() {}

fn main() {
    restore_default_sigpipe();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn help_flag_after_a_serve_role_asks_for_domain_help() {
        let serve = domain("serve").expect("serve domain exists");
        for flag in ["--help", "-h", "help"] {
            assert!(wants_domain_help(serve, &args(&["queue-broker", flag])));
            assert!(wants_domain_help(serve, &args(&["secrets-broker", flag])));
            assert!(wants_domain_help(serve, &args(&["search", flag])));
        }
        // A bare role still starts the daemon; extra args after the
        // flag position are the role's own business.
        assert!(!wants_domain_help(serve, &args(&["queue-broker"])));
        assert!(!wants_domain_help(
            serve,
            &args(&["queue-broker", "--bind", "--help"])
        ));
    }

    #[cfg(feature = "search-serve")]
    #[test]
    fn search_serve_is_one_role_in_both_spellings() {
        // `fleet serve search ...` and `atlas-search serve ...` (the
        // argv0 route into the search domain) are the same role, and
        // both need the search-serve capability, not the client's.
        let serve = domain("serve").unwrap();
        let search = domain("search").unwrap();
        let a = args(&["search", "--config", "x"]);
        let b = args(&["serve", "--config", "x"]);
        assert_eq!(search_serve_tail(serve, &a), Some(&a[1..]));
        assert_eq!(search_serve_tail(search, &b), Some(&b[1..]));
        assert_eq!(
            required_capability(serve, &a).ok(),
            Some(Capability::SearchServe)
        );
        assert_eq!(
            required_capability(search, &b).ok(),
            Some(Capability::SearchServe)
        );
        // Everything else under `search` stays the client surface; the
        // other serve roles are not the search role.
        for client in [
            &["q"][..],
            &["get-chunk", "1"][..],
            &["search", "q"][..],
            &[][..],
        ] {
            assert_eq!(search_serve_tail(search, &args(client)), None);
            assert_eq!(
                required_capability(search, &args(client)).ok(),
                Some(Capability::Search)
            );
        }
        assert_eq!(search_serve_tail(serve, &args(&["queue-broker"])), None);
        assert_eq!(search_serve_tail(search, &args(&["search"])), None);
    }

    #[test]
    fn broker_roles_reject_trailing_args_through_serve_and_argv0_aliases() {
        let serve = domain("serve").expect("serve domain exists");
        let surface = Surface::new(None, fleet_config::Config::default());

        for role in ["queue-broker", "secrets-broker"] {
            let trailing = args(&[role, "--bogus-junk", "extra"]);
            let expected = format!("unexpected argument '--bogus-junk' for 'fleet serve {role}'");
            assert_eq!(
                broker_argument_error(serve, &trailing).as_deref(),
                Some(expected.as_str()),
                "the first unexpected argument should be named"
            );
            assert_eq!(
                run_domain(&surface, serve, &trailing, "fleet"),
                EXIT_USAGE,
                "fleet serve {role} must reject extras before starting the daemon"
            );

            // The argv0 alias dispatch prepends its implied role and then
            // uses run_domain; exercise that same route without invoking run().
            let route = alias_route(role).expect("broker alias is registered");
            let routed_domain = domain(route.domain).expect("alias route has a domain");
            let mut routed = Vec::new();
            if let Some(implied) = route.implied_arg {
                routed.push(implied.to_string());
            }
            routed.push("--bogus-junk".to_string());
            assert_eq!(
                broker_argument_error(routed_domain, &routed).as_deref(),
                Some(expected.as_str())
            );
            assert_eq!(
                run_domain(&surface, routed_domain, &routed, role),
                EXIT_USAGE,
                "{role} alias must reject extras before starting the daemon"
            );

            // A bare role must retain the run path. Do not call run_domain
            // here: the real daemon intentionally blocks waiting for work.
            assert_eq!(broker_argument_error(serve, &args(&[role])), None);
        }

        assert_eq!(
            broker_argument_error(serve, &args(&["search", "subcommand"])),
            None,
            "the serve search role owns its own arguments"
        );
        for role in ["queue-broker", "secrets-broker"] {
            for flag in ["--help", "-h", "help"] {
                assert!(wants_domain_help(serve, &args(&[role, flag])));
                assert_eq!(
                    run_domain(&surface, serve, &args(&[role, flag]), "fleet"),
                    EXIT_OK,
                    "broker help must return before argument rejection or daemon startup"
                );
            }
        }
    }

    #[test]
    fn proxy_serve_args_classify_help_and_reject_strays() {
        // Bare serve runs the broker; the argv0 alias route
        // (laptop-agent-broker) lands here with the same tail.
        assert_eq!(classify_proxy_serve_args(&args(&[])), ProxyServeArgs::Run);
        for flag in ["--help", "-h", "help"] {
            assert_eq!(
                classify_proxy_serve_args(&args(&[flag])),
                ProxyServeArgs::Help,
                "{flag} after proxy serve must print usage, not bind the socket"
            );
        }
        // Help wins anywhere in the tail (mirrors verb_status, #5560);
        // any other stray is rejected, never silently dropped.
        assert_eq!(
            classify_proxy_serve_args(&args(&["--bogus", "--help"])),
            ProxyServeArgs::Help
        );
        assert_eq!(
            classify_proxy_serve_args(&args(&["--bogus"])),
            ProxyServeArgs::Reject("--bogus".to_string())
        );
        assert_eq!(
            classify_proxy_serve_args(&args(&["extra", "args"])),
            ProxyServeArgs::Reject("extra".to_string())
        );
    }

    #[test]
    fn proxy_connect_and_init_help_exit_ok() {
        // #5619: connect/init --help answered through the error
        // channel (exit 2, stderr). Both now print usage to stdout
        // and succeed, before any broker probe or spawn.
        let surface = Surface::new(None, fleet_config::Config::default());
        for flag in ["--help", "-h"] {
            assert_eq!(
                run_proxy(&surface, &args(&["connect", flag])),
                EXIT_OK,
                "proxy connect {flag} must succeed"
            );
            assert_eq!(
                run_proxy(&surface, &args(&["init", flag])),
                EXIT_OK,
                "proxy init {flag} must succeed"
            );
        }
    }

    #[test]
    fn proxy_ssh_uses_connect_capability_and_explicit_separator() {
        let proxy = domain("proxy").expect("proxy domain exists");
        assert_eq!(
            required_capability(proxy, &args(&["ssh"])),
            Ok(Capability::ProxyConnect)
        );
        assert_eq!(
            run_proxy_ssh(&args(&["--help"]), &fleet_config::Config::default()),
            EXIT_OK
        );
        assert_eq!(
            run_proxy_ssh(&args(&[]), &fleet_config::Config::default()),
            EXIT_USAGE
        );
        assert_eq!(
            run_proxy_ssh(
                &args(&["-L", "1234:localhost:22"]),
                &fleet_config::Config::default()
            ),
            EXIT_USAGE
        );
    }

    #[test]
    fn proxy_ssh_builds_pinned_argv_and_preserves_remote_arguments() {
        let ssh_args = args(&[
            "-L",
            "127.0.0.1:2222:localhost:22",
            "-o",
            "LogLevel=ERROR",
            "bash",
            "-s",
            "echo; touch /tmp/not-interpolated",
        ]);
        let executable = std::path::Path::new("/tmp/fleet path/fleet'bin");
        // A Fleet home that is not the passwd home: the recovery
        // paths must come from Fleet's home, never a literal `~`
        // (OpenSSH would expand that from the passwd database).
        let home = std::path::Path::new("/fleet-home");
        let argv = build_proxy_ssh_argv_with_home(
            executable,
            &fleet_config::Config::default(),
            home,
            &ssh_args,
        )
        .expect("valid SSH argv");
        let strings: Vec<&str> = argv.iter().map(|arg| arg.to_str().unwrap()).collect();
        for setting in [
            "StrictHostKeyChecking=yes",
            "HostName=100.99.113.103",
            "User=user",
            "UserKnownHostsFile=/fleet-home/.ssh/known_hosts_laptop",
            "IdentitiesOnly=yes",
            "ConnectTimeout=60",
            "ConnectionAttempts=3",
            "ServerAliveInterval=15",
            "ServerAliveCountMax=8",
            "ControlMaster=auto",
            "ControlPath=/fleet-home/.ssh/cm-%r@%h:%p",
            "ControlPersist=3600",
        ] {
            assert!(
                strings.windows(2).any(|pair| pair == ["-o", setting]),
                "missing fixed SSH option {setting}"
            );
        }
        assert!(strings.contains(&"/fleet-home/.ssh/id_ed25519"));
        assert!(
            strings.iter().all(|arg| !arg.contains('~')),
            "no literal ~ may reach OpenSSH: {strings:?}"
        );
        assert!(strings.iter().any(|arg| {
            *arg == "ProxyCommand='/tmp/fleet path/fleet'\\''bin' proxy connect --stdio --host '100.99.113.103' --port %p"
        }));
        assert!(strings.contains(&"-L"));
        assert!(strings.contains(&"127.0.0.1:2222:localhost:22"));
        let target = strings
            .iter()
            .position(|arg| *arg == "user@100.99.113.103")
            .expect("fixed destination");
        assert_eq!(strings[target + 1], "bash");
        assert_eq!(strings[target + 2], "-s");
        assert_eq!(strings[target + 3], "echo; touch /tmp/not-interpolated");
    }

    #[test]
    fn proxy_ssh_splits_clustered_options_and_operands() {
        // Clustered forwarding: `-vL` walks v (flag) then L
        // (operand-bearing, last in cluster) so the next argument is
        // L's operand and the command starts after it (#6679).
        let (options, command) = split_ssh_options_and_command(&args(&[
            "-vL",
            "127.0.0.1:2222:localhost:22",
            "bash",
            "-s",
        ]))
        .expect("clustered -vL splits");
        assert_eq!(options, args(&["-vL", "127.0.0.1:2222:localhost:22"]));
        assert_eq!(command, args(&["bash", "-s"]));

        // `-P` takes an operand (tag) even though the old
        // whole-argument table omitted it.
        let (options, command) =
            split_ssh_options_and_command(&args(&["-P", "mytag", "uptime"])).expect("-P splits");
        assert_eq!(options, args(&["-P", "mytag"]));
        assert_eq!(command, args(&["uptime"]));

        // Attached operand: the rest of the cluster is the operand,
        // so no following argument is consumed.
        let (options, command) =
            split_ssh_options_and_command(&args(&["-L127.0.0.1:2222:localhost:22", "uptime"]))
                .expect("attached -L splits");
        assert_eq!(options, args(&["-L127.0.0.1:2222:localhost:22"]));
        assert_eq!(command, args(&["uptime"]));

        // A clustered operand-bearing option with nothing after it
        // is an error, not a silent command start.
        assert!(
            split_ssh_options_and_command(&args(&["-vL"]))
                .unwrap_err()
                .contains("needs an argument")
        );
    }

    #[test]
    fn proxy_ssh_rejects_missing_option_operands_and_second_separator() {
        assert!(
            split_ssh_options_and_command(&args(&["-p"]))
                .unwrap_err()
                .contains("needs an argument")
        );
        assert!(
            split_ssh_options_and_command(&args(&["--", "other-host"]))
                .unwrap_err()
                .contains("destination is fixed")
        );
    }

    #[test]
    fn help_flag_positions_for_other_domains_are_unchanged() {
        let queue = domain("queue").expect("queue domain exists");
        assert!(wants_domain_help(queue, &args(&["--help"])));
        // A subcommand's own --help belongs to the domain's parser.
        assert!(!wants_domain_help(queue, &args(&["enqueue", "--help"])));
        assert!(!wants_domain_help(queue, &args(&[])));
    }

    #[test]
    fn root_builtins_reject_trailing_arguments() {
        // #5572: version / capabilities / --version silently discarded
        // trailing arguments and returned success.
        let surface = Surface::new(None, fleet_config::Config::default());
        for cmd in ["version", "capabilities", "--version", "-V"] {
            assert_eq!(
                run_root(&surface, &args(&[cmd, "--bogus"]), "fleet"),
                EXIT_USAGE,
                "{cmd} must reject a trailing argument"
            );
            assert_eq!(
                run_root(&surface, &args(&[cmd]), "fleet"),
                EXIT_OK,
                "bare {cmd} stays accepted"
            );
        }
    }

    #[test]
    fn root_help_rejects_arguments_after_the_domain_name() {
        // #5652: `fleet help queue --bogus` printed the queue help and
        // exited 0 — the help arm resolved args[1] as the domain and
        // never looked at args[2..], unlike every sibling built-in.
        let surface = Surface::new(None, fleet_config::Config::default());
        assert_eq!(
            run_root(&surface, &args(&["help", "queue", "--bogus"]), "fleet"),
            EXIT_USAGE,
            "help must reject an argument after the domain name"
        );
        assert_eq!(
            run_root(
                &surface,
                &args(&["help", "queue", "claim", "extra"]),
                "fleet"
            ),
            EXIT_USAGE,
            "help must reject the first of several extra arguments"
        );
        assert_eq!(
            run_root(&surface, &args(&["help", "queue"]), "fleet"),
            EXIT_OK,
            "help with exactly one domain name stays accepted"
        );
        assert_eq!(
            run_root(&surface, &args(&["help", "nosuchdomain"]), "fleet"),
            EXIT_USAGE,
            "an unknown domain name stays a usage error"
        );
    }

    #[test]
    fn queue_domain_version_rejects_trailing_arguments() {
        // #5603: the queue domain's --version shortcut in run_domain
        // checked only the first argument, so `fleet queue --version
        // --bogus` (and the repo-queue argv0 form) returned success.
        let surface = Surface::new(None, fleet_config::Config::default());
        let queue = domain("queue").expect("queue domain exists");
        for flag in ["--version", "-V"] {
            assert_eq!(
                run_domain(&surface, queue, &args(&[flag, "--bogus"]), "fleet"),
                EXIT_USAGE,
                "{flag} must reject a trailing argument"
            );
            assert_eq!(
                run_domain(&surface, queue, &args(&[flag]), "fleet"),
                EXIT_OK,
                "bare {flag} stays accepted"
            );
        }
    }
}
