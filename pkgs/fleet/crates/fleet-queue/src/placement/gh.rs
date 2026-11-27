//! placement::gh — GitHub side-effect surface (#16255).
//!
//! Split from placement.rs for #16255 (production-line guard):
//! a sequential, behavior-preserving module split. The parent
//! module re-exports `Gh` (and `RealGh` crate-internally), so all
//! paths used elsewhere (`&dyn Gh`, `RealGh::new()`, adapter
//! `use super::*`) are unchanged.

use super::*;

// ------------------------------------------------------- side-effect surface

/// The boundary between the pure placement logic and the world:
/// the GitHub API (as the App), the fleet-paperwork token flow, and
/// the `gh codespace` surface. Production uses [`RealGh`]; tests
/// substitute a fake.
pub trait Gh {
    /// An installation token for the repo's owner via the
    /// fleet-paperwork helper, or None when the app does not cover
    /// the repo / the helper is unavailable.
    fn mint_token(&self, repo: &str) -> Option<String>;
    /// Like the reference's app_token(): minting is mandatory on the
    /// dispatch path — a surface the app cannot reach is ineligible,
    /// loudly.
    fn app_token(&self, repo: &str) -> R<String> {
        self.mint_token(repo).ok_or_else(|| {
            Die(format!(
                "app token mint failed for {repo} (app does not cover it, \
                 or the fleet-paperwork helper is unavailable) — surface \
                 is ineligible for app dispatch until covered"
            ))
        })
    }
    /// The signed-in user's own token, for a surface whose registry
    /// entry says `"dispatch_as": "user"`. Dispatching a workflow with
    /// a token the fleet already holds is a routine operation
    /// (FDR-0013), so a surface the App is not installed on is still
    /// usable. None where no user token is available.
    fn user_token(&self) -> Option<String> {
        None
    }
    fn api(&self, method: &str, path: &str, token: &str, body: Option<&Value>) -> R<(u16, Value)>;
    /// `gh codespace list --json name,repository,state`, parsed.
    fn codespace_list(&self) -> R<Vec<Value>>;
    /// `gh codespace ssh -c NAME -- COMMAND`; returns the exit code.
    fn codespace_ssh(&self, name: &str, command: &str) -> R<i32>;
    /// Like codespace_ssh but also returns trimmed stdout (used to
    /// validate the reused codespace's checked-out revision).
    fn codespace_ssh_capture(&self, name: &str, command: &str) -> R<(i32, String)> {
        Ok((self.codespace_ssh(name, command)?, String::new()))
    }
    fn sleep_secs(&self, secs: u64) {
        std::thread::sleep(Duration::from_secs(secs));
    }
    fn now_epoch(&self) -> i64 {
        now_epoch()
    }
}

pub(crate) struct RealGh {
    agent: ureq::Agent,
    tokens: RefCell<HashMap<String, String>>,
    /// The user's token, looked up at most once per run: a miss is
    /// remembered too, so a cascade over several `dispatch_as: user`
    /// surfaces does not repeat a slow lookup.
    user: RefCell<Option<Option<String>>>,
}

/// The fleet-secrets name of the `gh` hosts file (the personal leg
/// `fleet gh` uses).
const USER_HOSTS_SECRET: &str = "github-cli.hosts-yml";

/// The `oauth_token` value in gh's hosts.yml content. Line-shaped on
/// purpose, like `fleet gh`'s own reader: the file is a small
/// fixed-shape config.
pub(crate) fn hosts_yml_token(hosts_yml: &str) -> Option<String> {
    hosts_yml.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("oauth_token:")?;
        let value = rest.trim().trim_matches('"').trim_matches('\'');
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// The first usable token out of an ordered list of lookups; a
/// lookup that yields only whitespace counts as a miss.
pub(crate) fn first_token(lookups: &[&dyn Fn() -> Option<String>]) -> Option<String> {
    lookups
        .iter()
        .filter_map(|lookup| lookup())
        .map(|t| t.trim().to_string())
        .find(|t| !t.is_empty())
}

impl RealGh {
    pub(crate) fn new() -> Self {
        // Platform root store, not webpki-roots alone: hosts
        // behind a TLS-intercepting egress proxy present the
        // proxy's CA, which only the platform store carries (the
        // sandbox seat's dispatch died with UnknownIssuer before
        // this — code#13122; same fix as fleet's gh.rs App leg).
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(60)))
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .into();
        RealGh {
            agent,
            user: RefCell::new(None),
            tokens: RefCell::new(HashMap::new()),
        }
    }
}

