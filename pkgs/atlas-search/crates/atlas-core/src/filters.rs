//! Filters, sensitivity ceilings, and SQL construction — a faithful
//! port of the Python's `Filters` class and the branch/detail SQL
//! builders. SQL strings are assembled with the same fragment
//! concatenation the Python's f-strings use (including the double
//! space the empty Turso `sens_join` leaves in two statements): the
//! strings are the contract with the database, and the parity
//! fixtures record them byte-for-byte.

use serde_json::{Map, Value as Json};

use crate::error::{Error, Result};
use crate::types::{Stmt, Value};

/// The sensitivity ladder, least to most restricted (Python
/// `SENS_ORDER`). A ceiling admits its own level and everything
/// below it.
pub const SENS_ORDER: [&str; 4] = ["shareable", "personal", "local-only", "secret-bearing"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sensitivity {
    Shareable,
    Personal,
    LocalOnly,
    SecretBearing,
}

impl Sensitivity {
    pub fn from_name(name: &str) -> Option<Sensitivity> {
        match name {
            "shareable" => Some(Sensitivity::Shareable),
            "personal" => Some(Sensitivity::Personal),
            "local-only" => Some(Sensitivity::LocalOnly),
            "secret-bearing" => Some(Sensitivity::SecretBearing),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        SENS_ORDER[self as usize]
    }

    /// Every level a ceiling admits, in ladder order (the Python's
    /// `SENS_ORDER[:index+1]`).
    pub fn allowed(self) -> Vec<&'static str> {
        SENS_ORDER[..=self as usize].to_vec()
    }
}

impl Default for Sensitivity {
    /// The production default ceiling.
    fn default() -> Self {
        Sensitivity::Personal
    }
}

/// The per-backend SQL shape the builders parameterize on (the
/// Python's backend class attributes).
#[derive(Debug, Clone, Copy)]
pub struct BackendShape {
    pub sens_expr: &'static str,
    pub labels_table: &'static str,
    pub sens_join: &'static str,
}

pub const TURSO_SHAPE: BackendShape = BackendShape {
    sens_expr: "c.sensitivity",
    labels_table: "session_labels",
    sens_join: "",
};

pub const LOCAL_SHAPE: BackendShape = BackendShape {
    sens_expr: "cs.sensitivity",
    labels_table: "v_labels",
    sens_join: "LEFT JOIN v_chunk_sens cs ON cs.id = c.id",
};

/// The Python's `split_csv`: comma-split, trimmed, empties dropped.
pub fn split_csv(v: Option<&str>) -> Vec<String> {
    match v {
        None => Vec::new(),
        Some(s) => s
            .split(',')
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(str::to_string)
            .collect(),
    }
}

/// Classification + ceiling filters (the Python's `Filters`).
#[derive(Debug, Clone, Default)]
pub struct Filters {
    pub kinds: Vec<String>,
    pub topics: Vec<String>,
    pub projects: Vec<String>,
    pub clients: Vec<String>,
    pub max_sensitivity: Sensitivity,
    pub since: Option<String>,
    /// The `until` date as given (used by `describe` and validation).
    pub until: Option<String>,
    /// `until` + 1 day, the exclusive SQL bound.
    pub until_exclusive: Option<String>,
}

impl Filters {
    /// Build with the Python's validation (its `die()` texts on bad
    /// ceiling / date inputs). `until` is stored raw; the exclusive
    /// bound is computed from the parsed date.
    pub fn new(
        kinds: Vec<String>,
        topics: Vec<String>,
        projects: Vec<String>,
        clients: Vec<String>,
        max_sensitivity: &str,
        since: Option<String>,
        until: Option<String>,
    ) -> Result<Filters> {
        let ceiling = Sensitivity::from_name(max_sensitivity).ok_or_else(|| {
            Error::fatal(
                "--max-sensitivity must be one of \
                 ['shareable', 'personal', 'local-only', 'secret-bearing']",
            )
        })?;
        let mut until_exclusive = None;
        if let Some(u) = &until {
            let d = parse_iso_date(u)
                .ok_or_else(|| Error::fatal(format!("--until must be YYYY-MM-DD, got {u}")))?;
            // The exclusive bound is the following day, and the max
            // ISO date has none: CPython raises OverflowError adding
            // a day to date(9999, 12, 31). Computing it anyway would
            // bind "10000-01-01", a five-digit year whose
            // lexicographic order excludes every realistic
            // created_at (#5394).
            if d == (9999, 12, 31) {
                return Err(Error::fatal(format!(
                    "--until {u} is the last ISO date; it has no following day for the exclusive bound"
                )));
            }
            until_exclusive = Some(format_iso_date(add_one_day(d)));
        }
        // `since` is bound verbatim as the created_at >= parameter, and
        // created_at values are dashed ISO timestamps -- so, like `until`
        // above, it is normalized to the dashed form from the parsed
        // date. Storing the raw spelling would let the accepted basic
        // form (YYYYMMDD) reach the comparison unnormalized, where
        // lexicographic order silently inverts the window (#5378).
        let since = match &since {
            Some(s) => {
                let d = parse_iso_date(s)
                    .ok_or_else(|| Error::fatal(format!("--since must be YYYY-MM-DD, got {s}")))?;
                Some(format_iso_date(d))
            }
            None => None,
        };
        Ok(Filters {
            kinds,
            topics,
            projects,
            clients,
            max_sensitivity: ceiling,
            since,
            until,
            until_exclusive,
        })
    }

    pub fn allowed_sens(&self) -> Vec<&'static str> {
        self.max_sensitivity.allowed()
    }

    fn joins(&self, shape: BackendShape) -> String {
        format!(
            "JOIN messages m ON m.id = c.message_id \
             JOIN sessions s ON s.id = m.session_id \
             LEFT JOIN {} l ON l.session_id = s.id {}",
            shape.labels_table, shape.sens_join
        )
    }

    /// The Python's `Filters.where`: full WHERE fragment (ceiling
    /// always applied; NULL sensitivity never passes) + params.
    pub fn where_clause(&self, shape: BackendShape) -> (String, Vec<Value>) {
        let mut clauses: Vec<String> = Vec::new();
        let mut params: Vec<Value> = Vec::new();
        let allowed = self.allowed_sens();
        clauses.push(format!(
            "{} IN ({})",
            shape.sens_expr,
            placeholders(allowed.len())
        ));
        params.extend(allowed.iter().map(|s| Value::text(*s)));
        for (col, vals) in [("l.kind", &self.kinds), ("s.client", &self.clients)] {
            if !vals.is_empty() {
                clauses.push(format!("{col} IN ({})", placeholders(vals.len())));
                params.extend(vals.iter().map(|v| Value::text(v.clone())));
            }
        }
        for (col, vals) in [("l.topics", &self.topics), ("l.projects", &self.projects)] {
            for v in vals {
                clauses.push(format!("{col} LIKE ?"));
                params.push(Value::text(format!("%\"{v}\"%")));
            }
        }
        if let Some(s) = &self.since {
            clauses.push("s.created_at >= ?".to_string());
            params.push(Value::text(s.clone()));
        }
        if let Some(u) = &self.until_exclusive {
            clauses.push("s.created_at < ?".to_string());
            params.push(Value::text(u.clone()));
        }
        (clauses.join(" AND "), params)
    }

    /// The Python's `Filters.session_clauses`: session-level filters
    /// only (everything except the ceiling), with caller-chosen
    /// aliases, for the Turso vector branch's IN-subquery.
    pub fn session_clauses(&self, s_alias: &str, l_alias: &str) -> (Vec<String>, Vec<Value>) {
        let mut clauses: Vec<String> = Vec::new();
        let mut params: Vec<Value> = Vec::new();
        for (col, vals) in [
            (format!("{l_alias}.kind"), &self.kinds),
            (format!("{s_alias}.client"), &self.clients),
        ] {
            if !vals.is_empty() {
                clauses.push(format!("{col} IN ({})", placeholders(vals.len())));
                params.extend(vals.iter().map(|v| Value::text(v.clone())));
            }
        }
        for (col, vals) in [
            (format!("{l_alias}.topics"), &self.topics),
            (format!("{l_alias}.projects"), &self.projects),
        ] {
            for v in vals {
                clauses.push(format!("{col} LIKE ?"));
                params.push(Value::text(format!("%\"{v}\"%")));
            }
        }
        if let Some(s) = &self.since {
            clauses.push(format!("{s_alias}.created_at >= ?"));
            params.push(Value::text(s.clone()));
        }
        if let Some(u) = &self.until_exclusive {
            clauses.push(format!("{s_alias}.created_at < ?"));
            params.push(Value::text(u.clone()));
        }
        (clauses, params)
    }

    /// The Python's `Filters.describe` (field order preserved).
    pub fn describe(&self) -> Json {
        let mut d = Map::new();
        d.insert(
            "max_sensitivity".to_string(),
            Json::from(self.max_sensitivity.name()),
        );
        let mut list = |key: &str, vals: &Vec<String>| {
            if !vals.is_empty() {
                d.insert(key.to_string(), Json::from(vals.clone()));
            }
        };
        list("kind", &self.kinds);
        list("topic", &self.topics);
        list("project", &self.projects);
        list("client", &self.clients);
        if let Some(s) = &self.since {
            d.insert("since".to_string(), Json::from(s.clone()));
        }
        if let Some(u) = &self.until {
            d.insert("until".to_string(), Json::from(u.clone()));
        }
        Json::Object(d)
    }
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}

