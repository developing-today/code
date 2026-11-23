//! The Metadata create path for `fleet gh` (developing-today/code#9073;
//! design `doc/2026-11-12T22-50-00Z_design_pm-board-hierarchy-genealogy`
//! §12–§15, amendment slice 9; extends #9016 step 2).
//!
//! `fleet gh issue create` and `fleet gh pr create` are Metadata-aware.
//! The caller's body may already carry a Metadata block (§12, delimited
//! by `<!-- pm-metadata:start/end -->` HTML comments around a fenced
//! JSON object). This layer:
//!
//!   1. parses the block — a malformed block refuses the create before
//!      any GitHub write, because silently dropping caller-owned
//!      fields is the one unacceptable outcome (the hosted door,
//!      #12585, refuses the same way);
//!   2. stamps the fleet-owned `provenance` set in full from fleet
//!      sources (§13): the queue record bound to the caller's session
//!      (queue id, scheduled start, pickup time, lane, claim model,
//!      the lane start's issue set), session state (§14), host,
//!      worktree, and the local clock. Caller-supplied provenance
//!      values, forged or honest, are replaced wholesale; the
//!      caller-owned fields (`direct_parent`, `links`, `lineage`)
//!      are preserved exactly, with required-field defaults injected
//!      when absent, and every other caller top-level key is kept;
//!   3. signs the provenance when a stamp key is configured
//!      (#12586's `v1.ed25519.<kid>.<sig>` format over the JCS-shaped
//!      canonical provenance bytes). The hosted edition's golden
//!      vectors (pkgs/fleet/hosted-worker/test/stamp-sig.test.ts)
//!      are this module's conformance fixtures and reproduce
//!      byte-exact in its tests;
//!   4. runs the create through the gh domain's normal rotation with
//!      the stamped body, then inserts the full record — full
//!      Metadata JSON plus #9074's extracted keys — into the
//!      metadata store (§15) in the same call.
//!
//! Failure posture (the native CLI edition; the hosted door's 503s
//! are a hosted service's posture, not a CLI's):
//!   - malformed caller block -> the create is refused (exit 2)
//!     before any write, with the block problem named;
//!   - session/queue inputs missing -> the affected provenance
//!     fields stamp null; signing configured but unresolvable ->
//!     the block stamps unsigned and the failure is noted on
//!     stderr; the create proceeds either way;
//!   - DB insert failure -> the GitHub item stands, gh's exit code
//!     is unchanged, and the miss is reported on stderr: ingestion
//!     (§16; the sweeper of #12590) heals it, as the design provides.
//!
//! Everything that is not an `issue create` / `pr create` with a
//! stampable body passes through byte-exact, as before. Creates that
//! leave the body to a human or to gh itself — an interactive
//! terminal with no `--body`, `--web`, `--fill*`, `--template`
//! without a body, `--body-file -` — pass through unstamped; §16
//! backfill owns those items.
//!
//! The metadata store is #9074's `pm_metadata` schema
//! (pm-stack/aggregator/src/metadata-store.ts) over SQLite, with the
//! same artifact identity, upsert and extraction rules, so a store
//! file written here is readable by the aggregator's store and vice
//! versa. The artifact id for an item created here is
//! `gh:<repo>#<kind>:<number>` — deterministic from the item's own
//! identity; the row also carries repo/kind/number as columns, so a
//! future identity-convention change reconciles on those.
//!
//! Controls (environment):
//!   FLEET_METADATA_DB       metadata store path (default:
//!                           `<fleet state dir>/metadata.db`)
//!   FLEET_GH_STAMP_KEY_ID   stamp signing key id; when set, the key
//!                           material is resolved by name from the
//!                           secrets broker (secret
//!                           `fleet-gh.stamp-signing-key-pkcs8`,
//!                           base64url PKCS#8, the hosted config's
//!                           encoding) and stamped blocks are
//!                           signed. Unset: blocks stamp unsigned.

use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use fleet_config::Config;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub const METADATA_START: &str = "<!-- pm-metadata:start -->";
pub const METADATA_END: &str = "<!-- pm-metadata:end -->";
pub const STAMP_KEY_SECRET_NAME: &str = "fleet-gh.stamp-signing-key-pkcs8";
pub const STAMP_KEY_ID_ENV: &str = "FLEET_GH_STAMP_KEY_ID";
pub const METADATA_DB_ENV: &str = "FLEET_METADATA_DB";

const CALLER_OWNED_FIELDS: [&str; 3] = ["direct_parent", "links", "lineage"];

// ---------------------------------------------------------------------
// Time: RFC 3339 (seconds, UTC) from unix seconds, no date crate.

/// Format unix seconds as `YYYY-MM-DDTHH:MM:SSZ` (the stamp set's
/// timestamp shape; the golden vectors carry seconds precision).
pub fn rfc3339(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    )
}

/// Howard Hinnant's civil-from-days algorithm (public domain math).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------
// The Metadata block: parse, render, assemble (the hosted metadata.ts
// semantics, ported).

#[derive(Debug)]
pub struct ParsedBody {
    /// The parsed block (a JSON object with `pm_meta: 1`), or None
    /// when the body carries no block.
    pub block: Option<Value>,
    /// Body text before the block (the whole body when block is None).
    pub before: String,
    /// Body text after the block ("" when block is None).
    pub after: String,
}

fn invalid_block(message: &str) -> String {
    format!("invalid pm-metadata block: {message}")
}

/// Locate and parse the pm-metadata block in a body. A body with no
/// markers parses to `block: None`. A body whose markers are present
/// but whose payload is not a fenced JSON object with `pm_meta: 1`
/// fails closed: the caller fixes its block, the create is refused.
pub fn parse_metadata_block(body: &str) -> Result<ParsedBody, String> {
    let Some(start) = body.find(METADATA_START) else {
        if body.contains(METADATA_END) {
            return Err(invalid_block(
                "pm-metadata end marker without a start marker",
            ));
        }
        return Ok(ParsedBody {
            block: None,
            before: body.to_string(),
            after: String::new(),
        });
    };
    let inner_start = start + METADATA_START.len();
    let Some(end_rel) = body[inner_start..].find(METADATA_END) else {
        return Err(invalid_block(
            "pm-metadata start marker without an end marker",
        ));
    };
    let end = inner_start + end_rel;
    let inner = body[inner_start..end].trim();
    let Some(fenced) = inner.strip_prefix("```") else {
        return Err(invalid_block(
            "pm-metadata block does not carry a fenced JSON payload",
        ));
    };
    let fenced = fenced.strip_prefix("json").unwrap_or(fenced);
    let Some(payload) = fenced.strip_suffix("```") else {
        return Err(invalid_block(
            "pm-metadata block does not carry a fenced JSON payload",
        ));
    };
    let parsed: Value = serde_json::from_str(payload.trim())
        .map_err(|_| invalid_block("pm-metadata block payload is not valid JSON"))?;
    if !parsed.is_object() {
        return Err(invalid_block(
            "pm-metadata block payload must be a JSON object",
        ));
    }
    let pm_meta_ok = parsed
        .get("pm_meta")
        .is_some_and(|v| v.as_i64() == Some(1) || v.as_f64() == Some(1.0));
    if !pm_meta_ok {
        return Err(invalid_block("pm-metadata block must carry pm_meta: 1"));
    }
    Ok(ParsedBody {
        block: Some(parsed),
        before: body[..start].to_string(),
        after: body[end + METADATA_END.len()..].to_string(),
    })
}

/// Render a block in the §12 canonical form.
pub fn render_metadata_block(block: &Value) -> String {
    let json = serde_json::to_string_pretty(block).unwrap_or_else(|_| "{}".to_string());
    format!("{METADATA_START}\n```json\n{json}\n```\n{METADATA_END}")
}

/// Splice a block into a body: in place of the parsed block's span
/// when one was parsed, else appended at the end (separated from the
/// caller's text by a blank line). Text outside the block is
/// preserved byte-for-byte.
pub fn render_body_with_block(parsed: &ParsedBody, block: &Value) -> String {
    let rendered = render_metadata_block(block);
    if parsed.block.is_some() {
        return format!("{}{}{}", parsed.before, rendered, parsed.after);
    }
    let trimmed = parsed.before.trim_end();
    if trimmed.is_empty() {
        format!("{rendered}\n")
    } else {
        format!("{trimmed}\n\n{rendered}\n")
    }
}

/// Required-field defaults for the caller-owned fields (§13's
/// "ensures required fields are present — injecting defaults").
pub fn caller_owned_defaults() -> Map<String, Value> {
    let mut lineage = Map::new();
    lineage.insert("design".to_string(), Value::Null);
    lineage.insert("roadmap_item".to_string(), Value::Null);
    lineage.insert("priority".to_string(), Value::Null);
    let mut out = Map::new();
    out.insert("direct_parent".to_string(), Value::Null);
    out.insert("links".to_string(), Value::Array(Vec::new()));
    out.insert("lineage".to_string(), Value::Object(lineage));
    out
}

/// Assemble the stamped block: the caller's block (or defaults) with
/// its provenance replaced in full by the fleet-assembled, already
/// signed provenance object. Caller-owned fields are preserved
/// exactly as parsed; absent ones take the required-field defaults.
/// Every other caller top-level key is preserved; `pm_meta` is
/// pinned to 1.
pub fn assemble_stamped_block(caller_block: Option<&Value>, signed_provenance: Value) -> Value {
    let mut out: Map<String, Value> = caller_block
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let defaults = caller_owned_defaults();
    for field in CALLER_OWNED_FIELDS {
        if !out.contains_key(field) {
            out.insert(field.to_string(), defaults[field].clone());
        }
    }
    out.insert("pm_meta".to_string(), Value::from(1));
    out.insert("provenance".to_string(), signed_provenance);
    Value::Object(out)
}

// ---------------------------------------------------------------------
// Canonical provenance form + stamp signatures (the hosted
// stamp-sig.ts semantics, ported; its golden vectors are the tests).

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const MAX_DEPTH: usize = 32;

/// JSON.stringify's string escaping, exactly: the canonical form is
/// defined over the hosted edition's serializer, so the escaper is
/// written to its rules rather than serde_json's (which spells some
/// control characters differently).
fn escape_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn canonicalize_value(value: &Value, depth: usize) -> Result<String, String> {
    if depth > MAX_DEPTH {
        return Err("uncanonicalizable provenance: nesting too deep".to_string());
    }
    match value {
        Value::Null => Ok("null".to_string()),
        Value::Bool(b) => Ok(if *b { "true" } else { "false" }.to_string()),
        Value::Number(n) => {
            let ok = n
                .as_i64()
                .filter(|i| i.abs() <= MAX_SAFE_INTEGER)
                .map(|i| i.to_string())
                .or_else(|| {
                    n.as_u64()
                        .filter(|u| *u <= MAX_SAFE_INTEGER as u64)
                        .map(|u| u.to_string())
                });
            ok.ok_or_else(|| {
                "uncanonicalizable provenance: numbers must be safe integers".to_string()
            })
        }
        Value::String(s) => Ok(escape_json_string(s)),
        Value::Array(items) => {
            let parts: Result<Vec<String>, String> = items
                .iter()
                .map(|item| canonicalize_value(item, depth + 1))
                .collect();
            Ok(format!("[{}]", parts?.join(",")))
        }
        Value::Object(map) => {
            // Keys sort by Unicode scalar order (byte order); every
            // key in the signed domain is an ASCII field name by
            // construction, where this and the hosted edition's
            // UTF-16 code-unit order agree.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Result<Vec<String>, String> = keys
                .iter()
                .map(|key| {
                    canonicalize_value(&map[*key], depth + 1)
                        .map(|v| format!("{}:{}", escape_json_string(key), v))
                })
                .collect();
            Ok(format!("{{{}}}", parts?.join(",")))
        }
    }
}

