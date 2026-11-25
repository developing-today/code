//! shared support helpers for the repo-queue client: time/format, CLI argument parsing, option resolution, telemetry and session model stamps (split from client.rs, #8588).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::process::exit;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use super::*;

// ---------------------------------------------------------------------------
// Time helpers (ISO 8601 <-> epoch, civil-date math)
// ---------------------------------------------------------------------------

pub(super) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub(super) fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Parse an ISO 8601 timestamp to epoch microseconds (None if unusable).
pub(super) fn parse_ts(v: &Value) -> Option<i64> {
    let s = v.as_str()?;
    parse_ts_str(s)
}

pub(super) fn parse_ts_str(s: &str) -> Option<i64> {
    let (date, rest) = s.split_once('T')?;
    let mut dp = date.split('-');
    let y: i64 = dp.next()?.parse().ok()?;
    let mo: i64 = dp.next()?.parse().ok()?;
    let d: i64 = dp.next()?.parse().ok()?;
    let (time, offset_min) = if let Some(t) = rest.strip_suffix('Z') {
        (t, 0i64)
    } else if let Some(idx) = rest.rfind(['+', '-']) {
        if idx == 0 {
            return None;
        }
        let (t, off) = rest.split_at(idx);
        let sign = if rest.as_bytes()[idx] == b'-' { -1 } else { 1 };
        let mut op = off[1..].split(':');
        let oh: i64 = op.next()?.parse().ok()?;
        let om: i64 = op.next().unwrap_or("0").parse().ok()?;
        (t, sign * (oh * 60 + om))
    } else {
        (rest, 0)
    };
    let mut tp = time.split(':');
    let h: i64 = tp.next()?.parse().ok()?;
    let mi: i64 = tp.next()?.parse().ok()?;
    let sec_part = tp.next().unwrap_or("0");
    let (sec_s, frac) = match sec_part.split_once('.') {
        Some((a, b)) => (a, b),
        None => (sec_part, ""),
    };
    let sec: i64 = sec_s.parse().ok()?;
    let mut micros: i64 = 0;
    let mut scale = 100_000i64;
    for c in frac.chars().take(6) {
        micros += c.to_digit(10)? as i64 * scale;
        scale /= 10;
    }
    let days = days_from_civil(y, mo, d);
    let epoch_s = days * 86400 + h * 3600 + mi * 60 + sec - offset_min * 60;
    Some(epoch_s * 1_000_000 + micros)
}

pub(super) fn now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

pub(super) fn utcnow_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

pub(super) fn age_str(created: &Value) -> String {
    let Some(c) = parse_ts(created) else {
        return String::new();
    };
    let secs = ((now_micros() - c) / 1_000_000).max(0);
    let (d, rem) = (secs / 86400, secs % 86400);
    let (h, rem) = (rem / 3600, rem % 3600);
    let (m, s) = (rem / 60, rem % 60);
    if d > 0 {
        format!("{d}d {h:02}:{m:02}")
    } else {
        format!("{h}:{m:02}:{s:02}")
    }
}

// ---------------------------------------------------------------------------
// Formatting (mirrors repo-queue.py tables)
// ---------------------------------------------------------------------------

pub(super) fn fmt_cell(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::Object(_)) | Some(Value::Array(_)) => {
            serde_json::to_string(v.unwrap()).unwrap_or_default()
        }
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

pub(super) fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let head: String = s.chars().take(n - 1).collect();
        format!("{head}…")
    }
}

pub(super) fn print_table(cols: &[String], rows: &[Vec<String>]) {
    let mut widths: Vec<usize> = cols.iter().map(|c| c.chars().count()).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            widths[i] = widths[i].max(c.chars().count());
        }
    }
    let fmt_row = |cells: &[String]| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let pad = widths[i] - c.chars().count();
                format!("{}{}", c, " ".repeat(pad))
            })
            .collect::<Vec<_>>()
            .join("  ")
    };
    println!("{}", fmt_row(cols));
    let dashes: Vec<String> = cols.iter().map(|c| "-".repeat(c.chars().count())).collect();
    println!("{}", fmt_row(&dashes));
    for r in rows {
        println!("{}", fmt_row(r));
    }
}

// ---------------------------------------------------------------------------
// Argument parsing (argparse-shaped subset the fleet uses)
// ---------------------------------------------------------------------------

pub(super) struct Args {
    pub(super) pos: Vec<String>,
    pub(super) opts: Map<String, Value>,
}

/// Top-level usage, printed by bare `repo-queue --help` and appended
/// to the unknown-command error (#10838).
pub(crate) const TOP_USAGE: &str = "\
usage: repo-queue <verb> ...\n\
\n\
Run `repo-queue <verb> --help` for flags:\n\
  validate <packet-path>\n\
  enqueue <packet-path> [--queue <name>] [--platform <p>] [--meta key=json]... [--dry-run] [--shadow-intent]\n\
  list [--queue <name>] [--status <s>] [--limit <n>] [--not-before <ts>] [--lane <l>] [--worker <w>] [--task <id>] [--meta key=value]... [--label <name|prefix*>] [--json]\n\
  meta <get|set|unset|keys> ...\n\
  claim [--queue <name>] [--worker <id>] [--lease-seconds <n>] [--lane <l>] [--task <id>] [--count <n>] [--from-session <id>] [--platform <p>] [--session-id <id>] [--model-provider <p>] [--model-id <id>] [--model-variant <v>] [--filter <f>] [--pool <p>] [--launched-by <who>] [--remaining-budget-minutes <n>] [--handoff] [--bind]\n\
  release <task-id> [--worker <id>] [--lane <id>] --reason <text>\n\
  lane <task-id> [--worker <id>] [--exclusion-reason <text>] [--opencode-reason <text>] [--limit-platform <p>] [--limit-reason <text>] [--clear-platform-limit]\n\
  bind <task-id> [--worker <id>] --session-id <id> [--launched-by <who>] [--model-provider <p>] [--model-id <id>] [--model-variant <v>]\n\
  heartbeat <task-id> [--worker <id>] [--lane <id>] [--extend-seconds <n>] [--progress <p>] [--progress-file <f>]\n\
  checkpoint <task-id> [--worker <id>] [--lane <id>] --stage <s> [--progress <p>] [--progress-file <f>]\n\
  reprioritize <task-id> <priority> [--worker <id>] [--reason <text>]\n\
  park <task-id> [--worker <id>] [--lane <id>] --reason <text> [--delay-seconds <n>] [--not-before <ts>] [--progress <p>] [--progress-file <f>] [--ready-condition <c>] [--execution <id>] [--required-budget-minutes <n>] [--hands <kind> --action <text>]\n\
  wake <task-id> [--worker <id>] [--reason <text>]\n\
  block <task-id> [--worker <id>] [--lane <id>] --reason-class <c> --blocker <b> [--progress <p>] [--progress-file <f>] [--ledger-id <id>] [--question <text>] [--context <text>] [--options-json <json>] [--links-json <json>] [--blocked-by-pr <n>] [--blocked-by-issue <n>] [--blocked-by-task <id>] [--blocked-by-ci <ref>] [--dep-recheck-seconds <n>] [--dep-max-ttl-seconds <n>] [--hands <kind> --action <text>]\n\
  unblock <task-id> [--worker <id>] [--reason <text>] [--drop-dep <list>]\n\
  dep-recheck [--task <id>] [--limit <n>]\n\
  quarantine-recheck [--task <id>] [--limit <n>]\n\
  repair <task-id> [--worker <id>] [--reason <text>] [--dry-run]\n\
  resolution-backfill\n\
  cancel <task-id> [--worker <id>] --reason <text> --resolution <name> [--substatus <name>] [--ref <target>]\n\
  revive <task-id> [--worker <id>] [--reason <text>] [--not-before <ts>]\n\
  fail <task-id> [--worker <id>] [--lane <id>] --error <text> [--reason-class <c>] [--execution <id>]\n\
  complete <task-id> [--worker <id>] [--lane <id>] [--result <json>] [--result-file <f>] [--telemetry <json>] [--execution <id>] [--resolution <name>] [--substatus <name>] [--reason <text>] [--ref <target>]\n\
  verify-disposition <issue> --repo <owner/name> [--pr <n,n>] [--follow-up-task <id> | --follow-up-issue <n|url> | --follow-up-pr <n>] [--reason <text>] [--staleness-seconds <n>] [--json]\n\
  timeline <task-id> [--worker <id>] [--json]\n\
  history <task-id> [--worker <id>] [--json]  (alias of timeline)\n\
  show <task-id> [--limit <n>] [--json]\n\
  note <add|list|edit|remove> ...\n\
  link <add|remove|list> ...\n\
  models [--days <n>] [--json]\n\
  health [--queue <name>] [--worker <id>]\n\
  signal [--queue <name>] [--reason <text>]\n\
  fallback <scan|status|claim|heartbeat|release|complete|unblock> ...\n\
  outbox <enqueue|list|metrics|claim|resolve|drain> ...\n\
  dispatch|session|surfaces ...\n\
  stack <record|list|transition|restack|replay> ...";

