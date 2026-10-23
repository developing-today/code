//! The "what could be missed" checks, on real stores over iroh (except (b), which runs the
//! scheme in memory on crafted IDs).
//!
//! (a) a chain with a sequence gap on one side
//! (b) XOR cancellation: same fingerprint, with and without the count check
//! (c) bounds at keys containing 0x00 bytes
//! (d) an interrupted session, restarted from scratch
//!
//! Usage: scenarios <dir> <base-1000.redb>

use anyhow::{Context, Result, bail};
use p2panda_core::SigningKey;
use s7_range_real::chain::{self, EVENTS_PER_CHAIN, chain_events, chain_key, split_record};
use s7_range_real::range::{Party, simulate};
use s7_range_real::session::{Cut, Params, Run, Side, sync_pair};
use s7_range_real::store::{Id, Item, Key, Store, key_of};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Default)]
struct Report {
    failures: usize,
}

impl Report {
    fn check(&mut self, name: &str, ok: bool, detail: impl AsRef<str>) {
        if ok {
            println!("  PASS {name}: {}", detail.as_ref());
        } else {
            self.failures += 1;
            println!("  FAIL {name}: {}", detail.as_ref());
        }
    }
}

/// Copies `base` to `dst` and returns its path.
fn copy_to(base: &Path, dst: PathBuf) -> Result<PathBuf> {
    let _ = std::fs::remove_file(&dst);
    std::fs::copy(base, &dst)?;
    Ok(dst)
}

/// Adds events `EVENTS_PER_CHAIN..EVENTS_PER_CHAIN + count` on each `(key, count)` to a store.
/// Returns the IDs added.
fn add_next(path: &Path, extra: &[(&SigningKey, u32)]) -> Result<HashSet<Id>> {
    let store = Store::open(path)?;
    let mut items = Vec::new();
    let mut ids = HashSet::new();
    for (key, count) in extra {
        let events = chain_events(key, EVENTS_PER_CHAIN + count);
        for ev in events.iter().skip(EVENTS_PER_CHAIN as usize) {
            let v = chain::verify(&ev.header, &ev.body)?;
            ids.insert(v.id);
            items.push(Item {
                key: key_of(&v.author, v.seq),
                id: v.id,
                record: chain::record(&ev.header, &ev.body),
            });
        }
    }
    store.insert(&items)?;
    Ok(ids)
}

/// Checks each author's rows: a missing seq counts as a gap, and a row whose predecessor is held
/// must link to it by backlink. Returns (gaps, bad links).
fn audit(store: &Store) -> Result<(usize, usize)> {
    let mut gaps = 0usize;
    let mut bad = 0usize;
    let mut prev: Option<([u8; 32], u32, Id)> = None;
    for (key, id) in store.pairs()? {
        let author: [u8; 32] = key[..32].try_into()?;
        let seq = u64::from_be_bytes(key[32..].try_into()?) as u32;
        let same = matches!(prev, Some((a, _, _)) if a == author);
        if same {
            let (_, ps, pid) = prev.context("prev")?;
            if seq > ps + 1 {
                gaps += (seq - ps - 1) as usize;
            }
            if seq == ps + 1 {
                let rec = store.record(&id)?;
                let (header, body) = split_record(&rec)?;
                if chain::verify(header, body)?.backlink != Some(pid) {
                    bad += 1;
                }
            }
        } else if seq != 0 {
            gaps += seq as usize;
        }
        prev = Some((author, seq, id));
    }
    Ok((gaps, bad))
}

async fn sync_paths(a: &Path, b: &Path, p: Params, cut: Cut) -> Result<(Run, Arc<Store>, Arc<Store>)> {
    let store_a = Arc::new(Store::open(a)?);
    let party_a = Arc::new(store_a.index()?);
    let store_b = Arc::new(Store::open(b)?);
    let party_b = Arc::new(store_b.index()?);
    let run = sync_pair(
        Side { store: store_a.clone(), party: party_a },
        Side { store: store_b.clone(), party: party_b },
        p,
        cut,
    )
    .await?;
    Ok((run, store_a, store_b))
}

fn same_rows(a: &Store, b: &Store) -> Result<bool> {
    Ok(a.pairs()? == b.pairs()? && a.counts()? == b.counts()?)
}

// (a) ------------------------------------------------------------------------------------------

