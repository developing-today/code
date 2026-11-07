//! GitHub-native fallback mode for the queue domain (Drew's design,
//! ordered 2026-11-06; tracking issue developing-today/code#6619's
//! outage is the motivating incident).
//!
//! When the queue backend (queue-broker → Supabase PostgREST) is
//! unreachable, the queue's workers starve: they cannot claim, and the
//! fleet's work sits in GitHub issues doing nothing. Fallback mode
//! moves the queue's coordination surface into the issues themselves,
//! where the work already lives:
//!
//!   * Work items are open issues carrying the fleet's existing
//!     `queue/queued` (waiting) or `queue/claimed` (held) labels — the
//!     same labels the broker-backed system already maintains
//!     (worker instruction §9; pm-stack dispatch's LABEL_DEFS). No
//!     parallel taxonomy.
//!   * A worker CLAIMS an issue by posting a comment whose first line
//!     is a machine-readable marker (below). The human-readable line
//!     under it says who picked the item up.
//!   * The claim is a LEASE: it stays live while the holder posts
//!     heartbeat markers as it works. If no marker from the holder
//!     lands for the lease window (default 30 minutes), the claim is
//!     stale and any other worker may claim the issue, superseding
//!     it. Drew's range was 20–30 minutes; 30 is the default because
//!     worker checkpoints land at most ~15 minutes apart on long
//!     work and the fix timebox is 45 minutes — a 20-minute lease
//!     would let a live claim look dead between two checkpoints.
//!     Override with `--lease-min` or FLEET_QUEUE_FALLBACK_LEASE_MIN.
//!   * A worker RELEASES by posting a release marker with the
//!     outcome (`done` = the work landed, `free` = handed back
//!     unworked, which restores the `queue/queued` label).
//!   * All state lives in the comments. There is no database, no
//!     broker, and nothing to reconcile when the backend returns:
//!     the broker system re-derives queue state from labels and its
//!     own tables exactly as it already does after any drift.
//!
//! Why the marker carries the worker id: every fleet worker posts to
//! GitHub as the same account, so comment authorship cannot tell
//! workers apart. The marker is the only worker identity the lease
//! can be keyed on — which is also why plain (unmarked) comments do
//! NOT renew a claim, and why `--worker` is required, never
//! defaulted: two workers silently sharing an id would share a lease.
//!
//! Marker grammar (first line of the comment, exactly):
//!
//!   <!-- fleet-claim v1 kind=claim worker=<id> ts=<rfc3339-utc> -->
//!   <!-- fleet-claim v1 kind=heartbeat worker=<id> ts=<rfc3339-utc> -->
//!   <!-- fleet-claim v1 kind=release worker=<id> outcome=<done|free> ts=<rfc3339-utc> -->
//!
//! Exit codes mirror the rest of the client: 0 ok, 2 usage,
//! 3 the GitHub transport itself failed (gh missing/erroring),
//! 5 the verb ran but the claim state forbids the action (the
//! NO_CLAIM_EXIT convention from `claim`).

use std::process::{Command, exit};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

/// The fleet's working repos, in pilot-queue order (code first).
pub const REPOS: [&str; 5] = [
    "developing-today/code",
    "developing-today-forks/opencode",
    "developing-today-forks/openchamber",
    "dezren39/darwin-nix",
    "dezren39/fleet",
];

/// Short names workers actually type, mapped to the working repos.
const REPO_SHORTCUTS: [(&str, &str); 5] = [
    ("code", "developing-today/code"),
    ("opencode", "developing-today-forks/opencode"),
    ("openchamber", "developing-today-forks/openchamber"),
    ("nix", "dezren39/darwin-nix"),
    ("fleet", "dezren39/fleet"),
];

/// Default lease window in minutes (see the module doc for why 30).
pub const DEFAULT_LEASE_MIN: i64 = 30;

const MARKER_PREFIX: &str = "<!-- fleet-claim v1 ";
const MARKER_SUFFIX: &str = " -->";

const EXIT_USAGE: i32 = 2;
const EXIT_TRANSPORT: i32 = 3;
const EXIT_NO_CLAIM: i32 = 5;

fn die(code: i32, msg: &str) -> ! {
    eprintln!("repo-queue fallback: {msg}");
    exit(code);
}

fn print_json(v: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(v).unwrap_or_else(|_| "null".to_string())
    );
}

// ---------------------------------------------------------------------------
// Time: minimal RFC 3339 (UTC) format/parse, no external crates.
// Epoch <-> civil date uses Howard Hinnant's well-known algorithms.
// ---------------------------------------------------------------------------

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Days since 1970-01-01 -> (year, month, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// (year, month, day) -> days since 1970-01-01.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y_adj = if m <= 2 { y - 1 } else { y };
    let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
    let yoe = (y_adj - era * 400) as u64; // [0, 399]
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64; // [0, 11]
    let doy = (153 * mp + 2) / 5 + (d as u64) - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe as i64 - 719_468
}

