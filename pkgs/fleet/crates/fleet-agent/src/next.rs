//! `fleet agent next` — the next piece of work, down the intake ladder.
//!
//! One verb for every agent, whatever started it (a wake hook, a cron
//! run, a person). It tries four rungs in order and returns the first
//! task it finds:
//!
//! 1. **push** — a command addressed to this agent on the bridge. This
//!    is how central, a hook or another agent sends a task to one
//!    specific place.
//! 2. **queue** — a claim on the one fleet queue.
//! 3. **shortlist** — the top of this host's local shortlist. A run
//!    that finds the list low, with nobody refilling, is told to refill
//!    it instead (rung `refill`): that run switches purpose.
//! 4. **priority** — the highest-priority open issue, straight from
//!    GitHub labels.
//!
//! A rung that is down (the bridge unreachable, the broker not
//! answering, no `gh`) is recorded in `tried` and the ladder moves on.
//! Reaching nothing is never an error on its own: an agent on a seat
//! with only HTTPS still gets work from rungs 1 and 4.
//!
//! Nothing here treats a person as a gate. An issue labelled
//! `user/escalated` or blocked "for the owner" is handed out like any
//! other, with a note saying it was parked on the owner and that the
//! decision is the worker's to make (FDR-0013). Only the hold labels
//! of FDR-0012 keep an issue out.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::shortlist::{self, Entry, rank_of};
use crate::{
    DEFAULT_REPO, EXIT_NO_WORK, EXIT_OK, Out, R, World, argv, check_name, check_repo, parse, usage,
};

/// Labels that hold an issue (FDR-0012): a person set them on purpose.
pub const HOLD_LABELS: [&str; 3] = ["held", "user/owned", "do-not-auto-edit"];
/// Labels on an issue that indexes other work and is not itself a
/// piece of work to hand out.
pub const NOT_WORK_LABELS: [&str; 1] = ["kind/tracker"];
/// How an issue body marks a hands item
/// (`<!-- hands:v1 kind=... action="..." -->`).
pub const HANDS_MARKER: &str = "<!-- hands:v1 ";
/// Labels that say someone is already on it, or it waits on something real.
pub const BUSY_LABELS: [&str; 5] = [
    "status/in-progress",
    "status:in-review",
    "status/parked",
    "status/waiting-dependency",
    "status/deferred",
];
/// Labels an agent put on work it parked on the owner. Not holds.
pub const OWNER_PARK_LABELS: [&str; 2] = ["user/escalated", "status/blocked"];
/// The label queries the priority rung makes, soonest first.
pub const RANKED_LABELS: [&str; 5] = [
    "expedite",
    "priority/critical",
    "now",
    "priority/high",
    "next",
];

/// The lease, in seconds, that a queue claim taken by `next`
/// carries. Agent work runs one to three hours between the claim
/// and its completion, and nothing in this flow heartbeats, so the
/// queue's 45-minute default lease (2700s) lapses mid-task: the
/// claim evaporates, the task returns to the queue to be handed to
/// the next caller, and the worker's `complete` is refused
/// (#12618). Four hours covers the observed range with headroom
/// and stays inside the queue's one-day maximum; the hand-out
/// names the lease, so the deadline is visible instead of silent.
pub const AGENT_CLAIM_LEASE_SECONDS: u64 = 14_400;

fn encode_label(label: &str) -> String {
    label.replace('/', "%2F").replace(':', "%3A")
}

/// A way to reach GitHub from a seat: the fleet's token rotation
/// (`fleet gh`) or the seat's own signed-in `gh`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhRoute {
    Fleet,
    Plain,
}

impl GhRoute {
    pub fn name(self) -> &'static str {
        match self {
            GhRoute::Fleet => "fleet",
            GhRoute::Plain => "plain",
        }
    }

    pub fn parse(text: &str) -> Option<GhRoute> {
        match text.trim() {
            "fleet" => Some(GhRoute::Fleet),
            "plain" => Some(GhRoute::Plain),
            _ => None,
        }
    }

    fn other(self) -> GhRoute {
        match self {
            GhRoute::Fleet => GhRoute::Plain,
            GhRoute::Plain => GhRoute::Fleet,
        }
    }
}

