//! Tests for the agent domain and `fleet shell`.
//!
//! Oracles:
//!
//! * the intake ladder's contract is the design's four rungs in order;
//!   each test scripts what the legs answer and states the rung that
//!   must win, by hand;
//! * the shortlist's order is "lowest rank, then oldest": the property
//!   test checks it against a sort of the same entries;
//! * shell quoting is checked against `/bin/sh` itself: a quoted word,
//!   echoed back by a real shell, must equal the original.
//!
//! No test touches the network, the real bridge, the queue or `gh`:
//! every sub-invocation goes through [`Fake`], and a call no test
//! scripted answers exit 127 so an unplanned call cannot pass quietly.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use proptest::prelude::*;
use serde_json::{Value, json};

use crate::shell::{self, Options, Target};
use crate::shortlist::{self, Entry, Shortlist};
use crate::{AgentError, EXIT_DOWN, EXIT_NO_WORK, EXIT_OK, Out, World, argv, execute};

static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

struct Fake {
    /// (needle, answers): the first rule whose needle occurs in the
    /// joined command answers; a rule's last answer repeats.
    rules: RefCell<Vec<(String, Vec<Out>)>>,
    calls: RefCell<Vec<String>>,
    attached: RefCell<Vec<String>>,
    env: RefCell<HashMap<String, String>>,
    now: Cell<u64>,
    dir: PathBuf,
}

