//! GitHub-native fallback mode for the queue domain (Drew's design,
//! ordered 2026-11-06; tracking issue developing-today/code#6619's
//! outage is the motivating incident).
//!
//! When the queue backend (queue-broker → Turso fleet-queue) is
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
//!   * A worker COMPLETES with broker parity (#13023) by handing
//!     `complete` the disposition record `verify-disposition`
//!     produced for the issue: the record must pass the broker's
//!     offline gate (`crate::disposition::check`) locally before
//!     anything is posted — a rejection posts nothing and the
//!     claim stays live, exactly as a broker `complete` rejection
//!     leaves the task claimed. On a pass, `complete` posts the
//!     release-done marker with the summary and the full record
//!     in a `fleet-disposition` fenced block, so the record stays
//!     consumable from the marker stream when the broker returns
//!     (`disposition_record_in_comment`). The one broker-only
//!     check — a follow_up's task_id against the live queue
//!     (`check_successor_task`) — cannot run from a fallback
//!     seat; a record naming one is completed with that check
//!     flagged UNCHECKED in the comment and the verb output,
//!     never silently treated as checked.
//!   * A BLOCKED item -- its issue carries the `queue/blocked`
//!     label the broker-backed system maintains -- sits outside
//!     the scan sweep and cannot be claimed. A worker UNBLOCKS
//!     it with the fallback form of the broker's manual override
//!     (#12777): worker-scoped, a reason is required, the
//!     override is recorded as an `unblock` marker comment (the
//!     audit; the broker records the same override as an event),
//!     and the label transition `queue/blocked` -> `queue/queued`
//!     returns the item to the pool. An issue that is not
//!     blocked refuses the override, and so does one under a
//!     live or contested claim.
//!   * A claim posted while another worker's claim is live CONTESTS
//!     the issue: hands-off for everyone while the contest is
//!     younger than the lease. A contest is not permanent (#6668):
//!     it ages out once every challenger has been silent for a full
//!     lease window (a repeat claim refreshes that challenger's
//!     contest), and a release from a challenger clears that
//!     challenger's contest at once. The holder may heartbeat
//!     throughout; once
//!     a contest has aged out, the holder's last activity governs as
//!     usual — live if the holder kept heartbeating, stale (and
//!     claimable) if everyone went silent.
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
//!   <!-- fleet-claim v1 kind=unblock worker=<id> ts=<rfc3339-utc> -->
//!
//! Exit codes mirror the rest of the client: 0 ok, 2 usage,
//! 3 the GitHub transport itself failed (gh missing/erroring),
//! 5 the verb ran but the claim state forbids the action (the
//! NO_CLAIM_EXIT convention from `claim`), and 1 a `complete`
//! whose disposition the gate rejected or whose record could
//! not be read (verify-disposition's own gate-failure code).

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