/// The canonical bytes a stamp signature covers: the provenance
/// object with `stamp_sig` removed, serialized deterministically.
/// Identical for a signed and an unsigned copy of the same
/// provenance, and independent of key order — that is what makes
/// verification edition-blind.
pub fn canonical_provenance_bytes(provenance: &Value) -> Result<Vec<u8>, String> {
    let Some(map) = provenance.as_object() else {
        return Err("uncanonicalizable provenance: provenance must be an object".to_string());
    };
    let mut unsigned = map.clone();
    unsigned.remove("stamp_sig");
    Ok(canonicalize_value(&Value::Object(unsigned), 0)?.into_bytes())
}

fn b64url_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn b64url_decode(text: &str) -> Result<Vec<u8>, String> {
    let normalized: String = text
        .chars()
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            c => c,
        })
        .collect();
    let padded = format!(
        "{}{}",
        normalized,
        "=".repeat((4 - normalized.len() % 4) % 4)
    );
    base64::engine::general_purpose::STANDARD
        .decode(padded)
        .map_err(|e| format!("not base64: {e}"))
}

/// A valid stamp key id: `[A-Za-z0-9_-]+` (the hosted config's rule).
pub fn is_valid_stamp_key_id(key_id: &str) -> bool {
    !key_id.is_empty()
        && key_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub struct StampSigner {
    pub key_id: String,
    signing_key: SigningKey,
}

/// The Ed25519 PKCS#8 DER prefix (RFC 8410): a 32-byte seed follows.
const PKCS8_ED25519_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

/// Build a signer from its configured material: base64url (or
/// standard base64) PKCS#8 DER — the hosted config's encoding of
/// STAMP_SIGNING_KEY_PKCS8.
pub fn signer_from_pkcs8_base64(key_id: &str, encoded: &str) -> Result<StampSigner, String> {
    if !is_valid_stamp_key_id(key_id) {
        return Err(format!("stamp key id '{key_id}' is not a valid key id"));
    }
    let der = b64url_decode(encoded.trim())?;
    if der.len() != PKCS8_ED25519_PREFIX.len() + 32 || !der.starts_with(&PKCS8_ED25519_PREFIX) {
        return Err("stamp signing key is not an Ed25519 PKCS#8 key".to_string());
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&der[PKCS8_ED25519_PREFIX.len()..]);
    Ok(StampSigner {
        key_id: key_id.to_string(),
        signing_key: SigningKey::from_bytes(&seed),
    })
}

/// Sign a provenance object and return a NEW object carrying the
/// `stamp_sig` (the input is not mutated). Any stamp_sig already
/// present is replaced: re-stamping is a new mint by the signing
/// authority.
pub fn sign_stamp(provenance: &Value, signer: &StampSigner) -> Result<Value, String> {
    let signature: Signature = signer
        .signing_key
        .sign(&canonical_provenance_bytes(provenance)?);
    let stamp_sig = format!(
        "v1.ed25519.{}.{}",
        signer.key_id,
        b64url_encode(&signature.to_bytes())
    );
    let mut out = provenance
        .as_object()
        .cloned()
        .ok_or_else(|| "provenance must be an object".to_string())?;
    out.insert("stamp_sig".to_string(), Value::String(stamp_sig));
    Ok(Value::Object(out))
}

/// Outcome of verifying a block's provenance (§15's reporting:
/// verified, diverged, or unsigned — plus unverifiable when this
/// reader holds no keys).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StampVerification {
    Verified { key_id: String },
    Unsigned,
    Diverged { reason: &'static str },
    Unverifiable { reason: &'static str },
}

/// Verify a block's provenance — the same code path both editions'
/// stamps go through. A forged or edited block is data to report
/// (Diverged), not an error. Only a provenance that cannot be
/// canonicalized at all returns Err, because then no verification
/// statement about it is possible (the hosted edition throws the
/// same way).
pub fn verify_stamp(
    provenance: &Value,
    verify_keys: &HashMap<String, [u8; 32]>,
) -> Result<StampVerification, String> {
    let Some(map) = provenance.as_object() else {
        return Err("provenance must be an object".to_string());
    };
    let Some(stamp_sig) = map.get("stamp_sig") else {
        // Pre-signature legacy blocks are unsigned, not forgeries.
        return Ok(StampVerification::Unsigned);
    };
    if stamp_sig.is_null() {
        return Ok(StampVerification::Unsigned);
    }
    let Some(text) = stamp_sig.as_str() else {
        return Ok(StampVerification::Diverged {
            reason: "malformed_stamp_sig",
        });
    };
    let parts: Vec<&str> = text.split('.').collect();
    if parts.len() != 4 {
        return Ok(StampVerification::Diverged {
            reason: "malformed_stamp_sig",
        });
    }
    let (version, algorithm, key_id, sig_text) = (parts[0], parts[1], parts[2], parts[3]);
    if version != "v1" || algorithm != "ed25519" {
        return Ok(StampVerification::Diverged {
            reason: "unsupported_stamp_version",
        });
    }
    if !is_valid_stamp_key_id(key_id) || sig_text.is_empty() {
        return Ok(StampVerification::Diverged {
            reason: "malformed_stamp_sig",
        });
    }
    let Ok(sig_bytes) = b64url_decode(sig_text) else {
        return Ok(StampVerification::Diverged {
            reason: "malformed_stamp_sig",
        });
    };
    let Ok(sig_array) = <[u8; 64]>::try_from(sig_bytes.as_slice()) else {
        return Ok(StampVerification::Diverged {
            reason: "malformed_stamp_sig",
        });
    };
    if verify_keys.is_empty() {
        return Ok(StampVerification::Unverifiable {
            reason: "no_verify_keys_configured",
        });
    }
    let Some(raw_key) = verify_keys.get(key_id) else {
        return Ok(StampVerification::Diverged {
            reason: "unknown_key",
        });
    };
    let Ok(verifying_key) = VerifyingKey::from_bytes(raw_key) else {
        return Ok(StampVerification::Diverged {
            reason: "unknown_key",
        });
    };
    let signature = Signature::from_bytes(&sig_array);
    match verifying_key.verify(&canonical_provenance_bytes(provenance)?, &signature) {
        Ok(()) => Ok(StampVerification::Verified {
            key_id: key_id.to_string(),
        }),
        Err(_) => Ok(StampVerification::Diverged {
            reason: "signature_mismatch",
        }),
    }
}

// ---------------------------------------------------------------------
// Provenance assembly (§13's stamp set, from fleet sources).

/// The queue record's stamp-relevant facts, read from the queue
/// broker — never from caller assertion (the hosted
/// QueueRecordSource's shape).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueueRecord {
    /// The queue task id, in string form (the form #9074's index
    /// column and the hosted route both carry).
    pub queue_id: Option<String>,
    /// The queue packet's scheduled start (`not_before`), when set.
    pub scheduled_start: Option<String>,
    /// The claim record's pickup time (`claimed_at`).
    pub picked_up_at: Option<String>,
    /// The work-start time, when a lane recorded one in progress.
    pub work_started_at: Option<String>,
    /// The claim's lane (`lane_id`).
    pub lane: Option<String>,
    /// The claim's model stamp (`claim_model`).
    pub model: Option<String>,
    /// The issue numbers of every task the same lane start
    /// (session) holds — §13's "issues worked by the same lane
    /// start".
    pub lane_start_issues: Vec<i64>,
}

/// Everything the stamp assembly reads from the caller's context.
/// Gathered by `gather_facts`; injectable so the assembly is
/// unit-testable without a broker, a session, or a clock.
pub struct StampFacts {
    pub now_unix: i64,
    pub session_id: Option<String>,
    pub host: Option<String>,
    pub worktree: Option<String>,
    pub workspace_ref: Option<Value>,
    pub auth_subject: Option<String>,
    pub queue: Option<QueueRecord>,
}

/// Assemble the full provenance set. Every key is always present —
/// the stamp set is stamped in full, so absence and null are not
/// interchangeable after signing; a fact the fleet does not know
/// stamps null.
pub fn assemble_provenance(facts: &StampFacts) -> Value {
    let queue = facts.queue.clone().unwrap_or_default();
    let mut prov = Map::new();
    prov.insert("queue_id".to_string(), opt_string(queue.queue_id));
    prov.insert(
        "scheduled_start".to_string(),
        opt_string(queue.scheduled_start),
    );
    prov.insert("picked_up_at".to_string(), opt_string(queue.picked_up_at));
    prov.insert(
        "work_started_at".to_string(),
        opt_string(queue.work_started_at),
    );
    prov.insert(
        "created_at".to_string(),
        Value::String(rfc3339(facts.now_unix)),
    );
    prov.insert(
        "session_id".to_string(),
        opt_string(facts.session_id.clone()),
    );
    prov.insert("lane".to_string(), opt_string(queue.lane));
    prov.insert("host".to_string(), opt_string(facts.host.clone()));
    prov.insert("worktree".to_string(), opt_string(facts.worktree.clone()));
    prov.insert("model".to_string(), opt_string(queue.model));
    prov.insert(
        "lane_start_issues".to_string(),
        Value::Array(
            queue
                .lane_start_issues
                .iter()
                .map(|n| Value::from(*n))
                .collect(),
        ),
    );
    prov.insert(
        "workspace_ref".to_string(),
        facts.workspace_ref.clone().unwrap_or(Value::Null),
    );
    prov.insert(
        "auth_subject".to_string(),
        opt_string(facts.auth_subject.clone()),
    );
    prov.insert(
        "stamped_by".to_string(),
        Value::String(format!("fleet {} (native)", env!("CARGO_PKG_VERSION"))),
    );
    Value::Object(prov)
}

fn opt_string(value: Option<String>) -> Value {
    value.map_or(Value::Null, Value::String)
}

fn nonempty_str(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// The issue number a queue task row works, from its packet
/// (payload.issue.number), with the dedupe-key fallback the close
/// guard uses.
fn row_issue_number(row: &Value) -> Option<i64> {
    let payload = row.get("payload")?;
    payload
        .get("issue")
        .and_then(|i| i.get("number"))
        .and_then(Value::as_i64)
        .or_else(|| {
            let repo = payload.get("repo")?.as_str()?;
            let key = row.get("dedupe_key")?.as_str()?;
            let marker = format!("{repo}:issue:");
            key.split_once(&marker)?
                .1
                .split(|c: char| !c.is_ascii_digit())
                .next()?
                .parse()
                .ok()
        })
}

/// Build the queue record for a session from queue task rows (the
/// broker's rich list shape): the session's live (claimed/parked)
/// tasks, the most recently claimed of them as the primary record,
/// and every issue the lane start holds. Pure over the rows so the
/// mapping is unit-testable without a broker.
pub fn queue_record_from_rows(rows: &[Value], session_id: &str) -> Option<QueueRecord> {
    let mine: Vec<&Value> = rows
        .iter()
        .filter(|row| {
            matches!(
                row.get("status").and_then(Value::as_str),
                Some("claimed" | "parked")
            ) && row.get("claim_session_id").and_then(Value::as_str) == Some(session_id)
        })
        .collect();
    let primary = mine.iter().max_by(|a, b| {
        let key = |row: &Value| {
            (
                row.get("claimed_at")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                row.get("id").and_then(Value::as_i64).unwrap_or(0),
            )
        };
        key(a).cmp(&key(b))
    })?;
    let mut issues: Vec<i64> = mine
        .iter()
        .filter_map(|row| row_issue_number(row))
        .collect();
    issues.sort_unstable();
    issues.dedup();
    let str_field = |name: &str| {
        primary
            .get(name)
            .and_then(Value::as_str)
            .and_then(|v| nonempty_str(Some(v)))
    };
    Some(QueueRecord {
        queue_id: primary
            .get("id")
            .and_then(Value::as_i64)
            .map(|id| id.to_string()),
        scheduled_start: str_field("not_before"),
        picked_up_at: str_field("claimed_at"),
        work_started_at: primary
            .get("progress")
            .and_then(|p| p.get("work_started_at").or_else(|| p.get("started_at")))
            .and_then(Value::as_str)
            .and_then(|v| nonempty_str(Some(v))),
        lane: str_field("lane_id"),
        model: str_field("claim_model"),
        lane_start_issues: issues,
    })
}

/// The production queue-record source: the queue broker's rich list,
/// scanned for the caller's session binding — the same read the
/// close guard performs. Broker errors answer None (fail-open: a
/// broker outage must not block or mis-stamp a create; the fields
/// stamp null and ingestion is the net). In builds without the
/// queue client there is no broker to ask, so the record is None
/// the same way.
#[cfg(feature = "queue")]
pub fn queue_record_for_session(session_id: &str) -> Option<QueueRecord> {
    let rows = fleet_queue::client::guard_rows(500).ok()?;
    queue_record_from_rows(&rows, session_id)
}

#[cfg(not(feature = "queue"))]
pub fn queue_record_for_session(_session_id: &str) -> Option<QueueRecord> {
    None
}

// ---------------------------------------------------------------------
// Extraction: #9074's versioned reader rules (parsePmMetadata),
// ported — the create path stores exactly what the aggregator's
// store would extract from the same block, so both writers agree.

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Extracted {
    pub direct_parent_repo: Option<String>,
    pub direct_parent_number: Option<i64>,
    pub session_id: Option<String>,
    pub lane: Option<String>,
    pub host: Option<String>,
    pub queue_id: Option<String>,
    /// The indexes-form links (only correctly shaped fields kept),
    /// stored as the items row's links_json.
    pub links: Vec<Map<String, Value>>,
    /// The derived link rows for pm_metadata_links, with the raw
    /// link object each row was derived from.
    pub link_rows: Vec<(LinkRow, Value)>,
    /// Unknown keys and wrong-shaped registered values, preserved
    /// rather than discarded (#9074's extras rule).
    pub extras: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinkRow {
    pub target_repo: Option<String>,
    pub target_number: Option<i64>,
    pub relation: Option<String>,
    pub status: Option<String>,
}

fn nonempty_json_str(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn positive_json_int(value: &Value) -> Option<i64> {
    value.as_i64().filter(|n| *n > 0)
}

pub fn extract(metadata: &Value) -> Extracted {
    let mut out = Extracted::default();
    let Some(block) = metadata.as_object() else {
        return out;
    };
    const REGISTERED: [&str; 5] = ["pm_meta", "direct_parent", "links", "lineage", "provenance"];
    for (key, value) in block {
        if !REGISTERED.contains(&key.as_str()) {
            out.extras.insert(key.clone(), value.clone());
        }
    }
    match block.get("direct_parent") {
        None | Some(Value::Null) => {}
        Some(parent) => {
            let shaped = parent.as_object().and_then(|p| {
                Some((
                    nonempty_json_str(p.get("repo")?)?,
                    positive_json_int(p.get("number")?)?,
                    nonempty_json_str(p.get("kind")?)?,
                ))
            });
            match shaped {
                Some((repo, number, _kind)) => {
                    out.direct_parent_repo = Some(repo);
                    out.direct_parent_number = Some(number);
                }
                None => {
                    out.extras
                        .insert("direct_parent".to_string(), parent.clone());
                }
            }
        }
    }
    if let Some(provenance) = block.get("provenance") {
        match provenance.as_object() {
            None => {
                if !provenance.is_null() {
                    out.extras
                        .insert("provenance".to_string(), provenance.clone());
                }
            }
            Some(prov) => {
                const KNOWN: [&str; 10] = [
                    "queue_id",
                    "session_id",
                    "lane",
                    "host",
                    "worktree",
                    "picked_up_at",
                    "work_started_at",
                    "created_at",
                    "model",
                    "lane_start_issues",
                ];
                for (key, value) in prov {
                    if !KNOWN.contains(&key.as_str()) {
                        out.extras
                            .insert(format!("provenance.{key}"), value.clone());
                    }
                }
                for field in ["session_id", "lane", "host", "queue_id"] {
                    match prov.get(field) {
                        None | Some(Value::Null) => {}
                        Some(value) => match nonempty_json_str(value) {
                            Some(text) => match field {
                                "session_id" => out.session_id = Some(text),
                                "lane" => out.lane = Some(text),
                                "host" => out.host = Some(text),
                                _ => out.queue_id = Some(text),
                            },
                            None => {
                                out.extras
                                    .insert(format!("provenance.{field}"), value.clone());
                            }
                        },
                    }
                }
            }
        }
    }
    match block.get("links") {
        None | Some(Value::Null) => {}
        Some(Value::Array(links)) => {
            for (position, raw) in links.iter().enumerate() {
                let mut index_link = Map::new();
                let mut row = LinkRow::default();
                let target = raw.get("target");
                let target_repo = target
                    .and_then(|t| t.get("repo"))
                    .and_then(nonempty_json_str);
                let target_number = target
                    .and_then(|t| t.get("number"))
                    .and_then(positive_json_int);
                let relation = raw.get("relation").and_then(nonempty_json_str);
                let status = raw.get("status").and_then(nonempty_json_str);
                let well_shaped = raw.is_object()
                    && target.is_some_and(|t| t.is_object())
                    && target_repo.is_some()
                    && target_number.is_some()
                    && relation.is_some()
                    && status.is_some();
                if !well_shaped {
                    out.extras.insert(format!("links.{position}"), raw.clone());
                }
                if let Some(repo) = &target_repo {
                    index_link.insert("targetRepo".to_string(), Value::String(repo.clone()));
                    row.target_repo = Some(repo.clone());
                }
                if let Some(number) = target_number {
                    index_link.insert("targetNumber".to_string(), Value::from(number));
                    row.target_number = Some(number);
                }
                if let Some(rel) = &relation {
                    index_link.insert("relation".to_string(), Value::String(rel.clone()));
                    row.relation = Some(rel.clone());
                }
                if let Some(status_text) = &status {
                    index_link.insert("status".to_string(), Value::String(status_text.clone()));
                    row.status = Some(status_text.clone());
                }
                out.links.push(index_link);
                out.link_rows.push((row, raw.clone()));
            }
        }
        Some(other) => {
            out.extras.insert("links".to_string(), other.clone());
        }
    }
    out
}

// ---------------------------------------------------------------------
// The metadata store (§15): #9074's pm_metadata schema over SQLite.

/// One metadata DB row, as the create path writes it and the query
/// side reads it back.
#[derive(Debug, Clone, PartialEq)]
pub struct MetadataRecord {
    pub repo: String,
    /// "issue" or "pr".
    pub kind: String,
    pub number: i64,
    pub url: String,
    /// The full Metadata block, exactly as stamped into the body.
    pub metadata: Value,
    /// The block's own stamp time (provenance.created_at).
    pub source_updated_at: Option<String>,
}

/// A stored row read back in full (the #9074 get shape).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredRow {
    pub artifact_id: String,
    pub repo: String,
    pub number: i64,
    pub kind: String,
    pub url: String,
    pub schema_version: i64,
    pub metadata: Value,
    pub extras: Value,
    pub extracted: Extracted,
    pub source_updated_at: Option<String>,
}

/// The artifact identity of an item this path creates:
/// deterministic from the item's own identity (see module header).
pub fn artifact_id(repo: &str, kind: &str, number: i64) -> String {
    format!("gh:{repo}#{kind}:{number}")
}

pub trait MetadataStore {
    fn insert(&self, record: &MetadataRecord) -> Result<(), String>;
    fn get(&self, artifact_id: &str) -> Result<Option<StoredRow>, String>;
}

/// #9074's DDL, verbatim (PM_METADATA_STORE_DDL).
const PM_METADATA_DDL: [&str; 9] = [
    "CREATE TABLE IF NOT EXISTS pm_metadata_items (
    artifact_id TEXT PRIMARY KEY,
    repo TEXT NOT NULL,
    item_number INTEGER NOT NULL,
    item_kind TEXT NOT NULL,
    source_url TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    metadata_json TEXT NOT NULL,
    extras_json TEXT NOT NULL,
    direct_parent_repo TEXT,
    direct_parent_number INTEGER,
    session_id TEXT,
    lane TEXT,
    host TEXT,
    queue_id TEXT,
    links_json TEXT NOT NULL,
    source_updated_at TEXT
  )",
    "CREATE INDEX IF NOT EXISTS pm_metadata_parent_idx ON pm_metadata_items (direct_parent_repo, direct_parent_number)",
    "CREATE INDEX IF NOT EXISTS pm_metadata_session_idx ON pm_metadata_items (session_id)",
    "CREATE INDEX IF NOT EXISTS pm_metadata_lane_idx ON pm_metadata_items (lane)",
    "CREATE INDEX IF NOT EXISTS pm_metadata_host_idx ON pm_metadata_items (host)",
    "CREATE INDEX IF NOT EXISTS pm_metadata_queue_idx ON pm_metadata_items (queue_id)",
    "CREATE TABLE IF NOT EXISTS pm_metadata_links (
    artifact_id TEXT NOT NULL,
    link_index INTEGER NOT NULL,
    target_repo TEXT,
    target_number INTEGER,
    relation TEXT,
    status TEXT,
    link_json TEXT NOT NULL,
    PRIMARY KEY (artifact_id, link_index)
  )",
    "CREATE INDEX IF NOT EXISTS pm_metadata_link_status_idx ON pm_metadata_links (status, relation)",
    "CREATE INDEX IF NOT EXISTS pm_metadata_link_target_idx ON pm_metadata_links (target_repo, target_number)",
];

/// The production store: a SQLite file in #9074's schema. Upsert
/// semantics are #9074's: replaying an artifact replaces its row
/// and rebuilds its derived link rows in one transaction.
pub struct SqliteMetadataStore {
    path: PathBuf,
}

impl SqliteMetadataStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The default store path: `$FLEET_METADATA_DB`, else
    /// `<fleet state dir>/metadata.db`.
    pub fn default_path() -> PathBuf {
        if let Ok(p) = std::env::var(METADATA_DB_ENV) {
            if !p.trim().is_empty() {
                return PathBuf::from(p);
            }
        }
        Config::state_dir().join("metadata.db")
    }

    fn open(&self) -> Result<rusqlite::Connection, String> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
            }
        }
        let conn = rusqlite::Connection::open(&self.path)
            .map_err(|e| format!("cannot open metadata store {}: {e}", self.path.display()))?;
        conn.execute_batch(&PM_METADATA_DDL.join(";\n"))
            .map_err(|e| format!("cannot initialize metadata store: {e}"))?;
        Ok(conn)
    }
}