async fn scenario_a(dir: &Path, base: &Path, rep: &mut Report) -> Result<()> {
    println!("(a) sequence gap: chain 7 lacks seq 20 on the acceptor, and has seq 21..49");
    let victim = chain_key(7);
    let author = *victim.verifying_key().as_bytes();
    let truth: HashSet<Id> = [chain_events(&victim, EVENTS_PER_CHAIN)[20].id()].into_iter().collect();

    let a = copy_to(base, dir.join("a-gap.redb"))?;
    let b = copy_to(base, dir.join("b-gap.redb"))?;
    Store::open(&b)?.remove(&key_of(&author, 20))?;

    let store_b = Store::open(&b)?;
    let (gaps_before, bad_before) = audit(&store_b)?;
    rep.check("gap is present before sync", gaps_before == 1 && bad_before == 0, format!("gaps={gaps_before} bad links={bad_before}"));
    drop(store_b);

    // Height view: the largest seq per author, as a height-based Have message would carry.
    let heights = |path: &Path| -> Result<std::collections::BTreeMap<[u8; 32], u32>> {
        let store = Store::open(path)?;
        let mut h = std::collections::BTreeMap::new();
        for (key, _) in store.pairs()? {
            let author: [u8; 32] = key[..32].try_into()?;
            let seq = u64::from_be_bytes(key[32..].try_into()?) as u32;
            let e = h.entry(author).or_insert(0);
            *e = (*e).max(seq);
        }
        Ok(h)
    };
    let (ha, hb) = (heights(&a)?, heights(&b)?);
    let height_diffs = ha.iter().filter(|(k, v)| hb.get(*k) != Some(v)).count();
    rep.check(
        "height-only comparison sees no difference",
        height_diffs == 0,
        format!("authors whose largest seq differs: {height_diffs} of {}", ha.len()),
    );

    let (run, sa, sb) = sync_paths(&a, &b, Params { k: 16, t: 16, batch: 256 }, Cut::Never).await?;
    let out_a = run.dialer?;
    let out_b = run.acceptor?;
    rep.check(
        "range sync moves exactly the missing event",
        out_b.inserted == 1 && out_a.inserted == 0 && out_b.learned == truth && out_a.learned == truth,
        format!(
            "acceptor inserted {}, dialer inserted {}, learned = true diff: {}",
            out_b.inserted,
            out_a.inserted,
            out_a.learned == truth && out_b.learned == truth
        ),
    );
    rep.check("stores equal after sync", same_rows(&sa, &sb)?, "every row matches");
    let (gaps, bad) = audit(&sb)?;
    rep.check("chain 7 is contiguous with valid links", gaps == 0 && bad == 0, format!("gaps={gaps} bad links={bad}"));
    println!("  flights (A+B): {}, control bytes: {}", out_a.flights + out_b.flights, out_a.control_bytes + out_b.control_bytes);
    Ok(())
}

// (b) ------------------------------------------------------------------------------------------

/// The 16-byte truncation of an ID, as the fingerprint sees it.
fn trunc(id: &Id) -> [u8; 16] {
    id[..16].try_into().expect("16 bytes")
}

fn xor16(a: &[u8; 16], b: &[u8; 16]) -> [u8; 16] {
    std::array::from_fn(|i| a[i] ^ b[i])
}

fn synthetic_id(tag: &str) -> Id {
    *blake3::hash(tag.as_bytes()).as_bytes()
}

/// An ID whose first 16 bytes are `target` XOR `other`, so that `fp` matches under XOR.
fn id_with_truncation(seed: &str, target: [u8; 16]) -> Id {
    let mut id = synthetic_id(seed);
    for (dst, t) in id[..16].iter_mut().zip(target) {
        *dst = t;
    }
    id
}

fn party_of(author: u8, ids: &[Id]) -> Party {
    let mut items: Vec<(Key, Id)> = ids
        .iter()
        .enumerate()
        .map(|(seq, id)| (key_of(&[author; 32], seq as u32), *id))
        .collect();
    items.sort_by_key(|(k, _)| *k);
    let (keys, ids): (Vec<Key>, Vec<Id>) = items.into_iter().unzip();
    Party::new(keys, ids)
}

