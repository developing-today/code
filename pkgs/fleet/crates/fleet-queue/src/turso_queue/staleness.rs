//! turso_queue::staleness — the claim-time staleness check (#10626).
//!
//! The 2026-11-16 outcome-taxonomy measurement found 27.8% of
//! non-triage completions were pre-done: the linked issue was
//! already closed, or the linked PR already merged, before a lane
//! claimed the task (625 of 652 before the task was even created,
//! from a ~3-week-old filing snapshot). Each still spawned a
//! session to verify and close it.
//!
//! The default claim pick now checks the picked row's linked work
//! before it hands the row out. A row whose work already landed is
//! retired in place — status done, result outcome `already_done`
//! with the evidence, a `staleness_autocomplete` event — and the
//! pick moves on to the next candidate. No session is bound and no
//! lane slot is consumed: the retirement happens before the claim's
//! lease, binding and `claimed` event are written.
//!
//! Tiers: tier 1 is work that landed at or before the task was
//! created (the strongest signal); tier 2 is work that landed
//! between creation and this claim. Both retire; the evidence names
//! the tier so the daily measurement can count them apart, and
//! `FLEET_STALENESS_TIER2=0` on the broker turns tier 2 off.
//!
//! Fail open, always (standing rule 2026-10-30): no probe, no
//! linkage, a lookup error, a timeout or a recent failure all leave
//! the claim exactly as before. Lookups are bounded by a short
//! timeout in the probe and cached here (terminal facts for good,
//! open states for a few minutes), and after a failed lookup the
//! check stands down for a minute so a GitHub outage never adds
//! latency to every claim. `FLEET_STALENESS_CHECK=0` turns the
//! whole check off; a packet with `staleness_check: false` opts out.
//!
//! Only the default pick runs the check. A by-id claim, a transfer
//! and the handoff pick name their row deliberately — the override
//! path #9178 keeps — so they claim as before.

use super::*;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long one staleness lookup may take before the claim goes
/// ahead without it (acceptance 5: p95 added latency under 2s).
pub const STALENESS_LOOKUP_TIMEOUT: Duration = Duration::from_millis(1500);

/// How long an open state stays cached. Terminal facts (closed,
/// merged) are cached for the life of the broker.
pub const STALENESS_OPEN_TTL: Duration = Duration::from_secs(600);

/// How long the check stands down after a failed lookup.
pub const STALENESS_FAILURE_BACKOFF: Duration = Duration::from_secs(60);

/// The most rows one claim call may retire as already done before
/// it gives up and returns no claim (the next call carries on).
pub const MAX_STALENESS_RETIRES_PER_CALL: usize = 64;

/// The kind of linked work a task carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkKind {
    Issue,
    Pr,
}

impl LinkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkKind::Issue => "issue",
            LinkKind::Pr => "pr",
        }
    }
}

/// One piece of linked work: an issue or a PR in a repo.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Link {
    pub kind: LinkKind,
    pub repo: String,
    pub number: i64,
}

impl Link {
    pub fn key(&self) -> String {
        format!("{}:{}#{}", self.kind.as_str(), self.repo, self.number)
    }
}

/// What GitHub says about a link, for the staleness check.
#[derive(Clone, Debug, PartialEq)]
pub enum Landing {
    /// Still open (or a PR closed without merging): work remains.
    Open,
    /// The work landed: an issue closed, or a PR merged, at `at`.
    Landed { state: String, at: String },
    /// The lookup could not be made; nothing is inferred.
    Unknown(String),
}