/// Names the route to try first (`fleet` or `plain`).
pub const GH_ROUTE_ENV: &str = "FLEET_AGENT_GH";

/// The route to try first from this seat: what `FLEET_AGENT_GH`
/// says, else the route enrolment found working, else the fleet's
/// rotation. Only the order: the other route is still the fallback.
pub fn first_route(world: &dyn World) -> GhRoute {
    world
        .env(GH_ROUTE_ENV)
        .as_deref()
        .and_then(GhRoute::parse)
        .or_else(|| {
            crate::enroll::read_profile(world, None)
                .and_then(|p| p.gh)
                .as_deref()
                .and_then(GhRoute::parse)
        })
        .unwrap_or(GhRoute::Fleet)
}

/// `gh api <path>` by `first`, else by the other route. Returns the
/// answer and the route that gave it, so a caller making several
/// calls pays for a route that cannot answer only once.
fn gh_api(world: &dyn World, path: &str, first: GhRoute) -> Result<(Value, GhRoute), String> {
    let ask = |route: GhRoute| match route {
        GhRoute::Fleet => world.fleet(&argv(&["gh", "api", path])),
        GhRoute::Plain => world.program("gh", &argv(&["api", path])),
    };
    let mut whys = Vec::new();
    for route in [first, first.other()] {
        let out = ask(route);
        if out.ok() {
            if let Some(v) = out.json() {
                return Ok((v, route));
            }
        }
        let label = match route {
            GhRoute::Fleet => "fleet gh",
            GhRoute::Plain => "gh",
        };
        whys.push(format!("{label}: {}", out.why()));
    }
    Err(format!("GitHub did not answer ({})", whys.join("; ")))
}