/// Per-verb usage for `repo-queue <verb> --help` (#10838). Unknown
/// verbs (and the top level) fall back to TOP_USAGE so a usage error
/// never prints nothing.
pub(crate) fn usage(verb: &str) -> &'static str {
    match verb {
        "validate" => {
            "usage: repo-queue validate <packet-path>\n\nValidates a task packet without enqueuing it."
        }
        "enqueue" => {
            "usage: repo-queue enqueue <packet-path> [--queue <name>] [--platform <p>] [--meta key=json]... [--dry-run] [--shadow-intent]\n\nEnqueues a task packet with optional metadata."
        }
        "list" => {
            "usage: repo-queue list [--queue <name>] [--status <s>] [--limit <n>] [--not-before <ts>] [--lane <l>] [--worker <w>] [--task <id>] [--meta key=value]... [--label <name|prefix*>] [--json]\n\nLists tasks; repeat --meta for ANDed JSON equality filters. --label filters on cached GitHub labels (a trailing * matches a prefix, e.g. user/*). A bare `list` shows one queue in the broker's default ordering (newest terminal rows first) — pass --status queued|parked|claimed|blocked."
        }
        "meta" => {
            "usage: repo-queue meta get <task-id> [key] | set <task-id> <key> <json-value> | set <task-id> [--json '{...}'] | unset <task-id> <key> | keys\n\nReads or merge-patches queue metadata. Null values delete keys; metadata writes are audited in task history."
        }
        "claim" => {
            "usage: repo-queue claim [--queue <name>] [--worker <id>] [--lease-seconds <n>] [--lane <l>] [--task <id>] [--count <n>] [--from-session <id>] [--platform <p>] [--session-id <id>] [--model-provider <p>] [--model-id <id>] [--model-variant <v>] [--filter <f>] [--pool <p>] [--launched-by <who>] [--remaining-budget-minutes <n>] [--handoff] [--bind]\n\nClaims the highest-priority eligible task."
        }
        "release" => {
            "usage: repo-queue release <task-id> [--worker <id>] [--lane <id>] --reason <text>\n\nReleases a held claim."
        }
        "lane" => {
            "usage: repo-queue lane <task-id> [--worker <id>] [--exclusion-reason <text>] [--opencode-reason <text>] [--limit-platform <p>] [--limit-reason <text>] [--clear-platform-limit]\n\nReports lane progress on a task."
        }
        "bind" => {
            "usage: repo-queue bind <task-id> [--worker <id>] --session-id <id> [--launched-by <who>] [--model-provider <p>] [--model-id <id>] [--model-variant <v>]\n\nBinds a worker to a task."
        }
        "heartbeat" => {
            "usage: repo-queue heartbeat <task-id> [--worker <id>] [--lane <id>] [--extend-seconds <n>] [--progress <p>] [--progress-file <f>]\n\nRefreshes a claim lease."
        }
        "checkpoint" => {
            "usage: repo-queue checkpoint <task-id> [--worker <id>] [--lane <id>] --stage <s> [--progress <p>] [--progress-file <f>]\n\nRecords a progress checkpoint."
        }
        "reprioritize" => {
            "usage: repo-queue reprioritize <task-id> <priority> [--worker <id>] [--reason <text>]\n\nChanges a live task priority (one of 10, 20, 30, 40, 50, 70, 90) and synchronizes its linked issue category."
        }
        "park" => {
            "usage: repo-queue park <task-id> [--worker <id>] [--lane <id>] --reason <text> [--delay-seconds <n>] [--not-before <ts>] [--progress <p>] [--progress-file <f>] [--ready-condition <c>] [--execution <id>] [--required-budget-minutes <n>] [--hands <kind> --action <text>]\n\nParks a task. A park that waits on a person is refused unless it is a hands item (FDR-0013): --hands physical|interactive-login|payment|signature with --action, one sentence a person can do in five minutes."
        }
        "wake" => {
            "usage: repo-queue wake <task-id> [--worker <id>] [--reason <text>]\n\nWakes a parked task."
        }
        "block" => {
            "usage: repo-queue block <task-id> [--worker <id>] [--lane <id>] --reason-class <c> --blocker <b> [--progress <p>] [--progress-file <f>] [--ledger-id <id>] [--question <text>] [--context <text>] [--options-json <json>] [--links-json <json>] [--blocked-by-pr <n>] [--blocked-by-issue <n>] [--blocked-by-task <id>] [--blocked-by-ci <ref>] [--dep-recheck-seconds <n>] [--dep-max-ttl-seconds <n>] [--hands <kind> --action <text>]\n\nMarks a task blocked. A block that waits on a person is refused unless it is a hands item (FDR-0013): --hands physical|interactive-login|payment|signature with --action, one sentence a person can do in five minutes."
        }
        "unblock" => {
            "usage: repo-queue unblock <task-id> [--worker <id>] [--reason <text>] [--drop-dep <list>]\n\nClears a task's blocked state. With --drop-dep, only the named dep entries are dropped from the task's record: a comma-separated list of kind:ref — issue:owner/repo#N, pr:owner/repo#N, ci:owner/repo#RUN, task:ID. The task stays blocked on the remaining deps; dropping the last one releases it."
        }
        "dep-recheck" => {
            "usage: repo-queue dep-recheck [--task <id>] [--limit <n>]\n\nRechecks task dependencies."
        }
        "quarantine-recheck" => {
            "usage: repo-queue quarantine-recheck [--task <id>] [--limit <n>]\n\nRechecks quarantined tasks."
        }
        "repair" => {
            "usage: repo-queue repair <task-id> [--worker <id>] [--reason <text>] [--dry-run]\n\nRepairs a quarantined or malformed task packet in place: re-runs the packet validators and applies the deterministic repairs whose correct value can be re-derived (today the dedupe stamp, re-performed live — a live duplicate refuses). Unrepairable defects fail with the defect list and change nothing. --dry-run reports defects and planned repairs, writing nothing."
        }
        "resolution-backfill" => {
            "usage: repo-queue resolution-backfill\n\nRuns the idempotent Q2 terminal-resolution backfill and prints the terminal census, proven rows, and unresolved rows. Only verdicts evidenced by existing result data or a unique dedupe twin are written."
        }
        "cancel" => {
            "usage: repo-queue cancel <task-id> [--worker <id>] --reason <text> --resolution <name> [--substatus <name>] [--ref <target>]\n\nCancels a task with a required redirection or rejection resolution. Rejections require a reason; redirections require a reference. Without --worker, the cancellation is recorded under the default worker fleet-monitor."
        }
        "revive" => {
            "usage: repo-queue revive <task-id> [--worker <id>] [--reason <text>] [--not-before <ts>]\n\nRevives a terminal task."
        }
        "fail" => {
            "usage: repo-queue fail <task-id> [--worker <id>] [--lane <id>] --error <text> [--reason-class <c>] [--execution <id>]\n\nFails a task. --reason-class is one of transient (the default), deterministic, blocked."
        }
        "complete" => {
            "usage: repo-queue complete <task-id> [--worker <id>] [--lane <id>] [--result <json>] [--result-file <f>] [--telemetry <json>] [--execution <id>] [--resolution <name>] [--substatus <name>] [--reason <text>] [--ref <target>]\n\nCompletes a task. Defaults to resolution done/unverified; a verified disposition readback stamps done/verified."
        }
        "verify-disposition" => {
            "usage: repo-queue verify-disposition <issue> --repo <owner/name> [--pr <n,n>] [--follow-up-task <id> | --follow-up-issue <n|url> | --follow-up-pr <n>] [--reason <text>] [--staleness-seconds <n>] [--json]\n\nReads the issue and its linked PRs back from GitHub and prints the disposition a terminal completion must carry (#10611); exits 1 when the broker's gate would reject it."
        }
        "timeline" => {
            "usage: repo-queue timeline <task-id> [--worker <id>] [--json]\n\nShows a task's event timeline."
        }
        "history" => {
            "usage: repo-queue history <task-id> [--worker <id>] [--json]\n\nAlias of timeline: shows a task's event timeline."
        }
        "show" => {
            "usage: repo-queue show <task-id> [--limit <n>] [--json]\n\nShows one task's current details and recent history."
        }
        "note" => {
            "usage: repo-queue note <add|list|edit|remove> ...\n\nA task's notes: an append-only, authored stream (#14170 Q4). Run `repo-queue note <sub> --help` for flags."
        }
        "link" => {
            "usage: repo-queue link <add|remove|list> ...\n\nManages normalized relationships for a queue task."
        }
        "link add" => {
            "usage: repo-queue link add <task-id> [--kind <kind>] (--issue <owner/repo#n> | --pr <owner/repo#n> | --task <task-id>) [--worker <id>]\n\nAdds an issue, pull request, or task relationship."
        }
        "link remove" => {
            "usage: repo-queue link remove <task-id> [--kind <kind>] (--issue <owner/repo#n> | --pr <owner/repo#n> | --task <task-id>) [--worker <id>]\n\nSoft-removes a relationship."
        }
        "link list" => {
            "usage: repo-queue link list <task-id> [--include-removed] [--json]\n\nLists a task's relationships."
        }
        "note add" => {
            "usage: repo-queue note add <task-id> --text <text> [--worker <id>] [--kind agent|human|system] [--session-id <id>] [--model-provider <p>] [--model-id <id>] [--model-variant <v>]\n\nAppends a note to a task. The worker is the author. The body is at most 4096 characters. Without marks, a note by the task's claim holder copies the claim's session and model."
        }
        "note list" => {
            "usage: repo-queue note list <task-id> [--latest <n>] [--include-removed] [--json]\n\nLists a task's notes, oldest first, with the count of live notes. --latest keeps the newest n."
        }
        "note edit" => {
            "usage: repo-queue note edit <note-id> --text <text> [--worker <id>]\n\nReplaces a note's body. Only the note's author may edit it; the old body is kept in the task's history."
        }
        "note remove" => {
            "usage: repo-queue note remove <note-id> [--worker <id>]\n\nRemoves a note. The removal is soft: the row stays, hidden from list unless --include-removed. Only the note's author may remove it."
        }
        "models" => {
            "usage: repo-queue models [--days <n>] [--json]\n\nRolls up serving-model stamps."
        }
        "health" => "usage: repo-queue health [--queue <name>] [--worker <id>]\n\nBroker health.",
        "signal" => {
            "usage: repo-queue signal [--queue <name>] [--reason <text>]\n\nSends a queue signal."
        }
        "fallback" => crate::fallback::GROUP_USAGE,
        "outbox enqueue" => {
            "usage: repo-queue outbox enqueue [--repo <r>] [--kind <k>] [--dedup-key <d>] [--producer <p>] [--payload <json> | --payload-file <f>]\n\nEnqueues an outbox item."
        }
        "outbox list" => {
            "usage: repo-queue outbox list [--repo <r>] [--state <s>]\n\nLists outbox items."
        }
        "outbox metrics" => "usage: repo-queue outbox metrics\n\nOutbox metrics.",
        "outbox claim" => {
            "usage: repo-queue outbox claim [--owner <name>]\n\nClaims an outbox item."
        }
        "outbox resolve" => {
            "usage: repo-queue outbox resolve <id> [--outcome done|rate_limited|uncertain|failed] [--github-number <n>] [--github-url <u>] [--retry-after <s>] [--error <text>]\n\nResolves an outbox item."
        }
        "outbox drain" => {
            "usage: repo-queue outbox drain [--owner <name>] [--max <n>]\n\nDrains claimed outbox items."
        }
        "stack record" => {
            "usage: repo-queue stack record --stack <id> --repo <r> --branch <b> [--base-branch <b>] [--head-sha <sha>] [--issue <n>] [--pr <n>] --lane <l> [--worker <id>] [--session-id <id>]\n\nRecords a stack entry."
        }
        "stack list" => {
            "usage: repo-queue stack list <stack-id> [--stack <id>] [--worker <id>]\n\nLists a stack's intents."
        }
        "stack transition" => {
            "usage: repo-queue stack transition --stack <id> --position <n> --to <state> [--pr <n>] [--conflict-detail <text>] [--worker <id>]\n\nTransitions a stack."
        }
        "stack restack" => {
            "usage: repo-queue stack restack --stack <id> --position <n> --base <b> [--head-sha <sha>] [--worker <id>]\n\nRestacks intents."
        }
        "stack replay" => {
            "usage: repo-queue stack replay --stack <id> [--worker <id>] [--repo-dir <dir>] [--dry-run]\n\nReplays a stack plan."
        }
        _ => TOP_USAGE,
    }
}