pub(crate) fn paperwork_path() -> String {
    std::env::var("FLEET_PAPERWORK").unwrap_or_else(|_| {
        format!(
            "{}/git/code/infra/fleet-paperwork/fleet_paperwork.py",
            home_dir()
        )
    })
}

pub(crate) fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_default()
}

impl Gh for RealGh {
    /// Where the user's token comes from, cheapest first: the
    /// environment, the seat's own `gh` login, then the fleet secrets
    /// chain by name (the same hosts file `fleet gh` reads). The
    /// value goes into Authorization headers only.
    fn user_token(&self) -> Option<String> {
        if let Some(known) = self.user.borrow().as_ref() {
            return known.clone();
        }
        let from_env = || {
            ["GH_TOKEN", "GITHUB_TOKEN"]
                .iter()
                .filter_map(|k| std::env::var(k).ok())
                .find(|v| !v.trim().is_empty())
        };
        let from_gh = || {
            let out = std::process::Command::new("gh")
                .args(["auth", "token"])
                .stderr(std::process::Stdio::null())
                .output()
                .ok()?;
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).to_string())
        };
        let from_secrets = || {
            let config = fleet_config::Config::load().unwrap_or_default();
            let hosts = fleet_secrets::Resolver::default_chain(&config)
                .resolve(USER_HOSTS_SECRET)
                .ok()?;
            hosts_yml_token(hosts.expose())
        };
        let found = first_token(&[&from_env, &from_gh, &from_secrets]);
        *self.user.borrow_mut() = Some(found.clone());
        found
    }

    fn mint_token(&self, repo: &str) -> Option<String> {
        if let Some(tok) = self.tokens.borrow().get(repo) {
            return Some(tok.clone());
        }
        let path = paperwork_path();
        if !std::path::Path::new(&path).exists() {
            return None;
        }
        let out = std::process::Command::new("python3")
            .args([&path, "token", "--repo", repo])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let tok = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if tok.is_empty() {
            return None;
        }
        self.tokens
            .borrow_mut()
            .insert(repo.to_string(), tok.clone());
        Some(tok)
    }

    fn api(&self, method: &str, path: &str, token: &str, body: Option<&Value>) -> R<(u16, Value)> {
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("https://api.github.com{path}")
        };
        let auth = format!("Bearer {token}");
        let mut resp = if method == "POST" {
            self.agent
                .post(&url)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .header("Authorization", &auth)
                .send_json(body.cloned().unwrap_or_else(|| json!({})))
                .map_err(|e| Die(format!("github api {method} {path}: {e}")))?
        } else {
            self.agent
                .get(&url)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .header("Authorization", &auth)
                .call()
                .map_err(|e| Die(format!("github api {method} {path}: {e}")))?
        };
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        let payload = if text.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&text).unwrap_or_else(|_| json!({ "message": text }))
        };
        Ok((status, payload))
    }

    fn codespace_list(&self) -> R<Vec<Value>> {
        let out = std::process::Command::new("gh")
            .args(["codespace", "list", "--json", "name,repository,state"])
            .output()
            .map_err(|e| Die(format!("gh codespace list: {e}")))?;
        if !out.status.success() {
            return die(format!(
                "gh codespace list failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if text.is_empty() {
            return Ok(vec![]);
        }
        serde_json::from_str(&text).map_err(|e| Die(format!("gh codespace list json: {e}")))
    }

    fn codespace_ssh(&self, name: &str, command: &str) -> R<i32> {
        let out = std::process::Command::new("gh")
            .args(["codespace", "ssh", "-c", name, "--", command])
            .output()
            .map_err(|e| Die(format!("gh codespace ssh: {e}")))?;
        Ok(out.status.code().unwrap_or(-1))
    }

    fn codespace_ssh_capture(&self, name: &str, command: &str) -> R<(i32, String)> {
        let out = std::process::Command::new("gh")
            .args(["codespace", "ssh", "-c", name, "--", command])
            .output()
            .map_err(|e| Die(format!("gh codespace ssh: {e}")))?;
        Ok((
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
        ))
    }
}
