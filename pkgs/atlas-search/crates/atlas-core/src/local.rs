//! The Local backend: SQLite publish files, mirroring the Python's
//! `LocalBackend` — schema-normalizing temp views (labels and
//! chunk sensitivity from the file itself or an attached sibling
//! publish copy), vector side stores joined by **content hash**
//! (never chunk_id: the id spaces differ between stores), and an
//! in-process cosine scan (rusqlite's SQLite has no
//! `vector_distance_cos`; the Python uses numpy the same way).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{Connection, OpenFlags};

use crate::backend::SearchBackend;
use crate::error::{Error, Result};
use crate::filters::{
    Filters, LOCAL_SHAPE, MESSAGE_SQL, NEIGHBORS_SQL, details_sql, fts_stmt, local_allowed_stmt,
};
use crate::types::{
    BranchHit, BranchPools, BranchQuery, ChunkDetail, MessageRow, NeighborRow, Row, Stmt, Value,
    detail_from_row,
};

type RValue = rusqlite::types::Value;

fn to_rvalue(v: &Value) -> RValue {
    match v {
        Value::Null => RValue::Null,
        Value::Integer(i) => RValue::Integer(*i),
        Value::Float(f) => RValue::Real(*f),
        Value::Text(s) => RValue::Text(s.clone()),
        Value::Blob(b) => RValue::Blob(b.clone()),
    }
}

fn from_rvalue(v: RValue) -> Value {
    match v {
        RValue::Null => Value::Null,
        RValue::Integer(i) => Value::Integer(i),
        RValue::Real(f) => Value::Float(f),
        RValue::Text(s) => Value::Text(s),
        RValue::Blob(b) => Value::Blob(b),
    }
}

#[derive(Debug, Clone)]
enum VecSource {
    Main,
    IdKey(String),
    Mapped(String, String),
}

struct Matrix {
    ids: Vec<i64>,
    rows: Vec<Vec<f32>>,
}

/// (vec file, corpus file whose chunks(id, content_hash) maps its
/// ids) — the Python's KNOWN_VEC_PAIRS.
const KNOWN_VEC_PAIRS: [(&str, &str); 4] = [
    ("~/atlas/v3/vec-bge-small.db", "~/atlas/v3/corpus.db"),
    (
        "~/atlas/v3/vec-bge-small-topup.db",
        "~/atlas/v3/topup-chunks.db",
    ),
    (
        "~/atlas/atlas-delta-chunks2.db",
        "~/atlas/atlas-delta-chunks2.db",
    ),
    ("~/atlas/v3/vec-snowflake.db", "~/atlas/v3/corpus.db"),
];

fn expanduser(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        // Home-resolution contract (issues #5640 / #5762): expanding
        // "~/" with a missing or empty HOME fails loudly naming
        // HOME, never leaving the literal "~" to resolve
        // relative to the CWD.
        let home = match std::env::var_os("HOME") {
            Some(home) if !home.is_empty() => home,
            _ => panic!("HOME is not set or is empty: atlas-core requires a non-empty HOME to expand ~/ paths (issue #5640)"),
        };
        return PathBuf::from(home)
            .join(rest)
            .to_string_lossy()
            .into_owned();
    }
    p.to_string()
}

pub struct LocalBackend {
    db_path: String,
    con: Mutex<Connection>,
    vec_table: String,
    trunc: usize,
    notes: Mutex<Vec<String>>,
    /// Diagnostics the Python keeps; not part of the envelope.
    pub sens_source: String,
    pub chunk_sens_source: String,
    vec_sources: Vec<VecSource>,
    matrix: Mutex<Option<Option<Matrix>>>,
}

