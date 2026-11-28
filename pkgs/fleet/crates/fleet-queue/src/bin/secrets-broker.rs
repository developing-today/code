//! Standalone `secrets-broker` — the secrets broker role's own
//! artifact, kept building from the fleet workspace for the whole
//! migration as the standing rollback inventory (design note §7).
//! The same code runs inside the fleet binary as
//! `fleet serve secrets-broker`.

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match fleet_queue::bin_args::classify_bin_args(&argv) {
        fleet_queue::bin_args::BinArgs::Help => {
            println!("{}", fleet_queue::bin_args::SECRETS_BROKER_USAGE);
            return;
        }
        fleet_queue::bin_args::BinArgs::UsageError(arg) => {
            eprintln!("secrets-broker: unknown argument {arg:?}");
            eprintln!("{}", fleet_queue::bin_args::SECRETS_BROKER_USAGE);
            std::process::exit(2);
        }
        fleet_queue::bin_args::BinArgs::Run => {}
    }

    std::process::exit(fleet_queue::secrets_broker::run());
}