/// Help wins over every other argument (#10838, matching the broker
/// binaries' `bin_args.rs` convention): `repo-queue <verb> --help`
/// (or `-h`, or `help`) prints the verb's exact-form usage and
/// exits 0, even alongside other flags.
pub(super) fn is_help_arg(s: &str) -> bool {
    matches!(s, "--help" | "-h" | "help")
}

/// True when `rest` asks for help: a help token that is not the value of
/// one of `flags`. `fail 5 --error help` records the error text "help" and
/// `park 5 --reason -h` parks with reason "-h"; neither is a help request.
/// The value rule mirrors `parse_args_inner`: a value flag without an
/// inline `=` takes the next token unless that token starts with `--`.
pub(crate) fn wants_help(rest: &[String], flags: &[&str]) -> bool {
    let mut i = 0;
    while i < rest.len() {
        let a = rest[i].as_str();
        if is_help_arg(a) {
            return true;
        }
        if let Some(long) = a.strip_prefix("--")
            && !long.contains('=')
            && flags.contains(&long)
            && rest.get(i + 1).is_some_and(|v| !v.starts_with("--"))
        {
            i += 2;
            continue;
        }
        i += 1;
    }
    false
}

/// The full text of a usage error: the one-line message plus the
/// verb's usage. Pure so the shape is unit-testable (#10838).
pub(crate) fn usage_error_text(verb: &str, msg: &str) -> String {
    format!("repo-queue: {msg}\n{}", usage(verb))
}

