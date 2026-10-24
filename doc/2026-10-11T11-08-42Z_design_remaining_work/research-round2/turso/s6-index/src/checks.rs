//! Self-checks. `selftest` tests the key encodings with no store involved: the body
//! round-trips, alpha and omega order equal tuple order, and prefix ranges return
//! exactly the claims they should. `unique_for` shows how each store keeps
//! (author, seq) unique.

use crate::Rng;
use crate::keys::{self, Claim, Id};
use crate::redb_store::RedbStore;
use crate::sqlite_store::SqliteStore;
use crate::turso_store::{TursoRowidStore, TursoStore};
use crate::store::{Batch, Store};
use anyhow::{Context, Result, bail, ensure};
use std::path::{Path, PathBuf};

const A: Id = [0xA1; 32];
const B: Id = [0xB2; 32];
const S1: Id = [0x51; 32];
const S2: Id = [0x52; 32];

fn claim(author: Id, seq: u64, subject: Id, name: &str, value: Option<&str>) -> Claim {
    Claim {
        author,
        seq,
        subject,
        name: name.as_bytes().to_vec(),
        value: value.map(|v| v.as_bytes().to_vec()),
    }
}

/// Claims whose names and values are built from bytes that stress the escapes:
/// 0x00, 0x01, 0x02, 0x03, 0xFE and 0xFF. Subjects share long prefixes.
fn adversarial(n: usize, seed: u64) -> Vec<Claim> {
    const BYTES: &[u8] = &[0x00, 0x01, 0x02, 0x03, b'a', 0xFE, 0xFF];
    let mut rng = Rng::new(seed);
    let subjects: Vec<Id> = (0..6u8)
        .map(|i| {
            let mut s = [0u8; 32];
            s[0] = if i % 2 == 0 { 0x00 } else { 0xFF };
            s[1] = i;
            s[31] = rng.below(3) as u8;
            s
        })
        .collect();
    (0..n)
        .map(|i| {
            let name_len = rng.below(4) as usize;
            let name = rng.bytes(name_len, BYTES);
            let value = if rng.below(4) == 0 {
                None
            } else {
                let len = rng.below(5) as usize;
                Some(rng.bytes(len, BYTES))
            };
            let subject = subjects[rng.below(subjects.len() as u64) as usize];
            Claim { author: [0x77; 32], seq: i as u64 + 1, subject, name, value }
        })
        .collect()
}

pub fn selftest() -> Result<()> {
    let claims = adversarial(5000, 0x5E1F);
    let n = claims.len();
    let ids: Vec<Id> = claims.iter().map(Claim::id).collect();
    for c in &claims {
        ensure!(Claim::from_body(&c.body())? == *c, "body round trip failed at seq {}", c.seq);
    }

    let alpha: Vec<Vec<u8>> = (0..n).map(|i| keys::alpha_key(&claims[i], &ids[i])).collect();
    let omega: Vec<Vec<u8>> = (0..n).map(|i| keys::omega_key(&claims[i], &ids[i])).collect();

    let mut tuple_order: Vec<usize> = (0..n).collect();
    tuple_order.sort_by(|&x, &y| {
        (claims[x].subject, &claims[x].name, &claims[x].value, ids[x])
            .cmp(&(claims[y].subject, &claims[y].name, &claims[y].value, ids[y]))
    });
    let mut key_order: Vec<usize> = (0..n).collect();
    key_order.sort_by(|&x, &y| alpha[x].cmp(&alpha[y]));
    ensure!(tuple_order == key_order, "alpha key order differs from (subject, name, value, id)");

    let mut tuple_order: Vec<usize> = (0..n).collect();
    tuple_order.sort_by(|&x, &y| {
        (&claims[x].value, &claims[x].name, claims[x].subject, ids[x])
            .cmp(&(&claims[y].value, &claims[y].name, claims[y].subject, ids[y]))
    });
    let mut key_order: Vec<usize> = (0..n).collect();
    key_order.sort_by(|&x, &y| omega[x].cmp(&omega[y]));
    ensure!(tuple_order == key_order, "omega key order differs from (value, name, subject, id)");

    // Prefix ranges (binary search with prefix_end) must hold exactly the matching keys.
    let mut alpha_sorted = alpha.clone();
    alpha_sorted.sort();
    let mut omega_sorted = omega.clone();
    omega_sorted.sort();
    let range = |sorted: &[Vec<u8>], p: &[u8]| -> (usize, usize) {
        let lo = sorted.partition_point(|k| k.as_slice() < p);
        let hi = match keys::prefix_end(p) {
            Some(e) => sorted.partition_point(|k| k.as_slice() < e.as_slice()),
            None => sorted.len(),
        };
        (lo, hi)
    };
    for c in &claims {
        let (lo, hi) = range(&alpha_sorted, &c.subject);
        let want = claims.iter().filter(|d| d.subject == c.subject).count();
        ensure!(hi - lo == want, "alpha range for a subject holds {} keys, expected {want}", hi - lo);
        ensure!(alpha_sorted[lo..hi].iter().all(|k| k.starts_with(&c.subject)), "alpha range leaks");

        let (olo, ohi) = keys::omega_bounds(&c.name, c.value.as_deref());
        let lo = omega_sorted.partition_point(|k| k.as_slice() < olo.as_slice());
        let hi = omega_sorted.partition_point(|k| k.as_slice() < ohi.as_slice());
        let want = claims.iter().filter(|d| d.name == c.name && d.value == c.value).count();
        ensure!(hi - lo == want, "omega range for a (name, value) holds {} keys, expected {want}", hi - lo);
        ensure!(omega_sorted[lo..hi].iter().all(|k| k.starts_with(&olo)), "omega range leaks");
    }
    println!(
        "selftest: PASS ({n} claims: body round trip, alpha and omega order, prefix ranges)"
    );
    Ok(())
}

