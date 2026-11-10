//! Welcome size, build time and the joiner's check, by joiner history mode (`full`, `recent:N`).
//! Method and results are in RESULTS.md.
//!
//! Runs the scenario on two builds of p2panda-encryption: stock, and the serde_bytes variant. Prints
//! machine-readable lines (`ROW|`, `ERR|`, `PRE|`, `RUN|`, `OP|`). `summarize.sh` aggregates them.
//!
//! Usage: `welcome-recent --seed 1 --k 100,1000`
#![allow(dead_code, unused_imports)]

mod stock {
    use p2panda_encryption as enc;

    include!("harness.rs");
}

mod sb {
    use p2panda_encryption_sb as enc;

    include!("harness.rs");
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seed: u8 = flag(&args, "--seed").map_or(1, |v| v.parse().expect("seed is a byte"));
    let ks: Vec<usize> = flag(&args, "--k").map_or(vec![100, 1000], |v| {
        v.split(',').map(|k| k.parse().expect("K is a count")).collect()
    });

    // Op counts do not depend on the seed. Print them once per build.
    let op_rng = p2panda_encryption::Rng::from_seed([9; 32]);
    println!("## op counts, stock");
    stock::op_counts(&op_rng);
    let op_rng_sb = p2panda_encryption_sb::Rng::from_seed([9; 32]);
    println!("## op counts, serde-bytes");
    sb::op_counts(&op_rng_sb);

    for k in ks {
        stock::run("stock", seed, k);
        sb::run("serde-bytes", seed, k);
    }
}
