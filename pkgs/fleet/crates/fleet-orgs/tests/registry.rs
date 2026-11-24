//! Offline registry tests: the real schema and SQL against in-memory
//! SQLite, a fake GitHub, and fake App JWTs. No network.

use fleet_orgs::github::{collect_pages, parse_installation};
use fleet_orgs::list::{self, Filter};
use fleet_orgs::sql::{Rows, Sql};
use fleet_orgs::sync::{AppJwt, AppMeta, GitHubApi, Installation, sync};
use proptest::prelude::*;
use rusqlite::{Connection, types::ValueRef};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

struct Sqlite(RefCell<Connection>);

impl Sqlite {
    fn memory() -> Sqlite {
        Sqlite(RefCell::new(Connection::open_in_memory().unwrap()))
    }
}

impl Sql for Sqlite {
    fn execute(&self, sql: &str, args: &[Value]) -> Result<Rows, String> {
        let conn = self.0.borrow();
        let params: Vec<rusqlite::types::Value> = args
            .iter()
            .map(|a| match a {
                Value::Null => rusqlite::types::Value::Null,
                Value::Number(n) => rusqlite::types::Value::Integer(n.as_i64().unwrap()),
                Value::String(s) => rusqlite::types::Value::Text(s.clone()),
                other => rusqlite::types::Value::Text(other.to_string()),
            })
            .collect();
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let cols = stmt.column_count();
        let mut out = stmt
            .query(rusqlite::params_from_iter(params.iter()))
            .map_err(|e| e.to_string())?;
        let mut rows = Vec::new();
        while let Some(row) = out.next().map_err(|e| e.to_string())? {
            let mut cells = Vec::new();
            for i in 0..cols {
                cells.push(match row.get_ref(i).map_err(|e| e.to_string())? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) => json!(n),
                    ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                    _ => Value::Null,
                });
            }
            rows.push(cells);
        }
        Ok(Rows { rows })
    }
}

/// Fake GitHub: per App, its metadata and its installations, served in
/// pages of `page_size` linked by `next`. Keys are App JWT strings.
#[derive(Default)]
struct FakeGitHub {
    apps: BTreeMap<String, (AppMeta, Vec<Installation>)>,
    fail_installations: RefCell<Vec<String>>,
}

impl GitHubApi for FakeGitHub {
    fn app(&self, jwt: &str) -> Result<AppMeta, String> {
        self.apps
            .get(jwt)
            .map(|(m, _)| m.clone())
            .ok_or_else(|| format!("no app for {jwt}"))
    }

    fn installations(&self, jwt: &str) -> Result<Vec<Installation>, String> {
        if self.fail_installations.borrow().iter().any(|j| j == jwt) {
            return Err("installations: HTTP 500".into());
        }
        let (_, installs) = self.apps.get(jwt).ok_or("no app")?;
        let pages: Vec<Value> = installs
            .chunks(2)
            .map(|c| Value::Array(c.iter().map(install_json).collect()))
            .collect();
        let pages = RefCell::new(pages);
        let fetch = |url: &str| -> Result<(Value, Option<String>), String> {
            let i: usize = url.rsplit('=').next().unwrap().parse().unwrap();
            let pages = pages.borrow();
            let page = pages.get(i - 1).cloned().unwrap_or(json!([]));
            let next = (i < pages.len()).then(|| format!("https://x/?page={}", i + 1));
            Ok((page, next))
        };
        let entries = collect_pages("https://x/?page=1", fetch)?;
        entries
            .iter()
            .map(|e| parse_installation(e).ok_or_else(|| "bad".to_string()))
            .collect()
    }
}

fn install_json(i: &Installation) -> Value {
    json!({
        "id": i.installation_id,
        "account": {"login": i.account_login, "type": i.account_type},
        "repository_selection": i.repository_selection,
        "permissions": i.permissions,
    })
}

struct Jwts;
impl AppJwt for Jwts {
    fn jwt(&self, app: &str) -> Result<String, String> {
        Ok(app.to_string())
    }
}

fn inst(id: i64, login: &str, ty: &str, perms: Value) -> Installation {
    Installation {
        installation_id: id,
        account_login: login.into(),
        account_type: ty.into(),
        repository_selection: "all".into(),
        permissions: perms,
    }
}