impl Fake {
    fn new() -> Fake {
        let dir = std::env::temp_dir().join(format!(
            "fleet-agent-test-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let fake = Fake {
            rules: RefCell::new(Vec::new()),
            calls: RefCell::new(Vec::new()),
            attached: RefCell::new(Vec::new()),
            env: RefCell::new(HashMap::new()),
            now: Cell::new(1_800_000_000),
            dir,
        };
        fake.set_env("FLEET_SHORTLIST", fake.shortlist_path().to_str().unwrap());
        fake.set_env("FLEET_AGENT_PROFILE", fake.profile_path().to_str().unwrap());
        fake
    }
    fn shortlist_path(&self) -> PathBuf {
        self.dir.join("shortlist.json")
    }
    fn profile_path(&self) -> PathBuf {
        self.dir.join("agent.json")
    }
    fn set_env(&self, key: &str, value: &str) {
        self.env
            .borrow_mut()
            .insert(key.to_string(), value.to_string());
    }
    fn on(&self, needle: &str, answers: Vec<Out>) -> &Fake {
        self.rules.borrow_mut().push((needle.to_string(), answers));
        self
    }
    fn answer(&self, command: String) -> Out {
        self.calls.borrow_mut().push(command.clone());
        let mut rules = self.rules.borrow_mut();
        for (needle, answers) in rules.iter_mut() {
            if command.contains(needle.as_str()) {
                return if answers.len() > 1 {
                    answers.remove(0)
                } else {
                    answers[0].clone()
                };
            }
        }
        Out {
            code: 127,
            stdout: String::new(),
            stderr: format!("not scripted: {command}"),
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
    fn called(&self, needle: &str) -> bool {
        self.calls().iter().any(|c| c.contains(needle))
    }
    fn seed(&self, entries: Vec<Entry>) {
        shortlist::update(&self.shortlist_path(), self.now.get(), |list| {
            list.entries = entries;
        })
        .expect("seed the shortlist");
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl World for Fake {
    fn fleet(&self, args: &[String]) -> Out {
        self.answer(format!("fleet {}", args.join(" ")))
    }
    fn program(&self, program: &str, args: &[String]) -> Out {
        self.answer(format!("{program} {}", args.join(" ")))
    }
    fn attach(&self, program: &str, args: &[String]) -> i32 {
        self.attached
            .borrow_mut()
            .push(format!("{program} {}", args.join(" ")));
        0
    }
    fn env(&self, key: &str) -> Option<String> {
        self.env.borrow().get(key).cloned()
    }
    fn now(&self) -> u64 {
        self.now.get()
    }
    fn home(&self) -> Option<PathBuf> {
        Some(self.dir.clone())
    }
}

fn ok(body: Value) -> Out {
    Out {
        code: 0,
        stdout: body.to_string(),
        stderr: String::new(),
    }
}

fn fail(code: i32, stderr: &str) -> Out {
    Out {
        code,
        stdout: String::new(),
        stderr: stderr.to_string(),
    }
}

fn entry(issue: u64, rank: u64, added_at: u64) -> Entry {
    Entry {
        repo: "developing-today/code".to_string(),
        issue,
        title: format!("issue {issue}"),
        rank,
        why: "test".to_string(),
        labels: Vec::new(),
        added_by: "seed".to_string(),
        added_at,
    }
}

fn issue(number: u64, labels: &[&str]) -> Value {
    json!({
        "number": number,
        "title": format!("issue {number}"),
        "labels": labels.iter().map(|l| json!({"name": l})).collect::<Vec<_>>(),
    })
}

fn run(fake: &Fake, words: &[&str]) -> (Value, i32) {
    execute(fake, &argv(words)).expect("verb runs")
}

fn rung_results(out: &Value) -> Vec<(String, String)> {
    out["tried"]
        .as_array()
        .expect("tried is a list")
        .iter()
        .map(|t| {
            (
                t["rung"].as_str().unwrap().to_string(),
                t["result"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

const INBOX: &str = "fleet bridge inbox";
const CLAIM: &str = "fleet queue claim";
const EMPTY_INBOX: &str = r#"{"messages":[],"next_since_id":0}"#;

fn empty_inbox() -> Out {
    Out {
        code: 0,
        stdout: EMPTY_INBOX.to_string(),
        stderr: String::new(),
    }
}

fn no_claim() -> Out {
    Out {
        code: 5,
        stdout: r#"{"task":null}"#.to_string(),
        stderr: String::new(),
    }
}

// ---- the intake ladder ----

#[test]
fn a_pushed_command_wins_and_the_queue_is_never_asked() {
    let fake = Fake::new();
    fake.on(
        INBOX,
        vec![ok(
            json!({"messages": [{"id": 77, "kind": "command", "body": "rebase PR 12345 on main"}],
                       "next_since_id": 77}),
        )],
    );
    let (out, code) = run(&fake, &["next", "--agent", "muse-3"]);
    assert_eq!(code, EXIT_OK);
    assert_eq!(out["rung"], "push");
    assert_eq!(out["task"]["command_id"], 77);
    assert_eq!(out["task"]["instructions"], "rebase PR 12345 on main");
    // The answer names how to close the loop, with this command's id.
    assert!(out["then"][0].as_str().unwrap().contains("--reply-to 77"));
    // --take with --limit 1: one command is marked read, never a batch.
    assert!(fake.called("fleet bridge inbox --agent muse-3 --commands --take --limit 1"));
    assert!(
        !fake.called(CLAIM),
        "a pushed task must not also claim: {:?}",
        fake.calls()
    );
}

#[test]
fn an_empty_mailbox_falls_to_a_queue_claim() {
    let fake = Fake::new();
    fake.on(INBOX, vec![empty_inbox()]).on(
        CLAIM,
        vec![ok(
            json!({"id": 10811, "queue": "fleet", "status": "claimed",
                       "title": "Add claim tests (#5486)"}),
        )],
    );
    let (out, code) = run(
        &fake,
        &[
            "next",
            "--agent",
            "oc-go-1",
            "--lane",
            "go",
            "--platform",
            "opencode",
            "--remaining-budget-minutes",
            "90",
        ],
    );
    assert_eq!(code, EXIT_OK);
    assert_eq!(out["rung"], "queue");
    assert_eq!(out["task"]["queue_task_id"], 10811);
    assert_eq!(
        rung_results(&out),
        vec![("push".to_string(), "empty".to_string())]
    );
    // The claim carries the agent as worker, the lane, and the claimer's own flags.
    assert!(fake.called(
        "fleet queue claim --worker oc-go-1 --lane go --platform opencode --remaining-budget-minutes 90"
    ));
}

#[test]
fn legs_that_are_down_are_named_and_the_shortlist_answers() {
    let fake = Fake::new();
    fake.on(
        INBOX,
        vec![fail(3, "GET to the bridge failed: connection refused")],
    )
    .on(CLAIM, vec![fail(3, "queue broker unreachable")]);
    // Four entries is above the low-water mark of three: a take, not a refill.
    fake.seed(vec![
        entry(900, 5, 10),
        entry(557, 2, 20),
        entry(805, 2, 15),
        entry(1062, 3, 5),
    ]);
    let (out, code) = run(&fake, &["next", "--agent", "muse-3"]);
    assert_eq!(code, EXIT_OK);
    assert_eq!(out["rung"], "shortlist");
    // Lowest rank first (2), then the older of the two rank-2 entries (#805, added at 15).
    assert_eq!(out["task"]["issue"], 805);
    let tried = rung_results(&out);
    assert_eq!(
        tried,
        vec![
            ("push".to_string(), "down".to_string()),
            ("queue".to_string(), "down".to_string())
        ]
    );
    assert!(
        out["tried"][0]["why"]
            .as_str()
            .unwrap()
            .contains("connection refused")
    );
    // The taken entry is gone from the file and remembered as handed out.
    let list = shortlist::load(&fake.shortlist_path()).unwrap();
    assert_eq!(list.entries.len(), 3);
    assert!(list.recently_handed("developing-today/code", 805));
}

#[test]
fn a_low_shortlist_makes_this_run_the_filler_and_only_this_run() {
    let fake = Fake::new();
    fake.on(INBOX, vec![empty_inbox()])
        .on(CLAIM, vec![no_claim()])
        .on(
            "labels=now",
            vec![ok(json!([issue(557, &["now"]), issue(1067, &["now"])]))],
        )
        .on("gh api", vec![ok(json!([]))]);
    fake.seed(vec![entry(900, 5, 10), entry(901, 5, 11)]); // two entries: at or under three
    let (out, code) = run(&fake, &["next", "--agent", "muse-3"]);
    assert_eq!(code, EXIT_OK);
    assert_eq!(out["rung"], "refill");
    assert_eq!(out["task"]["purpose"], "refill-shortlist");
    // The filler is handed what GitHub ranks highest, to start from.
    assert_eq!(out["task"]["candidates"][0]["issue"], 557);
    // Nothing was taken: the two entries are still there, and the marker names the filler.
    let list = shortlist::load(&fake.shortlist_path()).unwrap();
    assert_eq!(list.entries.len(), 2);
    assert_eq!(list.refill, Some(("muse-3".to_string(), fake.now.get())));

    // A second run while the marker is fresh keeps working: it takes an entry.
    let (second, _) = run(&fake, &["next", "--agent", "muse-4"]);
    assert_eq!(second["rung"], "shortlist");
    assert_eq!(second["task"]["issue"], 900);

    // Past the marker's window the next low run becomes the filler.
    fake.now.set(fake.now.get() + shortlist::REFILL_FRESH_SECS);
    let (third, _) = run(&fake, &["next", "--agent", "muse-5"]);
    assert_eq!(third["rung"], "refill");
}

#[test]
fn the_priority_rung_skips_holds_and_keeps_owner_parks() {
    let fake = Fake::new();
    fake.on(INBOX, vec![empty_inbox()])
        .on(CLAIM, vec![no_claim()])
        .on(
            "labels=now",
            vec![ok(json!([
                {"number": 12000, "title": "a PR", "labels": [{"name": "now"}], "pull_request": {}},
                issue(100, &["now", "held"]),
                issue(101, &["now", "status/in-progress"]),
                issue(805, &["now", "user/escalated", "status/blocked"]),
                issue(557, &["now"]),
            ]))],
        )
        .on("gh api", vec![ok(json!([]))]);
    let (out, code) = run(&fake, &["next", "--agent", "muse-3", "--no-shortlist"]);
    assert_eq!(code, EXIT_OK);
    assert_eq!(out["rung"], "priority");
    // #100 is held and #101 is being worked: both stay out. #557 is free and
    // older issues sort first by number within a rank, so it is picked.
    assert_eq!(out["task"]["issue"], 557);
    assert_eq!(out["task"]["owner_park"], false);

    // The same host does not hand #557 out twice: the next pick is the owner park.
    let (second, _) = run(&fake, &["next", "--agent", "muse-4", "--no-shortlist"]);
    assert_eq!(second["task"]["issue"], 805);
    assert_eq!(second["task"]["owner_park"], true);
    let text = second["task"]["instructions"].as_str().unwrap();
    assert!(text.contains("not a hold (FDR-0013)"), "{text}");
    assert!(text.contains("decide-and-proceed"), "{text}");
}

#[test]
fn nothing_on_any_rung_is_exit_five_with_every_rung_accounted_for() {
    let fake = Fake::new();
    fake.on(INBOX, vec![empty_inbox()])
        .on(CLAIM, vec![no_claim()])
        .on("gh api", vec![ok(json!([]))]);
    // An empty list is low, so the first run refills; make someone else the filler.
    shortlist::update(&fake.shortlist_path(), fake.now.get(), |list| {
        list.refill = Some(("muse-9".to_string(), fake.now.get()));
    })
    .unwrap();
    let (out, code) = run(&fake, &["next", "--agent", "muse-3"]);
    assert_eq!(code, EXIT_NO_WORK);
    assert_eq!(out["rung"], "none");
    assert!(out["task"].is_null());
    let tried: Vec<String> = rung_results(&out)
        .into_iter()
        .map(|(r, s)| format!("{r}:{s}"))
        .collect();
    assert_eq!(
        tried,
        vec![
            "push:empty",
            "queue:empty",
            "shortlist:empty",
            "priority:empty"
        ]
    );
}

#[test]
fn github_not_answering_is_a_down_rung_not_an_empty_one() {
    let fake = Fake::new();
    fake.on(INBOX, vec![empty_inbox()])
        .on(CLAIM, vec![no_claim()])
        .on("gh api", vec![fail(1, "HTTP 401: Bad credentials")]);
    let (out, code) = run(&fake, &["next", "--agent", "muse-3", "--no-shortlist"]);
    assert_eq!(code, EXIT_NO_WORK);
    assert_eq!(out["tried"][3]["rung"], "priority");
    assert_eq!(out["tried"][3]["result"], "down");
    assert!(
        out["tried"][3]["why"]
            .as_str()
            .unwrap()
            .contains("Bad credentials")
    );
}

#[test]
fn switches_skip_rungs_and_say_so() {
    let fake = Fake::new();
    fake.on("gh api", vec![ok(json!([]))]);
    let (out, _) = run(
        &fake,
        &[
            "next",
            "--agent",
            "muse-3",
            "--no-push",
            "--no-queue",
            "--no-shortlist",
        ],
    );
    let tried: Vec<String> = rung_results(&out)
        .into_iter()
        .map(|(r, s)| format!("{r}:{s}"))
        .collect();
    assert_eq!(
        tried,
        vec![
            "push:skipped",
            "queue:skipped",
            "shortlist:skipped",
            "priority:empty"
        ]
    );
    assert!(!fake.called(INBOX) && !fake.called(CLAIM));
}

#[test]
fn next_refuses_what_it_cannot_mean() {
    let fake = Fake::new();
    let err = |words: &[&str]| execute(&fake, &argv(words)).unwrap_err();
    assert!(
        matches!(err(&["next", "--agent", "muse-3", "--bogus"]), AgentError::Usage(m) if m.contains("--bogus"))
    );
    assert!(matches!(
        err(&["next", "--agent", "Muse 3"]),
        AgentError::Usage(_)
    ));
    assert!(matches!(
        err(&["next", "--agent", "muse-3", "--repo", "code"]),
        AgentError::Usage(_)
    ));
    assert!(matches!(
        err(&["next", "--agent", "muse-3", "extra"]),
        AgentError::Usage(_)
    ));
    // No name anywhere: a usage error that says how to fix it, and no call made.
    assert!(matches!(err(&["next"]), AgentError::Usage(m) if m.contains("fleet agent enroll")));
    assert!(fake.calls().is_empty());
}

// ---- candidates ----

#[test]
fn candidates_rank_across_labels_and_never_repeat_an_issue() {
    let fake = Fake::new();
    fake.on(
        "labels=expedite",
        vec![ok(json!([issue(3418, &["expedite", "now"])]))],
    )
    .on("labels=priority%2Fcritical", vec![ok(json!([]))])
    .on(
        "labels=now",
        vec![ok(json!([
            issue(3418, &["expedite", "now"]),
            issue(557, &["now"])
        ]))],
    )
    .on(
        "labels=priority%2Fhigh",
        vec![ok(json!([issue(796, &["priority/high"])]))],
    )
    .on("labels=next", vec![ok(json!([issue(786, &["next"])]))]);
    let found = crate::next::candidates(&fake, "developing-today/code", 10).unwrap();
    let order: Vec<(u64, u64)> = found.iter().map(|e| (e.issue, e.rank)).collect();
    // expedite=0, now=2, priority/high=3, next=4; #3418 appears once.
    assert_eq!(order, vec![(3418, 0), (557, 2), (796, 3), (786, 4)]);
    // The label with a slash is percent-encoded in the query.
    assert!(fake.called("labels=priority%2Fcritical"));
}

#[test]
fn candidates_fall_back_from_fleet_gh_to_plain_gh() {
    let fake = Fake::new();
    fake.on("fleet gh api", vec![fail(69, "gh: not in this build")])
        .on("gh api", vec![ok(json!([issue(557, &["now"])]))]);
    let found = crate::next::candidates(&fake, "developing-today/code", 3).unwrap();
    assert_eq!(found[0].issue, 557);
}

fn count_calls(fake: &Fake, prefix: &str) -> usize {
    fake.calls()
        .iter()
        .filter(|c| c.starts_with(prefix))
        .count()
}

#[test]
fn a_github_route_that_cannot_answer_is_paid_for_once_per_lookup() {
    // Oracle: five ranked labels are asked. The route that fails is
    // tried for the first only; the other four go straight to the
    // route that answered.
    let fake = Fake::new();
    fake.on(
        "fleet gh api",
        vec![fail(3, "fleet gh gave no answer within 45s")],
    )
    .on("gh api", vec![ok(json!([]))]);
    let found = crate::next::candidates(&fake, "developing-today/code", 10).unwrap();
    assert!(found.is_empty());
    assert_eq!(count_calls(&fake, "fleet gh api repos/"), 1);
    assert_eq!(count_calls(&fake, "gh api repos/"), 5);
}

#[test]
fn the_first_github_route_can_be_named_and_the_other_stays_the_fallback() {
    // Named plain: the fleet's rotation is never asked while plain answers.
    let fake = Fake::new();
    fake.set_env("FLEET_AGENT_GH", "plain");
    fake.on("gh api", vec![ok(json!([]))]);
    crate::next::candidates(&fake, "developing-today/code", 10).unwrap();
    assert_eq!(count_calls(&fake, "fleet gh api"), 0);
    assert_eq!(count_calls(&fake, "gh api repos/"), 5);

    // Named plain, but plain is down: the rotation still answers, and
    // is then the route for the rest of the lookup.
    let fake = Fake::new();
    fake.set_env("FLEET_AGENT_GH", "plain");
    fake.on("fleet gh api", vec![ok(json!([issue(557, &["now"])]))])
        .on("gh api", vec![fail(1, "gh: not signed in")]);
    let found = crate::next::candidates(&fake, "developing-today/code", 10).unwrap();
    assert_eq!(found[0].issue, 557);
    assert_eq!(count_calls(&fake, "gh api repos/"), 1);
    assert_eq!(count_calls(&fake, "fleet gh api repos/"), 5);

    // A value that names no route changes nothing: rotation first.
    let fake = Fake::new();
    fake.set_env("FLEET_AGENT_GH", "sideways");
    fake.on("fleet gh api", vec![ok(json!([]))]);
    crate::next::candidates(&fake, "developing-today/code", 10).unwrap();
    assert_eq!(count_calls(&fake, "gh api repos/"), 0);
}

#[test]
fn enrolment_records_the_github_route_that_answered_and_later_lookups_start_there() {
    let fake = Fake::new();
    fake.on("fleet bridge hello", vec![ok(json!({"agent": "codex-2"}))])
        .on(
            "fleet queue health",
            vec![fail(3, "queue broker unreachable")],
        )
        .on(
            "fleet gh api",
            vec![fail(3, "fleet gh gave no answer within 45s")],
        )
        .on("gh api", vec![ok(json!([]))])
        .on(
            "fleet laptop-agent health",
            vec![fail(3, "no leg answered")],
        );
    let (out, code) = run(&fake, &["enroll", "--kind", "codex", "--name", "codex-2"]);
    assert_eq!(code, EXIT_OK);
    assert_eq!(out["legs"][2]["leg"], "github");
    assert_eq!(out["legs"][2]["route"], "plain");
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(fake.profile_path()).unwrap()).unwrap();
    assert_eq!(saved["gh"], "plain");
    // One probe went through the rotation at enrolment; no lookup does.
    assert_eq!(count_calls(&fake, "fleet gh api"), 1);
    crate::next::candidates(&fake, "developing-today/code", 10).unwrap();
    assert_eq!(count_calls(&fake, "fleet gh api"), 1);
    assert_eq!(count_calls(&fake, "gh api repos/"), 5);
}

// ---- the shortlist ----

#[test]
fn shortlist_add_refuses_duplicates_and_recent_hand_outs() {
    let mut list = Shortlist::default();
    assert!(list.add(entry(557, 2, 1)));
    assert!(!list.add(entry(557, 0, 2)), "the same issue is listed once");
    let taken = list.take("muse-3", 100).unwrap();
    assert_eq!(taken.issue, 557);
    assert!(
        !list.add(entry(557, 2, 101)),
        "handed out here inside the window"
    );
    list.prune(100 + shortlist::HANDED_TTL_SECS);
    assert!(list.add(entry(557, 2, 9000)), "the window has passed");
}

#[test]
fn a_shortlist_file_that_does_not_parse_is_an_error_and_is_left_alone() {
    let fake = Fake::new();
    std::fs::write(fake.shortlist_path(), "{ not a shortlist").unwrap();
    let err = execute(&fake, &argv(&["shortlist", "status"])).unwrap_err();
    assert!(
        matches!(&err, AgentError::Down(m) if m.contains("shortlist.json")),
        "{err:?}"
    );
    assert_eq!(err.exit_code(), EXIT_DOWN);
    assert_eq!(
        std::fs::read_to_string(fake.shortlist_path()).unwrap(),
        "{ not a shortlist"
    );
}

#[test]
fn shortlist_fill_tops_up_to_the_target_and_releases_the_filler() {
    let fake = Fake::new();
    fake.on(
        "labels=now",
        vec![ok(json!([
            issue(1, &["now"]),
            issue(2, &["now"]),
            issue(3, &["now"]),
            issue(4, &["now"])
        ]))],
    )
    .on("gh api", vec![ok(json!([]))]);
    fake.seed(vec![entry(2, 2, 1)]);
    shortlist::update(&fake.shortlist_path(), fake.now.get(), |list| {
        list.refill = Some(("muse-3".to_string(), fake.now.get()));
    })
    .unwrap();
    let (out, code) = run(
        &fake,
        &["shortlist", "fill", "--target", "3", "--agent", "muse-3"],
    );
    assert_eq!(code, EXIT_OK);
    // One entry was there; #2 is already listed; #1 and #3 bring it to three; #4 is not needed.
    assert_eq!(out["added"], 2);
    assert_eq!(out["count"], 3);
    let list = shortlist::load(&fake.shortlist_path()).unwrap();
    let mut issues: Vec<u64> = list.entries.iter().map(|e| e.issue).collect();
    issues.sort();
    assert_eq!(issues, vec![1, 2, 3]);
    assert_eq!(list.refill, None);
    assert!(
        list.entries
            .iter()
            .filter(|e| e.issue != 2)
            .all(|e| e.added_by == "muse-3")
    );
}

#[test]
fn shortlist_take_on_an_empty_list_is_exit_five() {
    let fake = Fake::new();
    let (out, code) = run(&fake, &["shortlist", "take", "--agent", "muse-3"]);
    assert_eq!(code, EXIT_NO_WORK);
    assert!(out["entry"].is_null());
}

proptest! {
    /// Taking until empty yields the entries in (rank, added_at, issue)
    /// order, each exactly once, and every take is remembered.
    #[test]
    fn take_order_matches_a_sort(specs in proptest::collection::btree_map(1u64..500, (0u64..7, 0u64..1000), 0..12)) {
        let mut list = Shortlist::default();
        for (issue, (rank, added_at)) in &specs {
            list.add(entry(*issue, *rank, *added_at));
        }
        let mut expected: Vec<(u64, u64, u64)> =
            specs.iter().map(|(issue, (rank, at))| (*rank, *at, *issue)).collect();
        expected.sort();
        let mut got = Vec::new();
        while let Some(e) = list.take("muse-3", 5000) {
            got.push((e.rank, e.added_at, e.issue));
        }
        prop_assert_eq!(&got, &expected);
        prop_assert_eq!(list.handed.len(), specs.len());
        prop_assert!(list.entries.is_empty());
    }

    /// What is written is what is read back.
    #[test]
    fn the_file_round_trips(specs in proptest::collection::btree_map(1u64..500, (0u64..7, 0u64..1000), 0..8), marker in proptest::option::of(0u64..1000)) {
        let fake = Fake::new();
        let mut list = Shortlist::default();
        for (issue, (rank, added_at)) in &specs {
            list.add(entry(*issue, *rank, *added_at));
        }
        list.refill = marker.map(|at| ("muse-3".to_string(), fake.now.get() - at));
        let want = list.clone();
        shortlist::update(&fake.shortlist_path(), fake.now.get(), |on_disk| *on_disk = list).unwrap();
        prop_assert_eq!(shortlist::load(&fake.shortlist_path()).unwrap(), want);
    }
}

// ---- enrolment ----

#[test]
fn enrolling_registers_the_mailbox_saves_the_profile_and_reports_the_rungs() {
    let fake = Fake::new();
    fake.on(
        "fleet bridge hello",
        vec![ok(json!({"agent": "muse-7", "groups": ["muse", "night"]}))],
    )
    .on(
        "fleet queue health",
        vec![fail(3, "queue broker unreachable")],
    )
    .on(
        "fleet gh api rate_limit",
        vec![ok(json!({"rate": {"remaining": 4000}}))],
    )
    .on(
        "fleet laptop-agent health",
        vec![fail(3, "no leg answered")],
    );
    let (out, code) = run(
        &fake,
        &[
            "enroll",
            "--kind",
            "muse",
            "--name",
            "muse-7",
            "--groups",
            "night,muse",
        ],
    );
    // One answering intake leg is enough to be enrolled.
    assert_eq!(code, EXIT_OK);
    assert_eq!(out["registered"], true);
    // The kind's group comes first; an extra group is added once.
    assert_eq!(out["groups"], json!(["muse", "night"]));
    assert!(fake.called("fleet bridge hello --agent muse-7 --groups muse,night"));
    assert_eq!(
        out["rungs"],
        json!({"push": true, "queue": false, "shortlist": true, "priority": true})
    );
    assert_eq!(out["address"], "agent:muse-7");
    // A leg that is down says what it needs.
    assert_eq!(out["legs"][1]["leg"], "queue");
    assert_eq!(out["legs"][1]["ok"], false);
    assert!(
        out["legs"][1]["needs"]
            .as_str()
            .unwrap()
            .contains("queue broker")
    );
    // The profile lets later verbs know who is asking.
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(fake.profile_path()).unwrap()).unwrap();
    assert_eq!(saved["name"], "muse-7");
    assert_eq!(saved["kind"], "muse");

    let fake_next = &fake;
    fake_next
        .on(INBOX, vec![empty_inbox()])
        .on(CLAIM, vec![no_claim()])
        .on("gh api", vec![ok(json!([]))]);
    let (next, _) = run(fake_next, &["next", "--no-shortlist"]);
    assert_eq!(next["agent"], "muse-7", "next reads the saved profile");
}

#[test]
fn a_seat_where_no_intake_leg_answers_is_exit_three_and_still_explains_itself() {
    let fake = Fake::new();
    fake.on("fleet ", vec![fail(3, "unreachable")])
        .on("gh ", vec![fail(1, "not signed in")]);
    let (out, code) = run(&fake, &["enroll", "--kind", "generic", "--name", "box-1"]);
    assert_eq!(code, EXIT_DOWN);
    assert_eq!(out["registered"], false);
    assert_eq!(
        out["rungs"],
        json!({"push": false, "queue": false, "shortlist": true, "priority": false})
    );
    assert_eq!(out["legs"].as_array().unwrap().len(), 4);
}

#[test]
fn a_connector_has_an_address_and_no_mailbox() {
    let fake = Fake::new();
    let (out, code) = run(&fake, &["enroll", "--kind", "claude"]);
    assert_eq!(code, EXIT_OK);
    assert_eq!(out["address"], "claude");
    assert_eq!(out["registered"], false);
    assert!(
        fake.calls().is_empty(),
        "no hello for a connector: {:?}",
        fake.calls()
    );
    assert!(!fake.profile_path().exists());
}

#[test]
fn an_http_agent_is_registered_from_this_seat_without_taking_its_profile() {
    let fake = Fake::new();
    fake.on("fleet bridge hello", vec![ok(json!({"agent": "dot"}))])
        .on("fleet ", vec![fail(3, "unreachable")])
        .on("gh ", vec![fail(1, "x")]);
    let (out, code) = run(&fake, &["enroll", "--kind", "dot", "--name", "dot"]);
    assert_eq!(code, EXIT_OK);
    assert_eq!(out["seat"], "http");
    assert_eq!(out["registered"], true);
    assert!(
        out["http"]["routes"][0]
            .as_str()
            .unwrap()
            .contains("/hello")
    );
    assert!(out["profile"].is_null());
    assert!(
        !fake.profile_path().exists(),
        "this seat's own profile must not become dot's"
    );
}

#[test]
fn a_dry_run_registers_nothing_and_saves_nothing() {
    let fake = Fake::new();
    fake.on("fleet bridge health", vec![ok(json!({"ok": true}))])
        .on("fleet ", vec![fail(3, "unreachable")])
        .on("gh ", vec![fail(1, "x")]);
    let (out, code) = run(
        &fake,
        &["enroll", "--kind", "muse", "--name", "muse-7", "--dry-run"],
    );
    assert_eq!(code, EXIT_OK);
    assert_eq!(out["registered"], false);
    assert!(!fake.called("bridge hello"));
    assert!(!fake.profile_path().exists());
}

#[test]
fn enrolment_refuses_an_unknown_kind_and_a_bad_name() {
    let fake = Fake::new();
    let err = |words: &[&str]| execute(&fake, &argv(words)).unwrap_err();
    assert!(
        matches!(err(&["enroll", "--kind", "robot", "--name", "r2"]), AgentError::Usage(m) if m.contains("generic"))
    );
    assert!(matches!(
        err(&["enroll", "--kind", "muse", "--name", "Muse_7!"]),
        AgentError::Usage(_)
    ));
    assert!(
        matches!(err(&["enroll", "--kind", "muse"]), AgentError::Usage(m) if m.contains("--name"))
    );
    assert!(fake.calls().is_empty());
}

#[test]
fn every_kind_has_a_usable_row() {
    let table = crate::kinds::table();
    let rows = table["kinds"].as_array().unwrap();
    assert!(rows.len() >= 10);
    for kind in crate::kinds::KINDS {
        assert!(
            crate::check_name("kind", kind.name).is_ok(),
            "{}",
            kind.name
        );
        for group in kind.groups {
            assert!(crate::check_name("group", group).is_ok(), "{group}");
        }
        // A connector is addressed by channel; nothing else has a fixed address.
        assert_eq!(
            kind.address.is_some(),
            kind.seat == crate::kinds::Seat::Connector,
            "{}",
            kind.name
        );
    }
    // The kinds the fleet runs today are all enrolable by name.
    for name in [
        "muse",
        "opencode",
        "openchamber",
        "openclaw",
        "grok",
        "dot",
        "duet",
        "claude",
        "chatgpt",
        "antigravity",
    ] {
        assert!(crate::kinds::kind(name).is_some(), "{name}");
    }
}

// ---- fleet shell ----

fn opts() -> Options {
    Options {
        cwd: None,
        build: false,
        timeout: None,
        detach: false,
    }
}

fn words(list: &[&str]) -> Vec<String> {
    argv(list)
}

#[test]
fn a_laptop_command_goes_through_the_laptop_agent_ladder() {
    let plan = shell::plan(
        Target::Laptop,
        "test".into(),
        &words(&["cargo", "test", "-p", "id-core"]),
        &opts(),
    )
    .unwrap();
    assert_eq!(plan.program, "fleet");
    assert_eq!(
        plan.args,
        argv(&[
            "laptop-agent",
            "exec",
            "--timeout",
            "120",
            "--shell",
            "--",
            "cargo test -p id-core"
        ])
    );
}

#[test]
fn a_build_takes_a_lock_slot_and_an_hour() {
    let plan = shell::plan(
        Target::Laptop,
        "test".into(),
        &words(&["cargo", "test", "-p", "id-core"]),
        &Options {
            cwd: Some("/home/user/code/pkgs/id".into()),
            build: true,
            timeout: None,
            detach: false,
        },
    )
    .unwrap();
    assert_eq!(
        plan.args,
        argv(&[
            "laptop-agent",
            "exec",
            "--cwd",
            "/home/user/code/pkgs/id",
            "--timeout",
            "3600",
            "--shell",
            "--",
            "~/tools/fleet-build-lock cargo test -p id-core",
        ])
    );
}

#[test]
fn detach_starts_a_job_and_only_on_the_laptop() {
    let detached = Options {
        cwd: None,
        build: false,
        timeout: None,
        detach: true,
    };
    let plan = shell::plan(
        Target::Laptop,
        "t".into(),
        &words(&["nix", "build"]),
        &detached,
    )
    .unwrap();
    assert_eq!(
        plan.args,
        argv(&["laptop-agent", "run", "--shell", "--", "nix build"])
    );
    assert!(matches!(
        shell::plan(
            Target::Local,
            "t".into(),
            &words(&["nix", "build"]),
            &detached
        ),
        Err(AgentError::Usage(_))
    ));
}

#[test]
fn no_command_opens_a_shell_and_build_flags_then_make_no_sense() {
    let plan = shell::plan(Target::Laptop, "t".into(), &[], &opts()).unwrap();
    assert_eq!(
        (plan.program.as_str(), plan.args),
        ("fleet", argv(&["proxy", "ssh", "--"]))
    );
    let plan = shell::plan(Target::Codespace("cs-1".into()), "t".into(), &[], &opts()).unwrap();
    assert_eq!(
        (plan.program.as_str(), plan.args),
        ("gh", argv(&["codespace", "ssh", "-c", "cs-1"]))
    );
    let build = Options {
        cwd: None,
        build: true,
        timeout: None,
        detach: false,
    };
    assert!(matches!(
        shell::plan(Target::Laptop, "t".into(), &[], &build),
        Err(AgentError::Usage(_))
    ));
}

#[test]
fn a_tilde_is_quoted_so_the_far_shell_does_not_expand_it() {
    // Found by the property test below: a bare `~` came back as the
    // home directory. Oracle: POSIX tilde expansion applies to an
    // unquoted leading tilde only.
    assert_eq!(shell::sh_quote("~"), "'~'");
    assert_eq!(shell::sh_quote("~/tools/x"), "'~/tools/x'");
    assert_eq!(shell::sh_quote("a~b"), "'a~b'");
    // Positive control: a word with none of it stays bare.
    assert_eq!(shell::sh_quote("pkgs/id"), "pkgs/id");
}

#[test]
fn a_single_word_is_a_shell_string_and_several_words_are_quoted() {
    assert_eq!(
        shell::shell_string(&words(&["cd pkgs/id && cargo test"])),
        "cd pkgs/id && cargo test"
    );
    assert_eq!(
        shell::shell_string(&words(&["echo", "two words", "it's"])),
        "echo 'two words' 'it'\\''s'"
    );
}

fn config() -> fleet_config::Config {
    fleet_config::Config::default()
}

#[test]
fn auto_on_the_laptop_runs_here_without_probing() {
    let fake = Fake::new();
    fake.set_env("FLEET_LOCATION", "laptop");
    let (target, because) = shell::resolve(&config(), &fake, "auto", None).unwrap();
    assert_eq!(target, Target::Local);
    assert!(because.contains("laptop"));
    assert!(fake.calls().is_empty());
}

#[test]
fn auto_elsewhere_prefers_the_laptop_then_a_codespace_then_here() {
    let fake = Fake::new();
    fake.on("fleet laptop-agent health", vec![ok(json!({"ok": true}))]);
    assert_eq!(
        shell::resolve(&config(), &fake, "auto", None).unwrap().0,
        Target::Laptop
    );

    let fake = Fake::new();
    fake.on("fleet laptop-agent health", vec![fail(3, "no leg answered")]).on(
        "gh codespace list",
        vec![ok(json!([{"name": "sleepy", "state": "Shutdown"}, {"name": "awake", "state": "Available"}]))],
    );
    assert_eq!(
        shell::resolve(&config(), &fake, "auto", None).unwrap().0,
        Target::Codespace("awake".to_string()),
        "a running codespace is preferred over the first listed"
    );

    let fake = Fake::new();
    fake.on(
        "fleet laptop-agent health",
        vec![fail(3, "no leg answered")],
    )
    .on("gh codespace list", vec![fail(1, "HTTP 403")]);
    let (target, because) = shell::resolve(&config(), &fake, "auto", None).unwrap();
    assert_eq!(target, Target::Local);
    assert!(because.contains("running here"), "{because}");
}

#[test]
fn an_explicit_target_is_never_swapped_for_another() {
    let fake = Fake::new();
    fake.on(
        "fleet laptop-agent health",
        vec![fail(3, "no leg answered")],
    );
    // Asked for the laptop while it is down: still the laptop, so the caller
    // sees the failure instead of a build quietly running in a sandbox.
    assert_eq!(
        shell::resolve(&config(), &fake, "laptop", None).unwrap().0,
        Target::Laptop
    );
    assert!(fake.calls().is_empty(), "an explicit target is not probed");
    // Asked for a codespace when there is none: an error, not a fallback.
    let fake = Fake::new();
    fake.on("gh codespace list", vec![ok(json!([]))]);
    assert!(matches!(
        shell::resolve(&config(), &fake, "codespace", None),
        Err(AgentError::Down(_))
    ));
}

#[test]
fn the_environment_can_pin_auto_and_a_bad_pin_is_refused() {
    let fake = Fake::new();
    fake.set_env("FLEET_SHELL_TARGET", "local");
    assert_eq!(
        shell::resolve(&config(), &fake, "auto", None).unwrap().0,
        Target::Local
    );
    fake.set_env("FLEET_SHELL_TARGET", "moon");
    assert!(matches!(
        shell::resolve(&config(), &fake, "auto", None),
        Err(AgentError::Usage(_))
    ));
    // An explicit --target wins over the environment.
    assert_eq!(
        shell::resolve(&config(), &fake, "laptop", None).unwrap().0,
        Target::Laptop
    );
}

#[test]
fn shell_runs_the_plan_or_prints_it() {
    let fake = Fake::new();
    fake.set_env("FLEET_LOCATION", "sandbox");
    fake.on("fleet laptop-agent health", vec![ok(json!({"ok": true}))]);
    let printed = shell::run(
        &config(),
        &fake,
        &argv(&["--print", "--build", "--", "cargo", "check"]),
    )
    .unwrap();
    match printed {
        shell::Done::Printed(plan) => {
            assert_eq!(plan["target"], "laptop");
            assert_eq!(plan["args"][6], "~/tools/fleet-build-lock cargo check");
        }
        shell::Done::Ran(_) => panic!("--print must not run"),
    }
    assert!(fake.attached.borrow().is_empty());
    let ran = shell::run(&config(), &fake, &argv(&["--", "uname", "-a"])).unwrap();
    assert!(matches!(ran, shell::Done::Ran(0)));
    assert_eq!(
        fake.attached.borrow().as_slice(),
        ["fleet laptop-agent exec --timeout 120 --shell -- uname -a"]
    );
    // A word before `--` is a mistake, not a command.
    assert!(matches!(
        shell::run(&config(), &fake, &argv(&["uname"])),
        Err(AgentError::Usage(_))
    ));
}

proptest! {
    /// A quoted word, handed to a real shell, comes back unchanged.
    #[test]
    fn sh_quote_survives_a_real_shell(word in "[ -~]{0,24}") {
        let line = format!("printf '%s' {}", shell::sh_quote(&word));
        let out = std::process::Command::new("sh").arg("-c").arg(&line).output().expect("sh runs");
        prop_assert!(out.status.success(), "sh rejected: {}", line);
        prop_assert_eq!(String::from_utf8_lossy(&out.stdout).into_owned(), word);
    }
}

// ---- bounded steps ----
//
// Oracle: the contract in `capture`'s doc comment, checked against
// the real /bin/sh: an answer in time is passed through whole; no
// answer in time is exit 3 (down) and a line saying how long.

#[test]
fn a_step_that_answers_in_time_is_passed_through_whole() {
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.args(["-c", "echo out; echo err >&2; exit 4"]);
    let out = crate::capture(cmd, "probe", std::time::Duration::from_secs(20));
    assert_eq!(out.code, 4);
    assert_eq!(out.stdout, "out\n");
    assert_eq!(out.stderr, "err\n");
}

#[test]
fn a_step_that_does_not_answer_in_time_is_down_not_a_hang() {
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.args(["-c", "sleep 30"]);
    let started = std::time::Instant::now();
    let out = crate::capture(cmd, "fleet queue health", std::time::Duration::from_secs(1));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "the limit must end the wait, not the child's own 30s"
    );
    assert_eq!(out.code, crate::EXIT_DOWN);
    assert!(!out.ok());
    assert_eq!(out.stderr, "fleet queue health gave no answer within 1s");
}

#[test]
fn a_step_that_cannot_start_says_so() {
    let cmd = std::process::Command::new("/nonexistent/fleet-agent-test-binary");
    let out = crate::capture(cmd, "probe", std::time::Duration::from_secs(5));
    assert_eq!(out.code, 127);
    assert!(
        out.stderr.starts_with("cannot run probe: "),
        "{}",
        out.stderr
    );
}

#[test]
fn a_large_output_does_not_block_the_step() {
    // More than a pipe buffer: the step must still finish.
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.args(["-c", "head -c 300000 /dev/zero | tr '\\0' 'a'"]);
    let out = crate::capture(cmd, "probe", std::time::Duration::from_secs(20));
    assert_eq!(out.code, 0);
    assert_eq!(out.stdout.len(), 300_000);
}

#[test]
fn the_step_limit_is_a_positive_number_of_seconds_or_the_default() {
    let default = std::time::Duration::from_secs(crate::DEFAULT_STEP_TIMEOUT_SECS);
    assert_eq!(crate::step_limit(None), default);
    for junk in ["", "0", "-5", "soon", "1.5"] {
        assert_eq!(crate::step_limit(Some(junk)), default, "{junk:?}");
    }
    assert_eq!(
        crate::step_limit(Some(" 7 ")),
        std::time::Duration::from_secs(7)
    );
}
