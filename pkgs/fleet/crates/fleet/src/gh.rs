//! `fleet gh` — the real `gh` CLI behind one token rotation
//! (developing-today/code#8887, Drew's 2026-11-12 directive).
//!
//! Every argument passes through to the real `gh` binary unchanged;
//! the caller sees the final attempt's stdout, stderr and exit code
//! exactly as `gh` produced them. What this domain adds is the
//! identity: before each attempt it resolves one token by name from
//! the fleet secrets broker and exports it as `GH_TOKEN` for that
//! attempt only. On a failure class that proves the request was
//! refused before execution — a rate limit or a permission/scope
//! rejection — or on any failure of a read-only call, it fails over
//! to the next token in the rotation instead of failing the caller.
//!
//! The rotation, in order:
//!   1. installation tokens of the fleet's THREE GitHub Apps, in
//!      App order, minted in-process per owner: the fleet App
//!      ("developing.today fleet", App ID 5612056), the
//!      developing.today App, and the developing-today-backup App
//!      (the latter two carry their App IDs in the broker alongside
//!      their keys). Each App's private key is resolved by name, an
//!      RS256 JWT is signed with the system `openssl` (key passed
//!      over an inherited fd — never written to disk, never in
//!      argv), and the installation for the owner is resolved live
//!      from GET /app/installations (paginated, per the #7930
//!      lesson). Default owners: fleet App — developing-today,
//!      developing-today-forks, dezren39; developing.today App —
//!      developing-today, dezren39; backup App — developing-today.
//!      When the call names a repo or org, that owner's installation
//!      moves to the front of each App leg — an installation token
//!      only sees its own installation's scope.
//!      Additional `[[gh.apps]]` entries follow the built-ins in file
//!      order, using configured owner lists and named key secrets.
//!   2. the personal token, LAST (Drew, 2026-11-23, code#13310):
//!      Drew's `gh` OAuth token, parsed out of the broker secret
//!      named `github-cli.hosts-yml` (the hosts file itself, stored
//!      by name — the value never leaves the resolver). It is the
//!      final fallback, not the first identity tried.
//!
//! The App leg's own HTTPS validates against the PLATFORM root
//! store (ureq's platform-verifier), not webpki-roots alone: hosts
//! behind a TLS-intercepting egress proxy (the sandbox) present the
//! proxy's CA, which lives in the platform store — with webpki
//! roots the leg died with UnknownIssuer while `gh` itself worked.
//!
//! `fleet gh tiers` (a shadow subcommand; real `gh` has none)
//! prints every rotation source with its core and GraphQL
//! rate-limit headroom; `tiers --all` instead enumerates every
//! installation of every configured App, including owners omitted
//! from the rotation lists. Both exit 3 when any source is down.
//!
//! `fleet gh cache-serve` (also a shadow subcommand) runs the
//! credential/token cache daemon (code#10742): secret resolutions
//! (short TTL), App JWTs (within expiry), installation tokens
//! (until shortly before expiry) and per-App installation lists,
//! memory-only with single-flight minting. `fleet gh cache-status`
//! probes the configured daemons. Every normal call resolves through
//! the localhost daemon, then the laptop daemon, when one answers;
//! with no daemon it resolves in-process and says so on stderr.
//!
//! Raw mode — `fleet gh --raw <gh args...>` or FLEET_GH_RAW=1 —
//! is the first-class bypass (Drew, 2026-11-14): the real `gh`
//! execs directly with the caller's ambient credentials and
//! environment; no token is resolved or injected and there is
//! no rotation or failover. It exists so callers that must pin
//! one identity (and the `gh` -> `fleet gh` PATH shim, whose
//! escape hatch this is) never have to choose between the
//! rotation and bypassing fleet entirely.
//!
//! Retry-safety rule (the #8887 contract): a WRITE is only repeated
//! under a fresh token when the failure proves GitHub refused the
//! request before processing it — rate-limit rejections (the limiter
//! answers before the mutation runs) and permission/scope rejections
//! (INSUFFICIENT_SCOPES, "Resource not accessible by …") are both
//! pre-execution refusals. A "Could not resolve to a node" error is
//! NOT one: GraphQL executes mutation fields serially, so a batched
//! mutation may already have run an earlier field (e.g. an
//! `addComment`) before a later field's node fails to resolve — the
//! response can carry partial results alongside the error. Replaying
//! the whole mutation would duplicate the part that landed, so
//! node-resolution failures repeat only read-only calls, like any
//! other failure. Any other failure of a write is returned as-is:
//! the request may have landed, and repeating it could double-post.
//! Reads repeat on any failure class.
//!
//! Tokens are never printed, logged, or written to a file: provider
//! errors name the source and stage only, never values. Attempts
//! are silent by default — the final attempt's stdout, stderr and
//! exit code are byte-exact what `gh` produced (a caller capturing
//! both streams, as project-reconcile.sh does, must see gh's output
//! and nothing else); FLEET_GH_TRACE=1 names each attempt's token
//! *source* on stderr. Output capture note: attempts are captured
//! (piped), not streamed, because the failure text is what
//! classifies a failover.
//!
//! Controls (environment):
//!   FLEET_GH_BIN        the `gh` binary to run (default: `gh`)
//!   FLEET_GH_SHIM_ACTIVE set to 1 on every child `gh` fleet
//!                       spawns: the gh-fleet shim's recursion
//!                       guard, so a PATH `gh` that is the shim
//!                       execs the pristine gh instead of
//!                       re-entering a second fleet (#14650)
//!   FLEET_GH_APP_ID     GitHub App ID (default: 5612056)
//!   FLEET_GH_APP_OWNERS comma list replacing the default App owners
//!   FLEET_GH_ORDER      comma list fixing the rotation exactly:
//!                       `personal`, `app:<owner>` (fleet App),
//!                       `devtoday:<owner>`, `backup:<owner>`, or a
//!                       configured `<slug>:<owner>` entry
//!   FLEET_GH_DEVTODAY_OWNERS / FLEET_GH_BACKUP_OWNERS
//!                       comma lists replacing those Apps' owners
//!   FLEET_GH_TRACE=1    name each attempt's token source on stderr
//!   FLEET_GH_RAW=1      raw mode (same as a leading `--raw`):
//!                       exec the real gh with ambient credentials,
//!                       no rotation, no token injection
//!   FLEET_GH_CACHE      comma list of cache-daemon addresses
//!                       (`host[:port]`, default port 4802),
//!                       beating fleet config's `[gh]` table
//!   FLEET_GH_CACHE_BIND bind address for `cache-serve`
//!                       (default 127.0.0.1:4802)
//!
//! Close guard (#9177): `issue close` and `issue edit` with an
//! `--add-label status:done` are refused when the queue broker has a
//! claimed or parked task bound to that repo#issue, unless the bound
//! task's completion record (a `fleet-closure/v1` close record
//! carrying its task id) is presented with
//! `--completion-record <path>` — a wrapper-only flag, stripped
//! before gh runs. An issue without a live bound task is unaffected.
//! A guard refusal exits 2 and never reaches the token rotation.
//! The queue query fails open (a broker outage must not block every
//! close — the queue sweep's detective check is the net), and a
//! build without the queue client skips the guard the same way.
//!
//! Stdin values (#15556): `gh secret set` / `gh variable set`
//! without `--body` take the value from stdin — a one-shot stream
//! the first rotation attempt consumes. Before this fix a retryable
//! first attempt (rate limit / permission refusal) left the failover
//! attempt reading EOF, and real gh stores that as an EMPTY value
//! with exit 0: a silent provisioning failure. So when stdin is not
//! a terminal and supplies the value, fleet reads it once, refuses
//! an empty value loudly (exit 2, before any attempt), and replays
//! the same bytes to every attempt. An explicitly empty `--body`
//! is refused the same way. A terminal stdin is inherited as
//! before, so gh's interactive prompt is unchanged.
//!
//! Metadata create path (#9073, design §12–§15): `issue create` and
//! `pr create` bodies are stamped with the fleet Metadata block
//! (gh_metadata.rs) before the rotation runs, and the full record
//! is inserted into the metadata store after a successful create.
//! A malformed caller block refuses the create before any write;
//! a failed insert never loses the item (the miss is reported on
//! stderr and ingestion heals it). Controls: FLEET_METADATA_DB
//! (store path), FLEET_GH_STAMP_KEY_ID (stamp signing key id; the
//! key material is resolved by name from the secrets broker).

use crate::gh_cache;
use crate::gh_closure;
use crate::gh_content_ledger;
use crate::gh_guard;
use crate::gh_metadata;
use crate::gh_sync;
use fleet_config::{Config, DEFAULT_GH_CACHE_PORT};
use fleet_secrets::Resolver;
use std::io::{IsTerminal, Read, Write};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const EXIT_OK: i32 = 0;
const EXIT_USAGE: i32 = 2;
/// The final attempt's exit code is propagated as-is; 3 is ours only
/// for "no token source could be resolved at all" (matches the
/// secrets/dispatch unresolved exit shape).
const EXIT_UNRESOLVED: i32 = 3;
/// The shared content-creation ledger refused the reservation, so
/// the create did not run (code#2227). Mirrors gh-post's exit 5:
/// an unledgered filing is the failure mode the ledger exists to
/// kill, so the refusal is loud and counted, never silent.
const EXIT_LEDGER: i32 = 5;

fn usage_error(message: &str) -> i32 {
    eprintln!("error: {message}");
    EXIT_USAGE
}

const PERSONAL_SECRET_NAME: &str = "github-cli.hosts-yml";
const APP_KEY_SECRET_NAME: &str = "github-app.developing-today-fleet.private-key";
const DEFAULT_APP_ID: &str = "5612056";
const DEFAULT_APP_OWNERS: [&str; 3] = ["developing-today", "developing-today-forks", "dezren39"];
const DEVTODAY_APP_ID_SECRET_NAME: &str = "kaneo.github-app-id";
const DEVTODAY_APP_KEY_SECRET_NAME: &str = "kaneo.github-app-private-key";
const DEVTODAY_DEFAULT_OWNERS: [&str; 2] = ["developing-today", "dezren39"];
const BACKUP_APP_ID_SECRET_NAME: &str = "github-app.backup-app-id";
const BACKUP_APP_KEY_SECRET_NAME: &str = "github-app.backup-private-key";
const BACKUP_DEFAULT_OWNERS: [&str; 1] = ["developing-today"];
const GITHUB_API: &str = "https://api.github.com";

const USAGE: &str = "\
Usage: fleet gh <gh args...>
       fleet gh tiers [--all]
       fleet gh cache-serve [--bind <addr>]
       fleet gh cache-status
       fleet gh closures [<item>]
       fleet gh [--cache-addr <addr>...] [--no-cache] <gh args...>
       fleet gh --raw <gh args...>

Run the real `gh` CLI with every argument passed through unchanged,
under a token selected from the fleet rotation (built-in and configured
GitHub Apps' installation tokens, then the personal token last).
A rate-limit or
permission failure fails over to the next token; the caller sees only
the final attempt's output and exit code. `fleet gh tiers` lists each
rotation source with its rate-limit headroom. `fleet gh --raw` (or
FLEET_GH_RAW=1) bypasses the rotation entirely: the real gh runs with
the caller's ambient credentials, no token injected. See code#8887.

Credential cache (code#10742): token resolution goes through the
cache daemon when one answers — the localhost daemon first, then the
laptop daemon (fleet config `[gh] cache_local` / `cache_laptop`, or
FLEET_GH_CACHE as a comma list; an address without a port gets port
4802). `fleet gh cache-serve` runs the daemon (default bind
127.0.0.1:4802, `--bind` or FLEET_GH_CACHE_BIND to change it);
`fleet gh cache-status` probes the configured daemons. `--cache-addr`
overrides every daemon address for one call; `--no-cache` resolves
in-process. When no daemon answers, the call resolves in-process and
says so on stderr — never silently.

Sync verbs (code#10973, step 2.2 of #9016): the `sync-*` shadow
verbs are the typed Queue/GitHub sync surface for scripted flows
(pm-stack dispatch's fallback, the opencode claimant, job bodies).
Each declares its whole command line, validates it strictly, and
rewrites to the canonical gh call before the guard, closure and
metadata layers run, inheriting the rotation and the retry-safety
rule. `fleet gh sync-labels --repo OWNER/REPO <issue|pr> <n>` prints
an item's label names as JSON; `sync-view` prints labels,state,title
JSON; `sync-comment` posts a comment (`--body` or `--body-file`);
`sync-issue-create`, `sync-issue-get`, `sync-labels-add`,
`sync-label-ensure` and `sync-issue-close` (fleet closure reasons)
cover the issue writes dispatch composes. See gh_sync.rs.";

/// Built-in and config-defined GitHub Apps. Only secret names and
/// public IDs are configured; private keys resolve through the broker.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GhApp {
    Fleet,
    DevToday,
    Backup,
    Configured(String),
}

impl GhApp {
    const ALL: [GhApp; 3] = [GhApp::Fleet, GhApp::DevToday, GhApp::Backup];

    /// The FLEET_GH_ORDER / label prefix for this App's sources.
    fn slug(&self) -> &str {
        match self {
            GhApp::Fleet => "app",
            GhApp::DevToday => "devtoday",
            GhApp::Backup => "backup",
            GhApp::Configured(slug) => slug,
        }
    }

    /// Parse a slug back to its App (the cache wire names Apps by
    /// slug, so the daemon side resolves them without importing
    /// the rotation's enum spelling twice).
    fn parse(slug: &str, config: &Config) -> Option<GhApp> {
        match slug {
            "app" => Some(GhApp::Fleet),
            "devtoday" => Some(GhApp::DevToday),
            "backup" => Some(GhApp::Backup),
            _ => config
                .gh
                .apps
                .iter()
                .find(|app| app.slug == slug)
                .map(|app| GhApp::Configured(app.slug.clone())),
        }
    }

    fn key_secret<'a>(&self, config: &'a Config) -> &'a str {
        match self {
            GhApp::Fleet => APP_KEY_SECRET_NAME,
            GhApp::DevToday => DEVTODAY_APP_KEY_SECRET_NAME,
            GhApp::Backup => BACKUP_APP_KEY_SECRET_NAME,
            GhApp::Configured(slug) => {
                &config
                    .gh
                    .apps
                    .iter()
                    .find(|app| app.slug == *slug)
                    .expect("configured gh app")
                    .key_secret
            }
        }
    }

    /// The secret carrying the App ID, unless a public ID is used.
    fn id_secret<'a>(&self, config: &'a Config) -> Option<&'a str> {
        match self {
            GhApp::Fleet => None,
            GhApp::DevToday => Some(DEVTODAY_APP_ID_SECRET_NAME),
            GhApp::Backup => Some(BACKUP_APP_ID_SECRET_NAME),
            GhApp::Configured(slug) => config
                .gh
                .apps
                .iter()
                .find(|app| app.slug == *slug)
                .expect("configured gh app")
                .app_id_secret
                .as_deref(),
        }
    }

    fn default_owners(&self) -> &'static [&'static str] {
        match self {
            GhApp::Fleet => &DEFAULT_APP_OWNERS,
            GhApp::DevToday => &DEVTODAY_DEFAULT_OWNERS,
            GhApp::Backup => &BACKUP_DEFAULT_OWNERS,
            GhApp::Configured(_) => &[],
        }
    }
}

/// One identity the rotation can run `gh` under. The label is all
/// that ever reaches stderr; the token value stays in the provider.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TokenSource {
    Personal,
    AppInstallation { app: GhApp, owner: String },
}

impl TokenSource {
    fn app(app: GhApp, owner: &str) -> TokenSource {
        TokenSource::AppInstallation {
            app,
            owner: owner.to_string(),
        }
    }

    fn label(&self) -> String {
        match self {
            TokenSource::Personal => "personal".to_string(),
            TokenSource::AppInstallation { app, owner } => {
                format!("{}:{owner}", app.slug())
            }
        }
    }
}

/// Why an attempt failed, as far as the retry decision cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureClass {
    /// GitHub's limiter refused the request before processing it.
    RateLimit,
    /// Authorization refused the request before processing it
    /// (missing scope, resource not accessible to this identity).
    Permission,
    /// A node the identity cannot see failed to resolve ("Could not
    /// resolve to a node"). GitHub hides inaccessible nodes this
    /// way, but resolution is per-field, not a whole-operation
    /// pre-execution check: in a batched mutation an earlier field
    /// may already have executed, so this class repeats only
    /// read-only calls (#12154 review).
    NodeResolution,
    /// Anything else: the request may or may not have landed.
    Other,
}