/// The linked work a task's packet names (#10626 step 1): the
/// packet's `pr` / `prs[]` and its result's `pr` / `pr_url` when
/// present, else its `issue`. A linked PR decides on its own: a
/// shepherd task for an open PR stays live even when its issue was
/// closed. Links resolve under the task's repo; a PR entry may name
/// its own repo or a full pull URL.
pub fn task_links(row: &Value) -> Vec<Link> {
    let payload = task_payload(row);
    let repo = task_repo_identity(row);
    let mut prs: Vec<Link> = Vec::new();
    let push_pr = |v: &Value, prs: &mut Vec<Link>| {
        let (own_repo, number) = match v {
            Value::Number(n) => (None, n.as_i64()),
            Value::String(s) => (pr_url_repo(s), crate::disposition::pr_number_in(s)),
            Value::Object(o) => {
                let url = o.get("url").and_then(|u| u.as_str()).unwrap_or("");
                let number = o
                    .get("number")
                    .and_then(|n| n.as_i64())
                    .or_else(|| crate::disposition::pr_number_in(url));
                let own = o
                    .get("repo")
                    .and_then(|r| r.as_str())
                    .filter(|r| r.contains('/'))
                    .map(str::to_string)
                    .or_else(|| pr_url_repo(url));
                (own, number)
            }
            _ => (None, None),
        };
        let repo = own_repo.unwrap_or_else(|| repo.clone());
        if let Some(number) = number.filter(|n| *n > 0) {
            if repo.contains('/') {
                let link = Link {
                    kind: LinkKind::Pr,
                    repo,
                    number,
                };
                if !prs.contains(&link) {
                    prs.push(link);
                }
            }
        }
    };
    if let Some(pr) = payload.get("pr") {
        push_pr(pr, &mut prs);
    }
    if let Some(list) = payload.get("prs").and_then(|v| v.as_array()) {
        for pr in list {
            push_pr(pr, &mut prs);
        }
    }
    let result = row.get("result").cloned().unwrap_or(Value::Null);
    let result = match result {
        Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::Null),
        v => v,
    };
    for key in ["pr", "pr_url"] {
        if let Some(pr) = result.get(key) {
            push_pr(pr, &mut prs);
        }
    }
    if !prs.is_empty() {
        return prs;
    }
    let number = payload
        .get("issue")
        .and_then(|i| i.get("number"))
        .and_then(|n| n.as_i64())
        .filter(|n| *n > 0);
    match number {
        Some(number) if repo.contains('/') => vec![Link {
            kind: LinkKind::Issue,
            repo,
            number,
        }],
        _ => Vec::new(),
    }
}

/// `owner/repo` from a GitHub pull URL.
fn pr_url_repo(url: &str) -> Option<String> {
    let rest = url.split("github.com/").nth(1)?;
    let (repo, _) = rest.split_once("/pull/")?;
    (repo.split('/').count() == 2).then(|| repo.to_string())
}

/// Why the check retired a row: the evidence that lands on the
/// result and the event.
#[derive(Clone, Debug, PartialEq)]
pub struct Stale {
    pub tier: u8,
    pub link: Link,
    pub state: String,
    pub landed_at: String,
    pub created_at: String,
    pub checked_at: String,
}

impl Stale {
    pub fn evidence(&self) -> Value {
        json!({
            "tier": self.tier,
            "kind": self.link.kind.as_str(),
            "repo": self.link.repo,
            "number": self.link.number,
            "ref": self.link.key(),
            "state": self.state,
            "landed_at": self.landed_at,
            "task_created_at": self.created_at,
            "checked_at": self.checked_at,
        })
    }
}

/// The verdict for one picked row (pure; the lookups come through
/// `look`). Some(stale) retires the row; None claims it as before.
/// Every link must have landed for the row to be stale; any open or
/// unknown link keeps the claim.
pub fn staleness_verdict(
    row: &Value,
    now: &str,
    tier2: bool,
    look: &mut dyn FnMut(&Link) -> Landing,
) -> Option<Stale> {
    let payload = task_payload(row);
    if payload.get("staleness_check").and_then(|v| v.as_bool()) == Some(false) {
        return None;
    }
    let links = task_links(row);
    if links.is_empty() {
        return None;
    }
    let created_at = row
        .get("created_at")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let created = crate::disposition::parse_rfc3339_epoch(&created_at);
    let now_s = crate::disposition::parse_rfc3339_epoch(now)?;
    let mut verdict: Option<Stale> = None;
    for link in &links {
        let Landing::Landed { state, at } = look(link) else {
            return None;
        };
        let landed = crate::disposition::parse_rfc3339_epoch(&at)?;
        let tier = match created {
            Some(c) if landed <= c => 1,
            _ if landed <= now_s => 2,
            _ => return None,
        };
        if tier == 2 && !tier2 {
            return None;
        }
        // The row's tier is its weakest link's.
        if verdict.as_ref().is_none_or(|v| tier > v.tier) {
            verdict = Some(Stale {
                tier,
                link: link.clone(),
                state,
                landed_at: at,
                created_at: created_at.clone(),
                checked_at: now.to_string(),
            });
        }
    }
    verdict
}

