//! Local-file shards: a read-only SQLite connection that speaks the
//! same pipeline contract as the Turso transport, with the two
//! TursoDB vector functions the scan statement uses registered
//! (`vector32(text)` and `vector_distance_cos(blob, blob)`), so the
//! *identical* SQL runs against a local fixture and a hosted shard.
//! This is the offline oracle for the fan-out tests and the seam for
//! embedded replicas (design 5.5) later.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::functions::FunctionFlags;
use rusqlite::{Connection, OpenFlags};

use super::{SHARD_PREFIX, ShardEndpoint, ShardLister, ShardLocation};
use crate::error::{Error, Result};
use crate::turso::PipelineTransport;
use crate::types::{Row, Stmt, Value};

type RValue = rusqlite::types::Value;

fn blob_to_f32(b: &[u8]) -> Option<Vec<f32>> {
    if b.len() % 4 != 0 {
        return None;
    }
    Some(
        b.chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

/// `vector32('[0.1, 0.2, ...]')` -> little-endian f32 blob.
fn vector32(text: &str) -> Option<Vec<u8>> {
    let vals: Vec<f64> = serde_json::from_str(text).ok()?;
    let mut out = Vec::with_capacity(vals.len() * 4);
    for v in vals {
        out.extend_from_slice(&(v as f32).to_le_bytes());
    }
    Some(out)
}

/// Cosine distance, `1 - cos(a, b)`, as TursoDB's
/// `vector_distance_cos`. Mismatched lengths or zero vectors have no
/// distance (NULL).
pub fn cosine_distance(a: &[f32], b: &[f32]) -> Option<f64> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let (mut dot, mut na, mut nb) = (0f64, 0f64, 0f64);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (*x as f64, *y as f64);
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return None;
    }
    Some(1.0 - dot / (na.sqrt() * nb.sqrt()))
}

pub struct LocalShardTransport {
    con: Mutex<Connection>,
}

impl LocalShardTransport {
    pub fn open(path: &Path) -> Result<LocalShardTransport> {
        if !path.exists() {
            return Err(Error::fatal(format!(
                "local shard: file not found: {}",
                path.display()
            )));
        }
        let con = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| Error::backend(format!("local shard: open {}: {e}", path.display())))?;
        let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
        con.create_scalar_function("vector32", 1, flags, |ctx| {
            let t: String = ctx.get(0)?;
            Ok(vector32(&t))
        })
        .map_err(|e| Error::backend(e.to_string()))?;
        con.create_scalar_function("vector_distance_cos", 2, flags, |ctx| {
            let a: Vec<u8> = ctx.get(0)?;
            let b: Vec<u8> = ctx.get(1)?;
            Ok(match (blob_to_f32(&a), blob_to_f32(&b)) {
                (Some(a), Some(b)) => cosine_distance(&a, &b),
                _ => None,
            })
        })
        .map_err(|e| Error::backend(e.to_string()))?;
        Ok(LocalShardTransport {
            con: Mutex::new(con),
        })
    }
}

impl PipelineTransport for LocalShardTransport {
    fn pipeline(&self, stmts: &[Stmt]) -> Result<Vec<Vec<Row>>> {
        let con = self.con.lock().expect("shard connection mutex");
        let mut out = Vec::new();
        for s in stmts {
            let mut q = con
                .prepare(&s.sql)
                .map_err(|e| Error::fatal(format!("local shard: SQL error: {e}")))?;
            let names: Vec<String> = q.column_names().iter().map(|n| n.to_string()).collect();
            let params: Vec<RValue> = s
                .args
                .iter()
                .map(|v| match v {
                    Value::Null => RValue::Null,
                    Value::Integer(i) => RValue::Integer(*i),
                    Value::Float(f) => RValue::Real(*f),
                    Value::Text(t) => RValue::Text(t.clone()),
                    Value::Blob(b) => RValue::Blob(b.clone()),
                })
                .collect();
            let mut rows = q
                .query(rusqlite::params_from_iter(params.iter()))
                .map_err(|e| Error::fatal(format!("local shard: SQL error: {e}")))?;
            let mut result = Vec::new();
            while let Some(r) = rows
                .next()
                .map_err(|e| Error::fatal(format!("local shard: SQL error: {e}")))?
            {
                let mut pairs = Vec::new();
                for (i, n) in names.iter().enumerate() {
                    let v: RValue = r
                        .get(i)
                        .map_err(|e| Error::backend(format!("local shard: column {n}: {e}")))?;
                    pairs.push((
                        n.clone(),
                        match v {
                            RValue::Null => Value::Null,
                            RValue::Integer(i) => Value::Integer(i),
                            RValue::Real(f) => Value::Float(f),
                            RValue::Text(t) => Value::Text(t),
                            RValue::Blob(b) => Value::Blob(b),
                        },
                    ));
                }
                result.push(Row::new(pairs));
            }
            out.push(result);
        }
        Ok(out)
    }
}

/// Candidate shards from a directory of `atlas-core-*.db` files
/// (database name = file stem). New files join on the next
/// discovery.
pub struct DirLister {
    pub dir: PathBuf,
}

impl ShardLister for DirLister {
    fn list(&self) -> Result<Vec<ShardEndpoint>> {
        let rd = std::fs::read_dir(&self.dir)
            .map_err(|e| Error::fatal(format!("shard dir {}: {e}", self.dir.display())))?;
        let mut out = Vec::new();
        for ent in rd.flatten() {
            let p = ent.path();
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if p.extension().and_then(|e| e.to_str()) == Some("db")
                && stem.starts_with(SHARD_PREFIX)
            {
                out.push(ShardEndpoint {
                    name: stem.to_string(),
                    location: ShardLocation::File(p.clone()),
                });
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_basics() {
        assert!((cosine_distance(&[1.0, 0.0], &[1.0, 0.0]).unwrap()).abs() < 1e-12);
        assert!((cosine_distance(&[1.0, 0.0], &[0.0, 1.0]).unwrap() - 1.0).abs() < 1e-12);
        assert!((cosine_distance(&[1.0, 0.0], &[-1.0, 0.0]).unwrap() - 2.0).abs() < 1e-12);
        assert_eq!(cosine_distance(&[1.0], &[1.0, 2.0]), None);
        assert_eq!(cosine_distance(&[0.0, 0.0], &[1.0, 2.0]), None);
    }

    #[test]
    fn vector32_roundtrip() {
        let b = vector32("[1.5, -2.0]").unwrap();
        assert_eq!(blob_to_f32(&b).unwrap(), vec![1.5f32, -2.0]);
        assert_eq!(vector32("nope"), None);
    }
}