/// Die with a usage error (exit 2): the message plus the verb's
/// usage, so the error teaches the accepted flags.
pub(super) fn usage_error(verb: &str, msg: &str) -> ! {
    eprint(&usage_error_text(verb, msg));
    exit(2);
}

/// Verb-aware argument parsing: `--help`/`-h` anywhere prints the
/// verb's usage and exits 0; any shape rejection (unknown or
/// duplicate flag, flag without its value) prints the error plus
/// the verb's usage and exits 2.
pub(super) fn parse_verb_args(verb: &str, rest: &[String], flags: &[&str], bools: &[&str]) -> Args {
    if wants_help(rest, flags) {
        println!("{}", usage(verb));
        exit(0);
    }
    match parse_args_inner(rest, flags, bools) {
        Ok(args) => args,
        Err(msg) => usage_error(verb, &msg),
    }
}
/// Pure core of `parse_verb_args`, returning the usage error instead
/// of exiting so the rejection shapes are unit-testable (#5939).
///
/// Duplicate and contradictory flags are usage errors, never silent
/// last-wins: repeating a value flag used to retarget an invocation
/// (e.g. `--queue a --queue b` recorded queue=b), and a boolean given
/// an inline value (`--dry-run=false`) used to be read as `true` —
/// a real enqueue silently became a dry run that enqueued nothing
/// and exited 0. A value flag followed by another flag token is the
/// same class (#8582, Codex P1 on PR #8790): the flag token is not a
/// value — `--head-sha --dry-run` used to persist "--dry-run" as the
/// SHA on a mutating stack record instead of erroring.
pub(super) fn parse_args_inner(
    rest: &[String],
    flags: &[&str],
    bools: &[&str],
) -> Result<Args, String> {
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
            if opts.contains_key(&name) {
                return Err(format!("argument --{name}: duplicate flag"));
            }
            if bools.contains(&name.as_str()) {
                if let Some(v) = inline {
                    return Err(format!(
                        "argument --{name}: boolean flag takes no value (got ={v})"
                    ));
                }
                opts.insert(name, Value::Bool(true));
                i += 1;
            } else if flags.contains(&name.as_str()) {
                if let Some(v) = inline {
                    opts.insert(name, Value::String(v));
                    i += 1;
                } else if i + 1 < rest.len() && !rest[i + 1].starts_with("--") {
                    opts.insert(name, Value::String(rest[i + 1].clone()));
                    i += 2;
                } else {
                    return Err(format!("argument --{name}: expected one argument"));
                }
            } else {
                return Err(format!("unrecognized argument: --{name}"));
            }
        } else {
            pos.push(a.clone());
            i += 1;
        }
    }
    Ok(Args { pos, opts })
}

impl Args {
    pub(super) fn pos(&self, idx: usize) -> Option<String> {
        self.pos.get(idx).cloned()
    }
    pub(super) fn get(&self, name: &str) -> Option<String> {
        self.opts
            .get(name)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    pub(super) fn has(&self, name: &str) -> bool {
        self.opts.contains_key(name)
    }
    pub(super) fn get_i64(&self, name: &str) -> Option<i64> {
        match self.get(name) {
            None => None,
            Some(s) => match parse_i64_flag(name, &s) {
                Ok(n) => Some(n),
                Err(msg) => die(2, &msg),
            },
        }
    }
    pub(super) fn pos_i64(&self, idx: usize, what: &str) -> i64 {
        match self.pos.get(idx).and_then(|s| s.parse::<i64>().ok()) {
            Some(n) => n,
            None => die(2, &format!("argument {what}: invalid int value")),
        }
    }
}

/// Die with a usage error when `args` carries more than `expected`
/// positionals for `verb` (#5592). `parse_verb_args` collects positionals
/// without arity checking, and verbs read only the ones they expect —
/// so an extra positional (a second packet path, a stray word after a
/// task id, junk after `health`) was silently dropped while the verb
/// did its real work and exited 0. Every verb rejects extras through
/// this check before any broker call; unknown `--flags` were already
/// rejected in `parse_verb_args`, this gives positionals the same treatment.
/// The error carries the verb's usage (#10838).
pub(super) fn reject_extra_pos(verb: &str, args: &Args, expected: usize) {
    if let Some(extra) = extra_positional(&args.pos, expected) {
        usage_error(
            verb,
            &format!("{verb}: unexpected extra argument {extra:?}"),
        );
    }
}

/// The first positional past `expected`, if any.
pub(super) fn extra_positional(pos: &[String], expected: usize) -> Option<&str> {
    pos.get(expected).map(|s| s.as_str())
}

/// Parse a numeric flag's value (#5939): a non-numeric value is a
/// usage error naming the flag and the value — the same treatment
/// `pos_i64` gives positionals — never a silent default (a mistyped
/// `--limit abc` used to fall back to 20 without a word).
pub(super) fn parse_i64_flag(name: &str, raw: &str) -> Result<i64, String> {
    raw.parse::<i64>()
        .map_err(|_| format!("argument --{name}: invalid int value {raw:?}"))
}

/// The accepted range for `list --limit` (#6419; design #6448 Phase 3
/// pin: out-of-range is REJECTED, never clamped, at both the client
/// and the broker). `limit=0` used to silently report no tasks and an
/// unbounded value reached the PostgREST query verbatim.
pub(crate) const LIST_LIMIT_MAX: i64 = 500;
pub(crate) const LIST_LIMIT_DEFAULT: i64 = 20;

/// Validate a `list --limit` value against `1..=LIST_LIMIT_MAX`; the
/// error names `--limit` so a usage failure points at the flag.
pub(crate) fn validate_list_limit(limit: i64) -> Result<i64, String> {
    if (1..=LIST_LIMIT_MAX).contains(&limit) {
        Ok(limit)
    } else {
        Err(format!(
            "argument --limit: value {limit} out of range 1..={LIST_LIMIT_MAX}"
        ))
    }
}

/// The `list --limit` value for a call: the flag's value validated, or
/// the unchanged default when the flag is absent.
pub(crate) fn list_limit_or_default(raw: Option<i64>) -> Result<i64, String> {
    validate_list_limit(raw.unwrap_or(LIST_LIMIT_DEFAULT))
}

/// One page of a `list` fetch (#7871): a single /list response
/// reliably carries at most ~100 rows — above that the reply comes
/// back unusable and, before this fix, the client rendered it as a
/// silent "(no tasks)" with exit 0. `cmd_list` therefore fetches
/// any limit in pages of this size, advancing an offset the broker
/// honors on both backends (PostgREST `offset=` and Turso
/// `OFFSET`), so a large --limit returns rows page by page instead
/// of one oversized response.
pub(crate) const LIST_PAGE_SIZE: i64 = 100;

/// The (offset, page_limit) requests that fetch `limit` rows in
/// pages of LIST_PAGE_SIZE (#7871). Pure, so the paging plan is
/// unit-testable without a broker. Oracle: the pages are
/// contiguous, non-overlapping, in ascending offset order, and
/// their page_limits sum to exactly `limit`.
pub(crate) fn list_pages(limit: i64) -> Vec<(i64, i64)> {
    let mut pages = Vec::new();
    let mut offset = 0;
    while offset < limit {
        let page = LIST_PAGE_SIZE.min(limit - offset);
        pages.push((offset, page));
        offset += page;
    }
    pages
}

/// The /list `offset` for a broker request (#7871): absent means
/// the first page; a negative value is never interpolated into a
/// query — it clamps to 0. The client only ever sends offsets its
/// own paging plan produced; the broker re-derives the value here
/// because it is the trust boundary for non-CLI callers.
pub(crate) fn list_offset_or_default(raw: Option<i64>) -> i64 {
    raw.unwrap_or(0).max(0)
}

/// The rows of a /list envelope (#7871): the `rows` field must be
/// present and an array. Before this guard, a malformed or
/// truncated broker reply (which parses to a Null body) fell
/// through `unwrap_or_default` in cmd_list and printed
/// "(no tasks)" with exit 0 — a silent empty that reads as "task
/// does not exist". A reply we cannot read is an error, never an
/// empty list.
pub(crate) fn list_rows(resp: &Value) -> Result<Vec<Value>, String> {
    resp.get("rows")
        .and_then(|r| r.as_array())
        .cloned()
        .ok_or_else(|| {
            "queue-broker /list reply had no rows array — refusing to report an empty list"
                .to_string()
        })
}

/// The /list reply envelope (#10376; contract #6622 rule 3 —
/// complete, bounded, or say so): alongside the rows, the total
/// number of matching tasks when the backend knows it (null when
/// it does not) and whether this window is a strict prefix of the
/// matching set. Additive: existing consumers read only `rows`.
pub(crate) fn list_envelope(rows: Value, total: Option<i64>, offset: i64) -> Value {
    let returned = rows.as_array().map(|a| a.len() as i64).unwrap_or(0);
    let truncated = total.map(|t| offset + returned < t).unwrap_or(false);
    json!({"rows": rows, "total": total, "truncated": truncated})
}

/// The total a /list reply carries (#10376), when the broker
/// reported one. Absent on replies from a pre-#10376 broker.
pub(crate) fn list_total(resp: &Value) -> Option<i64> {
    resp.get("total").and_then(|t| t.as_i64())
}

/// The accepted ranges for the queue duration flags (#6420; design
/// #6448 Phase 3 pin, same as `list --limit`: out-of-range is
/// REJECTED, never clamped, at both the client and the broker).
/// A 0/negative lease is born expired, a 0/negative heartbeat extend
/// lapses the holder's own claim, and a negative park delay puts
/// `not_before` in the past and silently defeats the park; an
/// unbounded value overflows the SQL `integer` RPC parameters.
/// Maxima: a lease or extend beyond a day defeats the expiry the
/// lease exists to provide; a park delay beyond the dep-block TTL
/// ceiling (7 days, #8276) outlives every broker wait.
pub(crate) const LEASE_SECONDS_MAX: i64 = 86_400;
pub(crate) const LEASE_SECONDS_DEFAULT: i64 = 2_700;
pub(crate) const EXTEND_SECONDS_MAX: i64 = 86_400;
pub(crate) const EXTEND_SECONDS_DEFAULT: i64 = 900;
pub(crate) const DELAY_SECONDS_MAX: i64 = 604_800;

/// Validate one duration flag's value against `1..=max`; the error
/// names the flag so a usage failure points at it. Zero is rejected
/// for every flag: for `--delay-seconds` that is the explicit #6420
/// decision — a park that waits zero seconds is a requeue, and a
/// requeue already has its own verb (`release`).
pub(crate) fn validate_duration(flag: &str, value: i64, max: i64) -> Result<i64, String> {
    if (1..=max).contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "argument --{flag}: value {value} out of range 1..={max}"
        ))
    }
}

