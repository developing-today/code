//! Spike S2: p2panda-auth StrongRemove for the `id` design. Scenarios (a), the `after` mapping (b),
//! unknown dependencies (c), friendship (d), and the delegation cascade (e).
//!
//! Built on p2panda-auth 0.7.1 at git rev 3e84cac, `test_utils` types (char members, u32 op ids).

mod dag;
mod delegation;
mod friendship;
mod panics;
mod scenarios;

fn main() {
    let mut failures = 0;
    failures += scenarios::run();
    failures += panics::run();
    failures += friendship::run();
    failures += delegation::run();
    println!("\nchecks failed: {failures}");
    std::process::exit(i32::from(failures != 0));
}