/// Classify a failed attempt from its captured output.
/// Oracle: the real gh CLI's and GitHub API's refusal strings, as
/// observed fleet-side (code#8887, code#1408, code#6129) — the same
/// strings project-reconcile.sh's `is_limit` ladder keys on, widened
/// with the permission shapes.
fn classify_failure(output: &str) -> FailureClass {
    let lower = output.to_lowercase();
    if lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("too many requests")
        || lower.contains("http 429")
    {
        return FailureClass::RateLimit;
    }
    // GraphQL hides nodes the identity cannot see as unresolvable
    // rather than inaccessible — but resolution is per-field, not a
    // whole-operation pre-execution check, so a batched mutation may
    // already have partially executed (#12154 review). This class
    // repeats read-only calls only; see may_retry.
    if lower.contains("could not resolve to a node") {
        return FailureClass::NodeResolution;
    }
    if lower.contains("missing required scopes")
        || lower.contains("insufficient_scopes")
        || lower.contains("resource not accessible")
        || lower.contains("not accessible by integration")
        || lower.contains("not accessible by personal access token")
        || lower.contains("this endpoint requires you to be authenticated")
        || lower.contains("bad credentials")
        || lower.contains("http 401")
    {
        return FailureClass::Permission;
    }
    FailureClass::Other
}

/// The `gh` command word, past any inherited flags given before it
/// (`gh -R o/r secret set ...`). `-R`/`--repo` take a value.
fn command_word(args: &[String]) -> Option<&str> {
    let mut i = 0;
    while let Some(a) = args.get(i) {
        let a = a.as_str();
        if a == "-R" || a == "--repo" {
            i += 2;
        } else if a.starts_with('-') {
            i += 1;
        } else {
            return Some(a);
        }
    }
    None
}

/// The call's arguments as they go into the audit log, with the
/// values that are themselves secrets replaced: `gh secret set`
/// bodies and `Authorization` headers. Only the logged copy changes;
/// `gh` still gets the real arguments.
fn audit_args(args: &[String]) -> Vec<String> {
    const REDACTED: &str = "[redacted]";
    let is_secret = command_word(args) == Some("secret");
    // A header is redacted when its name can carry a credential: the
    // standard auth and cookie headers, and any custom header whose name
    // mentions a token, key, secret, password, session or auth.
    let redact_header = |v: &str| -> Option<String> {
        let (name, _) = v.split_once(':')?;
        let name = name.trim();
        let lower = name.to_ascii_lowercase();
        let sensitive = [
            "auth",
            "cookie",
            "token",
            "secret",
            "key",
            "password",
            "session",
            "credential",
        ]
        .iter()
        .any(|w| lower.contains(w));
        sensitive.then(|| format!("{name}: {REDACTED}"))
    };
    let mut out = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if is_secret && (a == "--body" || a == "-b") {
            out.push(a.to_string());
            if i + 1 < args.len() {
                out.push(REDACTED.to_string());
            }
            i += 2;
            continue;
        }
        if is_secret && (a.starts_with("--body=") || (a.starts_with("-b") && a.len() > 2)) {
            let flag = if a.starts_with("--body=") {
                "--body="
            } else {
                "-b"
            };
            out.push(format!("{flag}{REDACTED}"));
            i += 1;
            continue;
        }
        if a == "-H" || a == "--header" {
            if let Some(r) = args.get(i + 1).and_then(|v| redact_header(v)) {
                out.push(a.to_string());
                out.push(r);
                i += 2;
                continue;
            }
        }
        // Attached spellings: --header=V, -HV and -H=V all send V.
        let attached = a
            .strip_prefix("--header=")
            .map(|v| ("--header=", v))
            .or_else(|| a.strip_prefix("-H=").map(|v| ("-H=", v)))
            .or_else(|| a.strip_prefix("-H").map(|v| ("-H", v)));
        if let Some((flag, v)) = attached {
            if let Some(r) = redact_header(v) {
                out.push(format!("{flag}{r}"));
                i += 1;
                continue;
            }
        }
        out.push(a.to_string());
        i += 1;
    }
    out
}

/// Append one audit record as a single locked write, so records from
/// concurrent fleet processes sharing the file never interleave.
fn append_audit_record(path: &std::path::Path, entry: &serde_json::Value) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = format!("{entry}\n");
    // SAFETY: flock on an fd this function owns for its whole lifetime.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let written = f.write_all(line.as_bytes()).and_then(|()| f.flush());
    // Closing the file releases the lock too; unlock explicitly anyway.
    unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_UN) };
    written
}

/// Is this `gh` invocation read-only? Only the shapes the fleet
/// actually scripts are whitelisted; anything unrecognized is a
/// write (the safe default: writes repeat only on pre-execution
/// refusals). Oracle: `gh <verb> --help` — list/view/status verbs
/// mutate nothing; `api` mutates only when fields or an explicit
/// non-GET method make it POST-class.
fn is_read_call(args: &[String]) -> bool {
    let Some(verb) = args.first().map(|s| s.as_str()) else {
        return false;
    };
    match verb {
        // A clustered short option such as `-it VALUE` hides which
        // token is the endpoint and which is a value, so it is never
        // trusted as a read (#12157 review).
        "api" if args[1..].iter().any(|a| is_short_option_cluster(a)) => false,
        "api" if api_endpoint(&args[1..]) == Some("graphql") => is_graphql_read(&args[1..]),
        "api" => {
            // gh api: GET unless --method says otherwise or a field
            // flag (-f/-F/--field/--raw-field/--input) implies POST.
            let mut i = 1;
            while i < args.len() {
                let a = args[i].as_str();
                // `-fkey=value` / `-Fkey=value` are the same field
                // flags with the value attached.
                if a.starts_with("-f")
                    || a.starts_with("-F")
                    || a == "--field"
                    || a == "--raw-field"
                    || a == "--input"
                    || a.starts_with("--field=")
                    || a.starts_with("--raw-field=")
                    || a.starts_with("--input=")
                {
                    return false;
                }
                if a == "--method" || a == "-X" {
                    match args.get(i + 1).map(|s| s.as_str()) {
                        Some("GET") | Some("HEAD") => {}
                        _ => return false,
                    }
                    i += 1;
                } else if let Some(m) = a.strip_prefix("--method=").or(a.strip_prefix("-X")) {
                    if m != "GET" && m != "HEAD" {
                        return false;
                    }
                }
                i += 1;
            }
            true
        }
        "list" | "view" | "status" | "search" | "diff" | "checks" => true,
        "project" | "issue" | "pr" | "repo" | "label" | "release" | "run" | "workflow" | "gist"
        | "cache" | "ruleset" | "secret" | "variable" | "org" | "alias" | "config" => {
            matches!(
                args.get(1).map(|s| s.as_str()),
                Some("list" | "view" | "status" | "field-list" | "item-list" | "diff" | "checks")
            )
        }
        "auth" => matches!(args.get(1).map(|s| s.as_str()), Some("status" | "token")),
        _ => false,
    }
}

/// `gh api` flags that take the next argument as their value
/// (`gh api --help`), so that value is not mistaken for the endpoint.
const GH_API_VALUE_FLAGS: [&str; 17] = [
    "-f",
    "-F",
    "--field",
    "--raw-field",
    "--input",
    "-X",
    "--method",
    "-H",
    "--header",
    "-q",
    "--jq",
    "-t",
    "--template",
    "-p",
    "--preview",
    "--hostname",
    "--cache",
];

/// The endpoint argument of a `gh api` call (the first positional),
/// given the arguments after `api`.
/// Is `a` several short flags run together, like `-it`? gh api's
/// short options that take a value (`-f -F -H -X -p -q -t`) may carry
/// it attached (`-tTEMPLATE`), which is one option, not a cluster.
/// Any other short option longer than two characters starts with a
/// boolean flag (`-i`) followed by more flags.
fn is_short_option_cluster(a: &str) -> bool {
    let mut chars = a.chars();
    chars.next() == Some('-')
        && a.len() > 2
        && !a.starts_with("--")
        && !matches!(chars.next(), Some('f' | 'F' | 'H' | 'X' | 'p' | 'q' | 't'))
}

fn api_endpoint(args: &[String]) -> Option<&str> {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if GH_API_VALUE_FLAGS.contains(&a) {
            i += 2;
            continue;
        }
        if !a.starts_with('-') {
            return Some(a);
        }
        i += 1;
    }
    None
}

/// Is this `gh api graphql` call a query rather than a mutation?
/// GraphQL always POSTs, so the field flags say nothing; the
/// operation does. Read-only only when the `query` field is given
/// inline and its operation is an anonymous `{ ... }` selection or a
/// named `query`. A query read from a file (`query=@path`) or stdin
/// (`--input`), or any operation that cannot be told apart, counts
/// as a write (#12154).
fn is_graphql_read(args: &[String]) -> bool {
    let mut query: Option<&str> = None;
    let mut queries = 0;
    let mut typed = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--input" || a.starts_with("--input=") {
            return false;
        }
        let is_field_flag = a == "-f" || a == "-F" || a == "--field" || a == "--raw-field";
        if !is_field_flag && GH_API_VALUE_FLAGS.contains(&a) {
            // Another option's value (a --template, --jq, header...) is
            // never a field, even when it looks like `-fquery=...`.
            i += 2;
            continue;
        }
        // `-F`/`--field` values are typed: gh fills `{owner}`, `{repo}`
        // and `{branch}` in them before sending.
        let is_typed = a == "-F"
            || a == "--field"
            || a.starts_with("--field=")
            || (a.starts_with("-F") && a.len() > 2);
        let field = if is_field_flag {
            i += 1;
            args.get(i).map(|s| s.as_str())
        } else {
            a.strip_prefix("--field=")
                .or(a.strip_prefix("--raw-field="))
                .or(a.strip_prefix("-f").filter(|v| !v.is_empty()))
                .or(a.strip_prefix("-F").filter(|v| !v.is_empty()))
        };
        if let Some(v) = field.and_then(|f| f.strip_prefix("query=")) {
            query = Some(v);
            typed = is_typed;
            queries += 1;
        }
        i += 1;
    }
    let Some(query) = query else {
        return false;
    };
    // Which of several query fields gh sends is not ours to guess.
    if queries > 1 {
        return false;
    }
    if query.starts_with('@') {
        return false;
    }
    // A typed field's placeholders are expanded by gh, so the text we
    // see is not the operation gh sends; `{repo}` could become
    // `mutation`. Treat any expandable placeholder as a write.
    if typed
        && ["{owner}", "{repo}", "{branch}"]
            .iter()
            .any(|p| query.contains(p))
    {
        return false;
    }
    // A document can hold several operations and `operationName` picks
    // one, so a mutation or subscription anywhere in it makes the call
    // a write. The word scan is coarse on purpose: a `mutation` inside
    // a string or comment only errs toward treating a read as a write.
    let has_write_operation = query
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| word == "mutation" || word == "subscription");
    if has_write_operation {
        return false;
    }
    let mut rest = query.trim_start();
    // Skip leading `#` comments.
    while let Some(after) = rest.strip_prefix('#') {
        rest = after
            .split_once('\n')
            .map_or("", |(_, tail)| tail)
            .trim_start();
    }
    if rest.starts_with('{') {
        return true;
    }
    match rest.strip_prefix("query") {
        Some(tail) => !tail.starts_with(|c: char| c.is_alphanumeric() || c == '_'),
        None => false,
    }
}

/// The owner a call is aimed at, when the arguments name one —
/// `--repo`/`-R owner/repo`, `--owner org`, an `api repos/<owner>/…`
/// path, or a github.com URL. The matching App installation leads
/// the App leg because installation tokens are scope-bound.
fn owner_hint(args: &[String]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let value = if a == "--repo" || a == "-R" || a == "--owner" {
            args.get(i + 1).map(|s| s.as_str())
        } else if let Some(v) = a
            .strip_prefix("--repo=")
            .or(a.strip_prefix("-R"))
            .or(a.strip_prefix("--owner="))
        {
            Some(v)
        } else {
            None
        };
        if let Some(v) = value {
            let owner = v.split('/').next().unwrap_or("");
            if !owner.is_empty() && owner != "." && owner != ".." {
                return Some(owner.to_string());
            }
        }
        if let Some(rest) = a.strip_prefix("repos/") {
            let owner = rest.split('/').next().unwrap_or("");
            if !owner.is_empty() {
                return Some(owner.to_string());
            }
        }
        if let Some(idx) = a.find("github.com/") {
            let owner = a[idx + "github.com/".len()..]
                .split('/')
                .next()
                .unwrap_or("");
            if !owner.is_empty() {
                return Some(owner.to_string());
            }
        }
        i += 1;
    }
    None
}

/// Per-App owner overrides, from the FLEET_GH_*_OWNERS environment
/// variables; None for an App means its compiled-in defaults.
#[derive(Default)]
struct Owners {
    fleet: Option<Vec<String>>,
    devtoday: Option<Vec<String>>,
    backup: Option<Vec<String>>,
}

