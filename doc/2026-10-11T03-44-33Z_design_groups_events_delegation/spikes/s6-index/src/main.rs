//! Spike S6: local index storage, redb versus SQLite (WAL).
//!
//! Subcommands are listed in RESULTS.md. The helper subcommands at the bottom are
//! started as child processes by `concurrency`.

mod bench;
mod checks;
mod concurrency;
mod durability;
mod keys;
mod redb_store;
mod sqlite_store;
mod store;

use anyhow::{Result, anyhow, bail};
use std::path::{Path, PathBuf};

/// SplitMix64: deterministic and dependency-free.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    pub fn id(&mut self) -> [u8; 32] {
        let mut out = [0u8; 32];
        for chunk in out.chunks_mut(8) {
            chunk.copy_from_slice(&self.next_u64().to_le_bytes());
        }
        out
    }

    pub fn bytes(&mut self, len: usize, alphabet: &[u8]) -> Vec<u8> {
        (0..len)
            .map(|_| alphabet[self.below(alphabet.len() as u64) as usize])
            .collect()
    }
}

/// A `/proc/self/status` field in kB, such as "VmHWM:" (peak RSS).
pub fn status_kb(field: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with(field))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0)
}

fn bench(kind: &str, dir: &Path, n: usize) -> Result<()> {
    match kind {
        "redb" => bench::run::<redb_store::RedbStore>(dir, n),
        "sqlite" => bench::run::<sqlite_store::SqliteStore>(dir, n),
        other => bail!("unknown store {other}"),
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| -> Result<&str> {
        args.get(i)
            .map(String::as_str)
            .ok_or_else(|| anyhow!("missing argument {i}"))
    };
    let num = |i: usize| -> Result<u64> { Ok(arg(i)?.parse()?) };
    match arg(0)? {
        "selftest" => checks::selftest(),
        "bench" => {
            let n = if args.len() > 3 {
                num(3)? as usize
            } else {
                200_000
            };
            bench(arg(1)?, Path::new(arg(2)?), n)
        }
        "durable-child" => durability::child_for(arg(1)?, Path::new(arg(2)?), num(3)?),
        "durable-test" => durability::test_for(arg(1)?, num(2)? as usize, Path::new(arg(3)?)),
        "unique" => checks::unique_for(arg(1)?, Path::new(arg(2)?)),
        "compact-redb" => {
            let (before, after, shrunk) = redb_store::compact(Path::new(arg(1)?))?;
            println!("compact-redb: before={before} after={after} shrunk={shrunk}");
            Ok(())
        }
        "concurrency" => concurrency::run(Path::new(arg(1)?)),
        "redb-hold" => concurrency::redb_hold(Path::new(arg(1)?), num(2)?),
        "redb-probe" => concurrency::redb_probe(Path::new(arg(1)?)),
        "sqlite-setup" => concurrency::sqlite_setup(Path::new(arg(1)?), num(2)?),
        "sqlite-hold" => concurrency::sqlite_hold(Path::new(arg(1)?), num(2)?, num(3)?, num(4)?),
        "sqlite-read" => concurrency::sqlite_read(Path::new(arg(1)?)),
        "sqlite-read-hold" => concurrency::sqlite_read_hold(Path::new(arg(1)?), num(2)?),
        "sqlite-write" => concurrency::sqlite_write(Path::new(arg(1)?), num(2)?, num(3)?, num(4)?),
        other => bail!("unknown subcommand {other}"),
    }
}

/// The run directory, inside this spike's own workspace.
pub fn run_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("run")
}