fn scenario_b(rep: &mut Report) {
    println!("(b) XOR cancellation, in memory on crafted IDs (real BLAKE3 IDs cannot be steered this way)");

    // b1. Unequal sizes: A holds {a, b, e} and B holds {c, d}, with equal XOR fingerprints.
    let a = synthetic_id("b1-a");
    let b = synthetic_id("b1-b");
    let e = synthetic_id("b1-e");
    let c = synthetic_id("b1-c");
    let d = id_with_truncation("b1-d", xor16(&xor16(&trunc(&a), &trunc(&b)), &xor16(&trunc(&e), &trunc(&c))));
    let fp_a = [a, b, e].iter().fold([0u8; 16], |acc, id| xor16(&acc, &trunc(id)));
    let fp_b = [c, d].iter().fold([0u8; 16], |acc, id| xor16(&acc, &trunc(id)));
    println!("  b1: fingerprints equal: {}, counts 3 vs 2", fp_a == fp_b);
    let pa = party_of(0x0A, &[a, b, e]);
    let pb = party_of(0x0B, &[c, d]);
    let sim = simulate(&pa, &pb, 16, 16);
    let truth: HashSet<Id> = [a, b, e, c, d].into_iter().collect();
    rep.check(
        "b1: the count check catches it",
        sim.learned_a == truth && sim.learned_b == truth && sim.flights > 2,
        format!("flights={}, both sides learned the true 5-item difference: {}", sim.flights, sim.learned_a == truth && sim.learned_b == truth),
    );

    // b2. Equal sizes: A holds {a, b}, B holds {c, d}, with equal XOR fingerprints and equal counts.
    let a2 = synthetic_id("b2-a");
    let b2 = synthetic_id("b2-b");
    let c2 = synthetic_id("b2-c");
    let d2 = id_with_truncation("b2-d", xor16(&xor16(&trunc(&a2), &trunc(&b2)), &trunc(&c2)));
    let fp_a2 = [a2, b2].iter().fold([0u8; 16], |acc, id| xor16(&acc, &trunc(id)));
    let fp_b2 = [c2, d2].iter().fold([0u8; 16], |acc, id| xor16(&acc, &trunc(id)));
    println!("  b2: fingerprints equal: {}, counts 2 vs 2", fp_a2 == fp_b2);
    let pa2 = party_of(0x0A, &[a2, b2]);
    let pb2 = party_of(0x0B, &[c2, d2]);
    let sim2 = simulate(&pa2, &pb2, 16, 16);
    let truth2: HashSet<Id> = [a2, b2, c2, d2].into_iter().collect();
    rep.check(
        "b2: equal fingerprint and count is not caught",
        sim2.flights == 2 && sim2.learned_a.is_empty() && sim2.learned_b.is_empty() && truth2.len() == 4,
        format!(
            "flights={}, learned A={} B={}, true difference={}: the 4 events are missed",
            sim2.flights,
            sim2.learned_a.len(),
            sim2.learned_b.len(),
            truth2.len()
        ),
    );
    println!("  note: 128-bit fingerprints make a chance match 2^-128 per compared range. Constructing one with");
    println!("        real events needs a 4-way XOR collision (about 2^43 signed events by generalized birthday).");
}

// (c) ------------------------------------------------------------------------------------------

/// A key whose public key has zero bytes at the given positions. Ground by trying seeds.
fn grind(zero_at: &[usize], salt: u64) -> SigningKey {
    let mut i = 0u64;
    loop {
        let seed = *blake3::hash(&[&salt.to_be_bytes()[..], &i.to_be_bytes()[..]].concat()).as_bytes();
        let key = SigningKey::from_bytes(&seed);
        let pk = *key.verifying_key().as_bytes();
        if zero_at.iter().all(|&p| pk[p] == 0) {
            return key;
        }
        i += 1;
    }
}