impl Owners {
    fn from_env() -> Owners {
        let parse = |name: &str| {
            std::env::var(name).ok().map(|list| {
                list.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
        };
        Owners {
            fleet: parse("FLEET_GH_APP_OWNERS"),
            devtoday: parse("FLEET_GH_DEVTODAY_OWNERS"),
            backup: parse("FLEET_GH_BACKUP_OWNERS"),
        }
    }

    fn for_app(&self, app: &GhApp, config: &Config) -> Vec<String> {
        let over = match app {
            GhApp::Fleet => &self.fleet,
            GhApp::DevToday => &self.devtoday,
            GhApp::Backup => &self.backup,
            GhApp::Configured(slug) => {
                return config
                    .gh
                    .apps
                    .iter()
                    .find(|app| app.slug == *slug)
                    .expect("configured gh app")
                    .owners
                    .clone();
            }
        };
        over.clone()
            .unwrap_or_else(|| app.default_owners().iter().map(|s| s.to_string()).collect())
    }
}

/// The rotation for one invocation: each App's owners with the
/// hinted owner (if any) leading each App leg, then the personal
/// token LAST. `FLEET_GH_ORDER` replaces the whole order when set.
/// Oracle: code#8887's rotation definition as extended by Drew
/// 2026-11-12 (personal + all three fleet Apps' installations),
/// hint-first per the installation-scope fact; personal moved last
/// by Drew 2026-11-23 (code#13310).
fn rotation(args: &[String], config: &Config) -> Vec<TokenSource> {
    let order_env = std::env::var("FLEET_GH_ORDER").ok();
    rotation_with_config(args, order_env.as_deref(), &Owners::from_env(), config)
}

/// `rotation` with its environment inputs made explicit, so the
/// order logic is testable without env mutation races.
#[cfg(test)]
fn rotation_with(
    args: &[String],
    order_env: Option<&str>,
    owners_env: &Owners,
) -> Vec<TokenSource> {
    rotation_with_config(args, order_env, owners_env, &Config::default())
}

fn rotation_with_config(
    args: &[String],
    order_env: Option<&str>,
    owners_env: &Owners,
    config: &Config,
) -> Vec<TokenSource> {
    if let Some(order) = order_env {
        let parsed: Vec<TokenSource> = order
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| {
                if let Some((slug, owner)) = s.split_once(':') {
                    if let Some(app) = GhApp::parse(slug, config) {
                        return TokenSource::app(app, owner);
                    }
                }
                TokenSource::Personal
            })
            .collect();
        if !parsed.is_empty() {
            return parsed;
        }
    }
    let hint = owner_hint(args);
    let mut out = Vec::new();
    let apps = GhApp::ALL.into_iter().chain(
        config
            .gh
            .apps
            .iter()
            .map(|app| GhApp::Configured(app.slug.clone())),
    );
    for app in apps {
        let mut owners = owners_env.for_app(&app, config);
        if let Some(hint) = &hint {
            if let Some(pos) = owners.iter().position(|o| o.eq_ignore_ascii_case(hint)) {
                let owner = owners.remove(pos);
                owners.insert(0, owner);
            }
        }
        out.extend(
            owners
                .into_iter()
                .map(|o| TokenSource::app(app.clone(), &o)),
        );
    }
    // Drew 2026-11-23 (code#13310): the personal token is the final
    // fallback, never the first identity tried.
    out.push(TokenSource::Personal);
    out
}

/// May this failed attempt be repeated under the next token?
/// The #8887 retry-safety rule: pre-execution refusals (rate limit,
/// permission) may repeat anything; other failures may repeat only
/// reads. Oracle: the rule as stated in this module's header and on
/// code#8887.
fn may_retry(class: FailureClass, read_only: bool, attempts_left: bool) -> bool {
    if !attempts_left {
        return false;
    }
    match class {
        FailureClass::RateLimit | FailureClass::Permission => true,
        // Node resolution (and anything else) may have partially
        // executed: repeat only when the call cannot have landed.
        FailureClass::NodeResolution | FailureClass::Other => read_only,
    }
}

// ---------------------------------------------------------------------------
// Token providers. A provider error is a value-free string: source,
// stage, HTTP status — never key or token material.
// ---------------------------------------------------------------------------

/// Where tokens come from. Abstracted so tests drive the rotation
/// with fake identities and a fake `gh` (the oracle tests below).
trait TokenProvider {
    fn token_for(&mut self, source: &TokenSource) -> Result<String, String>;
}

/// Raw credential operations, with no caching: the
/// fleet-secrets resolver chain for named secrets, plus in-process
/// App installation minting. Both the cache daemon and the
/// no-daemon in-process fallback compose the same
/// [`gh_cache::Cache`] over this backend, so one implementation
/// serves both paths (code#10742).
struct RawMinter {
    config: Config,
    agent: ureq::Agent,
    app_ids: Mutex<Vec<(GhApp, String)>>,
}

impl RawMinter {
    fn new(config: &Config) -> Self {
        // Platform root store, not webpki-roots alone: hosts
        // behind a TLS-intercepting egress proxy present the
        // proxy's CA, which only the platform store carries (the
        // sandbox App leg died with UnknownIssuer before this).
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(60)))
            .timeout_connect(Some(std::time::Duration::from_secs(10)))
            .http_status_as_error(false)
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .into();
        RawMinter {
            config: config.clone(),
            agent,
            app_ids: Mutex::new(Vec::new()),
        }
    }

    fn resolve_named(&self, name: &str) -> Result<String, String> {
        Resolver::default_chain(&self.config)
            .resolve(name)
            .map(|v| v.expose().to_string())
            .map_err(|e| format!("secret '{name}' unresolved: {e}"))
    }

    /// Resolve and memoize an App ID. Configured Apps may use a public
    /// literal or the same named-secret resolver as the built-in Apps.
    fn app_id(&self, app: &GhApp) -> Result<String, String> {
        self.app_id_with(app, |name| self.resolve_named(name))
    }

    fn app_id_with(
        &self,
        app: &GhApp,
        resolve: impl FnOnce(&str) -> Result<String, String>,
    ) -> Result<String, String> {
        if let Some((_, id)) = self
            .app_ids
            .lock()
            .expect("gh app-id lock")
            .iter()
            .find(|(a, _)| a == app)
        {
            return Ok(id.clone());
        }
        let id = match app.id_secret(&self.config) {
            None if matches!(app, GhApp::Fleet) => std::env::var("FLEET_GH_APP_ID")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_APP_ID.to_string()),
            Some(name) => resolve(name)?.trim().to_string(),
            None => self
                .config
                .gh
                .apps
                .iter()
                .find(|entry| entry.slug == app.slug())
                .and_then(|entry| entry.app_id)
                .expect("configured gh app id")
                .to_string(),
        };
        if id.is_empty() {
            return Err(format!("app id for {} resolved empty", app.slug()));
        }
        self.app_ids
            .lock()
            .expect("gh app-id lock")
            .push((app.clone(), id.clone()));
        Ok(id)
    }
}

impl gh_cache::Backend for RawMinter {
    fn resolve_secret(&self, name: &str) -> Result<String, String> {
        self.resolve_named(name)
    }

    /// One App's JWT: header and payload are public claims; only
    /// the signature touches the private key, and that happens
    /// inside `openssl` with the key arriving on an inherited fd.
    /// The TTL is the JWT's own lifetime minus the cache skew, so
    /// the cache serves it only within expiry.
    fn sign_jwt(&self, app: &str) -> Result<(String, u64), String> {
        let app =
            GhApp::parse(app, &self.config).ok_or_else(|| format!("unknown gh app {app:?}"))?;
        let app_id = self.app_id(&app)?;
        let key_pem = self.resolve_named(app.key_secret(&self.config))?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock before epoch".to_string())?
            .as_secs();
        let header = base64url(br#"{"alg":"RS256","typ":"JWT"}"#);
        let payload = base64url(
            format!(
                "{{\"iat\":{},\"exp\":{},\"iss\":\"{}\"}}",
                now.saturating_sub(60),
                now + 540,
                app_id
            )
            .as_bytes(),
        );
        let signing_input = format!("{header}.{payload}");
        let signature = openssl_rs256_sign(key_pem.as_bytes(), signing_input.as_bytes())?;
        let jwt = format!("{signing_input}.{}", base64url(&signature));
        Ok((jwt, 540u64.saturating_sub(gh_cache::EXPIRY_SKEW_SECS)))
    }

    /// One App's installations as `(owner_login, id)` pairs,
    /// resolved live and paginated: the App has 100+ installations
    /// and a first-page-only read silently loses the fleet's (the
    /// fleet-paperwork #7930 break).
    fn list_installations(&self, app: &str) -> Result<Vec<(String, i64)>, String> {
        let (jwt, _) = self.sign_jwt(app)?;
        let mut pairs: Vec<(String, i64)> = Vec::new();
        let mut page = 1;
        loop {
            let url = format!("{GITHUB_API}/app/installations?per_page=100&page={page}");
            let (status, body) = api_call(&self.agent, "GET", &url, &jwt, None)?;
            if status != 200 {
                return Err(format!(
                    "app installations list failed: HTTP {status} (page {page})"
                ));
            }
            let entries: serde_json::Value =
                serde_json::from_str(&body).map_err(|_| "installations: unparseable body")?;
            let Some(list) = entries.as_array() else {
                return Err("installations: body is not an array".to_string());
            };
            let count = list.len();
            for entry in list {
                let login = entry
                    .pointer("/account/login")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !login.is_empty()
                    && let Some(id) = entry.get("id").and_then(|v| v.as_i64())
                {
                    pairs.push((login.to_string(), id));
                }
            }
            if count < 100 {
                break;
            }
            page += 1;
        }
        Ok(pairs)
    }

    /// Mint an installation token for one installation id. The TTL
    /// comes from the response's `expires_at`, minus the cache
    /// skew: the cache serves the token until shortly before its
    /// own expiry.
    fn mint_token(&self, app: &str, installation_id: i64) -> Result<(String, u64), String> {
        let (jwt, _) = self.sign_jwt(app)?;
        let url = format!("{GITHUB_API}/app/installations/{installation_id}/access_tokens");
        let (status, body) = api_call(&self.agent, "POST", &url, &jwt, Some("{}"))?;
        if status != 201 {
            return Err(format!("installation token mint failed: HTTP {status}"));
        }
        let parsed: serde_json::Value =
            serde_json::from_str(&body).map_err(|_| "mint response unparseable")?;
        let token = parsed
            .get("token")
            .and_then(|v| v.as_str())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| "mint response carried no token".to_string())?
            .to_string();
        let ttl = parsed
            .get("expires_at")
            .and_then(|v| v.as_str())
            .map(gh_cache::token_ttl_from_expires_at)
            .unwrap_or(3500);
        Ok((token, ttl))
    }
}

/// The production provider: a process-local [`gh_cache::Cache`]
/// over [`RawMinter`], fronted by the cache daemons when one
/// answers. Daemon addresses arrive ordered (localhost first, then
/// the laptop daemon); the first answer wins per request.
struct FleetTokenProvider {
    cache: gh_cache::Cache<RawMinter>,
    addrs: Vec<String>,
    daemon_dead: bool,
    noted: bool,
}

impl FleetTokenProvider {
    fn new(config: &Config, addrs: Vec<String>) -> Self {
        FleetTokenProvider {
            cache: gh_cache::Cache::new(RawMinter::new(config)),
            addrs,
            daemon_dead: false,
            noted: false,
        }
    }

    /// One App's live installation list, through the process-local
    /// cache (the daemon wire has no installations-list op, so this
    /// always mints in-process; the TTL still applies per call).
    fn installations(&self, app: &GhApp) -> Result<Vec<(String, i64)>, String> {
        self.cache.installations(app.slug())
    }

    /// Resolve one cache request: the first answering daemon wins;
    /// when none answers, the process-local cache mints instead —
    /// with one visible note on stderr, never silence (code#10742).
    /// A daemon that ANSWERS with an error is authoritative: its
    /// error is the result, not a cue to mint locally (the local
    /// mint would fail the same way a moment later).
    fn via(
        &mut self,
        req: serde_json::Value,
        local: impl FnOnce(&gh_cache::Cache<RawMinter>) -> Result<String, String>,
    ) -> Result<String, String> {
        if !self.daemon_dead && !self.addrs.is_empty() {
            match gh_cache::fetch_any(&self.addrs, &req, gh_cache::CONNECT_TIMEOUT) {
                Ok(resp) => return gh_cache::response_token(&resp),
                Err(e) => {
                    self.daemon_dead = true;
                    if !self.noted {
                        self.noted = true;
                        eprintln!(
                            "fleet gh: no cache daemon answered (tried {}: {e}); resolving in-process",
                            self.addrs.join(", ")
                        );
                    }
                }
            }
        }
        local(&self.cache)
    }
}

impl TokenProvider for FleetTokenProvider {
    fn token_for(&mut self, source: &TokenSource) -> Result<String, String> {
        match source {
            TokenSource::Personal => {
                // The `personal` op resolves the hosts.yml SECRET,
                // not a finished token: the daemon is a dumb secret
                // cache for this op (it never learns which secret
                // personal means), and the process-local cache
                // returns the same secret. Parse the token out of
                // whichever copy answered. Parsing only the local
                // copy once handed gh the whole hosts.yml as
                // GH_TOKEN whenever a daemon answered, and gh
                // refused it client-side: net/http: invalid header
                // field value for "Authorization" — reads failed
                // over to an App token and masked it; writes, which
                // repeat only on pre-execution refusals, surfaced
                // it (2026-11-22).
                let req = gh_cache::request("personal", None, None, Some(PERSONAL_SECRET_NAME));
                let hosts = self.via(req, |cache| cache.secret(PERSONAL_SECRET_NAME))?;
                personal_token(&hosts)
            }
            TokenSource::AppInstallation { app, owner } => {
                let req = gh_cache::request("token", Some(app.slug()), Some(owner), None);
                self.via(req, |cache| cache.installation_token(app.slug(), owner))
            }
        }
    }
}

/// The personal rotation token inside its hosts.yml secret: the
/// one place daemon-resolved and locally-resolved copies of the
/// secret converge, so neither path can skip the parse.
fn personal_token(hosts_yml: &str) -> Result<String, String> {
    parse_hosts_yml_token(hosts_yml)
        .ok_or_else(|| format!("secret '{PERSONAL_SECRET_NAME}' carries no oauth_token"))
}

/// Extract the `oauth_token` value from gh's hosts.yml content.
/// Line-shaped on purpose: the file is a small fixed-shape config,
/// and a full YAML dependency for one string is not warranted.
fn parse_hosts_yml_token(hosts_yml: &str) -> Option<String> {
    for line in hosts_yml.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("oauth_token:") {
            let value = rest.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// One GitHub API call with a bearer token. Returns (status, body);
/// transport failures are value-free errors.
fn api_call(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    bearer: &str,
    body: Option<&str>,
) -> Result<(u16, String), String> {
    let auth = format!("Bearer {bearer}");
    let mut resp = match (method, body) {
        ("GET", _) => agent
            .get(url)
            .header("Authorization", &auth)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "fleet-gh")
            .call()
            .map_err(|e| format!("github api transport error: {e}"))?,
        _ => agent
            .post(url)
            .header("Authorization", &auth)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "fleet-gh")
            .send(body.unwrap_or("{}"))
            .map_err(|e| format!("github api transport error: {e}"))?,
    };
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("github api body read error: {e}"))?;
    Ok((status, text))
}

/// RS256-sign `data` with an RSA private key (PEM) by handing the
/// key to `openssl dgst` on inherited fd 3. The key is never
/// written to a file and never appears in argv or the environment.
fn openssl_rs256_sign(key_pem: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    use std::os::unix::io::BorrowedFd;
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err("openssl sign: pipe creation failed".to_string());
    }
    let (read_fd, write_fd) = (fds[0], fds[1]);
    // The key is ~1.7 KB — far under the 64 KB pipe buffer, so this
    // write completes before the child starts and cannot deadlock.
    let written = unsafe {
        libc::write(
            write_fd,
            key_pem.as_ptr() as *const libc::c_void,
            key_pem.len(),
        )
    };
    unsafe {
        libc::close(write_fd);
    }
    if written < 0 || written as usize != key_pem.len() {
        unsafe { libc::close(read_fd) };
        return Err("openssl sign: key pipe write failed".to_string());
    }
    // The child must inherit read_fd: clear its CLOEXEC flag and
    // keep the fd alive in this process until spawn returns.
    unsafe {
        let flags = libc::fcntl(read_fd, libc::F_GETFD);
        libc::fcntl(read_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
    }
    let key_arg = format!("/dev/fd/{read_fd}");
    let mut child = Command::new("openssl")
        .args(["dgst", "-sha256", "-sign", &key_arg])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("openssl sign: spawn failed: {e}"))?;
    // The child has its own copy of read_fd after spawn; ours can go.
    unsafe {
        let fd = BorrowedFd::borrow_raw(read_fd);
        drop(fd);
        libc::close(read_fd);
    }
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(data);
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("openssl sign: wait failed: {e}"))?;
    if !out.status.success() {
        return Err("openssl sign: signing failed (key shape?)".to_string());
    }
    Ok(out.stdout)
}

/// base64url without padding (RFC 7515). Twenty lines beats a
/// dependency for the two JWT segments and one signature.
fn base64url(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0] as u32,
            *chunk.get(1).unwrap_or(&0) as u32,
            *chunk.get(2).unwrap_or(&0) as u32,
        ];
        let n = (b[0] << 16) | (b[1] << 8) | b[2];
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[n as usize & 63] as char);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The passthrough runner.
// ---------------------------------------------------------------------------

struct Attempt {
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Remedy for a `gh` spawn failure (#10311 (d)): the binary is
/// missing or not executable. Names the fix so the caller is not
/// left guessing: install `gh`, or point FLEET_GH_BIN at it.
fn spawn_failure_message(gh_bin: &str, e: &std::io::Error) -> String {
    format!("failed to run '{gh_bin}': {e} (install gh, or set FLEET_GH_BIN to the gh binary)")
}

/// Distinct provider errors, first-seen order (#10311 (d)): every
/// source that fails the same way (e.g. the same broker outage)
/// reports once, then the summary line.
fn distinct_errors(errors: &[String]) -> Vec<&str> {
    let mut seen: Vec<&str> = Vec::new();
    for e in errors {
        if !seen.contains(&e.as_str()) {
            seen.push(e.as_str());
        }
    }
    seen
}

/// The gh-fleet shim's recursion guard (pkgs/gh-fleet): a child
/// `gh` that sees this set to 1 execs the pristine gh directly
/// instead of routing through `fleet gh`. Fleet sets it on every
/// `gh` it spawns — fleet IS the routing layer, so a child that
/// re-entered fleet would run the call through a second, possibly
/// different-revision fleet, which among other failures rejects
/// this fleet's own generated closure arguments (#14650). An
/// explicit FLEET_GH_BIN is untouched: the guard never changes
/// which binary was chosen, it only tells the shim not to route.
const SHIM_GUARD_ENV: &str = "FLEET_GH_SHIM_ACTIVE";

/// The index and word of the `gh` command in `args`, past any
/// inherited flags given before it (the scan `command_word` does,
/// keeping the position so the subcommand after it can be read).
fn command_word_at(args: &[String]) -> Option<(usize, &str)> {
    let mut i = 0;
    while let Some(a) = args.get(i) {
        let a = a.as_str();
        if a == "-R" || a == "--repo" {
            i += 2;
        } else if a.starts_with('-') {
            i += 1;
        } else {
            return Some((i, a));
        }
    }
    None
}

/// Is this a `secret set` / `variable set` call — the two commands
/// whose value can come from `--body`, an `--env-file`, or stdin
/// (`gh secret set --help`, `gh variable set --help`)?
fn is_value_set(args: &[String]) -> bool {
    let Some((i, word)) = command_word_at(args) else {
        return false;
    };
    (word == "secret" || word == "variable") && args.get(i + 1).map(|s| s.as_str()) == Some("set")
}

/// The `--body` value of a value-set call, when a body flag is
/// present in any spelling (`--body V`, `--body=V`, `-b V`, `-bV`).
fn body_value(args: &[String]) -> Option<Option<&str>> {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--body" || a == "-b" {
            return Some(args.get(i + 1).map(|s| s.as_str()));
        }
        if let Some(v) = a.strip_prefix("--body=") {
            return Some(Some(v));
        }
        if a.starts_with("-b") && a.len() > 2 {
            return Some(Some(&a[2..]));
        }
        i += 1;
    }
    None
}

