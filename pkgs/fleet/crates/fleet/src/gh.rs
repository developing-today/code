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
//!   1. the personal token: Drew's `gh` OAuth token, parsed out of
//!      the broker secret named `github-cli.hosts-yml` (the hosts
//!      file itself, stored by name — the value never leaves the
//!      resolver);
//!   2. installation tokens of the fleet's THREE GitHub Apps, in
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
//!
//! The App leg's own HTTPS validates against the PLATFORM root
//! store (ureq's platform-verifier), not webpki-roots alone: hosts
//! behind a TLS-intercepting egress proxy (the sandbox) present the
//! proxy's CA, which lives in the platform store — with webpki
//! roots the leg died with UnknownIssuer while `gh` itself worked.
//!
//! `fleet gh tiers` (a shadow subcommand; real `gh` has none)
//! prints every rotation source with its core and GraphQL
//! rate-limit headroom and exits 3 when any source is down.
//!
//! Retry-safety rule (the #8887 contract): a WRITE is only repeated
//! under a fresh token when the failure proves GitHub refused the
//! request before processing it — rate-limit rejections (the limiter
//! answers before the mutation runs) and permission/scope rejections
//! (INSUFFICIENT_SCOPES, "Resource not accessible by …") are both
//! pre-execution refusals. Any other failure of a write is returned
//! as-is: the request may have landed, and repeating it could
//! double-post. Reads repeat on any failure class.
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
//!   FLEET_GH_APP_ID     GitHub App ID (default: 5612056)
//!   FLEET_GH_APP_OWNERS comma list replacing the default App owners
//!   FLEET_GH_ORDER      comma list fixing the rotation exactly:
//!                       `personal`, `app:<owner>` (fleet App),
//!                       `devtoday:<owner>`, `backup:<owner>` entries
//!   FLEET_GH_DEVTODAY_OWNERS / FLEET_GH_BACKUP_OWNERS
//!                       comma lists replacing those Apps' owners
//!   FLEET_GH_TRACE=1    name each attempt's token source on stderr

use fleet_config::Config;
use fleet_secrets::Resolver;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const EXIT_OK: i32 = 0;
const EXIT_USAGE: i32 = 2;
/// The final attempt's exit code is propagated as-is; 3 is ours only
/// for "no token source could be resolved at all" (matches the
/// secrets/dispatch unresolved exit shape).
const EXIT_UNRESOLVED: i32 = 3;

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
       fleet gh tiers

Run the real `gh` CLI with every argument passed through unchanged,
under a token selected from the fleet rotation (personal token, then
the fleet's three GitHub Apps' installation tokens). A rate-limit or
permission failure fails over to the next token; the caller sees only
the final attempt's output and exit code. `fleet gh tiers` lists each
rotation source with its rate-limit headroom. See code#8887.";

/// The fleet's GitHub Apps, in rotation order. Each carries its
/// private key (and, except the fleet App whose ID is a public
/// constant, its App ID) as broker secrets resolved by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GhApp {
    Fleet,
    DevToday,
    Backup,
}

impl GhApp {
    const ALL: [GhApp; 3] = [GhApp::Fleet, GhApp::DevToday, GhApp::Backup];