impl MetadataStore for SqliteMetadataStore {
    fn insert(&self, record: &MetadataRecord) -> Result<(), String> {
        if record.repo.is_empty() || record.url.is_empty() || record.number <= 0 {
            return Err("metadata record needs a repo, a URL and a positive number".to_string());
        }
        let extracted = extract(&record.metadata);
        let artifact = artifact_id(&record.repo, &record.kind, record.number);
        let schema_version = record
            .metadata
            .get("pm_meta")
            .and_then(Value::as_i64)
            .unwrap_or(1);
        let metadata_json = record.metadata.to_string();
        let extras_json = Value::Object(extracted.extras.clone()).to_string();
        let links_json = Value::Array(
            extracted
                .links
                .iter()
                .map(|l| Value::Object(l.clone()))
                .collect(),
        )
        .to_string();
        let mut conn = self.open()?;
        let tx = conn
            .transaction()
            .map_err(|e| format!("metadata store transaction failed: {e}"))?;
        tx.execute(
            "INSERT INTO pm_metadata_items (
              artifact_id, repo, item_number, item_kind, source_url, schema_version,
              metadata_json, extras_json, direct_parent_repo, direct_parent_number,
              session_id, lane, host, queue_id, links_json, source_updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            ON CONFLICT(artifact_id) DO UPDATE SET
              repo = excluded.repo,
              item_number = excluded.item_number,
              item_kind = excluded.item_kind,
              source_url = excluded.source_url,
              schema_version = excluded.schema_version,
              metadata_json = excluded.metadata_json,
              extras_json = excluded.extras_json,
              direct_parent_repo = excluded.direct_parent_repo,
              direct_parent_number = excluded.direct_parent_number,
              session_id = excluded.session_id,
              lane = excluded.lane,
              host = excluded.host,
              queue_id = excluded.queue_id,
              links_json = excluded.links_json,
              source_updated_at = excluded.source_updated_at",
            rusqlite::params![
                artifact,
                record.repo,
                record.number,
                record.kind,
                record.url,
                schema_version,
                metadata_json,
                extras_json,
                extracted.direct_parent_repo,
                extracted.direct_parent_number,
                extracted.session_id,
                extracted.lane,
                extracted.host,
                extracted.queue_id,
                links_json,
                record.source_updated_at,
            ],
        )
        .map_err(|e| format!("metadata insert failed: {e}"))?;
        tx.execute(
            "DELETE FROM pm_metadata_links WHERE artifact_id = ?1",
            rusqlite::params![artifact],
        )
        .map_err(|e| format!("metadata link rebuild failed: {e}"))?;
        for (index, (row, raw)) in extracted.link_rows.iter().enumerate() {
            tx.execute(
                "INSERT INTO pm_metadata_links (
                  artifact_id, link_index, target_repo, target_number, relation, status, link_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    artifact,
                    index as i64,
                    row.target_repo,
                    row.target_number,
                    row.relation,
                    row.status,
                    raw.to_string(),
                ],
            )
            .map_err(|e| format!("metadata link insert failed: {e}"))?;
        }
        tx.commit()
            .map_err(|e| format!("metadata store commit failed: {e}"))?;
        Ok(())
    }

    fn get(&self, artifact: &str) -> Result<Option<StoredRow>, String> {
        let conn = self.open()?;
        let row = conn
            .query_row(
                "SELECT artifact_id, repo, item_number, item_kind, source_url,
                        schema_version, metadata_json, extras_json,
                        direct_parent_repo, direct_parent_number,
                        session_id, lane, host, queue_id, source_updated_at
                 FROM pm_metadata_items WHERE artifact_id = ?1",
                rusqlite::params![artifact],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, i64>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, String>(7)?,
                        r.get::<_, Option<String>>(8)?,
                        r.get::<_, Option<i64>>(9)?,
                        r.get::<_, Option<String>>(10)?,
                        r.get::<_, Option<String>>(11)?,
                        r.get::<_, Option<String>>(12)?,
                        r.get::<_, Option<String>>(13)?,
                        r.get::<_, Option<String>>(14)?,
                    ))
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => String::new(),
                other => format!("metadata read failed: {other}"),
            });
        let (
            artifact_id,
            repo,
            number,
            kind,
            url,
            schema_version,
            metadata_json,
            extras_json,
            dp_repo,
            dp_number,
            session_id,
            lane,
            host,
            queue_id,
            source_updated_at,
        ) = match row {
            Ok(values) => values,
            Err(e) if e.is_empty() => return Ok(None),
            Err(e) => return Err(e),
        };
        let metadata: Value = serde_json::from_str(&metadata_json)
            .map_err(|e| format!("stored metadata_json does not parse: {e}"))?;
        let extras: Value = serde_json::from_str(&extras_json)
            .map_err(|e| format!("stored extras_json does not parse: {e}"))?;
        // Rebuild the extracted view from the stored block (the same
        // reader rules), then overlay the indexed columns so the
        // readback is the row's truth, links included.
        let mut extracted = extract(&metadata);
        extracted.direct_parent_repo = dp_repo;
        extracted.direct_parent_number = dp_number;
        extracted.session_id = session_id;
        extracted.lane = lane;
        extracted.host = host;
        extracted.queue_id = queue_id;
        let mut stmt = conn
            .prepare(
                "SELECT target_repo, target_number, relation, status, link_json
                 FROM pm_metadata_links WHERE artifact_id = ?1 ORDER BY link_index",
            )
            .map_err(|e| format!("metadata link read failed: {e}"))?;
        let link_rows = stmt
            .query_map(rusqlite::params![artifact], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| format!("metadata link read failed: {e}"))?;
        let mut stored_links = Vec::new();
        for link in link_rows {
            let (target_repo, target_number, relation, status, link_json) =
                link.map_err(|e| format!("metadata link read failed: {e}"))?;
            let raw: Value = serde_json::from_str(&link_json).unwrap_or(Value::Null);
            stored_links.push((
                LinkRow {
                    target_repo,
                    target_number,
                    relation,
                    status,
                },
                raw,
            ));
        }
        extracted.link_rows = stored_links;
        Ok(Some(StoredRow {
            artifact_id,
            repo,
            number,
            kind,
            url,
            schema_version,
            metadata,
            extras,
            extracted,
            source_updated_at,
        }))
    }
}

