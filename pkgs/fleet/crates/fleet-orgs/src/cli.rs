//! `fleet orgs`: `list` reads the Turso registry, `sync` refreshes it from
//! each App's JWT. Configuration: `FLEET_ORGS_TURSO_URL` (the database's
//! HTTP URL) and the token secret `turso.fleet-orgs-token`.

use crate::github::HttpGitHub;
use crate::jwt::app_jwt;
use crate::list::{self, Filter};
use crate::sql::Sql;
use crate::sync::{AppJwt, sync};
use crate::turso::Turso;
use fleet_config::Config;
use fleet_secrets::Resolver;
use std::time::{SystemTime, UNIX_EPOCH};

const EXIT_OK: i32 = 0;
const EXIT_USAGE: i32 = 2;
const EXIT_UNAVAILABLE: i32 = 69;

pub const ORGS_USAGE: &str = "\
Usage: fleet orgs list [--app SLUG] [--org LOGIN] [--permission NAME[=LEVEL]] [--json]
       fleet orgs sync

list: the stored registry, one row per installation (org, App,
installation id, repository selection, permissions) and the last
successful sync time. --app matches the App's GitHub slug, --org the
account login, --permission a permission name (any level) or NAME=LEVEL.
--json prints the same rows as JSON.

sync: records a sync run, then for each fleet App (app, devtoday,
backup) reads the App's own GET /app and GET /app/installations with
its JWT, every page, and upserts orgs, Apps and installations. An
installation GitHub no longer lists is removed. OAuth-token org reads
are never used for coverage.

Store: FLEET_ORGS_TURSO_URL, token secret turso.fleet-orgs-token.";

/// The fleet's Apps by fleet slug, as `gh`'s rotation names them, with
/// the secret holding each App's private key and, if not public, its ID.
const APPS: [(&str, &str, Option<&str>); 3] = [
    ("app", "github-app.developing-today-fleet.private-key", None),
    (
        "devtoday",
        "kaneo.github-app-private-key",
        Some("kaneo.github-app-id"),
    ),
    (
        "backup",
        "github-app.backup-private-key",
        Some("github-app.backup-app-id"),
    ),
];
const FLEET_APP_ID: &str = "5612056";
const TOKEN_SECRET: &str = "turso.fleet-orgs-token";

pub fn run(config: &Config, args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("list") => match list_cmd(config, &args[1..]) {
            Ok(out) => {
                print!("{out}");
                EXIT_OK
            }
            Err(Failure::Usage(m)) => usage(&m),
            Err(Failure::Unavailable(m)) => fail(EXIT_UNAVAILABLE, &m),
        },
        Some("sync") if args.len() == 1 => match sync_cmd(config) {
            Ok(line) => {
                println!("{line}");
                EXIT_OK
            }
            Err(Failure::Usage(m)) => usage(&m),
            Err(Failure::Unavailable(m)) => fail(EXIT_UNAVAILABLE, &m),
        },
        Some("--help") | Some("-h") | None => {
            println!("{ORGS_USAGE}");
            EXIT_OK
        }
        Some(other) => usage(&format!("unknown orgs command {other:?}")),
    }
}

enum Failure {
    Usage(String),
    Unavailable(String),
}

fn usage(msg: &str) -> i32 {
    eprintln!("error: {msg}\n\n{ORGS_USAGE}");
    EXIT_USAGE
}

fn fail(code: i32, msg: &str) -> i32 {
    eprintln!("error: {msg}");
    code
}

fn store(config: &Config) -> Result<Turso, Failure> {
    let url = std::env::var("FLEET_ORGS_TURSO_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .ok_or_else(|| Failure::Unavailable("FLEET_ORGS_TURSO_URL is not set".into()))?;
    let token = resolve(config, TOKEN_SECRET)?;
    Ok(Turso::new(&url, &token))
}

fn resolve(config: &Config, name: &str) -> Result<String, Failure> {
    Resolver::default_chain(config)
        .resolve(name)
        .map(|v| v.expose().to_string())
        .map_err(|e| Failure::Unavailable(format!("secret '{name}' unresolved: {e}")))
}

fn list_cmd(config: &Config, args: &[String]) -> Result<String, Failure> {
    let mut filter = Filter::default();
    let mut json = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |flag: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| Failure::Usage(format!("{flag} needs a value")))
        };
        match arg.as_str() {
            "--app" => filter.app = Some(value("--app")?),
            "--org" => filter.org = Some(value("--org")?),
            "--permission" => {
                let raw = value("--permission")?;
                filter.permission = Some(list::parse_permission(&raw).map_err(Failure::Usage)?);
            }
            "--json" => json = true,
            other => return Err(Failure::Usage(format!("unknown option {other:?}"))),
        }
    }
    let db = store(config)?;
    let registry = list::load(&db).map_err(Failure::Unavailable)?;
    let entries = list::filter(&registry.entries, &filter);
    if json {
        let body = list::render_json(&registry, &entries);
        Ok(format!(
            "{}\n",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        ))
    } else {
        Ok(list::render_table(&registry, &entries))
    }
}

fn sync_cmd(config: &Config) -> Result<String, Failure> {
    let db = store(config)?;
    let source = SecretJwts { config };
    let gh = HttpGitHub::default();
    let apps: Vec<&str> = APPS.iter().map(|(slug, _, _)| *slug).collect();
    let clock = || {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    };
    let report =
        sync(&db as &dyn Sql, &gh, &source, &apps, &clock).map_err(Failure::Unavailable)?;
    let parts: Vec<String> = report
        .apps
        .iter()
        .map(|(slug, n)| format!("{slug}={n}"))
        .collect();
    Ok(format!(
        "sync run {} ok: {}",
        report.run_id,
        parts.join(" ")
    ))
}

/// App JWTs minted from the broker's App keys (and IDs, where not public).
struct SecretJwts<'a> {
    config: &'a Config,
}

impl AppJwt for SecretJwts<'_> {
    fn jwt(&self, app: &str) -> Result<String, String> {
        let (_, key_name, id_name) = APPS
            .iter()
            .find(|(slug, _, _)| *slug == app)
            .ok_or_else(|| format!("unknown fleet app {app:?}"))?;
        let app_id = match id_name {
            None => std::env::var("FLEET_GH_APP_ID")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| FLEET_APP_ID.to_string()),
            Some(name) => resolve(self.config, name)
                .map_err(|_| format!("app id secret for {app} unresolved"))?
                .trim()
                .to_string(),
        };
        let key = resolve(self.config, key_name)
            .map_err(|_| format!("app key secret for {app} unresolved"))?;
        app_jwt(&app_id, &key)
    }
}