    /// The FLEET_GH_ORDER / label prefix for this App's sources.
    fn slug(&self) -> &'static str {
        match self {
            GhApp::Fleet => "app",
            GhApp::DevToday => "devtoday",
            GhApp::Backup => "backup",
        }
    }

    fn key_secret(&self) -> &'static str {
        match self {
            GhApp::Fleet => APP_KEY_SECRET_NAME,
            GhApp::DevToday => DEVTODAY_APP_KEY_SECRET_NAME,
            GhApp::Backup => BACKUP_APP_KEY_SECRET_NAME,
        }
    }

    /// The secret carrying the App ID, when the ID is not a public
    /// constant compiled in (the fleet App's is: 5612056).
    fn id_secret(&self) -> Option<&'static str> {
        match self {
            GhApp::Fleet => None,
            GhApp::DevToday => Some(DEVTODAY_APP_ID_SECRET_NAME),
            GhApp::Backup => Some(BACKUP_APP_ID_SECRET_NAME),
        }
    }

    fn default_owners(&self) -> &'static [&'static str] {
        match self {
            GhApp::Fleet => &DEFAULT_APP_OWNERS,
            GhApp::DevToday => &DEVTODAY_DEFAULT_OWNERS,
            GhApp::Backup => &BACKUP_DEFAULT_OWNERS,
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
        "api" => {
            // gh api: GET unless --method says otherwise or a field
            // flag (-f/-F/--field/--raw-field/--input) implies POST.
            let mut i = 1;
            while i < args.len() {
                let a = args[i].as_str();
                if a == "-f"
                    || a == "-F"
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

    fn for_app(&self, app: GhApp) -> Vec<String> {
        let over = match app {
            GhApp::Fleet => &self.fleet,
            GhApp::DevToday => &self.devtoday,
            GhApp::Backup => &self.backup,
        };
        over.clone()
            .unwrap_or_else(|| app.default_owners().iter().map(|s| s.to_string()).collect())
    }
}

/// The rotation for one invocation: personal first, then each
/// App's owners with the hinted owner (if any) leading each App leg.
/// `FLEET_GH_ORDER` replaces the whole order when set.
/// Oracle: code#8887's rotation definition as extended by Drew
/// 2026-11-12 (personal + all three fleet Apps' installations),
/// hint-first per the installation-scope fact.
fn rotation(args: &[String]) -> Vec<TokenSource> {
    let order_env = std::env::var("FLEET_GH_ORDER").ok();
    rotation_with(args, order_env.as_deref(), &Owners::from_env())
}

/// `rotation` with its environment inputs made explicit, so the
/// order logic is testable without env mutation races.
fn rotation_with(
    args: &[String],
    order_env: Option<&str>,
    owners_env: &Owners,
) -> Vec<TokenSource> {
    if let Some(order) = order_env {
        let parsed: Vec<TokenSource> = order
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| {
                for app in GhApp::ALL {
                    if let Some(owner) = s.strip_prefix(&format!("{}:", app.slug())) {
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
    let mut out = vec![TokenSource::Personal];
    for app in GhApp::ALL {
        let mut owners = owners_env.for_app(app);
        if let Some(hint) = &hint {
            if let Some(pos) = owners.iter().position(|o| o.eq_ignore_ascii_case(hint)) {
                let owner = owners.remove(pos);
                owners.insert(0, owner);
            }
        }
        out.extend(owners.into_iter().map(|o| TokenSource::app(app, &o)));
    }
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
        FailureClass::Other => read_only,
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

/// The production provider: the fleet-secrets resolver chain for
/// named secrets, plus in-process App installation minting.
struct FleetTokenProvider<'a> {
    config: &'a Config,
    app_ids: Vec<(GhApp, String)>,
    app_jwts: Vec<(GhApp, String)>,
    minted: Vec<(GhApp, String, String)>,
}

impl<'a> FleetTokenProvider<'a> {
    fn new(config: &'a Config) -> Self {
        FleetTokenProvider {
            config,
            app_ids: Vec::new(),
            app_jwts: Vec::new(),
            minted: Vec::new(),
        }
    }

    /// The App ID for one App: the fleet App's is a public constant
    /// (FLEET_GH_APP_ID may override it); the other Apps carry
    /// theirs in the broker next to their keys.
    fn app_id(&mut self, app: GhApp) -> Result<String, String> {
        if let Some((_, id)) = self.app_ids.iter().find(|(a, _)| *a == app) {
            return Ok(id.clone());
        }
        let id = match app.id_secret() {
            None => std::env::var("FLEET_GH_APP_ID")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_APP_ID.to_string()),
            Some(name) => self.resolve_named(name)?.trim().to_string(),
        };
        if id.is_empty() {
            return Err(format!("app id for {} resolved empty", app.slug()));
        }
        self.app_ids.push((app, id.clone()));
        Ok(id)
    }

    fn resolve_named(&self, name: &str) -> Result<String, String> {
        Resolver::default_chain(self.config)
            .resolve(name)
            .map(|v| v.expose().to_string())
            .map_err(|e| format!("secret '{name}' unresolved: {e}"))
    }

    /// One App's JWT, built once per process per App: header and
    /// payload are public claims; only the signature touches the
    /// private key, and that happens inside `openssl` with the key
    /// arriving on an inherited fd.
    fn jwt(&mut self, app: GhApp) -> Result<String, String> {
        if let Some((_, jwt)) = self.app_jwts.iter().find(|(a, _)| *a == app) {
            return Ok(jwt.clone());
        }
        let app_id = self.app_id(app)?;
        let key_pem = self.resolve_named(app.key_secret())?;
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
        self.app_jwts.push((app, jwt.clone()));
        Ok(jwt)
    }

    fn mint_installation_token(&mut self, app: GhApp, owner: &str) -> Result<String, String> {
        if let Some((_, _, token)) = self.minted.iter().find(|(a, o, _)| *a == app && o == owner) {
            return Ok(token.clone());
        }
        let jwt = self.jwt(app)?;
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
        // Resolve the owner's installation live, paginating: the App
        // has 100+ installations and a first-page-only read silently
        // loses the fleet's (the fleet-paperwork #7930 break).
        let mut installation_id: Option<i64> = None;
        let mut page = 1;
        while installation_id.is_none() {
            let url = format!("{GITHUB_API}/app/installations?per_page=100&page={page}");
            let (status, body) = api_call(&agent, "GET", &url, &jwt, None)?;
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
                if login.eq_ignore_ascii_case(owner) {
                    installation_id = entry.get("id").and_then(|v| v.as_i64());
                    break;
                }
            }
            if count < 100 {
                break;
            }
            page += 1;
        }
        let Some(iid) = installation_id else {
            return Err(format!("no app installation found for owner '{owner}'"));
        };
        let url = format!("{GITHUB_API}/app/installations/{iid}/access_tokens");
        let (status, body) = api_call(&agent, "POST", &url, &jwt, Some("{}"))?;
        if status != 201 {
            return Err(format!(
                "installation token mint for '{owner}' failed: HTTP {status}"
            ));
        }
        let parsed: serde_json::Value =
            serde_json::from_str(&body).map_err(|_| "mint response unparseable")?;
        let token = parsed
            .get("token")
            .and_then(|v| v.as_str())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| "mint response carried no token".to_string())?
            .to_string();
        self.minted.push((app, owner.to_string(), token.clone()));
        Ok(token)
    }
}

impl TokenProvider for FleetTokenProvider<'_> {
    fn token_for(&mut self, source: &TokenSource) -> Result<String, String> {
        match source {
            TokenSource::Personal => {
                let hosts = self.resolve_named(PERSONAL_SECRET_NAME)?;
                parse_hosts_yml_token(&hosts).ok_or_else(|| {
                    format!("secret '{PERSONAL_SECRET_NAME}' carries no oauth_token")
                })
            }
            TokenSource::AppInstallation { app, owner } => {
                self.mint_installation_token(*app, owner)
            }
        }
    }
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

/// Run the real `gh` once, under `token`, capturing output for
/// classification. stdin is inherited so prompts behave; stdout and
/// stderr are piped (see the module header for why).
fn run_gh_once(gh_bin: &str, args: &[String], token: &str) -> std::io::Result<Attempt> {
    let out = Command::new(gh_bin)
        .args(args)
        .env("GH_TOKEN", token)
        .stdin(Stdio::inherit())
        .output()?;
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
        let attempt = match run_gh_once(gh_bin, args, &token) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("fleet gh: failed to run '{gh_bin}': {e}");
                return EXIT_UNRESOLVED;
            }
        };
        if attempt.code == 0 {
            emit(&attempt);
            return EXIT_OK;
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
                        FailureClass::Other => "error on a read-only call",
                    }
                );
            }
            last = Some(attempt);
            continue;
        }
        emit(&attempt);
        return attempt.code;
    }
    // No attempt produced output: either every source failed to
    // resolve, or the last failover had no successor. Prefer showing
    // the caller the real gh failure over our own summary.
    if let Some(attempt) = last {
        let code = attempt.code;
        emit(&attempt);
        return code;
    }
    for e in &provider_errors {
        eprintln!("fleet gh: {e}");
    }
    eprintln!("fleet gh: no token source could be resolved");
    EXIT_UNRESOLVED
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