// ---------------------------------------------------------------------
// Create recognition: `issue create` / `pr create` argument surgery.

#[derive(Debug)]
enum BodySource {
    Inline(String),
    File(PathBuf),
    Absent,
}

/// A recognized, stampable create call: the normalized gh arguments
/// (the body carried as `--body <value>`; the slot index is where
/// the stamped body lands) plus where the caller's body text lives.
#[derive(Debug)]
pub struct CreateIntent {
    pub kind: &'static str,
    pub repo: Option<String>,
    args: Vec<String>,
    body_slot: Option<usize>,
    source: BodySource,
}

impl CreateIntent {
    /// The caller's body text. A body file that cannot be read
    /// refuses the create (gh would fail on it too; the Metadata
    /// layer names the problem first).
    pub fn caller_body(&self) -> Result<String, String> {
        match &self.source {
            BodySource::Inline(text) => Ok(text.clone()),
            BodySource::File(path) => std::fs::read_to_string(path).map_err(|e| {
                format!(
                    "fleet gh: cannot read --body-file {} for Metadata stamping: {e}",
                    path.display()
                )
            }),
            BodySource::Absent => Ok(String::new()),
        }
    }

    /// The final gh arguments with the stamped body in place.
    pub fn apply(mut self, stamped_body: String) -> Vec<String> {
        match self.body_slot {
            Some(slot) => {
                self.args[slot] = stamped_body;
            }
            None => {
                self.args.push("--body".to_string());
                self.args.push(stamped_body);
            }
        }
        self.args
    }
}

/// gh create flags that take a value (besides --body/--body-file/
/// --repo, handled on their own). A value that merely looks like a
/// flag (a title of "--body") must not be mistaken for one.
const VALUE_FLAGS: [&str; 23] = [
    "--title",
    "-t",
    "--assignee",
    "-a",
    "--milestone",
    "-m",
    "--project",
    "-p",
    "--label",
    "-l",
    "--template",
    "-T",
    "--type",
    "--parent",
    "--blocked-by",
    "--blocking",
    "--base",
    "-B",
    "--head",
    "-H",
    "--reviewer",
    "-r",
    "--team-reviewer",
];

/// Recognize `issue create` / `pr create` calls whose body this
/// layer can stamp. Returns Ok(None) for everything else — every
/// other verb, and the creates listed in the module header that
/// leave the body to a human or to gh itself.
pub fn parse_create(
    args: &[String],
    stdin_is_terminal: bool,
) -> Result<Option<CreateIntent>, String> {
    if args.len() < 2 {
        return Ok(None);
    }
    let kind = match args[0].as_str() {
        "issue" => "issue",
        "pr" => "pr",
        _ => return Ok(None),
    };
    if args[1] != "create" {
        return Ok(None);
    }
    let mut out: Vec<String> = vec![args[0].clone(), args[1].clone()];
    let mut repo: Option<String> = None;
    let mut source = BodySource::Absent;
    let mut body_slot: Option<usize> = None;
    let mut saw_inline_body = false;
    let mut saw_file_body = false;
    let mut saw_web = false;
    let mut saw_fill = false;
    let mut saw_template = false;
    let mut saw_help = false;
    let mut i = 2;
    while i < args.len() {
        let token = &args[i];
        if token == "--" {
            out.extend_from_slice(&args[i..]);
            break;
        }
        // Attached short forms for the two body flags (-b<text>,
        // -F<path>): rewrite to the normalized two-token form.
        if !token.starts_with("--") && token.len() > 2 {
            if let Some(value) = token.strip_prefix("-b") {
                let value = value.strip_prefix('=').unwrap_or(value);
                saw_inline_body = true;
                source = BodySource::Inline(value.to_string());
                out.push("--body".to_string());
                out.push(value.to_string());
                body_slot = Some(out.len() - 1);
                i += 1;
                continue;
            }
            if let Some(value) = token.strip_prefix("-F") {
                let value = value.strip_prefix('=').unwrap_or(value);
                saw_file_body = true;
                source = BodySource::File(PathBuf::from(value));
                i += 1;
                continue;
            }
        }
        let (name, inline) = match token.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (token.as_str(), None),
        };
        let take_value = |i: &mut usize| -> Result<String, String> {
            if let Some(v) = &inline {
                return Ok(v.clone());
            }
            *i += 1;
            args.get(*i)
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match name {
            "--body" | "-b" => {
                let value = take_value(&mut i)?;
                saw_inline_body = true;
                source = BodySource::Inline(value.clone());
                out.push("--body".to_string());
                out.push(value);
                body_slot = Some(out.len() - 1);
            }
            "--body-file" | "-F" => {
                let value = take_value(&mut i)?;
                saw_file_body = true;
                source = BodySource::File(PathBuf::from(value));
                // Dropped from the normalized args: apply() appends
                // --body <stamped> instead, so the caller's file is
                // never mutated.
            }
            "--repo" | "-R" => {
                let value = take_value(&mut i)?;
                repo = Some(value.clone());
                out.push(name.to_string());
                out.push(value);
            }
            "--web" | "-w" => {
                saw_web = true;
                out.push(token.clone());
            }
            "--fill" | "--fill-first" | "--fill-verbose" => {
                saw_fill = true;
                out.push(token.clone());
            }
            "--help" | "-h" => {
                saw_help = true;
                out.push(token.clone());
            }
            _ if VALUE_FLAGS.contains(&name) => {
                if name == "--template" || name == "-T" {
                    saw_template = true;
                }
                let value = take_value(&mut i)?;
                out.push(name.to_string());
                out.push(value);
            }
            _ => {
                out.push(token.clone());
            }
        }
        i += 1;
    }
    if saw_help || saw_web {
        return Ok(None);
    }
    if saw_inline_body && saw_file_body {
        // gh itself refuses the combination; let it.
        return Ok(None);
    }
    match &source {
        BodySource::File(path) if path.as_os_str() == "-" => return Ok(None),
        BodySource::Absent if saw_template || saw_fill => return Ok(None),
        BodySource::Absent if stdin_is_terminal => return Ok(None),
        _ => {}
    }
    Ok(Some(CreateIntent {
        kind,
        repo,
        args: out,
        body_slot,
        source,
    }))
}