/// Validate a `claim --lease-seconds` value against
/// `1..=LEASE_SECONDS_MAX`; the error names `--lease-seconds`.
pub(crate) fn validate_lease_seconds(value: i64) -> Result<i64, String> {
    validate_duration("lease-seconds", value, LEASE_SECONDS_MAX)
}

/// The `claim --lease-seconds` value for a call: the flag's value
/// validated, or the unchanged default when the flag is absent.
pub(crate) fn lease_seconds_or_default(raw: Option<i64>) -> Result<i64, String> {
    validate_lease_seconds(raw.unwrap_or(LEASE_SECONDS_DEFAULT))
}

/// Validate a `heartbeat --extend-seconds` value against
/// `1..=EXTEND_SECONDS_MAX`; the error names `--extend-seconds`.
pub(crate) fn validate_extend_seconds(value: i64) -> Result<i64, String> {
    validate_duration("extend-seconds", value, EXTEND_SECONDS_MAX)
}

/// The `heartbeat --extend-seconds` value for a call: the flag's
/// value validated, or the unchanged default when the flag is absent.
pub(crate) fn extend_seconds_or_default(raw: Option<i64>) -> Result<i64, String> {
    validate_extend_seconds(raw.unwrap_or(EXTEND_SECONDS_DEFAULT))
}

/// Validate a `park --delay-seconds` value against
/// `1..=DELAY_SECONDS_MAX`; the error names `--delay-seconds`.
/// (No default: the flag is optional, and an absent flag leaves
/// `delay_seconds` out of the park request entirely.)
pub(crate) fn validate_delay_seconds(value: i64) -> Result<i64, String> {
    validate_duration("delay-seconds", value, DELAY_SECONDS_MAX)
}

/// Validate a `--not-before` flag value (#6426): the same rule the
/// packet validator and the broker apply — a non-empty RFC 3339
/// timestamp no more than 366 days in the future. Until now the raw
/// string went into the park/revive body unchecked, so garbage
/// failed only as a downstream DB cast error. A valid value is
/// returned unchanged, so the broker body for valid timestamps is
/// byte-for-byte what it was before; an invalid one is a usage
/// error naming `--not-before` (the caller dies with exit 2).
pub(crate) fn validate_not_before_flag(raw: &str) -> Result<String, String> {
    crate::validate::validate_not_before(&json!(raw), "--not-before")?;
    Ok(raw.to_string())
}

pub(super) fn env_or_default(value: Option<&str>, default: &str) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
        .to_string()
}

pub(super) fn default_queue_from_env(value: Option<&str>) -> String {
    env_or_default(value, DEFAULT_QUEUE)
}

pub(super) fn signal_path_from_env(
    value: Option<&str>,
    home_dir: Option<&str>,
) -> Result<String, &'static str> {
    let home_dir = crate::home::resolve(home_dir)?;
    let default = format!(
        "{home_dir}/workspace/goals/keep-the-four-repos-healthy/hidden_files/repo-queue-signal.json"
    );
    Ok(env_or_default(value, &default))
}

pub(super) fn token_file_from_env(
    value: Option<&str>,
    home_dir: Option<&str>,
) -> Result<String, &'static str> {
    let home_dir = crate::home::resolve(home_dir)?;
    let default = format!("{home_dir}/.config/queue-broker/token");
    Ok(env_or_default(value, &default))
}

pub(super) fn default_queue() -> String {
    let value = std::env::var("REPO_QUEUE_DEFAULT_QUEUE").ok();
    default_queue_from_env(value.as_deref())
}

pub(super) fn opt_queue(args: &Args) -> String {
    args.get("queue").unwrap_or_else(default_queue)
}

/// The queue scope for `claim` and `list` (#8097 one-queue model,
/// #7358): an explicit --queue wins, then the REPO_QUEUE_DEFAULT_QUEUE
/// env; with neither, the verb is UNSCOPED (None) and the broker works
/// across the one fleet queue. Neither verb falls back to the built-in
/// shadow default, so a bare `claim` never burns a lease on
/// repo-health-code-shadow and a bare `list` never shows only the
/// shadow stream as if it were the whole picture. Every other command
/// keeps opt_queue's built-in default.
pub(super) fn opt_queue_scoped(args: &Args) -> Option<String> {
    let env = std::env::var("REPO_QUEUE_DEFAULT_QUEUE").ok();
    queue_scope_from(args.get("queue"), env.as_deref())
}

