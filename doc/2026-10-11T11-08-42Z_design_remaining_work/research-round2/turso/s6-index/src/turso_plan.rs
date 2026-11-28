//! Diagnostic: is the (author, seq) lookup a primary-key seek? Prints the query plans on
//! the WITHOUT ROWID schema and on the rowid schema, then times 1,000 author_seq lookups
//! at 1,000 rows and at 20,000 rows for each. Time that grows with the row count means a
//! full scan per lookup.

use crate::block::block_on;
use crate::turso_store::{SCHEMA, SCHEMA_ROWID};
use anyhow::{Context, Result};
use std::path::Path;
use std::time::Instant;
use turso::{Builder, Connection, Statement, Value, params_from_iter};

const QUERIES: [(&str, &str); 3] = [
    ("author_seq lookup", "SELECT id FROM author_seq WHERE author = ?1 AND seq = ?2"),
    ("claims by id", "SELECT body FROM claims WHERE id = ?1"),
    ("alpha range", "SELECT key FROM alpha WHERE key >= ?1 AND key < ?2 ORDER BY key"),
];

const LOOKUP: &str = "SELECT id FROM author_seq WHERE author = ?1 AND seq = ?2";

fn author(i: u32) -> Vec<u8> {
    let mut a = vec![0u8; 32];
    a[..4].copy_from_slice(&i.to_be_bytes());
    a
}

async fn plans(conn: &Connection, schema: &str) -> Result<()> {
    for (label, q) in QUERIES {
        let n = q.matches('?').count();
        let params: Vec<Value> = (0..n).map(|i| Value::Blob(vec![0u8; 32 + i])).collect();
        let mut rows = conn.query(&format!("EXPLAIN QUERY PLAN {q}"), params_from_iter(params)).await?;
        let mut detail = Vec::new();
        while let Some(r) = rows.next().await? {
            detail.push(format!("{:?}", r.get_value(3)?));
        }
        println!("  {schema} {label}: {}", detail.join(" | "));
    }
    Ok(())
}

/// 1,000 lookups of existing (author, seq = 1) rows. Returns milliseconds.
async fn lookups(look: &mut Statement) -> Result<f64> {
    let t = Instant::now();
    for j in 0..1_000u32 {
        let mut rows = look.query((author(j), 1i64)).await?;
        while rows.next().await?.is_some() {}
    }
    Ok(t.elapsed().as_secs_f64() * 1e3)
}

async fn fill_and_time(conn: &Connection, label: &str) -> Result<()> {
    conn.execute("BEGIN IMMEDIATE", ()).await?;
    let mut ins = conn.prepare("INSERT INTO author_seq (author, seq, id) VALUES (?1, ?2, ?3)").await?;
    for i in 0..20_000u32 {
        ins.execute((author(i), 1i64, vec![1u8; 32])).await?;
        if i + 1 == 1_000 {
            let mut look = conn.prepare(LOOKUP).await?;
            println!("  {label}: 1000 lookups with 1000 rows: {:.1} ms", lookups(&mut look).await?);
        }
    }
    conn.execute("COMMIT", ()).await?;
    let mut look = conn.prepare(LOOKUP).await?;
    println!("  {label}: 1000 lookups with 20000 rows: {:.1} ms", lookups(&mut look).await?);
    Ok(())
}

pub fn plan(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, schema, without_rowid, label) in [
        ("plan-without-rowid.db", SCHEMA, true, "WITHOUT ROWID"),
        ("plan-rowid.db", SCHEMA_ROWID, false, "rowid"),
    ] {
        let p = dir.join(name);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", p.display()));
        }
        let uri = p.to_str().context("non-UTF-8 path")?.to_string();
        block_on(async {
            let conn = Builder::new_local(&uri)
                .experimental_without_rowid(without_rowid)
                .build()
                .await?
                .connect()?;
            conn.execute_batch(schema).await?;
            plans(&conn, label).await?;
            fill_and_time(&conn, label).await
        })?;
    }
    Ok(())
}