/// Format an epoch second as RFC 3339 UTC (`2026-11-06T19:08:00Z`).
fn format_ts(epoch: i64) -> String {
    let days = epoch.div_euclid(86_400);
    let secs = epoch.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Parse an RFC 3339 timestamp (`...Z` or with a numeric offset,
/// optional fractional seconds) to epoch seconds.
fn parse_ts(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = s.split_once('T').or_else(|| s.split_once(' '))?;
    let mut dp = date.split('-');
    let year: i64 = dp.next()?.parse().ok()?;
    let month: u32 = dp.next()?.parse().ok()?;
    let day: u32 = dp.next()?.parse().ok()?;
    // The day must exist in the given month and year (#6667):
    // days_from_civil silently normalizes impossible dates
    // (2026-02-31 -> 2026-03-03), which would shift a marker's
    // authoritative ts — and with it all lease math — by days.
    // Oracle: the Gregorian calendar (month lengths; February has
    // 29 days only in leap years: divisible by 4, except centuries
    // not divisible by 400).
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
    let (time, offset_s) = if let Some(t) = rest.strip_suffix('Z') {
        (t, 0i64)
    } else if let Some(idx) = rest.rfind(['+', '-']) {
        if idx == 0 {
            return None;
        }
        let (t, off) = rest.split_at(idx);
        let sign: i64 = if rest.as_bytes()[idx] == b'+' { 1 } else { -1 };
        let mut op = off[1..].split(':');
        let oh: i64 = op.next()?.parse().ok()?;
        let om: i64 = op.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        (t, sign * (oh * 3600 + om * 60))
    } else {
        (rest, 0)
    };
    let time = time.split('.').next()?; // drop fractional seconds
    let mut tp = time.split(':');
    let hh: i64 = tp.next()?.parse().ok()?;
    let mm: i64 = tp.next()?.parse().ok()?;
    let ss: i64 = tp.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    if !(0..24).contains(&hh) || !(0..60).contains(&mm) || !(0..61).contains(&ss) {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hh * 3600 + mm * 60 + ss - offset_s)
}

// ---------------------------------------------------------------------------
// Claim markers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum MarkerKind {
    Claim,
    Heartbeat,
    Release,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Marker {
    kind: MarkerKind,
    worker: String,
    outcome: Option<String>, // release only: "done" | "free"
    ts: i64,
}

/// Worker ids are single tokens from a conservative charset: the id
/// is embedded in an HTML comment and parsed back by whitespace, so
/// anything else would make the marker ambiguous.
fn valid_worker(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn marker_line(kind: &MarkerKind, worker: &str, outcome: Option<&str>, ts: i64) -> String {
    let kind_s = match kind {
        MarkerKind::Claim => "claim",
        MarkerKind::Heartbeat => "heartbeat",
        MarkerKind::Release => "release",
    };
    let outcome_s = match outcome {
        Some(o) => format!(" outcome={o}"),
        None => String::new(),
    };
    format!(
        "{MARKER_PREFIX}kind={kind_s} worker={worker}{outcome_s} ts={} -->",
        format_ts(ts)
    )
}

/// Find the first well-formed fleet-claim marker in a comment body.
/// A marker is a whole line of exactly the marker shape; anything
/// malformed (unknown kind, missing fields, unparseable timestamp,
/// bad worker charset) is not a marker at all — quoted protocol text
/// and near-misses must never move claim state.
fn parse_marker(body: &str) -> Option<Marker> {
    for line in body.lines() {
        let line = line.trim();
        let Some(inner) = line
            .strip_prefix(MARKER_PREFIX)
            .and_then(|rest| rest.strip_suffix(MARKER_SUFFIX))
        else {
            continue;
        };
        let mut kind = None;
        let mut worker = None;
        let mut outcome = None;
        let mut ts = None;
        let mut ok = true;
        for field in inner.split_whitespace() {
            match field.split_once('=') {
                Some(("kind", "claim")) => kind = Some(MarkerKind::Claim),
                Some(("kind", "heartbeat")) => kind = Some(MarkerKind::Heartbeat),
                Some(("kind", "release")) => kind = Some(MarkerKind::Release),
                Some(("worker", w)) if valid_worker(w) => worker = Some(w.to_string()),
                Some(("outcome", o @ ("done" | "free"))) => outcome = Some(o.to_string()),
                Some(("ts", t)) => match parse_ts(t) {
                    Some(v) => ts = Some(v),
                    None => ok = false,
                },
                _ => ok = false,
            }
        }
        let (Some(kind), Some(worker), Some(ts)) = (kind, worker, ts) else {
            continue;
        };
        if !ok {
            continue;
        }
        // outcome is meaningful (and required) only on release.
        if kind == MarkerKind::Release && outcome.is_none() {
            continue;
        }
        if kind != MarkerKind::Release && outcome.is_some() {
            continue;
        }
        return Some(Marker {
            kind,
            worker,
            outcome,
            ts,
        });
    }
    None
}

// ---------------------------------------------------------------------------
// Claim state: a fold over the issue's marker events, in comment
// order. This fold is the protocol's model — the hand-computed cases
// in the tests are its oracle.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClaimState {
    /// No claim events at all.
    Free,
    /// The last claim ended with a release.
    Released {
        worker: String,
        outcome: String,
        ts: i64,
    },
    /// A holder whose last marker is inside the lease window.
    Live {
        worker: String,
        last_activity: i64,
        age_seconds: i64,
    },
    /// A holder whose last marker is older than the lease window:
    /// any worker may claim, superseding.
    Stale {
        worker: String,
        last_activity: i64,
        age_seconds: i64,
    },
    /// A second worker claimed while the holder's claim was live.
    /// Hands off for everyone; a human or the fleet monitor resolves.
    Contested {
        holder: String,
        challenger: String,
        ts: i64,
    },
}

impl ClaimState {
    fn name(&self) -> &'static str {
        match self {
            ClaimState::Free => "free",
            ClaimState::Released { .. } => "released",
            ClaimState::Live { .. } => "live",
            ClaimState::Stale { .. } => "stale",
            ClaimState::Contested { .. } => "contested",
        }
    }

    /// Claimable = a new worker may claim right now.
    fn claimable(&self) -> bool {
        matches!(
            self,
            ClaimState::Free | ClaimState::Released { .. } | ClaimState::Stale { .. }
        )
    }

    fn holder(&self) -> Option<&str> {
        match self {
            ClaimState::Live { worker, .. }
            | ClaimState::Stale { worker, .. }
            | ClaimState::Contested { holder: worker, .. } => Some(worker),
            _ => None,
        }
    }

    fn to_json(&self) -> Value {
        match self {
            ClaimState::Free => json!({"state": "free", "claimable": true}),
            ClaimState::Released {
                worker,
                outcome,
                ts,
            } => json!({
                "state": "released", "claimable": true,
                "worker": worker, "outcome": outcome,
                "released_at": format_ts(*ts),
            }),
            ClaimState::Live {
                worker,
                last_activity,
                age_seconds,
            }
            | ClaimState::Stale {
                worker,
                last_activity,
                age_seconds,
            } => json!({
                "state": self.name(), "claimable": self.claimable(),
                "worker": worker,
                "last_activity": format_ts(*last_activity),
                "age_seconds": age_seconds,
            }),
            ClaimState::Contested {
                holder,
                challenger,
                ts,
            } => json!({
                "state": "contested", "claimable": false,
                "worker": holder, "challenger": challenger,
                "contested_at": format_ts(*ts),
            }),
        }
    }
}

/// Fold marker events (chronological) into the claim state at `now`.
/// A claim posted while another worker's claim is still inside its
/// lease at that claim's own timestamp contests the issue instead of
/// taking it — the tool never posts such a claim, so a contested
/// state means someone bypassed the tool, and the safe answer is
/// hands-off.
fn fold_claim(events: &[Marker], now: i64, lease_seconds: i64) -> ClaimState {
    let mut holder: Option<(String, i64)> = None; // (worker, last activity)
    let mut released: Option<(String, String, i64)> = None;
    let mut contested: Option<(String, String, i64)> = None;
    for ev in events {
        match ev.kind {
            MarkerKind::Claim => {
                released = None;
                match &holder {
                    None => holder = Some((ev.worker.clone(), ev.ts)),
                    Some((w, _)) if *w == ev.worker => {
                        holder = Some((ev.worker.clone(), ev.ts));
                    }
                    Some((w, activity)) => {
                        if ev.ts - activity > lease_seconds {
                            // The previous claim was already stale
                            // when this claim landed: clean supersede.
                            holder = Some((ev.worker.clone(), ev.ts));
                            contested = None;
                        } else if contested.is_none() {
                            contested = Some((w.clone(), ev.worker.clone(), ev.ts));
                        }
                    }
                }
            }
            MarkerKind::Heartbeat => {
                if let Some((w, _)) = &holder {
                    if *w == ev.worker {
                        holder = Some((ev.worker.clone(), ev.ts));
                    }
                }
            }
            MarkerKind::Release => {
                if let Some((w, _)) = &holder {
                    if *w == ev.worker {
                        holder = None;
                        contested = None;
                        released = Some((
                            ev.worker.clone(),
                            ev.outcome.clone().unwrap_or_else(|| "done".to_string()),
                            ev.ts,
                        ));
                    }
                }
            }
        }
    }
    if let Some((holder_w, challenger, ts)) = contested {
        if holder.is_some() {
            return ClaimState::Contested {
                holder: holder_w,
                challenger,
                ts,
            };
        }
    }
    match holder {
        Some((worker, last_activity)) => {
            let age = now - last_activity;
            if age <= lease_seconds {
                ClaimState::Live {
                    worker,
                    last_activity,
                    age_seconds: age,
                }
            } else {
                ClaimState::Stale {
                    worker,
                    last_activity,
                    age_seconds: age,
                }
            }
        }
        None => match released {
            Some((worker, outcome, ts)) => ClaimState::Released {
                worker,
                outcome,
                ts,
            },
            None => ClaimState::Free,
        },
    }
}

// ---------------------------------------------------------------------------
// GitHub transport: the `gh` CLI only (fleet rule, Drew 2026-11-02).
// ---------------------------------------------------------------------------

fn gh_capture(args: &[String]) -> Result<String, String> {
    let out = Command::new("gh")
        .args(args)
        .output()
        .map_err(|e| format!("cannot run gh: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(format!(
            "gh {} failed: {}",
            args.first().cloned().unwrap_or_default(),
            stderr
        ))
    }
}