/// The `--env-file` target of a value-set call, when present in any
/// spelling (`--env-file F`, `--env-file=F`, `-f F`, `-fF`); `-`
/// means gh reads the file from stdin.
fn env_file_value(args: &[String]) -> Option<&str> {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--env-file" || a == "-f" {
            return args.get(i + 1).map(|s| s.as_str());
        }
        if let Some(v) = a.strip_prefix("--env-file=") {
            return Some(v);
        }
        if a.starts_with("-f") && a.len() > 2 {
            return Some(&a[2..]);
        }
        i += 1;
    }
    None
}

/// Does stdin supply this call's value? True for a value-set call
/// with no `--body` whose `--env-file` is absent or `-`: real gh
/// then reads the value from stdin (`gh secret set --help`: the
/// body flag "reads from standard input if not specified").
/// Oracle: those --help texts (#15556).
fn stdin_supplies_value(args: &[String]) -> bool {
    is_value_set(args)
        && body_value(args).is_none()
        && matches!(env_file_value(args), None | Some("-"))
}

/// The loud refusal for a value-set call that would store an empty
/// value, or None when the call carries a non-empty value.
/// `piped_stdin` is the stdin bytes fleet buffered for replay, or
/// None when stdin is a terminal (gh prompts interactively; the
/// prompt is the caller's own choice and is not second-guessed).
/// An empty `--body` is refused regardless of stdin. Oracle:
/// #15556 — real gh stores an empty stdin read / empty body as an
/// empty secret and exits 0, so the failure is silent downstream.
fn empty_value_error(args: &[String], piped_stdin: Option<&[u8]>) -> Option<String> {
    if !is_value_set(args) {
        return None;
    }
    if let Some(body) = body_value(args) {
        return match body {
            Some(v) if !v.is_empty() => None,
            _ => Some(
                "secret/variable set with an empty --body would store an empty value; \
                 refusing (pass a non-empty --body, or pipe the value on stdin)"
                    .to_string(),
            ),
        };
    }
    if !stdin_supplies_value(args) {
        return None;
    }
    match piped_stdin {
        Some(bytes) if bytes.iter().all(|b| b.is_ascii_whitespace()) => Some(
            "secret/variable set reads its value from stdin, but stdin is empty; \
             refusing to store an empty value (pipe the value, or pass --body)"
                .to_string(),
        ),
        _ => None,
    }
}

/// Run the real `gh` once, under `token`, capturing output for
/// classification. stdin is inherited so prompts behave; stdout and
/// stderr are piped (see the module header for why). The child
/// carries the shim recursion guard (SHIM_GUARD_ENV).
fn run_gh_once(gh_bin: &str, args: &[String], token: &str) -> std::io::Result<Attempt> {
    run_gh_once_with_stdin(gh_bin, args, token, None)
}

/// `run_gh_once` with stdin bytes to replay (#15556): when
/// `stdin_bytes` is Some, the child reads exactly those bytes from
/// a pipe — the same bytes every attempt gets, so a failover after
/// a stdin-consuming first attempt cannot observe EOF and store
/// an empty value. The write runs on its own thread so a child
/// filling its output pipes cannot deadlock the feed.
fn run_gh_once_with_stdin(
    gh_bin: &str,
    args: &[String],
    token: &str,
    stdin_bytes: Option<&[u8]>,
) -> std::io::Result<Attempt> {
    let Some(bytes) = stdin_bytes else {
        let out = Command::new(gh_bin)
            .args(args)
            .env("GH_TOKEN", token)
            .env(SHIM_GUARD_ENV, "1")
            .stdin(Stdio::inherit())
            .output()?;
        return Ok(Attempt {
            code: out.status.code().unwrap_or(1),
            stdout: out.stdout,
            stderr: out.stderr,
        });
    };
    let mut child = Command::new(gh_bin)
        .args(args)
        .env("GH_TOKEN", token)
        .env(SHIM_GUARD_ENV, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut sin) = child.stdin.take() {
        let bytes = bytes.to_vec();
        std::thread::spawn(move || {
            let _ = sin.write_all(&bytes);
        });
    }
    let out = child.wait_with_output()?;
    Ok(Attempt {
        code: out.status.code().unwrap_or(1),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// The rotation loop, generic over the token provider so tests can
/// drive it with fakes. Writes the final attempt's bytes out and
/// returns its exit code; failover notes go to stderr (source
/// labels only) whenever an attempt is superseded.
fn run_with_provider<P: TokenProvider>(
    provider: &mut P,
    gh_bin: &str,
    args: &[String],
    sources: &[TokenSource],
    trace: bool,
) -> i32 {
    run_with_provider_capturing(provider, gh_bin, args, sources, trace, false, None).0
}

/// run_with_provider, additionally handing back the final attempt's
/// stdout when `capture_stdout` is set (the Metadata create path
/// reads the created item's URL from it, #9073). The capture is a
/// clone taken only on the path that asked for it, so the hot
/// passthrough pays nothing.
fn run_with_provider_capturing<P: TokenProvider>(
    provider: &mut P,
    gh_bin: &str,
    args: &[String],
    sources: &[TokenSource],
    trace: bool,
    capture_stdout: bool,
    stdin_bytes: Option<&[u8]>,
) -> (i32, Option<Vec<u8>>) {
    let read_only = is_read_call(args);
    let mut last: Option<Attempt> = None;
    let mut provider_errors: Vec<String> = Vec::new();
    for (idx, source) in sources.iter().enumerate() {
        let token = match provider.token_for(source) {
            Ok(t) => t,
            Err(e) => {
                if trace {
                    eprintln!("fleet gh: skipping {} ({e})", source.label());
                }
                provider_errors.push(e);
                continue;
            }
        };
        if trace {
            eprintln!("fleet gh: attempt {} under {}", idx + 1, source.label());
        }
        let attempt = match run_gh_once_with_stdin(gh_bin, args, &token, stdin_bytes) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("fleet gh: {}", spawn_failure_message(gh_bin, &e));
                return (EXIT_UNRESOLVED, None);
            }
        };
        if attempt.code == 0 {
            let captured = capture_stdout.then(|| attempt.stdout.clone());
            emit(&attempt);
            return (EXIT_OK, captured);
        }
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&attempt.stdout),
            String::from_utf8_lossy(&attempt.stderr)
        );
        let class = classify_failure(&combined);
        let attempts_left = idx + 1 < sources.len();
        if may_retry(class, read_only, attempts_left) {
            if trace {
                eprintln!(
                    "fleet gh: {} failed ({}); failing over",
                    source.label(),
                    match class {
                        FailureClass::RateLimit => "rate limit",
                        FailureClass::Permission => "permission/scope refusal",
                        FailureClass::NodeResolution => "node-resolution error on a read-only call",
                        FailureClass::Other => "error on a read-only call",
                    }
                );
            }
            last = Some(attempt);
            continue;
        }
        let captured = capture_stdout.then(|| attempt.stdout.clone());
        emit(&attempt);
        return (attempt.code, captured);
    }
    // No attempt produced output: either every source failed to
    // resolve, or the last failover had no successor. Prefer showing
    // the caller the real gh failure over our own summary.
    if let Some(attempt) = last {
        let code = attempt.code;
        let captured = capture_stdout.then(|| attempt.stdout.clone());
        emit(&attempt);
        return (code, captured);
    }
    for e in distinct_errors(&provider_errors) {
        eprintln!("fleet gh: {e}");
    }
    eprintln!("fleet gh: no token source could be resolved");
    (EXIT_UNRESOLVED, None)
}

/// Write an attempt's captured bytes to the real stdout/stderr,
/// verbatim. Broken pipes are quiet (#5569's rule).
fn emit(attempt: &Attempt) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(&attempt.stdout);
    let _ = out.flush();
    let mut err = std::io::stderr().lock();
    let _ = err.write_all(&attempt.stderr);
    let _ = err.flush();
}

const TIERS_USAGE: &str = "\
Usage: fleet gh tiers [--all]

Probe every token-rotation source (built-in and configured GitHub Apps'
installation tokens, then the personal token) and print each one's
rate-limit headroom.

  --all   List every installation of every configured App instead of
          the configured rotation, so accounts outside the configured
          owner lists get a row too.";

const CLOSURES_USAGE: &str = "\
Usage: fleet gh closures [<item>]

Print the close-guard closure records, one JSON object per line,
optionally only those for one item number. Takes no other
arguments.";

/// `fleet gh tiers`: probe every rotation source and print its
/// core and GraphQL rate-limit headroom. Labels and numbers only —
/// token values never leave the provider. Exit 0 when every source
/// answers, EXIT_UNRESOLVED when any source is down.
///
/// With `all`, the source set is every installation of every App
/// (built-in and configured, resolved live and paginated) plus the
/// personal token, instead of the configured rotation — so owners
/// outside the configured owner lists still get a row (#10311 (c)).
fn run_tiers(config: &Config, addrs: Vec<String>, all: bool) -> i32 {
    let gh_bin = std::env::var("FLEET_GH_BIN").unwrap_or_else(|_| "gh".to_string());
    let mut provider = FleetTokenProvider::new(config, addrs);
    let mut listed_down = false;
    let sources = if all {
        tiers_all_sources(config, &mut provider, &mut listed_down)
    } else {
        rotation(&[], config)
    };
    let mut any_down = listed_down;
    for source in &sources {
        let label = source.label();
        let token = match provider.token_for(source) {
            Ok(t) => t,
            Err(e) => {
                println!("{label}\tDOWN\t{e}");
                any_down = true;
                continue;
            }
        };
        let args: Vec<String> = vec!["api".to_string(), "rate_limit".to_string()];
        match run_gh_once(&gh_bin, &args, &token) {
            Ok(attempt) if attempt.code == 0 => {
                let body: serde_json::Value =
                    serde_json::from_slice(&attempt.stdout).unwrap_or_default();
                let show = |name: &str| {
                    let r = &body["resources"][name];
                    format!(
                        "{}/{}",
                        r["remaining"].as_i64().unwrap_or(-1),
                        r["limit"].as_i64().unwrap_or(-1)
                    )
                };
                println!(
                    "{label}\tcore {}\tgraphql {}",
                    show("core"),
                    show("graphql")
                );
            }
            Ok(attempt) => {
                println!("{label}\tDOWN\tgh api rate_limit exited {}", attempt.code);
                any_down = true;
            }
            Err(e) => {
                println!("{label}\tDOWN\t{}", spawn_failure_message(&gh_bin, &e));
                any_down = true;
            }
        }
    }
    if any_down { EXIT_UNRESOLVED } else { EXIT_OK }
}

/// Every App in tiers order: the built-ins, then the configured
/// Apps in file order (the same order the rotation uses).
fn all_apps(config: &Config) -> Vec<GhApp> {
    GhApp::ALL
        .into_iter()
        .chain(
            config
                .gh
                .apps
                .iter()
                .map(|app| GhApp::Configured(app.slug.clone())),
        )
        .collect()
}

/// The `--all` source set: every installation of every App (live,
/// paginated, sorted per App for stable output), then the personal
/// token. A listing failure prints its DOWN row immediately and
/// sets `listed_down`, so the probe loop below only handles token
/// and `gh` failures.
fn tiers_all_sources(
    config: &Config,
    provider: &FleetTokenProvider,
    listed_down: &mut bool,
) -> Vec<TokenSource> {
    let mut lists: Vec<(GhApp, Vec<String>)> = Vec::new();
    for app in all_apps(config) {
        match provider.installations(&app) {
            Ok(pairs) => {
                lists.push((app, pairs.into_iter().map(|(login, _)| login).collect()));
            }
            Err(e) => {
                println!("{}\tDOWN\t{e}", app.slug());
                *listed_down = true;
            }
        }
    }
    tiers_all_from_lists(&lists)
}

/// Pure `--all` builder, so the ordering contract is testable
/// without network: per-App owners sorted for stable output,
/// exact duplicates dropped, personal last.
fn tiers_all_from_lists(entries: &[(GhApp, Vec<String>)]) -> Vec<TokenSource> {
    let mut out = Vec::new();
    for (app, owners) in entries {
        let mut sorted = owners.clone();
        sorted.sort();
        sorted.dedup();
        out.extend(
            sorted
                .into_iter()
                .map(|owner| TokenSource::AppInstallation {
                    app: app.clone(),
                    owner,
                }),
        );
    }
    out.push(TokenSource::Personal);
    out
}

/// Raw-mode split: returns the arguments to hand the real `gh`
/// when raw mode is requested — a leading `--raw` (stripped) or
/// FLEET_GH_RAW=1 (args unchanged) — and None when the call goes
/// through the rotation. Only a LEADING `--raw` is ours: later
/// positions belong to `gh` (its own `--raw-field` family lives
/// further along the command line, never first).
fn raw_args(args: &[String], raw_env: bool) -> Option<Vec<String>> {
    if args.first().is_some_and(|a| a == "--raw") {
        return Some(args[1..].to_vec());
    }
    if raw_env {
        return Some(args.to_vec());
    }
    None
}

/// Raw mode: exec the real `gh` with the caller's ambient
/// environment untouched — no token resolved, none injected, no
/// failover. Stdio is inherited (not captured) so interactive
/// `gh` behaves exactly as it does unwrapped; the exit code is
/// the child's own. The one deliberate exception to "untouched"
/// is the shim recursion guard (SHIM_GUARD_ENV): without it a
/// PATH `gh` that is the gh-fleet shim re-enters fleet instead
/// of reaching the pristine gh raw mode promises (#14650).
fn run_raw(gh_bin: &str, args: &[String]) -> i32 {
    match Command::new(gh_bin)
        .args(args)
        .env(SHIM_GUARD_ENV, "1")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("fleet gh --raw: {}", spawn_failure_message(gh_bin, &e));
            EXIT_UNRESOLVED
        }
    }
}

fn closure_binding(
    initialize: impl FnOnce() -> Result<Option<String>, String>,
) -> gh_closure::Binding {
    let resolved_session_id = match initialize() {
        Ok(id) => id,
        Err(error) => {
            eprintln!("fleet gh: {error}; closure record will omit the session id");
            return gh_closure::Binding::from_failed_session_resolution();
        }
    };
    gh_closure::Binding::from_resolved_session_id(resolved_session_id)
}