/// The FTS branch statement (shared by both backends; the Python
/// builds the identical text in `branch_queries` and in the local
/// path of `run_search`).
pub fn fts_stmt(shape: BackendShape, filters: &Filters, match_expr: &str, pool: usize) -> Stmt {
    let (where_sql, params) = filters.where_clause(shape);
    let sql = format!(
        "SELECT c.id AS cid FROM chunks c \
         JOIN chunks_fts f ON f.rowid = c.id {} \
         WHERE chunks_fts MATCH ? AND {where_sql} \
         ORDER BY bm25(chunks_fts) LIMIT ?",
        filters.joins(shape)
    );
    let mut args = vec![Value::text(match_expr)];
    args.extend(params);
    args.push(Value::Integer(pool as i64));
    Stmt::new(sql, args)
}

/// The Turso scan vector-branch statement (chunks-join ceiling +
/// optional session-filter IN-subquery; see the Python's
/// `branch_queries` docstring for why the full join is avoided).
pub fn turso_vector_scan_stmt(
    vec_table: &str,
    filters: &Filters,
    qvec_json: &str,
    pool: usize,
) -> Stmt {
    let allowed = filters.allowed_sens();
    let mut sql = format!(
        "SELECT v.chunk_id AS cid, vector_distance_cos(v.vec, vector32(?)) AS dist \
         FROM {vec_table} v JOIN chunks c ON c.id = v.chunk_id \
         WHERE c.sensitivity IN ({})",
        placeholders(allowed.len())
    );
    let mut args: Vec<Value> = vec![Value::text(qvec_json)];
    args.extend(allowed.iter().map(|s| Value::text(*s)));
    let (sess_clauses, sess_params) = filters.session_clauses("s2", "l2");
    if !sess_clauses.is_empty() {
        sql.push_str(
            " AND v.chunk_id IN (SELECT c2.id FROM chunks c2 \
             JOIN messages m2 ON m2.id = c2.message_id \
             JOIN sessions s2 ON s2.id = m2.session_id \
             LEFT JOIN session_labels l2 ON l2.session_id = s2.id \
             WHERE ",
        );
        sql.push_str(&sess_clauses.join(" AND "));
        sql.push(')');
        args.extend(sess_params);
    }
    sql.push_str(" ORDER BY dist ASC LIMIT ?");
    args.push(Value::Integer(pool as i64));
    Stmt::new(sql, args)
}