fn gh_json(args: &[String]) -> Result<Value, String> {
    let text = gh_capture(args)?;
    serde_json::from_str(&text).map_err(|e| format!("gh returned unparseable JSON: {e}"))
}

/// Marker events on one issue, in the order GitHub returns comments
/// (chronological). The marker's own embedded `ts` is authoritative —
/// comment metadata is never consulted for lease math.
fn fetch_markers(repo: &str, number: i64) -> Result<Vec<Marker>, String> {
    let mut markers = Vec::new();
    for page in 1..=10 {
        let path = format!("repos/{repo}/issues/{number}/comments?per_page=100&page={page}");
        let page_json = gh_json(&["api".into(), path])?;
        let Some(comments) = page_json.as_array() else {
            return Err("comments response was not an array".to_string());
        };
        let count = comments.len();
        for c in comments {
            if let Some(body) = c.get("body").and_then(|b| b.as_str()) {
                if let Some(m) = parse_marker(body) {
                    markers.push(m);
                }
            }
        }
        if count < 100 {
            break;
        }
    }
    Ok(markers)
}

fn post_comment(repo: &str, number: i64, body: &str) -> Result<String, String> {
    let out = gh_capture(&[
        "issue".into(),
        "comment".into(),
        number.to_string(),
        "--repo".into(),
        repo.into(),
        "--body".into(),
        body.into(),
    ])?;
    Ok(out.trim().to_string())
}