// ---------------------------------------------------------------------
// Facts from the caller's context.

fn hostname() -> Option<String> {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: gethostname writes a NUL-terminated string into
        // the buffer or fails; the buffer outlives the call.
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
        if rc == 0 {
            let len = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
            return String::from_utf8(buf[..len].to_vec())
                .ok()
                .filter(|s| !s.is_empty());
        }
        None
    }
    #[cfg(not(unix))]
    {
        std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty())
    }
}

fn git_output(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

fn repo_from_git_remote() -> Option<String> {
    let remote = git_output(&["remote", "get-url", "origin"])?;
    crate::gh_guard::github_repo_from_remote(&remote)
}

/// The caller's fleet identity for `auth_subject`: the agent name
/// (FLEET_AGENT, else the enrolled agent profile's name) — the
/// native analog of the hosted edition's credential-resolved
/// binding_id.
fn agent_identity() -> Option<String> {
    if let Some(name) = nonempty_str(std::env::var("FLEET_AGENT").ok().as_deref()) {
        return Some(name);
    }
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".config"))
        })?;
    let text = std::fs::read_to_string(config_home.join("fleet").join("agent.json")).ok()?;
    let profile: Value = serde_json::from_str(&text).ok()?;
    nonempty_str(profile.get("name").and_then(Value::as_str))
}

fn gather_facts(repo_flag: Option<&str>) -> StampFacts {
    let session_id = nonempty_str(std::env::var("FLEET_SESSION_ID").ok().as_deref())
        .or_else(|| crate::session_state::initialize(None).ok().flatten());
    let queue = session_id.as_deref().and_then(queue_record_for_session);
    let host = hostname();
    let worktree = git_output(&["rev-parse", "--show-toplevel"]);
    let repo = nonempty_str(repo_flag)
        .or_else(|| nonempty_str(std::env::var("GH_REPO").ok().as_deref()))
        .or_else(repo_from_git_remote);
    let workspace_ref = if let Some(wt) = &worktree {
        let mut map = Map::new();
        map.insert("kind".to_string(), Value::String("worktree".to_string()));
        map.insert("id".to_string(), Value::String(wt.clone()));
        map.insert(
            "repo".to_string(),
            repo.clone().map_or(Value::Null, Value::String),
        );
        map.insert(
            "branch".to_string(),
            git_output(&["rev-parse", "--abbrev-ref", "HEAD"])
                .filter(|b| b != "HEAD")
                .map_or(Value::Null, Value::String),
        );
        map.insert(
            "sha".to_string(),
            git_output(&["rev-parse", "HEAD"]).map_or(Value::Null, Value::String),
        );
        Some(Value::Object(map))
    } else {
        session_id.as_ref().map(|session| {
            let mut map = Map::new();
            map.insert("kind".to_string(), Value::String("session".to_string()));
            map.insert("id".to_string(), Value::String(session.clone()));
            map.insert(
                "repo".to_string(),
                repo.clone().map_or(Value::Null, Value::String),
            );
            map.insert("branch".to_string(), Value::Null);
            map.insert("sha".to_string(), Value::Null);
            Value::Object(map)
        })
    };
    StampFacts {
        now_unix: now_unix(),
        session_id,
        host,
        worktree,
        workspace_ref,
        auth_subject: agent_identity(),
        queue,
    }
}

/// The configured stamp signer, when FLEET_GH_STAMP_KEY_ID is set:
/// the key material resolved by name from the secrets broker. An
/// unset key id is Ok(None) (unsigned by configuration); a set key
/// id whose material cannot be resolved or imported is Err — the
/// caller notes it on stderr and stamps unsigned, because a native
/// CLI create must not die on a provisioning gap.
fn configured_signer(config: &Config) -> Result<Option<StampSigner>, String> {
    let Some(key_id) = nonempty_str(std::env::var(STAMP_KEY_ID_ENV).ok().as_deref()) else {
        return Ok(None);
    };
    let resolver = fleet_secrets::Resolver::default_chain(config);
    let material = resolver
        .resolve(STAMP_KEY_SECRET_NAME)
        .map_err(|e| format!("cannot resolve secret {STAMP_KEY_SECRET_NAME}: {e}"))?;
    signer_from_pkcs8_base64(&key_id, material.expose()).map(Some)
}

// ---------------------------------------------------------------------
// Prepare (before the create runs) and finish (after it succeeds).

pub struct PreparedCreate {
    /// The gh arguments with the stamped body in place.
    pub args: Vec<String>,
    /// The stamped Metadata block (what the DB record stores).
    pub block: Value,
}

/// Recognize a create in `args`, stamp its body, and return the
/// rewritten call. Ok(None) passes the call through untouched. Err
/// refuses the create before any GitHub write (a malformed caller
/// block, or a body file that cannot be read).
pub fn prepare_create(
    config: &Config,
    args: &[String],
    stdin_is_terminal: bool,
) -> Result<Option<PreparedCreate>, String> {
    let Some(intent) = parse_create(args, stdin_is_terminal)? else {
        return Ok(None);
    };
    let caller_body = intent.caller_body()?;
    let parsed = parse_metadata_block(&caller_body)?;
    let facts = gather_facts(intent.repo.as_deref());
    let mut provenance = assemble_provenance(&facts);
    match configured_signer(config) {
        Ok(Some(signer)) => {
            provenance = sign_stamp(&provenance, &signer)?;
        }
        Ok(None) => {}
        Err(e) => {
            eprintln!(
                "fleet gh: stamp signing is configured but unavailable ({e}); \
                 the Metadata block stamps unsigned"
            );
        }
    }
    let block = assemble_stamped_block(parsed.block.as_ref(), provenance);
    let stamped_body = render_body_with_block(&parsed, &block);
    Ok(Some(PreparedCreate {
        args: intent.apply(stamped_body),
        block,
    }))
}

/// The created item, read back from gh's own output line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedItem {
    pub repo: String,
    pub kind: String,
    pub number: i64,
    pub url: String,
}

/// gh prints the created item's URL on stdout. Parse the first
/// github.com issue/PR URL line; anything else means the record
/// cannot be written from this call (ingestion heals it).
pub fn parse_created_url(stdout: &str) -> Option<CreatedItem> {
    for line in stdout.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("https://github.com/") else {
            continue;
        };
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() != 4 || parts[0].is_empty() || parts[1].is_empty() {
            continue;
        }
        let kind = match parts[2] {
            "issues" => "issue",
            "pull" => "pr",
            _ => continue,
        };
        let Ok(number) = parts[3].parse::<i64>() else {
            continue;
        };
        if number <= 0 {
            continue;
        }
        return Some(CreatedItem {
            repo: format!("{}/{}", parts[0], parts[1]),
            kind: kind.to_string(),
            number,
            url: line.to_string(),
        });
    }
    None
}

pub enum CreateOutcome {
    Inserted(CreatedItem),
    InsertFailed {
        item: CreatedItem,
        error: String,
    },
    /// gh's output carried no item URL; no record was written.
    NoUrl,
}

/// After a successful create: insert the full record into the
/// metadata store. The outcome is reported, never fatal — the
/// GitHub item already exists and gh's exit code stands.
pub fn finish_create(store: &dyn MetadataStore, stdout: &str, block: &Value) -> CreateOutcome {
    let Some(item) = parse_created_url(stdout) else {
        return CreateOutcome::NoUrl;
    };
    let record = MetadataRecord {
        repo: item.repo.clone(),
        kind: item.kind.clone(),
        number: item.number,
        url: item.url.clone(),
        metadata: block.clone(),
        source_updated_at: block
            .get("provenance")
            .and_then(|p| p.get("created_at"))
            .and_then(Value::as_str)
            .map(str::to_string),
    };
    match store.insert(&record) {
        Ok(()) => CreateOutcome::Inserted(item),
        Err(error) => CreateOutcome::InsertFailed { item, error },
    }
}