/// The Python's `branch_queries`: [FTS?, vector-scan?] in order.
pub fn branch_queries(
    shape: BackendShape,
    vec_table: &str,
    filters: &Filters,
    match_expr: Option<&str>,
    qvec_json: Option<&str>,
    pool: usize,
) -> Vec<Stmt> {
    let mut stmts = Vec::new();
    if let Some(me) = match_expr {
        stmts.push(fts_stmt(shape, filters, me, pool));
    }
    if let Some(qj) = qvec_json {
        stmts.push(turso_vector_scan_stmt(vec_table, filters, qj, pool));
    }
    stmts
}

/// The local vector branch's allowed-id statement (full joins +
/// full where; the Python's local path of `run_search`).
pub fn local_allowed_stmt(shape: BackendShape, filters: &Filters) -> Stmt {
    let (where_sql, params) = filters.where_clause(shape);
    Stmt::new(
        format!(
            "SELECT c.id AS cid FROM chunks c {} WHERE {where_sql}",
            filters.joins(shape)
        ),
        params,
    )
}

/// Sensitivity half of the Python's `turso_allowed_ids`.
pub fn turso_allowed_sens_stmt(filters: &Filters) -> Stmt {
    let allowed = filters.allowed_sens();
    Stmt::new(
        format!(
            "SELECT c.id AS cid FROM chunks c WHERE c.sensitivity IN ({})",
            placeholders(allowed.len())
        ),
        allowed.iter().map(|s| Value::text(*s)).collect(),
    )
}

