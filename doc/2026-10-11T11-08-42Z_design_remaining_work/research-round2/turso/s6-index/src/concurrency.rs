//! Concurrency. A second process opens the same index file. redb takes a file lock, so
//! a second opener should be refused. SQLite WAL lets other processes read beside one
//! writer, and a second writer waits on the write lock. The helper functions below run
//! as child processes, started by `run`.

use crate::durability::expected_claim;
use crate::keys::Claim;
use crate::sqlite_store::{SqliteStore, insert_in, open_read_only};
use crate::store::Store;
use anyhow::{Context, Result, bail};
use redb::{Database, ReadOnlyDatabase};
use rusqlite::{Connection, TransactionBehavior};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

/// A helper process whose stdout is read line by line.
struct Proc {
    child: Child,
    out: BufReader<ChildStdout>,
}

impl Proc {
    fn start(args: &[&str]) -> Result<Proc> {
        let mut child = Command::new(std::env::current_exe()?)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("spawning {args:?}"))?;
        let out = BufReader::new(child.stdout.take().context("child has no stdout")?);
        Ok(Proc { child, out })
    }

    /// Reads lines until one starts with `prefix`, and returns that line.
    fn wait_for(&mut self, prefix: &str) -> Result<String> {
        let mut line = String::new();
        loop {
            line.clear();
            if self.out.read_line(&mut line)? == 0 {
                bail!("helper exited before printing '{prefix}'");
            }
            if line.starts_with(prefix) {
                return Ok(line.trim_end().to_string());
            }
        }
    }

    /// Returns the lines the helper still has to print, after it exits.
    fn finish(mut self) -> Result<Vec<String>> {
        let mut lines = Vec::new();
        let mut line = String::new();
        while self.out.read_line(&mut line)? > 0 {
            lines.push(line.trim_end().to_string());
            line.clear();
        }
        self.child.wait()?;
        Ok(lines)
    }
}

fn run_to_end(args: &[&str]) -> Result<Vec<String>> {
    Proc::start(args)?.finish()
}

fn show(label: &str, lines: &[String]) {
    for l in lines {
        println!("  {label}: {l}");
    }
}