/// Pure core of [`opt_queue_scoped`], so the no-default rule is
/// unit-testable without touching the process env.
pub(super) fn queue_scope_from(flag: Option<String>, env: Option<&str>) -> Option<String> {
    if let Some(q) = flag {
        return Some(q);
    }
    env.map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// #8606: why a claim refuses as bare, or None when it carries
/// intent. A claim with no task, no filter, no queue scope (flag
/// or env), no batch count, no handoff pick, and no identity
/// marks (worker, lane, session, pool, platform, from-session)
/// is a probing accident, never a meaningful intent: five
/// recorded firings (tasks 10945, 12706, 14884, 11421, 5865)
/// each stole the queue's top task under the default identity
/// while a lane probed the CLI's usage. Every documented claim
/// form names at least one of these — lanes pass --worker and
/// --lane, directed claims pass --task, designated lanes pass
/// --filter, scoped claims pass --queue — so the refusal fires
/// only on the zero-intent shape, before any broker contact.
pub(super) fn bare_claim_refusal(args: &Args, queue_scope: &Option<String>) -> Option<String> {
    if queue_scope.is_some() || args.has("handoff") || args.has("bind") {
        return None;
    }
    const INTENT_FLAGS: [&str; 10] = [
        "task",
        "filter",
        "queue",
        "count",
        "worker",
        "lane",
        "session-id",
        "pool",
        "platform",
        "from-session",
    ];
    if INTENT_FLAGS.iter().any(|name| args.get(name).is_some()) {
        return None;
    }
    Some(
        "claim: refusing a bare claim with no task, filter, queue, or identity \
         arguments — a bare claim is never a meaningful intent and has repeatedly \
         stolen the queue's top task during CLI probing (#8606). Pass the claim \
         you mean: --worker <w> --lane <l> for a lane claim, --task <id> for a \
         directed claim, or --filter '<expr>' / --queue <name> to scope it."
            .to_string(),
    )
}

pub(super) fn opt_worker(args: &Args) -> String {
    args.get("worker")
        .unwrap_or_else(|| DEFAULT_WORKER.to_string())
}

/// The caller's lane id for the holder-checked verbs (heartbeat,
/// checkpoint, park, release, block, fail, complete): `--lane`, else
/// `REPO_QUEUE_LANE` from the environment, else none (#6985). Every
/// scheduled lane shares one worker id, so the lane is what tells
/// the broker whose claim a verb acts on; with none, the broker
/// checks the worker alone, as before.
pub(super) fn lane_arg(args: &Args) -> Value {
    args.get("lane")
        .or_else(|| std::env::var("REPO_QUEUE_LANE").ok())
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .map_or(Value::Null, Value::String)
}

pub(super) fn load_packet(path: &str) -> Value {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => die(2, &format!("cannot read packet file: {e}")),
    };
    match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => die(2, &format!("packet file is not valid JSON: {e}")),
    }
}

pub(super) fn progress_arg(args: &Args) -> Option<Value> {
    let raw = args.get("progress");
    let pfile = args.get("progress-file");
    if raw.is_some() && pfile.is_some() {
        die(2, "pass only one of --progress / --progress-file");
    }
    let raw = if let Some(f) = pfile {
        match std::fs::read_to_string(&f) {
            Ok(t) => Some(t),
            Err(e) => die(2, &format!("cannot read progress file: {e}")),
        }
    } else {
        raw
    };
    let raw = raw?;
    let val: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => die(2, &format!("--progress is not valid JSON: {e}")),
    };
    if !val.is_object() {
        die(2, "--progress must be a JSON object");
    }
    Some(val)
}

/// #7598: two different "lane" vocabularies share the word. The
/// claim path's `--lane` takes a registry lane id (code-a, any-n);
/// `--execution.lane` historically took only the execution MODE
/// ('opencode' | 'direct') and died on a lane id, so a worker that
/// naturally passed its own lane id was rejected client-side and
/// retried with the surface name -- recording executed_lane as the
/// surface next to a claim stamped with the lane id: the recorded
/// claim-to-complete "lane mismatch". The split: `lane` keeps the
/// mode; a lane id passed in `lane` (or explicitly as `lane_id`)
/// is recorded under `lane_id` and the mode is derived from its
/// family (the opencode family executed via an opencode session;
/// every other family executed directly). Mirrors the broker's
/// lane_family_of (turso_queue.rs) family rule.
pub(super) fn execution_lane_id_family(lane_id: &str) -> Option<&'static str> {
    const FAMILIES: [&str; 7] = [
        "code",
        "any",
        "not-code",
        "opencode",
        "openchamber",
        "nix",
        "darwin-nix",
    ];
    let (family, suffix) = lane_id.rsplit_once('-')?;
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_lowercase()) {
        return None;
    }
    FAMILIES.iter().find(|f| **f == family).copied()
}

pub(super) fn normalize_execution(mut val: Value) -> Result<Value, String> {
    let mut lane_id: Option<String> = None;
    {
        let obj = val.as_object_mut().expect("execution object");
        if let Some(raw_id) = obj.get("lane_id").cloned() {
            let Some(id) = raw_id.as_str() else {
                return Err(
                    "--execution.lane_id must be a string lane id (e.g. 'code-a')".to_string(),
                );
            };
            if execution_lane_id_family(id).is_none() {
                return Err(format!(
                    "--execution.lane_id '{id}' is not a known lane id (family + lowercase suffix, e.g. 'code-a', 'any-n', 'opencode-a')"
                ));
            }
            lane_id = Some(id.to_string());
        }
        if let Some(raw_lane) = obj.get("lane").cloned() {
            let Some(lane) = raw_lane.as_str() else {
                return Err(
                    "--execution.lane must be a string: the execution mode 'opencode' or 'direct', or a lane id (e.g. 'code-a')"
                        .to_string(),
                );
            };
            if lane == "opencode" || lane == "direct" {
                // The mode, exactly as before.
            } else if let Some(family) = execution_lane_id_family(lane) {
                if let Some(existing) = &lane_id {
                    if existing != lane {
                        return Err(format!(
                            "--execution.lane '{lane}' and --execution.lane_id '{existing}' disagree"
                        ));
                    }
                }
                lane_id = Some(lane.to_string());
                let mode = if family == "opencode" {
                    "opencode"
                } else {
                    "direct"
                };
                obj.insert("lane".to_string(), json!(mode));
            } else {
                return Err(format!(
                    "--execution.lane '{lane}' is neither the execution mode ('opencode' or 'direct') nor a known lane id (e.g. 'code-a'); pass the mode in 'lane' and the executing lane in 'lane_id'"
                ));
            }
        }
        if let Some(id) = &lane_id {
            obj.insert("lane_id".to_string(), json!(id));
        }
    }
    Ok(val)
}

pub(super) fn execution_arg(args: &Args) -> Option<Value> {
    let raw = args.get("execution")?;
    let val: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => die(2, &format!("--execution is not valid JSON: {e}")),
    };
    if !val.is_object() {
        die(2, "--execution must be a JSON object");
    }
    match normalize_execution(val) {
        Ok(v) => Some(v),
        Err(e) => die(2, &e),
    }
}

pub(super) fn py_str_list(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|s| format!("'{s}'")).collect();
    format!("[{}]", inner.join(", "))
}

pub(super) fn check_telemetry_fields(obj: &Value, source: &str) {
    for field in TELEMETRY_FIELDS {
        let Some(v) = obj.get(field) else { continue };
        if v.is_null() {
            continue;
        }
        if field == "cost_usd" {
            let ok = matches!(v, Value::Number(n) if n.as_f64().map(|x| x >= 0.0).unwrap_or(false));
            if !ok {
                die(
                    2,
                    &format!(
                        "{source}: telemetry field 'cost_usd' must be a non-negative number or null"
                    ),
                );
            }
        } else if !matches!(v, Value::String(s) if !s.is_empty()) {
            die(
                2,
                &format!("{source}: telemetry field '{field}' must be a non-empty string or null"),
            );
        }
    }
}