fn meta(slug: &str, id: i64) -> AppMeta {
    AppMeta {
        slug: slug.into(),
        app_id: id,
        name: slug.into(),
        owner: "developing-today".into(),
        permissions: json!({"contents": "write", "issues": "read"}),
        events: json!(["push"]),
    }
}

fn fleet() -> FakeGitHub {
    let mut gh = FakeGitHub::default();
    gh.apps.insert(
        "app".into(),
        (
            meta("developing-today-fleet", 5612056),
            vec![
                inst(1, "acme", "Organization", json!({"contents": "write"})),
                inst(2, "beta", "Organization", json!({"issues": "read"})),
                inst(3, "dez", "User", json!({"contents": "read"})),
            ],
        ),
    );
    gh.apps.insert(
        "backup".into(),
        (
            meta("developing-today-backup", 99),
            vec![inst(7, "acme", "Organization", json!({"contents": "read"}))],
        ),
    );
    gh
}

fn clock() -> impl Fn() -> i64 {
    let t = Cell::new(1_700_000_000);
    move || {
        t.set(t.get() + 1);
        t.get()
    }
}

#[test]
fn sync_writes_apps_orgs_and_installations_from_all_pages() {
    let db = Sqlite::memory();
    let gh = fleet();
    let report = sync(&db, &gh, &Jwts, &["app", "backup"], &clock()).unwrap();
    assert_eq!(
        report.apps,
        vec![
            ("developing-today-fleet".into(), 3),
            ("developing-today-backup".into(), 1)
        ]
    );

    let reg = list::load(&db).unwrap();
    assert_eq!(reg.entries.len(), 4);
    assert!(reg.last_sync.is_some());

    let app = db
        .execute(
            "SELECT owner FROM apps WHERE slug = 'developing-today-fleet'",
            &[],
        )
        .unwrap();
    assert_eq!(app.rows, vec![vec![json!("developing-today")]]);
    let org_columns = db.execute("PRAGMA table_info(orgs)", &[]).unwrap();
    let names: Vec<&str> = org_columns
        .rows
        .iter()
        .filter_map(|r| r.get(1).and_then(Value::as_str))
        .collect();
    for expected in ["login", "role", "repo_count", "verified_at"] {
        assert!(
            names.contains(&expected),
            "orgs schema must include {expected}"
        );
    }

    let orgs = db
        .execute("SELECT login FROM orgs ORDER BY login", &[])
        .unwrap();
    assert_eq!(orgs.rows, vec![vec![json!("acme")], vec![json!("beta")]]);
}

#[test]
fn a_resync_is_idempotent() {
    let db = Sqlite::memory();
    let gh = fleet();
    sync(&db, &gh, &Jwts, &["app", "backup"], &clock()).unwrap();
    let before = list::load(&db).unwrap().entries;
    sync(&db, &gh, &Jwts, &["app", "backup"], &clock()).unwrap();
    let after = list::load(&db).unwrap();
    assert_eq!(before, after.entries);
    let runs = db
        .execute("SELECT COUNT(*), SUM(status = 'ok') FROM sync_runs", &[])
        .unwrap();
    assert_eq!(runs.rows, vec![vec![json!(2), json!(2)]]);
}

#[test]
fn same_app_and_account_upsert_updates_installation_and_verified_time() {
    let db = Sqlite::memory();
    let mut gh = FakeGitHub::default();
    gh.apps.insert(
        "app".into(),
        (
            meta("fleet", 1),
            vec![inst(
                10,
                "acme",
                "Organization",
                json!({"contents": "read"}),
            )],
        ),
    );
    let time = clock();
    sync(&db, &gh, &Jwts, &["app"], &time).unwrap();
    let prior = db.execute(
        "SELECT MAX(verified_at) FROM installations WHERE app_slug = 'fleet' AND account_login = 'acme'",
        &[],
    ).unwrap().rows[0][0].as_i64().unwrap();
    gh.apps.get_mut("app").unwrap().1 = vec![inst(
        11,
        "acme",
        "Organization",
        json!({"contents": "write"}),
    )];
    sync(&db, &gh, &Jwts, &["app"], &time).unwrap();

    let rows = db.execute(
        "SELECT COUNT(*), MAX(installation_id), MAX(verified_at) FROM installations WHERE app_slug = 'fleet' AND account_login = 'acme'",
        &[],
    ).unwrap();
    assert_eq!(
        rows.rows[0][0],
        json!(1),
        "oracle: one (App, account) row remains"
    );
    assert_eq!(
        rows.rows[0][1],
        json!(11),
        "oracle: the latest installation id is stored"
    );
    assert!(
        rows.rows[0][2].as_i64().unwrap() > prior,
        "oracle: verified_at advances on upsert"
    );
}