/// The broker's staleness lookups: the cache and the failure
/// stand-down. Lives on the backend so it outlasts one claim.
#[derive(Default)]
pub struct StalenessState {
    inner: Mutex<StalenessInner>,
}

#[derive(Default)]
struct StalenessInner {
    cache: HashMap<String, (Landing, Instant)>,
    failed_at: Option<Instant>,
}

impl StalenessState {
    /// One link's landing, from the cache or the probe. A failure
    /// starts the stand-down; during it every lookup is Unknown.
    pub fn look(&self, probe: &dyn DepProbe, link: &Link) -> Landing {
        let key = link.key();
        {
            let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(at) = inner.failed_at {
                if at.elapsed() < STALENESS_FAILURE_BACKOFF {
                    return Landing::Unknown("standing down after a failed lookup".to_string());
                }
            }
            if let Some((landing, at)) = inner.cache.get(&key) {
                let fresh =
                    matches!(landing, Landing::Landed { .. }) || at.elapsed() < STALENESS_OPEN_TTL;
                if fresh {
                    return landing.clone();
                }
            }
        }
        let started = Instant::now();
        let landing = probe.probe_landing(link.kind, &link.repo, link.number);
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match &landing {
            Landing::Unknown(_) => inner.failed_at = Some(Instant::now()),
            _ if started.elapsed() > STALENESS_LOOKUP_TIMEOUT => {
                // Answered, but slowly: keep the answer, and stand
                // down so the next claims do not queue behind it.
                inner.failed_at = Some(Instant::now());
                inner.cache.insert(key, (landing.clone(), Instant::now()));
            }
            _ => {
                inner.failed_at = None;
                inner.cache.insert(key, (landing.clone(), Instant::now()));
            }
        }
        landing
    }
}

/// Is the check on? `FLEET_STALENESS_CHECK` and
/// `FLEET_STALENESS_TIER2` are on unless set to 0/off/false/no.
fn flag_on(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no"
        ),
        Err(_) => true,
    }
}

/// The claim-path check for one picked row (#10626). None when the
/// check is off, the broker has no GitHub probe, or the row is not
/// stale: the claim proceeds exactly as before.
pub(crate) fn stale_pick(t: &TursoBackend, row: &Value) -> Option<Stale> {
    if !flag_on("FLEET_STALENESS_CHECK") {
        return None;
    }
    let probe = t.gh_probe()?;
    let tier2 = flag_on("FLEET_STALENESS_TIER2");
    staleness_verdict(row, &now_iso(), tier2, &mut |link| {
        t.staleness.look(probe, link)
    })
}