/// Session-filter half of the Python's `turso_allowed_ids`
/// (`None` when no session-level clause applies).
pub fn turso_allowed_session_stmt(filters: &Filters) -> Option<Stmt> {
    let (clauses, params) = filters.session_clauses("s2", "l2");
    if clauses.is_empty() {
        return None;
    }
    Some(Stmt::new(
        format!(
            "SELECT c2.id AS cid FROM chunks c2 \
             JOIN messages m2 ON m2.id = c2.message_id \
             JOIN sessions s2 ON s2.id = m2.session_id \
             LEFT JOIN session_labels l2 ON l2.session_id = s2.id \
             WHERE {}",
            clauses.join(" AND ")
        ),
        params,
    ))
}

/// The index lookup statement (Turso vector branch, index modes).
pub fn vector_top_k_stmt(vec_table: &str, qvec_json: &str, depth: usize) -> Stmt {
    Stmt::new(
        "SELECT id FROM vector_top_k(?, vector32(?), ?)",
        vec![
            Value::text(format!("{vec_table}_idx")),
            Value::text(qvec_json),
            Value::Integer(depth as i64),
        ],
    )
}

/// Exact distances for index-branch survivors.
pub fn survivor_dist_stmt(vec_table: &str, qvec_json: &str, survivors: &[i64]) -> Stmt {
    let mut args = vec![Value::text(qvec_json)];
    args.extend(survivors.iter().map(|i| Value::Integer(*i)));
    Stmt::new(
        format!(
            "SELECT chunk_id AS cid, vector_distance_cos(vec, vector32(?)) AS dist \
             FROM {vec_table} WHERE chunk_id IN ({})",
            placeholders(survivors.len())
        ),
        args,
    )
}

/// The Python's `details_sql` with the placeholder list expanded.
pub fn details_sql(shape: BackendShape, n_ids: usize) -> String {
    format!(
        "SELECT c.id AS chunk_id, c.text AS text, c.chunk_idx AS chunk_idx, \
         c.message_id AS message_id, m.role AS m_role, m.session_id AS session_id, \
         s.title AS s_title, s.client AS s_client, s.created_at AS s_created_at, \
         l.kind AS l_kind, l.topics AS l_topics, l.projects AS l_projects, \
         l.sensitivity AS l_sensitivity, {} AS chunk_sensitivity \
         FROM chunks c \
         JOIN messages m ON m.id = c.message_id \
         JOIN sessions s ON s.id = m.session_id \
         LEFT JOIN {} l ON l.session_id = s.id {} \
         WHERE c.id IN ({})",
        shape.sens_expr,
        shape.labels_table,
        shape.sens_join,
        placeholders(n_ids)
    )
}