#[test]
fn an_installation_removed_on_github_is_pruned_on_the_next_run() {
    let db = Sqlite::memory();
    let mut gh = fleet();
    sync(&db, &gh, &Jwts, &["app"], &clock()).unwrap();
    gh.apps
        .get_mut("app")
        .unwrap()
        .1
        .retain(|i| i.installation_id != 2);
    sync(&db, &gh, &Jwts, &["app"], &clock()).unwrap();
    let ids: Vec<i64> = list::load(&db)
        .unwrap()
        .entries
        .iter()
        .map(|e| e.installation_id)
        .collect();
    assert_eq!(ids, vec![1, 3]);
}

#[test]
fn a_failed_app_is_recorded_as_failed_and_its_rows_are_kept() {
    let db = Sqlite::memory();
    let gh = fleet();
    sync(&db, &gh, &Jwts, &["app"], &clock()).unwrap();
    gh.fail_installations.borrow_mut().push("backup".into());
    let err = sync(&db, &gh, &Jwts, &["app", "backup"], &clock());
    assert!(err.is_err());
    let runs = db
        .execute("SELECT status, error FROM sync_runs ORDER BY id", &[])
        .unwrap();
    assert_eq!(runs.rows[1][0], json!("failed"));
    assert!(runs.rows[1][1].as_str().unwrap().contains("HTTP 500"));
    let reg = list::load(&db).unwrap();
    assert_eq!(
        reg.entries.len(),
        3,
        "the failed App keeps its last good rows"
    );
}

#[test]
fn last_sync_reflects_only_successful_runs() {
    let db = Sqlite::memory();
    let gh = fleet();
    sync(&db, &gh, &Jwts, &["app"], &clock()).unwrap();
    let ok_at = list::load(&db).unwrap().last_sync;
    gh.fail_installations.borrow_mut().push("app".into());
    let _ = sync(&db, &gh, &Jwts, &["app"], &clock());
    assert_eq!(list::load(&db).unwrap().last_sync, ok_at);
}

#[test]
fn list_filters_and_renders_over_the_stored_registry() {
    let db = Sqlite::memory();
    sync(&db, &fleet(), &Jwts, &["app", "backup"], &clock()).unwrap();
    let reg = list::load(&db).unwrap();

    let writers = list::filter(
        &reg.entries,
        &Filter {
            permission: Some(list::parse_permission("contents=write").unwrap()),
            ..Default::default()
        },
    );
    assert_eq!(writers.len(), 1);
    assert_eq!(writers[0].org, "acme");

    let backup = list::filter(
        &reg.entries,
        &Filter {
            app: Some("developing-today-backup".into()),
            ..Default::default()
        },
    );
    assert_eq!(backup.len(), 1);

    let table = list::render_table(&reg, &writers);
    assert!(table.contains("ORG"));
    assert!(table.contains("contents:write"));
    assert!(table.contains("last sync: "));

    let json = list::render_json(&reg, &backup);
    assert_eq!(json["installations"][0]["org"], "acme");
    assert!(json["last_sync"].is_string());
}

proptest! {
    #[test]
    fn any_resync_leaves_the_registry_as_the_last_run_saw_it(
        ids in proptest::collection::btree_set(1i64..50, 0..12),
        second in proptest::collection::btree_set(1i64..50, 0..12),
    ) {
        let db = Sqlite::memory();
        let mut gh = FakeGitHub::default();
        let make = |set: &std::collections::BTreeSet<i64>| -> Vec<Installation> {
            set.iter().map(|id| inst(*id, &format!("org{id}"), "Organization", json!({"contents": "read"}))).collect()
        };
        gh.apps.insert("app".into(), (meta("fleet", 1), make(&ids)));
        sync(&db, &gh, &Jwts, &["app"], &clock()).unwrap();
        gh.apps.get_mut("app").unwrap().1 = make(&second);
        sync(&db, &gh, &Jwts, &["app"], &clock()).unwrap();
        let mut got: Vec<i64> = list::load(&db).unwrap().entries.iter().map(|e| e.installation_id).collect();
        got.sort();
        let want: Vec<i64> = second.iter().copied().collect();
        prop_assert_eq!(got, want);
    }
}