/// Shared validate flow for `validate` and `enqueue`: returns warnings
/// after printing them; dies (exit 2) on errors exactly like
/// repo-queue.py's check_packet.
pub(super) fn validate_via_broker(packet: &Value, quiet: bool) -> Vec<String> {
    let resp = call("/validate", &json!({"packet": packet}));
    let resp = unwrap_ok(resp);
    let warnings: Vec<String> = resp
        .get("warnings")
        .and_then(|w| w.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|w| w.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let errors: Vec<String> = resp
        .get("errors")
        .and_then(|w| w.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|w| w.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    for w in &warnings {
        eprint(&format!("warning: {w}"));
    }
    if !errors.is_empty() {
        for e in &errors {
            eprint(&format!("error: {e}"));
        }
        die(
            2,
            &format!("packet validation failed ({} error(s))", errors.len()),
        );
    }
    if !quiet {
        eprint("packet valid (repo-task/v1)");
    }
    warnings
}

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------

/// #8097 Rule 2: a --platform value is a coarse FAMILY selector,
/// never a lane name: exactly "openchamber" or "muse". Shared by
/// the enqueue and claim verbs so both spell the rule once.
pub(super) fn platform_flag(v: &str) -> Result<String, String> {
    match v {
        "openchamber" | "muse" => Ok(v.to_string()),
        other => Err(format!(
            "invalid --platform {other:?} (expected \"openchamber\" or \"muse\")"
        )),
    }
}

/// #8176: the claim-model stamp flags. The claiming session reports
/// its own serving leaf — provider and model together (the leaf it
/// read from its session surface, route_usage's lastServing,
/// opencode#1891), variant optional. Naming either half alone is a
/// usage error, mirroring the broker's parse: a half-stamp must
/// never reach the wire.
pub(super) fn model_stamp_flags(
    provider: Option<&str>,
    model: Option<&str>,
    variant: Option<&str>,
) -> Result<Option<(String, String, Option<String>)>, String> {
    fn clean<'a>(v: Option<&'a str>) -> Option<&'a str> {
        v.map(str::trim).filter(|s| !s.is_empty())
    }
    match (clean(provider), clean(model)) {
        (None, None) => {
            if clean(variant).is_some() {
                return Err(
                    "--model-variant requires --model-provider and --model-id".to_string(),
                );
            }
            Ok(None)
        }
        (Some(p), Some(m)) => Ok(Some((
            p.to_string(),
            m.to_string(),
            clean(variant).map(str::to_string),
        ))),
        (Some(_), None) => Err(
            "--model-provider requires --model-id (the claim stamp names the serving leaf, provider and model together)"
                .to_string(),
        ),
        (None, Some(_)) => Err(
            "--model-id requires --model-provider (the claim stamp names the serving leaf, provider and model together)"
                .to_string(),
        ),
    }
}

/// The provider id that addresses configured routes, not serving
/// leaves: routes appear in the catalog under this provider
/// (opencode-route/<route-id>), so a model addressed under it is a
/// route alias, never the leaf that served a claim (#9408).
pub(super) const ROUTE_PROVIDER: &str = "opencode-route";

/// One stamp derivation, shared by the session-model path (id key
/// "id") and the route_usage lastServing path (id key "modelID"):
/// providerID + id, variant only when the record carries a real
/// effort variant (the server reports "default" when none was
/// chosen; that is the absence of a variant, not a variant).
pub(super) fn stamp_from(record: &Value, id_key: &str) -> Option<(String, String, Option<String>)> {
    let provider = record
        .get("providerID")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let id = record
        .get(id_key)
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let variant = record
        .get("variant")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "default");
    Some((
        provider.to_string(),
        id.to_string(),
        variant.map(str::to_string),
    ))
}

/// #8176 (2026-11-12 fix): read the claim stamp off a session
/// record (the OpenCode server's GET /api/session/<id> shape,
/// enveloped in `data` or bare): the session's own model —
/// providerID + id, variant only when the session carries a real
/// effort variant (the server reports "default" when none was
/// chosen; that is the absence of a variant, not a variant).
/// Pure: the oracle for the derivation below.
///
/// #9408: the stamp must name the SERVING leaf, not the declared
/// route. When the record carries the route_usage readout
/// (opencode#1891: route_usage.lastServing, providerID + modelID +
/// variant), it wins. Otherwise the session model is used only when
/// it is not a route alias: a model under the opencode-route
/// provider is the configured route (opencode-route/workhorse),
/// which says nothing about which leaf served the claim, so the
/// claim is left unstamped rather than permanently recording the
/// alias. (The deployed server's session records today carry no
/// route_usage field at all — verified 2026-11-14 — so routed
/// claims go unstamped until the readout reaches the record.)
pub(super) fn session_model_stamp(session: &Value) -> Option<(String, String, Option<String>)> {
    let inner = if session.get("model").is_some() {
        session
    } else {
        session.get("data")?
    };
    // The serving-leaf readout, when the record carries one: under
    // route_usage (the tool's output shape), or bare at the top of
    // the record. A present readout that does not parse stamps
    // nothing — falling back to the declared model would record
    // exactly the alias this path exists to avoid.
    let readout = inner
        .get("route_usage")
        .and_then(|ru| ru.get("lastServing"))
        .or_else(|| inner.get("lastServing"));
    if let Some(leaf) = readout {
        return stamp_from(leaf, "modelID").filter(|s| s.0 != ROUTE_PROVIDER);
    }
    let model = inner.get("model")?;
    let stamp = stamp_from(model, "id")?;
    if stamp.0 == ROUTE_PROVIDER {
        return None;
    }
    Some(stamp)
}

/// Minimal base64 (standard alphabet, padded) for the server's
/// Basic auth header — the crate carries no base64 dependency and
/// this is the only encoding the claim path needs.
pub(super) fn b64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let n = ((chunk[0] as u32) << 16)
            | ((*chunk.get(1).unwrap_or(&0) as u32) << 8)
            | (*chunk.get(2).unwrap_or(&0) as u32);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// OpenCode server base URL for the claim-stamp lookup (#9409),
/// resolved consistently with the claimant tooling
/// (pm-stack/opencode-claimant/claimant.py and poll.py): the
/// `OPENCODE_URL` override wins, then the legacy
/// `OPENCODE_SERVER_URL` this lookup originally read, then the
/// loopback default both sides share. Pure, so the resolution
/// order is unit-testable.
pub(super) fn resolve_opencode_base_url(
    env_url: Option<&str>,
    env_server_url: Option<&str>,
) -> String {
    for candidate in [env_url, env_server_url] {
        if let Some(u) = candidate
            .map(|v| v.trim().trim_end_matches('/'))
            .filter(|v| !v.is_empty())
        {
            return u.to_string();
        }
    }
    "http://127.0.0.1:13420".to_string()
}

/// OpenCode credential file for the claim-stamp lookup (#9409),
/// resolved consistently with the claimant tooling: the
/// `OPENCODE_AUTH_FILE` override wins, then the production state
/// file under the user's home. Pure, so the resolution is
/// unit-testable.
pub(super) fn resolve_opencode_auth_file(env_file: Option<&str>, home: &str) -> String {
    match env_file.map(str::trim).filter(|v| !v.is_empty()) {
        Some(f) => f.to_string(),
        None => format!("{home}/.local/state/opencode/service-prod.json"),
    }
}

/// #8176 (2026-11-12 fix): derive the claim stamp for a claim that
/// names its claiming session (--session-id) but carries no
/// explicit --model-* flags. The dispatch surfaces claim on behalf
/// of a live session and name it, but the #8176 wiring stopped at
/// the transfer path — the batch claims that carry the fleet's
/// volume never passed the flags, so in production the stamp
/// landed only on hand-driven proof claims (4 stamped claims of
/// 12,684 in the Nov 10-12 window, 3 of them worker `proof-8176`).
/// The session's own record is the authority (the same record
/// route_usage reads), fetched from the OpenCode server with its
/// Basic auth (password in the server's state file — claimant.py's
/// route; the value is used for the header only, never logged).
/// FAIL-SOFT by design: any failure — no state file, server
/// unreachable, session unknown, malformed record — returns None
/// and the claim proceeds exactly as an unstamped claim always
/// has; a claim must never fail because its stamp could not be
/// derived.
pub(super) fn derive_claim_model_stamp(
    session_id: &str,
) -> Option<(String, String, Option<String>)> {
    let session = fetch_session_record(session_id)?;
    session_model_stamp(&session)
}

