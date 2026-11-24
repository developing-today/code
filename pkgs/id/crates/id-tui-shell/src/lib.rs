//! Terminal (TUI) shell for `id` — more-shells v2 slice 2
//! (#12818; design `doc/2026-11-23T03-17-14Z_design_more_shells_v2`,
//! §2 "TUI — feature completion", from #1071).
//!
//! The TUI is a shell in the F13.1 shell contract's terms, not a
//! debug view: a ratatui client covering the full text-native
//! workflow of a world session — join, read, act, reconnect under
//! the v1 history-identity rule — at parity with the browser
//! shell's text surface.
//!
//! Per decision D3 of the v2 outline, the claimed subset is
//! recorded as this shell's capability statement *before*
//! implementation starts. That statement is this crate's first
//! recorded content: [`CAPABILITY_STATEMENT_JSON`] is the verbatim
//! record (`capability-statement.json` at the crate root), and
//! `tests/capability_statement.rs` pins it to the contract file
//! and to the workflow #12818 names, so the claim cannot drift
//! silently from the contract it is a statement against. The
//! frame builders, session fold, and presentation glue of feature
//! completion land in this crate on top of the statement, and the
//! pinning tests grow with them — the same progression the
//! hardware shell (#10929) and the native shell (#10928) followed.

/// Shell-contract name this client speaks (F13.1).
pub const CONTRACT_NAME: &str = "id-shell";

/// Shell-contract major version this shell's statement is
/// recorded against.
pub const CONTRACT_VERSION: u64 = 1;

/// The TUI shell's recorded capability statement, verbatim:
/// the client frames it sends, the server frames it presents,
/// the F13.7 providers it declares (none), its posture and
/// reconnect rule, and its exclusions by name. The canonical
/// record is `capability-statement.json` at the crate root;
/// this constant embeds that file so the statement ships with
/// the shell it describes.
pub const CAPABILITY_STATEMENT_JSON: &str = include_str!("../capability-statement.json");

/// The recorded capability statement, parsed. The canonical
/// record is the JSON text in [`CAPABILITY_STATEMENT_JSON`];
/// this is a convenience parse of exactly that text.
///
/// # Errors
///
/// Returns the `serde_json` error if the recorded statement
/// shipped in this crate is not valid JSON — a build-time
/// defect the crate's capability-statement tests exist to
/// catch.
pub fn capability_statement() -> Result<serde_json::Value, serde_json::Error> {
    serde_json::from_str(CAPABILITY_STATEMENT_JSON)
}