pub fn run(config: &Config, args: &[String]) -> i32 {
    // Cache flags come first: leading `--cache-addr` (repeatable)
    // and `--no-cache` are fleet's own, stripped before anything
    // else sees the arguments. Only LEADING flags are ours, like
    // `--raw`: anything later belongs to `gh`.
    let flags = match split_cache_flags(args) {
        Ok(f) => f,
        Err(e) => return usage_error(&e),
    };
    let args = &flags.rest;
    // Raw mode comes first: `--raw` with no further args is the
    // real gh with no args, not this domain's usage text.
    let raw_env = std::env::var("FLEET_GH_RAW").is_ok_and(|v| v == "1");
    if let Some(raw) = raw_args(args, raw_env) {
        let gh_bin = std::env::var("FLEET_GH_BIN").unwrap_or_else(|_| "gh".to_string());
        return run_raw(&gh_bin, &raw);
    }
    if args.is_empty()
        || args.first().is_some_and(|a| a == "--help" || a == "-h") && args.len() == 1
    {
        println!("{USAGE}");
        return EXIT_OK;
    }
    // `tiers`, `cache-serve` and `cache-status` are shadow
    // subcommands: real `gh` has no such verbs, so intercepting
    // them cannot shadow a passthrough call.
    if args.first().is_some_and(|a| a == "tiers") {
        if args.len() == 2 && matches!(args[1].as_str(), "--help" | "-h") {
            println!("{TIERS_USAGE}");
            return EXIT_OK;
        }
        if args.len() == 2 && args[1] == "--all" {
            return run_tiers(config, cache_addrs(config, &flags), true);
        }
        if args.len() > 1 {
            return usage_error(&format!(
                "unexpected argument '{}' for 'fleet gh tiers'",
                args[1]
            ));
        }
        return run_tiers(config, cache_addrs(config, &flags), false);
    }
    if args.first().is_some_and(|a| a == "cache-serve") {
        return run_cache_serve(config, &args[1..]);
    }
    if args.first().is_some_and(|a| a == "cache-status") {
        return run_cache_status(&args[1..], &cache_addrs(config, &flags));
    }
    if args.first().is_some_and(|a| a == "closures") {
        return run_closures(&args[1..]);
    }
    // Sync verbs (#10973, step 2.2 of #9016): strictly-parsed typed
    // verbs that rewrite to the canonical gh call. The rewrite is
    // applied here, before the close guard, the closure layer and
    // the metadata create path parse the arguments, so a sync call
    // inherits every passthrough control — and the rotation below —
    // exactly as the hand-written call it replaces would have.
    let sync_rewrite;
    let args: &[String] = match gh_sync::parse(args) {
        Ok(Some(canonical)) => {
            sync_rewrite = canonical;
            &sync_rewrite
        }
        Ok(None) => args,
        Err(e) => {
            eprintln!("{e}");
            return EXIT_USAGE;
        }
    };
    let guard_intent = match gh_guard::parse(args) {
        Ok(intent) => intent,
        Err(e) => {
            eprintln!("{e}");
            return EXIT_USAGE;
        }
    };
    if let Some(intent) = &guard_intent {
        if let Err(e) = gh_guard::check(intent) {
            eprintln!("fleet gh close guard: {e}");
            return EXIT_USAGE;
        }
    }
    let parsed_args = guard_intent
        .as_ref()
        .map(|intent| intent.gh_args.as_slice())
        .unwrap_or(args);
    let intent = match gh_closure::parse(parsed_args) {
        Ok(intent) => intent,
        Err(e) => {
            eprintln!("{e}");
            return EXIT_USAGE;
        }
    };
    let call_args = match &intent {
        Some(i) => i.gh_args_with_reason(),
        None => parsed_args.to_vec(),
    };
    // Metadata create path (#9073): stamp an issue/pr create's body
    // before the rotation runs and remember the stamped block for
    // the post-create record insert. A refusal (a malformed caller
    // Metadata block, an unreadable --body-file) happens before any
    // GitHub write and never reaches the rotation.
    let prepared_create =
        match gh_metadata::prepare_create(config, &call_args, std::io::stdin().is_terminal()) {
            Ok(prepared) => prepared,
            Err(e) => {
                eprintln!("fleet gh: {e}");
                return EXIT_USAGE;
            }
        };
    let call_args = match &prepared_create {
        Some(prepared) => prepared.args.clone(),
        None => call_args,
    };
    // Content-creation ledger (code#2227): a classified create
    // reserves its shared-ledger row BEFORE any GitHub write. The
    // ledger owns the failure mode — a reservation failure refuses
    // the create (exit 5), never an unledgered filing.
    let ledger_pending = match gh_content_ledger::start(&call_args) {
        Ok(pending) => pending,
        Err(e) => {
            eprintln!(
                "fleet gh: content-ledger reserve failed ({e}); \
                 refusing to post unledgered (code#2227)"
            );
            return EXIT_LEDGER;
        }
    };
    // Stdin values (#15556): a value-set call taking its value from
    // piped stdin gets those bytes buffered once, here — an empty
    // value is refused before any attempt, and the buffered bytes
    // are replayed to every rotation attempt, so a failover cannot
    // turn the value into an empty store. A terminal stdin stays
    // inherited (gh prompts), and every other call is untouched.
    let stdin_replay: Option<Vec<u8>> =
        if stdin_supplies_value(&call_args) && !std::io::stdin().is_terminal() {
            let mut buf = Vec::new();
            match std::io::stdin().read_to_end(&mut buf) {
                Ok(_) => Some(buf),
                Err(e) => {
                    eprintln!("fleet gh: reading the value from stdin failed: {e}");
                    return EXIT_UNRESOLVED;
                }
            }
        } else {
            None
        };
    if let Some(message) = empty_value_error(&call_args, stdin_replay.as_deref()) {
        eprintln!("fleet gh: {message}");
        return EXIT_USAGE;
    }
    let gh_bin = std::env::var("FLEET_GH_BIN").unwrap_or_else(|_| "gh".to_string());
    let sources = rotation(&call_args, config);
    let trace = std::env::var("FLEET_GH_TRACE").is_ok_and(|v| v == "1");
    let mut provider = FleetTokenProvider::new(config, cache_addrs(config, &flags));
    let (code, final_stdout) = run_with_provider_capturing(
        &mut provider,
        &gh_bin,
        &call_args,
        &sources,
        trace,
        // The ledger's verify step reads the created item's URL
        // from stdout (code#2227); capture changes nothing
        // user-visible — emit() still streams every byte.
        prepared_create.is_some() || ledger_pending.is_some(),
        stdin_replay.as_deref(),
    );
    // Content-creation ledger (code#2227): finalise the reservation
    // with what actually happened, on every path below. The row
    // records the truth; gh's exit code stands either way (the
    // metadata create path's posture).
    if let Some(pending) = &ledger_pending {
        match gh_content_ledger::finish(pending, code, final_stdout.as_deref()) {
            Ok(Some(notice)) => eprintln!("fleet gh: {notice}"),
            Ok(None) => {}
            Err(e) => eprintln!(
                "fleet gh: content-ledger finalize failed ({e}); \
                 the create stands, the ledger is short one outcome"
            ),
        }
    }
    // Durable audit trail for App-token writes (#9177 control 3):
    // record the caller/session identity, the call, and its outcome
    // so an App-identity write is attributable to a component. Gated
    // on FLEET_GH_TRACE=1 or FLEET_GH_AUDIT=1; default off to keep
    // the hot path quiet, but any fleet job that wants attribution
    // sets one of them (and FLEET_GH_CALLER for its identity).
    let audit_enabled = std::env::var("FLEET_GH_TRACE").is_ok_and(|v| v == "1")
        || std::env::var("FLEET_GH_AUDIT").is_ok_and(|v| v == "1");
    if audit_enabled && !is_read_call(&call_args) {
        let caller = std::env::var("FLEET_GH_CALLER").unwrap_or_else(|_| "unknown".to_string());
        let path = std::env::var("FLEET_GH_AUDIT_FILE").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            format!("{home}/.fleet/gh-audit.jsonl")
        });
        let entry = serde_json::json!({
            "ts": gh_closure::now_unix(),
            "caller": caller,
            "read_only": false,
            "args": audit_args(&call_args),
            "exit_code": code,
        });
        // The call already ran; a lost record must not change its
        // outcome or repeat it, so report the gap and carry on.
        if let Err(e) = append_audit_record(std::path::Path::new(&path), &entry) {
            eprintln!(
                "fleet gh: the call finished (exit {code}), but its audit record was not written to {path}: {e}"
            );
        }
    }
    if let Some(i) = intent.filter(|_| code == EXIT_OK) {
        // Only a successfully parsed and completed Fleet-owned closure
        // operation consumes recovered worktree identity. This is deliberately
        // not a global startup hook or a scan of passthrough `gh` arguments.
        let binding = closure_binding(|| crate::session_state::initialize(None));
        let record = i.record(&binding, gh_closure::now_unix());
        if let Err(e) = gh_closure::append(&gh_closure::log_path(), &record) {
            eprintln!(
                "fleet gh: {} done, but the closure record was not written: {e}",
                i.action
            );
            return EXIT_UNRESOLVED;
        }
    }
    // Metadata create path (#9073): the create succeeded, so insert
    // the full record into the metadata store. The item already
    // exists and gh's exit code stands either way — a failed insert
    // is reported on stderr and ingestion (§16) heals the miss.
    if let Some(prepared) = &prepared_create {
        if code == EXIT_OK {
            let store = gh_metadata::SqliteMetadataStore::new(
                gh_metadata::SqliteMetadataStore::default_path(),
            );
            let stdout = final_stdout
                .as_deref()
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                .unwrap_or_default();
            match gh_metadata::finish_create(&store, &stdout, &prepared.block) {
                gh_metadata::CreateOutcome::Inserted(_) => {}
                gh_metadata::CreateOutcome::InsertFailed { item, error } => {
                    eprintln!(
                        "fleet gh: created {}, but its Metadata record was not \
                         inserted ({error}); the ingestion sweeper will heal the miss",
                        item.url
                    );
                }
                gh_metadata::CreateOutcome::NoUrl => {
                    eprintln!(
                        "fleet gh: the create succeeded but gh's output carried \
                         no item URL; its Metadata record was not inserted; \
                         the ingestion sweeper will heal the miss"
                    );
                }
            }
        }
    }
    code
}

/// `fleet gh closures [<item>]`: print closure records, one JSON
/// object per line, optionally only those for one item number.
fn run_closures(tail: &[String]) -> i32 {
    if tail.len() == 1 && matches!(tail[0].as_str(), "--help" | "-h") {
        println!("{CLOSURES_USAGE}");
        return EXIT_OK;
    }
    if tail.len() > 1 {
        return usage_error(&format!(
            "unexpected argument '{}' for 'fleet gh closures'",
            tail[1]
        ));
    }
    let log = match std::fs::read_to_string(gh_closure::log_path()) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            eprintln!("fleet gh closures: cannot read the closure log: {e}");
            return EXIT_UNRESOLVED;
        }
    };
    let item = tail.first().map(|s| s.trim_start_matches('#'));
    for r in gh_closure::query(&log, item) {
        println!("{r}");
    }
    EXIT_OK
}

/// Leading cache flags (code#10742): `--cache-addr <addr>`
/// (repeatable, also `--cache-addr=<addr>`) and `--no-cache`.
/// Parsing stops at the first non-flag, which with everything
/// after it belongs to `gh` (the `--raw` rule).
struct CacheFlags {
    addrs: Vec<String>,
    no_cache: bool,
    rest: Vec<String>,
}

fn split_cache_flags(args: &[String]) -> Result<CacheFlags, String> {
    let mut addrs = Vec::new();
    let mut no_cache = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--no-cache" => {
                no_cache = true;
                i += 1;
            }
            "--cache-addr" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "--cache-addr needs an address".to_string())?;
                if value.trim().is_empty() {
                    return Err("--cache-addr must not be empty".to_string());
                }
                addrs.push(value.clone());
                i += 1;
            }
            arg if arg.starts_with("--cache-addr=") => {
                let value = arg["--cache-addr=".len()..].to_string();
                if value.trim().is_empty() {
                    return Err("--cache-addr must not be empty".to_string());
                }
                addrs.push(value);
                i += 1;
            }
            _ => break,
        }
    }
    Ok(CacheFlags {
        addrs,
        no_cache,
        rest: args[i..].to_vec(),
    })
}

/// Daemon addresses for one call (code#10742 precedence):
/// `--cache-addr` flag beats `FLEET_GH_CACHE` beats config beats
/// defaults (the env/config/default legs live in
/// [`Config::gh_cache_addrs`]). `--no-cache` empties the route:
/// in-process resolution only.
fn cache_addrs(config: &Config, flags: &CacheFlags) -> Vec<String> {
    if flags.no_cache {
        return Vec::new();
    }
    if !flags.addrs.is_empty() {
        return flags
            .addrs
            .iter()
            .map(|a| Config::with_default_port(a))
            .collect();
    }
    config.gh_cache_addrs()
}

const CACHE_SERVE_USAGE: &str = "\
Usage: fleet gh cache-serve [--bind <addr>]

