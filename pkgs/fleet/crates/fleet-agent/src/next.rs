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

use crate::shortlist::{self, Entry, rank_of};
use crate::{
    DEFAULT_REPO, EXIT_NO_WORK, EXIT_OK, R, World, argv, check_name, check_repo, parse, usage,
};

/// Labels that hold an issue (FDR-0012): a person set them on purpose.
pub const HOLD_LABELS: [&str; 3] = ["held", "user/owned", "do-not-auto-edit"];
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

/// Open issues worth doing next in `repo`, soonest first: by label
/// rank, then oldest. Pull requests, held issues and issues someone is
/// already on are left out. Owner parks are kept.
pub fn candidates(world: &dyn World, repo: &str, want: usize) -> Result<Vec<Entry>, String> {
    let mut found: Vec<Entry> = Vec::new();
    let mut answered = false;
    let mut last_error = String::new();
    let mut route = first_route(world);
    for label in RANKED_LABELS {
        if found.len() >= want {
            break;
        }
        let path = format!(
            "repos/{repo}/issues?state=open&labels={}&per_page=30&sort=created&direction=asc",
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
                continue;
            }
            Err(e) => {
                last_error = e;
                continue;
            }
        };
        for issue in &page {
            if issue.get("pull_request").is_some() {
                continue;
            }
            let Some(number) = issue.get("number").and_then(Value::as_u64) else {
                continue;
            };
            let labels = label_names(issue);
            let kept_out = labels
                .iter()
                .any(|l| HOLD_LABELS.contains(&l.as_str()) || BUSY_LABELS.contains(&l.as_str()));
            if kept_out || found.iter().any(|e| e.issue == number) {
                continue;
            }
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
    }
    if !answered {
        return Err(last_error);
    }
    found.sort_by_key(|e| (e.rank, e.issue));
    found.truncate(want);
    Ok(found)
}

fn owner_parked(labels: &[String]) -> bool {
    labels
        .iter()
        .any(|l| OWNER_PARK_LABELS.contains(&l.as_str()))
}

/// The task an issue becomes, the same shape from rung 3 and rung 4.
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
        None => agent.clone(),
    };
    let repo = check_repo(&a.last("--repo").unwrap_or_else(|| DEFAULT_REPO.to_string()))?;
    let low_water = a
        .uint("--low-water")?
        .unwrap_or(shortlist::DEFAULT_LOW_WATER);
    let now = world.now();
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
        let mut claim = argv(&["queue", "claim", "--worker", &agent, "--lane", &lane]);
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
                    "instructions": "Work the packet as it says. Checkpoint as you go; finish with \
                                     complete, or park with a resume condition that is not a person.",
                });
                let then = vec![
                    format!("fleet queue checkpoint {id} --worker {agent} --lane {lane} ..."),
                    format!("fleet queue complete {id} --worker {agent} --lane {lane} ..."),
                ];
                return Ok(answer("queue", task, then, log));
            }
            (0, None) | (EXIT_NO_WORK, _) => tried(&mut log, "queue", "empty", "no task claimable"),
            _ => tried(&mut log, "queue", "down", out.why()),
        }
    }

    // Rung 3: this host's shortlist, or the purpose switch to refill it.
    let list_path = shortlist::path(world, a.last("--shortlist"));
    if a.has("--no-shortlist") {
        tried(&mut log, "shortlist", "skipped", "--no-shortlist");
    } else {
        let step = list_path.clone().and_then(|file| {
            shortlist::update(&file, now, |list| {
                if list.is_low(low_water) && list.refill.is_none() {
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
        match step {
            Ok(FromShortlist::Take(entry)) => {
                let then = vec![
                    "post one progress note on the issue when a PR is open".to_string(),
                    format!("fleet agent next --agent {agent}"),
                ];
                return Ok(answer("shortlist", issue_task(&entry), then, log));
            }
            Ok(FromShortlist::Refill) => {
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
                });
                let then = vec![
                    format!("fleet agent shortlist fill --agent {agent} --repo {repo}"),
                    format!(
                        "fleet agent shortlist add --agent {agent} --issue <n> --title <t> --why <reason>"
                    ),
                    format!("fleet agent next --agent {agent}"),
                ];
                return Ok(answer("refill", task, then, log));
            }
            Ok(FromShortlist::Empty) => tried(
                &mut log,
                "shortlist",
                "empty",
                "another run is refilling it",
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
                // Do not hand out what this host handed out in the last two hours.
                let handed = list_path
                    .as_ref()
                    .ok()
                    .and_then(|file| shortlist::load(file).ok())
                    .map(|mut list| {
                        list.prune(now);
                        list
                    })
                    .unwrap_or_default();
                let pick = found
                    .into_iter()
                    .find(|e| !handed.recently_handed(&e.repo, e.issue));
                match pick {
                    Some(entry) => {
                        if let Ok(file) = &list_path {
                            // Remembering the hand-out is best effort: a host
                            // that cannot write the file still gets its task.
                            let _ = shortlist::update(file, now, |list| {
                                list.handed.push(shortlist::Handed {
                                    repo: entry.repo.clone(),
                                    issue: entry.issue,
                                    to: agent.clone(),
                                    at: now,
                                });
                            });
                        }
                        let then = vec![
                            "post one progress note on the issue when a PR is open".to_string(),
                            format!("fleet agent next --agent {agent}"),
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
