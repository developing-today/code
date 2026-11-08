//! Argument handling shared by the standalone broker binaries.

/// What a standalone broker binary should do with its argument vector.
#[derive(Debug, PartialEq, Eq)]
pub enum BinArgs {
    /// No arguments: start the broker.
    Run,
    /// `-h` or `--help` was supplied: print usage and exit successfully.
    Help,
    /// The first unsupported argument: print a usage error and exit 2.
    UsageError(String),
}

/// Classify arguments for a broker binary, which accepts no options.
/// Help anywhere wins so `--help` remains safe alongside a stale flag.
pub fn classify_bin_args(argv: &[String]) -> BinArgs {
    if argv
        .iter()
        .any(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        return BinArgs::Help;
    }

    match argv.first() {
        None => BinArgs::Run,
        Some(first) => BinArgs::UsageError(first.clone()),
    }
}

pub const QUEUE_BROKER_USAGE: &str =
    "usage: queue-broker [--help|-h]\n\nStarts the standalone queue broker.";
pub const SECRETS_BROKER_USAGE: &str =
    "usage: secrets-broker [--help|-h]\n\nStarts the standalone secrets broker.";

#[cfg(test)]
mod tests {
    use super::{BinArgs, classify_bin_args};

    fn classify(argv: &[&str]) -> BinArgs {
        let argv: Vec<String> = argv.iter().map(|arg| (*arg).to_string()).collect();
        classify_bin_args(&argv)
    }

    #[test]
    fn bare_invocation_runs() {
        assert_eq!(classify(&[]), BinArgs::Run);
    }

    #[test]
    fn both_help_flags_are_recognized() {
        assert_eq!(classify(&["--help"]), BinArgs::Help);
        assert_eq!(classify(&["-h"]), BinArgs::Help);
    }

    #[test]
    fn unexpected_flags_and_positional_arguments_are_rejected() {
        assert_eq!(
            classify(&["--bogus-junk"]),
            BinArgs::UsageError("--bogus-junk".to_string())
        );
        assert_eq!(
            classify(&["frobnicate"]),
            BinArgs::UsageError("frobnicate".to_string())
        );
    }

    #[test]
    fn help_wins_when_a_stale_argument_is_also_present() {
        assert_eq!(classify(&["--bogus-junk", "--help"]), BinArgs::Help);
    }
}
