//! The queue domain's one logging facility (design note §1).
//!
//! Every line carries the domain as its field — the established
//! `<domain>: <message>` shape the brokers have always emitted on
//! stderr — so one journal query follows a request across client,
//! queue-broker, and secrets-broker, and the port changed no log
//! text anyone already greps for.

/// Emit one stderr line for `domain`: `"<domain>: <message>"`.
pub fn info(domain: &str, message: &str) {
    eprintln!("{domain}: {message}");
}
