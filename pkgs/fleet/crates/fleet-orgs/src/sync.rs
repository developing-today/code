//! One sync run: each App's own metadata and installations, written into
//! the registry. A run is recorded in `sync_runs` before any write and
//! closed as `ok` or `failed`; a failed App is never pruned.

use crate::schema::{
    FINISH_RUN, PRUNE_STALE, START_RUN, UPSERT_APP, UPSERT_INSTALLATION, UPSERT_ORG, migrate,
};
use crate::sql::Sql;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub struct AppMeta {
    pub slug: String,
    pub app_id: i64,
    pub name: String,
    pub owner: String,
    pub permissions: Value,
    pub events: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Installation {
    pub installation_id: i64,
    pub account_login: String,
    pub account_type: String,
    pub repository_selection: String,
    pub permissions: Value,
}

/// The App-scoped reads. The live client is [`crate::github::HttpGitHub`].
pub trait GitHubApi {
    fn app(&self, jwt: &str) -> Result<AppMeta, String>;
    fn installations(&self, jwt: &str) -> Result<Vec<Installation>, String>;
}

/// A short-lived App JWT for one App, named by its fleet slug.
pub trait AppJwt {
    fn jwt(&self, app: &str) -> Result<String, String>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct SyncReport {
    pub run_id: i64,
    /// `(app slug, installations written)` per App, in sync order.
    pub apps: Vec<(String, usize)>,
}

pub fn sync(
    db: &dyn Sql,
    gh: &dyn GitHubApi,
    jwts: &dyn AppJwt,
    apps: &[&str],
    clock: &dyn Fn() -> i64,
) -> Result<SyncReport, String> {
    migrate(db)?;
    let run_id = start_run(db, clock())?;
    match sync_apps(db, gh, jwts, apps, clock, run_id) {
        Ok(report) => {
            let total: usize = report.apps.iter().map(|(_, n)| n).sum();
            finish_run(db, run_id, clock(), "ok", Some(total as i64), None)?;
            Ok(report)
        }
        Err(e) => {
            finish_run(db, run_id, clock(), "failed", None, Some(&e))?;
            Err(e)
        }
    }
}

fn sync_apps(
    db: &dyn Sql,
    gh: &dyn GitHubApi,
    jwts: &dyn AppJwt,
    apps: &[&str],
    clock: &dyn Fn() -> i64,
    run_id: i64,
) -> Result<SyncReport, String> {
    let mut report = SyncReport {
        run_id,
        apps: Vec::new(),
    };
    for app in apps {
        let jwt = jwts.jwt(app)?;
        let meta = gh.app(&jwt)?;
        let installs = gh.installations(&jwt)?;
        let now = clock();
        write_app(db, &meta, now)?;
        for inst in &installs {
            write_installation(db, &meta.slug, inst, run_id, now)?;
        }
        db.execute(PRUNE_STALE, &[json!(meta.slug), json!(run_id)])?;
        report.apps.push((meta.slug, installs.len()));
    }
    Ok(report)
}

fn start_run(db: &dyn Sql, now: i64) -> Result<i64, String> {
    let rows = db.execute(START_RUN, &[json!(now)])?;
    rows.rows
        .first()
        .and_then(|r| r.first())
        .and_then(Value::as_i64)
        .ok_or_else(|| "sync_runs: no id returned".to_string())
}

fn finish_run(
    db: &dyn Sql,
    run_id: i64,
    finished: i64,
    status: &str,
    installations: Option<i64>,
    error: Option<&str>,
) -> Result<(), String> {
    db.execute(
        FINISH_RUN,
        &[
            json!(finished),
            json!(status),
            installations.map_or(Value::Null, Value::from),
            error.map_or(Value::Null, Value::from),
            json!(run_id),
        ],
    )?;
    Ok(())
}

fn write_app(db: &dyn Sql, meta: &AppMeta, now: i64) -> Result<(), String> {
    db.execute(
        UPSERT_APP,
        &[
            json!(meta.slug),
            json!(meta.app_id),
            json!(meta.name),
            json!(meta.owner),
            json!(meta.permissions.to_string()),
            json!(meta.events.to_string()),
            json!(now),
        ],
    )?;
    Ok(())
}

fn write_installation(
    db: &dyn Sql,
    app: &str,
    inst: &Installation,
    run_id: i64,
    now: i64,
) -> Result<(), String> {
    db.execute(
        UPSERT_INSTALLATION,
        &[
            json!(app),
            json!(inst.installation_id),
            json!(inst.account_login),
            json!(inst.account_type),
            json!(inst.repository_selection),
            json!(inst.permissions.to_string()),
            json!(run_id),
            json!(now),
        ],
    )?;
    if inst.account_type == "Organization" {
        db.execute(UPSERT_ORG, &[json!(inst.account_login), json!(now)])?;
    }
    Ok(())
}