fn label_names(issue: &Value) -> Vec<String> {
    issue
        .get("labels")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|l| {
                    l.get("name")
                        .and_then(Value::as_str)
                        .or_else(|| l.as_str())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Issues read per request, and how many pages of one rank are read
/// before moving on. Held, busy and pull-request rows are dropped after
/// a page is read, so one page is not enough to know a rank is empty.
const PER_PAGE: usize = 30;
const MAX_PAGES_PER_RANK: usize = 5;

/// Why an issue, as GitHub returns it, may not be handed to an agent;
/// None when it may. One rule for the ranking and for the recheck
/// before a hand-out.
fn kept_out(issue: &Value) -> Option<&'static str> {
    if issue.get("pull_request").is_some() {
        return Some("it is a pull request");
    }
    let labels = label_names(issue);
    if labels.iter().any(|l| HOLD_LABELS.contains(&l.as_str())) {
        return Some("it carries a hold label");
    }
    if labels.iter().any(|l| BUSY_LABELS.contains(&l.as_str())) {
        return Some("someone is on it");
    }
    if labels.iter().any(|l| NOT_WORK_LABELS.contains(&l.as_str())) {
        return Some("it is a tracker");
    }
    // A hands item waits on a person's hands (FDR-0013): no agent can
    // do it, so no agent is handed it.
    let hands_item = issue
        .get("body")
        .and_then(Value::as_str)
        .is_some_and(|body| body.contains(HANDS_MARKER));
    if hands_item {
        return Some("it is a hands item");
    }
    None
}

/// Open issues worth doing next in `repo`, soonest first: by label
/// rank, then oldest. Pull requests, held issues and issues someone is
/// already on are left out. Owner parks are kept.
pub fn candidates(world: &dyn World, repo: &str, want: usize) -> Result<Vec<Entry>, String> {
    let mut found: Vec<Entry> = Vec::new();
    let mut answered = false;
    let mut last_error = String::new();
    let mut route = first_route(world);
    for label in RANKED_LABELS {
        for page_number in 1..=MAX_PAGES_PER_RANK {
            if found.len() >= want {
                break;
            }
            let path = format!(
                "repos/{repo}/issues?state=open&labels={}&per_page={PER_PAGE}&sort=created&direction=asc&page={page_number}",
                encode_label(label)
            );
            let page = match gh_api(world, &path, route) {
                Ok((Value::Array(items), used)) => {
                    answered = true;
                    route = used;
                    items
                }
                Ok(_) => {
                    last_error = format!("GitHub's answer for label {label} was not a list");
                    break;
                }
                Err(e) => {
                    last_error = e;
                    break;
                }
            };
            for issue in &page {
                let Some(number) = issue.get("number").and_then(Value::as_u64) else {
                    continue;
                };
                if kept_out(issue).is_some() || found.iter().any(|e| e.issue == number) {
                    continue;
                }
                let labels = label_names(issue);
                found.push(Entry {
                    repo: repo.to_string(),
                    issue: number,
                    title: issue
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    rank: rank_of(&labels),
                    why: format!("label {label}"),
                    labels,
                    added_by: String::new(),
                    added_at: 0,
                });
            }
            // A short page is the last page of this rank.
            if page.len() < PER_PAGE {
                break;
            }
        }
    }
    if !answered {
        return Err(last_error);
    }
    found.sort_by_key(|e| (e.rank, e.issue));
    found.truncate(want);
    Ok(found)
}

/// What a recheck of a listed issue found.
pub enum Recheck {
    /// Still free: the entry with today's title and labels.
    Free(Entry),
    /// No longer to be handed out, and why.
    Gone(&'static str),
    /// GitHub did not answer, so nothing is known.
    Unknown(String),
}

/// Read one listed issue again before it is handed out. A shortlist
/// entry was free when it was added; a hold, a claim or a close since
/// then must win.
pub fn recheck(world: &dyn World, entry: &Entry) -> Recheck {
    let path = format!("repos/{}/issues/{}", entry.repo, entry.issue);
    let issue = match gh_api(world, &path, first_route(world)) {
        Ok((issue, _)) => issue,
        Err(e) => return Recheck::Unknown(e),
    };
    if issue.get("number").and_then(Value::as_u64) != Some(entry.issue) {
        return Recheck::Unknown("GitHub's answer was not this issue".to_string());
    }
    if issue.get("state").and_then(Value::as_str) != Some("open") {
        return Recheck::Gone("it is closed");
    }
    if let Some(why) = kept_out(&issue) {
        return Recheck::Gone(why);
    }
    let labels = label_names(&issue);
    Recheck::Free(Entry {
        title: issue
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(&entry.title)
            .to_string(),
        rank: rank_of(&labels),
        labels,
        ..entry.clone()
    })
}

/// The task an issue becomes, the same shape from rung 3 and rung 4.
fn owner_parked(labels: &[String]) -> bool {
    labels
        .iter()
        .any(|l| OWNER_PARK_LABELS.contains(&l.as_str()))
}

fn issue_task(entry: &Entry) -> Value {
    let parked = owner_parked(&entry.labels);
    let mut instructions = format!(
        "Land the next slice of {}#{} (.agents/skills/feature-slice): one PR that makes one \
         acceptance criterion pass or moves one measured number, with its test.",
        entry.repo, entry.issue
    );
    if parked {
        instructions.push_str(
            " This issue was parked on the owner. That is not a hold (FDR-0013): state what was \
             asked, decide it, record the decision on the issue, remove user/escalated and \
             status/blocked, and continue (.agents/skills/decide-and-proceed).",
        );
    }
    json!({
        "repo": entry.repo,
        "issue": entry.issue,
        "title": entry.title,
        "url": format!("https://github.com/{}/issues/{}", entry.repo, entry.issue),
        "rank": entry.rank,
        "why": entry.why,
        "labels": entry.labels,
        "owner_park": parked,
        "instructions": instructions,
    })
}

fn tried(list: &mut Vec<Value>, rung: &str, result: &str, why: impl Into<String>) {
    list.push(json!({"rung": rung, "result": result, "why": why.into()}));
}

/// How the broker spells a lane verdict: every refusal that is about
/// the lane itself — it belongs to another worker, it is disabled, it
/// is bound to another queue, its registration did not land — is a
/// `claim_repo_task: lane ...` error (fleet-queue's claim path).
const LANE_VERDICT_MARKER: &str = "claim_repo_task: lane ";

/// The lane verdict in a failed claim's answer, when the failure is
/// one (#6853, criterion 2). A verdict means the claim reached the
/// broker and the broker refused this lane: the queue rung is
/// `rejected`, not `down`, so a reader does not mistake a healthy
/// broker's refusal for a store outage and go fix the wrong thing.
///
/// The verdict reaches this process in two spellings of the same
/// answer: the queue verb's fallback switch puts it in the JSON
/// envelope's `error` field on stdout (and repeats it inside its
/// stderr wrapper line), while the plain die-on-error path carries
/// it as the error text itself. Both are searched.
fn lane_verdict(out: &Out) -> Option<String> {
    let envelope = out
        .json()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned));
    let texts: Vec<&str> = envelope
        .iter()
        .map(String::as_str)
        .chain([out.stderr.as_str(), out.stdout.as_str()])
        .collect();
    for text in texts {
        let Some(at) = text.find(LANE_VERDICT_MARKER) else {
            continue;
        };
        // The verdict runs to the end of its line; the fallback
        // wrapper appends its own sentence after a `;`, and a raw
        // envelope would continue with more JSON — stop at either.
        let verdict: String = text[at..]
            .lines()
            .next()
            .unwrap_or("")
            .split([';', '"'])
            .next()
            .unwrap_or("")
            .trim()
            .chars()
            .take(200)
            .collect();
        if !verdict.is_empty() {
            return Some(verdict);
        }
    }
    None
}

