//! Turso-only probes, each a subcommand. They print what turso does, for RESULTS.md:
//! the defaults on a plain open, the experimental flags, WITHOUT ROWID limits against a
//! rowid table with the same unique rule, encryption at rest, the open-time file lock
//! across processes, and two processes writing through the experimental multi-process WAL.

use crate::block::{PENDING_POLLS, block_on};
use crate::durability::expected_claim;
use crate::keys::Claim;
use crate::status_kb;
use crate::store::Store;
use crate::turso_store::TursoStore;
use anyhow::{Context, Result, bail, ensure};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::Instant;
use turso::{Builder, EncryptionOpts};

/// 32 bytes of hex, for the encryption probe. A test key only.
const PROBE_HEXKEY: &str = "2d7a30108d3eb3e45c90a732041fe54778bdcf707c76749fab7da335d1b39c1d";

fn reset(dir: &Path, name: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    for suffix in ["", "-wal", "-shm", ".tshm", "-log"] {
        let _ = std::fs::remove_file(dir.join(format!("{name}{suffix}")));
    }
    Ok(dir.join(name))
}

fn uri_of(p: &Path) -> Result<String> {
    Ok(p.to_str().context("non-UTF-8 path")?.to_string())
}

/// Prints one turso result as OK or ERR.
fn show<T: std::fmt::Debug>(label: &str, r: turso::Result<T>) {
    match r {
        Ok(v) => println!("  {label}: OK {v:?}"),
        Err(e) => println!("  {label}: ERR {e}"),
    }
}

/// Prints the lines a helper process wrote, with a label.
fn show_lines(label: &str, lines: &[String]) {
    for l in lines {
        println!("  {label}: {l}");
    }
}