/// Retire a picked row whose work already landed: done, outcome
/// `already_done` with the evidence on its result, and one
/// `staleness_autocomplete` event. The row is still the picking
/// claim's, so nothing else holds it; no lease, binding or session
/// was written for it.
pub(crate) fn retire_stale(
    t: &TursoBackend,
    worker: &str,
    row: &Value,
    stale: &Stale,
) -> Result<(), TErr> {
    let id = row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
    let prior = match row.get("result").cloned().unwrap_or(json!({})) {
        Value::String(s) => serde_json::from_str(&s).unwrap_or(json!({})),
        Value::Null => json!({}),
        v => v,
    };
    let evidence = stale.evidence();
    let result = merge_obj(
        &prior,
        &json!({
            "outcome": "already_done",
            "staleness": evidence,
            "summary": format!(
                "Retired at claim (#10626): {} {} {} at {}, before any session ran.",
                stale.link.kind.as_str(),
                stale.link.key(),
                stale.state,
                stale.landed_at
            ),
        }),
    );
    let now = now_iso();
    t.exec(
        "UPDATE tasks SET status = 'done', lease_until = NULL, result = ?, \
           completed_at = ?, updated_at = ? WHERE id = ?",
        &[
            Arg::Text(result.to_string()),
            Arg::Text(now.clone()),
            Arg::Text(now),
            Arg::Int(id),
        ],
    )?;
    t.exec("DELETE FROM leases WHERE task_id = ?", &[Arg::Int(id)])?;
    event(
        t,
        Some(worker),
        Some(id),
        "staleness_autocomplete",
        &with_marks(
            &json!({"via": "claim_staleness_check", "outcome": "already_done", "evidence": evidence}),
            row,
        ),
    )?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn row(payload: Value, created_at: &str) -> Value {
        json!({"id": 7, "queue": "repo-health-code-pilot", "created_at": created_at, "payload": payload})
    }

    fn issue_task(n: i64) -> Value {
        row(
            json!({"repo": "developing-today/code", "issue": {"number": n}}),
            "2026-11-10T00:00:00Z",
        )
    }

    const NOW: &str = "2026-11-18T12:00:00Z";

    fn landed(at: &str) -> Landing {
        Landing::Landed {
            state: "closed".to_string(),
            at: at.to_string(),
        }
    }

    #[test]
    fn closed_before_creation_is_tier_one() {
        let stale = staleness_verdict(&issue_task(5), NOW, true, &mut |_| {
            landed("2026-10-25T00:00:00Z")
        })
        .unwrap();
        assert_eq!(stale.tier, 1);
        assert_eq!(stale.link.key(), "issue:developing-today/code#5");
        let ev = stale.evidence();
        assert_eq!(ev["landed_at"], "2026-10-25T00:00:00Z");
        assert_eq!(ev["task_created_at"], "2026-11-10T00:00:00Z");
        assert_eq!(ev["checked_at"], NOW);
    }

    #[test]
    fn closed_between_filing_and_claim_is_tier_two() {
        let mut look = |_: &Link| landed("2026-11-12T00:00:00Z");
        let stale = staleness_verdict(&issue_task(5), NOW, true, &mut look).unwrap();
        assert_eq!(stale.tier, 2);
        assert!(staleness_verdict(&issue_task(5), NOW, false, &mut look).is_none());
    }

    #[test]
    fn open_unknown_or_unlinked_claims_as_before() {
        assert!(staleness_verdict(&issue_task(5), NOW, true, &mut |_| Landing::Open).is_none());
        assert!(
            staleness_verdict(&issue_task(5), NOW, true, &mut |_| Landing::Unknown(
                "github http 503".into()
            ))
            .is_none()
        );
        let mut called = false;
        let bare = row(
            json!({"repo": "developing-today/code"}),
            "2026-11-10T00:00:00Z",
        );
        assert!(
            staleness_verdict(&bare, NOW, true, &mut |_| {
                called = true;
                landed("2026-10-01T00:00:00Z")
            })
            .is_none()
        );
        assert!(!called, "no linkage means no lookup");
        // A landing stamp in the future, or unparsable, infers nothing.
        assert!(
            staleness_verdict(&issue_task(5), NOW, true, &mut |_| landed(
                "2026-12-01T00:00:00Z"
            ))
            .is_none()
        );
        assert!(staleness_verdict(&issue_task(5), NOW, true, &mut |_| landed("soon")).is_none());
    }

    #[test]
    fn packet_can_opt_out() {
        let t = row(
            json!({"repo": "developing-today/code", "issue": {"number": 5}, "staleness_check": false}),
            "2026-11-10T00:00:00Z",
        );
        assert!(
            staleness_verdict(&t, NOW, true, &mut |_| landed("2026-10-01T00:00:00Z")).is_none()
        );
    }

    #[test]
    fn a_linked_pr_decides_over_its_issue() {
        let t = row(
            json!({"kind": "pr_shepherd", "repo": "developing-today/code",
                   "issue": {"number": 5}, "pr": {"number": 9, "url": "https://github.com/developing-today/code/pull/9"}}),
            "2026-11-10T00:00:00Z",
        );
        assert_eq!(
            task_links(&t),
            vec![Link {
                kind: LinkKind::Pr,
                repo: "developing-today/code".into(),
                number: 9
            }]
        );
        // Issue closed but PR still open: the shepherd task stays live.
        let mut look = |l: &Link| match l.kind {
            LinkKind::Issue => landed("2026-10-01T00:00:00Z"),
            LinkKind::Pr => Landing::Open,
        };
        assert!(staleness_verdict(&t, NOW, true, &mut look).is_none());
    }

    #[test]
    fn links_from_prs_list_and_result() {
        let t = json!({
            "id": 1, "queue": "repo-health-code-pilot", "created_at": "2026-11-10T00:00:00Z",
            "payload": {"repo": "developing-today/code", "issue": {"number": 5},
                        "prs": [12, "https://github.com/other/repo/pull/3", {"number": 12}]},
            "result": "{\"pr_url\": \"https://github.com/developing-today/code/pull/40\"}",
        });
        let keys: Vec<String> = task_links(&t).iter().map(Link::key).collect();
        assert_eq!(
            keys,
            vec![
                "pr:developing-today/code#12",
                "pr:other/repo#3",
                "pr:developing-today/code#40"
            ]
        );
        // Every link must have landed; the weakest tier wins.
        let mut look = |l: &Link| {
            if l.number == 40 {
                landed("2026-11-12T00:00:00Z")
            } else {
                landed("2026-10-01T00:00:00Z")
            }
        };
        assert_eq!(staleness_verdict(&t, NOW, true, &mut look).unwrap().tier, 2);
        let mut look = |l: &Link| {
            if l.number == 3 {
                Landing::Open
            } else {
                landed("2026-10-01T00:00:00Z")
            }
        };
        assert!(staleness_verdict(&t, NOW, true, &mut look).is_none());
    }

    struct Fixture {
        calls: Mutex<usize>,
        answer: Landing,
    }

    impl DepProbe for Fixture {
        fn probe_issue(&self, _: &str, _: i64) -> DepObs {
            DepObs::Failed("unused".into())
        }
        fn probe_pr(&self, _: &str, _: i64) -> DepObs {
            DepObs::Failed("unused".into())
        }
        fn probe_ci(&self, _: &str, _: i64) -> DepObs {
            DepObs::Failed("unused".into())
        }
        fn probe_landing(&self, _: LinkKind, _: &str, _: i64) -> Landing {
            *self.calls.lock().unwrap() += 1;
            self.answer.clone()
        }
    }

    fn link() -> Link {
        Link {
            kind: LinkKind::Issue,
            repo: "developing-today/code".into(),
            number: 5,
        }
    }

    #[test]
    fn landed_facts_are_cached() {
        let state = StalenessState::default();
        let probe = Fixture {
            calls: Mutex::new(0),
            answer: landed("2026-10-01T00:00:00Z"),
        };
        for _ in 0..3 {
            assert!(matches!(
                state.look(&probe, &link()),
                Landing::Landed { .. }
            ));
        }
        assert_eq!(*probe.calls.lock().unwrap(), 1);
    }

    #[test]
    fn a_failed_lookup_stands_the_check_down() {
        let state = StalenessState::default();
        let probe = Fixture {
            calls: Mutex::new(0),
            answer: Landing::Unknown("github http 502".into()),
        };
        for _ in 0..3 {
            assert!(matches!(state.look(&probe, &link()), Landing::Unknown(_)));
        }
        // One real lookup; the rest stood down without calling out.
        assert_eq!(*probe.calls.lock().unwrap(), 1);
    }

    #[test]
    fn probes_without_landing_support_fail_open() {
        struct Old;
        impl DepProbe for Old {
            fn probe_issue(&self, _: &str, _: i64) -> DepObs {
                DepObs::Cleared("issue closed".into())
            }
            fn probe_pr(&self, _: &str, _: i64) -> DepObs {
                DepObs::Cleared("pr merged".into())
            }
            fn probe_ci(&self, _: &str, _: i64) -> DepObs {
                DepObs::Failed("unused".into())
            }
        }
        assert!(matches!(
            Old.probe_landing(LinkKind::Issue, "developing-today/code", 5),
            Landing::Unknown(_)
        ));
    }

    #[test]
    fn no_probe_means_no_check() {
        let backend = TursoBackend::new(
            "http://127.0.0.1:0".to_string(),
            "test".to_string(),
            ureq::agent(),
        );
        assert!(stale_pick(&backend, &issue_task(5)).is_none());
    }

    #[test]
    fn claim_path_checks_before_binding_and_only_on_the_default_pick() {
        let src = include_str!("lifecycle.rs");
        let pick_start = src.find("fn claim_pick_once(").unwrap();
        let pick_end = src[pick_start..].find("\npub(crate) fn ").unwrap() + pick_start;
        let pick = &src[pick_start..pick_end];
        let check = pick
            .find("stale_pick(t, &candidate)")
            .expect("pick runs the check");
        let lease = pick.find("INSERT INTO leases").unwrap();
        let bound = pick.find("record_session_bound(").unwrap();
        let claimed = pick.find("\"claimed\"").unwrap();
        assert!(check < lease && check < bound && check < claimed);
        assert!(pick[..check].contains("!handoff_pick"));
        let claim_src = include_str!("claim.rs");
        assert!(
            !claim_src.contains("stale_pick("),
            "by-id and transfer claims stay overrides"
        );
    }
}