/// The queue validates lane IDs as a known family plus a lowercase
/// alphabetic suffix. Preserve a full lane ID supplied as the agent
/// name; otherwise give a standalone agent an unfiltered `any-*` lane.
pub(super) fn default_lane(agent: &str) -> String {
    let families = [
        "code",
        "any",
        "not-code",
        "opencode",
        "openchamber",
        "nix",
        "darwin-nix",
    ];
    let is_lane = agent.rsplit_once('-').is_some_and(|(family, suffix)| {
        families.contains(&family)
            && !suffix.is_empty()
            && suffix.bytes().all(|b| b.is_ascii_lowercase())
    });
    if is_lane {
        return agent.to_string();
    }
    // Lane rows are bound to their first worker, so two worker names
    // that normalize to the same slug must still get different lanes.
    // Keep 128 digest bits encoded as letters (two a-p letters per byte).
    let digest = Sha256::digest(agent.as_bytes());
    let digest_suffix: String = digest[..16]
        .iter()
        .flat_map(|byte| [b'a' + (byte >> 4), b'a' + (byte & 0x0f)])
        .map(char::from)
        .collect();
    let slug: String = agent
        .chars()
        .filter(|c| c.is_ascii_lowercase())
        .take(20)
        .collect();
    format!(
        "any-{}{digest_suffix}",
        if slug.is_empty() { "agent" } else { &slug }
    )
}

/// The broker binds registered lanes to workers. Agent names identify
/// the mailbox, but a known lane may belong to a shared queue worker
/// (for example `code-i` belongs to `repo-worker-code`).
fn registered_lane_worker(lane: &str) -> Option<&'static str> {
    // The `oc-*` scheduled lanes carry a registered family under an
    // `oc-` prefix (`oc-code-02`); the broker binds them to the same
    // shared worker as the bare family (#13073).
    if let Some(inner) = lane.strip_prefix("oc-") {
        return oc_lane_worker(inner);
    }
    if matches!(lane, "not-code-a" | "not-code-g") {
        return Some("repo-worker-nix");
    }
    if matches!(lane, "not-code-b" | "not-code-e") {
        return Some("repo-worker-opencode");
    }
    if lane == "not-code-d" {
        return Some("repo-worker-openchamber");
    }

    let (family, suffix) = lane.rsplit_once('-')?;
    if suffix.len() != 1 || !suffix.bytes().all(|b| b.is_ascii_lowercase()) {
        return None;
    }
    family_worker(family)
}