Run the gh credential/token cache daemon in the foreground (code#10742):
secret resolutions, App JWTs, installation tokens and per-App
installation lists, memory-only with single-flight minting. Nothing is
written to disk; a restart is a cold cache, never a stale one.

The bind address defaults to 127.0.0.1:4802 (FLEET_GH_CACHE_BIND beats
the default; --bind beats both; a bind without a port gets 4802).";

/// `fleet gh cache-serve`: the cache daemon role. The bind leaf
/// takes only `--bind` (or help); anything else is a usage error,
/// never something silently dropped before the socket is bound.
fn run_cache_serve(config: &Config, tail: &[String]) -> i32 {
    let mut bind: Option<String> = None;
    let mut i = 0;
    while i < tail.len() {
        match tail[i].as_str() {
            "--help" | "-h" => {
                println!("{CACHE_SERVE_USAGE}");
                return EXIT_OK;
            }
            "--bind" => {
                i += 1;
                match tail.get(i) {
                    Some(v) if !v.trim().is_empty() => bind = Some(v.clone()),
                    _ => {
                        return usage_error(&format!(
                            "'--bind' needs an address\n\n{CACHE_SERVE_USAGE}"
                        ));
                    }
                }
            }
            arg if arg.starts_with("--bind=") => {
                let value = arg["--bind=".len()..].to_string();
                if value.trim().is_empty() {
                    return usage_error(&format!(
                        "'--bind' must not be empty\n\n{CACHE_SERVE_USAGE}"
                    ));
                }
                bind = Some(value);
            }
            other => {
                return usage_error(&format!(
                    "unexpected argument '{other}' for 'fleet gh cache-serve'\n\n{CACHE_SERVE_USAGE}"
                ));
            }
        }
        i += 1;
    }
    let raw = bind
        .or_else(|| {
            std::env::var("FLEET_GH_CACHE_BIND")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
        .unwrap_or_else(|| format!("127.0.0.1:{DEFAULT_GH_CACHE_PORT}"));
    let bind = Config::with_default_port(&raw);
    eprintln!("fleet gh cache-serve: listening on {bind}");
    gh_cache::serve(RawMinter::new(config), &bind)
}

const CACHE_STATUS_USAGE: &str = "\
Usage: fleet gh cache-status

Probe the configured cache daemons (code#10742) and print each one's
snapshot: entry counts by kind, hit/miss counters, and the soonest
expiry. Key names and values never leave the daemon — counts only.
Exits 3 when no daemon answered.";

fn run_cache_status(tail: &[String], addrs: &[String]) -> i32 {
    if tail.first().is_some_and(|a| a == "--help" || a == "-h") {
        println!("{CACHE_STATUS_USAGE}");
        return EXIT_OK;
    }
    if let Some(extra) = tail.first() {
        return usage_error(&format!(
            "unexpected argument '{extra}' for 'fleet gh cache-status'\n\n{CACHE_STATUS_USAGE}"
        ));
    }
    let req = gh_cache::request("status", None, None, None);
    let mut any_up = false;
    for addr in addrs {
        match gh_cache::fetch_one(addr, &req, gh_cache::CONNECT_TIMEOUT) {
            Ok(resp) => {
                if resp.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                    any_up = true;
                    println!("{addr}\tUP\t{}", summarize_status(&resp));
                } else {
                    let e = resp
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("reported failure");
                    println!("{addr}\tDOWN\t{e}");
                }
            }
            Err(e) => println!("{addr}\tDOWN\t{e}"),
        }
    }
    if addrs.is_empty() {
        println!("no cache daemon configured (--no-cache); nothing to probe");
    }
    if any_up { EXIT_OK } else { EXIT_UNRESOLVED }
}

/// One daemon's status object as a single tab-separated line:
/// counts by kind, hits/misses, soonest expiry — nothing keyed by
/// name, per the kind-only status rule.
fn summarize_status(resp: &serde_json::Value) -> String {
    let status = &resp["status"];
    let kinds = status
        .get("kinds")
        .and_then(|k| k.as_object())
        .map(|m| {
            let mut rows: Vec<String> = m.iter().map(|(k, v)| format!("{k}={v}")).collect();
            rows.sort();
            rows.join(" ")
        })
        .unwrap_or_default();
    format!(
        "entries={} kinds:[{kinds}] hits={} misses={} min_ttl={}s uptime={}s",
        status
            .get("live_entries")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        status.get("hits").and_then(|v| v.as_u64()).unwrap_or(0),
        status.get("misses").and_then(|v| v.as_u64()).unwrap_or(0),
        status
            .get("min_ttl_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        status
            .get("uptime_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
    )
}

// ---------------------------------------------------------------------------
// Tests. Every test names its oracle (fleet testing doctrine,
// code 2026-11-04): the fixed external truth the assertion is
// checked against.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[test]
    fn successful_closure_binding_consumes_resolved_worktree_session() {
        let mut initialized = false;
        let binding = closure_binding(|| {
            initialized = true;
            Ok(Some("ses_from_worktree".to_owned()))
        });
        assert!(
            initialized,
            "closure binding initializes session resolution"
        );
        assert_eq!(
            binding.session_id.as_deref(),
            Some("ses_from_worktree"),
            "the resolved worktree identity reaches the closure binding"
        );
    }

    #[test]
    fn clustered_short_options_are_never_reads() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // -i then -t (which takes "graphql"): the endpoint is the issues URL.
        assert!(!is_read_call(&v(&[
            "api",
            "-it",
            "graphql",
            "repos/o/r/issues",
            "-f",
            "title=x",
            "-f",
            "query={viewer{login}}"
        ])));
        assert!(!is_read_call(&v(&["api", "-it", "x", "repos/o/r/issues"])));
        // An attached value is one option, not a cluster.
        assert!(is_read_call(&v(&["api", "repos/o/r/issues", "-q.[]"])));
        assert!(is_read_call(&v(&["api", "repos/o/r/issues", "-tx", "-i"])));
        assert!(is_short_option_cluster("-it"));
        assert!(!is_short_option_cluster("-i"));
        assert!(!is_short_option_cluster("-tfoo"));
        assert!(!is_short_option_cluster("--paginate"));
    }

    use super::*;

    // oracle: GitHub's refusal strings as observed in fleet logs
    // (code#1408's "Resource not accessible by personal access
    // token", code#6129's rate-limit ladder, gh's INSUFFICIENT_SCOPES
    // GraphQL error) — classification must catch each shape.
    #[test]
    fn classify_failure_recognizes_refusals() {
        assert_eq!(
            classify_failure("GraphQL: API rate limit exceeded for user ID 1."),
            FailureClass::RateLimit
        );
        assert_eq!(
            classify_failure("You have exceeded a secondary rate limit."),
            FailureClass::RateLimit
        );
        assert_eq!(
            classify_failure("Resource not accessible by personal access token"),
            FailureClass::Permission
        );
        assert_eq!(
            classify_failure("INSUFFICIENT_SCOPES: the token needs the 'project' scope"),
            FailureClass::Permission
        );
        assert_eq!(
            classify_failure("Resource not accessible by integration"),
            FailureClass::Permission
        );
        assert_eq!(
            classify_failure(
                "GraphQL: Could not resolve to a node with the global id of 'PVT_kwHOA123'"
            ),
            FailureClass::NodeResolution
        );
        assert_eq!(
            classify_failure("gh: Not Found (HTTP 404)"),
            FailureClass::Other
        );
        assert_eq!(
            classify_failure("error connecting to api.github.com"),
            FailureClass::Other
        );
    }

    // oracle: the #8887 retry-safety rule — writes repeat only on
    // pre-execution refusals; reads repeat on anything while a
    // token remains.
    #[test]
    fn retry_gate_matches_the_rule() {
        assert!(may_retry(FailureClass::RateLimit, false, true));
        assert!(may_retry(FailureClass::Permission, false, true));
        assert!(!may_retry(FailureClass::NodeResolution, false, true));
        assert!(may_retry(FailureClass::NodeResolution, true, true));
        assert!(!may_retry(FailureClass::NodeResolution, true, false));
        assert!(!may_retry(FailureClass::Other, false, true));
        assert!(may_retry(FailureClass::Other, true, true));
        assert!(!may_retry(FailureClass::RateLimit, false, false));
        assert!(!may_retry(FailureClass::Other, true, false));
    }

    // oracle: `gh <verb> --help` — list/view/status surfaces mutate
    // nothing; create/comment/edit surfaces mutate; `api` mutates
    // only with field flags or a non-GET method.
    #[test]
    fn audit_args_redact_secret_values() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            audit_args(&v(&[
                "secret", "set", "TOKEN", "--body", "hunter2", "--repo", "o/r"
            ])),
            v(&[
                "secret",
                "set",
                "TOKEN",
                "--body",
                "[redacted]",
                "--repo",
                "o/r"
            ])
        );
        assert_eq!(
            audit_args(&v(&["secret", "set", "TOKEN", "-bhunter2"])),
            v(&["secret", "set", "TOKEN", "-b[redacted]"])
        );
        assert_eq!(
            audit_args(&v(&["secret", "set", "TOKEN", "--body=hunter2"])),
            v(&["secret", "set", "TOKEN", "--body=[redacted]"])
        );
        // Inherited flags before the command word.
        assert_eq!(
            audit_args(&v(&[
                "-R", "o/r", "secret", "set", "TOKEN", "--body", "hunter2"
            ])),
            v(&[
                "-R",
                "o/r",
                "secret",
                "set",
                "TOKEN",
                "--body",
                "[redacted]"
            ])
        );
        assert_eq!(
            audit_args(&v(&["--repo=o/r", "secret", "set", "TOKEN", "-bhunter2"])),
            v(&["--repo=o/r", "secret", "set", "TOKEN", "-b[redacted]"])
        );
        assert_eq!(
            audit_args(&v(&[
                "api",
                "user",
                "-X",
                "PATCH",
                "-H",
                "authorization: token abc"
            ])),
            v(&[
                "api",
                "user",
                "-X",
                "PATCH",
                "-H",
                "authorization: [redacted]"
            ])
        );
        assert_eq!(
            audit_args(&v(&["api", "user", "--header=Authorization: bearer abc"])),
            v(&["api", "user", "--header=Authorization: [redacted]"])
        );
        assert_eq!(
            audit_args(&v(&["api", "user", "-H=Authorization: bearer abc"])),
            v(&["api", "user", "-H=Authorization: [redacted]"])
        );
        assert_eq!(
            audit_args(&v(&[
                "api",
                "user",
                "-X",
                "POST",
                "-H",
                "Cookie: session=SECRET"
            ])),
            v(&["api", "user", "-X", "POST", "-H", "Cookie: [redacted]"])
        );
        assert_eq!(
            audit_args(&v(&[
                "api",
                "user",
                "-HX-Api-Key: k1",
                "--header",
                "Accept: json"
            ])),
            v(&[
                "api",
                "user",
                "-HX-Api-Key: [redacted]",
                "--header",
                "Accept: json"
            ])
        );
        // Ordinary bodies and headers stay readable.
        let plain = v(&[
            "issue",
            "comment",
            "1",
            "--body",
            "hello",
            "-H",
            "Accept: x",
        ]);
        assert_eq!(audit_args(&plain), plain);
    }

    #[test]
    fn audit_records_append_whole_lines() {
        let dir = std::env::temp_dir().join(format!("fleet-audit-{}", std::process::id()));
        let path = dir.join("nested").join("gh-audit.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        let threads: Vec<_> = (0..8)
            .map(|n| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for k in 0..50 {
                        let entry = serde_json::json!({"n": n, "k": k, "pad": "x".repeat(2000)});
                        append_audit_record(&path, &entry).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let log = std::fs::read_to_string(&path).unwrap();
        assert_eq!(log.lines().count(), 400);
        for line in log.lines() {
            serde_json::from_str::<serde_json::Value>(line).unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_call_whitelist() {
        let read = |a: &[&str]| is_read_call(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(read(&[
            "project",
            "item-list",
            "4",
            "--owner",
            "developing-today"
        ]));
        assert!(read(&[
            "project",
            "field-list",
            "4",
            "--owner",
            "developing-today"
        ]));
        assert!(read(&["issue", "list", "--repo", "developing-today/code"]));
        assert!(read(&["api", "repos/developing-today/code"]));
        // GraphQL: the operation decides, not the field flags (#12154).
        assert!(read(&["api", "graphql", "-f", "query={viewer{login}}"]));
        assert!(read(&[
            "api",
            "graphql",
            "-f",
            "query=query($o:String!){organization(login:$o){projectV2(number:4){id}}}",
            "-F",
            "o=developing-today"
        ]));
        assert!(read(&[
            "api",
            "-H",
            "X: y",
            "graphql",
            "--raw-field=query= # board\n query Q { viewer { login } }"
        ]));
        assert!(!read(&[
            "api",
            "graphql",
            "-f",
            "query=mutation{addStar(input:{starrableId:\"x\"}){clientMutationId}}"
        ]));
        assert!(!read(&["api", "graphql", "-f", "query= mutation M { x }"]));
        // operationName can select a mutation from a document that opens
        // with a query.
        assert!(!read(&[
            "api",
            "graphql",
            "-f",
            "query=query Q { viewer { login } } mutation M { x }",
            "-f",
            "operationName=M"
        ]));
        assert!(!read(&[
            "api",
            "graphql",
            "-f",
            "query={ a } subscription S { b }"
        ]));
        assert!(!read(&["api", "graphql", "-f", "query=queryish { x }"]));
        // Another option's value that looks like a query field is not one.
        assert!(!read(&[
            "api",
            "graphql",
            "-f",
            "query=mutation { x }",
            "--template",
            "-fquery={viewer{login}}"
        ]));
        assert!(read(&[
            "api",
            "graphql",
            "--jq",
            "-fquery=mutation{x}",
            "-f",
            "query={viewer{login}}"
        ]));
        assert!(!read(&[
            "api",
            "graphql",
            "-f",
            "query={ a }",
            "-f",
            "query={ b }"
        ]));
        assert!(!read(&["api", "graphql", "-F", "query=@board.graphql"]));
        // gh expands placeholders in typed fields, so what it sends is
        // unknown; raw fields keep them literally.
        assert!(!read(&[
            "api",
            "graphql",
            "-F",
            "query={repo} {addComment(input:{}){x}}"
        ]));
        assert!(!read(&[
            "api",
            "graphql",
            "--field=query={owner} { viewer { login } }"
        ]));
        assert!(!read(&["api", "graphql", "-Fquery={branch} { x }"]));
        assert!(read(&[
            "api",
            "graphql",
            "-f",
            "query={ viewer { login } } # {repo}"
        ]));
        assert!(!read(&["api", "graphql", "--input", "body.json"]));
        assert!(!read(&["api", "graphql"]));
        // A field named query on a REST call is still a POST-class write.
        assert!(!read(&["api", "repos/o/r/issues", "-f", "query=x"]));
        // A field value attached to its flag still makes the call POST.
        assert!(!read(&["api", "repos/o/r/issues", "-ftitle=x"]));
        assert!(!read(&["api", "repos/o/r/labels", "-Fname=x"]));
        assert!(read(&["api", "user", "--method", "GET"]));
        assert!(!read(&["api", "user", "--method", "PATCH"]));
        assert!(!read(&["issue", "comment", "1", "--body", "x"]));
        assert!(!read(&["issue", "create", "--title", "x"]));
        assert!(!read(&["project", "item-edit", "--id", "x"]));
        assert!(!read(&["pr", "merge", "1"]));
        assert!(!read(&["frobnicate", "list"]));
    }

    // oracle: code#8887's rotation definition as extended by Drew
    // 2026-11-12 — the installations of all three fleet Apps
    // (fleet: 3 owners, developing.today: 2, backup: 1), with the
    // personal token LAST (Drew 2026-11-23, code#13310); a call
    // naming an owner leads each App leg with that owner's
    // installation where the App has one.
    #[test]
    fn rotation_order_and_owner_hint() {
        let args: Vec<String> = ["api", "repos/developing-today/code"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(owner_hint(&args).as_deref(), Some("developing-today"));
        let order = rotation_with(&args, None, &Owners::default());
        assert_eq!(
            order,
            vec![
                TokenSource::app(GhApp::Fleet, "developing-today"),
                TokenSource::app(GhApp::Fleet, "developing-today-forks"),
                TokenSource::app(GhApp::Fleet, "dezren39"),
                TokenSource::app(GhApp::DevToday, "developing-today"),
                TokenSource::app(GhApp::DevToday, "dezren39"),
                TokenSource::app(GhApp::Backup, "developing-today"),
                TokenSource::Personal,
            ]
        );
        assert_eq!(order.len(), 7);
        assert!(order.contains(&TokenSource::app(GhApp::DevToday, "dezren39")));
        assert!(order.contains(&TokenSource::app(GhApp::Backup, "developing-today")));
        assert!(!order.contains(&TokenSource::app(GhApp::Backup, "dezren39")));

        let fork_args: Vec<String> = ["issue", "list", "--repo", "developing-today-forks/opencode"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let order = rotation_with(&fork_args, None, &Owners::default());
        assert_eq!(
            order[0],
            TokenSource::app(GhApp::Fleet, "developing-today-forks")
        );
        assert_eq!(order[6], TokenSource::Personal);

        let fixed = rotation_with(
            &fork_args,
            Some("app:dezren39,personal"),
            &Owners::default(),
        );
        assert_eq!(
            fixed,
            vec![
                TokenSource::app(GhApp::Fleet, "dezren39"),
                TokenSource::Personal
            ]
        );

        let fixed2 = rotation_with(
            &fork_args,
            Some("devtoday:developing-today,backup:developing-today"),
            &Owners::default(),
        );
        assert_eq!(
            fixed2,
            vec![
                TokenSource::app(GhApp::DevToday, "developing-today"),
                TokenSource::app(GhApp::Backup, "developing-today")
            ]
        );
        // Drew 2026-11-23 (code#13310): an operator-specified
        // order wins over the default exactly as given — personal
        // first included.
        let fixed3 = rotation_with(
            &fork_args,
            Some("personal,app:dezren39"),
            &Owners::default(),
        );
        assert_eq!(
            fixed3,
            vec![
                TokenSource::Personal,
                TokenSource::app(GhApp::Fleet, "dezren39")
            ]
        );
        assert_eq!(
            TokenSource::app(GhApp::DevToday, "dezren39").label(),
            "devtoday:dezren39"
        );
        assert_eq!(
            TokenSource::app(GhApp::Fleet, "dezren39").label(),
            "app:dezren39"
        );
    }

    fn configured_apps_fixture() -> Config {
        let mut config = Config::default();
        config.gh.apps = vec![
            fleet_config::GhAppConfig {
                slug: "extra".to_string(),
                app_id: Some(123456),
                app_id_secret: None,
                key_secret: "github.extra.key".to_string(),
                owners: vec!["first-org".to_string(), "layercloud".to_string()],
            },
            fleet_config::GhAppConfig {
                slug: "secret-id".to_string(),
                app_id: None,
                app_id_secret: Some("github.secret-id.id".to_string()),
                key_secret: "github.secret-id.key".to_string(),
                owners: vec!["second-org".to_string()],
            },
        ];
        config
    }

    // Oracle: #10311 (c) requires every installation, independently
    // of the configured rotation owner set, in stable App/owner order.
    #[test]
    fn tiers_all_lists_every_installation_then_personal() {
        let config = configured_apps_fixture();
        let apps = all_apps(&config);
        assert_eq!(apps.len(), 5);
        assert_eq!(apps[3].slug(), "extra");
        assert_eq!(apps[4].slug(), "secret-id");

        let sources = tiers_all_from_lists(&[
            (GhApp::Fleet, vec!["z-org".into(), "a-org".into()]),
            (GhApp::Configured("extra".into()), vec!["a-org".into()]),
        ]);
        assert_eq!(
            sources.iter().map(TokenSource::label).collect::<Vec<_>>(),
            ["app:a-org", "app:z-org", "extra:a-org", "personal"]
        );
        assert_eq!(
            rotation_with_config(&[], None, &Owners::default(), &config)
                .iter()
                .map(TokenSource::label)
                .collect::<Vec<_>>(),
            [
                "app:developing-today",
                "app:developing-today-forks",
                "app:dezren39",
                "devtoday:developing-today",
                "devtoday:dezren39",
                "backup:developing-today",
                "extra:first-org",
                "extra:layercloud",
                "secret-id:second-org",
                "personal"
            ]
        );
    }

    // Oracle: #10311's additive App contract: built-ins unchanged,
    // configured Apps in file order before personal, hint reorders only.
    #[test]
    fn configured_apps_rotation_and_order() {
        let config = configured_apps_fixture();
        for (hint, expected) in [
            ("layercloud", vec!["extra:layercloud", "extra:first-org"]),
            ("LAYERCLOUD", vec!["extra:layercloud", "extra:first-org"]),
            ("unknown", vec!["extra:first-org", "extra:layercloud"]),
        ] {
            let args = vec!["--repo".to_string(), format!("{hint}/repo")];
            let sources = rotation_with_config(&args, None, &Owners::default(), &config);
            let builtins = rotation_with(&args, None, &Owners::default());
            assert_eq!(&sources[..6], &builtins[..6]);
            assert_eq!(
                sources[6..8]
                    .iter()
                    .map(TokenSource::label)
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(sources.last(), Some(&TokenSource::Personal));
            assert_eq!(sources[8].label(), "secret-id:second-org");
            assert_eq!(sources.len(), 10);
        }
        let fixed = rotation_with_config(
            &[],
            Some("secret-id:other,personal,extra:layercloud,app:dezren39"),
            &Owners::default(),
            &config,
        );
        assert_eq!(
            fixed.iter().map(TokenSource::label).collect::<Vec<_>>(),
            [
                "secret-id:other",
                "personal",
                "extra:layercloud",
                "app:dezren39"
            ]
        );
        let mut no_owners = config.clone();
        no_owners
            .gh
            .apps
            .iter_mut()
            .for_each(|app| app.owners.clear());
        assert_eq!(
            rotation_with_config(&[], None, &Owners::default(), &no_owners),
            rotation_with(&[], None, &Owners::default())
        );
    }

    // Oracle: cache requests carry only the stable slug; the daemon's
    // own config must map it back to the same ID/key secret names.
    #[test]
    fn configured_apps_cache_slug_and_credentials() {
        let config = configured_apps_fixture();
        for entry in &config.gh.apps {
            let request = gh_cache::request("token", Some(&entry.slug), Some("layercloud"), None);
            let wire: serde_json::Value = serde_json::from_str(&request.to_string()).unwrap();
            let app = GhApp::parse(wire["app"].as_str().unwrap(), &config).unwrap();
            assert_eq!(app.slug(), entry.slug);
            assert_eq!(app.key_secret(&config), entry.key_secret);
            assert_eq!(app.id_secret(&config), entry.app_id_secret.as_deref());
        }
        assert_eq!(GhApp::parse("absent", &config), None);
        assert_eq!(GhApp::parse("extra", &Config::default()), None);
        let minter = RawMinter::new(&config);
        let literal = GhApp::parse("extra", &config).unwrap();
        assert_eq!(minter.app_id(&literal).unwrap(), "123456");
        assert_eq!(minter.app_id(&literal).unwrap(), "123456");
        assert_eq!(minter.app_ids.lock().unwrap().len(), 1);
        let secret = GhApp::parse("secret-id", &config).unwrap();
        assert_eq!(
            minter
                .app_id_with(&secret, |name| {
                    assert_eq!(name, "github.secret-id.id");
                    Ok(" 789012\n".to_string())
                })
                .unwrap(),
            "789012"
        );
        assert_eq!(
            minter
                .app_id_with(&secret, |_| panic!("ID must be memoized"))
                .unwrap(),
            "789012"
        );
        let empty = RawMinter::new(&config);
        assert_eq!(
            empty
                .app_id_with(&secret, |_| Ok(" \n".to_string()))
                .unwrap_err(),
            "app id for secret-id resolved empty"
        );
    }

    proptest::proptest! {
        // Oracle: every valid non-reserved configured slug round-trips
        // through order labels and the existing cache wire's app field.
        #[test]
        fn configured_app_slug_round_trip(suffix in "[a-z0-9-]{1,30}") {
            let mut config = configured_apps_fixture();
            let slug = format!("custom-{suffix}");
            config.gh.apps[0].slug = slug.clone();
            let order = format!("{slug}:layercloud");
            let sources = rotation_with_config(&[], Some(&order), &Owners::default(), &config);
            proptest::prop_assert_eq!(sources.len(), 1);
            proptest::prop_assert_eq!(sources[0].label(), order);
            let request = gh_cache::request("token", Some(&slug), Some("layercloud"), None);
            let app = GhApp::parse(request["app"].as_str().unwrap(), &config).unwrap();
            proptest::prop_assert_eq!(app.slug(), slug.as_str());
            proptest::prop_assert_eq!(app.key_secret(&config), "github.extra.key");
        }
    }

    // oracle: gh's hosts.yml shape (user block carrying oauth_token)
    // — the parser takes the token line and nothing else.
    #[test]
    fn hosts_yml_token_parse() {
        let yml = "github.com:\n    git_protocol: https\n    users:\n        dezren39:\n            oauth_token: TOKEN_VALUE\n    user: dezren39\n";
        assert_eq!(parse_hosts_yml_token(yml).as_deref(), Some("TOKEN_VALUE"));
        assert_eq!(parse_hosts_yml_token("github.com:\n    user: x\n"), None);
        assert_eq!(parse_hosts_yml_token("oauth_token: \n"), None);
    }

    // oracle: the 2026-11-22 write-path break — a daemon answering
    // the `personal` op returns the hosts.yml secret itself, and
    // the value that reaches GH_TOKEN must be the parsed token:
    // header-safe (no whitespace or control characters), never the
    // multi-line file. A secret with no oauth_token is an error
    // naming the secret, on either resolution path.
    #[test]
    fn personal_token_is_the_parsed_token_not_the_secret() {
        let yml = "github.com:\n    git_protocol: https\n    users:\n        dezren39:\n            oauth_token: gho_0123456789abcdefghijklmnopqrstuv\n    user: dezren39\n";
        let token = personal_token(yml).expect("token parses");
        assert_eq!(token, "gho_0123456789abcdefghijklmnopqrstuv");
        assert!(
            token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "token must be a valid Authorization header value: {token:?}"
        );
        assert!(personal_token("github.com:\n    user: x\n").is_err());
    }

    // oracle: RFC 4648 base64url test vectors (padding dropped).
    #[test]
    fn base64url_vectors() {
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(b"\xfb\xff"), "-_8");
    }

    /// Fake identities for the runner tests.
    struct FakeProvider {
        fail_sources: Vec<String>,
    }

    impl TokenProvider for FakeProvider {
        fn token_for(&mut self, source: &TokenSource) -> Result<String, String> {
            let label = source.label();
            if self.fail_sources.contains(&label) {
                Err(format!("fake provider: {label} unavailable"))
            } else {
                Ok(format!("tok-{label}"))
            }
        }
    }

    /// Write a fake `gh` whose behavior keys on the token it was
    /// handed: the personal token is rate-limited, every App token
    /// succeeds; every invocation appends the token it saw to a
    /// log file whose path is baked into the script (no shared env,
    /// so parallel tests cannot race). Returns (dir, script path).
    fn fake_gh(tag: &str) -> (String, String) {
        let dir = std::env::temp_dir().join(format!("fleet-gh-test-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("calls.log");
        let script = dir.join("fake-gh");
        let script_text = format!(
            "#!/bin/sh\n\
             echo \"$GH_TOKEN\" >> \"{log}\"\n\
             if [ \"$GH_TOKEN\" = \"tok-personal\" ]; then\n\
             \techo 'GraphQL: API rate limit exceeded' >&2\n\
             \texit 1\n\
             fi\n\
             if [ \"$GH_TOKEN\" = \"tok-boom\" ]; then\n\
             \techo 'some unclassifiable failure' >&2\n\
             \texit 7\n\
             fi\n\
             if [ \"$GH_TOKEN\" = \"tok-node\" ]; then\n\
             \techo 'GraphQL: Could not resolve to a node with the global id of PVT_kwHOA123' >&2\n\
             \texit 1\n\
             fi\n\
             echo 'fake gh output'\n\
             exit 0\n",
            log = log.display()
        );
        // The script must never be open for writing in this process.
        // Tests run on parallel threads, and any of them may fork to
        // spawn a child; until that child execs it holds a copy of
        // every descriptor this process has open, including one we are
        // writing the script through. Exec'ing the script in that
        // window fails with ETXTBSY (#11834, seen under the Nix build
        // sandbox). So the text goes to a side file this test never
        // runs, and a separate `cp` process, which has exited by the
        // time we go on, writes the file that is executed.
        let source = dir.join("fake-gh.src");
        std::fs::write(&source, script_text).unwrap();
        let _ = std::fs::remove_file(&script);
        let copied = std::process::Command::new("cp")
            .arg(&source)
            .arg(&script)
            .status()
            .expect("run cp to place the fake gh");
        assert!(copied.success(), "cp of the fake gh failed: {copied}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        (
            dir.to_string_lossy().to_string(),
            script.to_string_lossy().to_string(),
        )
    }

    // oracle: the shared-ledger contract in gh_content_ledger — a
    // reservation must be committed before the external create starts;
    // empty stdout on exit 0 is finalized failed without rewriting gh's
    // exit code. CONTENT_LEDGER_PATH points into this test's isolated dir.
    #[test]
    fn create_reserves_before_gh_and_empty_output_records_failure() {
        let dir = std::env::temp_dir().join(format!(
            "fleet-gh-ledger-e2e-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let ledger = dir.join("ledger.sqlite");
        let probe = dir.join("probe");
        let script = format!(
            "#!/bin/sh\n\
             python3 -c 'import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); r=c.execute(\"select outcome from events\").fetchall(); assert r == [(\"reserved\",)], r' \"{}\" || exit 91\n\
             touch \"{}\"\n\
             exit 0\n",
            ledger.display(), probe.display()
        );
        let script_path = place_script(&dir, "ledger-gh", &script);
        let args: Vec<String> = ["issue", "create", "--repo", "owner/repo", "--title", "test"]
            .iter().map(|s| s.to_string()).collect();
        let pending = gh_content_ledger::start_at(&args, &ledger)
            .expect("reserve succeeds")
            .unwrap();
        let attempt = run_gh_once(&script_path, &[], "test-token").expect("fake gh runs");
        assert_eq!(attempt.code, 0, "gh's exit code is preserved");
        assert!(probe.exists(), "fake gh ran after observing reservation");
        gh_content_ledger::finish(&pending, attempt.code, Some(&attempt.stdout))
            .expect("finalize reservation");
        let conn = rusqlite::Connection::open(&ledger).unwrap();
        let (outcome, detail): (String, String) = conn
            .query_row("SELECT outcome, detail FROM events", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(outcome, "failed", "empty successful output is not a created item");
        assert_eq!(detail, "gh exited 0 with empty output");
    }

    /// Place an executable script the way fake_gh does: the text
    /// goes to a side file and a separate `cp` (exited before we
    /// return) writes the file that is executed, so no descriptor
    /// this process holds open can make a child's exec fail with
    /// ETXTBSY (#11834). Returns the script's path.
    fn place_script(dir: &std::path::Path, name: &str, text: &str) -> String {
        let source = dir.join(format!("{name}.src"));
        std::fs::write(&source, text).unwrap();
        let script = dir.join(name);
        let _ = std::fs::remove_file(&script);
        let copied = std::process::Command::new("cp")
            .arg(&source)
            .arg(&script)
            .status()
            .expect("run cp to place the script");
        assert!(copied.success(), "cp of {name} failed: {copied}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        script.to_string_lossy().to_string()
    }

    /// Build the #14650 shim world: a directory holding three
    /// executables that reproduce the production gh-fleet boundary
    /// with the shim's real guard semantics (pkgs/gh-fleet): `gh`
    /// is the shim — guarded (FLEET_GH_SHIM_ACTIVE=1) it execs the
    /// pristine gh, unguarded it exports FLEET_GH_BIN + the guard
    /// and execs `fleet gh`; `fleet` stands in for the nested
    /// fleet a re-entry would create (it logs the re-entry, then
    /// does what that fleet would do — run its resolved
    /// FLEET_GH_BIN); `pristine-gh` is the real gh. Every hop
    /// appends one line to a log whose path is baked into the
    /// scripts (no shared env, so parallel tests cannot race).
    /// Returns (dir, shim path).
    fn shim_world(tag: &str) -> (String, String) {
        let dir = std::env::temp_dir().join(format!("fleet-gh-shim-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("calls.log");
        let log = log.display();
        let pristine_path = dir.join("pristine-gh").display().to_string();
        let fleet_path = dir.join("fleet").display().to_string();
        let pristine = format!(
            "#!/bin/sh\n\
             echo \"pristine $*\" >> \"{log}\"\n\
             echo 'pristine gh output'\n\
             exit 0\n"
        );
        let fleet = format!(
            "#!/bin/sh\n\
             echo \"fleet-reentry $*\" >> \"{log}\"\n\
             exec \"$FLEET_GH_BIN\" \"$@\"\n"
        );
        let shim = format!(
            "#!/bin/sh\n\
             if [ \"${{FLEET_GH_SHIM_ACTIVE:-}}\" = \"1\" ]; then\n\
             exec \"{pristine_path}\" \"$@\"\n\
             fi\n\
             export FLEET_GH_BIN=\"{pristine_path}\"\n\
             export FLEET_GH_SHIM_ACTIVE=1\n\
             exec \"{fleet_path}\" gh \"$@\"\n"
        );
        place_script(&dir, "pristine-gh", &pristine);
        place_script(&dir, "fleet", &fleet);
        let shim_path = place_script(&dir, "gh", &shim);
        (dir.to_string_lossy().to_string(), shim_path)
    }

    // oracle: #14650 — a direct `fleet gh` whose `gh` is the
    // gh-fleet shim must reach the pristine gh in ONE hop. The
    // production shim's guard semantics are the truth: guarded,
    // the shim execs the pristine gh; unguarded, it re-enters
    // fleet. The scripts' log decides — any "fleet-reentry" line
    // (or a second line at all) means the guard was not set.
    #[test]
    fn run_gh_once_does_not_reenter_fleet_through_the_shim() {
        // The guard must start absent from the ambient environment,
        // or the child would inherit it and the test could not tell
        // whether run_gh_once set it. Nothing else in this crate
        // reads the variable, so clearing it cannot race a reader.
        unsafe { std::env::remove_var(SHIM_GUARD_ENV) };
        let (dir, shim) = shim_world("once");
        let log = format!("{dir}/calls.log");
        let args: Vec<String> = ["issue", "list"].iter().map(|s| s.to_string()).collect();
        let attempt = run_gh_once(&shim, &args, "tok-test").unwrap();
        assert_eq!(attempt.code, 0);
        assert_eq!(
            String::from_utf8_lossy(&attempt.stdout),
            "pristine gh output\n"
        );
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            calls.lines().count(),
            1,
            "exactly one pristine call, no re-entry: {calls}"
        );
        assert!(
            calls.starts_with("pristine "),
            "the one call is the pristine gh: {calls}"
        );
    }

    // oracle: #14650, raw path — the same guard duty on run_raw:
    // raw mode hands the caller the pristine gh, never a nested
    // fleet. run_raw inherits stdio, so the scripts' log (not
    // captured output) is the evidence.
    #[test]
    fn run_raw_does_not_reenter_fleet_through_the_shim() {
        unsafe { std::env::remove_var(SHIM_GUARD_ENV) };
        let (dir, shim) = shim_world("raw");
        let log = format!("{dir}/calls.log");
        let args: Vec<String> = ["issue", "list"].iter().map(|s| s.to_string()).collect();
        let code = run_raw(&shim, &args);
        assert_eq!(code, 0);
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            calls.lines().count(),
            1,
            "exactly one pristine call, no re-entry: {calls}"
        );
        assert!(
            calls.starts_with("pristine "),
            "the one call is the pristine gh: {calls}"
        );
    }

    // oracle: code#8887's headline acceptance — a rate-limited
    // first attempt fails over and the caller sees only the final
    // result (exit 0, the succeeding attempt's output).
    #[test]
    fn failover_on_rate_limit() {
        let (dir, script) = fake_gh("failover");
        let log = format!("{dir}/calls.log");
        let mut provider = FakeProvider {
            fail_sources: vec![],
        };
        let args: Vec<String> = ["issue", "list"].iter().map(|s| s.to_string()).collect();
        let sources = vec![
            TokenSource::Personal,
            TokenSource::app(GhApp::Fleet, "developing-today"),
        ];
        let code = run_with_provider(&mut provider, &script, &args, &sources, false);
        assert_eq!(code, 0);
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(calls.lines().count(), 2, "both tokens attempted: {calls}");
    }

    // oracle: #15556 — `gh secret set --help` / `gh variable set
    // --help`: the value comes from stdin exactly when there is no
    // --body and no real --env-file; `--env-file -` is stdin too.
    #[test]
    fn stdin_supplies_value_shapes() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(stdin_supplies_value(&v(&["secret", "set", "NAME"])));
        assert!(stdin_supplies_value(&v(&[
            "-R", "o/r", "secret", "set", "NAME"
        ])));
        assert!(stdin_supplies_value(&v(&["variable", "set", "NAME"])));
        assert!(stdin_supplies_value(&v(&[
            "secret",
            "set",
            "NAME",
            "--env-file",
            "-"
        ])));
        assert!(!stdin_supplies_value(&v(&[
            "secret", "set", "NAME", "--body", "x"
        ])));
        assert!(!stdin_supplies_value(&v(&[
            "secret", "set", "NAME", "--body=x"
        ])));
        assert!(!stdin_supplies_value(&v(&["secret", "set", "-f", ".env"])));
        assert!(!stdin_supplies_value(&v(&["secret", "list"])));
        assert!(!stdin_supplies_value(&v(&["issue", "list"])));
    }

    // oracle: #15556's evidence — real gh stores an empty stdin
    // read (or an empty --body) as an empty secret and exits 0, so
    // fleet must refuse both before any attempt; a non-empty piped
    // value, a non-empty body, and the interactive (terminal)
    // prompt path are all left alone.
    #[test]
    fn empty_value_error_shapes() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let set = v(&["secret", "set", "NAME", "--repo", "o/r"]);
        assert!(empty_value_error(&set, Some(b"")).is_some());
        assert!(empty_value_error(&set, Some(b"  \n")).is_some());
        assert!(empty_value_error(&set, Some(b"the-value\n")).is_none());
        assert!(empty_value_error(&set, None).is_none(), "terminal prompt");
        let body = v(&["secret", "set", "NAME", "--body", ""]);
        assert!(empty_value_error(&body, None).is_some());
        let body_ok = v(&["secret", "set", "NAME", "--body=x"]);
        assert!(empty_value_error(&body_ok, Some(b"")).is_none());
        let list = v(&["secret", "list"]);
        assert!(empty_value_error(&list, Some(b"")).is_none());
    }

    // oracle: #15556 at the exec boundary — a fake gh that reads
    // its stdin dry and then fails with a rate limit under the
    // first token (the failover shape): the second attempt must
    // receive the SAME stdin bytes, not EOF. Before the replay,
    // the second attempt logged an empty value and real gh stored
    // it as an empty secret with exit 0.
    #[test]
    fn stdin_replayed_to_failover_attempt() {
        let dir =
            std::env::temp_dir().join(format!("fleet-gh-test-{}-stdin-replay", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("stdin.log");
        let script = dir.join("fake-gh");
        let script_text = format!(
            "#!/bin/sh\n\
             v=$(cat)\n\
             echo \"$GH_TOKEN|$v\" >> \"{log}\"\n\
             if [ \"$GH_TOKEN\" = \"tok-personal\" ]; then\n\
             \techo 'GraphQL: API rate limit exceeded' >&2\n\
             \texit 1\n\
             fi\n\
             exit 0\n",
            log = log.display()
        );
        // Same ETXTBSY guard as fake_gh: write a side file, place
        // the executable with a separate `cp` process.
        let source = dir.join("fake-gh.src");
        std::fs::write(&source, script_text).unwrap();
        let _ = std::fs::remove_file(&script);
        let copied = std::process::Command::new("cp")
            .arg(&source)
            .arg(&script)
            .status()
            .expect("run cp to place the fake gh");
        assert!(copied.success(), "cp of the fake gh failed: {copied}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let script = script.to_string_lossy().to_string();
        let mut provider = FakeProvider {
            fail_sources: vec![],
        };
        let args: Vec<String> = ["secret", "set", "NAME", "--repo", "o/r"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let sources = vec![
            TokenSource::Personal,
            TokenSource::app(GhApp::Fleet, "developing-today"),
        ];
        let (code, _) = run_with_provider_capturing(
            &mut provider,
            &script,
            &args,
            &sources,
            false,
            false,
            Some(b"the-value"),
        );
        assert_eq!(code, 0);
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            calls, "tok-personal|the-value\ntok-app:developing-today|the-value\n",
            "both attempts received the full stdin value: {calls:?}"
        );
    }

    // oracle: the #8887 retry-safety rule — a write whose failure
    // is unclassifiable (the request may have landed) is NOT
    // repeated under the next token; its exit code and output are
    // the caller's result.
    #[test]
    fn write_not_repeated_on_unknown_failure() {
        let (dir, script) = fake_gh("write-once");
        let log = format!("{dir}/calls.log");
        struct BoomProvider;
        impl TokenProvider for BoomProvider {
            fn token_for(&mut self, source: &TokenSource) -> Result<String, String> {
                match source {
                    TokenSource::Personal => Ok("tok-boom".to_string()),
                    other => Ok(format!("tok-{}", other.label())),
                }
            }
        }
        let mut provider = BoomProvider;
        let args: Vec<String> = ["issue", "comment", "1", "--body", "hi"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let sources = vec![
            TokenSource::Personal,
            TokenSource::app(GhApp::Fleet, "developing-today"),
        ];
        let code = run_with_provider(&mut provider, &script, &args, &sources, false);
        assert_eq!(code, 7, "the first attempt's exit code is propagated");
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            calls.lines().count(),
            1,
            "write attempted exactly once: {calls}"
        );
    }

    // oracle: the #12154 review — a batched GraphQL mutation whose
    // later field fails node resolution may already have executed an
    // earlier field, so the mutation is NOT replayed under the next
    // token; the same failure on a read IS retried.
    #[test]
    fn write_not_repeated_on_node_resolution_failure() {
        struct NodeProvider;
        impl TokenProvider for NodeProvider {
            fn token_for(&mut self, source: &TokenSource) -> Result<String, String> {
                match source {
                    TokenSource::Personal => Ok("tok-node".to_string()),
                    other => Ok(format!("tok-{}", other.label())),
                }
            }
        }
        let sources = vec![
            TokenSource::Personal,
            TokenSource::app(GhApp::Fleet, "developing-today"),
        ];
        // A batched mutation: addComment first, a node reference second.
        let mutation: Vec<String> = [
            "api",
            "graphql",
            "-f",
            "query=mutation{addComment(input:{subjectId:\"x\",body:\"y\"}){clientMutationId} node(id:\"PVT_kwHOA123\"){id}}",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert!(!is_read_call(&mutation));
        let (dir, script) = fake_gh("write-node");
        let log = format!("{dir}/calls.log");
        let mut provider = NodeProvider;
        let code = run_with_provider(&mut provider, &script, &mutation, &sources, false);
        assert_eq!(code, 1, "the first attempt's exit code is propagated");
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            calls.lines().count(),
            1,
            "mutation attempted exactly once: {calls}"
        );
        // The same failure on a read fails over to the next token.
        let query: Vec<String> = ["api", "graphql", "-f", "query=query{viewer{login}}"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(is_read_call(&query));
        let (dir2, script2) = fake_gh("read-node");
        let log2 = format!("{dir2}/calls.log");
        let mut provider = NodeProvider;
        let code = run_with_provider(&mut provider, &script2, &query, &sources, false);
        assert_eq!(code, 0);
        let calls = std::fs::read_to_string(&log2).unwrap();
        assert_eq!(calls.lines().count(), 2, "both tokens attempted: {calls}");
    }

    // oracle: Drew's 2026-11-14 raw-mode contract — a LEADING
    // `--raw` is fleet's and is stripped; `--raw` anywhere else
    // is gh's own argument and must not trigger the bypass;
    // FLEET_GH_RAW=1 selects raw with the args untouched.
    #[test]
    fn raw_args_split() {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<String>>();
        assert_eq!(
            raw_args(&v(&["--raw", "api", "user"]), false),
            Some(v(&["api", "user"]))
        );
        assert_eq!(raw_args(&v(&["--raw"]), false), Some(v(&[])));
        assert_eq!(raw_args(&v(&["api", "--raw-field", "x=1"]), false), None);
        assert_eq!(raw_args(&v(&["issue", "list"]), false), None);
        assert_eq!(
            raw_args(&v(&["issue", "list"]), true),
            Some(v(&["issue", "list"]))
        );
    }

    // oracle: the same contract, at the exec boundary — raw mode
    // runs the real gh exactly once and hands it NO token: the
    // fake gh logs the GH_TOKEN it saw, and raw must log an empty
    // one, or the ambient GH_TOKEN untouched when the test runner
    // carries one, where every rotated call above logs a tok-* value.
    #[test]
    fn raw_mode_runs_gh_without_token() {
        let (dir, script) = fake_gh("raw");
        let log = format!("{dir}/calls.log");
        let args: Vec<String> = ["issue", "list"].iter().map(|s| s.to_string()).collect();
        let code = run_raw(&script, &args);
        assert_eq!(code, 0);
        let calls = std::fs::read_to_string(&log).unwrap();
        let ambient = std::env::var("GH_TOKEN").unwrap_or_default();
        assert_eq!(
            calls,
            format!("{ambient}\n"),
            "exactly one call, with no token injected: {calls:?}"
        );
    }

    // oracle: #10311 (d) — a missing `gh` binary must name the
    // remedy (install gh, or set FLEET_GH_BIN); the spawn-error
    // path returns exit 3 with no attempt made.
    #[test]
    fn missing_gh_error_names_the_remedy() {
        let e = std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory");
        let msg = spawn_failure_message("gh", &e);
        assert!(msg.contains("install gh"), "no install remedy: {msg}");
        assert!(msg.contains("FLEET_GH_BIN"), "no FLEET_GH_BIN remedy: {msg}");
        assert!(msg.contains("'gh'"), "binary name lost: {msg}");
    }

    #[test]
    fn missing_gh_spawn_returns_unresolved() {
        let mut provider = FakeProvider {
            fail_sources: vec![],
        };
        let args: Vec<String> = ["issue", "list"].iter().map(|s| s.to_string()).collect();
        let sources = vec![TokenSource::Personal];
        let code = run_with_provider(
            &mut provider,
            "/nonexistent/fleet-gh-test-gh",
            &args,
            &sources,
            false,
        );
        assert_eq!(code, EXIT_UNRESOLVED, "spawn failure must exit 3");
    }

    // oracle: #10311 (d) — the no-credentials path prints each
    // distinct provider error once, first-seen order.
    #[test]
    fn provider_errors_printed_once() {
        let errors = vec![
            "broker unreachable".to_string(),
            "broker unreachable".to_string(),
            "cache daemon down".to_string(),
            "broker unreachable".to_string(),
        ];
        assert_eq!(
            distinct_errors(&errors),
            vec!["broker unreachable", "cache daemon down"]
        );
        assert!(distinct_errors(&[]).is_empty());
    }
}

/// Property tests for the read classifier behind arbitrary-error
/// retries. Oracle: the invariant that a call able to mutate (a
/// mutation or subscription anywhere in the document, a typed field
/// gh may rewrite, more than one query field, or a body from
/// `--input`) is never a read; and, in the other direction, that a
/// plain read document sent as a raw field is one.
#[cfg(test)]
mod graphql_read_props {
    use super::is_read_call;
    use proptest::prelude::*;

    fn comment() -> impl Strategy<Value = String> {
        prop_oneof![
            Just(String::new()),
            Just("# note\n".to_string()),
            Just("  # a\n# b\n".to_string())
        ]
    }

    fn name() -> impl Strategy<Value = String> {
        "[A-Z][a-z]{0,5}"
    }

    fn read_doc() -> impl Strategy<Value = String> {
        (comment(), name()).prop_flat_map(|(c, n)| {
            prop_oneof![
                Just(format!("{c}{{ viewer {{ login }} }}")),
                Just(format!("{c}query {{ x }}")),
                Just(format!("{c}query {n} {{ x }}")),
                Just(format!("{c}query($n: Int) {{ x(n: $n) }}")),
            ]
        })
    }

    fn write_doc() -> impl Strategy<Value = String> {
        (comment(), name(), name()).prop_flat_map(|(c, a, b)| {
            prop_oneof![
                Just(format!("{c}mutation {{ addStar(input: {{}}) {{ x }} }}")),
                Just(format!("{c}mutation {a} {{ x }}")),
                Just(format!("{c}subscription {{ x }}")),
                Just(format!("{c}subscription {a} {{ x }}")),
                Just(format!("{c}query {a} {{ x }} mutation {b} {{ y }}")),
                Just(format!("{c}{{ x }}\nmutation {b} {{ y }}")),
                Just(format!("{c}query {a} {{ x }}\nsubscription {b} {{ y }}")),
            ]
        })
    }

    /// How the query field is written: (args, typed).
    fn field(doc: String) -> impl Strategy<Value = (Vec<String>, bool)> {
        let q = format!("query={doc}");
        prop_oneof![
            Just((vec!["-f".to_string(), q.clone()], false)),
            Just((vec!["--raw-field".to_string(), q.clone()], false)),
            Just((vec![format!("-f{q}")], false)),
            Just((vec![format!("--raw-field={q}")], false)),
            Just((vec!["-F".to_string(), q.clone()], true)),
            Just((vec!["--field".to_string(), q.clone()], true)),
            Just((vec![format!("-F{q}")], true)),
            Just((vec![format!("--field={q}")], true)),
        ]
    }

    /// Options that neither carry a query nor change the method.
    fn noise() -> impl Strategy<Value = Vec<String>> {
        let one = prop_oneof![
            Just(vec!["--paginate".to_string()]),
            Just(vec![
                "-H".to_string(),
                "Accept: application/json".to_string()
            ]),
            Just(vec!["--header=X-Y: z".to_string()]),
            Just(vec!["--jq".to_string(), ".data".to_string()]),
            Just(vec!["-f".to_string(), "owner=o".to_string()]),
            Just(vec!["-F".to_string(), "n=1".to_string()]),
        ];
        prop::collection::vec(one, 0..3).prop_map(|v| v.concat())
    }

    fn call(before: Vec<String>, field: Vec<String>, after: Vec<String>) -> Vec<String> {
        let mut v = vec!["api".to_string(), "graphql".to_string()];
        v.extend(before);
        v.extend(field);
        v.extend(after);
        v
    }

    proptest! {
        #[test]
        fn generated_writes_are_never_reads(
            (f, _) in write_doc().prop_flat_map(field),
            before in noise(),
            after in noise(),
        ) {
            let args = call(before, f, after);
            prop_assert!(!is_read_call(&args), "write classified as read: {args:?}");
        }

        #[test]
        fn raw_read_documents_are_reads(
            // No placeholder in these documents, so typed fields are reads too.
            (f, _) in read_doc().prop_flat_map(field),
            before in noise(),
            after in noise(),
        ) {
            let args = call(before, f, after);
            prop_assert!(is_read_call(&args), "read classified as write: {args:?}");
        }

        #[test]
        fn typed_placeholders_are_never_reads(
            doc in read_doc(),
            p in prop_oneof![Just("{owner}"), Just("{repo}"), Just("{branch}")],
            before in noise(),
            after in noise(),
        ) {
            let doc = format!("{doc} # {p}");
            for flag in ["-F", "--field"] {
                let args = call(before.clone(), vec![flag.to_string(), format!("query={doc}")], after.clone());
                prop_assert!(!is_read_call(&args), "typed placeholder classified as read: {args:?}");
            }
        }

        #[test]
        fn a_second_query_field_or_input_is_never_a_read(
            (f1, _) in read_doc().prop_flat_map(field),
            (f2, _) in read_doc().prop_flat_map(field),
            before in noise(),
            input in any::<bool>(),
        ) {
            let args = if input {
                call(before, f1, vec!["--input".to_string(), "body.json".to_string()])
            } else {
                call(before, f1, f2)
            };
            prop_assert!(!is_read_call(&args), "ambiguous call classified as read: {args:?}");
        }
    }
}