pub const NEIGHBORS_SQL: &str = "SELECT id, chunk_idx, text FROM chunks \
    WHERE message_id = ? AND chunk_idx BETWEEN ? AND ? ORDER BY chunk_idx";

pub const MESSAGE_SQL: &str = "SELECT id, role, text FROM messages WHERE id = ?";

// ------------------------------------------------------------- dates ----

/// (year, month, day) for the ISO forms CPython 3.11+'s
/// `date.fromisoformat` accepts for our inputs: `YYYY-MM-DD` and
/// `YYYYMMDD`, validated as a real calendar date.
pub fn parse_iso_date(s: &str) -> Option<(i32, u32, u32)> {
    let b = s.as_bytes();
    let (y, m, d);
    if b.len() == 10 && b[4] == b'-' && b[7] == b'-' {
        y = digits(&b[0..4])?;
        m = digits(&b[5..7])?;
        d = digits(&b[8..10])?;
    } else if b.len() == 8 {
        y = digits(&b[0..4])?;
        m = digits(&b[4..6])?;
        d = digits(&b[6..8])?;
    } else {
        return None;
    }
    // CPython dates have no year 0: fromisoformat refuses years
    // outside 1..=9999 (#5387). The 4-digit spellings cap the top end
    // already; the bottom end needs this explicit check, or 0000-02-29
    // passes the leap rule below (0 is a multiple of 400).
    if !(1..=9999).contains(&y) {
        return None;
    }
    if !(1..=12).contains(&m) {
        return None;
    }
    let dim = days_in_month(y, m);
    if d < 1 || d > dim {
        return None;
    }
    Some((y as i32, m, d))
}

fn digits(b: &[u8]) -> Option<u32> {
    if b.iter().all(|c| c.is_ascii_digit()) {
        b.iter()
            .try_fold(0u32, |a, c| Some(a * 10 + u32::from(c - b'0')))
    } else {
        None
    }
}

fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

pub fn add_one_day((y, m, d): (i32, u32, u32)) -> (i32, u32, u32) {
    if d < days_in_month(y as u32, m) {
        (y, m, d + 1)
    } else if m < 12 {
        (y, m + 1, 1)
    } else {
        (y + 1, 1, 1)
    }
}