// ---------------------------------------------------------------------
// Tests. Oracles: the hosted edition's golden vectors (byte-exact),
// #9074's extraction rules, the design's acceptance cases, and gh's
// own argument contract.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fleet-gh-metadata-test-{}-{}-{tag}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // -- clock ------------------------------------------------------

    #[test]
    fn rfc3339_known_values() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_735_689_600), "2025-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_800_000_000), "2027-01-15T08:00:00Z");
    }

    // -- Golden vectors: pkgs/fleet/hosted-worker/test/stamp-sig.test.ts
    // The native implementation must reproduce these byte-for-byte;
    // the hosted tests carry the same fixtures, so the two editions
    // sign and verify each other's stamps.

    const HOSTED_KID: &str = "hosted-2026-11";
    const HOSTED_PKCS8: &str = "MC4CAQAwBQYDK2VwBCIEIAABAgMEBQYHCAkKCwwNDg8QERITFBUWFxgZGhscHR4f";
    const HOSTED_PUB: &str = "A6EHv_POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg";
    const HOSTED_CANONICAL: &str = "{\"auth_subject\":\"9f2c1a44b7e54d0f8c3a5b6d7e8f9012\",\"created_at\":\"2026-11-22T14:31:09Z\",\"host\":\"fleet-hosted\",\"lane\":\"code\",\"lane_start_issues\":[12584,12586],\"model\":\"claude-opus-4-5\",\"picked_up_at\":\"2026-11-22T14:31:02Z\",\"queue_id\":21092,\"session_id\":\"ses_0123456789abcdef0123456789abcdef\",\"stamped_by\":\"fleet-hosted@0.1.0\",\"work_started_at\":\"2026-11-22T14:31:05Z\",\"workspace_ref\":{\"branch\":null,\"id\":\"fleet-hosted\",\"kind\":\"worker\",\"repo\":\"developing-today/code\",\"sha\":null},\"worktree\":null}";
    const HOSTED_STAMP_SIG: &str = "v1.ed25519.hosted-2026-11.NiP0TffLEBFw6qLqVfQlRajtgwKghveQNSoqcIpxMuo8sYYVSFw1Qi1-zBhZIG1usHSz5tL1-oO64fBS84EeAA";

    const NATIVE_KID: &str = "native-2026-11";
    const NATIVE_PKCS8: &str = "MC4CAQAwBQYDK2VwBCIEIKChoqOkpaanqKmhChGhKhOhShWhahehihmiCiGiKiOi";
    const NATIVE_PUB: &str = "C0rwl7rmIyCh6BbtAC971q4Cep4qVSKBtRZpZOCR1Ss";
    const NATIVE_CANONICAL: &str = "{\"auth_subject\":\"9f2c1a44b7e54d0f8c3a5b6d7e8f9012\",\"created_at\":\"2026-11-22T15:02:14Z\",\"host\":\"amd-6\",\"lane\":\"code\",\"lane_start_issues\":[],\"model\":\"claude-opus-4-5\",\"picked_up_at\":null,\"queue_id\":null,\"session_id\":\"ses_fedcba9876543210fedcba9876543210\",\"stamped_by\":\"fleet 0.9.1 (native)\",\"work_started_at\":\"2026-11-22T15:02:11Z\",\"worktree\":\"/home/user/code-12586\"}";
    const NATIVE_STAMP_SIG: &str = "v1.ed25519.native-2026-11.TNCmecEcLsL9SgyxUPg8cJ_affdmD3RYYu3Ukuyqzb107E9mbm_EfIwHjW_FJBVtxjWrGCYfbmP64OOmpjGaDQ";

    fn hosted_provenance() -> Value {
        json!({
            "queue_id": 21092,
            "session_id": "ses_0123456789abcdef0123456789abcdef",
            "lane": "code",
            "host": "fleet-hosted",
            "worktree": null,
            "workspace_ref": {
                "kind": "worker",
                "id": "fleet-hosted",
                "repo": "developing-today/code",
                "branch": null,
                "sha": null
            },
            "picked_up_at": "2026-11-22T14:31:02Z",
            "work_started_at": "2026-11-22T14:31:05Z",
            "created_at": "2026-11-22T14:31:09Z",
            "model": "claude-opus-4-5",
            "lane_start_issues": [12584, 12586],
            "auth_subject": "9f2c1a44b7e54d0f8c3a5b6d7e8f9012",
            "stamped_by": "fleet-hosted@0.1.0"
        })
    }

    fn native_provenance() -> Value {
        json!({
            "queue_id": null,
            "session_id": "ses_fedcba9876543210fedcba9876543210",
            "lane": "code",
            "host": "amd-6",
            "worktree": "/home/user/code-12586",
            "picked_up_at": null,
            "work_started_at": "2026-11-22T15:02:11Z",
            "created_at": "2026-11-22T15:02:14Z",
            "model": "claude-opus-4-5",
            "lane_start_issues": [],
            "auth_subject": "9f2c1a44b7e54d0f8c3a5b6d7e8f9012",
            "stamped_by": "fleet 0.9.1 (native)"
        })
    }

    fn verify_keys(kid: &str, pub_b64: &str) -> HashMap<String, [u8; 32]> {
        let raw = b64url_decode(pub_b64).unwrap();
        HashMap::from([(
            kid.to_string(),
            <[u8; 32]>::try_from(raw.as_slice()).unwrap(),
        )])
    }

    #[test]
    fn golden_hosted_vector_byte_exact() {
        let signer = signer_from_pkcs8_base64(HOSTED_KID, HOSTED_PKCS8).unwrap();
        assert_eq!(
            b64url_encode(&signer.signing_key.verifying_key().to_bytes()),
            HOSTED_PUB
        );
        let provenance = hosted_provenance();
        assert_eq!(
            canonical_provenance_bytes(&provenance).unwrap(),
            HOSTED_CANONICAL.as_bytes()
        );
        let signed = sign_stamp(&provenance, &signer).unwrap();
        assert_eq!(signed["stamp_sig"], json!(HOSTED_STAMP_SIG));
        // The native verifier accepts the hosted stamp.
        assert_eq!(
            verify_stamp(&signed, &verify_keys(HOSTED_KID, HOSTED_PUB)).unwrap(),
            StampVerification::Verified {
                key_id: HOSTED_KID.to_string()
            }
        );
    }

    #[test]
    fn golden_native_vector_byte_exact() {
        let signer = signer_from_pkcs8_base64(NATIVE_KID, NATIVE_PKCS8).unwrap();
        assert_eq!(
            b64url_encode(&signer.signing_key.verifying_key().to_bytes()),
            NATIVE_PUB
        );
        let provenance = native_provenance();
        assert_eq!(
            canonical_provenance_bytes(&provenance).unwrap(),
            NATIVE_CANONICAL.as_bytes()
        );
        let signed = sign_stamp(&provenance, &signer).unwrap();
        assert_eq!(signed["stamp_sig"], json!(NATIVE_STAMP_SIG));
        assert_eq!(
            verify_stamp(&signed, &verify_keys(NATIVE_KID, NATIVE_PUB)).unwrap(),
            StampVerification::Verified {
                key_id: NATIVE_KID.to_string()
            }
        );
    }

    #[test]
    fn verify_stamp_outcomes() {
        let signer = signer_from_pkcs8_base64(HOSTED_KID, HOSTED_PKCS8).unwrap();
        let keys = verify_keys(HOSTED_KID, HOSTED_PUB);
        let signed = sign_stamp(&hosted_provenance(), &signer).unwrap();

        // An edited field diverges.
        let mut tampered = signed.clone();
        tampered["lane"] = json!("other-lane");
        assert_eq!(
            verify_stamp(&tampered, &keys).unwrap(),
            StampVerification::Diverged {
                reason: "signature_mismatch"
            }
        );
        // Absent and null stamps are unsigned, not forgeries.
        assert_eq!(
            verify_stamp(&hosted_provenance(), &keys).unwrap(),
            StampVerification::Unsigned
        );
        let mut null_stamp = hosted_provenance();
        null_stamp["stamp_sig"] = Value::Null;
        assert_eq!(
            verify_stamp(&null_stamp, &keys).unwrap(),
            StampVerification::Unsigned
        );
        // Unknown key id.
        assert_eq!(
            verify_stamp(&signed, &verify_keys(NATIVE_KID, NATIVE_PUB)).unwrap(),
            StampVerification::Diverged {
                reason: "unknown_key"
            }
        );
        // Malformed signature payloads.
        for bad in [
            "v1.ed25519.hosted-2026-11.!!!",
            "v1.ed25519.hosted-2026-11.AAAA",
        ] {
            let mut forged = hosted_provenance();
            forged["stamp_sig"] = json!(bad);
            assert_eq!(
                verify_stamp(&forged, &keys).unwrap(),
                StampVerification::Diverged {
                    reason: "malformed_stamp_sig"
                },
                "{bad}"
            );
        }
        // Wrong version.
        let mut v2 = hosted_provenance();
        v2["stamp_sig"] = json!(
            "v2.ed25519.hosted-2026-11.NiP0TffLEBFw6qLqVfQlRajtgwKghveQNSoqcIpxMuo8sYYVSFw1Qi1-zBhZIG1usHSz5tL1-oO64fBS84EeAA"
        );
        assert_eq!(
            verify_stamp(&v2, &keys).unwrap(),
            StampVerification::Diverged {
                reason: "unsupported_stamp_version"
            }
        );
        // A stamp with no keys configured is unverifiable, never verified.
        assert_eq!(
            verify_stamp(&signed, &HashMap::new()).unwrap(),
            StampVerification::Unverifiable {
                reason: "no_verify_keys_configured"
            }
        );
    }

    #[test]
    fn signer_rejects_bad_material() {
        assert!(signer_from_pkcs8_base64("bad key id!", HOSTED_PKCS8).is_err());
        assert!(signer_from_pkcs8_base64(HOSTED_KID, "not-base64!!!").is_err());
        // A raw seed is not a PKCS#8 key.
        assert!(signer_from_pkcs8_base64(HOSTED_KID, &b64url_encode(&[7u8; 32])).is_err());
    }

    // -- The block: parse / assemble / render ------------------------

    const BLOCK_BODY: &str = "Intro text.\n\n<!-- pm-metadata:start -->\n```json\n{\n  \"pm_meta\": 1,\n  \"direct_parent\": {\"repo\": \"developing-today/code\", \"number\": 9016, \"kind\": \"issue\"},\n  \"links\": [],\n  \"lineage\": {\"design\": \"doc/x\", \"roadmap_item\": null, \"priority\": null},\n  \"provenance\": {\"queue_id\": \"caller-forgery\"},\n  \"custom\": \"kept\"\n}\n```\n<!-- pm-metadata:end -->\n\nTrailing notes.\n";

    #[test]
    fn block_parse_accepts_and_preserves_surroundings() {
        let parsed = parse_metadata_block(BLOCK_BODY).unwrap();
        let block = parsed.block.expect("block present");
        assert_eq!(block["pm_meta"], json!(1));
        assert_eq!(block["custom"], json!("kept"));
        assert_eq!(parsed.before, "Intro text.\n\n");
        assert_eq!(parsed.after, "\n\nTrailing notes.\n");
    }

    #[test]
    fn block_parse_no_block_is_none() {
        let parsed = parse_metadata_block("plain body, no markers").unwrap();
        assert!(parsed.block.is_none());
        assert_eq!(parsed.before, "plain body, no markers");
    }

    #[test]
    fn block_parse_refusals() {
        for (body, needle) in [
            (
                "text <!-- pm-metadata:end -->",
                "end marker without a start",
            ),
            (
                "<!-- pm-metadata:start -->\n```json\n{}\n```\n",
                "start marker without an end",
            ),
            (
                "<!-- pm-metadata:start -->\n{\"pm_meta\": 1}\n<!-- pm-metadata:end -->",
                "fenced JSON payload",
            ),
            (
                "<!-- pm-metadata:start -->\n```json\n{nope}\n```\n<!-- pm-metadata:end -->",
                "not valid JSON",
            ),
            (
                "<!-- pm-metadata:start -->\n```json\n[1, 2]\n```\n<!-- pm-metadata:end -->",
                "must be a JSON object",
            ),
            (
                "<!-- pm-metadata:start -->\n```json\n{\"pm_meta\": 2}\n```\n<!-- pm-metadata:end -->",
                "pm_meta: 1",
            ),
        ] {
            let err = parse_metadata_block(body).unwrap_err();
            assert!(err.contains(needle), "{err} should name {needle}");
            assert!(err.contains("invalid pm-metadata block"), "{err}");
        }
    }

    #[test]
    fn assemble_preserves_caller_fields_and_replaces_provenance() {
        let parsed = parse_metadata_block(BLOCK_BODY).unwrap();
        let provenance = json!({"queue_id": "20273", "created_at": "2026-11-23T00:00:00Z"});
        let block = assemble_stamped_block(parsed.block.as_ref(), provenance.clone());
        // Caller-owned fields survive exactly.
        assert_eq!(
            block["direct_parent"],
            json!({"repo": "developing-today/code", "number": 9016, "kind": "issue"})
        );
        assert_eq!(block["lineage"]["design"], json!("doc/x"));
        assert_eq!(block["custom"], json!("kept"));
        // Provenance is replaced wholesale — the caller's forgery is gone.
        assert_eq!(block["provenance"], provenance);
        assert_eq!(block["pm_meta"], json!(1));
    }

    #[test]
    fn assemble_from_no_block_injects_defaults() {
        let block = assemble_stamped_block(None, json!({"queue_id": null}));
        assert_eq!(block["direct_parent"], Value::Null);
        assert_eq!(block["links"], json!([]));
        assert_eq!(
            block["lineage"],
            json!({"design": null, "roadmap_item": null, "priority": null})
        );
        assert_eq!(block["pm_meta"], json!(1));
    }

    #[test]
    fn render_splices_in_place_and_appends() {
        let parsed = parse_metadata_block(BLOCK_BODY).unwrap();
        let block = assemble_stamped_block(parsed.block.as_ref(), json!({"queue_id": "1"}));
        let out = render_body_with_block(&parsed, &block);
        assert!(out.starts_with("Intro text.\n\n<!-- pm-metadata:start -->"));
        assert!(out.ends_with("<!-- pm-metadata:end -->\n\nTrailing notes.\n"));
        // The spliced body re-parses to the same block.
        let reparsed = parse_metadata_block(&out).unwrap();
        assert_eq!(reparsed.block.unwrap(), block);

        let plain = parse_metadata_block("Just text.").unwrap();
        let appended = render_body_with_block(&plain, &block);
        assert!(appended.starts_with("Just text.\n\n<!-- pm-metadata:start -->"));
        assert!(appended.ends_with("<!-- pm-metadata:end -->\n"));
    }

    // -- Create recognition ------------------------------------------

    #[test]
    fn intent_inline_body_forms() {
        // Separate-token --body.
        let intent = parse_create(
            &argv(&["issue", "create", "--title", "T", "--body", "hello"]),
            false,
        )
        .unwrap()
        .expect("stampable");
        assert_eq!(intent.kind, "issue");
        assert_eq!(intent.caller_body().unwrap(), "hello");
        let applied = intent.apply("STAMPED".to_string());
        assert_eq!(
            applied,
            argv(&["issue", "create", "--title", "T", "--body", "STAMPED"])
        );
        // --body=<v> form.
        let intent = parse_create(&argv(&["pr", "create", "--body=world"]), false)
            .unwrap()
            .expect("stampable");
        assert_eq!(intent.kind, "pr");
        assert_eq!(intent.caller_body().unwrap(), "world");
        // Attached -b<v> form.
        let intent = parse_create(&argv(&["issue", "create", "-battached"]), false)
            .unwrap()
            .expect("stampable");
        assert_eq!(intent.caller_body().unwrap(), "attached");
        // Last --body wins (gh's own rule).
        let intent = parse_create(
            &argv(&["issue", "create", "--body", "first", "--body", "second"]),
            false,
        )
        .unwrap()
        .expect("stampable");
        assert_eq!(intent.caller_body().unwrap(), "second");
        let applied = intent.apply("S".to_string());
        let bodies: Vec<&String> = applied
            .windows(2)
            .filter(|w| w[0] == "--body")
            .map(|w| &w[1])
            .collect();
        assert_eq!(bodies, vec!["first", "S"]);
    }

    #[test]
    fn intent_flag_values_are_not_mistaken_for_flags() {
        // A title whose text is "--body" is a value, not the flag.
        let intent = parse_create(
            &argv(&["issue", "create", "--title", "--body", "--body", "real"]),
            false,
        )
        .unwrap()
        .expect("stampable");
        assert_eq!(intent.caller_body().unwrap(), "real");
        // --repo is captured for the workspace facts.
        let intent = parse_create(
            &argv(&["issue", "create", "-R", "o/r", "--body", "x"]),
            false,
        )
        .unwrap()
        .expect("stampable");
        assert_eq!(intent.repo.as_deref(), Some("o/r"));
    }

    #[test]
    fn intent_body_file_is_read_and_replaced_not_mutated() {
        let dir = temp_dir("bodyfile");
        let path = dir.join("body.md");
        std::fs::write(&path, "from file").unwrap();
        let intent = parse_create(
            &argv(&[
                "issue",
                "create",
                "--title",
                "T",
                "--body-file",
                path.to_str().unwrap(),
            ]),
            false,
        )
        .unwrap()
        .expect("stampable");
        assert_eq!(intent.caller_body().unwrap(), "from file");
        let applied = intent.apply("STAMPED".to_string());
        assert!(!applied.iter().any(|a| a == "--body-file"));
        assert_eq!(applied[applied.len() - 2], "--body");
        assert_eq!(applied[applied.len() - 1], "STAMPED");
        // The caller's file is untouched.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "from file");
        // An unreadable body file refuses with a named error.
        let intent = parse_create(
            &argv(&["issue", "create", "--body-file", "/nonexistent/nope.md"]),
            false,
        )
        .unwrap()
        .expect("recognized");
        assert!(intent.caller_body().unwrap_err().contains("cannot read"));
    }

    #[test]
    fn intent_passthrough_cases() {
        for (args, tty) in [
            (argv(&["issue", "list"]), false),
            (argv(&["issue", "create", "--web", "--title", "T"]), false),
            (argv(&["issue", "create", "--fill"]), false),
            (argv(&["issue", "create", "--fill-first"]), false),
            (argv(&["issue", "create", "--template", "bug.md"]), false),
            (argv(&["issue", "create", "--body-file", "-"]), false),
            (argv(&["issue", "create", "--title", "T"]), true),
            (argv(&["issue", "create", "--help"]), false),
            (
                argv(&["issue", "create", "--body", "x", "--body-file", "f.md"]),
                false,
            ),
        ] {
            assert!(
                parse_create(&args, tty).unwrap().is_none(),
                "{args:?} (tty={tty}) must pass through"
            );
        }
        // No body and a non-terminal stdin: the editor/prompt flow is
        // not in play, so a block-only body is injected.
        let intent = parse_create(&argv(&["issue", "create", "--title", "T"]), false)
            .unwrap()
            .expect("stampable");
        assert_eq!(intent.caller_body().unwrap(), "");
        let applied = intent.apply("BLOCK".to_string());
        assert_eq!(applied[applied.len() - 2], "--body");
        assert_eq!(applied[applied.len() - 1], "BLOCK");
    }

    // -- Queue record mapping -----------------------------------------

    fn queue_row(id: i64, status: &str, session: &str, claimed_at: &str, issue: i64) -> Value {
        json!({
            "id": id,
            "status": status,
            "claim_session_id": session,
            "claimed_at": claimed_at,
            "not_before": null,
            "lane_id": "code",
            "claim_model": "claude-opus-4-5",
            "progress": {"work_started_at": "2026-11-22T10:00:00Z"},
            "dedupe_key": format!("repo-health:v1:fix:developing-today/code:issue:{issue}"),
            "payload": {"repo": "developing-today/code", "issue": {"number": issue}}
        })
    }

    #[test]
    fn queue_record_maps_the_sessions_tasks() {
        let rows = vec![
            queue_row(101, "claimed", "ses_abc", "2026-11-22T09:00:00Z", 9073),
            queue_row(102, "parked", "ses_abc", "2026-11-22T11:00:00Z", 9074),
            queue_row(103, "claimed", "ses_other", "2026-11-22T12:00:00Z", 1),
            queue_row(104, "completed", "ses_abc", "2026-11-22T13:00:00Z", 2),
            queue_row(105, "claimed", "", "2026-11-22T14:00:00Z", 3),
        ];
        let record = queue_record_from_rows(&rows, "ses_abc").expect("record");
        // Primary = the most recently claimed of the session's tasks.
        assert_eq!(record.queue_id.as_deref(), Some("102"));
        assert_eq!(record.picked_up_at.as_deref(), Some("2026-11-22T11:00:00Z"));
        assert_eq!(record.lane.as_deref(), Some("code"));
        assert_eq!(record.model.as_deref(), Some("claude-opus-4-5"));
        assert_eq!(
            record.work_started_at.as_deref(),
            Some("2026-11-22T10:00:00Z")
        );
        // Every issue the lane start holds, sorted and deduped.
        assert_eq!(record.lane_start_issues, vec![9073, 9074]);
        // No session binding, no record.
        assert!(queue_record_from_rows(&rows, "ses_absent").is_none());
        assert!(queue_record_from_rows(&[], "ses_abc").is_none());
    }

    // -- Provenance assembly ------------------------------------------

    #[test]
    fn provenance_stamps_the_full_set() {
        let facts = StampFacts {
            now_unix: 1_735_689_600,
            session_id: Some("ses_abc".to_string()),
            host: Some("amd-6".to_string()),
            worktree: Some("/home/user/wt".to_string()),
            workspace_ref: None,
            auth_subject: Some("felix".to_string()),
            queue: Some(QueueRecord {
                queue_id: Some("20273".to_string()),
                picked_up_at: Some("2026-11-22T09:00:00Z".to_string()),
                lane: Some("code".to_string()),
                model: Some("claude-opus-4-5".to_string()),
                lane_start_issues: vec![9073],
                ..QueueRecord::default()
            }),
        };
        let prov = assemble_provenance(&facts);
        assert_eq!(prov["queue_id"], json!("20273"));
        assert_eq!(prov["created_at"], json!("2025-01-01T00:00:00Z"));
        assert_eq!(prov["session_id"], json!("ses_abc"));
        assert_eq!(prov["lane"], json!("code"));
        assert_eq!(prov["lane_start_issues"], json!([9073]));
        assert_eq!(prov["scheduled_start"], Value::Null);
        assert_eq!(prov["workspace_ref"], Value::Null);
        assert_eq!(prov["auth_subject"], json!("felix"));
        assert_eq!(
            prov["stamped_by"],
            json!(format!("fleet {} (native)", env!("CARGO_PKG_VERSION")))
        );
        // The assembled set canonicalizes (it is signable).
        assert!(canonical_provenance_bytes(&prov).is_ok());
    }

    // -- Extraction (#9074 conformance) --------------------------------

    #[test]
    fn extract_indexes_strings_and_extras_the_rest() {
        let block = json!({
            "pm_meta": 1,
            "direct_parent": {"repo": "o/r", "number": 9016, "kind": "issue"},
            "links": [
                {"target": {"repo": "o/r", "number": 5}, "relation": "blocks", "status": "open", "since": null},
                {"target": {"repo": "o/r"}, "relation": "relates"}
            ],
            "lineage": {"design": null, "roadmap_item": null, "priority": null},
            "provenance": {
                "queue_id": "20273",
                "session_id": "ses_abc",
                "lane": "code",
                "host": "amd-6",
                "scheduled_start": "2026-11-22T08:00:00Z",
                "stamp_sig": "v1.ed25519.k.sig"
            },
            "custom": {"x": 1}
        });
        let ex = extract(&block);
        assert_eq!(ex.direct_parent_repo.as_deref(), Some("o/r"));
        assert_eq!(ex.direct_parent_number, Some(9016));
        assert_eq!(ex.queue_id.as_deref(), Some("20273"));
        assert_eq!(ex.session_id.as_deref(), Some("ses_abc"));
        // Unknown keys ride in extras, never dropped.
        assert_eq!(ex.extras["custom"], json!({"x": 1}));
        assert_eq!(
            ex.extras["provenance.scheduled_start"],
            json!("2026-11-22T08:00:00Z")
        );
        assert_eq!(ex.extras["provenance.stamp_sig"], json!("v1.ed25519.k.sig"));
        // Link 0 is well-shaped and indexes in full.
        assert_eq!(ex.link_rows[0].0.target_number, Some(5));
        assert_eq!(ex.links[0]["targetRepo"], json!("o/r"));
        assert!(!ex.extras.contains_key("links.0"));
        // Link 1 is partial: its shaped fields still index, and the
        // raw link is preserved in extras.
        assert_eq!(ex.link_rows[1].0.target_repo.as_deref(), Some("o/r"));
        assert_eq!(ex.link_rows[1].0.target_number, None);
        assert_eq!(ex.extras["links.1"]["relation"], json!("relates"));
    }

    #[test]
    fn extract_numeric_queue_id_and_bad_parent_go_to_extras() {
        let block = json!({
            "pm_meta": 1,
            "direct_parent": {"repo": "o/r"},
            "provenance": {"queue_id": 21092, "session_id": 42}
        });
        let ex = extract(&block);
        assert_eq!(ex.queue_id, None);
        assert_eq!(ex.extras["provenance.queue_id"], json!(21092));
        assert_eq!(ex.extras["provenance.session_id"], json!(42));
        assert_eq!(ex.direct_parent_repo, None);
        assert_eq!(ex.extras["direct_parent"], json!({"repo": "o/r"}));
    }

    // -- The store ------------------------------------------------------

    fn sample_record(number: i64, links: Value) -> MetadataRecord {
        MetadataRecord {
            repo: "developing-today/code".to_string(),
            kind: "issue".to_string(),
            number,
            url: format!("https://github.com/developing-today/code/issues/{number}"),
            metadata: json!({
                "pm_meta": 1,
                "direct_parent": null,
                "links": links,
                "lineage": {"design": null, "roadmap_item": null, "priority": null},
                "provenance": {
                    "queue_id": "20273",
                    "session_id": "ses_abc",
                    "lane": "code",
                    "host": "amd-6",
                    "created_at": "2026-11-23T00:00:00Z"
                }
            }),
            source_updated_at: Some("2026-11-23T00:00:00Z".to_string()),
        }
    }

    #[test]
    fn store_round_trips_the_full_record() {
        let dir = temp_dir("store");
        let store = SqliteMetadataStore::new(dir.join("metadata.db"));
        let record = sample_record(
            12999,
            json!([{"target": {"repo": "o/r", "number": 5}, "relation": "blocks", "status": "open"}]),
        );
        store.insert(&record).unwrap();
        let id = artifact_id(&record.repo, &record.kind, record.number);
        assert_eq!(id, "gh:developing-today/code#issue:12999");
        let row = store.get(&id).unwrap().expect("row present");
        assert_eq!(row.metadata, record.metadata);
        assert_eq!(row.repo, record.repo);
        assert_eq!(row.number, 12999);
        assert_eq!(row.schema_version, 1);
        assert_eq!(row.extracted.queue_id.as_deref(), Some("20273"));
        assert_eq!(row.extracted.session_id.as_deref(), Some("ses_abc"));
        assert_eq!(row.extracted.link_rows.len(), 1);
        assert_eq!(row.extracted.link_rows[0].0.target_number, Some(5));
        assert_eq!(
            row.source_updated_at.as_deref(),
            Some("2026-11-23T00:00:00Z")
        );
        // A replay (the ingestion heal) replaces the row and its links.
        let replay = sample_record(12999, json!([]));
        store.insert(&replay).unwrap();
        let row = store.get(&id).unwrap().expect("row present");
        assert_eq!(row.extracted.link_rows.len(), 0);
        // Unknown artifact reads back as absent, not an error.
        assert!(store.get("gh:o/r#issue:1").unwrap().is_none());
    }

    #[test]
    fn store_refuses_a_shapeless_record() {
        let dir = temp_dir("store-bad");
        let store = SqliteMetadataStore::new(dir.join("metadata.db"));
        let mut record = sample_record(1, json!([]));
        record.number = 0;
        assert!(store.insert(&record).is_err());
        record.number = 1;
        record.repo = String::new();
        assert!(store.insert(&record).is_err());
    }

    // -- Finish: URL readback + insert outcomes -------------------------

    struct FakeStore {
        fail: bool,
    }

    impl MetadataStore for FakeStore {
        fn insert(&self, _record: &MetadataRecord) -> Result<(), String> {
            if self.fail {
                Err("forced insert failure".to_string())
            } else {
                Ok(())
            }
        }
        fn get(&self, _artifact_id: &str) -> Result<Option<StoredRow>, String> {
            Ok(None)
        }
    }

    #[test]
    fn parse_created_url_forms() {
        let item = parse_created_url("https://github.com/o/r/issues/42\n").unwrap();
        assert_eq!(item.repo, "o/r");
        assert_eq!(item.kind, "issue");
        assert_eq!(item.number, 42);
        let item = parse_created_url("noise\nhttps://github.com/o/r/pull/7\n").unwrap();
        assert_eq!(item.kind, "pr");
        assert!(parse_created_url("https://github.com/o/r/issues/abc").is_none());
        assert!(parse_created_url("https://example.com/o/r/issues/1").is_none());
        assert!(parse_created_url("").is_none());
    }

    #[test]
    fn finish_create_outcomes() {
        let block = json!({"pm_meta": 1, "provenance": {"created_at": "2026-11-23T00:00:00Z"}});
        let stdout = "https://github.com/o/r/issues/42\n";
        match finish_create(&FakeStore { fail: false }, stdout, &block) {
            CreateOutcome::Inserted(item) => assert_eq!(item.number, 42),
            _ => panic!("expected Inserted"),
        }
        // A forced insert failure is reported, never fatal — the
        // GitHub item stands (acceptance case 3's native half).
        match finish_create(&FakeStore { fail: true }, stdout, &block) {
            CreateOutcome::InsertFailed { item, error } => {
                assert_eq!(item.number, 42);
                assert!(error.contains("forced"));
            }
            _ => panic!("expected InsertFailed"),
        }
        match finish_create(&FakeStore { fail: false }, "no url here", &block) {
            CreateOutcome::NoUrl => {}
            _ => panic!("expected NoUrl"),
        }
    }

    // -- #9095 version conformance ------------------------------------
    //
    // Oracle: the hand-authored fixture set. The canonical copy lives
    // under the schema v1 design folder at
    // doc/2026-11-21T17-56-24Z_design_atlas_metadata_schema_v1/fixtures/
    // pm-metadata-v1.json; the executable copy this test reads is
    // vendored at tests/fixtures/pm-metadata-v1.json because the
    // fleet nix source closure (pkgs/fleet/default.nix fleetSrc,
    // rooted at pkgs/) does not contain repo-root doc/ — a
    // CARGO_MANIFEST_DIR-anchored walk to the repo root breaks under
    // the nix sandbox (code#13617). Where the design copy is present
    // (normal checkouts) the test asserts both copies are byte-equal
    // so the vendored copy cannot drift from the oracle; under the
    // sandbox only the vendored copy is enforced, never skipped.
    // Each `ok` case must parse and preserve its declared keys
    // through parse and through the stamp assembly round-trip; each
    // `reject` case must fail closed. Doctrine-projection
    // expectations in the fixture are the oracle for design §7 slice
    // 2 and are not asserted here.
    #[test]
    fn pm_metadata_version_conformance_fixtures() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/pm-metadata-v1.json"
        );
        let raw = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("conformance fixture set unreadable at {path}: {e}"));
        // Drift guard: the design-folder copy is the canonical
        // oracle. It is absent from the nix source closure, so only
        // compare when it exists; the conformance assertions below
        // always run against the vendored copy.
        let design_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../../doc/2026-11-21T17-56-24Z_design_atlas_metadata_schema_v1/fixtures/pm-metadata-v1.json"
        );
        if std::path::Path::new(design_path).exists() {
            let design_raw = std::fs::read_to_string(design_path)
                .unwrap_or_else(|e| panic!("design oracle unreadable at {design_path}: {e}"));
            assert_eq!(
                raw, design_raw,
                "vendored fixture diverged from the design oracle; sync tests/fixtures/pm-metadata-v1.json from {design_path}"
            );
        }
        let set: Value = serde_json::from_str(&raw).expect("fixture set is valid JSON");
        let cases = set["cases"].as_array().expect("fixture set carries cases");
        assert!(!cases.is_empty(), "fixture set must not be empty");
        for case in cases {
            let id = case["id"].as_str().expect("case id");
            let body = case["body"].as_str().expect("case body");
            let parsed = parse_metadata_block(body);
            match case["reader_parse"].as_str().expect("reader_parse") {
                "reject" => {
                    assert!(parsed.is_err(), "{id}: expected fail-closed rejection");
                }
                "ok" => {
                    let parsed = parsed
                        .unwrap_or_else(|e| panic!("{id}: expected parse success, got: {e}"));
                    if case["block"].as_str() == Some("absent") {
                        assert!(parsed.block.is_none(), "{id}: expected no block");
                        continue;
                    }
                    let block = parsed.block.unwrap_or_else(|| panic!("{id}: expected a block"));
                    let keys = case["preserved_keys"]
                        .as_array()
                        .expect("preserved_keys")
                        .iter()
                        .map(|k| k.as_str().expect("key"))
                        .collect::<Vec<_>>();
                    for key in &keys {
                        assert!(
                            block.get(key).is_some(),
                            "{id}: key {key} lost at parse"
                        );
                    }
                    // Stamp round-trip: re-stamping with the block's own
                    // provenance must preserve every preserved key's
                    // value exactly.
                    let provenance = block
                        .get("provenance")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    let assembled = assemble_stamped_block(Some(&block), provenance);
                    for key in &keys {
                        // pm_meta is the one key assembly rewrites on
                        // purpose: it pins the integer current version
                        // (a float 1.0 input normalizes to 1).
                        if *key == "pm_meta" {
                            continue;
                        }
                        assert_eq!(
                            assembled.get(key),
                            block.get(key),
                            "{id}: key {key} changed by stamp assembly"
                        );
                    }
                    assert_eq!(
                        assembled.get("pm_meta"),
                        Some(&json!(1)),
                        "{id}: stamp assembly must pin pm_meta to integer 1"
                    );
                }
                other => panic!("{id}: unknown reader_parse expectation {other}"),
            }
        }
    }
}