/// The shared worker a registered lane family binds to, by family
/// name alone.
fn family_worker(family: &str) -> Option<&'static str> {
    match family {
        "code" | "any" | "not-code" => Some("repo-worker-code"),
        "opencode" => Some("repo-worker-opencode"),
        "openchamber" => Some("repo-worker-openchamber"),
        "nix" => Some("repo-worker-nix"),
        _ => None,
    }
}

/// An `oc-*` lane with the prefix stripped. A letter suffix resolves
/// exactly like the bare lane (`oc-not-code-a` is `not-code-a` under
/// the prefix); the scheduled lanes' numeric suffix (`oc-code-02`)
/// resolves by family. An unknown family stays unregistered, so the
/// caller still falls back to the agent name.
fn oc_lane_worker(inner: &str) -> Option<&'static str> {
    if let Some(worker) = registered_lane_worker(inner) {
        return Some(worker);
    }
    let (family, suffix) = inner.rsplit_once('-')?;
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    family_worker(family)
}

/// How many listed entries in a row may turn out stale in one run
/// before the shortlist rung gives way to the priority rung.
const MAX_STALE_ENTRIES: usize = 5;

enum FromShortlist {
    Take(Entry),
    Refill,
    Empty,
}

/// `fleet agent next`.
pub fn next(world: &dyn World, args: &[String]) -> R<(Value, i32)> {
    let a = parse(
        args,
        &["--no-push", "--no-queue", "--no-shortlist", "--no-priority"],
        &[
            "--agent",
            "--lane",
            "--repo",
            "--low-water",
            "--shortlist",
            "--profile",
            "--platform",
            "--pool",
            "--remaining-budget-minutes",
            "--filter",
            "--queue",
        ],
    )?;
    a.no_positional("next")?;
    let profile = crate::enroll::read_profile(world, a.last("--profile"));
    let agent = a
        .last("--agent")
        .or_else(|| world.env("FLEET_AGENT"))
        .or_else(|| profile.as_ref().map(|p| p.name.clone()))
        .ok_or_else(|| {
            usage(
                "who is asking? pass --agent <name>, set FLEET_AGENT, or run \
                 `fleet agent enroll` once on this seat",
            )
        })?;
    let agent = check_name("--agent", &agent)?;
    let lane = match a.last("--lane") {
        Some(l) => check_name("--lane", &l)?,
        None => default_lane(&agent),
    };
    let repo = check_repo(&a.last("--repo").unwrap_or_else(|| DEFAULT_REPO.to_string()))?;
    let low_water = a
        .uint("--low-water")?
        .unwrap_or(shortlist::DEFAULT_LOW_WATER);
    let now = world.now();
    // What the caller chose is carried into every follow-up command,
    // so the next step reads the same list, repo and profile.
    let mut carried = String::new();
    for flag in ["--repo", "--shortlist", "--profile"] {
        if let Some(value) = a.last(flag) {
            carried.push_str(&format!(" {flag} {}", crate::shell::sh_quote(&value)));
        }
    }
    let again = format!("fleet agent next --agent {agent} --lane {lane}{carried}");
    let mut log: Vec<Value> = Vec::new();
    let answer = |rung: &str, task: Value, then: Vec<String>, log: Vec<Value>| {
        (
            json!({"agent": agent, "rung": rung, "task": task, "then": then, "tried": log}),
            EXIT_OK,
        )
    };

    // Rung 1: a command pushed to this agent.
    if a.has("--no-push") {
        tried(&mut log, "push", "skipped", "--no-push");
    } else {
        let out = world.fleet(&argv(&[
            "bridge",
            "inbox",
            "--agent",
            &agent,
            "--commands",
            "--take",
            "--limit",
            "1",
        ]));
        let first = out
            .json()
            .and_then(|v| v.get("messages").and_then(Value::as_array).cloned())
            .and_then(|m| m.into_iter().next());
        match (out.ok(), first) {
            (true, Some(message)) => {
                let id = message.get("id").and_then(Value::as_u64).unwrap_or(0);
                let task = json!({
                    "command_id": id,
                    "instructions": message.get("body").and_then(Value::as_str).unwrap_or(""),
                    "message": message,
                });
                let then = vec![
                    format!(
                        "fleet bridge post --agent {agent} --reply-to {id} \"done: <one line, link if any>\""
                    ),
                    format!(
                        "fleet bridge post --agent {agent} --reply-to {id} \"could not: <reason>\""
                    ),
                ];
                return Ok(answer("push", task, then, log));
            }
            (true, None) => tried(&mut log, "push", "empty", "no command waiting"),
            (false, _) => tried(&mut log, "push", "down", out.why()),
        }
    }

    // Rung 2: a claim on the fleet queue.
    if a.has("--no-queue") {
        tried(&mut log, "queue", "skipped", "--no-queue");
    } else {
        let lease = AGENT_CLAIM_LEASE_SECONDS.to_string();
        let worker = registered_lane_worker(&lane).unwrap_or(&agent);
        let mut claim = argv(&[
            "queue",
            "claim",
            "--worker",
            worker,
            "--lane",
            &lane,
            "--lease-seconds",
            &lease,
        ]);
        for flag in [
            "--platform",
            "--pool",
            "--remaining-budget-minutes",
            "--filter",
            "--queue",
        ] {
            if let Some(v) = a.last(flag) {
                claim.push(flag.to_string());
                claim.push(v);
            }
        }
        // A claim changes the queue: it is given time to finish.
        let out = world.fleet_patient(&claim);
        let row = out
            .json()
            .filter(|v| v.get("id").is_some_and(|id| !id.is_null()));
        match (out.code, row) {
            (0, Some(row)) => {
                let id = row.get("id").cloned().unwrap_or(Value::Null);
                let task = json!({
                    "queue_task_id": id,
                    "title": row.get("title").cloned().unwrap_or(Value::Null),
                    "row": row,
                    "lease_seconds": AGENT_CLAIM_LEASE_SECONDS,
                    "instructions": format!(
                        "Work the packet as it says. Checkpoint as you go; finish with \
                         complete, or park with a resume condition that is not a person. \
                         Your claim lease is {} seconds (4 hours) from the claim: \
                         complete or park before it lapses, or the task returns to the \
                         queue and your completion will be refused.",
                        AGENT_CLAIM_LEASE_SECONDS
                    ),
                });
                let then = vec![
                    format!("fleet queue checkpoint {id} --worker {worker} --lane {lane} ..."),
                    format!("fleet queue complete {id} --worker {worker} --lane {lane} ..."),
                ];
                return Ok(answer("queue", task, then, log));
            }
            (0, None) | (EXIT_NO_WORK, _) => tried(&mut log, "queue", "empty", "no task claimable"),
            _ => match lane_verdict(&out) {
                Some(verdict) => tried(&mut log, "queue", "rejected", verdict),
                None => tried(&mut log, "queue", "down", out.why()),
            },
        }
    }

    // Rung 3: this host's shortlist, or the purpose switch to refill it.
    let list_path = shortlist::path(world, a.last("--shortlist"));
    if a.has("--no-shortlist") {
        tried(&mut log, "shortlist", "skipped", "--no-shortlist");
    } else {
        // An entry was free when it was listed. It is read again before
        // it is handed out, so a hold or a claim made since then wins;
        // a few stale entries in a row are dropped, not handed out.
        let mut dropped: Vec<String> = Vec::new();
        let step = loop {
            let step = list_path.clone().and_then(|file| {
                shortlist::update(&file, now, |list| {
                    if list.needs_refill(low_water, now) {
                        list.refill = Some((agent.clone(), now));
                        FromShortlist::Refill
                    } else {
                        match list.take(&agent, now) {
                            Some(entry) => FromShortlist::Take(entry),
                            None => FromShortlist::Empty,
                        }
                    }
                })
            });
            let Ok(FromShortlist::Take(entry)) = step else {
                break step.map(|s| (s, None));
            };
            match recheck(world, &entry) {
                Recheck::Free(fresh) => break Ok((FromShortlist::Take(fresh), None)),
                Recheck::Unknown(why) => break Ok((FromShortlist::Take(entry), Some(why))),
                Recheck::Gone(why) => {
                    dropped.push(format!("#{} dropped: {why}", entry.issue));
                    if dropped.len() >= MAX_STALE_ENTRIES {
                        break Ok((FromShortlist::Empty, None));
                    }
                }
            }
        };
        let stale = dropped.join("; ");
        match step {
            Ok((FromShortlist::Take(entry), unchecked)) => {
                let mut task = issue_task(&entry);
                task["rechecked"] = json!(unchecked.is_none());
                if let Some(why) = unchecked {
                    task["recheck_note"] = json!(format!(
                        "GitHub did not answer, so this entry's labels are as listed. Read the \
                         issue for a hold before you start. ({why})"
                    ));
                }
                let then = vec![
                    "post one progress note on the issue when a PR is open".to_string(),
                    again.clone(),
                ];
                return Ok(answer("shortlist", task, then, log));
            }
            Ok((FromShortlist::Refill, _)) => {
                // Best effort: hand the filler what GitHub ranks highest.
                let (found, note) = match candidates(world, &repo, 16) {
                    Ok(found) => (found, String::new()),
                    Err(e) => (Vec::new(), e),
                };
                let task = json!({
                    "purpose": "refill-shortlist",
                    "target": shortlist::DEFAULT_TARGET,
                    "instructions": "The shortlist on this host is low and nobody is refilling it. \
                        Your work this run is to fill it. Pick issues an agent can start now: \
                        prefer the active tracks in planning/plans/2026-11-21_results-first.md, \
                        skip anything with a hold label, and add each with a one-line reason. \
                        `fleet agent shortlist fill` adds the ranked candidates as they stand; use \
                        `fleet agent shortlist add` where you can do better than the ranking.",
                    "candidates": found.iter().map(issue_task).collect::<Vec<_>>(),
                    "candidates_note": note,
                    "dropped": stale,
                });
                let list_flag = a
                    .last("--shortlist")
                    .map(|p| format!(" --shortlist {}", crate::shell::sh_quote(&p)))
                    .unwrap_or_default();
                let then = vec![
                    format!("fleet agent shortlist fill --agent {agent} --repo {repo}{list_flag}"),
                    format!(
                        "fleet agent shortlist add --agent {agent} --repo {repo}{list_flag} --issue <n> --title <t> --why <reason>"
                    ),
                    again.clone(),
                ];
                return Ok(answer("refill", task, then, log));
            }
            Ok((FromShortlist::Empty, _)) => tried(
                &mut log,
                "shortlist",
                "empty",
                if stale.is_empty() {
                    "nothing is listed, and it was filled or claimed for filling recently"
                        .to_string()
                } else {
                    stale
                },
            ),
            Err(e) => tried(&mut log, "shortlist", "down", e.to_string()),
        }
    }

    // Rung 4: whatever has the highest priority.
    if a.has("--no-priority") {
        tried(&mut log, "priority", "skipped", "--no-priority");
    } else {
        match candidates(world, &repo, 12) {
            Ok(found) => {
                // Choose and reserve in one locked update, so two runs
                // on this host at the same moment cannot both take the
                // same issue. A host that cannot write the file still
                // gets a task: the first candidate.
                let pick = match &list_path {
                    Ok(file) => shortlist::reserve(file, now, &agent, &found)
                        .unwrap_or_else(|_| found.first().cloned()),
                    Err(_) => found.first().cloned(),
                };
                match pick {
                    Some(entry) => {
                        let then = vec![
                            "post one progress note on the issue when a PR is open".to_string(),
                            again.clone(),
                        ];
                        return Ok(answer("priority", issue_task(&entry), then, log));
                    }
                    None => tried(
                        &mut log,
                        "priority",
                        "empty",
                        "no open issue with a priority label is free",
                    ),
                }
            }
            Err(e) => tried(&mut log, "priority", "down", e),
        }
    }

    Ok((
        json!({"agent": agent, "rung": "none", "task": null, "then": [], "tried": log}),
        EXIT_NO_WORK,
    ))
}