/// Defaults, flags, and the WITHOUT ROWID rules. Prints each result.
pub fn features(dir: &Path) -> Result<()> {
    let s6 = TursoStore::open_with(&reset(dir, "s6-settings.db")?, false)?;
    println!("== settings after TursoStore::open (the S6 settings): {}", s6.settings()?);
    drop(s6);
    println!("== defaults: plain Builder::new_local, no pragmas");
    let uri = uri_of(&reset(dir, "defaults.db")?)?;
    block_on(async {
        let db = Builder::new_local(&uri).build().await?;
        let conn = db.connect()?;
        let mut rows = conn.query("PRAGMA journal_mode", ()).await?;
        if let Some(r) = rows.next().await? {
            println!("  journal_mode: {:?}", r.get_value(0)?);
        }
        let mut rows = conn.query("PRAGMA synchronous", ()).await?;
        if let Some(r) = rows.next().await? {
            println!("  synchronous (2 = FULL): {:?}", r.get_value(0)?);
        }
        Ok::<_, turso::Error>(())
    })?;

    println!("== WITHOUT ROWID without experimental_without_rowid");
    let uri = uri_of(&reset(dir, "no-flag.db")?)?;
    block_on(async {
        let db = Builder::new_local(&uri).build().await?;
        let conn = db.connect()?;
        show(
            "CREATE TABLE ... WITHOUT ROWID",
            conn.execute("CREATE TABLE w (k BLOB PRIMARY KEY) WITHOUT ROWID", ()).await,
        );
        Ok::<_, turso::Error>(())
    })?;

    println!("== WITHOUT ROWID with experimental_without_rowid (the flag the S6 schema needs)");
    let uri = uri_of(&reset(dir, "wr.db")?)?;
    block_on(async {
        let db = Builder::new_local(&uri).experimental_without_rowid(true).build().await?;
        let conn = db.connect()?;
        show(
            "CREATE",
            conn.execute("CREATE TABLE w (k BLOB PRIMARY KEY, v BLOB) WITHOUT ROWID", ()).await,
        );
        show("INSERT x2", async {
            conn.execute("INSERT INTO w VALUES (x'01', x'aa')", ()).await?;
            conn.execute("INSERT INTO w VALUES (x'02', x'bb')", ()).await
        }
        .await);
        show("UPDATE", conn.execute("UPDATE w SET v = x'cc' WHERE k = x'01'", ()).await);
        show("DELETE", conn.execute("DELETE FROM w WHERE k = x'01'", ()).await);
        show(
            "secondary UNIQUE (CREATE UNIQUE INDEX)",
            conn.execute("CREATE UNIQUE INDEX wv ON w (v)", ()).await,
        );
        Ok::<_, turso::Error>(())
    })?;

    println!("== rowid table with the same unique rule (no flag needed)");
    let uri = uri_of(&reset(dir, "rowid.db")?)?;
    block_on(async {
        let db = Builder::new_local(&uri).build().await?;
        let conn = db.connect()?;
        show(
            "CREATE rowid author_seq",
            conn.execute(
                "CREATE TABLE rw (author BLOB NOT NULL, seq INTEGER NOT NULL, id BLOB NOT NULL, PRIMARY KEY (author, seq))",
                (),
            )
            .await,
        );
        show("INSERT (a,1)", conn.execute("INSERT INTO rw VALUES (x'a1', 1, x'01')", ()).await);
        show(
            "INSERT (a,1) again, other id",
            conn.execute("INSERT INTO rw VALUES (x'a1', 1, x'02')", ()).await,
        );
        show("UPDATE", conn.execute("UPDATE rw SET id = x'03' WHERE seq = 1", ()).await);
        show("DELETE", conn.execute("DELETE FROM rw WHERE seq = 1", ()).await);
        let mut rows = conn.query("PRAGMA index_list('rw')", ()).await?;
        while let Some(r) = rows.next().await? {
            println!(
                "  index_list row: name={:?} unique={:?} origin={:?}",
                r.get_value(1)?,
                r.get_value(2)?,
                r.get_value(3)?
            );
        }
        Ok::<_, turso::Error>(())
    })?;

    println!("== encryption at rest (experimental_encryption + with_encryption, aes256gcm)");
    let p = reset(dir, "enc.db")?;
    let uri = uri_of(&p)?;
    let marker = b"secret-marker-7731";
    let enc = |hexkey: &str| EncryptionOpts { cipher: "aes256gcm".to_string(), hexkey: hexkey.to_string() };
    block_on(async {
        let db = Builder::new_local(&uri)
            .experimental_encryption(true)
            .with_encryption(enc(PROBE_HEXKEY))
            .build()
            .await?;
        let conn = db.connect()?;
        conn.execute("CREATE TABLE t (x BLOB)", ()).await?;
        conn.execute("INSERT INTO t VALUES (?1)", (marker.to_vec(),)).await?;
        Ok::<_, turso::Error>(())
    })?;
    let raw = std::fs::read(&p)?;
    let wal = std::fs::read(format!("{}-wal", p.display())).unwrap_or_default();
    println!("  file header (16 bytes): {:?}", String::from_utf8_lossy(&raw[..16.min(raw.len())]));
    println!("  marker visible in db file: {}", raw.windows(marker.len()).any(|w| w == marker));
    println!("  db-wal file bytes: {}; marker visible in db-wal: {}", wal.len(), wal.windows(marker.len()).any(|w| w == marker));
    let reopen = |key: Option<&str>| -> std::result::Result<String, String> {
        block_on(async {
            let mut b = Builder::new_local(&uri).experimental_encryption(true);
            if let Some(k) = key {
                b = b.with_encryption(enc(k));
            }
            let db = b.build().await.map_err(|e| format!("open: {e}"))?;
            let conn = db.connect().map_err(|e| format!("connect: {e}"))?;
            let mut rows = conn.query("SELECT x FROM t", ()).await.map_err(|e| format!("query: {e}"))?;
            let row = rows.next().await.map_err(|e| format!("next: {e}"))?;
            Ok(match row {
                Some(r) => format!("read {:?}", r.get_value(0).map_err(|e| e.to_string())?),
                None => "no rows".to_string(),
            })
        })
    };
    println!("  reopen with correct key: {:?}", reopen(Some(PROBE_HEXKEY)));
    println!("  reopen with no key: {:?}", reopen(None));
    let wrong = "00".repeat(32);
    println!("  reopen with wrong key: {:?}", reopen(Some(&wrong)));
    Ok(())
}

