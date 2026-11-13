//! Packet validation — faithful port of repo-queue.py's built-in
//! (manual) validator: envelope checks, manual schema checks, and the
//! dedupe-key family rules. (Legacy Python prefers the jsonschema
//! library when importable; messages for exotic packets can differ in
//! wording, but accept/reject semantics match the manual contract and
//! the database RPCs remain the backstop.)
//!
//! THE VALIDATION CONTRACT (design #6448; this statement landed
//! with the Phase 4 closing inventory, #6449 — a future forwarded
//! field or flag is added against this stated rule, not ad hoc):
//!
//! One rule: the queue validates every value it forwards — its
//! type, its range, and its agreement with the packet's other
//! fields — at the boundary, before the task row exists; a value
//! that cannot be honoured is refused with an error naming the
//! field, never forwarded verbatim. Reject, never clamp, never
//! warn-and-forward: warnings keep their existing role (design
//! defaults, size targets) and never carry a validity verdict.
//!
//! Bounds, with the same values enforced at all three points —
//! validate_packet (packet fields), the client (usage error,
//! exit 2, naming the flag), and the broker (rejection before
//! the RPC/SQL call; the broker is the trust boundary for
//! non-CLI callers):
//!
//!   max_attempts (either copy)  integer 1..=100; the two copies must agree
//!   not_before (packet + park/revive flags)
//!                               RFC 3339 string, at most 366 days in the future
//!   priority                    one of 10, 20, 30, 40, 50, 70, 90
//!   list --limit                1..=500 (default 20)
//!   --lease-seconds             1..=86400 (default 2700)
//!   --extend-seconds            1..=86400 (default 900)
//!   --delay-seconds             1..=604800 (no default; an explicit 0 is rejected)
//!   claim --count               1..=5 (batch claim cap)
//!
//! Broker-internal dep cadence/TTL values (#8276) are the one
//! deliberate exception to reject-never-clamp: the broker clamps
//! them (recheck 300..=3600, TTL 60..=604800) because they tune
//! its own waits, not a producer's stated intent.
//!
//! Four rows diverge from design #6448's original table
//! (max_attempts ceiling, delay ceiling, the lease/extend floor,
//! and the creation-outbox retry_after ceiling): the values
//! above are the implemented, live contract, and reconciling
//! the two statements is tracked on #9029. The inventory that
//! dispositioned every forwarded field, client flag, and broker
//! verb parameter against this contract is recorded on #6449.

use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

pub const PRIORITY_SCALE: [i64; 7] = [10, 20, 30, 40, 50, 70, 90];
pub const KINDS: [&str; 5] = [
    "pr_shepherd",
    "triage",
    "small_fix",
    "ci_investigation",
    "second_pass",
];
// Both the old and the current names of the two repos that moved to the
// developing-today-forks org (2026-11-04) stay allowlisted: in-flight queue
// tasks carry dedupe keys with the old names and must keep validating,
// while new tasks use the developing-today-forks names. The same applies to dezren39/nix, renamed to dezren39/darwin-nix.
pub const FLEET_REPOS: [&str; 9] = [
    "developing-today/code",
    "developing-today/opencode",
    "developing-today/openchamber",
    "developing-today-forks/opencode",
    "developing-today-forks/openchamber",
    "dezren39/nix",
    "dezren39/darwin-nix",
    "developing-today/zettel",
    "dezren39/dezren39",
];
const PAYLOAD_WARN_BYTES: usize = 4096;
const PAYLOAD_MAX_BYTES: usize = 16384;
const BODY_WARN_WORDS: usize = 250;

pub const DEDUPE_PATTERNS_DOC: &str = "fix:         repo-health:v1:fix:<repo>:issue:<n>\n\
triage:      repo-health:v1:triage:<repo>:issue:<n>\n\
pr:          repo-health:v1:pr:<repo>:pr:<n>\n\
pr-set:      repo-health:v1:pr-set:<repo>:<n>-<n>[-...] (sorted)\n\
set:         repo-health:v1:set:<origin issue>:owed:<repo>\n\
maintenance: repo-health:v1:maintenance:<repo>:<kind>:<YYYY-Www>\n\
second-pass: repo-health:v1:second-pass:<repo>:<issue>:pass-<n>";

fn manual_schema_errors(p: &Value) -> Vec<String> {
    let mut errs = Vec::new();
    let obj = match p.as_object() {
        Some(o) => o,
        None => return vec!["payload: must be a JSON object".to_string()],
    };
    let get = |f: &str| obj.get(f);
    if get("schema") != Some(&json!("repo-task/v1")) {
        errs.push("payload.schema: must be 'repo-task/v1'".to_string());
    }
    let kind_ok = matches!(get("kind"), Some(Value::String(k)) if KINDS.contains(&k.as_str()));
    if !kind_ok {
        let mut sorted = KINDS;
        sorted.sort();
        errs.push(format!("payload.kind: must be one of {sorted:?}"));
    }
    let repo_ok =
        matches!(get("repo"), Some(Value::String(r)) if FLEET_REPOS.contains(&r.as_str()));
    if !repo_ok {
        errs.push(format!(
            "payload.repo: {} is not a fleet repo",
            python_repr(get("repo"))
        ));
    }
    for f in ["problem", "priority_reason"] {
        match get(f) {
            Some(Value::String(s)) if !s.is_empty() => {}
            Some(Value::String(_)) | None => {
                errs.push(format!("payload.{f}: required field missing or empty"))
            }
            // A non-string problem/priority_reason (123, {x:1}) used to pass
            // is_empty and be forwarded verbatim (#6430).
            Some(_) => errs.push(format!("payload.{f}: must be a non-empty string")),
        }
    }
    for f in ["evidence", "acceptance", "tests", "context_refs"] {
        match get(f) {
            Some(Value::Array(a)) => {
                // Element types were never checked: [123], [null], [{x:1}]
                // and [[t]] all validated and were forwarded verbatim (#6430).
                if a.iter().any(|e| !matches!(e, Value::String(_))) {
                    errs.push(format!("payload.{f}: every element must be a string"));
                }
            }
            _ => errs.push(format!("payload.{f}: must be a list")),
        }
    }
    for f in ["evidence", "acceptance", "tests"] {
        if let Some(Value::Array(a)) = get(f) {
            if a.is_empty() {
                errs.push(format!("payload.{f}: must not be empty"));
            }
        }
    }
    match get("scope") {
        Some(Value::Object(scope)) => {
            match scope.get("include") {
                Some(Value::Array(a)) => {
                    if a.iter().any(|e| !matches!(e, Value::String(_))) {
                        errs.push(
                            "payload.scope.include: every element must be a string".to_string(),
                        );
                    }
                }
                _ => errs.push("payload.scope.include: must be a list".to_string()),
            }
            match scope.get("exclude") {
                Some(Value::Array(a)) if !a.is_empty() => {
                    if a.iter().any(|e| !matches!(e, Value::String(_))) {
                        errs.push(
                            "payload.scope.exclude: every element must be a string".to_string(),
                        );
                    }
                }
                _ => errs.push("payload.scope.exclude: required (scope exclusions)".to_string()),
            }
        }
        _ => errs.push("payload.scope: must be an object with include/exclude".to_string()),
    }
    match get("risk") {
        Some(Value::Object(risk)) if matches!(risk.get("level"), Some(Value::String(l)) if ["low","medium","high"].contains(&l.as_str())) => {
            match risk.get("flags") {
                Some(Value::Array(a)) => {
                    if a.iter().any(|e| !matches!(e, Value::String(_))) {
                        errs.push("payload.risk.flags: every element must be a string".to_string());
                    }
                }
                _ => errs.push("payload.risk.flags: must be a list".to_string()),
            }
        }
        _ => errs.push("payload.risk.level: must be low|medium|high".to_string()),
    }
    for f in ["timebox_minutes"] {
        let ok =
            matches!(get(f), Some(Value::Number(n)) if n.as_i64().map(|v| v >= 1).unwrap_or(false));
        if !ok {
            errs.push(format!("payload.{f}: must be a positive integer"));
        }
    }
    if let Some(value) = get("max_attempts") {
        if !valid_max_attempts(value) {
            errs.push("payload.max_attempts: must be an integer between 1 and 100".to_string());
        }
    }
    let ded_ok = matches!(get("dedupe"), Some(Value::Object(d))
        if matches!(d.get("checked_at"), Some(Value::String(s)) if !s.is_empty())
            && matches!(d.get("basis"), Some(Value::String(s)) if !s.is_empty()));
    if !ded_ok {
        errs.push(
            "payload.dedupe: must have checked_at and basis as non-empty strings".to_string(),
        );
    }
    // Identity numbers are GitHub numbers: positive integers only. A
    // non-positive issue number names no issue, whatever the dedupe key
    // says (a maintenance-family key has no issue cross-check to catch
    // it), so it is an error in its own right (#5549).
    if let Some(issue) = get("issue").and_then(|i| i.as_object()) {
        if !matches!(issue.get("number"), Some(Value::Number(n)) if n.as_i64().is_some_and(|v| v >= 1))
        {
            errs.push("payload.issue.number: must be a positive integer".to_string());
        }
    }
    let has_issue = matches!(get("issue"), Some(Value::Object(i))
        if matches!(i.get("number"), Some(Value::Number(n)) if n.as_i64().is_some_and(|v| v >= 1)));
    // A prs list satisfies identity only when every element is a PR
    // reference: an object with a positive-integer `number`. Placeholder
    // elements ([null], [{"number": "5424"}], [{}]) used to pass on the
    // array being non-empty alone, and the dedupe cross-check's
    // filter_map then silently dropped them (#5549). Every real packet
    // in the fleet corpus (87/87 elements) already has this shape.
    let has_prs = matches!(get("prs"), Some(Value::Array(a)) if !a.is_empty()
        && a.iter().all(|p| matches!(p.get("number"), Some(Value::Number(n)) if n.as_i64().is_some_and(|v| v >= 1))));
    // An empty object names no set (#5549); real set objects carry
    // content (origin_issue, prs, ...).
    let has_set = matches!(get("set"), Some(Value::Object(o)) if !o.is_empty());
    let has_source = matches!(get("source"), Some(Value::Object(s))
        if matches!(s.get("pr"), Some(Value::Number(n)) if n.as_i64().is_some_and(|v| v >= 1))
            && matches!(s.get("finding"), Some(Value::String(f)) if !f.is_empty()));
    if !(has_issue || has_prs || has_set || has_source) {
        errs.push(
            "payload identity: need issue.number, a non-empty prs list, a set object, \
             or a pre-filing source {pr, finding}"
                .to_string(),
        );
    }
    errs.extend(review_errors(p));
    errs
}