fn flush() -> Result<()> {
    std::io::stdout().flush()?;
    Ok(())
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

// ---- helpers (child processes) ----

pub fn redb_hold(path: &Path, secs: u64) -> Result<()> {
    let db = Database::create(path)?;
    println!("ready");
    flush()?;
    std::thread::sleep(Duration::from_secs(secs));
    drop(db);
    println!("released");
    Ok(())
}

pub fn redb_probe(path: &Path) -> Result<()> {
    match Database::open(path) {
        Ok(_) => println!("Database::open: OK"),
        Err(e) => println!("Database::open: Err({e:?}): {e}"),
    }
    match ReadOnlyDatabase::open(path) {
        Ok(_) => println!("ReadOnlyDatabase::open: OK"),
        Err(e) => println!("ReadOnlyDatabase::open: Err({e:?}): {e}"),
    }
    Ok(())
}

pub fn sqlite_setup(path: &Path, rows: u64) -> Result<()> {
    let mut store = SqliteStore::open_rw(path)?;
    let claims: Vec<Claim> = (1..=rows).map(expected_claim).collect();
    let b = store.insert_batch(&claims)?;
    println!("setup: committed {} claims", b.inserted);
    Ok(())
}

/// A writer that inserts seqs `from..=to` uncommitted, holds the write lock for `secs`,
/// then rolls back.
pub fn sqlite_hold(path: &Path, secs: u64, from: u64, to: u64) -> Result<()> {
    let mut store = SqliteStore::open_rw(path)?;
    let tx = store
        .connection_mut()
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let claims: Vec<Claim> = (from..=to).map(expected_claim).collect();
    let b = insert_in(&tx, &claims)?;
    println!("ready: {} claims written, not committed", b.inserted);
    flush()?;
    std::thread::sleep(Duration::from_secs(secs));
    tx.rollback()?;
    println!("released: rolled back");
    Ok(())
}

fn count_claims(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT count(*) FROM claims", [], |r| r.get(0))?)
}

pub fn sqlite_read(path: &Path) -> Result<()> {
    let conn = open_read_only(path)?;
    let t = Instant::now();
    let n = count_claims(&conn)?;
    println!("read: count={n} elapsed_ms={:.2}", ms(t));
    Ok(())
}

/// A reader that holds one read transaction open across a writer's commit.
pub fn sqlite_read_hold(path: &Path, secs: u64) -> Result<()> {
    let conn = open_read_only(path)?;
    conn.execute_batch("BEGIN")?;
    println!("r1 count={}", count_claims(&conn)?);
    flush()?;
    std::thread::sleep(Duration::from_secs(secs));
    println!("r2 count={} (same read transaction)", count_claims(&conn)?);
    conn.execute_batch("COMMIT")?;
    println!("r3 count={} (after COMMIT)", count_claims(&conn)?);
    Ok(())
}

pub fn sqlite_write(path: &Path, busy_ms: u64, from: u64, to: u64) -> Result<()> {
    let mut store = SqliteStore::open_rw_with(path, Duration::from_millis(busy_ms))?;
    let claims: Vec<Claim> = (from..=to).map(expected_claim).collect();
    let t = Instant::now();
    match store.insert_batch(&claims) {
        Ok(b) => println!("write: OK {b:?} elapsed_ms={:.1}", ms(t)),
        Err(e) => println!("write: ERR after {:.1} ms: {e:#}", ms(t)),
    }
    Ok(())
}

// ---- orchestration ----

fn reset(dir: &Path, name: &str) -> Result<String> {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(dir.join(format!("{name}{suffix}")));
    }
    Ok(dir.join(name).to_str().context("non-UTF-8 path")?.to_string())
}

pub fn run(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;

    println!("== redb: a second process opens the file while a holder has it open");
    let rp = reset(dir, "redb.db")?;
    let mut holder = Proc::start(&["redb-hold", &rp, "2"])?;
    println!("  holder: {}", holder.wait_for("ready")?);
    show("probe", &run_to_end(&["redb-probe", &rp])?);
    show("holder", &holder.finish()?);

    println!("== sqlite (WAL, synchronous=FULL): readers and writers across processes");
    let sp = reset(dir, "sqlite.db")?;
    show("setup", &run_to_end(&["sqlite-setup", &sp, "1000"])?);
    let mut writer = Proc::start(&["sqlite-hold", &sp, "6", "1001", "2000"])?;
    println!("  writer: {}", writer.wait_for("ready")?);
    show("reader during the open write", &run_to_end(&["sqlite-read", &sp])?);
    let readers = (0..4)
        .map(|_| Proc::start(&["sqlite-read", &sp]))
        .collect::<Result<Vec<_>>>()?;
    for r in readers {
        show("4 concurrent readers", &r.finish()?);
    }
    show(
        "second writer, 500 ms busy timeout",
        &run_to_end(&["sqlite-write", &sp, "500", "2001", "2100"])?,
    );
    println!("  writer: {}", writer.wait_for("released")?);
    show("writer exit", &writer.finish()?);
    show(
        "second writer after release, 10 s busy timeout",
        &run_to_end(&["sqlite-write", &sp, "10000", "2001", "3000"])?,
    );

    println!("== sqlite: a reader holds a snapshot while a writer commits");
    let mut reader = Proc::start(&["sqlite-read-hold", &sp, "3"])?;
    println!("  reader: {}", reader.wait_for("r1")?);
    show(
        "writer during the open read",
        &run_to_end(&["sqlite-write", &sp, "10000", "3001", "4000"])?,
    );
    show("reader", &reader.finish()?);
    Ok(())
}