/// Opens while another process holds the file (run during `concurrency`'s hold). A
/// read-write open, then a read-only open. Prints what each returns.
pub fn probe(path: &Path) -> Result<()> {
    let t = Instant::now();
    match TursoStore::open_with(path, false) {
        Ok(_) => println!("  rw open: OK ({:.1} ms)", t.elapsed().as_secs_f64() * 1e3),
        Err(e) => println!("  rw open: ERR after {:.1} ms: {e:#}", t.elapsed().as_secs_f64() * 1e3),
    }
    let uri = uri_of(path)?;
    let ro = block_on(async {
        let db = Builder::new_local(&uri)
            .experimental_without_rowid(true)
            .read_only(true)
            .build()
            .await?;
        let conn = db.connect()?;
        let mut rows = conn.query("SELECT count(*) FROM claims", ()).await?;
        let row = rows.next().await?.context("no row")?;
        Ok::<_, anyhow::Error>(format!("{:?}", row.get_value(0)?))
    });
    match ro {
        Ok(c) => println!("  ro open: OK, count(claims) = {c}"),
        Err(e) => println!("  ro open: ERR {e:#}"),
    }
    Ok(())
}

/// A second read-write writer. Prints the open result and, if it opened, the insert result.
pub fn write_try(path: &Path, from: u64, to: u64) -> Result<()> {
    let claims: Vec<Claim> = (from..=to).map(expected_claim).collect();
    let t = Instant::now();
    match TursoStore::open_with(path, false) {
        Err(e) => println!("  write: open ERR after {:.1} ms: {e:#}", t.elapsed().as_secs_f64() * 1e3),
        Ok(mut store) => match store.insert_batch(&claims) {
            Ok(b) => println!("  write: OK {b:?} elapsed_ms={:.1}", t.elapsed().as_secs_f64() * 1e3),
            Err(e) => println!("  write: insert ERR {e:#}"),
        },
    }
    Ok(())
}

/// Diagnostic: is the (author, seq) lookup in insert_in a primary-key seek? Prints the plan
/// for the lookup, then times 1,000 lookups against a 1,000-row and a 20,000-row table.
/// If the time grows with the row count, the lookup scans the table.
pub fn plan_probe(dir: &Path) -> Result<()> {
    let p = reset(dir, "plan.db")?;
    let uri = uri_of(&p)?;
    block_on(async {
        let conn = Builder::new_local(&uri).experimental_without_rowid(true).build().await?.connect()?;
        conn.execute_batch(crate::turso_store::SCHEMA).await?;
        for (label, q) in [
            ("author_seq lookup", "SELECT id FROM author_seq WHERE author = ?1 AND seq = ?2"),
            ("claims by id", "SELECT body FROM claims WHERE id = ?1"),
            ("alpha range", "SELECT key FROM alpha WHERE key >= ?1 AND key < ?2 ORDER BY key"),
        ] {
            print!("  WITHOUT ROWID {label}:");
            let mut rows = conn.query(&format!("EXPLAIN QUERY PLAN {q}"), (vec![0u8; 32], vec![0u8; 33])).await?;
            while let Some(r) = rows.next().await? {
                print!(" [{:?}]", r.get_value(3)?);
            }
            println!();
        }
        let rconn = Builder::new_local(&format!("{uri}.rowid")).build().await?.connect()?;
        rconn.execute_batch(crate::turso_store::SCHEMA_ROWID).await?;
        for (label, q) in [
            ("author_seq lookup", "SELECT id FROM author_seq WHERE author = ?1 AND seq = ?2"),
            ("claims by id", "SELECT body FROM claims WHERE id = ?1"),
            ("alpha range", "SELECT key FROM alpha WHERE key >= ?1 AND key < ?2 ORDER BY key"),
        ] {
            print!("  rowid {label}:");
            let mut rows = rconn.query(&format!("EXPLAIN QUERY PLAN {q}"), (vec![0u8; 32], vec![0u8; 33])).await?;
            while let Some(r) = rows.next().await? {
                print!(" [{:?}]", r.get_value(3)?);
            }
            println!();
        }
        let conn = rconn;
        let sql = "SELECT id FROM author_seq WHERE author = ?1 AND seq = ?2";
        let mut rows = conn.query(&format!("EXPLAIN QUERY PLAN {sql}"), (vec![0u8; 32], 1i64)).await?;
        while let Some(r) = rows.next().await? {
            println!("  plan: {:?}", r.get_value(3)?);
        }
        let author = |i: u32| {
            let mut a = vec![0u8; 32];
            a[..4].copy_from_slice(&i.to_be_bytes());
            a
        };
        let mut inserted = 0u32;
        let mut ins = conn.prepare("INSERT INTO author_seq (author, seq, id) VALUES (?1, ?2, ?3)").await?;
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        for i in 0..20_000u32 {
            ins.execute((author(i), 1i64, vec![1u8; 32])).await?;
            inserted += 1;
            if inserted == 1_000 {
                let t = Instant::now();
                let mut look = conn.prepare(sql).await?;
                for j in 0..1_000u32 {
                    let mut rows = look.query((author(j), 1i64)).await?;
                    while rows.next().await?.is_some() {}
                }
                println!("  lookups with 1000 rows: {:.1} ms per 1000", t.elapsed().as_secs_f64() * 1e3);
            }
        }
        conn.execute("COMMIT", ()).await?;
        let t = Instant::now();
        let mut look = conn.prepare(sql).await?;
        for j in 0..1_000u32 {
            let mut rows = look.query((author(j), 1i64)).await?;
            while rows.next().await?.is_some() {}
        }
        println!("  lookups with {inserted} rows: {:.1} ms per 1000", t.elapsed().as_secs_f64() * 1e3);
        Ok::<_, anyhow::Error>(())
    })
}

