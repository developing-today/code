//! Standalone `queue-broker` — the queue broker role's own
//! artifact, kept building from the fleet workspace for the whole
//! migration as the standing rollback inventory (design note §7).
//! The same code runs inside the fleet binary as
//! `fleet serve queue-broker`.

fn main() {
    std::process::exit(fleet_queue::queue_broker::run());
}
