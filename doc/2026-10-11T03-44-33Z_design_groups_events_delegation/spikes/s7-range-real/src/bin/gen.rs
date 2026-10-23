//! Builds the base store: `chains` device chains, 50 signed events each, every event verified
//! with `AnyHeader::decode` before it is stored.
//!
//! Usage: gen <dir> <chains>   -> <dir>/base-<chains>.redb

use anyhow::{Result, bail};
use s7_range_real::chain::{self, EVENTS_PER_CHAIN, chain_events, chain_key};
use s7_range_real::store::{Item, Store, key_of};
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        bail!("usage: gen <dir> <chains>");
    }
    let dir = PathBuf::from(&args[1]);
    let chains: usize = args[2].parse()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("base-{chains}.redb"));
    let _ = std::fs::remove_file(&path);
    let store = Store::open(&path)?;

    let started = Instant::now();
    let mut verified = 0usize;
    let mut pending: Vec<Item> = Vec::new();
    for i in 0..chains {
        for ev in chain_events(&chain_key(i), EVENTS_PER_CHAIN) {
            let v = chain::verify(&ev.header, &ev.body)?;
            verified += 1;
            pending.push(Item {
                key: key_of(&v.author, v.seq),
                id: v.id,
                record: chain::record(&ev.header, &ev.body),
            });
            if pending.len() == 10_000 {
                store.insert(&pending)?;
                pending.clear();
            }
        }
    }
    store.insert(&pending)?;
    let elapsed = started.elapsed();
    let (events, rows) = store.counts()?;
    let bytes = std::fs::metadata(&path)?.len();
    println!(
        "gen: chains={chains} events={events} rows={rows} verified={verified} \
         time={:.1}s file={:.1} MB",
        elapsed.as_secs_f64(),
        bytes as f64 / 1e6
    );
    Ok(())
}