/// The review payload block (design #2025 section 5.2; #2310): required
/// for kind second_pass, and shape-checked whenever a packet of any
/// kind carries one (the Stage B interim representation attaches it to
/// small_fix packets). gap_list and verdict_note_url are required, and
/// pass may not exceed pass_cap — the termination cap is enforced
/// here, in the enqueue code path, not by reviewer restraint.
fn review_errors(p: &Value) -> Vec<String> {
    let mut errs = Vec::new();
    let kind = p.get("kind").and_then(|k| k.as_str()).unwrap_or("");
    let review = p.get("review");
    if kind != "second_pass" && review.is_none() {
        return errs;
    }
    let Some(review) = review.and_then(|r| r.as_object()) else {
        errs.push(
            "payload.review: required object for kind second_pass (design #2025 section 5.2)"
                .to_string(),
        );
        return errs;
    };
    match review.get("verdict_note_url") {
        Some(Value::String(u)) if !u.is_empty() => {}
        _ => errs.push(
            "payload.review.verdict_note_url: required non-empty string for second_pass packets"
                .to_string(),
        ),
    }
    match review.get("gap_list") {
        Some(Value::Array(a))
            if !a.is_empty()
                && a.iter()
                    .all(|g| matches!(g, Value::String(s) if !s.is_empty())) => {}
        _ => errs.push(
            "payload.review.gap_list: required non-empty list of checkable gap strings for second_pass packets"
                .to_string(),
        ),
    }
    let pass = review.get("pass").and_then(|v| v.as_i64());
    let pass_cap = review.get("pass_cap").and_then(|v| v.as_i64());
    match pass {
        Some(n) if n >= 1 => {}
        _ => errs.push(
            "payload.review.pass: required positive integer (the pass being commissioned; the original work is pass 0)"
                .to_string(),
        ),
    }
    match pass_cap {
        Some(n) if n >= 1 => {}
        _ => errs.push("payload.review.pass_cap: required positive integer".to_string()),
    }
    if let (Some(pass), Some(cap)) = (pass, pass_cap) {
        if pass > cap {
            errs.push(format!(
                "payload.review.pass: pass {pass} exceeds pass_cap {cap} — the termination cap is enforced at enqueue (design #2025 section 5.1)"
            ));
        }
    }
    if let Some(verdict) = review.get("verdict") {
        if !matches!(verdict, Value::String(_)) {
            errs.push("payload.review.verdict: must be a string".to_string());
        }
    }
    if let Some(artifact) = review.get("artifact") {
        if !artifact.is_object() {
            errs.push("payload.review.artifact: must be an object {pr, issue, merged}".to_string());
        }
    }
    if let Some(producer) = review.get("producer") {
        if !producer.is_object() {
            errs.push("payload.review.producer: must be an object {task_id, session, pr}".to_string());
        }
    }
    if let Some(refs) = review.get("goal_context_refs") {
        if !matches!(refs, Value::Array(a) if a.iter().all(|r| matches!(r, Value::String(_)))) {
            errs.push("payload.review.goal_context_refs: must be a list of strings".to_string());
        }
    }
    if let Some(floor) = review.get("route_floor") {
        if !matches!(floor, Value::String(_)) {
            errs.push("payload.review.route_floor: must be a string".to_string());
        }
    }
    errs
}

/// Python-style repr for the one message that embeds a value.
fn python_repr(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "None".to_string(),
        Some(Value::String(s)) => format!("'{s}'"),
        Some(other) => other.to_string(),
    }
}