pub fn format_iso_date((y, m, d): (i32, u32, u32)) -> String {
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_filters() -> Filters {
        Filters::new(vec![], vec![], vec![], vec![], "personal", None, None).unwrap()
    }

    #[test]
    fn ceiling_prefixes() {
        assert_eq!(Sensitivity::Shareable.allowed(), vec!["shareable"]);
        assert_eq!(
            Sensitivity::Personal.allowed(),
            vec!["shareable", "personal"]
        );
        assert_eq!(Sensitivity::SecretBearing.allowed(), SENS_ORDER.to_vec());
        assert!(Sensitivity::from_name("bogus").is_none());
    }

    #[test]
    fn filters_validation_mirrors_python() {
        assert!(Filters::new(vec![], vec![], vec![], vec![], "bogus", None, None).is_err());
        assert!(
            Filters::new(
                vec![],
                vec![],
                vec![],
                vec![],
                "personal",
                None,
                Some("2026-13-01".into())
            )
            .is_err()
        );
        assert!(
            Filters::new(
                vec![],
                vec![],
                vec![],
                vec![],
                "personal",
                Some("10/01/2026".into()),
                None
            )
            .is_err()
        );
        let f = Filters::new(
            vec![],
            vec![],
            vec![],
            vec![],
            "personal",
            None,
            Some("2026-10-31".into()),
        )
        .unwrap();
        assert_eq!(f.until_exclusive.as_deref(), Some("2026-11-01"));
        // #5394: the max ISO date has no following day — both
        // spellings are refused instead of binding "10000-01-01".
        for u in ["9999-12-31", "99991231"] {
            assert!(
                Filters::new(
                    vec![],
                    vec![],
                    vec![],
                    vec![],
                    "personal",
                    None,
                    Some(u.into())
                )
                .is_err(),
                "until={u} must be refused"
            );
        }
    }

    #[test]
    fn dates() {
        // #5387: CPython has no year 0 — fromisoformat refuses it, so
        // both spellings of a year-0000 date are refused here too,
        // while the boundary years 1 and 9999 stay accepted.
        assert_eq!(parse_iso_date("0000-01-01"), None);
        assert_eq!(parse_iso_date("0000-02-29"), None);
        assert_eq!(parse_iso_date("00000101"), None);
        assert_eq!(parse_iso_date("0001-01-01"), Some((1, 1, 1)));
        assert_eq!(parse_iso_date("9999-12-31"), Some((9999, 12, 31)));
        assert_eq!(parse_iso_date("2026-02-29"), None);
        assert_eq!(parse_iso_date("2028-02-29"), Some((2028, 2, 29)));
        assert_eq!(parse_iso_date("20261031"), Some((2026, 10, 31)));
        assert_eq!(add_one_day((2026, 12, 31)), (2027, 1, 1));
        assert_eq!(format_iso_date((2026, 11, 1)), "2026-11-01");
    }

    #[test]
    fn where_clause_default_ceiling() {
        let f = default_filters();
        let (sql, params) = f.where_clause(TURSO_SHAPE);
        assert_eq!(sql, "c.sensitivity IN (?,?)");
        assert_eq!(
            params,
            vec![Value::text("shareable"), Value::text("personal")]
        );
    }

    #[test]
    fn session_clauses_exclude_ceiling() {
        let f = Filters::new(
            vec!["code".into()],
            vec![],
            vec![],
            vec![],
            "personal",
            None,
            None,
        )
        .unwrap();
        let (clauses, params) = f.session_clauses("s2", "l2");
        assert_eq!(clauses, vec!["l2.kind IN (?)".to_string()]);
        assert_eq!(params, vec![Value::text("code")]);
    }

    #[test]
    fn describe_shape() {
        let f = default_filters();
        assert_eq!(
            f.describe(),
            serde_json::json!({"max_sensitivity": "personal"})
        );
        let f = Filters::new(
            vec!["code".into()],
            vec![],
            vec![],
            vec![],
            "personal",
            Some("2026-10-01".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            f.describe().to_string(),
            "{\"max_sensitivity\":\"personal\",\"kind\":[\"code\"],\"since\":\"2026-10-01\"}"
        );
    }

    #[test]
    fn since_spellings_normalize_identically() {
        // #5378: parse_iso_date accepts YYYYMMDD as well as YYYY-MM-DD;
        // both spellings must bind the same dashed parameter everywhere
        // `since` reaches SQL, and describe() must report the same value.
        let basic = Filters::new(
            vec![],
            vec![],
            vec![],
            vec![],
            "personal",
            Some("20261031".into()),
            None,
        )
        .unwrap();
        let dashed = Filters::new(
            vec![],
            vec![],
            vec![],
            vec![],
            "personal",
            Some("2026-10-31".into()),
            None,
        )
        .unwrap();
        assert_eq!(basic.since.as_deref(), Some("2026-10-31"));
        assert_eq!(
            basic.where_clause(TURSO_SHAPE),
            dashed.where_clause(TURSO_SHAPE)
        );
        assert_eq!(
            basic.session_clauses("s2", "l2"),
            dashed.session_clauses("s2", "l2")
        );
        assert_eq!(basic.describe(), dashed.describe());
    }

    #[test]
    fn split_csv_python() {
        assert_eq!(split_csv(None), Vec::<String>::new());
        assert_eq!(split_csv(Some("a, b ,,c")), vec!["a", "b", "c"]);
        assert_eq!(split_csv(Some("")), Vec::<String>::new());
    }
}
