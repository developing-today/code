//! BeeKEM spike. Each subcommand answers one question from RESULTS.md.
//!
//!   beekem-spike costs <n> [fill]      Q1 costs, per-member state, joiner payload, leak check
//!   beekem-spike scenarios             Q2 deterministic concurrency scenarios
//!   beekem-spike fuzz <trials> <buffer|drop>   Q2 randomized convergence
//!   beekem-spike secrecy               Q4 forward secrecy and PCS
//!   beekem-spike join                  Q3 joiner payload and history requirement

mod common;
mod concurrency;
mod costs;
mod join;
mod party;
mod secrecy;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["costs", n] => {
            let n: usize = n.parse().expect("group size");
            costs::run(n, n - 1);
        }
        ["costs", n, fill] => costs::run(
            n.parse().expect("group size"),
            fill.parse().expect("fill count"),
        ),
        ["scenarios"] => concurrency::scenarios(),
        ["fuzz", trials, mode] => {
            let buffer = match *mode {
                "buffer" => true,
                "drop" => false,
                other => panic!("mode must be buffer or drop, got {other}"),
            };
            concurrency::fuzz(trials.parse().expect("trial count"), buffer);
        }
        ["secrecy"] => secrecy::run(),
        ["join"] => join::run(),
        _ => {
            eprintln!("usage: beekem-spike costs <n> | scenarios | fuzz <trials> <buffer|drop> | secrecy | join");
            std::process::exit(2);
        }
    }
}