/// Diagnostic: one table, 1000 inserts three ways. Explicit BEGIN IMMEDIATE / COMMIT SQL,
/// the Transaction API, and plain autocommit. Shows whether the Transaction API keeps the
/// statements in one transaction, which decides whether each statement commits.
pub fn txn_probe(dir: &Path) -> Result<()> {
    let p = reset(dir, "txn.db")?;
    let uri = uri_of(&p)?;
    block_on(async {
        let mut conn = Builder::new_local(&uri).experimental_without_rowid(true).build().await?.connect()?;
        conn.pragma_update("journal_mode", "'wal'").await?;
        conn.pragma_update("synchronous", "FULL").await?;
        conn.execute("CREATE TABLE t (k BLOB PRIMARY KEY, v BLOB)", ()).await?;
        let row = |i: u32| (i.to_be_bytes().to_vec(), vec![1u8; 64]);
        conn.execute("CREATE TABLE w (k BLOB PRIMARY KEY, v BLOB) WITHOUT ROWID", ()).await?;
        conn.execute("CREATE TABLE s (k BLOB PRIMARY KEY, v BLOB)", ()).await?;
        conn.execute("CREATE TABLE ws (k BLOB PRIMARY KEY, v BLOB) WITHOUT ROWID", ()).await?;

        // D: WITHOUT ROWID inserts only, one transaction.
        let t = Instant::now();
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        for i in 0..1000 {
            conn.execute("INSERT INTO w VALUES (?1, ?2)", row(i)).await?;
        }
        conn.execute("COMMIT", ()).await?;
        println!("  D WITHOUT ROWID inserts, one transaction: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);

        // E: rowid table, a SELECT before each INSERT, one transaction (the insert_in pattern).
        let t = Instant::now();
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        for i in 0..1000u32 {
            let mut rows = conn.query("SELECT v FROM s WHERE k = ?1", (i.to_be_bytes().to_vec(),)).await?;
            while rows.next().await?.is_some() {}
            conn.execute("INSERT INTO s VALUES (?1, ?2)", row(i)).await?;
        }
        conn.execute("COMMIT", ()).await?;
        println!("  E rowid, SELECT then INSERT, one transaction: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);

        // F: WITHOUT ROWID table, a SELECT before each INSERT, one transaction.
        let t = Instant::now();
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        for i in 0..1000u32 {
            let mut rows = conn.query("SELECT v FROM ws WHERE k = ?1", (i.to_be_bytes().to_vec(),)).await?;
            while rows.next().await?.is_some() {}
            conn.execute("INSERT INTO ws VALUES (?1, ?2)", row(i)).await?;
        }
        conn.execute("COMMIT", ()).await?;
        println!("  F WITHOUT ROWID, SELECT then INSERT, one transaction: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);

        // G: prepared once and reused, one transaction, WITHOUT ROWID inserts only.
        conn.execute("CREATE TABLE pw (k BLOB PRIMARY KEY, v BLOB) WITHOUT ROWID", ()).await?;
        let t = Instant::now();
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        let mut ins = conn.prepare("INSERT INTO pw VALUES (?1, ?2)").await?;
        for i in 0..1000u32 {
            ins.execute(row(i)).await?;
        }
        conn.execute("COMMIT", ()).await?;
        println!("  G prepared INSERT reused, WITHOUT ROWID, one transaction: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);

        // H: prepared SELECT and prepared INSERT, reused, WITHOUT ROWID (the insert_in pattern).
        conn.execute("CREATE TABLE ph (k BLOB PRIMARY KEY, v BLOB) WITHOUT ROWID", ()).await?;
        let t = Instant::now();
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        let mut sel = conn.prepare("SELECT v FROM ph WHERE k = ?1").await?;
        let mut ins = conn.prepare("INSERT INTO ph VALUES (?1, ?2)").await?;
        for i in 0..1000u32 {
            let mut rows = sel.query((i.to_be_bytes().to_vec(),)).await?;
            while rows.next().await?.is_some() {}
            ins.execute(row(i)).await?;
        }
        conn.execute("COMMIT", ()).await?;
        println!("  H prepared SELECT+INSERT reused, WITHOUT ROWID, one transaction: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);

        let t = Instant::now();
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        println!("  explicit: is_autocommit inside BEGIN = {:?}", conn.is_autocommit());
        for i in 0..1000 {
            conn.execute("INSERT INTO t VALUES (?1, ?2)", row(i)).await?;
        }
        conn.execute("COMMIT", ()).await?;
        println!("  A explicit BEGIN IMMEDIATE/COMMIT, 1000 inserts: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);

        let t = Instant::now();
        {
            let tx = conn.transaction_with_behavior(turso::transaction::TransactionBehavior::Immediate).await?;
            println!("  API: is_autocommit inside Transaction = {:?}", tx.is_autocommit());
            for i in 1000..2000 {
                tx.execute("INSERT INTO t VALUES (?1, ?2)", row(i)).await?;
            }
            tx.commit().await?;
        }
        println!("  B Transaction API, 1000 inserts: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);

        let t = Instant::now();
        for i in 2000..3000 {
            conn.execute("INSERT INTO t VALUES (?1, ?2)", row(i)).await?;
        }
        println!("  C autocommit, 1000 inserts: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        let mut rows = conn.query("SELECT count(*) FROM t", ()).await?;
        if let Some(r) = rows.next().await? {
            println!("  rows after A+B+C: {:?}", r.get_value(0)?);
        }
        Ok::<_, anyhow::Error>(())
    })
}

/// Diagnostic for a stall: `batches` batches of `batch` claims, one line per batch (time,
/// pending polls, WAL bytes), so the last line names the batch that hangs.
pub fn stress(path: &Path, batches: u64, batch: u64, pragmas: &[String]) -> Result<()> {
    let mut store = TursoStore::open(path)?;
    println!("start: PRAGMA cache_size = {:?}", store.query_int("PRAGMA cache_size"));
    for p in pragmas {
        store.exec(&format!("PRAGMA {p}"))?;
        println!("set: PRAGMA {p}");
    }
    println!("now: PRAGMA cache_size = {:?}", store.query_int("PRAGMA cache_size"));
    let wal = format!("{}-wal", path.display());
    let t0 = Instant::now();
    for b in 0..batches {
        let claims: Vec<Claim> = (b * batch + 1..=(b + 1) * batch).map(expected_claim).collect();
        let before = PENDING_POLLS.load(Ordering::Relaxed);
        let t = Instant::now();
        let r = store.insert_batch(&claims)?;
        println!(
            "batch {b}: inserted={} ms={:.1} pending_polls={} wal_bytes={} total_s={:.1}",
            r.inserted,
            t.elapsed().as_secs_f64() * 1e3,
            PENDING_POLLS.load(Ordering::Relaxed) - before,
            std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0),
            t0.elapsed().as_secs_f64()
        );
    }
    Ok(())
}

/// One writer in the multi-process WAL. Commits `from..=to` in batches of `batch`.
pub fn mp_write(path: &Path, from: u64, to: u64, batch: u64) -> Result<()> {
    let mut store = TursoStore::open_with(path, true)?;
    let t = Instant::now();
    let mut n = 0u64;
    let mut seq = from;
    while seq <= to {
        let end = (seq + batch - 1).min(to);
        let claims: Vec<Claim> = (seq..=end).map(expected_claim).collect();
        n += store.insert_batch(&claims)?.inserted as u64;
        seq = end + 1;
    }
    println!(
        "mp_write: inserted={n} elapsed_ms={:.1} pending_polls={}",
        t.elapsed().as_secs_f64() * 1e3,
        PENDING_POLLS.load(Ordering::Relaxed)
    );
    Ok(())
}

/// Two processes write the same file through the multi-process WAL, at once. Afterwards
/// the parent reopens the file in the default mode and checks every claim.
pub fn mp_test(dir: &Path) -> Result<()> {
    let p = reset(dir, "mp.db")?;
    // The parent creates the schema first, so the two children do not race on it.
    drop(TursoStore::open_with(&p, true)?);
    let exe = std::env::current_exe()?;
    let p_str = uri_of(&p)?;
    let spawn = |from: u64, to: u64| -> Result<Child> {
        Ok(Command::new(&exe)
            .args(["turso-mp-write", &p_str, &from.to_string(), &to.to_string(), "100"])
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?)
    };
    let a = spawn(1, 1000)?;
    let b = spawn(1001, 2000)?;
    for child in [a, b] {
        let out = child.wait_with_output()?;
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            println!("  child: {line}");
        }
        ensure!(out.status.success(), "a writer exited with {}", out.status);
    }
    let store = TursoStore::open(&p).context("default-mode reopen after both writers exited")?;
    let claims = store.all_claims()?;
    let mismatched = claims.iter().filter(|c| **c != expected_claim(c.seq)).count();
    println!(
        "  reopen (default mode): claims={} counts={:?} mismatched={mismatched}",
        claims.len(),
        store.counts()?
    );
    Ok(())
}

/// A helper process whose stdout is read line by line.
struct Proc {
    child: Child,
    out: BufReader<ChildStdout>,
}

impl Proc {
    fn start(args: &[&str], env: &[(&str, &str)]) -> Result<Proc> {
        let mut cmd = Command::new(std::env::current_exe()?);
        cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::inherit());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().with_context(|| format!("spawning {args:?}"))?;
        let out = BufReader::new(child.stdout.take().context("child has no stdout")?);
        Ok(Proc { child, out })
    }

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

fn run_to_end(args: &[&str], env: &[(&str, &str)]) -> Result<Vec<String>> {
    Proc::start(args, env)?.finish()
}

/// The open-time lock, across processes. A holder keeps one IMMEDIATE transaction open
/// while a second process probes and writes. Then the same probe runs with
/// LIMBO_DISABLE_FILE_LOCK=1, the environment switch turso reads at open.
pub fn concurrency(dir: &Path) -> Result<()> {
    let p = reset(dir, "conc.db")?;
    let p_str = uri_of(&p)?;
    {
        let mut store = TursoStore::open(&p)?;
        let claims: Vec<Claim> = (1..=1000).map(expected_claim).collect();
        let b = store.insert_batch(&claims)?;
        println!("setup: committed {} claims", b.inserted);
    }
    println!("== holder: IMMEDIATE transaction with 1000 uncommitted claims, 4 s");
    let mut holder = Proc::start(&["turso-hold", &p_str, "4", "1001", "2000"], &[])?;
    println!("  holder: {}", holder.wait_for("ready")?);
    show_lines("probe while held", &run_to_end(&["turso-probe", &p_str], &[])?);
    show_lines("second writer while held", &run_to_end(&["turso-write", &p_str, "2001", "2100"], &[])?);
    println!("  holder: {}", holder.wait_for("released")?);
    show_lines("holder exit", &holder.finish()?);
    show_lines("second writer after release", &run_to_end(&["turso-write", &p_str, "3001", "4000"], &[])?);

    println!("== holder again, then the probe with LIMBO_DISABLE_FILE_LOCK=1");
    let mut holder = Proc::start(&["turso-hold", &p_str, "3", "4001", "5000"], &[])?;
    println!("  holder: {}", holder.wait_for("ready")?);
    show_lines(
        "probe with lock disabled",
        &run_to_end(&["turso-probe", &p_str], &[("LIMBO_DISABLE_FILE_LOCK", "1")])?,
    );
    println!("  holder: {}", holder.wait_for("released")?);
    show_lines("holder exit", &holder.finish()?);
    println!("rss_kb (VmRSS) at end of concurrency: {}", status_kb("VmRSS:"));
    Ok(())
}