/// `fleet gh tiers`: probe every rotation source and print its
/// core and GraphQL rate-limit headroom. Labels and numbers only —
/// token values never leave the provider. Exit 0 when every source
/// answers, EXIT_UNRESOLVED when any source is down.
fn run_tiers(config: &Config) -> i32 {
    let gh_bin = std::env::var("FLEET_GH_BIN").unwrap_or_else(|_| "gh".to_string());
    let sources = rotation(&[]);
    let mut provider = FleetTokenProvider::new(config);
    let mut any_down = false;
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
                println!("{label}\tDOWN\tfailed to run '{gh_bin}': {e}");
                any_down = true;
            }
        }
    }
    if any_down { EXIT_UNRESOLVED } else { EXIT_OK }
}

pub fn run(config: &Config, args: &[String]) -> i32 {
    if args.is_empty()
        || args.first().is_some_and(|a| a == "--help" || a == "-h") && args.len() == 1
    {
        println!("{USAGE}");
        return EXIT_OK;
    }
    // `tiers` is a shadow subcommand: real `gh` has no such verb,
    // so intercepting it cannot shadow a passthrough call.
    if args.first().is_some_and(|a| a == "tiers") {
        return run_tiers(config);
    }
    let gh_bin = std::env::var("FLEET_GH_BIN").unwrap_or_else(|_| "gh".to_string());
    let sources = rotation(args);
    let trace = std::env::var("FLEET_GH_TRACE").is_ok_and(|v| v == "1");
    let mut provider = FleetTokenProvider::new(config);
    run_with_provider(&mut provider, &gh_bin, args, &sources, trace)
}