pub struct DedupeMatch {
    pub family: String,
    pub repo: String,
    pub num: Option<i64>,
    pub nums: Option<Vec<i64>>,
    pub origin: Option<i64>,
    /// The commissioned pass for the second-pass family (#2310);
    /// None for every other family.
    pub pass: Option<i64>,
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Parses a dedupe-key number in its canonical spelling only: plain
/// decimal digits, no leading zeros, and at least 1 (issue/PR numbers
/// start at 1). A non-canonical spelling (05424, 0) would be a
/// different dedupe_key string for the same number — a second dedupe
/// identity for the same work — so it matches nothing (#5510; the
/// pr-set/maintenance families got the same treatment in #5482).
fn canonical_number(s: &str) -> Option<i64> {
    if !all_digits(s) {
        return None;
    }
    let n: i64 = s.parse().ok()?;
    if n < 1 || n.to_string() != s {
        return None;
    }
    Some(n)
}

/// Hand-rolled equivalent of the DEDUPE_REGEXES family patterns.
fn match_dedupe(key: &str) -> Option<DedupeMatch> {
    let rest = key.strip_prefix("repo-health:v1:")?;
    // set family has a different shape: set:<num>:owed:<repo>
    if let Some(tail) = rest.strip_prefix("set:") {
        let (num_s, repo) = tail.split_once(":owed:")?;
        if let Some(origin) = canonical_number(num_s) {
            if FLEET_REPOS.contains(&repo) {
                return Some(DedupeMatch {
                    family: "set".into(),
                    repo: repo.to_string(),
                    num: None,
                    nums: None,
                    origin: Some(origin),
                    pass: None,
                });
            }
        }
        return None;
    }
    for fam in ["fix", "triage", "pr-set", "pr", "maintenance", "second-pass"] {
        let Some(after) = rest.strip_prefix(&format!("{fam}:")) else {
            continue;
        };
        for repo in FLEET_REPOS {
            let Some(tail) = after.strip_prefix(&format!("{repo}:")) else {
                continue;
            };
            match fam {
                "fix" | "triage" => {
                    if let Some(n) = tail.strip_prefix("issue:") {
                        if let Some(num) = canonical_number(n) {
                            return Some(DedupeMatch {
                                family: fam.into(),
                                repo: repo.to_string(),
                                num: Some(num),
                                nums: None,
                                origin: None,
                                pass: None,
                            });
                        }
                    }
                }
                "second-pass" => {
                    // repo-health:v1:second-pass:<repo>:<issue>:pass-<n>
                    // (design #2025 section 5.2): one live packet per
                    // (artifact, pass) by construction.
                    if let Some((issue_s, pass_s)) = tail.split_once(":pass-") {
                        if let (Some(num), Some(pass)) = (
                            canonical_number(issue_s),
                            canonical_number(pass_s),
                        ) {
                            return Some(DedupeMatch {
                                family: fam.into(),
                                repo: repo.to_string(),
                                num: Some(num),
                                nums: None,
                                origin: None,
                                pass: Some(pass),
                            });
                        }
                    }
                }
                "pr" => {
                    if let Some(n) = tail.strip_prefix("pr:") {
                        if let Some(num) = canonical_number(n) {
                            return Some(DedupeMatch {
                                family: fam.into(),
                                repo: repo.to_string(),
                                num: Some(num),
                                nums: None,
                                origin: None,
                                pass: None,
                            });
                        }
                    }
                }
                "pr-set" => {
                    // Canonical pr-set keys name at least two distinct PRs in
                    // strictly ascending order (DEDUPE_PATTERNS_DOC: <n>-<n>[-...]
                    // sorted). A single number is the `pr` family's key, and a
                    // repeated number is not a set — both spellings would give
                    // the same coverage a second valid dedupe key (#5482).
                    let parts: Vec<&str> = tail.split('-').collect();
                    let nums: Vec<i64> = parts.iter().filter_map(|p| canonical_number(p)).collect();
                    if parts.len() >= 2
                        && nums.len() == parts.len()
                        && nums.windows(2).all(|w| w[0] < w[1])
                    {
                        return Some(DedupeMatch {
                            family: fam.into(),
                            repo: repo.to_string(),
                            num: None,
                            nums: Some(nums),
                            origin: None,
                                pass: None,
                        });
                    }
                }
                "maintenance" => {
                    // <slug>:<YYYY-Www>; slug [a-z0-9][a-z0-9_-]*
                    if let Some((slug, week)) = tail.rsplit_once(':') {
                        let slug_ok = !slug.is_empty()
                            && slug
                                .bytes()
                                .next()
                                .map(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                                .unwrap_or(false)
                            && slug.bytes().all(|b| {
                                b.is_ascii_lowercase()
                                    || b.is_ascii_digit()
                                    || b == b'_'
                                    || b == b'-'
                            });
                        let wb = week.as_bytes();
                        // Shape YYYY-Www plus a real ISO week (01..=53):
                        // W00/W99 name no week, so a key spelled with one can
                        // never dedupe against a real week's key (#5482).
                        let week_ok = week.len() == 8
                            && all_digits(&week[..4])
                            && week.as_bytes()[4] == b'-'
                            && wb[5] == b'W'
                            && all_digits(&week[6..])
                            && week[6..]
                                .parse::<u32>()
                                .is_ok_and(|w| (1..=53).contains(&w));
                        if slug_ok && week_ok {
                            return Some(DedupeMatch {
                                family: fam.into(),
                                repo: repo.to_string(),
                                num: None,
                                nums: None,
                                origin: None,
                                pass: None,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        // Family prefix matched but no repo/tail did: Python would fall
        // through to trying other families too (regexes are tried in
        // order over the whole key). Keep scanning other families.
    }
    None
}

fn kind_families(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "small_fix" => Some(&["fix"]),
        "triage" => Some(&["triage", "maintenance"]),
        "pr_shepherd" => Some(&["pr", "pr-set", "set"]),
        "ci_investigation" => Some(&["fix", "triage", "pr"]),
        "second_pass" => Some(&["second-pass"]),
        _ => None,
    }
}

fn dedupe_errors(packet: &Value, payload: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    let key = match packet.get("dedupe_key") {
        Some(Value::String(k)) if !k.is_empty() => k.clone(),
        _ => {
            return vec![format!(
                "packet.dedupe_key: required deterministic key:\n{DEDUPE_PATTERNS_DOC}"
            )];
        }
    };
    let m = match match_dedupe(&key) {
        Some(m) => m,
        None => {
            return vec![format!(
                "packet.dedupe_key: '{key}' matches no design pattern:\n{DEDUPE_PATTERNS_DOC}"
            )];
        }
    };
    if let Some(repo) = payload.get("repo") {
        if repo != &json!(m.repo) {
            errors.push(format!(
                "packet.dedupe_key: repo '{}' in key does not match payload.repo {}",
                m.repo,
                python_repr(Some(repo))
            ));
        }
    }
    let kind = payload.get("kind").and_then(|k| k.as_str()).unwrap_or("");
    if let Some(allowed) = kind_families(kind) {
        if !allowed.contains(&m.family.as_str()) {
            let mut sorted = allowed.to_vec();
            sorted.sort();
            errors.push(format!(
                "packet.dedupe_key: family '{}' is not valid for kind '{}' (allowed: {:?})",
                m.family, kind, sorted
            ));
        }
    }
    let issue_no = payload
        .get("issue")
        .and_then(|i| i.get("number"))
        .and_then(|n| n.as_i64());
    let pr_numbers: Vec<i64> = payload
        .get("prs")
        .and_then(|p| p.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|p| p.get("number").and_then(|n| n.as_i64()))
                .collect()
        })
        .unwrap_or_default();
    // A pre-filing source identity (#6446): {pr, finding} naming the PR
    // the finding came from. It can stand in for the prs identity only
    // when it is the packet's ONLY identity (source-only).
    let source_pr = payload
        .get("source")
        .and_then(|s| s.get("pr"))
        .and_then(|n| n.as_i64());
    let has_set = payload
        .get("set")
        .and_then(|s| s.as_object())
        .is_some_and(|o| !o.is_empty());
    let source_only =
        source_pr.is_some() && issue_no.is_none() && pr_numbers.is_empty() && !has_set;
    match m.family.as_str() {
        "fix" | "triage" => {
            let n = m.num.unwrap_or(0);
            match issue_no {
                Some(ino) if n != ino => {
                    errors.push(format!(
                        "packet.dedupe_key: issue {n} in key does not match payload.issue.number {ino}"
                    ));
                }
                Some(_) => {}
                None => {
                    errors.push(format!(
                        "packet.dedupe_key: family '{}' requires payload.issue.number {n} matching the key issue, but the payload carries no issue.number (#6446)",
                        m.family
                    ));
                }
            }
        }
        "second-pass" => {
            let n = m.num.unwrap_or(0);
            match issue_no {
                Some(ino) if n != ino => {
                    errors.push(format!(
                        "packet.dedupe_key: issue {n} in key does not match payload.issue.number {ino}"
                    ));
                }
                Some(_) => {}
                None => {
                    errors.push(format!(
                        "packet.dedupe_key: family 'second-pass' requires payload.issue.number {n} matching the key issue, but the payload carries no issue.number (#6446)"
                    ));
                }
            }
            let key_pass = m.pass.unwrap_or(0);
            let review_pass = payload
                .get("review")
                .and_then(|r| r.get("pass"))
                .and_then(|v| v.as_i64());
            match review_pass {
                Some(rp) if rp != key_pass => {
                    errors.push(format!(
                        "packet.dedupe_key: pass {key_pass} in key does not match payload.review.pass {rp}"
                    ));
                }
                Some(_) => {}
                None => {
                    errors.push(format!(
                        "packet.dedupe_key: family 'second-pass' requires payload.review.pass {key_pass} matching the key pass, but the payload carries no review.pass"
                    ));
                }
            }
        }
        "pr" => {
            let n = m.num.unwrap_or(0);
            if !pr_numbers.is_empty() {
                if !pr_numbers.contains(&n) {
                    errors.push(format!(
                        "packet.dedupe_key: PR {n} in key is not in payload.prs {pr_numbers:?}"
                    ));
                }
            } else if source_only {
                let spr = source_pr.unwrap_or(0);
                if spr != n {
                    errors.push(format!(
                        "packet.dedupe_key: PR {n} in key does not match payload.source.pr {spr}"
                    ));
                }
            } else {
                errors.push(format!(
                    "packet.dedupe_key: family 'pr' requires a non-empty payload.prs containing PR {n} or a source-only payload.source.pr {n}, but neither is present (#6446)"
                ));
            }
        }
        "pr-set" => {
            let nums = m.nums.clone().unwrap_or_default();
            let mut sorted = nums.clone();
            sorted.sort();
            if nums != sorted {
                errors
                    .push("packet.dedupe_key: pr-set numbers must be sorted ascending".to_string());
            }
            if !pr_numbers.is_empty() {
                let mut sp = pr_numbers.clone();
                sp.sort();
                if nums != sp {
                    errors.push(format!(
                        "packet.dedupe_key: pr-set {nums:?} does not match payload.prs {sp:?}"
                    ));
                }
            } else if source_only {
                let spr = source_pr.unwrap_or(0);
                if !nums.contains(&spr) {
                    errors.push(format!(
                        "packet.dedupe_key: pr-set {nums:?} in key does not contain payload.source.pr {spr}"
                    ));
                }
            } else {
                errors.push(format!(
                    "packet.dedupe_key: family 'pr-set' requires a non-empty payload.prs matching pr-set {nums:?} or a source-only payload.source.pr in {nums:?}, but neither is present (#6446)"
                ));
            }
        }
        "set" => {
            let origin = m.origin.unwrap_or(0);
            let set_origin = payload
                .get("set")
                .and_then(|s| s.get("origin_issue"))
                .and_then(|v| v.as_i64());
            match set_origin {
                Some(oi) if oi != origin => {
                    errors.push(format!(
                        "packet.dedupe_key: origin issue {origin} does not match payload.set.origin_issue {oi}"
                    ));
                }
                Some(_) => {}
                None => {
                    errors.push(format!(
                        "packet.dedupe_key: family 'set' requires payload.set.origin_issue {origin} matching the key origin, but the payload carries no set.origin_issue (#6446)"
                    ));
                }
            }
        }
        _ => {}
    }
    errors
}

/// The not_before horizon (#6426; triage task 10871 decision): a
/// not_before more than 366 days in the future is REJECTED, never
/// clamped — a task gated that far out is born unclaimable while
/// still counting as queued. The same bound applies at every layer
/// that accepts a not_before: packet validation, the client
/// --not-before flag, and the broker's /park and /revive gates.
pub const NOT_BEFORE_MAX_FUTURE_SECONDS: i64 = 366 * 86_400;

/// (year, month, day) -> days since 1970-01-01 (Howard Hinnant's
/// algorithm, the same civil-date math fallback.rs and client.rs use).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y_adj = if m <= 2 { y - 1 } else { y };
    let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
    let yoe = (y_adj - era * 400) as u64; // [0, 399]
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64; // [0, 11]
    let doy = (153 * mp + 2) / 5 + (d as u64) - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe as i64 - 719_468
}

/// Parse an RFC 3339 timestamp (`...Z` or with a numeric offset,
/// optional fractional seconds) to epoch seconds. The day must exist
/// in the given month and year (the fallback.rs #6667 rule):
/// days_from_civil alone silently normalizes impossible dates.
/// Oracle: RFC 3339 + the Gregorian calendar.
fn parse_rfc3339_epoch(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = s
        .split_once('T')
        .or_else(|| s.split_once('t'))
        .or_else(|| s.split_once(' '))?;
    let mut dp = date.split('-');
    // RFC 3339 field widths come first (date-fullyear is 4DIGIT,
    // month/day/time components 2DIGIT): a year with unbounded
    // digits parses as an i64 and then overflows the civil-date
    // arithmetic — a panic in overflow-checked builds, a wrapped
    // negative epoch that passes the horizon gate in release
    // (Codex P2 on PR #8489). With the widths enforced, every
    // value below is provably inside i64 arithmetic range.
    fn digits(s: &str, n: usize) -> Option<i64> {
        if s.len() == n && s.bytes().all(|b| b.is_ascii_digit()) {
            s.parse().ok()
        } else {
            None
        }
    }
    let year: i64 = digits(dp.next()?, 4)?;
    let month: u32 = digits(dp.next()?, 2)?.try_into().ok()?;
    let day: u32 = digits(dp.next()?, 2)?.try_into().ok()?;
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if dp.next().is_some() || day < 1 || day > days_in_month {
        return None;
    }
    let (time, offset_s) = if let Some(t) = rest.strip_suffix(['Z', 'z']) {
        (t, 0i64)
    } else if let Some(idx) = rest.rfind(['+', '-']) {
        if idx == 0 {
            return None;
        }
        let (t, off) = rest.split_at(idx);
        let sign: i64 = if rest.as_bytes()[idx] == b'+' { 1 } else { -1 };
        let mut op = off[1..].split(':');
        let oh: i64 = digits(op.next()?, 2)?;
        // A present-but-invalid component is an error, never a
        // silent zero (Codex P2 on PR #8489, for seconds/fraction
        // below as well as here).
        let om: i64 = match op.next() {
            Some(v) => digits(v, 2)?,
            None => 0,
        };
        if op.next().is_some() {
            return None;
        }
        // RFC 3339 time-numoffset is a time-hour (00-23) and
        // time-minute (00-59): an unbounded offset would let the
        // epoch arithmetic wrap a far-future date back into the
        // accepted range (Codex P2 on PR #8489).
        if !(0..=23).contains(&oh) || !(0..=59).contains(&om) {
            return None;
        }
        (t, sign * (oh * 3600 + om * 60))
    } else {
        (rest, 0)
    };
    // Fractional seconds: when the '.' is present, the fraction
    // must be non-empty digits — discarding it unvalidated would
    // accept `00:00:00.garbage` (Codex P2 on PR #8489).
    let (time, fraction) = match time.split_once('.') {
        Some((t, f)) => (t, Some(f)),
        None => (time, None),
    };
    if let Some(f) = fraction {
        if f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    }
    let mut tp = time.split(':');
    let hh: i64 = digits(tp.next()?, 2)?;
    let mm: i64 = digits(tp.next()?, 2)?;
    let ss: i64 = match tp.next() {
        Some(v) => digits(v, 2)?,
        None => 0,
    };
    if tp.next().is_some() {
        return None;
    }
    if !(0..24).contains(&hh) || !(0..60).contains(&mm) || !(0..61).contains(&ss) {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hh * 3600 + mm * 60 + ss - offset_s)
}

fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Validate one not_before value against the #6426 rule at a given
/// "now" (epoch seconds): it must be a non-empty string that parses
/// as RFC 3339 and is no more than NOT_BEFORE_MAX_FUTURE_SECONDS in
/// the future. Past and near-future values pass unchanged. Every
/// error names `field` (`packet.not_before`, `--not-before`, or
/// `not_before`) so the rejection points at its source.
pub fn validate_not_before_at(value: &Value, field: &str, now: i64) -> Result<(), String> {
    let Some(s) = value.as_str() else {
        return Err(format!(
            "{field}: must be an RFC 3339 timestamp string, got {}",
            match value {
                Value::Number(_) => "a number",
                Value::Bool(_) => "a boolean",
                Value::Array(_) => "an array",
                Value::Object(_) => "an object",
                Value::Null => "null",
                Value::String(_) => unreachable!(),
            }
        ));
    };
    if s.trim().is_empty() {
        return Err(format!(
            "{field}: must not be empty (an RFC 3339 timestamp is required)"
        ));
    }
    let Some(epoch) = parse_rfc3339_epoch(s) else {
        return Err(format!("{field}: '{s}' is not a valid RFC 3339 timestamp"));
    };
    if epoch - now > NOT_BEFORE_MAX_FUTURE_SECONDS {
        return Err(format!(
            "{field}: {s} is more than 366 days in the future — a task gated that far out is born unclaimable (#6426); rejected, not clamped"
        ));
    }
    Ok(())
}

/// validate_not_before_at against the current time.
pub fn validate_not_before(value: &Value, field: &str) -> Result<(), String> {
    validate_not_before_at(value, field, now_epoch_seconds())
}

/// validate_packet: returns (errors, warnings).
fn valid_max_attempts(value: &Value) -> bool {
    value.as_u64().is_some_and(|n| (1..=100).contains(&n))
}

pub fn validate_packet(packet: &Value) -> (Vec<String>, Vec<String>) {
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let Some(obj) = packet.as_object() else {
        return (vec!["packet: must be a JSON object".to_string()], warnings);
    };
    let Some(payload) = obj.get("payload").filter(|p| p.is_object()) else {
        return (
            vec!["packet.payload: required object (repo-task/v1 payload)".to_string()],
            warnings,
        );
    };
    errors.extend(manual_schema_errors(payload));

    if let Some(value) = obj.get("max_attempts") {
        if !valid_max_attempts(value) {
            errors.push("packet.max_attempts: must be an integer between 1 and 100".to_string());
        } else if let Some(other) = payload.get("max_attempts") {
            if valid_max_attempts(other) && value != other {
                errors.push("packet.max_attempts: must match payload.max_attempts".to_string());
            }
        }
    }

    match obj.get("title") {
        Some(Value::String(t)) if !t.trim().is_empty() => {}
        _ => errors.push("packet.title: required non-empty string".to_string()),
    }
    match obj.get("priority") {
        None => errors.push(
            "packet.priority: required (numeric priority from the design scale: 10, 20, 30, 40, 50, 70, 90)"
                .to_string(),
        ),
        Some(Value::Number(n)) if n.is_i64() => {
            let v = n.as_i64().unwrap();
            if !PRIORITY_SCALE.contains(&v) {
                errors.push(format!(
                    "packet.priority: {v} is not on the design scale {:?}",
                    PRIORITY_SCALE
                ));
            }
        }
        Some(_) => errors.push("packet.priority: must be an integer".to_string()),
    }
    // not_before (#6426): absent or null keeps the historical
    // behaviour; a present value must satisfy validate_not_before —
    // until now it was forwarded verbatim, so garbage failed only as
    // a downstream DB cast error and a far-future value enqueued a
    // task the claim predicate would not admit for years.
    if let Some(nb) = obj.get("not_before").filter(|v| !v.is_null()) {
        if let Err(e) = validate_not_before(nb, "packet.not_before") {
            errors.push(e);
        }
    }
    match obj.get("body") {
        Some(Value::String(body)) if !body.trim().is_empty() => {
            let words = body.split_whitespace().count();
            if words > BODY_WARN_WORDS {
                warnings.push(format!(
                    "packet.body: {words} words exceeds the ~{BODY_WARN_WORDS}-word design target"
                ));
            }
        }
        // Absent is valid, and null is its stored spelling:
        // task_from_packet writes body:null for a body-less packet, and
        // the claim guard (#5444) revalidates stored rows through this
        // same function — a null body must not quarantine a task that
        // enqueue accepted (Codex P1 on PR #8477).
        None | Some(Value::Null) => {}
        // body=12345/{x:1}/[a]/true and even body='' used to pass silently
        // because the body was only read when it was a string (#6430).
        Some(_) => errors.push("packet.body: must be a non-empty string".to_string()),
    }
    let size = serde_json::to_string(payload).map(|s| s.len()).unwrap_or(0);
    if size > PAYLOAD_MAX_BYTES {
        errors.push(format!(
            "payload size: {size} bytes exceeds the {PAYLOAD_MAX_BYTES}-byte hard ceiling (evidence and pointers, not full diffs/logs)"
        ));
    } else if size > PAYLOAD_WARN_BYTES {
        warnings.push(format!(
            "payload size: {size} bytes is above the {PAYLOAD_WARN_BYTES}-byte design target"
        ));
    }
    if let Some(tb) = payload.get("timebox_minutes").and_then(|v| v.as_i64()) {
        if tb != 20 && tb != 45 {
            warnings.push(format!(
                "payload.timebox_minutes: {tb} is not a standard design timebox (20 triage / 45 standard)"
            ));
        }
    }
    if let Some(ma) = payload.get("max_attempts").and_then(|v| v.as_i64()) {
        let kind = payload.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        let want = if kind == "triage" || kind == "ci_investigation" {
            6
        } else {
            4
        };
        if ma != want {
            warnings.push(format!(
                "payload.max_attempts: design default for {kind} is {want}, packet says {ma}"
            ));
        }
    }
    errors.extend(dedupe_errors(packet, payload));
    (errors, warnings)
}

/// task_from_packet: the agent.tasks row shape the enqueue RPC takes.
pub fn task_from_packet(packet: &Value, queue: &str) -> Value {
    let payload = packet.get("payload").cloned().unwrap_or(json!({}));
    let max_attempts = packet
        .get("max_attempts")
        .filter(|v| !v.is_null())
        .or_else(|| payload.get("max_attempts"))
        .cloned()
        .unwrap_or(json!(4));
    json!({
        "queue": queue,
        "title": packet.get("title").cloned().unwrap_or(Value::Null),
        "body": packet.get("body").cloned().unwrap_or(Value::Null),
        "payload": payload,
        "dedupe_key": packet.get("dedupe_key").cloned().unwrap_or(Value::Null),
        "priority": packet.get("priority").cloned().unwrap_or(Value::Null),
        "not_before": packet.get("not_before").cloned().unwrap_or(Value::Null),
        "max_attempts": max_attempts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn packet_max_attempts_validation_and_selection() {
        let payload = payload_with(json!({"issue": {"number": 6423}}));
        let base = json!({"title": "test", "priority": 30, "payload": payload});
        for bad in [
            json!(0),
            json!(-1),
            json!(101),
            json!(1.5),
            json!("4"),
            json!(null),
        ] {
            for top in [true, false] {
                let mut packet = base.clone();
                if top {
                    packet["max_attempts"] = bad.clone();
                } else {
                    packet["payload"]["max_attempts"] = bad.clone();
                }
                let field = if top {
                    "packet.max_attempts"
                } else {
                    "payload.max_attempts"
                };
                assert!(
                    validate_packet(&packet).0.iter().any(|e| e.contains(field)),
                    "{packet}"
                );
            }
        }
        let mut packet = base.clone();
        packet["max_attempts"] = json!(5);
        assert!(
            validate_packet(&packet)
                .0
                .iter()
                .any(|e| e.contains("must match"))
        );
        for n in [1, 4, 100] {
            packet["max_attempts"] = json!(n);
            packet["payload"]["max_attempts"] = json!(n);
            assert!(
                !validate_packet(&packet)
                    .0
                    .iter()
                    .any(|e| e.contains("max_attempts"))
            );
            assert_eq!(task_from_packet(&packet, "q")["max_attempts"], n);
        }
        packet["payload"]
            .as_object_mut()
            .unwrap()
            .remove("max_attempts");
        assert!(
            !validate_packet(&packet)
                .0
                .iter()
                .any(|e| e.contains("max_attempts"))
        );
        assert_eq!(task_from_packet(&packet, "q")["max_attempts"], 100);
        packet.as_object_mut().unwrap().remove("max_attempts");
        assert!(
            !validate_packet(&packet)
                .0
                .iter()
                .any(|e| e.contains("max_attempts"))
        );
        assert_eq!(task_from_packet(&packet, "q")["max_attempts"], 4);
        packet["payload"]["max_attempts"] = json!(7);
        assert_eq!(task_from_packet(&packet, "q")["max_attempts"], 7);
    }

    fn payload_with(identity: serde_json::Value) -> Value {
        let mut payload = json!({
            "schema": "repo-task/v1",
            "kind": "small_fix",
            "repo": "developing-today/code",
            "problem": "p",
            "priority_reason": "r",
            "evidence": ["e"],
            "acceptance": ["a"],
            "tests": ["t"],
            "context_refs": [],
            "scope": {"include": ["x"], "exclude": ["y"]},
            "risk": {"level": "low", "flags": []},
            "timebox_minutes": 45,
            "max_attempts": 4,
            "dedupe": {"checked_at": "2026-11-04T00:00:00Z", "basis": "search"},
        });
        if let (Some(obj), Some(extra)) = (payload.as_object_mut(), identity.as_object()) {
            for (k, v) in extra {
                obj.insert(k.clone(), v.clone());
            }
        }
        payload
    }

    fn identity_errors(payload: &Value) -> Vec<String> {
        manual_schema_errors(payload)
            .into_iter()
            .filter(|e| e.contains("identity") || e.contains("issue.number"))
            .collect()
    }

    #[test]
    fn placeholder_identities_do_not_validate() {
        // #5549 variant A: prs [null] is not an identity.
        assert!(!identity_errors(&payload_with(json!({"prs": [null]}))).is_empty());
        // Variant B: prs elements without a positive-integer number.
        assert!(
            !identity_errors(&payload_with(json!({"prs": [{"number": "5424"}, {}]}))).is_empty()
        );
        // Variant C: an empty set object is not an identity.
        assert!(!identity_errors(&payload_with(json!({"set": {}}))).is_empty());
        // A non-positive issue number is an error in its own right.
        assert!(!identity_errors(&payload_with(json!({"issue": {"number": -5}}))).is_empty());
        assert!(!identity_errors(&payload_with(json!({"issue": {"number": 0}}))).is_empty());
        // Real shapes still satisfy identity.
        assert!(identity_errors(&payload_with(json!({"prs": [{"number": 5424}]}))).is_empty());
        assert!(identity_errors(&payload_with(json!({"set": {"prs": [605, 620]}}))).is_empty());
        assert!(identity_errors(&payload_with(json!({"issue": {"number": 5424}}))).is_empty());
        assert!(
            identity_errors(&payload_with(
                json!({"source": {"pr": 588, "finding": "f"}})
            ))
            .is_empty()
        );
    }

    #[test]
    fn pr_set_keys_are_canonical() {
        // A lone number is the `pr` family's spelling, not a pr-set.
        assert!(match_dedupe("repo-health:v1:pr-set:developing-today/code:5424").is_none());
        // A repeated number is not a set.
        assert!(match_dedupe("repo-health:v1:pr-set:developing-today/code:5424-5424").is_none());
        // Unsorted is not the canonical (sorted) spelling.
        assert!(match_dedupe("repo-health:v1:pr-set:developing-today/code:5424-100").is_none());
        // The canonical two-number and three-number shapes match.
        let m = match_dedupe("repo-health:v1:pr-set:developing-today/code:100-5424")
            .expect("two-number pr-set matches");
        assert_eq!(m.nums.as_deref(), Some([100, 5424].as_slice()));
        assert!(match_dedupe("repo-health:v1:pr-set:developing-today/code:100-200-5424").is_some());
        // The single-PR family is untouched.
        assert!(match_dedupe("repo-health:v1:pr:developing-today/code:pr:5424").is_some());
    }

    #[test]
    fn plain_number_keys_are_canonical() {
        // Leading zeros are a different dedupe_key string for the same
        // number: a second dedupe identity for the same work (#5510).
        assert!(match_dedupe("repo-health:v1:fix:developing-today/code:issue:05424").is_none());
        assert!(match_dedupe("repo-health:v1:fix:developing-today/code:issue:0005424").is_none());
        assert!(match_dedupe("repo-health:v1:triage:developing-today/code:issue:05424").is_none());
        assert!(match_dedupe("repo-health:v1:pr:developing-today/code:pr:05424").is_none());
        assert!(match_dedupe("repo-health:v1:set:05424:owed:developing-today/code").is_none());
        assert!(match_dedupe("repo-health:v1:pr-set:developing-today/code:0100-5424").is_none());
        // Issue/PR numbers start at 1: 0 names nothing.
        assert!(match_dedupe("repo-health:v1:fix:developing-today/code:issue:0").is_none());
        assert!(match_dedupe("repo-health:v1:pr:developing-today/code:pr:0").is_none());
        assert!(match_dedupe("repo-health:v1:set:0:owed:developing-today/code").is_none());
        // Canonical spellings are untouched.
        assert!(
            match_dedupe("repo-health:v1:fix:developing-today-forks/opencode:issue:5424").is_some()
        );
        assert!(
            match_dedupe("repo-health:v1:triage:developing-today-forks/openchamber:issue:5424")
                .is_some()
        );
        let m = match_dedupe("repo-health:v1:fix:developing-today/code:issue:5424")
            .expect("canonical fix key matches");
        assert_eq!(m.num, Some(5424));
        let m = match_dedupe("repo-health:v1:set:5424:owed:developing-today/code")
            .expect("canonical set key matches");
        assert_eq!(m.origin, Some(5424));
        assert!(match_dedupe("repo-health:v1:pr-set:developing-today/code:100-5424").is_some());
    }

    #[test]
    fn maintenance_keys_name_real_iso_weeks() {
        assert!(
            match_dedupe("repo-health:v1:maintenance:developing-today/code:weekly:2026-W44")
                .is_some()
        );
        assert!(
            match_dedupe("repo-health:v1:maintenance:developing-today/code:weekly:2026-W53")
                .is_some()
        );
        assert!(
            match_dedupe("repo-health:v1:maintenance:developing-today/code:weekly:2026-W54")
                .is_none()
        );
        assert!(
            match_dedupe("repo-health:v1:maintenance:developing-today/code:weekly:2026-W99")
                .is_none()
        );
        assert!(
            match_dedupe("repo-health:v1:maintenance:developing-today/code:weekly:2026-W00")
                .is_none()
        );
    }
    fn valid_packet() -> Value {
        json!({
            "title": "t",
            "body": "b",
            "priority": 20,
            "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:6430",
            "payload": payload_with(json!({"issue": {"number": 6430}})),
        })
    }

    #[test]
    fn packet_string_fields_and_list_elements_are_type_checked() {
        // Valid control: the well-typed packet still validates clean (#6430).
        let (errors, _) = validate_packet(&valid_packet());
        assert!(errors.is_empty(), "valid control errors: {errors:?}");

        // A body-less packet validates, in both spellings: the key
        // absent (enqueue form) and null (the stored-row form the claim
        // guard revalidates, #5444 / PR #8477 Codex P1).
        let mut no_body = valid_packet();
        no_body.as_object_mut().unwrap().remove("body");
        let (errors, _) = validate_packet(&no_body);
        assert!(errors.is_empty(), "absent body errors: {errors:?}");
        let mut null_body = valid_packet();
        null_body["body"] = Value::Null;
        let (errors, _) = validate_packet(&null_body);
        assert!(errors.is_empty(), "null body errors: {errors:?}");

        // The probe cases from issue #6430 (~/git/fleet/probe-queue-body):
        // every malformed variant must now error, naming its field.
        let cases: Vec<(&str, Value, &str)> = vec![
            ("/body", json!(12345), "packet.body"),
            ("/body", json!({"x": 1}), "packet.body"),
            ("/body", json!(["a"]), "packet.body"),
            ("/body", json!(true), "packet.body"),
            ("/body", json!(""), "packet.body"),
            ("/payload/problem", json!(123), "payload.problem"),
            ("/payload/problem", json!({"x": 1}), "payload.problem"),
            (
                "/payload/priority_reason",
                json!(123),
                "payload.priority_reason",
            ),
            ("/payload/dedupe/checked_at", json!(123), "payload.dedupe"),
            ("/payload/dedupe/basis", json!(123), "payload.dedupe"),
            ("/payload/evidence", json!([123]), "payload.evidence"),
            ("/payload/evidence", json!([null]), "payload.evidence"),
            ("/payload/evidence", json!([{"x": 1}]), "payload.evidence"),
            ("/payload/evidence", json!([["t"]]), "payload.evidence"),
            ("/payload/acceptance", json!([123]), "payload.acceptance"),
            ("/payload/tests", json!([123]), "payload.tests"),
            (
                "/payload/context_refs",
                json!([null]),
                "payload.context_refs",
            ),
            (
                "/payload/scope/include",
                json!([123]),
                "payload.scope.include",
            ),
            (
                "/payload/scope/exclude",
                json!([123]),
                "payload.scope.exclude",
            ),
            (
                "/payload/scope/exclude",
                json!("y"),
                "payload.scope.exclude",
            ),
            ("/payload/risk/flags", json!([123]), "payload.risk.flags"),
        ];
        for (pointer, bad, field) in cases {
            let mut packet = valid_packet();
            *packet
                .pointer_mut(pointer)
                .unwrap_or_else(|| panic!("pointer {pointer} exists")) = bad.clone();
            let (errors, _) = validate_packet(&packet);
            assert!(
                errors.iter().any(|e| e.contains(field)),
                "{pointer} = {bad} must error naming {field}; got {errors:?}"
            );
        }
    }

    fn dedupe_errs(key: &str, kind: &str, identity: serde_json::Value) -> Vec<String> {
        let mut payload = payload_with(identity);
        payload
            .as_object_mut()
            .expect("payload object")
            .insert("kind".to_string(), json!(kind));
        let packet = json!({"dedupe_key": key});
        dedupe_errors(&packet, &payload)
    }

    #[test]
    fn dedupe_cross_check_requires_the_family_identity() {
        // #6446 controls: the matching identity present and agreeing
        // validates, for every family.
        assert!(
            dedupe_errs(
                "repo-health:v1:fix:developing-today/code:issue:222",
                "small_fix",
                json!({"issue": {"number": 222}}),
            )
            .is_empty()
        );
        assert!(
            dedupe_errs(
                "repo-health:v1:pr:developing-today/code:pr:222",
                "pr_shepherd",
                json!({"prs": [{"number": 222}]}),
            )
            .is_empty()
        );
        assert!(
            dedupe_errs(
                "repo-health:v1:pr-set:developing-today/code:111-222",
                "pr_shepherd",
                json!({"prs": [{"number": 222}, {"number": 111}]}),
            )
            .is_empty()
        );
        assert!(
            dedupe_errs(
                "repo-health:v1:set:9999:owed:developing-today/code",
                "pr_shepherd",
                json!({"set": {"origin_issue": 9999}}),
            )
            .is_empty()
        );
        // Source-only controls: a pre-filing source identity whose pr
        // agrees with the key validates for the pr families.
        assert!(
            dedupe_errs(
                "repo-health:v1:pr:developing-today/code:pr:588",
                "pr_shepherd",
                json!({"source": {"pr": 588, "finding": "f"}}),
            )
            .is_empty()
        );
        assert!(
            dedupe_errs(
                "repo-health:v1:pr-set:developing-today/code:111-588",
                "pr_shepherd",
                json!({"source": {"pr": 588, "finding": "f"}}),
            )
            .is_empty()
        );
        // Existing mismatch controls still error.
        assert!(
            !dedupe_errs(
                "repo-health:v1:fix:developing-today/code:issue:8888",
                "small_fix",
                json!({"issue": {"number": 9999}}),
            )
            .is_empty()
        );

        // The 7 escape cases from #6446: identity via a different field
        // than the family names used to validate with errors=[].
        // 1. fix key vs prs-only identity.
        assert!(
            !dedupe_errs(
                "repo-health:v1:fix:developing-today/code:issue:222",
                "small_fix",
                json!({"prs": [{"number": 111}]}),
            )
            .is_empty()
        );
        // 2. triage key vs prs-only identity.
        assert!(
            !dedupe_errs(
                "repo-health:v1:triage:developing-today/code:issue:222",
                "triage",
                json!({"prs": [{"number": 111}]}),
            )
            .is_empty()
        );
        // 3. pr key vs issue-only identity.
        assert!(
            !dedupe_errs(
                "repo-health:v1:pr:developing-today/code:pr:222",
                "pr_shepherd",
                json!({"issue": {"number": 111}}),
            )
            .is_empty()
        );
        // 4. pr-set key vs issue-only identity.
        assert!(
            !dedupe_errs(
                "repo-health:v1:pr-set:developing-today/code:111-222",
                "pr_shepherd",
                json!({"issue": {"number": 9999}}),
            )
            .is_empty()
        );
        // 5. set key vs a set object without origin_issue.
        assert!(
            !dedupe_errs(
                "repo-health:v1:set:9999:owed:developing-today/code",
                "pr_shepherd",
                json!({"set": {"prs": [605, 620]}}),
            )
            .is_empty()
        );
        // 6. fix key vs source-only identity.
        assert!(
            !dedupe_errs(
                "repo-health:v1:fix:developing-today/code:issue:777",
                "small_fix",
                json!({"source": {"pr": 588, "finding": "f"}}),
            )
            .is_empty()
        );
        // 7. pr key vs source-only identity naming a different PR.
        assert!(
            !dedupe_errs(
                "repo-health:v1:pr:developing-today/code:pr:777",
                "pr_shepherd",
                json!({"source": {"pr": 588, "finding": "f"}}),
            )
            .is_empty()
        );
        // A set packet whose identity is an issue object has no
        // set.origin_issue to check the key against: rejected.
        assert!(
            !dedupe_errs(
                "repo-health:v1:set:9999:owed:developing-today/code",
                "pr_shepherd",
                json!({"issue": {"number": 9999}}),
            )
            .is_empty()
        );
    }

    #[test]
    fn not_before_is_validated_against_rfc3339_and_the_366_day_horizon() {
        // #6426: not_before was forwarded verbatim at every layer —
        // garbage failed only as a downstream DB cast error, and a
        // far-future value enqueued a task born unclaimable. Oracle:
        // RFC 3339 plus the triage decision (reject beyond 366 days).
        // Fixed "now": 2026-11-06T00:00:00Z.
        let now = parse_rfc3339_epoch("2026-11-06T00:00:00Z").expect("test now parses");
        // Non-string shapes are errors naming the field.
        for bad in [
            json!(42),
            json!(true),
            json!({}),
            json!(["2026-11-06T00:00:00Z"]),
        ] {
            let err = validate_not_before_at(&bad, "packet.not_before", now)
                .err()
                .expect("non-string not_before must be rejected");
            assert!(
                err.contains("packet.not_before"),
                "error names the field: {err}"
            );
        }
        // Empty and unparseable strings are errors naming the field.
        for bad in [
            "",
            "   ",
            "banana",
            "2026-13-01T00:00:00Z",
            "2026-02-31T00:00:00Z",
            "2026-11-06",
            // Offsets outside the RFC 3339 ranges are not valid
            // timestamps — and an unbounded one must not be able to
            // wrap a far-future date past the horizon check.
            "2026-11-06T00:00:00+24:00",
            "2026-11-06T00:00:00+00:60",
            "9999-01-01T00:00:00+70000000:00",
            // Field widths are RFC 3339 syntax, not decoration: an
            // unbounded year overflows the epoch arithmetic.
            "300000000000-01-01T00:00:00Z",
            "10000-01-01T00:00:00Z",
            "2026-1-06T00:00:00Z",
            // Present-but-invalid components must not silently
            // default to zero.
            "2026-11-06T00:00:bananaZ",
            "2026-11-06T00:00:00.garbageZ",
            "2026-11-06T00:00:00+05:xx",
            "2026-11-06T00:00:00:00Z",
        ] {
            let err = validate_not_before_at(&json!(bad), "packet.not_before", now)
                .err()
                .expect("garbage not_before must be rejected");
            assert!(
                err.contains("packet.not_before"),
                "error names the field: {err}"
            );
        }
        // Beyond the 366-day horizon is an error (a year-9999 value
        // must not silently enqueue as a normal queued task).
        for far in ["9999-01-01T00:00:00Z", "2027-11-07T00:00:01Z"] {
            let err = validate_not_before_at(&json!(far), "packet.not_before", now)
                .err()
                .expect("far-future not_before must be rejected");
            assert!(
                err.contains("packet.not_before"),
                "error names the field: {err}"
            );
            assert!(err.contains("366 days"), "error names the horizon: {err}");
        }
        // Exactly at the horizon, near-future, past, and offset forms
        // pass unchanged.
        for ok in [
            "2027-11-07T00:00:00Z",
            "2026-11-06T00:00:01Z",
            "2020-01-01T00:00:00Z",
            "2026-11-06T05:30:00+05:30",
            "2026-11-06T00:00:00.5Z",
            // RFC 3339 permits lowercase t/z markers; PostgreSQL's
            // timestamptz conversion accepted them before this
            // validation existed (Codex P2 on PR #8489).
            "2026-11-06t00:00:00z",
        ] {
            assert!(
                validate_not_before_at(&json!(ok), "packet.not_before", now).is_ok(),
                "valid not_before passes: {ok}"
            );
        }
    }

    #[test]
    fn validate_packet_applies_the_not_before_rule() {
        // Packet-level wiring: a present not_before is checked by
        // validate_packet; absent and null stay exactly as before.
        fn packet_with(not_before: Option<Value>) -> Value {
            let mut packet = json!({
                "title": "t",
                "priority": 40,
                "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:6426",
                "payload": payload_with(json!({"issue": {"number": 6426}})),
            });
            if let Some(nb) = not_before {
                packet
                    .as_object_mut()
                    .expect("packet object")
                    .insert("not_before".to_string(), nb);
            }
            packet
        }
        fn nb_errors(packet: &Value) -> Vec<String> {
            validate_packet(packet)
                .0
                .into_iter()
                .filter(|e| e.contains("not_before"))
                .collect()
        }
        assert!(nb_errors(&packet_with(None)).is_empty());
        assert!(nb_errors(&packet_with(Some(Value::Null))).is_empty());
        assert!(nb_errors(&packet_with(Some(json!("2020-01-01T00:00:00Z")))).is_empty());
        for bad in [
            json!("banana"),
            json!(""),
            json!(42),
            json!({}),
            json!("9999-01-01T00:00:00Z"),
        ] {
            let errs = nb_errors(&packet_with(Some(bad.clone())));
            assert_eq!(errs.len(), 1, "one not_before error for {bad}: {errs:?}");
            assert!(
                errs[0].contains("packet.not_before"),
                "error names the field: {}",
                errs[0]
            );
        }
    }

    /// Test-local civil_from_days (Howard Hinnant's algorithm) —
    /// the independent inverse of days_from_civil, used as the
    /// calendar oracle by the property tests below.
    fn civil_from_days(z: i64) -> (i64, u32, u32) {
        let z = z + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = (z - era * 146_097) as u64;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        (if m <= 2 { y + 1 } else { y }, m, d)
    }

    fn format_utc(epoch: i64) -> String {
        let (y, m, d) = civil_from_days(epoch.div_euclid(86_400));
        let secs = epoch.rem_euclid(86_400);
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
            secs / 3600,
            (secs % 3600) / 60,
            secs % 60
        )
    }

    proptest! {
        /// Calendar invariant (Codex P1 on PR #8489): the parser
        /// accepts a (year, month, day) triple iff that day exists,
        /// and the parsed value is the civil-date epoch. Oracle: the
        /// inverse pair days_from_civil/civil_from_days — a triple
        /// is a real date iff the round trip returns it unchanged
        /// (normalization moves impossible dates into the next
        /// month, so their round trip differs).
        #[test]
        fn parse_accepts_exactly_real_dates(
            year in 1600i64..2400,
            month in 1u32..=12,
            day in 1u32..=31,
        ) {
            let s = format!("{year:04}-{month:02}-{day:02}T00:00:00Z");
            let real = civil_from_days(days_from_civil(year, month, day)) == (year, month, day);
            prop_assert_eq!(parse_rfc3339_epoch(&s).is_some(), real, "{}", s);
            if real {
                prop_assert_eq!(
                    parse_rfc3339_epoch(&s),
                    Some(days_from_civil(year, month, day) * 86_400)
                );
            }
        }

        /// Round-trip invariant: any epoch in the RFC 3339 year
        /// range formats (via the civil_from_days oracle) to a
        /// string the parser reads back as the same epoch.
        #[test]
        fn parse_format_round_trip_any_epoch(epoch in -31_622_400i64..253_402_300_799) {
            prop_assert_eq!(parse_rfc3339_epoch(&format_utc(epoch)), Some(epoch));
        }

        /// Offset-equivalence invariant: the same wall clock with a
        /// numeric offset names the instant shifted by exactly that
        /// offset. Oracle: the offset-seconds arithmetic identity.
        #[test]
        fn offset_forms_shift_by_exactly_the_offset(
            epoch in -31_622_400i64..253_402_300_799,
            oh in 0i64..=23,
            om in 0i64..=59,
            sign in proptest::bool::ANY,
        ) {
            let (y, m, d) = civil_from_days(epoch.div_euclid(86_400));
            let secs = epoch.rem_euclid(86_400);
            let s = format!(
                "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}{}{oh:02}:{om:02}",
                secs / 3600,
                (secs % 3600) / 60,
                secs % 60,
                if sign { "+" } else { "-" }
            );
            let offset = (if sign { 1 } else { -1 }) * (oh * 3600 + om * 60);
            prop_assert_eq!(parse_rfc3339_epoch(&s), Some(epoch - offset), "{}", s);
        }

        /// Horizon invariant (#6426): validation accepts a formatted
        /// instant iff it is at most NOT_BEFORE_MAX_FUTURE_SECONDS
        /// past the given now. Oracle: the direct epoch comparison.
        #[test]
        fn horizon_accepts_iff_within_366_days(
            now in 0i64..2_000_000_000,
            delta in -40_000_000i64..40_000_000,
        ) {
            let epoch = now + delta;
            let ok = validate_not_before_at(&json!(format_utc(epoch)), "packet.not_before", now)
                .is_ok();
            prop_assert_eq!(ok, delta <= NOT_BEFORE_MAX_FUTURE_SECONDS, "delta {}", delta);
        }

        /// Robustness invariant: arbitrary text never panics the
        /// parser or the validator — every input resolves to
        /// None/Some or Err/Ok. (The enumerated shapes above pin
        /// the semantics; this pins the absence of panics on
        /// overflow-shaped inputs, the failure mode of Codex
        /// rounds 2-4 on PR #8489.)
        #[test]
        fn arbitrary_text_never_panics(
            chars in proptest::collection::vec(proptest::char::range('\u{20}', '\u{7e}'), 0..64),
        ) {
            let s: String = chars.into_iter().collect();
            let _ = parse_rfc3339_epoch(&s);
            let _ = validate_not_before_at(&json!(s), "packet.not_before", 1_762_700_000);
        }
    }

    fn second_pass_payload(review: serde_json::Value) -> Value {
        let mut payload = payload_with(json!({"issue": {"number": 2310}}));
        let obj = payload.as_object_mut().expect("payload object");
        obj.insert("kind".to_string(), json!("second_pass"));
        obj.insert("review".to_string(), review);
        payload
    }

    fn valid_review() -> serde_json::Value {
        json!({
            "verdict": "second-pass",
            "verdict_note_url": "https://github.com/developing-today/code/issues/2025#issuecomment-1",
            "artifact": {"pr": 9001, "issue": 2310, "merged": false},
            "pass": 1,
            "pass_cap": 2,
            "gap_list": ["ring-2 gap: no test pins the cap"],
            "goal_context_refs": ["https://github.com/developing-today/code/issues/2025"],
            "producer": {"task_id": 7325, "session": "ses_x", "pr": 9001},
            "route_floor": "specialist",
        })
    }

    #[test]
    fn second_pass_packet_validates() {
        let packet = json!({
            "title": "Second pass 1 on #2310",
            "priority": 20,
            "dedupe_key": "repo-health:v1:second-pass:developing-today/code:2310:pass-1",
            "payload": second_pass_payload(valid_review()),
        });
        let (errors, _warnings) = validate_packet(&packet);
        assert!(errors.is_empty(), "well-formed second_pass packet must validate; got {errors:?}");
    }

    #[test]
    fn second_pass_rejects_missing_gap_list() {
        let mut review = valid_review();
        review.as_object_mut().unwrap().remove("gap_list");
        let errors = manual_schema_errors(&second_pass_payload(review));
        assert!(
            errors.iter().any(|e| e.contains("review.gap_list")),
            "missing gap_list must be rejected; got {errors:?}"
        );
    }

    #[test]
    fn second_pass_rejects_missing_verdict_note_url() {
        let mut review = valid_review();
        review.as_object_mut().unwrap().remove("verdict_note_url");
        let errors = manual_schema_errors(&second_pass_payload(review));
        assert!(
            errors.iter().any(|e| e.contains("review.verdict_note_url")),
            "missing verdict_note_url must be rejected; got {errors:?}"
        );
    }

    #[test]
    fn second_pass_rejects_pass_above_cap() {
        let mut review = valid_review();
        review["pass"] = json!(3);
        let errors = manual_schema_errors(&second_pass_payload(review));
        assert!(
            errors.iter().any(|e| e.contains("exceeds pass_cap")),
            "pass > pass_cap must be rejected at enqueue; got {errors:?}"
        );
    }

    #[test]
    fn second_pass_requires_the_review_block() {
        let mut payload = payload_with(json!({"issue": {"number": 2310}}));
        payload
            .as_object_mut()
            .unwrap()
            .insert("kind".to_string(), json!("second_pass"));
        let errors = manual_schema_errors(&payload);
        assert!(
            errors.iter().any(|e| e.contains("payload.review")),
            "second_pass without a review block must be rejected; got {errors:?}"
        );
    }

    #[test]
    fn second_pass_dedupe_family_matches_issue_and_pass() {
        let identity = json!({"issue": {"number": 2310}, "review": {"pass": 1}});
        assert!(
            dedupe_errs(
                "repo-health:v1:second-pass:developing-today/code:2310:pass-1",
                "second_pass",
                identity.clone(),
            )
            .is_empty()
        );
        // Same (issue, pass) is the same key by construction: a second
        // commissioning collides on it instead of double-commissioning.
        let m = match_dedupe("repo-health:v1:second-pass:developing-today/code:2310:pass-1")
            .expect("second-pass key parses");
        assert_eq!(m.family, "second-pass");
        assert_eq!(m.num, Some(2310));
        assert_eq!(m.pass, Some(1));
        // Pass disagreement between key and payload is an error.
        let errors = dedupe_errs(
            "repo-health:v1:second-pass:developing-today/code:2310:pass-2",
            "second_pass",
            identity,
        );
        assert!(
            errors.iter().any(|e| e.contains("pass 2 in key")),
            "key pass must match review.pass; got {errors:?}"
        );
        // The second-pass family is not valid for other kinds, and
        // other families are not valid for second_pass.
        assert!(!dedupe_errs(
            "repo-health:v1:second-pass:developing-today/code:2310:pass-1",
            "small_fix",
            json!({"issue": {"number": 2310}}),
        )
        .is_empty());
        assert!(!dedupe_errs(
            "repo-health:v1:fix:developing-today/code:issue:2310",
            "second_pass",
            json!({"issue": {"number": 2310}}),
        )
        .is_empty());
    }
}