impl LocalBackend {
    /// Mirror of the Python constructor: `db_path` must exist;
    /// `vecdb_arg` corresponds to `--vecdb` (falling back to the
    /// `ATLAS_VEC_DB` env var, as the Python does).
    pub fn new(
        db_path: &str,
        vecdb_arg: Option<&str>,
        vec_table: &str,
        trunc: usize,
    ) -> Result<LocalBackend> {
        if !Path::new(db_path).exists() {
            return Err(Error::fatal(format!(
                "local backend: DB not found: {db_path}"
            )));
        }
        let con = Connection::open_with_flags(
            format!("file:{db_path}?mode=ro"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .map_err(|e| Error::backend(format!("local backend: open {db_path}: {e}")))?;
        let mut be = LocalBackend {
            db_path: db_path.to_string(),
            con: Mutex::new(con),
            vec_table: vec_table.to_string(),
            trunc,
            notes: Mutex::new(Vec::new()),
            sens_source: String::new(),
            chunk_sens_source: String::new(),
            vec_sources: Vec::new(),
            matrix: Mutex::new(None),
        };
        be.setup_views()?;
        be.vec_sources = be.find_vec_sources(vecdb_arg)?;
        Ok(be)
    }

    fn push_note(&self, note: String) {
        self.notes.lock().expect("notes mutex").push(note);
    }

    fn tables(&self, schema: &str) -> Result<HashSet<String>> {
        let con = self.con.lock().expect("connection mutex");
        let mut stmt = con
            .prepare(&format!(
                "SELECT name FROM {schema}.sqlite_master WHERE type IN ('table','view')"
            ))
            .map_err(|e| Error::backend(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| Error::backend(e.to_string()))?;
        let mut out = HashSet::new();
        for r in rows {
            out.insert(r.map_err(|e| Error::backend(e.to_string()))?);
        }
        Ok(out)
    }

    fn cols(&self, table: &str, schema: &str) -> Result<HashSet<String>> {
        let con = self.con.lock().expect("connection mutex");
        let mut stmt = con
            .prepare(&format!("PRAGMA {schema}.table_info({table})"))
            .map_err(|e| Error::backend(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(|e| Error::backend(e.to_string()))?;
        let mut out = HashSet::new();
        for r in rows {
            out.insert(r.map_err(|e| Error::backend(e.to_string()))?);
        }
        Ok(out)
    }

    fn exec_void(&self, sql: &str, params: &[RValue]) -> Result<()> {
        let con = self.con.lock().expect("connection mutex");
        con.execute(sql, rusqlite::params_from_iter(params.iter()))
            .map_err(|e| Error::backend(e.to_string()))?;
        Ok(())
    }

    /// The Python's `_setup_views`.
    fn setup_views(&mut self) -> Result<()> {
        let main_tables = self.tables("main")?;
        let mut aux: Option<String> = None;
        if !main_tables.contains("session_labels")
            || !self.cols("chunks", "main")?.contains("sensitivity")
        {
            // Look for a publish copy beside the DB (newest first).
            let db_abs = std::fs::canonicalize(&self.db_path).unwrap_or_default();
            let dir = db_abs.parent().map(Path::to_path_buf).unwrap_or_default();
            let mut sibs: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for ent in rd.flatten() {
                    let p = ent.path();
                    let name = ent.file_name().to_string_lossy().into_owned();
                    if name.starts_with("atlas-publish-") && name.ends_with(".db") {
                        if let Ok(canon) = std::fs::canonicalize(&p)
                            && canon == db_abs
                        {
                            continue;
                        }
                        let mtime = ent
                            .metadata()
                            .and_then(|m| m.modified())
                            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                        sibs.push((mtime, p));
                    }
                }
            }
            sibs.sort_by_key(|e| std::cmp::Reverse(e.0));
            for (_, sib) in sibs {
                let uri = format!("file:{}?mode=ro", sib.to_string_lossy());
                if self
                    .exec_void("ATTACH DATABASE ?1 AS aux", &[RValue::Text(uri)])
                    .is_ok()
                {
                    aux = Some(sib.to_string_lossy().into_owned());
                    break;
                }
            }
            if let Some(a) = &aux {
                let base = Path::new(a)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.push_note(format!(
                    "labels/sensitivity attached from sibling publish copy {base}"
                ));
            }
        }
        let aux_tables = if aux.is_some() {
            self.tables("aux")?
        } else {
            HashSet::new()
        };
        if main_tables.contains("session_labels") {
            self.exec_void(
                "CREATE TEMP VIEW v_labels AS SELECT * FROM main.session_labels",
                &[],
            )?;
            self.sens_source = "session_labels (this DB)".to_string();
        } else if aux_tables.contains("session_labels") {
            self.exec_void(
                "CREATE TEMP VIEW v_labels AS SELECT * FROM aux.session_labels",
                &[],
            )?;
            self.sens_source = "session_labels (attached publish copy)".to_string();
        } else {
            self.exec_void(
                "CREATE TEMP TABLE v_labels (session_id TEXT PRIMARY KEY, client TEXT, \
                 kind TEXT, topics TEXT, projects TEXT, sensitivity TEXT, \
                 confidence TEXT, scheme TEXT)",
                &[],
            )?;
            self.sens_source = "none — this DB carries no labels".to_string();
            self.push_note(
                "no session_labels available: kind/topic/project filters match nothing".to_string(),
            );
        }
        if self.cols("chunks", "main")?.contains("sensitivity") {
            self.exec_void(
                "CREATE TEMP VIEW v_chunk_sens AS SELECT id, sensitivity FROM main.chunks",
                &[],
            )?;
            self.chunk_sens_source = "chunks.sensitivity (this DB)".to_string();
        } else if aux.is_some() && self.cols("chunks", "aux")?.contains("sensitivity") {
            self.exec_void(
                "CREATE TEMP VIEW v_chunk_sens AS SELECT id, sensitivity FROM aux.chunks",
                &[],
            )?;
            self.chunk_sens_source = "chunks.sensitivity (attached publish copy)".to_string();
        } else {
            self.exec_void(
                "CREATE TEMP TABLE v_chunk_sens (id INTEGER PRIMARY KEY, sensitivity TEXT)",
                &[],
            )?;
            self.chunk_sens_source = "none — no sensitivity data available".to_string();
            self.push_note(
                "WARNING: no sensitivity data in this DB and no publish copy beside it; \
                 the sensitivity ceiling excludes every chunk (fail-closed). \
                 Point --db at a publish copy to search this corpus."
                    .to_string(),
            );
        }
        Ok(())
    }

    /// The Python's `_find_vec_sources`.
    fn find_vec_sources(&self, vecdb_arg: Option<&str>) -> Result<Vec<VecSource>> {
        if self.tables("main")?.contains(&self.vec_table) {
            return Ok(vec![VecSource::Main]);
        }
        let env = match vecdb_arg {
            Some(v) if !v.is_empty() => Some(v.to_string()),
            _ => std::env::var("ATLAS_VEC_DB").ok().filter(|v| !v.is_empty()),
        };
        if let Some(env) = env {
            return Ok(env
                .split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(|p| VecSource::IdKey(p.to_string()))
                .collect());
        }
        let mut out = Vec::new();
        let mut seen_vec: HashSet<String> = HashSet::new();
        for (vec_rel, map_rel) in KNOWN_VEC_PAIRS {
            let vec_p = expanduser(vec_rel);
            let map_p = expanduser(map_rel);
            if seen_vec.contains(&vec_p)
                || !Path::new(&vec_p).exists()
                || !Path::new(&map_p).exists()
            {
                continue;
            }
            let probe = Connection::open_with_flags(
                format!("file:{vec_p}?mode=ro"),
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
            );
            let Ok(probe) = probe else { continue };
            let tabs: HashSet<String> = probe
                .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','view')")
                .and_then(|mut s| {
                    let rows = s.query_map([], |r| r.get::<_, String>(0))?;
                    let mut set = HashSet::new();
                    for r in rows {
                        set.insert(r?);
                    }
                    Ok(set)
                })
                .unwrap_or_default();
            if !tabs.contains(&self.vec_table) {
                continue;
            }
            seen_vec.insert(vec_p.clone());
            out.push(VecSource::Mapped(vec_p, map_p));
        }
        if !out.is_empty() {
            self.push_note(format!(
                "vectors: {} side store(s) mapped by content_hash \
                 (id spaces differ between stores; content_hash is the identity)",
                out.len()
            ));
        }
        Ok(out)
    }

    fn execute(&self, stmt: &Stmt) -> Result<Vec<Row>> {
        let con = self.con.lock().expect("connection mutex");
        let mut prepared = con
            .prepare(&stmt.sql)
            .map_err(|e| Error::backend(e.to_string()))?;
        let col_names: Vec<String> = prepared
            .column_names()
            .into_iter()
            .map(str::to_string)
            .collect();
        let params: Vec<RValue> = stmt.args.iter().map(to_rvalue).collect();
        let mut rows = prepared
            .query(rusqlite::params_from_iter(params.iter()))
            .map_err(|e| Error::backend(e.to_string()))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(|e| Error::backend(e.to_string()))? {
            let mut pairs = Vec::with_capacity(col_names.len());
            for (i, name) in col_names.iter().enumerate() {
                let v = row
                    .get::<_, RValue>(i)
                    .map_err(|e| Error::backend(e.to_string()))?;
                pairs.push((name.clone(), from_rvalue(v)));
            }
            out.push(Row::new(pairs));
        }
        Ok(out)
    }

    fn execute_scalar_i64(&self, sql: &str) -> Result<i64> {
        let rows = self.execute(&Stmt::new(sql, vec![]))?;
        Ok(match rows.first().and_then(|r| r.first_value()) {
            Some(Value::Integer(i)) => *i,
            Some(Value::Float(f)) => *f as i64,
            _ => 0,
        })
    }

    /// The Python's `load_matrix` body: gather vectors from
    /// all sources (first source wins per chunk), keyed into
    /// the main DB's id space (side stores via content hash),
    /// rows L2-normalized.
    fn build_matrix(&self) -> Result<Option<Matrix>> {
        let total_chunks = self.execute_scalar_i64("SELECT count(*) FROM chunks")?;
        let mut vec_by_id: HashMap<i64, Vec<u8>> = HashMap::new();
        let mut mapped_used = false;
        for (i, src) in self.vec_sources.iter().enumerate() {
            match src {
                VecSource::Main => {
                    let rows = self.execute(&Stmt::new(
                        format!("SELECT chunk_id, vec FROM main.{}", self.vec_table),
                        vec![],
                    ))?;
                    for r in &rows {
                        if let (Some(cid), Some(Value::Blob(b))) =
                            (r.get_i64("chunk_id"), r.get("vec"))
                        {
                            vec_by_id.entry(cid).or_insert_with(|| b.clone());
                        }
                    }
                }
                VecSource::IdKey(path) => {
                    let schema = format!("vx{i}");
                    let uri = format!("file:{path}?mode=ro");
                    self.exec_void(
                        &format!("ATTACH DATABASE ?1 AS {schema}"),
                        &[RValue::Text(uri)],
                    )?;
                    if !self.tables(&schema)?.contains(&self.vec_table) {
                        continue;
                    }
                    let rows = self.execute(&Stmt::new(
                        format!("SELECT chunk_id, vec FROM {schema}.{}", self.vec_table),
                        vec![],
                    ))?;
                    for r in &rows {
                        if let (Some(cid), Some(Value::Blob(b))) =
                            (r.get_i64("chunk_id"), r.get("vec"))
                        {
                            vec_by_id.entry(cid).or_insert_with(|| b.clone());
                        }
                    }
                }
                VecSource::Mapped(vec_p, map_p) => {
                    mapped_used = true;
                    let vschema = format!("vm{i}");
                    let uri = format!("file:{vec_p}?mode=ro");
                    self.exec_void(
                        &format!("ATTACH DATABASE ?1 AS {vschema}"),
                        &[RValue::Text(uri)],
                    )?;
                    let mschema = if map_p != vec_p {
                        let mschema = format!("mm{i}");
                        let muri = format!("file:{map_p}?mode=ro");
                        self.exec_void(
                            &format!("ATTACH DATABASE ?1 AS {mschema}"),
                            &[RValue::Text(muri)],
                        )?;
                        mschema
                    } else {
                        vschema.clone()
                    };
                    if !self.tables(&vschema)?.contains(&self.vec_table) {
                        continue;
                    }
                    let mut id2hash: HashMap<i64, String> = HashMap::new();
                    for r in &self.execute(&Stmt::new(
                        format!("SELECT id, content_hash FROM {mschema}.chunks"),
                        vec![],
                    ))? {
                        if let (Some(id), Some(h)) = (r.get_i64("id"), r.get_text("content_hash")) {
                            id2hash.insert(id, h.to_string());
                        }
                    }
                    let mut hash2main: HashMap<String, i64> = HashMap::new();
                    for r in &self.execute(&Stmt::new(
                        "SELECT id, content_hash FROM main.chunks",
                        vec![],
                    ))? {
                        if let (Some(id), Some(h)) = (r.get_i64("id"), r.get_text("content_hash")) {
                            hash2main.insert(h.to_string(), id);
                        }
                    }
                    let rows = self.execute(&Stmt::new(
                        format!("SELECT chunk_id, vec FROM {vschema}.{}", self.vec_table),
                        vec![],
                    ))?;
                    for r in &rows {
                        if let (Some(cid), Some(Value::Blob(b))) =
                            (r.get_i64("chunk_id"), r.get("vec"))
                            && let Some(mid) = id2hash.get(&cid).and_then(|h| hash2main.get(h))
                        {
                            vec_by_id.entry(*mid).or_insert_with(|| b.clone());
                        }
                    }
                }
            }
        }
        if mapped_used {
            self.push_note(format!(
                "vector coverage: {} of {} chunks have vectors via content-hash mapping",
                vec_by_id.len(),
                total_chunks
            ));
        }
        let mut ids: Vec<i64> = vec_by_id.keys().copied().collect();
        ids.sort_unstable();
        if ids.is_empty() {
            return Ok(None);
        }
        let mut rows_out: Vec<Vec<f32>> = Vec::with_capacity(ids.len());
        for id in ids.iter() {
            let blob = &vec_by_id[id];
            if !blob.len().is_multiple_of(4) {
                return Err(Error::backend(format!(
                    "vector blob for chunk {id} is {} bytes, not a multiple of 4",
                    blob.len()
                )));
            }
            let mut v: Vec<f32> = blob
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            if self.trunc > 0 {
                v.truncate(self.trunc);
            }
            let squares: Vec<f32> = v.iter().map(|x| x * x).collect();
            let norm = atlas_embed::pairwise_sum_f32(&squares).sqrt();
            if norm > 0.0 {
                for x in v.iter_mut() {
                    *x /= norm;
                }
            }
            rows_out.push(v);
        }
        Ok(Some(Matrix {
            ids,
            rows: rows_out,
        }))
    }

    /// Lazily build (once) and lend the matrix, the way the
    /// Python's `load_matrix` returns the cached arrays.
    fn with_matrix<R>(&self, f: impl FnOnce(Option<&Matrix>) -> Result<R>) -> Result<R> {
        let mut cache = self.matrix.lock().expect("matrix mutex");
        if cache.is_none() {
            let built = self.build_matrix()?;
            *cache = Some(built);
        }
        let slot = cache.as_ref().expect("matrix cache filled");
        f(slot.as_ref())
    }

    /// The Python's `vector_search`: cosine sims over the allowed
    /// subset (or everything), best `pool` in rank order.
    /// Dot products sum with atlas-embed's NumPy-pairwise order;
    /// ties keep ascending chunk-id order (the matrix's storage
    /// order). numpy's argsort is unstable, so exact float ties are
    /// the one place ordering can differ from the Python in
    /// principle; well-separated corpora do not produce them.
    fn vector_search(
        &self,
        qvec: &[f32],
        allowed: Option<&HashSet<i64>>,
        pool: usize,
    ) -> Result<Vec<(i64, f32)>> {
        self.with_matrix(|matrix| {
            let Some(matrix) = matrix else {
                return Ok(Vec::new());
            };
            let mut cand: Vec<(i64, f32)> = Vec::new();
            for (pos, id) in matrix.ids.iter().enumerate() {
                if let Some(a) = allowed
                    && !a.contains(id)
                {
                    continue;
                }
                let row = &matrix.rows[pos];
                let products: Vec<f32> = row.iter().zip(qvec.iter()).map(|(a, b)| a * b).collect();
                let sim = atlas_embed::pairwise_sum_f32(&products);
                cand.push((*id, sim));
            }
            cand.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            cand.truncate(pool);
            Ok(cand)
        })
    }
}

impl SearchBackend for LocalBackend {
    fn name(&self) -> &'static str {
        "local"
    }

    fn vec_table(&self) -> &str {
        &self.vec_table
    }

    fn notes(&self) -> Vec<String> {
        self.notes.lock().expect("notes mutex").clone()
    }

    fn has_vector_store(&self) -> Option<bool> {
        Some(!self.vec_sources.is_empty())
    }

    fn branch_pools(&self, q: &BranchQuery, filters: &Filters) -> Result<BranchPools> {
        let mut pools = BranchPools::default();
        if let Some(me) = &q.match_expr {
            let rows = self.execute(&fts_stmt(LOCAL_SHAPE, filters, me, q.pool))?;
            pools.fts = rows
                .iter()
                .filter_map(|r| {
                    Some(BranchHit {
                        chunk_id: r.get_i64("cid")?,
                        dist: None,
                    })
                })
                .collect();
        }
        if let Some(qvec) = &q.query_vector {
            let rows = self.execute(&local_allowed_stmt(LOCAL_SHAPE, filters))?;
            let allowed: HashSet<i64> = rows.iter().filter_map(|r| r.get_i64("cid")).collect();
            let hits = self.vector_search(qvec, Some(&allowed), q.pool)?;
            pools.vector = hits
                .into_iter()
                .map(|(cid, sim)| BranchHit {
                    chunk_id: cid,
                    dist: Some(1.0 - f64::from(sim)),
                })
                .collect();
        }
        Ok(pools)
    }

    fn chunk_details(&self, ids: &[i64]) -> Result<Vec<ChunkDetail>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let sql = details_sql(LOCAL_SHAPE, ids.len());
        let args: Vec<Value> = ids.iter().map(|i| Value::Integer(*i)).collect();
        let rows = self.execute(&Stmt::new(sql, args))?;
        Ok(rows.iter().filter_map(detail_from_row).collect())
    }

    fn neighbors(&self, message_id: &str, chunk_idx: i64, n: i64) -> Result<Vec<NeighborRow>> {
        let rows = self.execute(&Stmt::new(
            NEIGHBORS_SQL,
            vec![
                Value::text(message_id),
                Value::Integer(chunk_idx - n),
                Value::Integer(chunk_idx + n),
            ],
        ))?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                Some(NeighborRow {
                    chunk_id: r.get_i64("id")?,
                    chunk_idx: r.get_i64("chunk_idx")?,
                    text: r.get_text("text")?.to_string(),
                })
            })
            .collect())
    }

    fn message(&self, message_id: &str) -> Result<Option<MessageRow>> {
        let rows = self.execute(&Stmt::new(MESSAGE_SQL, vec![Value::text(message_id)]))?;
        Ok(rows.first().and_then(|r| {
            Some(MessageRow {
                id: r.get_text("id")?.to_string(),
                role: r.get_text("role").map(str::to_string),
                text: r.get_text("text").unwrap_or("").to_string(),
            })
        }))
    }
}
