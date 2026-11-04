//! Standalone `repo-queue` — the queue client's own artifact, kept
//! building from the fleet workspace for the whole migration as the
//! standing rollback inventory (design note §7). The same client
//! code runs inside the fleet binary as the queue domain.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(fleet_queue::client::run(&args));
}
