//! Shared value/row model: the least common denominator between the
//! Turso hrana pipeline encoding and rusqlite's value types, mirroring
//! what the Python's row dicts carry.

use serde_json::Value as Json;

/// A SQL parameter / column value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    Float(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub fn text(s: impl Into<String>) -> Value {
        Value::Text(s.into())
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Text, preserving Python's `dict.get` semantics: anything that is
    /// not a text column reads as `None`.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

/// One result row: ordered (column name, value) pairs, like the
/// Python's `dict` rows.
#[derive(Debug, Clone, Default)]
pub struct Row {
    cols: Vec<(String, Value)>,
}

impl Row {
    pub fn new(cols: Vec<(String, Value)>) -> Row {
        Row { cols }
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.cols.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    pub fn get_i64(&self, name: &str) -> Option<i64> {
        self.get(name).and_then(Value::as_i64)
    }

    pub fn get_f64(&self, name: &str) -> Option<f64> {
        self.get(name).and_then(Value::as_f64)
    }

    pub fn get_text(&self, name: &str) -> Option<&str> {
        self.get(name).and_then(Value::as_text)
    }
}

/// A parametrized SQL statement: the unit both backends execute and
/// the unit the parity fixtures record.
#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub sql: String,
    pub args: Vec<Value>,
}

impl Stmt {
    pub fn new(sql: impl Into<String>, args: Vec<Value>) -> Stmt {
        Stmt {
            sql: sql.into(),
            args,
        }
    }
}

/// One branch hit in rank order. `dist` is the cosine distance for
/// vector hits (`None` for FTS hits; the Python carries `None` raw
/// values for the FTS branch and a possibly-missing distance for
/// index-branch survivors).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BranchHit {
    pub chunk_id: i64,
    pub dist: Option<f64>,
}

/// The fused pools one search runs over.
#[derive(Debug, Clone, Default)]
pub struct BranchPools {
    pub fts: Vec<BranchHit>,
    pub vector: Vec<BranchHit>,
}

/// The branch inputs, mirroring `run_search`'s per-mode state.
#[derive(Debug, Clone)]
pub struct BranchQuery {
    pub match_expr: Option<String>,
    /// Normalized (and possibly truncated) query vector.
    pub query_vector: Option<Vec<f32>>,
    pub pool: usize,
    pub vector_backend: crate::VectorBackend,
}

/// A chunk's detail row (the Python's `fetch_details` output), with
/// effective sensitivity resolved: chunk sensitivity, falling back to
/// the session label's (Python: `chunk_sensitivity or l_sensitivity`).
#[derive(Debug, Clone)]
pub struct ChunkDetail {
    pub chunk_id: i64,
    pub text: String,
    pub chunk_idx: i64,
    pub message_id: String,
    pub m_role: Option<String>,
    pub session_id: String,
    pub s_title: Option<String>,
    pub s_client: Option<String>,
    pub s_created_at: Option<String>,
    pub l_kind: Option<String>,
    pub l_topics: Option<String>,
    pub l_projects: Option<String>,
    pub eff_sensitivity: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NeighborRow {
    pub chunk_id: i64,
    pub chunk_idx: i64,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct MessageRow {
    pub id: String,
    pub role: Option<String>,
    pub text: String,
}

/// The Python's `parse_json_list`: non-list / unparsable / empty
/// inputs all read as an empty list.
pub fn parse_json_list(v: Option<&str>) -> Vec<Json> {
    let Some(s) = v else { return Vec::new() };
    if s.is_empty() {
        return Vec::new();
    }
    match serde_json::from_str::<Json>(s) {
        Ok(Json::Array(items)) => items,
        _ => Vec::new(),
    }
}

/// Python's `round(x, 6)`: round the exact binary value to six
/// decimal places (ties to even, as CPython's float round does via
/// correct decimal rounding), then take the nearest f64 — whose
/// shortest repr is the decimal the Python prints.
pub fn round6(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    format!("{x:.6}").parse::<f64>().unwrap_or(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round6_matches_python() {
        assert_eq!(round6(0.17424242424242425), 0.174242);
        assert_eq!(round6(1.0 / 11.0), 0.090909);
        assert_eq!(round6(0.0), 0.0);
        assert_eq!(round6(2.0), 2.0);
        assert_eq!(round6(0.0018811234), 0.001881);
    }

    #[test]
    fn parse_json_list_shapes() {
        assert_eq!(parse_json_list(None), Vec::<Json>::new());
        assert_eq!(parse_json_list(Some("")), Vec::<Json>::new());
        assert_eq!(parse_json_list(Some("not json")), Vec::<Json>::new());
        assert_eq!(parse_json_list(Some("{\"a\":1}")), Vec::<Json>::new());
        assert_eq!(
            parse_json_list(Some("[\"nixos\", \"backup\"]")),
            vec![Json::from("nixos"), Json::from("backup")]
        );
    }
}

/// Convert one `details_sql` row into a [`ChunkDetail`], resolving
/// effective sensitivity the Python's way (`chunk or label`).
/// Only the chunk id is structurally required; other columns fall
/// back to empty/None the way the Python's dict reads tolerate.
pub fn detail_from_row(r: &Row) -> Option<ChunkDetail> {
    let chunk_sens = r.get_text("chunk_sensitivity").filter(|s| !s.is_empty());
    let eff = chunk_sens
        .or_else(|| r.get_text("l_sensitivity"))
        .map(str::to_string);
    Some(ChunkDetail {
        chunk_id: r.get_i64("chunk_id")?,
        text: r.get_text("text").unwrap_or("").to_string(),
        chunk_idx: r.get_i64("chunk_idx").unwrap_or(0),
        message_id: r.get_text("message_id").unwrap_or("").to_string(),
        m_role: r.get_text("m_role").map(str::to_string),
        session_id: r.get_text("session_id").unwrap_or("").to_string(),
        s_title: r.get_text("s_title").map(str::to_string),
        s_client: r.get_text("s_client").map(str::to_string),
        s_created_at: r.get_text("s_created_at").map(str::to_string),
        l_kind: r.get_text("l_kind").map(str::to_string),
        l_topics: r.get_text("l_topics").map(str::to_string),
        l_projects: r.get_text("l_projects").map(str::to_string),
        eff_sensitivity: eff,
    })
}

impl Row {
    /// The first column's value (for single-column queries).
    pub fn first_value(&self) -> Option<&Value> {
        self.cols.first().map(|(_, v)| v)
    }
}

/// The production vector table (the CLI's `VEC_TABLE`).
pub const DEFAULT_VEC_TABLE: &str = "vec_snowflake_m_v2_768_f32";

/// The production embedder identity (the CLI's `MODEL`).
pub const DEFAULT_MODEL: &str = "Snowflake/snowflake-arctic-embed-m-v2.0";

/// How the vector branch gathers its pool. The default stays
/// `Scan` (the current production path) until the #5356
/// golden-parity cutover promotes a winner from the A/B.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VectorBackend {
    #[default]
    Scan,
    IndexFixed,
    IndexFill,
}

impl VectorBackend {
    pub fn as_str(&self) -> &'static str {
        match self {
            VectorBackend::Scan => "scan",
            VectorBackend::IndexFixed => "index-fixed",
            VectorBackend::IndexFill => "index-fill",
        }
    }

    pub fn parse(s: &str) -> Option<VectorBackend> {
        match s {
            "scan" => Some(VectorBackend::Scan),
            "index-fixed" => Some(VectorBackend::IndexFixed),
            "index-fill" => Some(VectorBackend::IndexFill),
            _ => None,
        }
    }
}