/// Best-effort label sync, mirroring the worker instruction's §9
/// label discipline for the transitions fallback mode owns. Label
/// failures (a repo missing part of the taxonomy) are reported as
/// warnings, never fatal: the comments are the state, labels are a
/// view of it.
fn edit_labels(repo: &str, number: i64, add: &[&str], remove: &[&str], warnings: &mut Vec<String>) {
    if add.is_empty() && remove.is_empty() {
        return;
    }
    let mut args: Vec<String> = vec![
        "issue".into(),
        "edit".into(),
        number.to_string(),
        "--repo".into(),
        repo.into(),
    ];
    for l in add {
        args.push("--add-label".into());
        args.push(l.to_string());
    }
    for l in remove {
        args.push("--remove-label".into());
        args.push(l.to_string());
    }
    if let Err(e) = gh_capture(&args) {
        warnings.push(format!("label sync skipped: {e}"));
    }
}

/// Open issues carrying one queue label in one repo. A repo where the
/// label does not exist (or the query otherwise fails) yields an
/// empty list plus a note — scan is a sweep, not a gate.
fn list_labeled_issues(repo: &str, label: &str, limit: i64, notes: &mut Vec<String>) -> Vec<Value> {
    let args: Vec<String> = vec![
        "issue".into(),
        "list".into(),
        "--repo".into(),
        repo.into(),
        "--state".into(),
        "open".into(),
        "--label".into(),
        label.into(),
        "--limit".into(),
        limit.to_string(),
        "--json".into(),
        "number,title,url,labels".into(),
    ];
    match gh_json(&args) {
        Ok(Value::Array(rows)) => rows,
        Ok(_) => {
            notes.push(format!("{repo}: unexpected list shape for label {label}"));
            Vec::new()
        }
        Err(e) => {
            notes.push(format!("{repo}: label {label} not scanned ({e})"));
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------
// Arguments (mirrors the client parser's rules: duplicate or unknown
// flags are usage errors, never silent last-wins).
// ---------------------------------------------------------------------------

struct FbArgs {
    pos: Vec<String>,
    opts: Map<String, Value>,
}

fn parse_fb_args(verb: &str, rest: &[String], flags: &[&str]) -> FbArgs {
    let mut pos = Vec::new();
    let mut opts = Map::new();
    let mut i = 0;
    while i < rest.len() {
        let a = &rest[i];
        if let Some(long) = a.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((n, v)) => (n.to_string(), Some(v.to_string())),
                None => (long.to_string(), None),
            };
            if !flags.contains(&name.as_str()) {
                die(
                    EXIT_USAGE,
                    &format!("fallback {verb}: unrecognized argument: --{name}"),
                );
            }
            if opts.contains_key(&name) {
                die(
                    EXIT_USAGE,
                    &format!("fallback {verb}: argument --{name}: duplicate flag"),
                );
            }
            if let Some(v) = inline {
                opts.insert(name, Value::String(v));
                i += 1;
            } else if i + 1 < rest.len() {
                opts.insert(name, Value::String(rest[i + 1].clone()));
                i += 2;
            } else {
                die(
                    EXIT_USAGE,
                    &format!("fallback {verb}: argument --{name}: expected one argument"),
                );
            }
        } else {
            pos.push(a.clone());
            i += 1;
        }
    }
    FbArgs { pos, opts }
}

impl FbArgs {
    fn get(&self, name: &str) -> Option<String> {
        self.opts
            .get(name)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    fn get_i64(&self, verb: &str, name: &str) -> Option<i64> {
        match self.get(name) {
            None => None,
            Some(s) => match s.parse::<i64>() {
                Ok(n) => Some(n),
                Err(_) => die(
                    EXIT_USAGE,
                    &format!("fallback {verb}: argument --{name}: invalid int value {s:?}"),
                ),
            },
        }
    }
}

fn resolve_repo(raw: &str) -> String {
    for (short, full) in REPO_SHORTCUTS {
        if raw == short {
            return full.to_string();
        }
    }
    raw.to_string()
}

fn repo_and_issue(verb: &str, args: &FbArgs) -> (String, i64) {
    if args.pos.len() != 2 {
        die(
            EXIT_USAGE,
            &format!("usage: repo-queue fallback {verb} <repo> <issue-number> [flags]"),
        );
    }
    let repo = resolve_repo(&args.pos[0]);
    if !REPOS.contains(&repo.as_str()) && !repo.contains('/') {
        die(
            EXIT_USAGE,
            &format!(
                "fallback {verb}: unknown repo {:?} (want owner/repo or one of: code, opencode, openchamber, nix, fleet)",
                args.pos[0]
            ),
        );
    }
    let number: i64 = match args.pos[1].parse() {
        Ok(n) if n >= 1 => n,
        _ => die(
            EXIT_USAGE,
            &format!("fallback {verb}: invalid issue number {:?}", args.pos[1]),
        ),
    };
    (repo, number)
}

fn lease_seconds(verb: &str, args: &FbArgs) -> i64 {
    if let Some(m) = args.get_i64(verb, "lease-min") {
        if m < 1 {
            die(
                EXIT_USAGE,
                &format!("fallback {verb}: --lease-min must be >= 1"),
            );
        }
        return m * 60;
    }
    if let Ok(v) = std::env::var("FLEET_QUEUE_FALLBACK_LEASE_MIN") {
        if let Ok(m) = v.trim().parse::<i64>() {
            if m >= 1 {
                return m * 60;
            }
        }
    }
    DEFAULT_LEASE_MIN * 60
}

fn worker_id(verb: &str, args: &FbArgs) -> String {
    let w = args
        .get("worker")
        .or_else(|| std::env::var("FLEET_FALLBACK_WORKER").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match w {
        Some(w) if valid_worker(&w) => w,
        Some(w) => die(
            EXIT_USAGE,
            &format!(
                "fallback {verb}: worker id {w:?} is not a valid marker token (letters, digits, '.', '_', '-', max 64)"
            ),
        ),
        None => die(
            EXIT_USAGE,
            &format!(
                "fallback {verb}: --worker <id> is required (or set FLEET_FALLBACK_WORKER): the claim marker carries your worker id, and two workers sharing an id would share one lease"
            ),
        ),
    }
}

fn issue_url(repo: &str, number: i64) -> String {
    format!("https://github.com/{repo}/issues/{number}")
}

// ---------------------------------------------------------------------------
// Verbs
// ---------------------------------------------------------------------------

fn cmd_scan(rest: &[String]) -> i32 {
    let args = parse_fb_args("scan", rest, &["repo", "limit", "lease-min"]);
    if !args.pos.is_empty() {
        die(
            EXIT_USAGE,
            "usage: repo-queue fallback scan [--repo <repo>]... [--limit <n>] [--lease-min <m>]",
        );
    }
    let lease = lease_seconds("scan", &args);
    let limit = args.get_i64("scan", "limit").unwrap_or(15);
    if !(1..=100).contains(&limit) {
        die(EXIT_USAGE, "fallback scan: --limit must be 1..=100");
    }
    let repos: Vec<String> = match args.get("repo") {
        Some(r) => vec![resolve_repo(&r)],
        None => REPOS.iter().map(|s| s.to_string()).collect(),
    };
    let now = now_epoch();
    let mut notes: Vec<String> = Vec::new();
    let mut items: Vec<Value> = Vec::new();
    for repo in &repos {
        let mut by_number: Map<String, Value> = Map::new();
        for label in ["queue/queued", "queue/claimed"] {
            for row in list_labeled_issues(repo, label, limit, &mut notes) {
                if let Some(n) = row.get("number").and_then(|v| v.as_i64()) {
                    by_number.entry(n.to_string()).or_insert(row);
                }
            }
        }
        let mut numbers: Vec<i64> = by_number.keys().filter_map(|k| k.parse().ok()).collect();
        numbers.sort_unstable();
        for n in numbers {
            let row = &by_number[&n.to_string()];
            let markers = match fetch_markers(repo, n) {
                Ok(m) => m,
                Err(e) => {
                    notes.push(format!("{repo}#{n}: comments not read ({e})"));
                    Vec::new()
                }
            };
            let state = fold_claim(&markers, now, lease);
            let labels: Vec<String> = row
                .get("labels")
                .and_then(|l| l.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|l| {
                            l.get("name")
                                .and_then(|x| x.as_str())
                                .map(|s| s.to_string())
                        })
                        .collect()
                })
                .unwrap_or_default();
            items.push(json!({
                "repo": repo,
                "number": n,
                "title": row.get("title").cloned().unwrap_or(Value::Null),
                "url": issue_url(repo, n),
                "labels": labels,
                "claim": state.to_json(),
            }));
        }
    }
    // Claimable work first, then live claims, then contested; stable
    // by repo order and issue number within each group.
    let rank = |it: &Value| match it["claim"]["state"].as_str() {
        Some("free") | Some("released") | Some("stale") => 0,
        Some("live") => 1,
        _ => 2,
    };
    items.sort_by_key(|it| rank(it));
    let claimable = items.iter().filter(|it| rank(it) == 0).count();
    print_json(&json!({
        "mode": "fallback",
        "generated_at": format_ts(now),
        "lease_seconds": lease,
        "items": items,
        "summary": {"total": items.len(), "claimable": claimable},
        "notes": notes,
    }));
    0
}

fn cmd_status(rest: &[String]) -> i32 {
    let args = parse_fb_args("status", rest, &["lease-min"]);
    let (repo, number) = repo_and_issue("status", &args);
    let lease = lease_seconds("status", &args);
    let markers = fetch_markers(&repo, number).unwrap_or_else(|e| die(EXIT_TRANSPORT, &e));
    let state = fold_claim(&markers, now_epoch(), lease);
    print_json(&json!({
        "mode": "fallback",
        "repo": repo,
        "number": number,
        "url": issue_url(&repo, number),
        "lease_seconds": lease,
        "claim": state.to_json(),
    }));
    0
}

fn cmd_claim(rest: &[String]) -> i32 {
    let args = parse_fb_args("claim", rest, &["worker", "lease-min", "note"]);
    let (repo, number) = repo_and_issue("claim", &args);
    let lease = lease_seconds("claim", &args);
    let worker = worker_id("claim", &args);
    let markers = fetch_markers(&repo, number).unwrap_or_else(|e| die(EXIT_TRANSPORT, &e));
    let state = fold_claim(&markers, now_epoch(), lease);
    let mut superseded: Option<String> = None;
    match &state {
        ClaimState::Live { worker: w, .. } if *w != worker => {
            print_json(&json!({
                "claimed": false, "reason": "live_claim",
                "repo": repo, "number": number, "claim": state.to_json(),
            }));
            return EXIT_NO_CLAIM;
        }
        ClaimState::Contested { .. } => {
            print_json(&json!({
                "claimed": false, "reason": "contested",
                "repo": repo, "number": number, "claim": state.to_json(),
            }));
            return EXIT_NO_CLAIM;
        }
        ClaimState::Stale { worker: w, .. } => superseded = Some(w.clone()),
        _ => {}
    }
    let now = now_epoch();
    let mut body = format!(
        "{}\n\nPicked up by `{worker}` — working this now. (Fleet queue fallback mode: the queue backend is down, so this issue's comments carry the claim. The claim stays live while `{worker}` posts progress markers; after {} minutes of silence any other worker may pick this up.)",
        marker_line(&MarkerKind::Claim, &worker, None, now),
        lease / 60
    );
    if let Some(note) = args.get("note") {
        body.push_str(&format!("\n\n{note}"));
    }
    let comment_url =
        post_comment(&repo, number, &body).unwrap_or_else(|e| die(EXIT_TRANSPORT, &e));
    // Read back and confirm we actually hold the claim — two workers
    // racing the same stale issue is the exact failure this mode
    // exists to prevent, so a lost race is reported, not hidden.
    let markers = fetch_markers(&repo, number).unwrap_or_else(|e| die(EXIT_TRANSPORT, &e));
    let after = fold_claim(&markers, now_epoch(), lease);
    if after.holder() != Some(worker.as_str()) {
        print_json(&json!({
            "claimed": false, "reason": "race_lost",
            "repo": repo, "number": number, "claim": after.to_json(),
        }));
        return EXIT_NO_CLAIM;
    }
    let mut warnings = Vec::new();
    let contested_by = match &after {
        ClaimState::Contested { challenger, .. } => Some(challenger.clone()),
        _ => None,
    };
    edit_labels(
        &repo,
        number,
        &["queue/claimed", "work/assigned"],
        &["queue/queued"],
        &mut warnings,
    );
    print_json(&json!({
        "claimed": true,
        "repo": repo, "number": number, "url": issue_url(&repo, number),
        "worker": worker,
        "comment_url": comment_url,
        "superseded": superseded,
        "contested_by": contested_by,
        "lease_seconds": lease,
        "warnings": warnings,
    }));
    0
}

fn cmd_heartbeat(rest: &[String]) -> i32 {
    let args = parse_fb_args("heartbeat", rest, &["worker", "lease-min", "note"]);
    let (repo, number) = repo_and_issue("heartbeat", &args);
    let lease = lease_seconds("heartbeat", &args);
    let worker = worker_id("heartbeat", &args);
    let markers = fetch_markers(&repo, number).unwrap_or_else(|e| die(EXIT_TRANSPORT, &e));
    let state = fold_claim(&markers, now_epoch(), lease);
    match state.holder() {
        Some(w) if w == worker => {}
        Some(w) => {
            print_json(&json!({
                "heartbeat": false, "reason": "held_by_other",
                "repo": repo, "number": number, "holder": w, "claim": state.to_json(),
            }));
            return EXIT_NO_CLAIM;
        }
        None => {
            print_json(&json!({
                "heartbeat": false, "reason": "no_claim",
                "repo": repo, "number": number, "claim": state.to_json(),
            }));
            return EXIT_NO_CLAIM;
        }
    }
    if matches!(state, ClaimState::Contested { .. }) {
        print_json(&json!({
            "heartbeat": false, "reason": "contested",
            "repo": repo, "number": number, "claim": state.to_json(),
        }));
        return EXIT_NO_CLAIM;
    }
    let note = args
        .get("note")
        .unwrap_or_else(|| "still working".to_string());
    let body = format!(
        "{}\n\nProgress from `{worker}`: {note}",
        marker_line(&MarkerKind::Heartbeat, &worker, None, now_epoch())
    );
    let comment_url =
        post_comment(&repo, number, &body).unwrap_or_else(|e| die(EXIT_TRANSPORT, &e));
    print_json(&json!({
        "heartbeat": true,
        "repo": repo, "number": number, "worker": worker,
        "comment_url": comment_url, "lease_seconds": lease,
    }));
    0
}

fn cmd_release(rest: &[String]) -> i32 {
    let args = parse_fb_args(
        "release",
        rest,
        &["worker", "lease-min", "summary", "outcome"],
    );
    let (repo, number) = repo_and_issue("release", &args);
    let lease = lease_seconds("release", &args);
    let worker = worker_id("release", &args);
    let outcome = args.get("outcome").unwrap_or_else(|| "done".to_string());
    if outcome != "done" && outcome != "free" {
        die(EXIT_USAGE, "fallback release: --outcome must be done|free");
    }
    let summary = args.get("summary").unwrap_or_else(|| {
        die(
            EXIT_USAGE,
            "fallback release: --summary is required (what landed, with PR links for outcome=done)",
        )
    });
    let markers = fetch_markers(&repo, number).unwrap_or_else(|e| die(EXIT_TRANSPORT, &e));
    let state = fold_claim(&markers, now_epoch(), lease);
    match state.holder() {
        Some(w) if w == worker => {}
        Some(w) => {
            print_json(&json!({
                "released": false, "reason": "held_by_other",
                "repo": repo, "number": number, "holder": w, "claim": state.to_json(),
            }));
            return EXIT_NO_CLAIM;
        }
        None => {
            print_json(&json!({
                "released": false, "reason": "no_claim",
                "repo": repo, "number": number, "claim": state.to_json(),
            }));
            return EXIT_NO_CLAIM;
        }
    }
    let body = format!(
        "{}\n\nReleased by `{worker}` (outcome: {outcome}). {summary}",
        marker_line(&MarkerKind::Release, &worker, Some(&outcome), now_epoch())
    );
    let comment_url =
        post_comment(&repo, number, &body).unwrap_or_else(|e| die(EXIT_TRANSPORT, &e));
    let mut warnings = Vec::new();
    if outcome == "free" {
        edit_labels(
            &repo,
            number,
            &["queue/queued"],
            &["queue/claimed", "work/assigned"],
            &mut warnings,
        );
    } else {
        edit_labels(
            &repo,
            number,
            &[],
            &["queue/claimed", "work/assigned"],
            &mut warnings,
        );
    }
    print_json(&json!({
        "released": true,
        "repo": repo, "number": number, "worker": worker,
        "outcome": outcome, "comment_url": comment_url,
        "warnings": warnings,
    }));
    0
}

pub fn run(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("usage: repo-queue fallback <scan|status|claim|heartbeat|release> ...");
        return EXIT_USAGE;
    }
    let sub = args[0].as_str();
    let rest = &args[1..];
    match sub {
        "scan" => cmd_scan(rest),
        "status" => cmd_status(rest),
        "claim" => cmd_claim(rest),
        "heartbeat" => cmd_heartbeat(rest),
        "release" => cmd_release(rest),
        _ => {
            eprintln!("repo-queue fallback: unknown command {sub:?}");
            EXIT_USAGE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const LEASE: i64 = 1800; // 30 minutes
    const T0: i64 = 1_762_476_000; // arbitrary fixed anchor for fold cases

    fn ev(kind: MarkerKind, worker: &str, ts: i64) -> Marker {
        Marker {
            kind,
            worker: worker.to_string(),
            outcome: None,
            ts,
        }
    }

    fn rel(worker: &str, outcome: &str, ts: i64) -> Marker {
        Marker {
            kind: MarkerKind::Release,
            worker: worker.to_string(),
            outcome: Some(outcome.to_string()),
            ts,
        }
    }

    #[test]
    fn ts_round_trip() {
        for epoch in [
            0,
            86_400,
            951_782_400,   // 2000-02-29 (leap day)
            1_762_476_000, // 2025-11-06
            1_767_225_600, // 2026-01-01
            4_102_444_800, // 2100-01-01
            -1,            // 1969-12-31T23:59:59Z
        ] {
            assert_eq!(parse_ts(&format_ts(epoch)), Some(epoch), "epoch {epoch}");
        }
        assert_eq!(format_ts(0), "1970-01-01T00:00:00Z");
        assert_eq!(parse_ts("2026-11-06T19:08:00Z"), Some(1_793_992_080));
        // Offsets and fractional seconds parse to the same instant.
        assert_eq!(parse_ts("2026-11-06T13:08:00-06:00"), Some(1_793_992_080));
        assert_eq!(parse_ts("2026-11-06T19:08:00.500Z"), Some(1_793_992_080));
        assert_eq!(parse_ts("not a timestamp"), None);
        assert_eq!(parse_ts("2026-13-06T19:08:00Z"), None);
    }

    #[test]
    fn parse_ts_rejects_impossible_dates() {
        // #6667: days that do not exist in their month/year must be
        // rejected, not normalized into the next month. Oracle: the
        // Gregorian calendar, as in the ts_round_trip controls below.
        for s in [
            "2026-02-31T00:00:00Z", // February never has 31 days
            "2026-02-30T00:00:00Z",
            "2026-02-29T00:00:00Z", // 2026 is not a leap year
            "2100-02-29T00:00:00Z", // century not divisible by 400
            "2026-04-31T00:00:00Z", // April has 30 days
            "2026-06-31T00:00:00Z",
            "2026-09-31T00:00:00Z",
            "2026-11-31T00:00:00Z",
            "2026-01-00T00:00:00Z", // day 0 exists in no month
            "2026-01-32T00:00:00Z",
        ] {
            assert_eq!(parse_ts(s), None, "{s} must not parse");
        }
        // Controls: real month-end and leap dates still parse.
        assert_eq!(parse_ts("2024-02-29T00:00:00Z"), Some(1_709_164_800));
        assert_eq!(parse_ts("2000-02-29T00:00:00Z"), Some(951_782_400));
        assert_eq!(parse_ts("2026-02-28T00:00:00Z"), Some(1_772_236_800));
        assert_eq!(parse_ts("2026-04-30T00:00:00Z"), Some(1_777_507_200));
        assert_eq!(parse_ts("2026-01-31T00:00:00Z"), Some(1_769_817_600));
    }

    #[test]
    fn marker_round_trip() {
        for (kind, outcome) in [
            (MarkerKind::Claim, None),
            (MarkerKind::Heartbeat, None),
            (MarkerKind::Release, Some("done")),
            (MarkerKind::Release, Some("free")),
        ] {
            let line = marker_line(&kind, "repo-worker-code", outcome, T0);
            let body = format!("{line}\n\nHuman text follows.");
            let parsed = parse_marker(&body).expect("own marker must parse");
            assert_eq!(parsed.kind, kind);
            assert_eq!(parsed.worker, "repo-worker-code");
            assert_eq!(parsed.outcome, outcome.map(|s| s.to_string()));
            assert_eq!(parsed.ts, T0);
        }
    }

    #[test]
    fn marker_rejects_near_misses() {
        // Wrong version.
        assert!(
            parse_marker("<!-- fleet-claim v2 kind=claim worker=a ts=2026-11-06T19:08:00Z -->")
                .is_none()
        );
        // Missing worker.
        assert!(
            parse_marker("<!-- fleet-claim v1 kind=claim ts=2026-11-06T19:08:00Z -->").is_none()
        );
        // Bad timestamp.
        assert!(parse_marker("<!-- fleet-claim v1 kind=claim worker=a ts=soon -->").is_none());
        // Release without outcome.
        assert!(
            parse_marker("<!-- fleet-claim v1 kind=release worker=a ts=2026-11-06T19:08:00Z -->")
                .is_none()
        );
        // Outcome on a non-release.
        assert!(
            parse_marker(
                "<!-- fleet-claim v1 kind=claim worker=a outcome=done ts=2026-11-06T19:08:00Z -->"
            )
            .is_none()
        );
        // Marker embedded in prose is not a whole line.
        assert!(
            parse_marker(
                "see <!-- fleet-claim v1 kind=claim worker=a ts=2026-11-06T19:08:00Z --> here"
            )
            .is_none()
        );
        // No marker at all.
        assert!(parse_marker("an ordinary progress comment").is_none());
    }

    #[test]
    fn fold_free_and_released() {
        assert_eq!(fold_claim(&[], T0, LEASE), ClaimState::Free);
        let events = vec![ev(MarkerKind::Claim, "a", T0), rel("a", "done", T0 + 60)];
        assert_eq!(
            fold_claim(&events, T0 + 120, LEASE),
            ClaimState::Released {
                worker: "a".to_string(),
                outcome: "done".to_string(),
                ts: T0 + 60
            }
        );
    }

    #[test]
    fn fold_live_then_stale_and_heartbeat_renews() {
        let claim = vec![ev(MarkerKind::Claim, "a", T0)];
        assert!(matches!(
            fold_claim(&claim, T0 + LEASE, LEASE),
            ClaimState::Live { .. }
        ));
        assert!(matches!(
            fold_claim(&claim, T0 + LEASE + 1, LEASE),
            ClaimState::Stale { .. }
        ));
        let renewed = vec![
            ev(MarkerKind::Claim, "a", T0),
            ev(MarkerKind::Heartbeat, "a", T0 + LEASE + 500),
        ];
        match fold_claim(&renewed, T0 + LEASE + 600, LEASE) {
            ClaimState::Live { worker, .. } => assert_eq!(worker, "a"),
            other => panic!("expected live, got {other:?}"),
        }
    }

    #[test]
    fn fold_stale_supersede_and_live_contest() {
        // B claims after A's claim went stale: clean takeover.
        let supersede = vec![
            ev(MarkerKind::Claim, "a", T0),
            ev(MarkerKind::Claim, "b", T0 + LEASE + 60),
        ];
        match fold_claim(&supersede, T0 + LEASE + 120, LEASE) {
            ClaimState::Live { worker, .. } => assert_eq!(worker, "b"),
            other => panic!("expected live b, got {other:?}"),
        }
        // B claims while A is live: contested, A remains holder.
        let contest = vec![
            ev(MarkerKind::Claim, "a", T0),
            ev(MarkerKind::Claim, "b", T0 + 60),
        ];
        match fold_claim(&contest, T0 + 120, LEASE) {
            ClaimState::Contested {
                holder, challenger, ..
            } => {
                assert_eq!(holder, "a");
                assert_eq!(challenger, "b");
            }
            other => panic!("expected contested, got {other:?}"),
        }
        // A releases: the contest dies with the claim.
        let mut resolved = contest.clone();
        resolved.push(rel("a", "free", T0 + 180));
        assert_eq!(
            fold_claim(&resolved, T0 + 240, LEASE),
            ClaimState::Released {
                worker: "a".to_string(),
                outcome: "free".to_string(),
                ts: T0 + 180
            }
        );
    }

    #[test]
    fn fold_ignores_non_holder_markers() {
        let events = vec![
            ev(MarkerKind::Claim, "a", T0),
            ev(MarkerKind::Heartbeat, "b", T0 + LEASE + 500), // not the holder
            rel("b", "done", T0 + LEASE + 600),               // not the holder
        ];
        // A's claim went stale; B's stray markers changed nothing.
        match fold_claim(&events, T0 + LEASE + 700, LEASE) {
            ClaimState::Stale { worker, .. } => assert_eq!(worker, "a"),
            other => panic!("expected stale a, got {other:?}"),
        }
    }

    proptest! {
        /// Invariants over arbitrary marker sequences (increasing
        /// timestamps, two workers): the fold never panics, is
        /// deterministic, and any live/stale holder is a worker that
        /// actually claimed.
        #[test]
        fn fold_invariants(
            kinds in prop::collection::vec(0u8..4, 0..24),
            gaps in prop::collection::vec(0i64..3600, 24),
        ) {
            let workers = ["a", "b"];
            let mut events = Vec::new();
            let mut ts = T0;
            for (i, k) in kinds.iter().enumerate() {
                ts += gaps[i % gaps.len().max(1)];
                let w = workers[i % 2];
                let ev = match k {
                    0 => ev(MarkerKind::Claim, w, ts),
                    1 => ev(MarkerKind::Heartbeat, w, ts),
                    2 => rel(w, "done", ts),
                    _ => rel(w, "free", ts),
                };
                events.push(ev);
            }
            let s1 = fold_claim(&events, ts + 60, LEASE);
            let s2 = fold_claim(&events, ts + 60, LEASE);
            prop_assert_eq!(&s1, &s2);
            if let Some(h) = s1.holder() {
                prop_assert!(workers.contains(&h));
                prop_assert!(events.iter().any(|e| e.kind == MarkerKind::Claim && e.worker == h));
            }
        }

        /// #6667 calendar invariant: parse_ts accepts a (year, month,
        /// day) triple iff that day actually exists. Oracle: the
        /// inverse pair days_from_civil/civil_from_days \u2014 a triple
        /// is a real date iff the round trip returns it unchanged
        /// (normalization moves impossible dates into the next
        /// month, so their round trip differs).
        #[test]
        fn parse_ts_accepts_exactly_real_dates(
            year in 1600i64..2400,
            month in 1u32..=12,
            day in 1u32..=31,
        ) {
            let s = format!("{year:04}-{month:02}-{day:02}T00:00:00Z");
            let real = civil_from_days(days_from_civil(year, month, day)) == (year, month, day);
            prop_assert_eq!(parse_ts(&s).is_some(), real, "{}", s);
            if real {
                prop_assert_eq!(parse_ts(&s), Some(days_from_civil(year, month, day) * 86_400));
            }
        }

        /// Round-trip property over arbitrary epochs (#6667
        /// acceptance: parse_ts(format_ts(e)) == Some(e)).
        #[test]
        fn ts_round_trip_any_epoch(epoch in -31_622_400i64..4_102_444_800) {
            prop_assert_eq!(parse_ts(&format_ts(epoch)), Some(epoch));
        }
    }
}