fn prepare(dir: &Path, name: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("unique-{name}.db"));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    Ok(path)
}

fn expect_ok<S: Store>(store: &mut S, label: &str, claims: &[Claim], want: Batch) -> Result<()> {
    let got = store.insert_batch(claims)?;
    ensure!(got == want, "{label}: got {got:?}, expected {want:?}");
    println!("  ok   {label}: {got:?}");
    Ok(())
}

fn expect_fork<S: Store>(store: &mut S, label: &str, claims: &[Claim]) -> Result<()> {
    match store.insert_batch(claims) {
        Err(e) => {
            println!("  ok   {label}: refused: {e:#}");
            Ok(())
        }
        Ok(b) => bail!("{label}: accepted ({b:?}), expected a fork refusal"),
    }
}

/// The shared (author, seq) cases. Each store runs them through the same trait calls.
fn cases<S: Store>(store: &mut S) -> Result<()> {
    let x1 = claim(A, 5, S1, "tag", Some("a"));
    let x2 = claim(A, 5, S2, "tag", Some("b"));
    let y = claim(A, 6, S1, "tag", Some("c"));
    let z1 = claim(B, 1, S1, "tag", Some("d"));
    let z2 = claim(B, 1, S2, "tag", Some("e"));

    expect_ok(store, "insert x1 at (A,5)", std::slice::from_ref(&x1), Batch { inserted: 1, duplicates: 0 })?;
    expect_ok(store, "replay x1 (identical)", std::slice::from_ref(&x1), Batch { inserted: 0, duplicates: 1 })?;
    expect_fork(store, "x2 at (A,5), held by x1", std::slice::from_ref(&x2))?;
    expect_fork(store, "batch [y at (A,6), x2 at (A,5)]", &[y.clone(), x2.clone()])?;
    expect_fork(store, "batch [z1, z2], both at (B,1)", &[z1.clone(), z2.clone()])?;

    let counts = store.counts()?;
    ensure!(counts == [1, 1, 1, 1], "counts after refused batches are {counts:?}");
    println!("  ok   counts after the refusals: {counts:?} (no part of a refused batch kept)");
    ensure!(store.by_author(&A)? == vec![x1.clone()], "by_author(A) changed");
    ensure!(store.by_subject(&S1)? == vec![x1.clone()], "by_subject(S1) changed");
    ensure!(store.subjects_with(b"tag", b"c")?.is_empty(), "y leaked into the index");
    println!("  ok   queries still show only x1");
    Ok(())
}

pub fn unique_for(kind: &str, dir: &Path) -> Result<()> {
    match kind {
        "redb" => {
            let path = prepare(dir, "redb")?;
            let mut store = RedbStore::open(&path).context("opening redb")?;
            println!("redb: (author, seq) cases");
            cases(&mut store)?;
            let x2 = claim(A, 5, S2, "tag", Some("b"));
            println!("  raw  bypass on (A,5): {}", store.bypass_check(&x2)?);
            let after = match store.by_author(&A) {
                Ok(v) => format!("{} claims", v.len()),
                Err(e) => format!("error: {e:#}"),
            };
            println!("  by_author(A) afterwards: {after}");
        }
        "sqlite" => {
            let path = prepare(dir, "sqlite")?;
            let mut store = SqliteStore::open_rw(&path)?;
            println!("sqlite: (author, seq) cases");
            cases(&mut store)?;
            for line in store.describe_unique()? {
                println!("  index on author_seq: {line}");
            }
            let x2 = claim(A, 5, S2, "tag", Some("b"));
            println!("  raw  INSERT at (A,5): {}", store.bypass_check(&x2)?);
        }
        "turso" => {
            let path = prepare(dir, "turso")?;
            let mut store = TursoStore::open(&path)?;
            println!("turso: (author, seq) cases");
            cases(&mut store)?;
            for line in store.describe_unique()? {
                println!("  index_list on author_seq: {line}");
            }
            let x2 = claim(A, 5, S2, "tag", Some("b"));
            println!("  raw  INSERT at (A,5): {}", store.bypass_check(&x2)?);
        }
        "turso-rowid" => {
            let path = prepare(dir, "turso-rowid")?;
            let mut store = TursoRowidStore::open(&path)?;
            println!("turso-rowid: (author, seq) cases");
            cases(&mut store)?;
            for line in store.0.describe_unique()? {
                println!("  index_list on author_seq: {line}");
            }
            let x2 = claim(A, 5, S2, "tag", Some("b"));
            println!("  raw  INSERT at (A,5): {}", store.0.bypass_check(&x2)?);
        }
        other => bail!("unknown store {other}"),
    }
    Ok(())
}