async fn scenario_c(dir: &Path, rep: &mut Report) -> Result<()> {
    println!("(c) bounds at keys with 0x00 bytes: authors with zero bytes at chosen public-key positions");
    let mut keys: Vec<SigningKey> = Vec::new();
    for salt in 0..4 {
        keys.push(grind(&[0, 1], 100 + salt));
    }
    for salt in 0..2 {
        keys.push(grind(&[15, 16], 200 + salt));
    }
    for salt in 0..4 {
        keys.push(grind(&[31], 300 + salt));
    }
    for i in 0..40 {
        keys.push(chain_key(10_000 + i));
    }
    let with_zero = keys
        .iter()
        .filter(|k| k.verifying_key().as_bytes().contains(&0))
        .count();
    println!("  {} authors, {} of them with a 0x00 byte in the public key; every seq < 256 also has 0x00 bytes", keys.len(), with_zero);

    let base = dir.join("base-c.redb");
    let _ = std::fs::remove_file(&base);
    {
        let store = Store::open(&base)?;
        let mut items = Vec::new();
        for key in &keys {
            for ev in chain_events(key, EVENTS_PER_CHAIN) {
                let v = chain::verify(&ev.header, &ev.body)?;
                items.push(Item { key: key_of(&v.author, v.seq), id: v.id, record: chain::record(&ev.header, &ev.body) });
            }
        }
        store.insert(&items)?;
    }

    let a = copy_to(&base, dir.join("a-c.redb"))?;
    let b = copy_to(&base, dir.join("b-c.redb"))?;
    // 20 new events: on the zero-byte authors first, then on ordinary ones.
    let extra: Vec<(&SigningKey, u32)> = keys.iter().take(20).map(|k| (k, 1)).collect();
    let truth = add_next(&a, &extra)?;

    // In memory first: the scheme with several K and T on this key set.
    let a_pairs = Store::open(&a)?.pairs()?;
    let mut all_ok = true;
    let mut rows = Vec::new();
    for (k, t) in [(2usize, 1usize), (2, 4), (16, 16), (32, 16), (4, 8)] {
        let pa = Party::new(a_pairs.iter().map(|x| x.0).collect(), a_pairs.iter().map(|x| x.1).collect());
        let b_pairs = Store::open(&b)?.pairs()?;
        let pb = Party::new(b_pairs.iter().map(|x| x.0).collect(), b_pairs.iter().map(|x| x.1).collect());
        let sim = simulate(&pa, &pb, t, k);
        let good = sim.learned_a == truth && sim.learned_b == truth;
        all_ok &= good;
        rows.push(format!("K={k} T={t}: flights {} ok={good}", sim.flights));
    }
    rep.check("(c) in-memory run learns the true difference", all_ok, rows.join("; "));

    let p = Params { k: 16, t: 16, batch: 256 };
    let (run, sa, sb) = sync_paths(&a, &b, p, Cut::Never).await?;
    let out_a = run.dialer?;
    let out_b = run.acceptor?;
    rep.check(
        "(c) wire sync with zero-byte keys is exact",
        out_b.inserted == truth.len() && out_b.learned == truth && out_a.learned == truth && same_rows(&sa, &sb)?,
        format!("acceptor inserted {} of {}, stores equal: {}", out_b.inserted, truth.len(), same_rows(&sa, &sb)?),
    );
    let (gaps, bad) = audit(&sb)?;
    rep.check("(c) links valid after sync", gaps == 0 && bad == 0, format!("gaps={gaps} bad links={bad}"));
    Ok(())
}

// (d) ------------------------------------------------------------------------------------------