/// Marker reads paginate issue comments at this page size, up to
/// MAX_MARKER_PAGES pages. The bound is a safety limit, not a silent
/// truncation point (#9044): a read that reaches it on a full page
/// errors instead of returning a partial marker set as complete.
const MARKER_PAGE_SIZE: usize = 100;
const MAX_MARKER_PAGES: usize = 10;

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
    Unblock,
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
        MarkerKind::Unblock => "unblock",
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
                Some(("kind", "unblock")) => kind = Some(MarkerKind::Unblock),
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
    /// Hands off for everyone while the contest is younger than the
    /// lease; after that the contest ages out (see `fold_claim`).
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
/// hands-off while the contest is young. A contest is not
/// permanent, though (#6668): it ages out one lease window after it
/// lands (the challenger has then posted no claim for a full
/// lease), and a release from the challenger clears it immediately.
/// Once a contest is gone the holder's last activity governs —
/// live if the holder kept heartbeating, stale and claimable if
/// everyone went silent.
fn fold_claim(events: &[Marker], now: i64, lease_seconds: i64) -> ClaimState {
    let mut holder: Option<(String, i64)> = None; // (worker, last activity)
    let mut released: Option<(String, String, i64)> = None;
    // Active challengers, in first-claim order: (worker, that
    // worker's LAST conflicting claim ts). A repeat claim refreshes
    // the challenger's entry — a contest only ages out once every
    // challenger has been silent for a full lease (Codex review on
    // PR #6676).
    let mut challengers: Vec<(String, i64)> = Vec::new();
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
                            challengers.clear();
                        } else if let Some(entry) =
                            challengers.iter_mut().find(|(cw, _)| *cw == ev.worker)
                        {
                            // A repeat claim from the same challenger
                            // refreshes its contest activity.
                            entry.1 = ev.ts;
                        } else {
                            challengers.push((ev.worker.clone(), ev.ts));
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
            MarkerKind::Unblock => {
                // Audit only (#12777): the override is recorded in
                // the comment stream, but blocked state lives in
                // the issue's labels, not in this fold -- an
                // unblock marker neither takes nor releases a
                // claim.
            }
            MarkerKind::Release => {
                if let Some((w, _)) = &holder {
                    if *w == ev.worker {
                        holder = None;
                        challengers.clear();
                        released = Some((
                            ev.worker.clone(),
                            ev.outcome.clone().unwrap_or_else(|| "done".to_string()),
                            ev.ts,
                        ));
                    }
                }
                // A release from a challenger withdraws only that
                // challenger's contest (#6668 Case 3; Codex review
                // on PR #6676): other challengers' contests stand.
                // Releases from anyone else change nothing.
                challengers.retain(|(cw, _)| *cw != ev.worker);
            }
        }
    }
    // A contest stands while at least one challenger has claimed
    // within the lease window. Once every challenger has been
    // silent for a full lease the contest has aged out and no
    // longer bricks the issue (#6668): fall through and let the
    // holder's last activity govern — Live if the holder kept
    // heartbeating, Stale (claimable) if everyone went silent.
    if let Some((worker, _)) = &holder {
        if let Some((challenger, ts)) = challengers
            .iter()
            .find(|(_, cts)| now - cts <= lease_seconds)
        {
            return ClaimState::Contested {
                holder: worker.clone(),
                challenger: challenger.clone(),
                ts: *ts,
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

/// The most recent unblock marker in the stream, if any -- the
/// fallback audit trail for the manual override (#12777).
fn last_unblock(events: &[Marker]) -> Option<&Marker> {
    events.iter().rev().find(|m| m.kind == MarkerKind::Unblock)
}

/// Whether an issue's labels mark its task blocked.
fn is_blocked(labels: &[String]) -> bool {
    labels.iter().any(|l| l == "queue/blocked")
}

/// The fallback unblock guard (#12777), mirroring the broker's
/// verb_unblock: the override exists only for a blocked task, so
/// anything else refuses -- a closed issue, an issue that is not
/// blocked, or one whose claim is live or contested (a blocked
/// task is unclaimed by construction; a live claim means the
/// state on the ground disagrees with the label, and the safe
/// answer is hands-off). Returns the refusal reason, or None
/// when the override may proceed.
fn unblock_refusal(open: bool, labels: &[String], state: &ClaimState) -> Option<&'static str> {
    if !open {
        return Some("issue_closed");
    }
    if !is_blocked(labels) {
        return Some("not_blocked");
    }
    match state {
        ClaimState::Live { .. } => Some("live_claim"),
        ClaimState::Contested { .. } => Some("contested"),
        _ => None,
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
    collect_markers(|page| {
        let path = format!(
            "repos/{repo}/issues/{number}/comments?per_page={MARKER_PAGE_SIZE}&page={page}"
        );
        gh_json(&["api".into(), path])
    })
}

/// The page loop behind `fetch_markers`, with fetching injected so
/// tests can pin the bound behavior. Reads under the bound return
/// exactly the markers found, in page order. A full final page means
/// more comments may exist past the bound, so the read fails loudly
/// (#9044; design #6622 contract rule 3: truncation is never silent)
/// instead of presenting a partial marker set as complete -- a live
/// claim past the bound would otherwise read as absent.
fn collect_markers(
    mut fetch_page: impl FnMut(usize) -> Result<Value, String>,
) -> Result<Vec<Marker>, String> {
    let mut markers = Vec::new();
    for page in 1..=MAX_MARKER_PAGES {
        let page_json = fetch_page(page)?;
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
        if count < MARKER_PAGE_SIZE {
            return Ok(markers);
        }
    }
    Err(format!(
        "issue comments exceed the {MAX_MARKER_PAGES}-page marker read bound          ({} comments); refusing to return a truncated marker set",
        MAX_MARKER_PAGES * MARKER_PAGE_SIZE
    ))
}

/// One issue's open/closed state and label names, for the verbs
/// whose state partly lives in labels (unblock's guard, claim's
/// blocked refusal, status's blocked report).
fn fetch_issue(repo: &str, number: i64) -> Result<(bool, Vec<String>), String> {
    let v = gh_json(&["api".into(), format!("repos/{repo}/issues/{number}")])?;
    let open = v.get("state").and_then(|s| s.as_str()) == Some("open");
    let labels = v
        .get("labels")
        .and_then(|l| l.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|l| {
                    l.get("name")
                        .and_then(|n| n.as_str())
                        .map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    Ok((open, labels))
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

// ---------------------------------------------------------------------------
// Discoverable usage (#10838): per-verb `--help`/`-h`, and every
// argument-shape rejection carries the verb's usage — the same
// contract as the primary client's parse_verb_args.
// ---------------------------------------------------------------------------

/// Group usage for `repo-queue fallback --help`.
pub(crate) const GROUP_USAGE: &str = "\
usage: repo-queue fallback <scan|status|claim|heartbeat|release|complete|unblock> ...\n\
\n\
GitHub-backed queue used when the primary broker is down; issue\n\
comments carry the claims. Run `repo-queue fallback <sub> --help`\n\
for flags:\n\
  scan <repo> <issue-number>  (--repo/--limit/--lease-min live on scan)\n\
  status <repo> <issue-number>\n\
  claim <repo> <issue-number> --worker <id>\n\
  heartbeat <repo> <issue-number> --worker <id>\n\
  release <repo> <issue-number> --worker <id>\n\
  complete <repo> <issue-number> --worker <id>\n\
  unblock <repo> <issue-number> --worker <id> --reason <text>";

/// Per-verb usage for the fallback verbs.
pub(crate) fn usage(verb: &str) -> &'static str {
    match verb {
        "scan" => {
            "usage: repo-queue fallback scan [--repo <repo>] [--limit <1..100>] [--lease-min <minutes>]\n\nScans the fallback repos' labelled issues for live/stale claims."
        }
        "status" => {
            "usage: repo-queue fallback status <repo> <issue-number> [--lease-min <minutes>]\n\nShows the folded claim state for one fallback issue."
        }
        "claim" => {
            "usage: repo-queue fallback claim <repo> <issue-number> --worker <id> [--lease-min <minutes>] [--note <text>]\n\nClaims a fallback issue by posting a claim marker comment (races are detected on read-back)."
        }
        "heartbeat" => {
            "usage: repo-queue fallback heartbeat <repo> <issue-number> --worker <id> [--lease-min <minutes>] [--note <text>]\n\nRefreshes your fallback claim with a progress marker."
        }
        "release" => {
            "usage: repo-queue fallback release <repo> <issue-number> --worker <id> [--lease-min <minutes>] [--outcome done|free] [--summary <text>]\n\nReleases a fallback claim (done keeps the terminal state, free returns it to the pool)."
        }
        "complete" => {
            "usage: repo-queue fallback complete <repo> <issue-number> --worker <id> --summary <text> --result <path> [--pr <n[,n...]>] [--lease-min <minutes>]\n\nCompletes a fallback claim with broker parity (#13023): the disposition record at --result (the output of `repo-queue verify-disposition <issue> --repo <repo> --json`) must pass the broker's offline disposition gate for this issue before anything is posted; a rejection posts nothing, the claim stays live, and the verb exits 1. On a pass it posts the release-done marker with the summary and the full record in a fleet-disposition fenced block, so the record stays consumable from the marker stream when the broker returns. A follow_up naming a task_id is recorded but flagged unchecked \u{2014} only the broker can check it against the queue. Plain `release --outcome done` remains for handing back work that carries no disposition."
        }
        "unblock" => {
            "usage: repo-queue fallback unblock <repo> <issue-number> --worker <id> --reason <text>\n\nReleases a blocked fallback issue back to the claimable pool: posts an unblock marker recording the override and its reason, clears the queue/blocked label and restores queue/queued. Refuses an issue that is closed, not blocked, or under a live or contested claim."
        }
        _ => GROUP_USAGE,
    }
}

fn is_help_arg(s: &str) -> bool {
    matches!(s, "--help" | "-h" | "help")
}

/// True when `rest` asks for help: a help token that is not the value of a
/// flag. `claim o/r 5 --note help` records the note "help"; it is not a help
/// request. Every fallback flag takes a value, and the value rule mirrors
/// `parse_fb_args`: without an inline `=`, the next token is the value.
fn wants_help(rest: &[String], flags: &[&str]) -> bool {
    let mut i = 0;
    while i < rest.len() {
        let a = rest[i].as_str();
        if is_help_arg(a) {
            return true;
        }
        if let Some(long) = a.strip_prefix("--")
            && !long.contains('=')
            && flags.contains(&long)
            && i + 1 < rest.len()
        {
            i += 2;
            continue;
        }
        i += 1;
    }
    false
}

/// Die with a usage error (exit 2): the message plus the verb's
/// usage, so the error teaches the accepted flags.
fn usage_error(verb: &str, msg: &str) -> ! {
    die(EXIT_USAGE, &format!("{msg}\n{}", usage(verb)));
}

fn parse_fb_args(verb: &str, rest: &[String], flags: &[&str]) -> FbArgs {
    // Help wins over every other argument (#10838).
    if wants_help(rest, flags) {
        println!("{}", usage(verb));
        exit(0);
    }
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
                usage_error(
                    verb,
                    &format!("fallback {verb}: unrecognized argument: --{name}"),
                );
            }
            if opts.contains_key(&name) {
                usage_error(
                    verb,
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
                usage_error(
                    verb,
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
                Err(_) => usage_error(
                    verb,
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
        usage_error(
            verb,
            &format!(
                "fallback {verb}: expected <repo> <issue-number>, got {} positional argument(s)",
                args.pos.len()
            ),
        );
    }
    let repo = resolve_repo(&args.pos[0]);
    if !REPOS.contains(&repo.as_str()) && !repo.contains('/') {
        usage_error(
            verb,
            &format!(
                "fallback {verb}: unknown repo {:?} (want owner/repo or one of: code, opencode, openchamber, nix, fleet)",
                args.pos[0]
            ),
        );
    }
    let number: i64 = match args.pos[1].parse() {
        Ok(n) if n >= 1 => n,
        _ => usage_error(
            verb,
            &format!("fallback {verb}: invalid issue number {:?}", args.pos[1]),
        ),
    };
    (repo, number)
}

fn lease_seconds(verb: &str, args: &FbArgs) -> i64 {
    if let Some(m) = args.get_i64(verb, "lease-min") {
        if m < 1 {
            usage_error(verb, &format!("fallback {verb}: --lease-min must be >= 1"));
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
        Some(w) => usage_error(
            verb,
            &format!(
                "fallback {verb}: worker id {w:?} is not a valid marker token (letters, digits, '.', '_', '-', max 64)"
            ),
        ),
        None => usage_error(
            verb,
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
        usage_error(
            "scan",
            "fallback scan: takes no positional arguments (the sweep covers the fallback repos)",
        );
    }
    let lease = lease_seconds("scan", &args);
    let limit = args.get_i64("scan", "limit").unwrap_or(15);
    if !(1..=100).contains(&limit) {
        usage_error("scan", "fallback scan: --limit must be 1..=100");
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
        for label in ["queue/queued", "queue/claimed", "queue/blocked"] {
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
            let blocked = is_blocked(&labels);
            items.push(json!({
                "repo": repo,
                "number": n,
                "title": row.get("title").cloned().unwrap_or(Value::Null),
                "url": issue_url(repo, n),
                "labels": labels,
                "blocked": blocked,
                "claim": state.to_json(),
            }));
        }
    }
    // Claimable work first, then live claims, then contested and
    // blocked; stable by repo order and issue number within each
    // group. A blocked item's claim fold may read free -- the
    // label is the block -- so it never ranks as claimable here.
    let rank = |it: &Value| {
        if it["blocked"].as_bool() == Some(true) {
            return 2;
        }
        match it["claim"]["state"].as_str() {
            Some("free") | Some("released") | Some("stale") => 0,
            Some("live") => 1,
            _ => 2,
        }
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
    let (_, labels) = fetch_issue(&repo, number).unwrap_or_else(|e| die(EXIT_TRANSPORT, &e));
    print_json(&json!({
        "mode": "fallback",
        "repo": repo,
        "number": number,
        "url": issue_url(&repo, number),
        "lease_seconds": lease,
        "blocked": is_blocked(&labels),
        "last_unblock": match last_unblock(&markers) {
            Some(m) => json!({"worker": m.worker, "ts": format_ts(m.ts)}),
            None => Value::Null,
        },
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
    // A blocked issue is not claimable (#12777): the manual
    // override (`fallback unblock`) is the only door back, exactly
    // as the broker refuses a claim on a blocked task. The block
    // lives in the issue's labels, so read them before posting.
    let (_, labels) = fetch_issue(&repo, number).unwrap_or_else(|e| die(EXIT_TRANSPORT, &e));
    if is_blocked(&labels) {
        print_json(&json!({
            "claimed": false, "reason": "blocked",
            "repo": repo, "number": number, "claim": state.to_json(),
        }));
        return EXIT_NO_CLAIM;
    }
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
    // A contested state does not refuse the holder's heartbeat
    // (#6668): the holder check above has already established that
    // the caller is the holder, heartbeating is how it proves it is
    // alive, and cmd_release likewise has no contested check — the
    // two verbs treat Contested consistently.
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
        usage_error("release", "fallback release: --outcome must be done|free");
    }
    let summary = args.get("summary").unwrap_or_else(|| {
        usage_error(
            "release",
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

/// The info string tagging the fenced block that carries a
/// completion's disposition record in its comment.
const DISPOSITION_FENCE: &str = "fleet-disposition";

/// Validate a verify-disposition record for a fallback completion:
/// the broker's offline gate (`crate::disposition`), run locally.
/// Pure -- the unit tests are its oracle. Returns the record, its
/// verdict and `checked_at`, and the follow-up task id only the
/// broker can check against the live queue: fallback mode has no
/// queue, so that id comes back as *unchecked*, never as checked.
fn check_fallback_disposition(
    record_text: &str,
    issue: i64,
    extra_linked: &[i64],
    now: i64,
) -> Result<(Value, String, String, Option<i64>), String> {
    let value: Value = serde_json::from_str(record_text)
        .map_err(|e| format!("--result is not a JSON disposition record ({e})"))?;
    let record = crate::disposition::parse_record(&value)
        .map_err(|e| format!("disposition record is not well-formed ({e})"))?;
    crate::disposition::check(
        &record,
        Some(issue),
        extra_linked,
        now,
        crate::disposition::DEFAULT_STALENESS_SECONDS,
    )?;
    let task_id = record.follow_up.as_ref().and_then(|f| f.task_id);
    Ok((value, record.verdict.clone(), record.checked_at.clone(), task_id))
}

/// The comment body a fallback `complete` posts: the release-done
/// marker, the summary, and the disposition record in its fenced
/// block. Pure -- the round-trip test is its oracle.
fn complete_body(
    worker: &str,
    summary: &str,
    record: &Value,
    verdict: &str,
    checked_at: &str,
    follow_up_task: Option<i64>,
    ts: i64,
) -> String {
    let pretty = serde_json::to_string_pretty(record).unwrap_or_else(|_| record.to_string());
    let task_note = match follow_up_task {
        Some(id) => format!(
            " The record's follow_up names task {id}: only the broker can check it \
             against the queue, so it is recorded here UNCHECKED -- the broker \
             runs check_successor_task on it when it returns."
        ),
        None => String::new(),
    };
    format!(
        "{}\n\nCompleted by `{worker}` (outcome: done). {summary}\n\nDisposition: verdict `{verdict}`, checked_at {checked_at} -- the record below passed the broker's offline disposition gate at completion time.{task_note}\n\n```{DISPOSITION_FENCE}\n{pretty}\n```",
        marker_line(&MarkerKind::Release, worker, Some("done"), ts)
    )
}

/// Extract the disposition record a fallback `complete` recorded
/// in a comment body: the first fenced block tagged
/// `fleet-disposition`. This is the consumption shape for when
/// the broker returns -- the extracted value is the record
/// exactly as `verify-disposition` produced it, re-checkable
/// offline with `crate::disposition::parse_record`.
pub(crate) fn disposition_record_in_comment(body: &str) -> Option<Value> {
    let fence = format!("```{DISPOSITION_FENCE}");
    let mut in_block = false;
    let mut text = String::new();
    for line in body.lines() {
        let t = line.trim();
        if !in_block {
            if t == fence {
                in_block = true;
            }
            continue;
        }
        if t == "```" {
            return serde_json::from_str(&text).ok();
        }
        text.push_str(line);
        text.push('\n');
    }
    None
}

fn cmd_complete(rest: &[String]) -> i32 {
    let args = parse_fb_args(
        "complete",
        rest,
        &["worker", "lease-min", "summary", "result", "pr"],
    );
    let (repo, number) = repo_and_issue("complete", &args);
    let lease = lease_seconds("complete", &args);
    let worker = worker_id("complete", &args);
    let summary = args.get("summary").unwrap_or_else(|| {
        usage_error(
            "complete",
            "fallback complete: --summary is required (what landed, with PR links)",
        )
    });
    let result_path = args.get("result").unwrap_or_else(|| {
        usage_error(
            "complete",
            "fallback complete: --result <path> is required -- the disposition record from `repo-queue verify-disposition <issue> --repo <repo> --json`",
        )
    });
    // PRs the completion claims beyond the readback's own
    // discovery: the gate requires each to be in the record,
    // exactly as the broker's complete requires the record to
    // cover the completion's claimed PRs.
    let mut extra_linked: Vec<i64> = Vec::new();
    if let Some(raw) = args.get("pr") {
        for part in raw.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            match part.trim_start_matches('#').parse::<i64>() {
                Ok(n) => {
                    if !extra_linked.contains(&n) {
                        extra_linked.push(n);
                    }
                }
                Err(_) => usage_error(
                    "complete",
                    &format!("fallback complete: argument --pr: invalid PR number {part:?}"),
                ),
            }
        }
    }
    let markers = fetch_markers(&repo, number).unwrap_or_else(|e| die(EXIT_TRANSPORT, &e));
    let state = fold_claim(&markers, now_epoch(), lease);
    match state.holder() {
        Some(w) if w == worker => {}
        Some(w) => {
            print_json(&json!({
                "completed": false, "reason": "held_by_other",
                "repo": repo, "number": number, "holder": w, "claim": state.to_json(),
            }));
            return EXIT_NO_CLAIM;
        }
        None => {
            print_json(&json!({
                "completed": false, "reason": "no_claim",
                "repo": repo, "number": number, "claim": state.to_json(),
            }));
            return EXIT_NO_CLAIM;
        }
    }
    let record_text = match std::fs::read_to_string(&result_path) {
        Ok(t) => t,
        Err(e) => {
            print_json(&json!({
                "completed": false, "reason": "result_unreadable",
                "repo": repo, "number": number,
                "detail": format!("cannot read --result {result_path:?}: {e}"),
            }));
            return 1;
        }
    };
    // The gate runs before anything is posted: a rejected
    // disposition leaves the claim live, as a broker rejection
    // leaves the task claimed.
    let (record, verdict, checked_at, follow_up_task) =
        match check_fallback_disposition(&record_text, number, &extra_linked, now_epoch()) {
            Ok(v) => v,
            Err(e) => {
                print_json(&json!({
                    "completed": false, "reason": "disposition_rejected",
                    "repo": repo, "number": number, "detail": e,
                }));
                return 1;
            }
        };
    let body = complete_body(
        &worker,
        &summary,
        &record,
        &verdict,
        &checked_at,
        follow_up_task,
        now_epoch(),
    );
    let comment_url =
        post_comment(&repo, number, &body).unwrap_or_else(|e| die(EXIT_TRANSPORT, &e));
    let mut warnings = Vec::new();
    edit_labels(
        &repo,
        number,
        &[],
        &["queue/claimed", "work/assigned"],
        &mut warnings,
    );
    print_json(&json!({
        "completed": true,
        "repo": repo, "number": number, "worker": worker,
        "outcome": "done", "verdict": verdict, "checked_at": checked_at,
        "follow_up_task_unchecked": follow_up_task,
        "comment_url": comment_url,
        "warnings": warnings,
    }));
    0
}

fn cmd_unblock(rest: &[String]) -> i32 {
    let args = parse_fb_args("unblock", rest, &["worker", "reason"]);
    let (repo, number) = repo_and_issue("unblock", &args);
    let lease = lease_seconds("unblock", &args);
    let worker = worker_id("unblock", &args);
    let reason = args
        .get("reason")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let reason = match reason {
        Some(r) => r,
        None => usage_error(
            "unblock",
            "fallback unblock: --reason is required (the override is audited with its reason, like the broker's unblock)",
        ),
    };
    let (open, labels) = fetch_issue(&repo, number).unwrap_or_else(|e| die(EXIT_TRANSPORT, &e));
    let markers = fetch_markers(&repo, number).unwrap_or_else(|e| die(EXIT_TRANSPORT, &e));
    let state = fold_claim(&markers, now_epoch(), lease);
    if let Some(why) = unblock_refusal(open, &labels, &state) {
        print_json(&json!({
            "unblocked": false, "reason": why,
            "repo": repo, "number": number, "claim": state.to_json(),
        }));
        return EXIT_NO_CLAIM;
    }
    let body = format!(
        "{}\n\nUnblocked by `{worker}` -- reason: {reason}\n\n(Fleet queue fallback mode: the queue backend is down, so the manual override is recorded here. The `queue/blocked` label is cleared and `queue/queued` restored; when the broker performs this override it records the same fact as an audit event.)",
        marker_line(&MarkerKind::Unblock, &worker, None, now_epoch())
    );
    let comment_url =
        post_comment(&repo, number, &body).unwrap_or_else(|e| die(EXIT_TRANSPORT, &e));
    let mut warnings = Vec::new();
    edit_labels(
        &repo,
        number,
        &["queue/queued"],
        &["queue/blocked"],
        &mut warnings,
    );
    // Read back, as claim does: the label transition is the state
    // change here, so an unblock whose labels did not move is
    // reported as not done, never hidden behind the audit comment.
    let (_, after_labels) = fetch_issue(&repo, number).unwrap_or_else(|e| die(EXIT_TRANSPORT, &e));
    if is_blocked(&after_labels) || !after_labels.iter().any(|l| l == "queue/queued") {
        print_json(&json!({
            "unblocked": false, "reason": "label_sync_failed",
            "repo": repo, "number": number, "worker": worker,
            "comment_url": comment_url, "warnings": warnings,
        }));
        return EXIT_NO_CLAIM;
    }
    print_json(&json!({
        "unblocked": true,
        "repo": repo, "number": number, "url": issue_url(&repo, number),
        "worker": worker, "reason": reason,
        "comment_url": comment_url, "warnings": warnings,
    }));
    0
}

/// Programmatic claim-state lookup (#6741): the GitHub-fallback
/// claim state of one issue, for the queue client's
/// no-double-serve guard. Same marker fold and lease as
/// `fallback status` (default 30 min, FLEET_QUEUE_FALLBACK_LEASE_MIN
/// override), minus the CLI surface.
pub fn issue_claim_state(repo: &str, number: i64) -> Result<Value, String> {
    let markers = fetch_markers(repo, number)?;
    let lease_min = std::env::var("FLEET_QUEUE_FALLBACK_LEASE_MIN")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|m| *m >= 1)
        .unwrap_or(DEFAULT_LEASE_MIN);
    Ok(fold_claim(&markers, now_epoch(), lease_min * 60).to_json())
}

pub fn run(args: &[String]) -> i32 {
    if args.is_empty() {
        println!("{GROUP_USAGE}");
        return EXIT_USAGE;
    }
    // A known subverb dispatches even when `--help` is present: the
    // verb's own parser prints the verb's exact-form usage (#10838).
    // Bare `fallback --help` prints the group usage.
    let sub = args[0].as_str();
    let rest = &args[1..];
    match sub {
        "scan" => cmd_scan(rest),
        "status" => cmd_status(rest),
        "claim" => cmd_claim(rest),
        "heartbeat" => cmd_heartbeat(rest),
        "release" => cmd_release(rest),
        "complete" => cmd_complete(rest),
        "unblock" => cmd_unblock(rest),
        s if is_help_arg(s) => {
            println!("{GROUP_USAGE}");
            0
        }
        _ => {
            die(
                EXIT_USAGE,
                &format!("unknown command {sub:?}\n{GROUP_USAGE}"),
            );
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

    // -- fallback complete (#13023): the disposition gate, run
    // locally by the fallback seat. The oracle is the broker's
    // own gate semantics (`crate::disposition::check`): a record
    // the broker would accept must pass here, and one it would
    // reject must not.

    fn disp_record(checked_epoch: i64, issue: Value, pulls: Value, verdict: &str) -> String {
        json!({
            "checked_at": format_ts(checked_epoch),
            "source": "verify-disposition",
            "issue": issue,
            "pulls": pulls,
            "verdict": verdict,
        })
        .to_string()
    }

    fn merged_pull(n: i64, at: i64) -> Value {
        json!({"number": n, "state": "merged", "is_draft": false, "merged_at": format_ts(at)})
    }

    fn draft_pull(n: i64) -> Value {
        json!({"number": n, "state": "open", "is_draft": true, "merged_at": null})
    }

    #[test]
    fn complete_gate_accepts_a_verified_record() {
        let text = disp_record(
            T0,
            json!({"number": 42, "state": "closed"}),
            json!([merged_pull(7, T0 - 60)]),
            "closed_verified",
        );
        let (record, verdict, checked_at, task) =
            check_fallback_disposition(&text, 42, &[], T0).expect("verified record passes");
        assert_eq!(verdict, "closed_verified");
        assert_eq!(checked_at, format_ts(T0));
        assert_eq!(task, None);
        assert_eq!(record["issue"]["number"], json!(42));
    }

    #[test]
    fn complete_gate_rejects_a_record_for_another_issue() {
        let text = disp_record(
            T0,
            json!({"number": 42, "state": "closed"}),
            json!([merged_pull(7, T0 - 60)]),
            "closed_verified",
        );
        let err = check_fallback_disposition(&text, 43, &[], T0).unwrap_err();
        assert!(err.contains("describes issue #42"), "{err}");
    }

    #[test]
    fn complete_gate_rejects_a_stale_record() {
        let text = disp_record(
            T0,
            json!({"number": 42, "state": "closed"}),
            json!([merged_pull(7, T0 - 60)]),
            "closed_verified",
        );
        let now = T0 + crate::disposition::DEFAULT_STALENESS_SECONDS + 1;
        let err = check_fallback_disposition(&text, 42, &[], now).unwrap_err();
        assert!(err.contains("re-run"), "{err}");
    }

    #[test]
    fn complete_gate_rejects_unfinished_work_under_a_merged_verdict() {
        // The #10611 failure class in miniature: a merged verdict
        // whose readback holds only an open draft.
        let text = disp_record(
            T0,
            json!({"number": 42, "state": "open"}),
            json!([draft_pull(7)]),
            "merged_verified",
        );
        let err = check_fallback_disposition(&text, 42, &[], T0).unwrap_err();
        assert!(err.contains("no pull in the readback is merged"), "{err}");
    }

    #[test]
    fn complete_gate_rejects_a_claimed_pr_missing_from_the_record() {
        let text = disp_record(
            T0,
            json!({"number": 42, "state": "closed"}),
            json!([merged_pull(7, T0 - 60)]),
            "closed_verified",
        );
        let err = check_fallback_disposition(&text, 42, &[9], T0).unwrap_err();
        assert!(err.contains("linked PR #9 is not in the readback"), "{err}");
    }

    #[test]
    fn complete_gate_rejects_malformed_records() {
        assert!(check_fallback_disposition("not json", 42, &[], T0).is_err());
        assert!(check_fallback_disposition("{}", 42, &[], T0).is_err());
    }

    #[test]
    fn complete_gate_surfaces_an_unchecked_follow_up_task() {
        // An open draft handed to a follow-up task: the offline
        // gate passes the record, and the task id comes back as
        // the broker-only check a fallback seat cannot run.
        let mut v: Value = serde_json::from_str(&disp_record(
            T0,
            json!({"number": 42, "state": "open"}),
            json!([draft_pull(7)]),
            "open_draft_with_followup",
        ))
        .unwrap();
        v["follow_up"] = json!({"task_id": 99, "reason": "the draft carries the rest"});
        let (_, verdict, _, task) =
            check_fallback_disposition(&v.to_string(), 42, &[], T0).expect("record passes");
        assert_eq!(verdict, "open_draft_with_followup");
        assert_eq!(task, Some(99));
    }

    #[test]
    fn complete_body_round_trips_marker_and_record() {
        let record: Value = serde_json::from_str(&disp_record(
            T0,
            json!({"number": 42, "state": "closed"}),
            json!([merged_pull(7, T0 - 60)]),
            "closed_verified",
        ))
        .unwrap();
        let body = complete_body(
            "lane-kf",
            "Landed in PR #7.",
            &record,
            "closed_verified",
            &format_ts(T0),
            None,
            T0,
        );
        // The claim fold must read the body as a release-done.
        let m = parse_marker(&body).expect("body carries a marker");
        assert_eq!(m.kind, MarkerKind::Release);
        assert_eq!(m.outcome.as_deref(), Some("done"));
        // And the recorded disposition extracts back as the same
        // JSON value, consumable when the broker returns.
        assert_eq!(disposition_record_in_comment(&body), Some(record.clone()));
        // The unchecked-task note appears when a task is named.
        let body2 = complete_body(
            "lane-kf",
            "Handed off.",
            &record,
            "open_draft_with_followup",
            &format_ts(T0),
            Some(99),
            T0,
        );
        assert!(body2.contains("task 99"), "{body2}");
        assert!(body2.contains("UNCHECKED"), "{body2}");
    }

    #[test]
    fn disposition_record_absent_from_plain_comments() {
        assert_eq!(disposition_record_in_comment("just a note"), None);
        assert_eq!(
            disposition_record_in_comment("```fleet-disposition\nnot json\n```"),
            None
        );
        assert_eq!(
            disposition_record_in_comment("```fleet-disposition\n{\"a\":1}"),
            None
        );
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
    fn collect_markers_full_last_page_is_a_truncation_error() {
        // #9044 oracle: a marker read whose final (10th) page is full
        // may be missing markers past 1,000 comments, so it must fail
        // explicitly -- never return the partial set as complete.
        let body = marker_line(&MarkerKind::Claim, "a", None, T0);
        let full_page = Value::Array(
            (0..MARKER_PAGE_SIZE)
                .map(|_| json!({"body": body}))
                .collect(),
        );
        let pages = std::cell::Cell::new(0usize);
        let err = collect_markers(|_page| {
            pages.set(pages.get() + 1);
            Ok(full_page.clone())
        })
        .expect_err("a full 10th page must error, not truncate silently");
        assert!(err.contains("truncated marker set"), "error was: {err}");
        assert_eq!(pages.get(), MAX_MARKER_PAGES);
    }

    #[test]
    fn collect_markers_under_the_bound_is_unchanged() {
        // Reads under the bound return every marker in page order and
        // stop at the first short page; non-marker comments are skipped.
        let body = marker_line(&MarkerKind::Claim, "a", None, T0);
        let full_page = Value::Array(
            (0..MARKER_PAGE_SIZE)
                .map(|_| json!({"body": body}))
                .collect(),
        );
        let last_page = json!([
            {"body": marker_line(&MarkerKind::Heartbeat, "a", None, T0 + 60)},
            {"body": "an ordinary progress comment"},
            {"body": marker_line(&MarkerKind::Release, "a", Some("done"), T0 + 120)},
        ]);
        let pages = std::cell::Cell::new(0usize);
        let markers = collect_markers(|page| {
            pages.set(pages.get() + 1);
            Ok(if page == 1 {
                full_page.clone()
            } else {
                last_page.clone()
            })
        })
        .expect("a read ending on a short page must succeed");
        assert_eq!(markers.len(), MARKER_PAGE_SIZE + 2);
        assert_eq!(markers[MARKER_PAGE_SIZE].kind, MarkerKind::Heartbeat);
        assert_eq!(markers[MARKER_PAGE_SIZE + 1].kind, MarkerKind::Release);
        assert_eq!(pages.get(), 2);
        // A single short (even empty) page is a complete read.
        let empty = collect_markers(|_page| Ok(json!([]))).expect("empty read must succeed");
        assert!(empty.is_empty());
    }

    #[test]
    fn marker_round_trip() {
        for (kind, outcome) in [
            (MarkerKind::Claim, None),
            (MarkerKind::Heartbeat, None),
            (MarkerKind::Release, Some("done")),
            (MarkerKind::Release, Some("free")),
            (MarkerKind::Unblock, None),
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
    fn fold_contest_ages_out_with_the_lease() {
        // Case 1 (#6660/#6668): contest, then both workers silent
        // for ~100 leases — the contest has aged out and the
        // holder's claim is stale and claimable, not contested
        // forever.
        let contest = vec![
            ev(MarkerKind::Claim, "a", T0),
            ev(MarkerKind::Claim, "b", T0 + 60),
        ];
        match fold_claim(&contest, T0 + 100 * LEASE, LEASE) {
            ClaimState::Stale { worker, .. } => assert_eq!(worker, "a"),
            other => panic!("expected stale a after contest aged out, got {other:?}"),
        }
        assert!(fold_claim(&contest, T0 + 100 * LEASE, LEASE).claimable());
        // Inside the lease the same events are still contested.
        assert!(matches!(
            fold_claim(&contest, T0 + LEASE, LEASE),
            ClaimState::Contested { .. }
        ));
    }

    #[test]
    fn fold_contest_holder_heartbeats_stay_live() {
        // Case 2 (#6660/#6668): the holder keeps heartbeating after
        // a contest. While the contest is young the state is
        // contested, but the heartbeats renew the holder's activity,
        // so once the challenger has been silent for a full lease
        // the contest ages out to a LIVE claim by the holder — not
        // a stale one.
        let events = vec![
            ev(MarkerKind::Claim, "a", T0),
            ev(MarkerKind::Claim, "b", T0 + 60),
            ev(MarkerKind::Heartbeat, "a", T0 + 120),
            ev(MarkerKind::Heartbeat, "a", T0 + 240),
        ];
        assert!(matches!(
            fold_claim(&events, T0 + 300, LEASE),
            ClaimState::Contested { .. }
        ));
        match fold_claim(&events, T0 + 60 + LEASE + 1, LEASE) {
            ClaimState::Live { worker, .. } => assert_eq!(worker, "a"),
            other => panic!("expected live a after contest aged out, got {other:?}"),
        }
    }

    #[test]
    fn fold_contest_challenger_release_clears() {
        // Case 3 (#6660/#6668): the challenger releases — the
        // contest is withdrawn, the holder's claim stands live, and
        // after the holder also goes silent past the lease the
        // issue is claimable again.
        let mut events = vec![
            ev(MarkerKind::Claim, "a", T0),
            ev(MarkerKind::Claim, "b", T0 + 60),
        ];
        events.push(rel("b", "free", T0 + 120));
        match fold_claim(&events, T0 + 240, LEASE) {
            ClaimState::Live { worker, .. } => assert_eq!(worker, "a"),
            other => panic!("expected live a after challenger release, got {other:?}"),
        }
        assert!(fold_claim(&events, T0 + 100 * LEASE, LEASE).claimable());
    }

    #[test]
    fn fold_contest_repeat_claim_refreshes_and_multi_challenger() {
        // Codex review on PR #6676, finding 1: a repeat claim from
        // the same challenger refreshes its contest activity, so
        // the contest does NOT age out one lease after the FIRST
        // conflicting claim while the challenger is still claiming.
        let events = vec![
            ev(MarkerKind::Claim, "a", T0),
            ev(MarkerKind::Claim, "b", T0 + 60),
            ev(MarkerKind::Claim, "b", T0 + LEASE - 100),
        ];
        match fold_claim(&events, T0 + 60 + LEASE + 1, LEASE) {
            ClaimState::Contested { challenger, .. } => assert_eq!(challenger, "b"),
            other => panic!("expected contested b after repeat claim, got {other:?}"),
        }
        // Finding 2: with two challengers, one withdrawing clears
        // only its own contest — the other challenger's stands.
        let mut multi = vec![
            ev(MarkerKind::Claim, "a", T0),
            ev(MarkerKind::Claim, "b", T0 + 60),
            ev(MarkerKind::Claim, "c", T0 + 90),
        ];
        multi.push(rel("b", "free", T0 + 120));
        match fold_claim(&multi, T0 + 240, LEASE) {
            ClaimState::Contested {
                holder, challenger, ..
            } => {
                assert_eq!(holder, "a");
                assert_eq!(challenger, "c");
            }
            other => panic!("expected contested by c after b withdrew, got {other:?}"),
        }
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

    #[test]
    fn unblock_marker_rejects_an_outcome() {
        // outcome is release-only: an unblock marker carrying one
        // is not a marker at all.
        let body = format!(
            "{MARKER_PREFIX}kind=unblock worker=a outcome=done ts={} -->",
            format_ts(T0)
        );
        assert_eq!(parse_marker(&body), None);
    }

    #[test]
    fn fold_ignores_unblock_markers() {
        // #12777 oracle: the unblock marker is audit-only -- it
        // neither takes nor releases a claim, wherever it lands in
        // the stream.
        let claim = [
            ev(MarkerKind::Claim, "a", T0),
            ev(MarkerKind::Unblock, "b", T0 + 10),
        ];
        assert_eq!(fold_claim(&claim, T0 + 20, LEASE).name(), "live");
        let alone = [ev(MarkerKind::Unblock, "b", T0)];
        assert_eq!(fold_claim(&alone, T0 + 20, LEASE), ClaimState::Free);
        let after_release = [
            ev(MarkerKind::Claim, "a", T0),
            rel("a", "free", T0 + 10),
            ev(MarkerKind::Unblock, "b", T0 + 20),
        ];
        assert_eq!(
            fold_claim(&after_release, T0 + 30, LEASE).name(),
            "released"
        );
        // last_unblock reports the newest override in the stream.
        assert_eq!(last_unblock(&after_release).map(|m| m.ts), Some(T0 + 20));
        assert_eq!(last_unblock(&alone).map(|m| m.worker.as_str()), Some("b"));
        assert_eq!(last_unblock(&[]), None);
    }

    #[test]
    fn unblock_guard_mirrors_the_broker() {
        // Oracle: the broker's verb_unblock refuses every task
        // whose status is not blocked; the fallback form adds the
        // closed-issue refusal and the claim-state safety (a
        // blocked task is unclaimed by construction, so a live or
        // contested claim means the label and the ground disagree
        // -- hands off).
        let blocked = vec!["queue/blocked".to_string()];
        let queued = vec!["queue/queued".to_string()];
        assert_eq!(
            unblock_refusal(false, &blocked, &ClaimState::Free),
            Some("issue_closed")
        );
        assert_eq!(
            unblock_refusal(true, &queued, &ClaimState::Free),
            Some("not_blocked")
        );
        assert_eq!(
            unblock_refusal(
                true,
                &blocked,
                &ClaimState::Live {
                    worker: "a".to_string(),
                    last_activity: T0,
                    age_seconds: 0,
                }
            ),
            Some("live_claim")
        );
        assert_eq!(
            unblock_refusal(
                true,
                &blocked,
                &ClaimState::Contested {
                    holder: "a".to_string(),
                    challenger: "b".to_string(),
                    ts: T0,
                }
            ),
            Some("contested")
        );
        assert_eq!(unblock_refusal(true, &blocked, &ClaimState::Free), None);
        assert_eq!(
            unblock_refusal(
                true,
                &blocked,
                &ClaimState::Stale {
                    worker: "a".to_string(),
                    last_activity: T0,
                    age_seconds: LEASE + 1,
                }
            ),
            None
        );
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

    // ---- #10838: discoverable fallback usage ----

    #[test]
    fn fallback_usage_covers_every_subverb() {
        for verb in ["scan", "status", "claim", "heartbeat", "release"] {
            let u = usage(verb);
            assert!(
                u.starts_with("usage: repo-queue fallback"),
                "{verb} usage header: {u:?}"
            );
            assert!(u.contains(verb), "{verb} usage names the verb: {u:?}");
        }
        assert!(GROUP_USAGE.contains("scan|status|claim|heartbeat|release"));
        // Unknown verbs fall back to the group usage, never nothing.
        assert_eq!(usage("bogus"), GROUP_USAGE);
    }

    #[test]
    fn fallback_help_flag_matching() {
        assert!(is_help_arg("--help"));
        assert!(is_help_arg("-h"));
        assert!(is_help_arg("help"));
        assert!(!is_help_arg("--bogus"));
    }

    #[test]
    fn a_help_word_given_as_a_flag_value_is_not_a_help_request() {
        let flags = ["worker", "note"];
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(!wants_help(&v(&["o/r", "5", "--note", "help"]), &flags));
        assert!(!wants_help(&v(&["o/r", "5", "--note", "-h"]), &flags));
        assert!(!wants_help(&v(&["o/r", "5", "--note", "--help"]), &flags));
        assert!(wants_help(&v(&["o/r", "5", "--help"]), &flags));
        assert!(wants_help(&v(&["--note=x", "-h"]), &flags));
        assert!(wants_help(&v(&["help"]), &flags));
    }
}
