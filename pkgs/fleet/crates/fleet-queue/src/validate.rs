//! Packet validation — faithful port of repo-queue.py's built-in
//! (manual) validator: envelope checks, manual schema checks, and the
//! dedupe-key family rules. (Legacy Python prefers the jsonschema
//! library when importable; messages for exotic packets can differ in
//! wording, but accept/reject semantics match the manual contract and
//! the database RPCs remain the backstop.)

use serde_json::{Value, json};

pub const PRIORITY_SCALE: [i64; 7] = [10, 20, 30, 40, 50, 70, 90];
pub const KINDS: [&str; 4] = ["pr_shepherd", "triage", "small_fix", "ci_investigation"];
// Both the old and the current names of the two repos that moved to the
// developing-today-forks org (2026-11-04) stay allowlisted: in-flight queue
// tasks carry dedupe keys with the old names and must keep validating,
// while new tasks use the developing-today-forks names. The same applies to dezren39/nix, renamed to dezren39/darwin-nix.
pub const FLEET_REPOS: [&str; 10] = [
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
maintenance: repo-health:v1:maintenance:<repo>:<kind>:<YYYY-Www>";

fn is_empty(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
        _ => false,
    }
}

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
        if is_empty(get(f)) {
            errs.push(format!("payload.{f}: required field missing or empty"));
        }
    }
    for f in ["evidence", "acceptance", "tests", "context_refs"] {
        if !matches!(get(f), Some(Value::Array(_))) {
            errs.push(format!("payload.{f}: must be a list"));
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
            if !matches!(scope.get("include"), Some(Value::Array(_))) {
                errs.push("payload.scope.include: must be a list".to_string());
            }
            if is_empty(scope.get("exclude")) {
                errs.push("payload.scope.exclude: required (scope exclusions)".to_string());
            }
        }
        _ => errs.push("payload.scope: must be an object with include/exclude".to_string()),
    }
    match get("risk") {
        Some(Value::Object(risk)) if matches!(risk.get("level"), Some(Value::String(l)) if ["low","medium","high"].contains(&l.as_str())) => {
            if !matches!(risk.get("flags"), Some(Value::Array(_))) {
                errs.push("payload.risk.flags: must be a list".to_string());
            }
        }
        _ => errs.push("payload.risk.level: must be low|medium|high".to_string()),
    }
    for f in ["timebox_minutes", "max_attempts"] {
        let ok =
            matches!(get(f), Some(Value::Number(n)) if n.as_i64().map(|v| v >= 1).unwrap_or(false));
        if !ok {
            errs.push(format!("payload.{f}: must be a positive integer"));
        }
    }
    let ded_ok = matches!(get("dedupe"), Some(Value::Object(d))
        if !is_empty(d.get("checked_at")) && !is_empty(d.get("basis")));
    if !ded_ok {
        errs.push("payload.dedupe: must have checked_at and basis".to_string());
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
                });
            }
        }
        return None;
    }
    for fam in ["fix", "triage", "pr-set", "pr", "maintenance"] {
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

/// validate_packet: returns (errors, warnings).
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
    if let Some(Value::String(body)) = obj.get("body") {
        let words = body.split_whitespace().count();
        if words > BODY_WARN_WORDS {
            warnings.push(format!(
                "packet.body: {words} words exceeds the ~{BODY_WARN_WORDS}-word design target"
            ));
        }
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
}