async fn scenario_d(dir: &Path, base: &Path, rep: &mut Report) -> Result<()> {
    println!("(d) interrupted session, restarted from scratch (1,000 chains; 60 chains get 3 new events each)");
    let extra_keys: Vec<SigningKey> = (0..60).map(|i| chain_key(i * 16)).collect();
    let extra: Vec<(&SigningKey, u32)> = extra_keys.iter().map(|k| (k, 3)).collect();

    // d1. Cut the dialer during the data phase. Batches of 32, so partial commits can happen.
    let a = copy_to(base, dir.join("a-d.redb"))?;
    let b = copy_to(base, dir.join("b-d.redb"))?;
    let truth = add_next(&a, &extra)?;
    let p = Params { k: 16, t: 16, batch: 32 };
    let base_rows = Store::open(base)?.counts()?.1;
    let (cut_run, _, sb) = sync_paths(&a, &b, p, Cut::AfterRecords(100)).await?;
    let cut_errors = cut_run.dialer.is_err() && cut_run.acceptor.is_err();
    println!("  cut run: dialer error = {:?}", cut_run.dialer.as_ref().err().map(|e| format!("{e:#}")));
    println!("  cut run: acceptor error = {:?}", cut_run.acceptor.as_ref().err().map(|e| format!("{e:#}")));
    let committed = sb.counts()?.1 - base_rows;
    rep.check("(d1) both sides report the cut", cut_errors, format!("dialer err={}, acceptor err={}", cut_run.dialer.is_err(), cut_run.acceptor.is_err()));
    rep.check(
        "(d1) only whole batches committed",
        committed % 32 == 0 && committed < truth.len() as u64,
        format!("{committed} of {} new events committed (batch 32)", truth.len()),
    );
    let (gaps, bad) = audit(&sb)?;
    rep.check("(d1) store consistent after the cut", gaps == 0 && bad == 0, format!("gaps={gaps} bad links={bad}"));

    // What is still missing after the cut: the true difference minus what was committed.
    let mut present = HashSet::new();
    for id in &truth {
        if sb.has(id)? {
            present.insert(*id);
        }
    }
    let remaining: HashSet<Id> = truth.difference(&present).copied().collect();
    rep.check(
        "(d1) committed events are the ones in the store",
        present.len() as u64 == committed,
        format!("{} of the new events are in the store, {committed} new rows", present.len()),
    );
    drop(sb);

    // Restart from scratch: new indexes, new reconciliation.
    let (run2, sa2, sb2) = sync_paths(&a, &b, Params { k: 16, t: 16, batch: 256 }, Cut::Never).await?;
    let out_a = run2.dialer?;
    let out_b = run2.acceptor?;
    rep.check(
        "(d1) restart sends only what is missing",
        out_b.inserted == remaining.len() && out_b.duplicates == 0 && out_a.inserted == 0,
        format!("restart inserted {} (expected {}), duplicates {}", out_b.inserted, remaining.len(), out_b.duplicates),
    );
    rep.check(
        "(d1) restart learns exactly the missing events",
        out_a.learned == remaining && out_b.learned == remaining,
        format!("learned {} and {}, missing {}", out_a.learned.len(), out_b.learned.len(), remaining.len()),
    );
    rep.check(
        "(d1) stores equal after restart",
        same_rows(&sa2, &sb2)? && sb2.has(truth.iter().next().context("truth")?)?,
        format!("stores equal: {}", same_rows(&sa2, &sb2)?),
    );
    let (gaps, bad) = audit(&sb2)?;
    rep.check("(d1) links valid after restart", gaps == 0 && bad == 0, format!("gaps={gaps} bad links={bad}"));

    // d2. Cut during reconciliation: the dialer closes after its first message.
    let a = copy_to(base, dir.join("a-d2.redb"))?;
    let b = copy_to(base, dir.join("b-d2.redb"))?;
    let truth2 = add_next(&a, &extra)?;
    let (cut2, _, sb3) = sync_paths(&a, &b, p, Cut::AfterFlights(1)).await?;
    let nothing_committed = sb3.counts()?.1 == base_rows;
    rep.check(
        "(d2) cut during reconciliation commits nothing",
        cut2.dialer.is_err() && nothing_committed,
        format!("dialer err={}, acceptor err={}, new rows={}", cut2.dialer.is_err(), cut2.acceptor.is_err(), sb3.counts()?.1 - base_rows),
    );
    drop(sb3);
    let (run3, sa3, sb4) = sync_paths(&a, &b, Params { k: 16, t: 16, batch: 256 }, Cut::Never).await?;
    let out_b3 = run3.acceptor?;
    run3.dialer?;
    rep.check(
        "(d2) restart completes with no duplicates",
        out_b3.inserted == truth2.len() && out_b3.duplicates == 0 && same_rows(&sa3, &sb4)?,
        format!("restart inserted {} of {}, stores equal: {}", out_b3.inserted, truth2.len(), same_rows(&sa3, &sb4)?),
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        bail!("usage: scenarios <dir> <base-1000.redb>");
    }
    let dir = PathBuf::from(&args[0]);
    let base = PathBuf::from(&args[1]);
    std::fs::create_dir_all(&dir)?;
    let mut rep = Report::default();
    scenario_b(&mut rep);
    scenario_a(&dir, &base, &mut rep).await?;
    scenario_c(&dir, &mut rep).await?;
    scenario_d(&dir, &base, &mut rep).await?;
    println!("failures: {}", rep.failures);
    if rep.failures > 0 {
        bail!("{} checks failed", rep.failures);
    }
    Ok(())
}