// ---------------------------------------------------------------------------
// Tests. Every test names its oracle (fleet testing doctrine,
// code 2026-11-04): the fixed external truth the assertion is
// checked against.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
        assert!(!may_retry(FailureClass::Other, false, true));
        assert!(may_retry(FailureClass::Other, true, true));
        assert!(!may_retry(FailureClass::RateLimit, false, false));
        assert!(!may_retry(FailureClass::Other, true, false));
    }

    // oracle: `gh <verb> --help` — list/view/status surfaces mutate
    // nothing; create/comment/edit surfaces mutate; `api` mutates
    // only with field flags or a non-GET method.
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
        assert!(read(&["api", "graphql", "-f", "query={viewer{login}}"]) == false);
        assert!(read(&["api", "user", "--method", "GET"]));
        assert!(!read(&["api", "user", "--method", "PATCH"]));
        assert!(!read(&["issue", "comment", "1", "--body", "x"]));
        assert!(!read(&["issue", "create", "--title", "x"]));
        assert!(!read(&["project", "item-edit", "--id", "x"]));
        assert!(!read(&["pr", "merge", "1"]));
        assert!(!read(&["frobnicate", "list"]));
    }

    // oracle: code#8887's rotation definition as extended by Drew
    // 2026-11-12 — personal first, then the installations of all
    // three fleet Apps (fleet: 3 owners, developing.today: 2,
    // backup: 1); a call naming an owner leads each App leg with
    // that owner's installation where the App has one.
    #[test]
    fn rotation_order_and_owner_hint() {
        let args: Vec<String> = ["api", "repos/developing-today/code"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(owner_hint(&args).as_deref(), Some("developing-today"));
        let order = rotation_with(&args, None, &Owners::default());
        assert_eq!(order[0], TokenSource::Personal);
        assert_eq!(order[1], TokenSource::app(GhApp::Fleet, "developing-today"));
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
            order[1],
            TokenSource::app(GhApp::Fleet, "developing-today-forks")
        );

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
        assert_eq!(
            TokenSource::app(GhApp::DevToday, "dezren39").label(),
            "devtoday:dezren39"
        );
        assert_eq!(
            TokenSource::app(GhApp::Fleet, "dezren39").label(),
            "app:dezren39"
        );
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
             echo 'fake gh output'\n\
             exit 0\n",
            log = log.display()
        );
        std::fs::write(&script, script_text).unwrap();
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
}