/// Read one session record from the OpenCode server (its GET
/// /api/session/<id> shape, enveloped in `data` or bare). The
/// credential is the server's state file password, Basic auth with
/// the `opencode` user — the same route claimant.py takes; the
/// value is used for the header only, never logged. FAIL-SOFT: any
/// miss returns None so callers that derive from the record (the
/// claim stamp above, the completion telemetry below) degrade to
/// their other sources instead of failing the verb.
pub(super) fn fetch_session_record(session_id: &str) -> Option<Value> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return None;
    }
    let base = resolve_opencode_base_url(
        std::env::var("OPENCODE_URL").ok().as_deref(),
        std::env::var("OPENCODE_SERVER_URL").ok().as_deref(),
    );
    let home = crate::home::from_env().ok()?;
    let auth_file =
        resolve_opencode_auth_file(std::env::var("OPENCODE_AUTH_FILE").ok().as_deref(), &home);
    let state = std::fs::read_to_string(auth_file).ok()?;
    let state: Value = serde_json::from_str(&state).ok()?;
    let password = state.get("password")?.as_str()?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(true)
        .timeout_global(Some(Duration::from_secs(3)))
        .build()
        .into();
    let auth = format!(
        "Basic {}",
        b64_encode(format!("opencode:{password}").as_bytes())
    );
    let mut resp = agent
        .get(format!("{base}/api/session/{session_id}"))
        .header("Authorization", &auth)
        .call()
        .ok()?;
    let text = resp.body_mut().read_to_string().ok()?;
    serde_json::from_str(&text).ok()
}

/// A trimmed, non-empty string field of a JSON object, or None.
/// Used by the completion-telemetry derivation for both the claim
/// row and the session record.
pub(super) fn nonempty_str<'a>(obj: &'a Value, key: &str) -> Option<&'a str> {
    obj.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// The configured route (pool) a session was dispatched through:
/// its DECLARED model, and only when it is addressed under the
/// route provider (`opencode-route/<route-id>`). A session whose
/// declared model is a concrete provider/model was not dispatched
/// through a route, so it names no pool. This is the route the
/// session was configured with, never an effort inference from the
/// route name (meta-reviewer guideline 2026-10-27).
pub(super) fn session_declared_pool(session: &Value) -> Option<String> {
    let inner = if session.get("model").is_some() {
        session
    } else {
        session.get("data")?
    };
    let model = inner.get("model")?;
    if nonempty_str(model, "providerID")? != ROUTE_PROVIDER {
        return None;
    }
    nonempty_str(model, "id").map(str::to_string)
}

/// #4919: the completion telemetry a task's own record supports,
/// filled in by `complete` when the caller left a field out. Every
/// source is a record the queue already holds — nothing is
/// inferred from a route name and `cost_usd` is never invented
/// (there is no cost readout to source it from, so it stays null).
///
/// - `model`: the claim stamp's serving leaf, else the session
///   record's serving leaf. A claim stamped through the route
///   provider names the pool (route id) in `claim_model`, not a
///   leaf — that alias is skipped, exactly as
///   `session_model_stamp` refuses it, so the session leaf or an
///   honest null supplies the model instead.
/// - `effort`: the claim stamp's variant (a real one — the server
///   writes "default" when none was chosen), else the session
///   record's variant.
/// - `pool`: the session's declared route, per
///   `session_declared_pool`.
/// - `cost_usd`: always null.
///
/// The map ALWAYS carries all four keys, each null when no source
/// could supply it: a present null says "this run had no source",
/// which reports can distinguish from a field nobody filled in.
pub(super) fn derive_completion_telemetry(
    claim: &Value,
    session: Option<&Value>,
) -> Map<String, Value> {
    let leaf: Option<(String, String, Option<String>)> = session.and_then(session_model_stamp);
    // #4919: a route-provider claim stamp (the transfer path stamps
    // the session's declared route) holds the route id in
    // claim_model — the pool, not a serving leaf. Recording it as
    // the completion's model would write the alias this derivation
    // exists to avoid (measured on real rows: result.model =
    // "workhorse"), so skip it and let the session leaf — or null —
    // speak.
    let claim_model = match nonempty_str(claim, "claim_provider") {
        Some(p) if p == ROUTE_PROVIDER => None,
        _ => nonempty_str(claim, "claim_model"),
    };
    let model = claim_model
        .map(str::to_string)
        .or_else(|| leaf.as_ref().map(|s| s.1.clone()));
    let effort = nonempty_str(claim, "claim_variant")
        .filter(|v| *v != "default")
        .map(str::to_string)
        .or_else(|| leaf.as_ref().and_then(|s| s.2.clone()));
    let pool = session.and_then(session_declared_pool);
    let mut out = Map::new();
    out.insert("pool".to_string(), json!(pool));
    out.insert("model".to_string(), json!(model));
    out.insert("effort".to_string(), json!(effort));
    out.insert("cost_usd".to_string(), Value::Null);
    out
}

/// Merge derived telemetry into a completion result: a caller
/// supplied value always wins, a missing one is filled.
pub(super) fn merge_completion_telemetry(result_obj: &mut Value, derived: Map<String, Value>) {
    let Some(obj) = result_obj.as_object_mut() else {
        return;
    };
    for (field, value) in derived {
        obj.entry(field).or_insert(value);
    }
}

/// The session a completion's telemetry is derived from: the
/// completing session's id from the environment (set in the
/// worker's own session), else the claim row's bound session
/// (`progress.session_id`).
pub(super) fn completion_session_id(env_value: Option<&str>, claim: &Value) -> Option<String> {
    env_value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            claim
                .get("progress")
                .and_then(|p| p.get("session_id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
}

/// The claim row a completion reads its telemetry sources from.
/// `/list` with no queue name (the broker omits the predicate when
/// unnamed — #9420) so a task id resolves regardless of queue, and
/// fail-soft: an unreachable or unreadable broker leaves the
/// caller with the environment-derived sources only.
pub(super) fn completion_claim_row(task_id: i64) -> Option<Value> {
    if task_id <= 0 {
        return None;
    }
    let resp = call_result("/list", &json!({"rich": true, "task": task_id, "limit": 1})).ok()?;
    if resp.get("ok").and_then(|v| v.as_bool()) == Some(false) {
        return None;
    }
    let row = list_rows(&resp).ok()?.into_iter().next()?;
    (row.get("id").and_then(|v| v.as_i64()) == Some(task_id)).then_some(row)
}

/// #4919: fill absent completion telemetry at `complete` time. The
/// helper is the queue's single writer, so defaulting the fields
/// here is what makes them present regardless of whether a worker
/// remembered `--telemetry`. Explicit caller values (top-level
/// `--result` keys or `--telemetry`) always win; every field is
/// inserted, null when unsourceable.
pub(super) fn fill_completion_telemetry(task_id: i64, result_obj: &mut Value) {
    let claim = completion_claim_row(task_id).unwrap_or(Value::Null);
    let session_id =
        completion_session_id(std::env::var("OPENCODE_SESSION_ID").ok().as_deref(), &claim);
    let session = session_id.as_deref().and_then(fetch_session_record);
    let derived = derive_completion_telemetry(&claim, session.as_ref());
    merge_completion_telemetry(result_obj, derived);
}
