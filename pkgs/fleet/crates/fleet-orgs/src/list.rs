//! `fleet orgs list`: the registry read back, one row per installation,
//! with the filters and the last successful sync time.

use crate::sql::Sql;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub org: String,
    pub app: String,
    pub installation_id: i64,
    pub repository_selection: String,
    /// `(permission name, access level)`, sorted by name.
    pub permissions: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Registry {
    /// `finished_at` of the latest successful sync, Unix seconds.
    pub last_sync: Option<i64>,
    pub entries: Vec<Entry>,
}

/// `--permission NAME` matches any level; `NAME=LEVEL` matches that level.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filter {
    pub app: Option<String>,
    pub org: Option<String>,
    pub permission: Option<(String, Option<String>)>,
}

pub fn parse_permission(raw: &str) -> Result<(String, Option<String>), String> {
    match raw.split_once('=') {
        Some((name, level)) if !name.is_empty() && !level.is_empty() => {
            Ok((name.to_string(), Some(level.to_string())))
        }
        None if !raw.is_empty() => Ok((raw.to_string(), None)),
        _ => Err(format!("--permission {raw:?}: expected NAME or NAME=LEVEL")),
    }
}

pub fn load(db: &dyn Sql) -> Result<Registry, String> {
    let rows = db.execute(
        "SELECT i.account_login, i.app_slug, i.installation_id, i.repository_selection,
                i.permissions,
                (SELECT MAX(finished_at) FROM sync_runs WHERE status = 'ok')
         FROM installations i
         ORDER BY i.account_login, i.app_slug",
        &[],
    )?;
    let mut last_sync = None;
    let mut entries = Vec::new();
    for row in rows.rows {
        let text = |i: usize| row.get(i).and_then(Value::as_str).unwrap_or("").to_string();
        let perms: Value = serde_json::from_str(&text(4)).unwrap_or(Value::Null);
        let mut permissions: Vec<(String, String)> = perms
            .as_object()
            .map(|o| {
                o.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                    .collect()
            })
            .unwrap_or_default();
        permissions.sort();
        last_sync = last_sync.or(row.get(5).and_then(Value::as_i64));
        entries.push(Entry {
            org: text(0),
            app: text(1),
            installation_id: row.get(2).and_then(Value::as_i64).unwrap_or(0),
            repository_selection: text(3),
            permissions,
        });
    }
    Ok(Registry { last_sync, entries })
}

pub fn filter(entries: &[Entry], f: &Filter) -> Vec<Entry> {
    entries
        .iter()
        .filter(|e| f.app.as_ref().is_none_or(|a| &e.app == a))
        .filter(|e| f.org.as_ref().is_none_or(|o| &e.org == o))
        .filter(|e| match &f.permission {
            None => true,
            Some((name, level)) => e
                .permissions
                .iter()
                .any(|(k, v)| k == name && level.as_ref().is_none_or(|l| l == v)),
        })
        .cloned()
        .collect()
}

fn perms_text(perms: &[(String, String)]) -> String {
    if perms.is_empty() {
        return "-".to_string();
    }
    perms
        .iter()
        .map(|(k, v)| format!("{k}:{v}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn sync_text(last: Option<i64>) -> String {
    match last {
        Some(secs) => rfc3339(secs),
        None => "never".to_string(),
    }
}

pub fn render_table(reg: &Registry, entries: &[Entry]) -> String {
    let header = ["ORG", "APP", "INSTALLATION", "SELECTION", "PERMISSIONS"];
    let body: Vec<[String; 5]> = entries
        .iter()
        .map(|e| {
            [
                e.org.clone(),
                e.app.clone(),
                e.installation_id.to_string(),
                e.repository_selection.clone(),
                perms_text(&e.permissions),
            ]
        })
        .collect();
    let mut widths = header.map(str::len);
    for row in &body {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.len());
        }
    }
    let line = |cells: &[String]| {
        cells
            .iter()
            .zip(&widths)
            .map(|(c, w)| format!("{c:<w$}"))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    let mut out = String::new();
    out.push_str(&line(&header.map(str::to_string)));
    out.push('\n');
    for row in &body {
        out.push_str(&line(row));
        out.push('\n');
    }
    out.push_str(&format!(
        "{} installation(s); last sync: {}\n",
        entries.len(),
        sync_text(reg.last_sync)
    ));
    out
}

pub fn render_json(reg: &Registry, entries: &[Entry]) -> Value {
    json!({
        "last_sync": reg.last_sync.map(rfc3339),
        "installations": entries.iter().map(|e| json!({
            "org": e.org,
            "app": e.app,
            "installation_id": e.installation_id,
            "repository_selection": e.repository_selection,
            "permissions": e.permissions.iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect::<serde_json::Map<_, _>>(),
        })).collect::<Vec<_>>(),
    })
}

/// RFC 3339 UTC for Unix seconds (civil-from-days, no dependency).
pub fn rfc3339(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_known_instants() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(86_400 * 365), "1971-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn permission_spec_forms() {
        assert_eq!(
            parse_permission("contents").unwrap(),
            ("contents".into(), None)
        );
        assert_eq!(
            parse_permission("contents=write").unwrap(),
            ("contents".into(), Some("write".into()))
        );
        assert!(parse_permission("").is_err());
        assert!(parse_permission("=write").is_err());
        assert!(parse_permission("contents=").is_err());
    }

    fn entry(org: &str, app: &str, perms: &[(&str, &str)]) -> Entry {
        Entry {
            org: org.into(),
            app: app.into(),
            installation_id: 1,
            repository_selection: "all".into(),
            permissions: perms
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn filters_compose() {
        let all = vec![
            entry("acme", "app", &[("contents", "write")]),
            entry("acme", "backup", &[("contents", "read")]),
            entry("beta", "app", &[("issues", "write")]),
        ];
        let by = |f: Filter| filter(&all, &f).len();
        assert_eq!(by(Filter::default()), 3);
        assert_eq!(
            by(Filter {
                app: Some("app".into()),
                ..Default::default()
            }),
            2
        );
        assert_eq!(
            by(Filter {
                org: Some("acme".into()),
                ..Default::default()
            }),
            2
        );
        assert_eq!(
            by(Filter {
                permission: Some(("contents".into(), Some("write".into()))),
                ..Default::default()
            }),
            1
        );
        assert_eq!(
            by(Filter {
                permission: Some(("contents".into(), None)),
                ..Default::default()
            }),
            2
        );
        assert_eq!(
            by(Filter {
                org: Some("acme".into()),
                app: Some("app".into()),
                permission: Some(("issues".into(), None)),
            }),
            0
        );
    }

    #[test]
    fn table_and_json_carry_last_sync() {
        let reg = Registry {
            last_sync: Some(0),
            entries: vec![entry("acme", "app", &[("contents", "write")])],
        };
        let table = render_table(&reg, &reg.entries);
        assert!(table.contains("contents:write"));
        assert!(table.ends_with("last sync: 1970-01-01T00:00:00Z\n"));
        let json = render_json(&reg, &reg.entries);
        assert_eq!(json["last_sync"], "1970-01-01T00:00:00Z");
        assert_eq!(json["installations"][0]["permissions"]["contents"], "write");
    }
}
