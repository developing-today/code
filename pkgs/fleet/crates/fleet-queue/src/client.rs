//! repo-queue client — the queue domain's client surface
//! (fleet-queue, step 2 of epic developing-today/code#5412, #5430;
//! ported from the standalone repo-queue crate, code#5362).
//!
//! Same CLI surface, exit codes, and stdout formats as the retired
//! repo-queue.py. Two transports reach the same queue-broker: from
//! the sandbox, the session proxy (the laptop-agent broker's Unix
//! socket); on the laptop itself, a direct HTTP connection selected
//! by REPO_QUEUE_BROKER_URL or ~/.config/repo-queue/broker-url (the
//! opencode claimant and the worker sessions it launches use this).
//! Without a configured URL, the direct path is probe-selected
//! (#6218): fleet-config's transport choice for the queue-broker
//! role picks the broker's Unix socket when it answers; an absent
//! or stale socket keeps the session-proxy route, and a failed
//! UDS attempt falls back to it within the same call.
//! No Bitwarden fetch, no direct database access. Exit 3 means the
//! broker path itself is unavailable.
//!
//! Entry points: the fleet binary's queue domain and the standalone
//! `repo-queue` artifact (src/bin/repo-queue.rs) both call `run`.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::process::exit;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

pub mod actions_exec;
mod commands_lifecycle;
mod commands_query;
mod fallback_stack;
mod mode_report;
pub(crate) mod placement_emit;
mod support;
mod transport;
mod verify_disposition;

pub(crate) use self::commands_lifecycle::*;
pub(crate) use self::commands_query::*;
pub(crate) use self::fallback_stack::*;
pub(crate) use self::mode_report::*;
pub(crate) use self::support::*;
pub(crate) use self::transport::*;
pub(crate) use self::verify_disposition::*;

pub fn run(args: &[String]) -> i32 {
    // Bare `repo-queue --help` prints the top-level usage (#10838).
    // `<verb> --help` is NOT handled here: it dispatches to the verb,
    // whose parse_verb_args prints the verb's exact-form usage.
    if args.is_empty() {
        println!("{TOP_USAGE}");
        return 2;
    }
    if args.len() == 1 && is_help_arg(&args[0]) {
        println!("{TOP_USAGE}");
        return 0;
    }
    // Fail before dispatching any verb, so unset and empty HOME never
    // reach a broker or filesystem path with a root-anchored default.
    let _ = home();
    let sub = args[0].as_str();
    let rest = &args[1..];
    match sub {
        "validate" => cmd_validate(rest),
        "enqueue" => cmd_enqueue(rest),
        "meta" => cmd_meta(rest),
        "staleness" => cmd_staleness(rest),
        "needs-user" => cmd_needs_user(rest),
        "list" => cmd_list(rest),
        "claim" => cmd_claim(rest),
        "release" => cmd_release(rest),
        "lane" => cmd_lane(rest),
        "bind" => cmd_bind(rest),
        "heartbeat" => cmd_heartbeat(rest),
        "checkpoint" => cmd_checkpoint(rest),
        "park" => cmd_park(rest),
        "reprioritize" => cmd_reprioritize(rest),
        "wake" => cmd_wake(rest),
        "block" => cmd_block(rest),
        "unblock" => cmd_unblock(rest),
        "dep-recheck" => cmd_dep_recheck(rest),
        "quarantine-recheck" => cmd_quarantine_recheck(rest),
        "time-recheck" => cmd_time_recheck(rest),
        "repair" => cmd_repair(rest),
        "cancel" => cmd_cancel(rest),
        "revive" => cmd_revive(rest),
        "fail" => cmd_fail(rest),
        "complete" => cmd_complete(rest),
        "resolution-backfill" => cmd_resolution_backfill(rest),
        "verify-disposition" => cmd_verify_disposition(rest),
        "timeline" => cmd_timeline("timeline", rest),
        "history" => cmd_timeline("history", rest),
        "show" => cmd_show(rest),
        "note" => cmd_note(rest),
        "link" => cmd_link(rest),
        "models" => cmd_models(rest),
        "health" => cmd_health(rest),
        "signal" => cmd_signal(rest),
        "fallback" => crate::fallback::run(rest),
        "dispatch" => crate::placement::cmd_dispatch(rest),
        "session" => crate::placement::cmd_session(rest),
        "surfaces" => crate::placement::cmd_surfaces(rest),
        "placement-load" => cmd_placement_load(rest),
        "outbox" => cmd_outbox(rest),
        "stack" => cmd_stack(rest),
        _ => {
            eprint(&format!("repo-queue: unknown command {sub:?}\n{TOP_USAGE}"));
            2
        }
    }
}

/// #9177 close-guard lookup: every `claimed` and `parked` task row
/// (the statuses whose live binding makes an issue's close or
/// status:done write a guarded one), across all queues, in the
/// rich /list form that carries each row's `payload` and
/// `dedupe_key` for client-side issue matching. Paged exactly like
/// `repo-queue list`. Library-shaped — never prints, never exits:
/// Err means the broker path was unavailable and the caller owns
/// the fail-open decision (fleet gh's standing rule: the guard
/// fails open, the sweep's detective check is the net).
///
/// A rejected client token still exits 3 from the session-proxy
/// leg, exactly like any queue verb: an auth wall is a wall, not a
/// fail-open condition.
pub fn guard_rows(limit: i64) -> Result<Vec<Value>, String> {
    let limit = validate_list_limit(limit).map_err(|e| e)?;
    let mut rows: Vec<Value> = Vec::new();
    for status in ["claimed", "parked"] {
        for (offset, page_limit) in list_pages(limit) {
            let resp = call_result(
                "/list",
                &json!({
                    "rich": true,
                    "status": status,
                    "limit": page_limit,
                    "offset": offset,
                }),
            )
            .map_err(|(_, msg)| msg)?;
            if resp.get("ok").and_then(|o| o.as_bool()) != Some(true) {
                let msg = resp
                    .get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("queue-broker /list error")
                    .to_string();
                return Err(msg);
            }
            let page = list_rows(&resp)?;
            let short_page = (page.len() as i64) < page_limit;
            rows.extend(page);
            if short_page {
                break;
            }
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn list_marks_manual_only_rows_9178() {
        let cells = list_row_cells(&json!({
            "id": 7,
            "priority": 10,
            "status": "queued",
            "manual_only": true,
            "created_at": "2026-11-25T00:00:00Z",
            "title": "manual item",
        }));
        assert_eq!(cells[2], "queued (manual-only)");
    }

    #[test]
    fn execution_lane_split_7598() {
        // Oracle (#7598): a worker passing its own lane id in
        // --execution.lane is accepted; the recorded execution
        // carries the mode in `lane` and the lane id in `lane_id`,
        // so a claim-to-complete round trip leaves claim lane ==
        // completion lane_id. The old mode values pass through
        // unchanged, and garbage still dies with a clear message.
        let v = normalize_execution(json!({"lane": "any-n"})).unwrap();
        assert_eq!(v["lane"], json!("direct"));
        assert_eq!(v["lane_id"], json!("any-n"));
        let v = normalize_execution(json!({"lane": "opencode-a"})).unwrap();
        assert_eq!(v["lane"], json!("opencode"));
        assert_eq!(v["lane_id"], json!("opencode-a"));
        let v = normalize_execution(json!({"lane": "code-a"})).unwrap();
        assert_eq!(v["lane"], json!("direct"));
        assert_eq!(v["lane_id"], json!("code-a"));
        // Mode-only callers are untouched (no lane_id invented).
        let v = normalize_execution(json!({"lane": "opencode"})).unwrap();
        assert_eq!(v["lane"], json!("opencode"));
        assert!(v.get("lane_id").is_none());
        let v = normalize_execution(json!({"lane": "direct"})).unwrap();
        assert_eq!(v["lane"], json!("direct"));
        assert!(v.get("lane_id").is_none());
        // Explicit lane_id alongside a mode is kept as-is.
        let v = normalize_execution(json!({"lane": "direct", "lane_id": "any-f"})).unwrap();
        assert_eq!(v["lane"], json!("direct"));
        assert_eq!(v["lane_id"], json!("any-f"));
        // Disagreement and garbage are refused, naming the semantics.
        assert!(normalize_execution(json!({"lane": "any-n", "lane_id": "any-f"})).is_err());
        assert!(normalize_execution(json!({"lane": "banana"})).is_err());
        assert!(normalize_execution(json!({"lane_id": "banana-1"})).is_err());
    }

    #[test]
    fn timeline_surfaces_claim_displacement_events_6985() {
        // #6985: the recurrence on task 17490 read as "exactly two
        // events" because the timeline's kind list omitted the
        // lease-expiry displacement and any claim refusal — the two
        // transitions that explain a double claim. They render now.
        let src = [
            include_str!("client/transport.rs"),
            include_str!("client/support.rs"),
            include_str!("client/commands_query.rs"),
            include_str!("client/commands_lifecycle.rs"),
            include_str!("client/fallback_stack.rs"),
            include_str!("client.rs"),
        ]
        .concat();
        let seg = src
            .split("fn cmd_timeline(")
            .nth(1)
            .and_then(|s| s.split("let resp = call(").next())
            .unwrap_or("");
        for kind in [
            "\"lease_expired\"",
            "\"lease_expired_at_cap\"",
            "\"claim_refused\"",
        ] {
            assert!(
                seg.contains(kind),
                "cmd_timeline queries {kind} so claim displacements are visible"
            );
        }
    }

    #[test]
    fn timeline_surfaces_staleness_autocomplete_14268() {
        // #14268: a claim-time staleness retirement writes exactly
        // one event, `staleness_autocomplete`, and no `claimed`
        // event. While the history kind lists omitted it, four
        // retired tasks read as "zero history events, no result"
        // through every client surface and were misdiagnosed as
        // silent completions. Both surfaces that assemble a task's
        // history query the kind now.
        let src = include_str!("client/commands_query.rs");
        let timeline_seg = src
            .split("fn cmd_timeline(")
            .nth(1)
            .and_then(|s| s.split("let resp = call(").next())
            .unwrap_or("");
        assert!(
            timeline_seg.contains("\"staleness_autocomplete\""),
            "cmd_timeline queries staleness_autocomplete so retirements are visible"
        );
        let show_seg = src
            .split("fn cmd_show(")
            .nth(1)
            .and_then(|s| s.split("fn cmd_timeline(").next())
            .unwrap_or("");
        assert!(
            show_seg.contains("\"staleness_autocomplete\""),
            "cmd_show queries staleness_autocomplete so retirements are visible"
        );
    }

    #[test]
    fn show_and_timeline_surface_suspect_completions_14268() {
        // #14268 ask 2: the broker stamps a suspect completion and
        // writes a suspect_completion event, but that only surfaces
        // if the read paths query the kind — the same blind-spot
        // shape as #6985 and the staleness_autocomplete half of
        // #14292. Both per-task surfaces must ask for it.
        let src = [
            include_str!("client/transport.rs"),
            include_str!("client/support.rs"),
            include_str!("client/commands_query.rs"),
            include_str!("client/commands_lifecycle.rs"),
            include_str!("client/fallback_stack.rs"),
            include_str!("client.rs"),
        ]
        .concat();
        let show = src
            .split("fn cmd_show(")
            .nth(1)
            .and_then(|s| s.split("fn cmd_timeline(").next())
            .unwrap_or("");
        assert!(
            show.contains("\"suspect_completion\""),
            "cmd_show queries suspect_completion so a suspect completion is visible in a task's history"
        );
        let timeline = src
            .split("fn cmd_timeline(")
            .nth(1)
            .and_then(|s| s.split("let resp = call(").next())
            .unwrap_or("");
        assert!(
            timeline.contains("\"suspect_completion\""),
            "cmd_timeline queries suspect_completion so a suspect completion is visible on the timeline"
        );
    }

    #[test]
    fn collapse_timeline_9046_drops_only_duplicate_writes() {
        // #9046 oracle: same (kind, session, leaf, trigger) inside
        // 120 s = one record written by two surfaces; a changed
        // field or a later repeat is a real second record.
        let ev = |id: i64, us: i64, model: &str| TimelineEvent {
            id,
            kind: "model_stamp".to_string(),
            session_id: Some("ses_a".to_string()),
            provider: Some("opencode-route".to_string()),
            model: Some(model.to_string()),
            variant: None,
            trigger: Some("claim".to_string()),
            created_at: "2026-11-12T00:00:00Z".to_string(),
            created_us: us,
        };
        let out = collapse_timeline(vec![
            ev(1, 0, "workhorse"),
            ev(2, 30_000_000, "workhorse"),
            ev(3, 60_000_000, "workhorse-x"),
            ev(4, 300_000_000, "workhorse-x"),
        ]);
        let ids: Vec<i64> = out.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![1, 3, 4]);
    }

    #[test]
    fn show_history_uses_history_event_fields_and_is_a_tail() {
        let event = |id, trigger: &str| TimelineEvent {
            id,
            kind: "claimed".to_string(),
            session_id: Some("ses_example".to_string()),
            provider: Some("opencode-route".to_string()),
            model: Some("workhorse".to_string()),
            variant: Some("medium".to_string()),
            trigger: Some(trigger.to_string()),
            created_at: format!("2026-11-21T00:00:{id:02}Z"),
            created_us: id * 1_000_000,
        };
        let events = vec![event(1, "claim"), event(2, "resume")];
        let tail = &events[events.len().saturating_sub(1)..];
        assert_eq!(tail[0].id, 2);
        assert_eq!(
            format_timeline_event(&tail[0]),
            "2026-11-21T00:00:02Z  claimed  session=ses_example  opencode-route/workhorse#medium  (resume)"
        );
    }

    #[test]
    fn bare_claim_refuses_and_intent_claims_pass_8606() {
        // #8606 oracle: the zero-intent claim shape that fired
        // five times during CLI probing refuses; every documented
        // claim form passes. Args are built through the same
        // parser cmd_claim uses, with its own flag/bool lists.
        let mk = |tokens: &[&str]| {
            let rest: Vec<String> = tokens.iter().map(|s| s.to_string()).collect();
            parse_verb_args(
                "claim",
                &rest,
                &[
                    "queue",
                    "worker",
                    "lease-seconds",
                    "lane",
                    "task",
                    "count",
                    "from-session",
                    "platform",
                    "session-id",
                    "model-provider",
                    "model-id",
                    "model-variant",
                    "filter",
                    "pool",
                    "launched-by",
                    "remaining-budget-minutes",
                    "manual-override",
                ],
                &["handoff", "bind"],
            )
        };
        // Bare: no arguments at all — the probing shape.
        assert!(bare_claim_refusal(&mk(&[]), &None).is_some());
        // A lease alone is not intent either.
        assert!(bare_claim_refusal(&mk(&["--lease-seconds", "60"]), &None).is_some());
        // An env queue scope is intent even with no flags.
        assert!(
            bare_claim_refusal(&mk(&[]), &Some("repo-health-code-pilot".to_string())).is_none()
        );
        // The documented forms all pass: scoped, lane, directed,
        // filtered, batch, handoff.
        assert!(bare_claim_refusal(&mk(&["--queue", "repo-health-code-pilot"]), &None).is_none());
        assert!(
            bare_claim_refusal(
                &mk(&[
                    "--worker",
                    "repo-worker-code",
                    "--lane",
                    "any-b",
                    "--lease-seconds",
                    "2700",
                    "--remaining-budget-minutes",
                    "120",
                ]),
                &None,
            )
            .is_none()
        );
        assert!(bare_claim_refusal(&mk(&["--task", "123"]), &None).is_none());
        assert!(bare_claim_refusal(&mk(&["--filter", "repo = code"]), &None).is_none());
        assert!(
            bare_claim_refusal(
                &mk(&["--count", "3", "--worker", "repo-worker-code"]),
                &None
            )
            .is_none()
        );
        assert!(bare_claim_refusal(&mk(&["--handoff"]), &None).is_none());
    }

    #[test]
    fn rollup_event_reads_row_task_id() {
        // Claim-link rollup: the row's own task_id column joins
        // the stamp (fleet-cli stamp details carry no task_id).
        let row = json!({
            "id": 7, "task_id": 42, "kind": "model_stamp",
            "detail": "{\"session_id\":\"ses_a1\",\"provider\":\"opencode-route\",\"model\":\"workhorse\",\"variant\":null,\"trigger\":\"bind\",\"via\":\"fleet-cli\"}",
            "created_at": "2026-11-14T17:13:09Z",
        });
        let re = rollup_event(&row).expect("row parses");
        assert_eq!(re.task_id, 42);
        assert_eq!(re.ev.session_id.as_deref(), Some("ses_a1"));
        let no_task = json!({"id": 8, "kind": "model_stamp", "detail": "{}", "created_at": "2026-11-14T17:13:09Z"});
        assert!(rollup_event(&no_task).is_none());
    }

    #[test]
    fn rollup_models_counts_tasks_once_per_model() {
        let ev = |task: i64, sid: Option<&str>, p: &str, m: &str| RollupEvent {
            task_id: task,
            ev: TimelineEvent {
                id: 0,
                kind: "model_stamp".to_string(),
                session_id: sid.map(|s| s.to_string()),
                provider: Some(p.to_string()),
                model: Some(m.to_string()),
                variant: None,
                trigger: Some("claim".to_string()),
                created_at: "2026-11-14T00:00:00Z".to_string(),
                created_us: 0,
            },
        };
        let events = vec![
            ev(1, Some("ses_a"), "opencode-route", "workhorse"),
            ev(1, Some("ses_a"), "opencode-route", "workhorse"),
            ev(2, None, "opencode-route", "workhorse"),
            ev(3, None, "muse-lane", "muse-lane"),
        ];
        let completed: std::collections::BTreeSet<i64> = [1, 3].into_iter().collect();
        let out = rollup_models(&events, &completed);
        assert_eq!(out.len(), 2);
        let wh = out.iter().find(|(k, _)| k.model == "workhorse").unwrap();
        assert_eq!(wh.1.tasks.len(), 2, "task 1 counts once despite two stamps");
        assert_eq!(wh.1.stamps, 3);
        assert_eq!(wh.1.sessions.len(), 1);
        assert_eq!(wh.1.completed, 1);
        let ml = out.iter().find(|(k, _)| k.model == "muse-lane").unwrap();
        assert_eq!(ml.1.completed, 1);
        assert_eq!(ml.1.sessions.len(), 0);
    }

    #[test]
    fn rollup_served_aggregates_windows_and_calls() {
        let key = |p: &str, m: &str| ModelKey {
            provider: p.to_string(),
            model: m.to_string(),
            variant: None,
        };
        let rows = vec![
            (1, key("opencode-go.drewrypope", "beam-free"), 13),
            (1, key("opencode-go.drewrypope", "beam-free"), 4),
            (2, key("opencode-go.drewrypope", "beam-free"), 7),
            (2, key("opencode-go.popesdrew", "beam-free"), 1),
        ];
        let out = rollup_served(&rows);
        assert_eq!(out.len(), 2);
        let top = &out[0].1;
        assert_eq!(top.tasks.len(), 2);
        assert_eq!(top.windows, 3);
        assert_eq!(top.calls, 24);
    }

    #[test]
    fn iso_utc_formats_the_store_shape() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_794_676_389), "2026-11-14T17:13:09Z");
    }

    #[test]
    fn timeline_interleaves_notes_into_one_story_14170_q4() {
        // #14170 Q4 oracle: notes and events render as one
        // chronological story; each keeps its own order, and a
        // same-second tie puts the event first.
        let ev = |id: i64, us: i64| TimelineEvent {
            id,
            kind: "claimed".to_string(),
            session_id: None,
            provider: None,
            model: None,
            variant: None,
            trigger: None,
            created_at: format!("e{id}"),
            created_us: us,
        };
        let note = |id: i64, us: i64| TimelineNote {
            id,
            author: "w".to_string(),
            author_kind: "agent".to_string(),
            session_id: None,
            provider: None,
            model: None,
            variant: None,
            body: format!("n{id}\nsecond line"),
            created_at: format!("n{id}"),
            created_us: us,
            edited_at: None,
            deleted_at: None,
        };
        let events = [ev(1, 10), ev(2, 20), ev(3, 40)];
        let notes = [note(7, 5), note(8, 20), note(9, 30), note(10, 50)];
        let story: Vec<String> = interleave_notes(&events, &notes)
            .iter()
            .map(|e| match e {
                TimelineEntry::Event(ev) => ev.created_at.clone(),
                TimelineEntry::Note(n) => n.created_at.clone(),
            })
            .collect();
        assert_eq!(story, ["n7", "e1", "e2", "n8", "n9", "e3", "n10"]);
        let line = format_timeline_note(&note(7, 5));
        assert!(line.starts_with("n7  note #7  by w"), "{line}");
        assert!(line.ends_with("\n    n7\n    second line"), "{line}");
        assert!(interleave_notes(&[], &[]).is_empty());
    }

    #[test]
    fn binding_windows_9046_pair_bounds_and_unbounds() {
        // #9046 oracle: windows pair per session in bound order;
        // an unclosed binding stays open (end None) — its slice
        // of the session runs to now.
        let ev = |id: i64, kind: &str, sid: &str, at: &str| TimelineEvent {
            id,
            kind: kind.to_string(),
            session_id: Some(sid.to_string()),
            provider: None,
            model: None,
            variant: None,
            trigger: None,
            created_at: at.to_string(),
            created_us: 0,
        };
        let out = binding_windows(&[
            ev(1, "session_bound", "ses_a", "t1"),
            ev(2, "session_unbound", "ses_a", "t2"),
            ev(3, "session_bound", "ses_b", "t3"),
        ]);
        assert_eq!(
            out,
            vec![
                (
                    "ses_a".to_string(),
                    "t1".to_string(),
                    Some("t2".to_string())
                ),
                ("ses_b".to_string(), "t3".to_string(), None),
            ]
        );
    }

    #[test]
    fn valid_session_id_9046_rejects_non_session_shapes() {
        // #9046 oracle: only the fleet's own session-id shape may
        // reach a route_attempt SQL string.
        assert!(valid_session_id("ses_e5e37addeb0ffeumu1UPO2bI8G"));
        assert!(!valid_session_id("ses_a'; DROP TABLE route_attempt; --"));
        assert!(!valid_session_id("not-a-session"));
        assert!(!valid_session_id("ses_"));
        assert!(!valid_session_id(""));
    }

    #[test]
    fn model_stamp_flags_require_provider_and_model_together() {
        // #8176 oracle: the claim stamp names the serving leaf —
        // provider and model together, variant optional. Either
        // half alone (or a variant alone) is a usage error.
        assert_eq!(
            model_stamp_flags(Some("_claude"), Some("claude-sonnet-5-5"), Some("medium")),
            Ok(Some((
                "_claude".to_string(),
                "claude-sonnet-5-5".to_string(),
                Some("medium".to_string()),
            )))
        );
        assert_eq!(
            model_stamp_flags(Some("openai.main"), Some("gpt-6-luna"), None),
            Ok(Some((
                "openai.main".to_string(),
                "gpt-6-luna".to_string(),
                None,
            )))
        );
        assert_eq!(model_stamp_flags(None, None, None), Ok(None));
        assert!(model_stamp_flags(Some("_claude"), None, None).is_err());
        assert!(model_stamp_flags(None, Some("claude-sonnet-5-5"), None).is_err());
        assert!(model_stamp_flags(None, None, Some("medium")).is_err());
    }

    #[test]
    fn session_model_stamp_reads_the_session_record() {
        // #8176 derivation oracle, as corrected by #9408: the stamp
        // is the SERVING leaf. A direct session's own model —
        // providerID + id from the server's session record
        // (enveloped or bare) — is its leaf; a "default" variant is
        // no variant.
        let bare = json!({
            "id": "ses_y", "model": {"id": "gpt-6-luna", "providerID": "openai-plan", "variant": "high"}
        });
        assert_eq!(
            session_model_stamp(&bare),
            Some((
                "openai-plan".to_string(),
                "gpt-6-luna".to_string(),
                Some("high".to_string()),
            ))
        );
        // #9408: a routed session's model is the configured route
        // alias (opencode-route/workhorse), not a serving leaf —
        // with no route_usage readout on the record the claim is
        // left unstamped instead of recording the alias.
        let enveloped = json!({
            "data": {"id": "ses_x", "model": {"id": "workhorse", "providerID": "opencode-route", "variant": "default"}}
        });
        assert_eq!(session_model_stamp(&enveloped), None);
        // #9408: when the record carries the route_usage readout
        // (opencode#1891), its lastServing leaf is the stamp — even
        // though the declared model is the route alias.
        let routed = json!({
            "data": {
                "id": "ses_x",
                "model": {"id": "workhorse", "providerID": "opencode-route", "variant": "default"},
                "route_usage": {"lastServing": {"providerID": "openai.main", "modelID": "gpt-6-luna", "variant": "high"}}
            }
        });
        assert_eq!(
            session_model_stamp(&routed),
            Some((
                "openai.main".to_string(),
                "gpt-6-luna".to_string(),
                Some("high".to_string()),
            ))
        );
        // A present-but-incomplete readout stamps nothing; it never
        // falls back to the route alias.
        let incomplete = json!({
            "model": {"id": "workhorse", "providerID": "opencode-route"},
            "route_usage": {"lastServing": {"providerID": "openai.main"}}
        });
        assert_eq!(session_model_stamp(&incomplete), None);
        // Half a model, or no model, stamps nothing.
        assert_eq!(session_model_stamp(&json!({"data": {"id": "ses_z"}})), None);
        assert_eq!(
            session_model_stamp(&json!({"model": {"id": "workhorse"}})),
            None
        );
        assert_eq!(
            session_model_stamp(&json!({"model": {"providerID": "opencode-route"}})),
            None
        );
    }

    #[test]
    fn b64_encode_matches_the_standard_alphabet() {
        // The server's Basic auth is the only consumer; vectors
        // are the RFC 4648 canonical set.
        assert_eq!(b64_encode(b""), "");
        assert_eq!(b64_encode(b"f"), "Zg==");
        assert_eq!(b64_encode(b"fo"), "Zm8=");
        assert_eq!(b64_encode(b"foo"), "Zm9v");
        assert_eq!(b64_encode(b"opencode:pw"), "b3BlbmNvZGU6cHc=");
    }

    #[test]
    fn platform_flag_accepts_only_the_two_families() {
        // #8097 Rule 2: the selector is a coarse family, never a
        // lane name — a lane id as --platform is a usage error.
        assert_eq!(platform_flag("openchamber"), Ok("openchamber".to_string()));
        assert_eq!(platform_flag("muse"), Ok("muse".to_string()));
        assert!(platform_flag("code-a").is_err());
        assert!(platform_flag("").is_err());
    }

    #[test]
    fn batch_count_flag_bounds_8217() {
        // #8217: absent/1 = the single claim; 2..=cap = a batch;
        // anything else is a usage error naming the cap.
        assert_eq!(batch_count_flag(None), Ok(1));
        assert_eq!(batch_count_flag(Some(1)), Ok(1));
        assert_eq!(batch_count_flag(Some(2)), Ok(2));
        assert_eq!(
            batch_count_flag(Some(crate::BATCH_CLAIM_MAX)),
            Ok(crate::BATCH_CLAIM_MAX)
        );
        assert!(batch_count_flag(Some(0)).is_err());
        assert!(batch_count_flag(Some(-3)).is_err());
        assert!(batch_count_flag(Some(crate::BATCH_CLAIM_MAX + 1)).is_err());
    }

    #[test]
    fn stack_transition_body_carries_the_transition_shape_8582() {
        // #8582 increment 4 oracle: the client transition body is
        // exactly the broker's transition shape (stack, position,
        // target state in the core's spelling, optional landing PR
        // number and conflict detail).
        let body = stack_transition_body("outage-2026-11-11", 2, "landed", Some(8810), None)
            .expect("a landing transition is well-formed");
        assert_eq!(body["stack_id"], json!("outage-2026-11-11"));
        assert_eq!(body["position"], json!(2));
        assert_eq!(body["to_state"], json!("landed"));
        assert_eq!(body["pr_number"], json!(8810));
        assert_eq!(body["conflict_detail"], Value::Null);
        let body = stack_transition_body(
            "outage-2026-11-11",
            1,
            "conflicted",
            None,
            Some("rebase onto main conflicts in client.rs"),
        )
        .expect("a conflicted transition with detail is well-formed");
        assert_eq!(body["to_state"], json!("conflicted"));
        assert_eq!(
            body["conflict_detail"],
            json!("rebase onto main conflicts in client.rs")
        );
    }

    #[test]
    fn stack_transition_body_rejects_unknown_state_and_bare_conflict_8582() {
        // A state the core cannot parse, and a conflict recorded
        // without the detail its authoring lane needs, are usage
        // errors here — they never reach the broker.
        assert!(stack_transition_body("s1", 0, "bogus", None, None).is_err());
        assert!(stack_transition_body("s1", 0, "conflicted", None, None).is_err());
        assert!(stack_transition_body("s1", 0, "conflicted", None, Some("  ")).is_err());
        assert!(stack_transition_body(" ", 0, "replaying", None, None).is_err());
        assert!(stack_transition_body("s1", 0, "replaying", None, None).is_ok());
    }

    #[test]
    fn stack_restack_body_carries_the_refresh_shape_8582() {
        // Oracle: the restack acknowledgement carries exactly the
        // fields the broker's plan_restack consumes (#8582) — stack,
        // position, the new base, and the restacked head when given.
        let body = stack_restack_body("outage-2026-11-11", 2, "main", Some("deadbeef"))
            .expect("valid restack body");
        assert_eq!(body["stack_id"], "outage-2026-11-11");
        assert_eq!(body["position"], 2);
        assert_eq!(body["new_base_branch"], "main");
        assert_eq!(body["head_sha"], "deadbeef");
        assert!(stack_restack_body("s1", 2, "main", None).is_ok());
        assert!(stack_restack_body(" ", 2, "main", None).is_err());
        assert!(stack_restack_body("s1", 2, " ", None).is_err());
        assert!(stack_restack_body("s1", 2, "main", Some(" ")).is_err());
    }

    #[test]
    fn stack_record_body_carries_the_append_shape_8582() {
        // #8582 increment 3 oracle: the client record body is
        // exactly the broker's append shape (stack, repo, branch,
        // owner lane/worker pair, optional base/head/issue/pr/
        // session), and the pure core's append rules are the
        // client's fail-fast rules too — an ownerless or
        // branchless record never reaches the broker.
        let body = stack_record_body(
            "outage-2026-11-11",
            "developing-today/code",
            "queue/1-2-fix",
            None,
            Some("abc123"),
            Some(8582),
            None,
            "any-a",
            "repo-worker-code",
            Some("ses_1"),
        )
        .expect("a complete append is valid");
        assert_eq!(body["stack_id"], json!("outage-2026-11-11"));
        assert_eq!(body["repo"], json!("developing-today/code"));
        assert_eq!(body["branch"], json!("queue/1-2-fix"));
        assert_eq!(body["base_branch"], Value::Null);
        assert_eq!(body["head_sha"], json!("abc123"));
        assert_eq!(body["issue_number"], json!(8582));
        assert_eq!(body["owner_lane"], json!("any-a"));
        assert_eq!(body["owner_worker"], json!("repo-worker-code"));
        assert_eq!(body["owner_session"], json!("ses_1"));
        assert!(
            stack_record_body(
                "s",
                "developing-today/code",
                "queue/1-2-fix",
                None,
                None,
                None,
                None,
                "",
                "repo-worker-code",
                None,
            )
            .is_err()
        );
        assert!(
            stack_record_body(
                "s",
                "developing-today/code",
                "",
                None,
                None,
                None,
                None,
                "any-a",
                "repo-worker-code",
                None,
            )
            .is_err()
        );
        assert!(
            stack_record_body(
                "s",
                "developing-today/code",
                "queue/1-2-fix",
                Some("  "),
                None,
                None,
                None,
                "any-a",
                "repo-worker-code",
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn fallback_switch_json_shape() {
        let v = fallback_switch_json("/claim", 4, "store down");
        assert_eq!(v["ok"], json!(false));
        assert_eq!(v["fallback"], json!(true));
        assert_eq!(v["switch_to"], json!("github-fallback"));
        assert_eq!(v["verb"], json!("/claim"));
        assert_eq!(v["broker_exit"], json!(4));
        assert_eq!(FALLBACK_SWITCH_EXIT, 6);
    }

    #[test]
    fn mode_line_reports_the_route_the_verb_actually_used() {
        // #8581 oracle: the client adapter selects through
        // route_for — a served verb on a healthy store reports
        // normal/cloud; a failed GitHub leg degrades the mode,
        // not the route (writes before reads); a dead store with
        // no client-held replica lands on the marker fallback,
        // db-degraded.
        let v = mode_line_for("claim", legs_served(true, true)).json_value();
        assert_eq!(
            v,
            json!({"verb": "claim", "mode": "normal", "route": "cloud"})
        );
        let v = mode_line_for("claim", legs_served(true, false)).json_value();
        assert_eq!(
            v,
            json!({"verb": "claim", "mode": "gh-read-degraded", "route": "cloud"})
        );
        let v = mode_line_for("claim", legs_served(false, true)).json_value();
        assert_eq!(
            v,
            json!({"verb": "claim", "mode": "gh-write-degraded", "route": "cloud"})
        );
        let v = mode_line_for("/claim", legs_store_down()).json_value();
        assert_eq!(
            v,
            json!({"verb": "claim", "mode": "db-degraded", "route": "github-markers"})
        );
        // The bind path is the claim verb served in one round
        // trip: /claim-flow reports as "claim".
        let v = mode_line_for("/claim-flow", legs_store_down()).json_value();
        assert_eq!(v["verb"], json!("claim"));
        assert_eq!(v["route"], json!("github-markers"));
    }

    #[test]
    fn fallback_switch_json_carries_the_contract_mode_line() {
        // #8581: the switch envelope names the mode and route the
        // contract assigns to a primary-store failure with no
        // local replica: db-degraded on github-markers, under the
        // same mode_line shape the served path prints.
        let v = fallback_switch_json("/heartbeat", 3, "broker unreachable");
        assert_eq!(
            v["mode_line"],
            json!({"verb": "heartbeat", "mode": "db-degraded", "route": "github-markers"})
        );
    }

    #[test]
    fn attach_mode_line_adds_the_key_to_objects_only() {
        // #8581: the mode line rides the verb's printed object
        // under `mode_line` (the guard_warning /
        // claim_flow_partial additive-key pattern); a non-object
        // output has nowhere to carry it and is left untouched.
        let mut row = json!({"id": 7, "status": "claimed"});
        attach_mode_line(&mut row, "heartbeat", legs_served(true, true));
        assert_eq!(
            row["mode_line"],
            json!({"verb": "heartbeat", "mode": "normal", "route": "cloud"})
        );
        assert_eq!(row["id"], json!(7));
        let mut scalar = json!(5);
        attach_mode_line(&mut scalar, "heartbeat", legs_served(true, true));
        assert_eq!(scalar, json!(5));
    }

    #[test]
    fn served_lifecycle_verbs_report_normal_cloud() {
        // #8581 acceptance remainder: every task-row lifecycle
        // verb whose served path prints the broker's row carries
        // the contract mode line — a broker-served verb on a
        // healthy store reports normal/cloud under its own CLI
        // verb name (claim/heartbeat/checkpoint wire their legs
        // explicitly and are covered above).
        for verb in [
            "release",
            "lane",
            "bind",
            "park",
            "wake",
            "block",
            "unblock",
            "dep-recheck",
            "quarantine-recheck",
            "time-recheck",
            "cancel",
            "revive",
            "fail",
            "complete",
        ] {
            let row = served_row(json!({"id": 7, "status": "claimed"}), verb);
            assert_eq!(
                row["mode_line"],
                json!({"verb": verb, "mode": "normal", "route": "cloud"}),
                "served_row for {verb}"
            );
            assert_eq!(row["id"], json!(7));
        }
    }

    #[test]
    fn guard_decision_holds_only_live_or_contested_others() {
        let live_other = json!({"state": "live", "worker": "lane-b"});
        assert_eq!(
            guard_decision(&live_other, "lane-a"),
            GuardDecision::HeldBy("lane-b".to_string())
        );
        let contested = json!({"state": "contested", "worker": "lane-b"});
        assert_eq!(
            guard_decision(&contested, "lane-a"),
            GuardDecision::HeldBy("lane-b".to_string())
        );
        // Held by me, stale, released, free: all servable.
        assert_eq!(guard_decision(&live_other, "lane-b"), GuardDecision::Serve);
        assert_eq!(
            guard_decision(&json!({"state": "stale", "worker": "lane-b"}), "lane-a"),
            GuardDecision::Serve
        );
        assert_eq!(
            guard_decision(&json!({"state": "released", "worker": "lane-b"}), "lane-a"),
            GuardDecision::Serve
        );
        assert_eq!(
            guard_decision(&json!({"state": "free"}), "lane-a"),
            GuardDecision::Serve
        );
    }

    #[test]
    fn task_issue_reads_payload_identity() {
        let row = json!({"payload": {"repo": "developing-today/code", "issue": {"number": 6741}}});
        assert_eq!(
            task_issue(&row),
            ("developing-today/code".to_string(), 6741)
        );
        assert_eq!(task_issue(&json!({"payload": {}})), (String::new(), 0));
    }

    fn pos(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn assert_env_resolution(resolve: impl Fn(Option<&str>) -> String, default: &str) {
        for value in [None, Some(""), Some(" \t\n")] {
            assert_eq!(resolve(value), default);
        }
        assert_eq!(
            resolve(Some(" /tmp/configured path ")),
            "/tmp/configured path"
        );
    }

    #[test]
    fn default_queue_empty_env_values_use_default() {
        assert_env_resolution(default_queue_from_env, DEFAULT_QUEUE);
    }

    #[test]
    fn opencode_base_url_honors_claimant_overrides() {
        // #9409: OPENCODE_URL (the claimant tooling's override) wins
        // over the legacy OPENCODE_SERVER_URL; trailing slashes and
        // blank values are normalized away; the shared loopback
        // default applies when neither is set.
        assert_eq!(
            resolve_opencode_base_url(Some("http://example:9999/"), Some("http://legacy:1")),
            "http://example:9999"
        );
        assert_eq!(
            resolve_opencode_base_url(None, Some("http://legacy:1/")),
            "http://legacy:1"
        );
        assert_eq!(
            resolve_opencode_base_url(Some("  "), Some("")),
            "http://127.0.0.1:13420"
        );
        assert_eq!(
            resolve_opencode_base_url(None, None),
            "http://127.0.0.1:13420"
        );
    }

    #[test]
    fn opencode_auth_file_honors_claimant_override() {
        // #9409: OPENCODE_AUTH_FILE (the claimant tooling's override)
        // wins; otherwise the production state file under home.
        assert_eq!(
            resolve_opencode_auth_file(Some("/tmp/alt-auth.json"), "/home/x"),
            "/tmp/alt-auth.json"
        );
        assert_eq!(
            resolve_opencode_auth_file(Some(" "), "/home/x"),
            "/home/x/.local/state/opencode/service-prod.json"
        );
        assert_eq!(
            resolve_opencode_auth_file(None, "/home/x"),
            "/home/x/.local/state/opencode/service-prod.json"
        );
    }

    #[test]
    fn enqueue_queue_resolution_never_falls_back_to_shadow() {
        // #7016: the flag wins over the packet, the packet wins over
        // the env, and with none of the three the enqueue fails
        // loudly — it never resolves to the shadow default.
        let packet = json!({"queue": "repo-health-code-pilot"});
        assert_eq!(
            resolve_enqueue_queue(Some("repo-health-nix-pilot"), &packet, Some("env-q")).unwrap(),
            "repo-health-nix-pilot"
        );
        assert_eq!(
            resolve_enqueue_queue(None, &packet, Some("env-q")).unwrap(),
            "repo-health-code-pilot"
        );
        assert_eq!(
            resolve_enqueue_queue(None, &json!({}), Some("env-q")).unwrap(),
            "env-q"
        );
        assert!(resolve_enqueue_queue(None, &json!({}), None).is_err());
        assert!(resolve_enqueue_queue(None, &json!({"queue": "  "}), Some("")).is_err());
        assert!(resolve_enqueue_queue(Some(""), &json!({}), None).is_err());
    }

    #[test]
    fn signal_path_empty_env_values_use_home_default() {
        let home_dir = "/home/test";
        let default = format!(
            "{home_dir}/workspace/goals/keep-the-four-repos-healthy/hidden_files/repo-queue-signal.json"
        );
        assert_env_resolution(
            |value| signal_path_from_env(value, Some(home_dir)).unwrap(),
            &default,
        );
    }

    #[test]
    fn configured_url_routes_unix_and_http_forms() {
        // #6218: unix:// selects the socket route; the historical
        // http:// form stays TCP; an empty unix path is rejected.
        assert_eq!(
            route_from_configured_url("unix:///run/fleet/queue-broker.sock"),
            Ok(DirectRoute::Unix(std::path::PathBuf::from(
                "/run/fleet/queue-broker.sock"
            )))
        );
        assert_eq!(
            route_from_configured_url("http://100.99.113.103:4801"),
            Ok(DirectRoute::Tcp("http://100.99.113.103:4801".to_string()))
        );
        assert!(route_from_configured_url("unix://").is_err());
    }

    #[test]
    fn finish_direct_response_parses_status_and_body() {
        // #6218: both direct legs share this response handling.
        let ok = b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n{\"ok\":true}";
        assert_eq!(
            finish_direct_response(ok).expect("200 parses"),
            json!({"ok": true})
        );
        assert!(finish_direct_response(b"no header split").is_err());
    }

    #[test]
    fn finish_direct_response_dechunks_and_rejects_invalid_success_json() {
        let response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"ok\r\n4\r\n\":tr\r\n3\r\nue}\r\n0\r\n\r\n";
        assert_eq!(
            finish_direct_response(response).unwrap(),
            json!({"ok": true})
        );

        let malformed = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nnope!";
        let error = finish_direct_response(malformed).unwrap_err();
        assert!(error.contains("invalid JSON body"), "{error}");

        let truncated = b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n[]";
        let error = finish_direct_response(truncated).unwrap_err();
        assert!(error.contains("response truncated"), "{error}");
    }

    #[test]
    fn chunk_decoder_rejects_truncated_and_malformed_chunks() {
        assert!(
            decode_chunked_body(b"5\r\nabc")
                .unwrap_err()
                .contains("truncated")
        );
        assert!(
            decode_chunked_body(b"x\r\nabc\r\n0\r\n\r\n")
                .unwrap_err()
                .contains("malformed chunk size")
        );
    }

    #[test]
    fn token_file_empty_env_values_use_home_default() {
        let home_dir = "/home/test";
        let default = format!("{home_dir}/.config/queue-broker/token");
        assert_env_resolution(
            |value| token_file_from_env(value, Some(home_dir)).unwrap(),
            &default,
        );
    }

    #[test]
    fn missing_or_empty_home_rejects_socket_token_and_signal_paths() {
        for home_dir in [None, Some("")] {
            assert_eq!(
                broker_sock_from_home(home_dir),
                Err(crate::home::HOME_REQUIRED)
            );
            assert_eq!(
                token_file_from_env(None, home_dir),
                Err(crate::home::HOME_REQUIRED)
            );
            assert_eq!(
                signal_path_from_env(None, home_dir),
                Err(crate::home::HOME_REQUIRED)
            );
        }
    }

    #[test]
    fn home_derived_socket_token_and_signal_paths_are_unchanged() {
        let home_dir = Some("/home/test");
        assert_eq!(
            broker_sock_from_home(home_dir),
            Ok("/home/test/.cache/laptop-agent/broker.sock".to_string())
        );
        assert_eq!(
            token_file_from_env(None, home_dir),
            Ok("/home/test/.config/queue-broker/token".to_string())
        );
        assert_eq!(
            signal_path_from_env(None, home_dir),
            Ok("/home/test/workspace/goals/keep-the-four-repos-healthy/hidden_files/repo-queue-signal.json".to_string())
        );
    }

    #[test]
    fn extra_positional_names_the_first_unexpected_one() {
        // #5592: at or under the verb's arity there is nothing to reject.
        assert_eq!(extra_positional(&pos(&[]), 0), None);
        assert_eq!(extra_positional(&pos(&["a.json"]), 1), None);
        assert_eq!(extra_positional(&pos(&["123"]), 1), None);
        // Past it, the FIRST extra is the one named in the usage error:
        // validate/enqueue read pos[0] only, task verbs pos_i64(0) only,
        // and list/health/signal read none.
        assert_eq!(extra_positional(&pos(&["junk"]), 0), Some("junk"));
        assert_eq!(
            extra_positional(&pos(&["a.json", "b.json"]), 1),
            Some("b.json")
        );
        assert_eq!(extra_positional(&pos(&["123", "456"]), 1), Some("456"));
    }

    #[test]
    fn duplicate_value_flag_is_a_usage_error() {
        // #5939: last-wins used to silently retarget the invocation.
        let err = parse_args_inner(&pos(&["--queue", "a", "--queue", "b"]), &["queue"], &[])
            .err()
            .expect("duplicate --queue must be rejected");
        assert!(err.contains("--queue"), "error names the flag: {err}");
        // Inline and separate forms of the same flag also collide.
        assert!(parse_args_inner(&pos(&["--queue=a", "--queue", "b"]), &["queue"], &[]).is_err());
    }

    #[test]
    fn flag_token_as_value_is_a_usage_error() {
        // #8582 (Codex P1 on PR #8790): a value flag followed by a flag
        // token used to consume that token as its value, so
        // `stack record --head-sha --dry-run` persisted "--dry-run" as
        // the head SHA on a mutating broker call instead of erroring.
        // Oracle: the operator argument contract (AGENTS.md #3352) - a
        // present-but-valueless option is a usage error naming the flag.
        let err = parse_args_inner(
            &pos(&["--head-sha", "--dry-run"]),
            &["head-sha"],
            &["dry-run"],
        )
        .err()
        .expect("flag token must not be consumed as a value");
        assert!(err.contains("--head-sha"), "error names the flag: {err}");
        assert!(
            err.contains("expected one argument"),
            "error names the defect: {err}"
        );
        // Any flag-shaped token is refused, known or not, and the
        // refusal fires even when a well-formed flag follows it.
        assert!(
            parse_args_inner(
                &pos(&["--queue", "--bogus", "--limit", "5"]),
                &["queue", "limit"],
                &[]
            )
            .is_err()
        );
        // Ordinary values still parse, including single-dash values.
        let args = parse_args_inner(&pos(&["--queue", "alpha"]), &["queue"], &[])
            .ok()
            .expect("ordinary value parses");
        assert_eq!(args.get("queue").as_deref(), Some("alpha"));
        let args = parse_args_inner(&pos(&["--limit", "-1"]), &["limit"], &[])
            .ok()
            .expect("single-dash value parses");
        assert_eq!(args.get("limit").as_deref(), Some("-1"));
    }

    #[test]
    fn duplicate_bool_flag_is_a_usage_error() {
        let err = parse_args_inner(&pos(&["--dry-run", "--dry-run"]), &[], &["dry-run"])
            .err()
            .expect("duplicate --dry-run must be rejected");
        assert!(err.contains("--dry-run"), "error names the flag: {err}");
    }

    #[test]
    fn bool_flag_with_inline_value_is_a_usage_error() {
        // #5939: --dry-run=false used to be read as true.
        for raw in ["--dry-run=false", "--dry-run=true"] {
            let err = parse_args_inner(&pos(&[raw]), &[], &["dry-run"])
                .err()
                .expect("bool with inline value must be rejected");
            assert!(err.contains("--dry-run"), "error names the flag: {err}");
        }
        // A bare boolean keeps working.
        let args = parse_args_inner(&pos(&["--dry-run"]), &[], &["dry-run"])
            .ok()
            .expect("bare bool parses");
        assert!(args.has("dry-run"));
    }

    #[test]
    fn non_numeric_numeric_flag_is_a_usage_error() {
        // #5939: get_i64 used parse().ok(), so --limit abc silently
        // fell back to the caller's default.
        let err = parse_i64_flag("limit", "abc")
            .err()
            .expect("non-numeric --limit must be rejected");
        assert!(err.contains("--limit"), "error names the flag: {err}");
        assert!(err.contains("abc"), "error names the value: {err}");
        assert_eq!(parse_i64_flag("limit", "20"), Ok(20));
    }

    #[test]
    fn list_limit_range_is_validated() {
        // #6419: 0 silently reported no tasks; negatives and unbounded
        // values reached the broker verbatim. Out-of-range is a usage
        // error naming --limit (design #6448 Phase 3: reject, not clamp).
        for bad in [0, -1, LIST_LIMIT_MAX + 1, i64::MAX] {
            let err = validate_list_limit(bad)
                .err()
                .expect("out-of-range --limit must be rejected");
            assert!(err.contains("--limit"), "error names the flag: {err}");
        }
        // Boundaries and the ordinary value pass through unchanged.
        assert_eq!(validate_list_limit(1), Ok(1));
        assert_eq!(validate_list_limit(20), Ok(20));
        assert_eq!(validate_list_limit(LIST_LIMIT_MAX), Ok(LIST_LIMIT_MAX));
        // Absent --limit keeps the existing default of 20.
        assert_eq!(list_limit_or_default(None), Ok(20));
        assert_eq!(list_limit_or_default(Some(0)), validate_list_limit(0));
    }

    #[test]
    fn claim_flow_partial_note_names_the_claimed_task_and_step() {
        // #5757-A Codex P1 oracle (PR #8493): a partial success
        // produces a note that preserves the claimed task's
        // identity and names the failed step; a full success (and
        // a plain /claim answer) produces no note.
        let partial = json!({
            "row": {"id": 4242},
            "claim_flow_partial": {"step": "bind", "exit": 4, "error": "PGRST002 boom"},
        });
        let note = claim_flow_partial_note(&partial).expect("partial yields a note");
        assert!(note.contains("4242"), "{note}");
        assert!(note.contains("bind"), "{note}");
        assert!(note.contains("do not re-claim"), "{note}");
        let full = json!({"row": {"id": 4242}, "lane_row": {}, "bind_row": {}});
        assert!(claim_flow_partial_note(&full).is_none());
        assert!(claim_flow_partial_note(&json!({"row": {"id": 1}})).is_none());
    }

    #[test]
    fn claim_output_preserves_claim_flow_partial_marker() {
        // #5757-A Codex P2 oracle (PR #8493): claim_output on a
        // partial /claim-flow answer keeps claim_flow_partial in
        // the printed JSON with the claimed task's identity and
        // exit 0; a full success passes the row through untouched.
        let partial = json!({
            "row": {"id": 4242, "status": "claimed"},
            "claim_flow_partial": {"step": "bind", "exit": 4, "error": "PGRST002 boom"},
        });
        let (out, code) = claim_output(&partial);
        assert_eq!(code, 0);
        assert_eq!(out["id"], 4242);
        assert_eq!(out["claim_flow_partial"]["step"], "bind");
        assert_eq!(out["claim_flow_partial"]["exit"], 4);
        let full =
            json!({"row": {"id": 4242, "status": "claimed"}, "lane_row": {}, "bind_row": {}});
        let (out, code) = claim_output(&full);
        assert_eq!(code, 0);
        assert!(out.get("claim_flow_partial").is_none());
    }

    #[test]
    fn no_claim_diagnostic_names_at_cap_reason_8154() {
        // #8154 oracle (follow-on #2937 acceptance 4): a null row
        // whose envelope carries the counts produces a diagnostic
        // naming the attempt cap; when every queued task is at cap
        // the line says so outright. No counts (older broker) -> no
        // diagnostic, and claim_output still carries the counts on
        // the printed {task: null} envelope with exit 5.
        let resp = json!({
            "row": Value::Null,
            "queued": 7,
            "at_cap_queued": 7,
            "queued_not_yet_due": 0,
            "parked_due": 2,
        });
        let line = no_claim_diagnostic(&resp).expect("counts yield a diagnostic");
        assert!(line.contains("7 queued"), "{line}");
        assert!(line.contains("7 at the attempt cap"), "{line}");
        assert!(
            line.contains("every queued task is at the attempt cap"),
            "{line}"
        );
        assert!(line.contains("2 parked due"), "{line}");
        let mixed = json!({
            "row": Value::Null,
            "queued": 9,
            "at_cap_queued": 3,
            "queued_not_yet_due": 4,
            "parked_due": 0,
        });
        let line = no_claim_diagnostic(&mixed).expect("counts yield a diagnostic");
        assert!(line.contains("3 at the attempt cap"), "{line}");
        assert!(line.contains("4 not yet due"), "{line}");
        assert!(!line.contains("every queued task"), "{line}");
        assert!(no_claim_diagnostic(&json!({"row": Value::Null})).is_none());
        let (out, code) = claim_output(&resp);
        assert_eq!(code, NO_CLAIM_EXIT);
        assert!(out["task"].is_null());
        assert_eq!(out["queued"], 7);
        assert_eq!(out["at_cap_queued"], 7);
        assert_eq!(out["queued_not_yet_due"], 0);
        assert_eq!(out["parked_due"], 2);
    }

    #[test]
    fn claim_bind_path_requires_explicit_bind_single_claim_and_session() {
        // #5757-A oracle: /claim-flow is chosen ONLY for an
        // explicit --bind with a session to bind, a single claim,
        // and no transfer; --session-id as a bare mark never
        // selects it, and every refused combination errors loudly
        // instead of silently claiming unbound.
        assert_eq!(claim_bind_path(false, 1, false, Some("ses_1")), Ok(false));
        assert_eq!(claim_bind_path(false, 1, false, None), Ok(false));
        assert_eq!(claim_bind_path(true, 1, false, Some("ses_1")), Ok(true));
        assert!(claim_bind_path(true, 1, false, None).is_err());
        assert!(claim_bind_path(true, 1, false, Some("")).is_err());
        assert!(claim_bind_path(true, 2, false, Some("ses_1")).is_err());
        assert!(claim_bind_path(true, 1, true, Some("ses_1")).is_err());
    }

    #[test]
    fn claim_flow_precheck_enforces_the_same_invariants() {
        // #5757-A oracle (broker side): the precheck returns the
        // session id for a well-formed flow request and refuses
        // a missing session, a batch, and a transfer — before
        // any step runs.
        assert_eq!(
            claim_flow_precheck(&json!({"session_id": "ses_1"})),
            Ok("ses_1".to_string())
        );
        assert!(claim_flow_precheck(&json!({})).is_err());
        assert!(claim_flow_precheck(&json!({"session_id": ""})).is_err());
        assert!(claim_flow_precheck(&json!({"session_id": "s", "count": 2})).is_err());
        assert!(claim_flow_precheck(&json!({"session_id": "s", "from_session": "ses_0"})).is_err());
        assert_eq!(
            claim_flow_precheck(&json!({"session_id": "s", "count": 1})),
            Ok("s".to_string())
        );
    }

    #[test]
    fn list_pages_cover_the_limit_exactly() {
        // #7871 oracle: pages are contiguous, ascending, and sum
        // to the requested limit; no page exceeds LIST_PAGE_SIZE
        // (the per-response row ceiling the bug tracks).
        assert_eq!(list_pages(1), vec![(0, 1)]);
        assert_eq!(list_pages(20), vec![(0, 20)]);
        assert_eq!(list_pages(LIST_PAGE_SIZE), vec![(0, LIST_PAGE_SIZE)]);
        assert_eq!(
            list_pages(LIST_PAGE_SIZE + 1),
            vec![(0, LIST_PAGE_SIZE), (LIST_PAGE_SIZE, 1)]
        );
        let pages = list_pages(LIST_LIMIT_MAX);
        assert_eq!(pages.len() as i64, LIST_LIMIT_MAX / LIST_PAGE_SIZE);
        assert_eq!(pages.iter().map(|(_, n)| n).sum::<i64>(), LIST_LIMIT_MAX);
        for (i, (offset, n)) in pages.iter().enumerate() {
            assert_eq!(*offset, i as i64 * LIST_PAGE_SIZE);
            assert!(*n >= 1 && *n <= LIST_PAGE_SIZE);
        }
        // Offsets clamp at 0, absent means the first page.
        assert_eq!(list_offset_or_default(None), 0);
        assert_eq!(list_offset_or_default(Some(-5)), 0);
        assert_eq!(list_offset_or_default(Some(200)), 200);
    }

    #[test]
    fn list_rows_rejects_a_reply_without_a_rows_array() {
        // #7871: the silent-empty path — a truncated broker reply
        // parses to Null, and an ok-envelope may still lack rows.
        // Both must be errors, never an empty list.
        assert!(list_rows(&Value::Null).is_err());
        assert!(list_rows(&json!({"ok": true})).is_err());
        assert!(list_rows(&json!({"ok": true, "rows": {}})).is_err());
        assert_eq!(list_rows(&json!({"ok": true, "rows": []})), Ok(vec![]));
        assert_eq!(
            list_rows(&json!({"ok": true, "rows": [{"id": 7}]})),
            Ok(vec![json!({"id": 7})])
        );
    }

    #[test]
    fn list_envelope_reports_total_and_truncation() {
        // #10376 oracle (contract #6622 rule 3): a listing whose
        // limit is below the matching count says so; an exhaustive
        // listing reports completeness; an unknown total is null,
        // never a fabricated zero.
        let rows = json!([{"id": 1}, {"id": 2}]);
        let limited = list_envelope(rows.clone(), Some(5), 0);
        assert_eq!(limited["total"], json!(5));
        assert_eq!(limited["truncated"], json!(true));
        assert_eq!(list_rows(&limited).map(|r| r.len()), Ok(2));
        let exhaustive = list_envelope(rows.clone(), Some(2), 0);
        assert_eq!(exhaustive["truncated"], json!(false));
        let windowed = list_envelope(rows.clone(), Some(5), 3);
        assert_eq!(windowed["truncated"], json!(false));
        let unknown = list_envelope(rows, None, 0);
        assert_eq!(unknown["total"], Value::Null);
        assert_eq!(unknown["truncated"], json!(false));
        assert_eq!(list_total(&limited), Some(5));
        assert_eq!(list_total(&json!({"rows": []})), None);
    }

    #[test]
    fn exact_status_found_on_the_exact_match() {
        // #8581 pilot oracle: the authoritative reply carrying
        // the exact task is `found`; the row rides attributed to
        // the cloud source with the row's own timestamp, and the
        // mode/route are the contract's served values.
        let resp = Ok(json!({
            "ok": true, "total": 1, "truncated": false,
            "rows": [{"id": 7, "queue": "repo-health", "status": "claimed",
                      "updated_at": "2026-11-23T12:00:00Z"}],
        }));
        let (rec, code) = exact_task_status("repo-health", 7, resp, "2026-11-23T12:05:00Z");
        assert_eq!(code, 0);
        assert_eq!(rec["verb"], json!("list"));
        assert_eq!(rec["mode"], json!("normal"));
        assert_eq!(rec["route"], json!("cloud"));
        assert_eq!(rec["result"], json!("found"));
        assert_eq!(rec["source"], json!("cloud"));
        assert_eq!(rec["as_of"], json!("2026-11-23T12:05:00Z"));
        assert_eq!(rec["task"]["status"], json!("claimed"));
        assert_eq!(rec["task_as_of"], json!("2026-11-23T12:00:00Z"));
    }

    #[test]
    fn exact_status_not_found_only_on_a_complete_authoritative_zero() {
        // #8581 pilot oracle: `not_found` is reserved for the
        // certified zero — ok reply, empty rows, total reported
        // as 0, no truncation.
        let resp = Ok(json!({"ok": true, "total": 0, "truncated": false, "rows": []}));
        let (rec, code) = exact_task_status("repo-health", 7, resp, "2026-11-23T12:05:00Z");
        assert_eq!(code, 1);
        assert_eq!(rec["result"], json!("not_found"));
        assert_eq!(rec["mode"], json!("normal"));
        assert_eq!(rec["route"], json!("cloud"));
        assert!(rec.get("task").is_none());
    }

    #[test]
    fn exact_status_zero_without_a_total_is_partial() {
        // A zero the broker never certified (pre-#10376 reply, no
        // total) cannot be told apart from an unread window:
        // partial, never absence.
        let resp = Ok(json!({"ok": true, "rows": []}));
        let (rec, code) = exact_task_status("repo-health", 7, resp, "2026-11-23T12:05:00Z");
        assert_eq!(code, 3);
        assert_eq!(rec["result"], json!("partial"));
        assert_eq!(rec["detail"], json!("total_unreported"));
    }

    #[test]
    fn exact_status_truncated_window_is_partial_never_absence() {
        // The matching set is larger than the window — the task
        // may sit beyond it, flagged or merely implied by total.
        let flagged = Ok(json!({"ok": true, "total": 5, "truncated": true, "rows": []}));
        let (rec, code) = exact_task_status("repo-health", 7, flagged, "2026-11-23T12:05:00Z");
        assert_eq!(code, 3);
        assert_eq!(rec["result"], json!("partial"));
        assert_eq!(rec["detail"], json!("truncated_reply"));
        let implied = Ok(json!({
            "ok": true, "total": 9, "truncated": false,
            "rows": [{"id": 8, "queue": "repo-health"}],
        }));
        let (rec, code) = exact_task_status("repo-health", 7, implied, "2026-11-23T12:05:00Z");
        assert_eq!(code, 3);
        assert_eq!(rec["result"], json!("partial"));
        assert_eq!(rec["detail"], json!("truncated_reply"));
    }

    #[test]
    fn exact_status_complete_reply_naming_other_tasks_is_partial() {
        // A complete, certified reply that names a different task
        // means the broker did not scope the read; its silence
        // about our task certifies nothing.
        let resp = Ok(json!({
            "ok": true, "total": 1, "truncated": false,
            "rows": [{"id": 8, "queue": "repo-health"}],
        }));
        let (rec, code) = exact_task_status("repo-health", 7, resp, "2026-11-23T12:05:00Z");
        assert_eq!(code, 3);
        assert_eq!(rec["result"], json!("partial"));
        assert_eq!(rec["detail"], json!("unscoped_reply"));
    }

    #[test]
    fn exact_status_row_for_a_different_queue_is_unknown() {
        // Identity mismatch: the matching id arrived wearing a
        // different queue than the one asked about — the answer
        // is not about the task this seat scoped, so the state
        // is unknown, not found.
        let resp = Ok(json!({
            "ok": true, "total": 1, "truncated": false,
            "rows": [{"id": 7, "queue": "someone-elses", "status": "claimed"}],
        }));
        let (rec, code) = exact_task_status("repo-health", 7, resp, "2026-11-23T12:05:00Z");
        assert_eq!(code, 3);
        assert_eq!(rec["result"], json!("unknown"));
        assert_eq!(rec["detail"], json!("identity_mismatch"));
        assert!(rec.get("task").is_none());
    }

    #[test]
    fn exact_status_malformed_reply_is_unknown() {
        // No rows array, or no ok flag at all: nothing readable
        // was learned, and neither shape may read as absence.
        let no_rows = Ok(json!({"ok": true, "total": 0}));
        let (rec, code) = exact_task_status("repo-health", 7, no_rows, "2026-11-23T12:05:00Z");
        assert_eq!(code, 3);
        assert_eq!(rec["result"], json!("unknown"));
        assert_eq!(rec["detail"], json!("malformed_reply"));
        let no_ok = Ok(json!({"rows": [], "total": 0, "truncated": false}));
        let (rec, code) = exact_task_status("repo-health", 7, no_ok, "2026-11-23T12:05:00Z");
        assert_eq!(code, 3);
        assert_eq!(rec["result"], json!("unknown"));
        assert_eq!(rec["detail"], json!("malformed_reply"));
    }

    #[test]
    fn exact_status_transport_failure_is_unknown_unavailable() {
        // No envelope at all: the record names the attempted
        // route family as unavailable and claims no fallback —
        // this slice attempts none.
        let resp = Err((3, "queue-broker socket missing".to_string()));
        let (rec, code) = exact_task_status("repo-health", 7, resp, "2026-11-23T12:05:00Z");
        assert_eq!(code, 3);
        assert_eq!(rec["result"], json!("unknown"));
        assert_eq!(rec["detail"], json!("unavailable"));
        assert_eq!(rec["mode"], json!("db-degraded"));
        assert_eq!(rec["route"], json!("unavailable"));
        assert!(rec.get("source").is_none());
    }

    #[test]
    fn exact_status_denial_is_unknown_denied_and_never_absence() {
        // Denied at the transport leg and denied by envelope are
        // the same outcome class, distinct from unavailable —
        // and a denial is never retried under another identity.
        let at_transport = Err((3, "session proxy: unauthorized token".to_string()));
        let (rec, code) = exact_task_status("repo-health", 7, at_transport, "2026-11-23T12:05:00Z");
        assert_eq!(code, 3);
        assert_eq!(rec["result"], json!("unknown"));
        assert_eq!(rec["detail"], json!("denied"));
        let by_envelope =
            Ok(json!({"ok": false, "error": "permission denied for this queue", "exit": 4}));
        let (rec, code) = exact_task_status("repo-health", 7, by_envelope, "2026-11-23T12:05:00Z");
        assert_eq!(code, 4);
        assert_eq!(rec["result"], json!("unknown"));
        assert_eq!(rec["detail"], json!("denied"));
    }

    #[test]
    fn exact_status_broker_error_envelope_is_unknown_with_its_exit() {
        // {ok:false} without its own exit defaults to 4, the
        // unwrap parity of the text verbs; an envelope exit is
        // carried through untouched.
        let plain = Ok(json!({"ok": false, "error": "store exploded"}));
        let (rec, code) = exact_task_status("repo-health", 7, plain, "2026-11-23T12:05:00Z");
        assert_eq!(code, 4);
        assert_eq!(rec["result"], json!("unknown"));
        assert_eq!(rec["detail"], json!("broker_error"));
        assert_eq!(rec["error"], json!("store exploded"));
        let exited = Ok(json!({"ok": false, "error": "store exploded", "exit": 9}));
        let (rec, code) = exact_task_status("repo-health", 7, exited, "2026-11-23T12:05:00Z");
        assert_eq!(code, 9);
        assert_eq!(rec["detail"], json!("broker_error"));
    }

    #[test]
    fn duration_flags_range_is_validated() {
        // #6420: 0/negative durations corrupt claim state (a lease
        // born expired, a heartbeat that lapses its own claim, a park
        // whose not_before lands in the past) and unbounded values
        // overflow the SQL integer RPC parameters. Out-of-range is a
        // usage error naming the flag (design #6448 Phase 3: reject,
        // not clamp). Park delay 0 is rejected by explicit decision:
        // a zero-wait park is a requeue, which has its own verb.
        for bad in [0, -1, LEASE_SECONDS_MAX + 1, i64::MAX] {
            let err = validate_lease_seconds(bad)
                .err()
                .expect("out-of-range --lease-seconds must be rejected");
            assert!(
                err.contains("--lease-seconds"),
                "error names the flag: {err}"
            );
        }
        for bad in [0, -1, EXTEND_SECONDS_MAX + 1, i64::MAX] {
            let err = validate_extend_seconds(bad)
                .err()
                .expect("out-of-range --extend-seconds must be rejected");
            assert!(
                err.contains("--extend-seconds"),
                "error names the flag: {err}"
            );
        }
        for bad in [0, -1, DELAY_SECONDS_MAX + 1, i64::MAX] {
            let err = validate_delay_seconds(bad)
                .err()
                .expect("out-of-range --delay-seconds must be rejected");
            assert!(
                err.contains("--delay-seconds"),
                "error names the flag: {err}"
            );
        }
        // Boundaries and ordinary values pass through unchanged.
        assert_eq!(validate_lease_seconds(1), Ok(1));
        assert_eq!(
            validate_lease_seconds(LEASE_SECONDS_MAX),
            Ok(LEASE_SECONDS_MAX)
        );
        assert_eq!(validate_extend_seconds(1), Ok(1));
        assert_eq!(
            validate_extend_seconds(EXTEND_SECONDS_MAX),
            Ok(EXTEND_SECONDS_MAX)
        );
        assert_eq!(validate_delay_seconds(1), Ok(1));
        assert_eq!(
            validate_delay_seconds(DELAY_SECONDS_MAX),
            Ok(DELAY_SECONDS_MAX)
        );
        // Absent flags keep the existing defaults (claim 2700,
        // heartbeat 900).
        assert_eq!(lease_seconds_or_default(None), Ok(2700));
        assert_eq!(extend_seconds_or_default(None), Ok(900));
        assert_eq!(lease_seconds_or_default(Some(0)), validate_lease_seconds(0));
        assert_eq!(
            extend_seconds_or_default(Some(-1)),
            validate_extend_seconds(-1)
        );
    }

    #[test]
    fn not_before_flag_is_validated() {
        // #6426: park/revive --not-before went into the broker body
        // as a raw string — garbage failed only as a downstream DB
        // cast error, and a far-future value parked a task out of
        // reach for years. It is now a usage error naming the flag
        // (cmd_park/cmd_revive die with exit 2 on Err).
        for bad in [
            "banana",
            "",
            "   ",
            "2026-02-31T00:00:00Z",
            "9999-01-01T00:00:00Z",
        ] {
            let err = validate_not_before_flag(bad)
                .err()
                .expect("invalid --not-before must be rejected");
            assert!(err.contains("--not-before"), "error names the flag: {err}");
        }
        // Valid timestamps pass through unchanged — the broker body
        // for a valid value is byte-for-byte what it was before.
        for ok in [
            "2020-01-01T00:00:00Z",
            "2026-11-06T00:00:00Z",
            "2026-11-06T05:30:00+05:30",
        ] {
            assert_eq!(validate_not_before_flag(ok), Ok(ok.to_string()));
        }
    }

    #[test]
    fn list_not_before_is_validated_before_the_call_9027() {
        // #9027: cmd_list forwarded --not-before raw; on the Turso
        // backend the TEXT comparison then silently narrowed the
        // listing (a value sorting before every timestamp hid every
        // row carrying a not_before; one sorting after disabled the
        // filter) with exit 0. cmd_list now validates the flag with
        // the park/revive rule and the /list body carries the
        // validated value.
        let src = [
            include_str!("client/transport.rs"),
            include_str!("client/support.rs"),
            include_str!("client/commands_query.rs"),
            include_str!("client/commands_lifecycle.rs"),
            include_str!("client/fallback_stack.rs"),
            include_str!("client.rs"),
        ]
        .concat();
        let seg = src
            .split("fn cmd_list(")
            .nth(1)
            .and_then(|s| s.split("fn cmd_").next())
            .unwrap_or("");
        assert!(
            seg.contains("validate_not_before_flag(&nb)"),
            "cmd_list validates --not-before before the /list call"
        );
        assert!(
            seg.contains("\"not_before\": not_before"),
            "the /list body carries the validated value, not the raw flag"
        );
        assert!(
            !seg.contains("\"not_before\": args.get(\"not-before\")"),
            "the raw flag must not reach the /list body"
        );
        // The exact garbage shapes from the finding die at the
        // validator the client now applies:
        for bad in ["0", "!", "banana"] {
            assert!(
                validate_not_before_flag(bad).is_err(),
                "{bad} must be rejected"
            );
        }
    }

    #[test]
    fn well_formed_invocation_is_unchanged() {
        // Control (#5939 acceptance): list --queue alpha records
        // queue=alpha and the absent --limit defaults at the call site.
        let args = parse_args_inner(&pos(&["--queue", "alpha"]), &["queue", "limit"], &[])
            .ok()
            .expect("well-formed invocation parses");
        assert_eq!(args.get("queue"), Some("alpha".to_string()));
        assert_eq!(args.get_i64("limit").unwrap_or(20), 20);
    }

    #[test]
    fn health_accepts_worker_flag() {
        // #7349 regression, oracle: the health verb's CLI flag set
        // accepts the same `--worker` the mutating verbs accept. The
        // value is attribution only — it never reaches the request
        // body — but a packet-literal claimant fire
        // (`health ... --worker opencode-claimant`) must not abort on
        // its first call with "unrecognized argument".
        let args = parse_args_inner(
            &pos(&[
                "--queue",
                "repo-health-code-pilot",
                "--worker",
                "opencode-claimant",
            ]),
            HEALTH_FLAGS,
            &[],
        )
        .ok()
        .expect("health must accept --worker");
        assert_eq!(
            args.get("queue"),
            Some("repo-health-code-pilot".to_string())
        );
        assert_eq!(args.get("worker"), Some("opencode-claimant".to_string()));

        // Acceptance: health without --worker is unchanged.
        let args = parse_args_inner(&pos(&["--queue", "q"]), HEALTH_FLAGS, &[])
            .ok()
            .expect("health without --worker parses");
        assert_eq!(args.get("queue"), Some("q".to_string()));
        assert_eq!(args.get("worker"), None);

        // The acceptance is narrow: unrelated flags stay rejected.
        assert!(parse_args_inner(&pos(&["--bogus", "x"]), HEALTH_FLAGS, &[]).is_err());
    }

    /// One spelling of a value flag: `--flag value` or `--flag=value`.
    fn flag_tokens(flag: &str, value: &str, inline: bool) -> Vec<String> {
        if inline {
            vec![format!("--{flag}={value}")]
        } else {
            vec![format!("--{flag}"), value.to_string()]
        }
    }

    // Property-based coverage of the #5939 parser invariants (the
    // oracle is the invariant itself; the fixed examples above stay
    // as smoke coverage). Per the repo testing doctrine, parsers get
    // PBT: a generator hammers spellings and positions no example
    // list would enumerate.

    proptest! {
        #[test]
        fn duplicate_flags_rejected_in_any_spelling_and_position(
            flag in prop_oneof![Just("queue"), Just("worker")],
            inline1 in any::<bool>(),
            inline2 in any::<bool>(),
            v1 in "[a-z]{1,4}",
            v2 in "[a-z]{1,4}",
            lead in proptest::collection::vec("[a-z]{1,3}", 0..3),
            mid in proptest::collection::vec("[a-z]{1,3}", 0..3),
        ) {
            let mut rest: Vec<String> = lead;
            rest.extend(flag_tokens(flag, &v1, inline1));
            rest.extend(mid);
            rest.extend(flag_tokens(flag, &v2, inline2));
            let err = parse_args_inner(&rest, &["queue", "worker"], &[])
                .err()
                .expect("a repeated flag must be rejected wherever it appears");
            prop_assert!(
                err.contains(&format!("--{flag}")),
                "error names the flag: {}",
                err
            );
        }

        #[test]
        fn bool_with_any_inline_value_rejected(
            v in prop_oneof![
                Just("true".to_string()),
                Just("false".to_string()),
                "[a-z]{0,5}",
                "[0-9]{1,3}",
            ],
        ) {
            let err = parse_args_inner(&pos(&[&format!("--dry-run={v}")]), &[], &["dry-run"])
                .err()
                .expect("a boolean with an inline value must be rejected");
            prop_assert!(err.contains("--dry-run"), "error names the flag: {}", err);
        }

        #[test]
        fn bare_bool_accepted_in_any_position(
            lead in proptest::collection::vec("[a-z]{1,3}", 0..3),
            tail in proptest::collection::vec("[a-z]{1,3}", 0..3),
        ) {
            let mut rest: Vec<String> = lead;
            rest.push("--dry-run".to_string());
            rest.extend(tail);
            let args = parse_args_inner(&rest, &["queue"], &["dry-run"])
                .ok()
                .expect("a bare boolean parses wherever it appears");
            prop_assert!(args.has("dry-run"));
        }

        #[test]
        fn numeric_flag_roundtrips_every_i64(n in any::<i64>()) {
            prop_assert_eq!(parse_i64_flag("limit", &n.to_string()), Ok(n));
        }

        #[test]
        fn non_numeric_flag_value_rejected_for_any_spelling(
            s in "[a-z][a-z0-9]{0,6}",
        ) {
            // A leading letter can never spell an i64, so the oracle
            // is decidable for every generated string.
            let err = parse_i64_flag("limit", &s)
                .err()
                .expect("a non-numeric value must be rejected");
            prop_assert!(err.contains("--limit"), "error names the flag: {}", err);
            prop_assert!(err.contains(&s), "error names the value: {}", err);
        }

        #[test]
        fn distinct_flags_all_preserved(
            use_queue in any::<bool>(),
            use_worker in any::<bool>(),
            use_lane in any::<bool>(),
            inline in any::<bool>(),
            vq in "[a-z]{1,4}",
            vw in "[a-z]{1,4}",
            vl in "[a-z]{1,4}",
        ) {
            let mut rest: Vec<String> = Vec::new();
            let mut expected: Vec<(&str, &str)> = Vec::new();
            if use_queue {
                rest.extend(flag_tokens("queue", &vq, inline));
                expected.push(("queue", &vq));
            }
            if use_worker {
                rest.extend(flag_tokens("worker", &vw, inline));
                expected.push(("worker", &vw));
            }
            if use_lane {
                rest.extend(flag_tokens("lane", &vl, inline));
                expected.push(("lane", &vl));
            }
            let args = parse_args_inner(&rest, &["queue", "worker", "lane"], &[])
                .ok()
                .expect("distinct flags are never rejected");
            for (name, value) in expected {
                prop_assert_eq!(args.get(name), Some(value.to_string()));
            }
        }
    }

    // #5486 regression coverage: the broker envelope -> CLI output
    // contract for claim, plus claimed-list recovery. Oracles are the
    // envelope/row shapes the broker actually returns (queue_broker
    // wraps RPC results as {ok:true, row:...} / {ok:true, rows:[...]}).

    fn claimed_row_fixture() -> Value {
        json!({
            "id": 10811,
            "queue": "repo-health-code-pilot",
            "status": "claimed",
            "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:5486",
            "title": "Add repo-queue claim response-shape regression tests (#5486)",
            "priority": 50,
            "worker_id": "repo-worker-code",
        })
    }

    #[test]
    fn claim_success_envelope_prints_the_real_row() {
        // The reported #5486 failure printed an all-null task row for
        // a claim the broker had actually made. The row must come
        // through verbatim: real id, queue, status, and dedupe_key.
        let row = claimed_row_fixture();
        let resp = json!({"ok": true, "row": row.clone()});
        let (out, code) = claim_output(&resp);
        assert_eq!(code, 0);
        assert_eq!(out, row);
        assert_eq!(out.get("id"), Some(&json!(10811)));
        assert_eq!(out.get("queue"), Some(&json!("repo-health-code-pilot")));
        assert_eq!(out.get("status"), Some(&json!("claimed")));
        assert_eq!(
            out.get("dedupe_key"),
            Some(&json!(
                "repo-health:v1:fix:developing-today/code:issue:5486"
            ))
        );
        for field in ["id", "queue", "status", "dedupe_key"] {
            assert!(
                !out.get(field).expect(field).is_null(),
                "claimed row field {field} must not be null"
            );
        }
    }

    #[test]
    fn claim_null_row_is_a_distinguishable_nonzero_no_claim() {
        // Both a null row and a missing row field are "no task
        // claimable": nonzero exit, and the marker carries none of
        // the fields that would make it look like a task row.
        for resp in [json!({"ok": true, "row": Value::Null}), json!({"ok": true})] {
            let (out, code) = claim_output(&resp);
            assert_eq!(code, NO_CLAIM_EXIT);
            assert_ne!(code, 0, "an empty claim must not exit as success");
            assert_eq!(out, json!({"task": Value::Null}));
            for field in ["id", "queue", "status", "dedupe_key"] {
                assert!(
                    out.get(field).is_none(),
                    "no-claim marker must not carry task field {field}"
                );
            }
        }
    }

    #[test]
    fn claim_no_claim_carries_budget_skip_summary_when_present() {
        // #3418 (#4712): a broker no-claim response carrying the
        // budget-skip summary keeps the distinguishable shape and
        // exit code, and surfaces the summary for impossible-fit
        // escalation. A response without the keys stays byte-exact.
        let (out, code) = claim_output(&json!({"ok": true, "row": Value::Null,
            "budget_skipped": 3, "max_required_budget_minutes": 120}));
        assert_eq!(code, NO_CLAIM_EXIT);
        assert_eq!(
            out,
            json!({"task": Value::Null, "budget_skipped": 3,
                "max_required_budget_minutes": 120})
        );
        for field in ["id", "queue", "status", "dedupe_key"] {
            assert!(
                out.get(field).is_none(),
                "no-claim marker must not carry task field {field}"
            );
        }
    }

    #[test]
    fn claim_payload_less_row_is_a_no_claim_not_a_task() {
        // #5444 defense in depth: a claimed row that CARRIES a
        // payload field which is null or an empty object is the
        // 2026-11-04 payload-less incident shape — it must degrade
        // to the distinguishable no-claim marker with a nonzero
        // exit, never print as a task a worker would try to route.
        for payload in [Value::Null, json!({})] {
            let mut row = claimed_row_fixture();
            row["payload"] = payload;
            let (out, code) = claim_output(&json!({"ok": true, "row": row}));
            assert_eq!(code, NO_CLAIM_EXIT);
            assert_eq!(out, json!({"task": Value::Null}));
        }
        // A row carrying a real payload object passes through.
        let mut row = claimed_row_fixture();
        row["payload"] = json!({"schema": "repo-task/v1", "kind": "small_fix"});
        let (out, code) = claim_output(&json!({"ok": true, "row": row.clone()}));
        assert_eq!(code, 0);
        assert_eq!(out, row);
    }

    #[test]
    fn claimed_list_recovers_the_claimed_task() {
        // After a successful claim, `list --status claimed` returns
        // rows the worker can recover the same task from, with its
        // identifying fields intact.
        let row = claimed_row_fixture();
        let other = json!({
            "id": 9999,
            "queue": "repo-health-code-pilot",
            "status": "claimed",
            "dedupe_key": "repo-health:v1:triage:developing-today/code:issue:1",
            "title": "some other claimed task",
            "priority": 30,
        });
        let rows = vec![other, row];
        let found = find_task_row(&rows, 10811).expect("claimed task must be recoverable by id");
        assert_eq!(found.get("status"), Some(&json!("claimed")));
        assert_eq!(
            found.get("dedupe_key"),
            Some(&json!(
                "repo-health:v1:fix:developing-today/code:issue:5486"
            ))
        );
        assert!(find_task_row(&rows, 424242).is_none());
        // The printed cells carry the same identifying fields.
        let cells = list_row_cells(found);
        assert_eq!(cells.len(), 6);
        assert_eq!(cells[0], "task 10811");
        assert_eq!(cells[1], "50");
        assert_eq!(cells[2], "claimed");
        assert_eq!(
            cells[4],
            "repo-health:v1:fix:developing-today/code:issue:5486"
        );
        assert_eq!(
            cells[5],
            "Add repo-queue claim response-shape regression tests (#5486)"
        );
    }

    proptest! {
        #[test]
        fn claim_output_passes_any_non_null_row_through_verbatim(
            id in 1i64..1_000_000,
            priority in 0i64..100,
            title in "[a-z ]{1,40}",
        ) {
            // Invariant (the #5486 oracle): whatever identifying
            // fields a successful claim row carries reach stdout
            // unchanged, with a success exit — no field is nulled,
            // dropped, or re-keyed in decoding.
            let row = json!({
                "id": id,
                "queue": "repo-health-code-pilot",
                "status": "claimed",
                "dedupe_key": format!("key-{id}"),
                "title": title,
                "priority": priority,
            });
            let (out, code) = claim_output(&json!({"ok": true, "row": row.clone()}));
            prop_assert_eq!(code, 0);
            prop_assert_eq!(out, row);
        }
    }

    #[test]
    fn broker_program_prefers_a_sibling_fleet_binary() {
        // #5758 acceptance: while a fleet binary exists beside the
        // running one, auto-start spawns `fleet proxy serve` —
        // never the Python broker.
        let dir =
            std::env::temp_dir().join(format!("fleet-queue-broker-program-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fleet"), b"#!/bin/sh\n").unwrap();
        let (program, args) = broker_program_in(&dir);
        assert_eq!(program, dir.join("fleet").to_string_lossy());
        assert_eq!(args, vec!["proxy".to_string(), "serve".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broker_program_falls_back_to_python_without_a_sibling() {
        // The standalone rollback builds (no fleet beside them)
        // keep the Python spawn.
        let dir = std::env::temp_dir().join(format!(
            "fleet-queue-broker-program-none-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (program, args) = broker_program_in(&dir);
        assert_eq!(program, "python3");
        assert_eq!(args.len(), 1);
        assert!(
            args[0].ends_with("/tools/laptop-agent-broker.py"),
            "{args:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- #8361: GhCreator marker-search seam tests ----
    // Oracle: the production drainer's reconciliation leg, driven
    // through a stub `gh` binary that logs its argv and plays back
    // canned responses. The argv log is the evidence for the GET
    // shape; the stub's exit/stderr shapes drive the three-way
    // lookup (found / checked-absent / could-not-check).

    // The stub-gh tests below spawn real child processes in
    // parallel by default; on loaded machines one spawn in the
    // batch intermittently fails (CouldNotCheck("spawn_failed"))
    // even though the stub is intact and executable — observed
    // 2026-11-12 on the fleet laptop (threads=1: 6/6 pass;
    // threads=7: ~50% of runs lose exactly one spawn; strace,
    // which serializes, passes 12/12). Serialize just these
    // process-spawning tests; every assertion is unchanged.
    static GH_STUB_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn stub_gh_8361(tag: &str, script: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "fleet-queue-ghcreator-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("argv.log");
        let bin = dir.join("gh");
        let full = format!("#!/bin/sh\necho \"$@\" >> '{}'\n{}", log.display(), script);
        std::fs::write(&bin, full).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        (bin, log)
    }

    fn outbox_entry_8361() -> Value {
        json!({
            "id": "OB-test-1",
            "repo": "developing-today/code",
            "kind": "issue",
            "payload": {"title": "t", "body": "b"},
        })
    }

    #[test]
    fn marker_search_is_a_get_and_finds_the_landed_marker() {
        let _gh_stub_guard = GH_STUB_LOCK.lock().unwrap();
        let (bin, log) = stub_gh_8361(
            "found",
            r#"case "$*" in
  *search/issues*) printf '%s' '{"total_count":1,"items":[{"number":42,"html_url":"https://github.com/developing-today/code/issues/42"}]}' ;;
  *view*) printf '%s' '{"number":42,"url":"https://github.com/developing-today/code/issues/42","body":"landed <!-- fleet-outbox:OB-test-1 -->"}' ;;
  *) echo "unexpected argv: $*" >&2; exit 3 ;;
esac
"#,
        );
        let mut creator = GhCreator::with_runner(GhRunner::with_bin(bin.to_string_lossy()));
        let found = creator.find_by_marker(&outbox_entry_8361());
        assert_eq!(
            found,
            MarkerLookup::Found(
                42,
                "https://github.com/developing-today/code/issues/42".to_string()
            )
        );
        let argv_log = std::fs::read_to_string(&log).unwrap();
        let search_line = argv_log
            .lines()
            .find(|l| l.contains("search/issues"))
            .expect("the stub must have seen the search call");
        assert!(
            search_line.contains("--method GET"),
            "marker search must be a GET, argv was: {search_line}"
        );
        let _ = std::fs::remove_dir_all(bin.parent().unwrap());
    }

    // ---- #8363: comment-kind marker resolution ----
    // Oracle: for issue_comment / pr_comment entries the search hit's
    // number is the ISSUE number, a different id namespace from
    // comment ids. Reconcile must resolve the marker through the
    // issue's comment list and record the COMMENT's id and html_url;
    // the old code read the issue number back as a comment id
    // (repos/{repo}/issues/comments/{issue_number}), which can never
    // match a landed comment, so landed comments were re-created.

    fn outbox_comment_entry_8363(kind: &str) -> Value {
        json!({
            "id": "OB-test-1",
            "repo": "developing-today/code",
            "kind": kind,
            "payload": {"target_number": 7, "body": "b"},
        })
    }

    #[test]
    fn comment_marker_resolves_through_the_issues_comment_list() {
        let _gh_stub_guard = GH_STUB_LOCK.lock().unwrap();
        let (bin, log) = stub_gh_8361(
            "comment8363",
            r#"case "$*" in
  *search/issues*) printf '%s' '{"total_count":1,"items":[{},{"number":7,"html_url":"https://github.com/developing-today/code/issues/7"}]}' ;;
  *issues/7/comments*) printf '%s' '[{"id":111,"html_url":"https://github.com/developing-today/code/issues/7#issuecomment-111","body":"an earlier comment"},{"id":6781256653,"html_url":"https://github.com/developing-today/code/issues/7#issuecomment-6781256653","body":"landed <!-- fleet-outbox:OB-test-1 -->"}]' ;;
  *) echo "unexpected argv: $*" >&2; exit 3 ;;
esac
"#,
        );
        let mut creator = GhCreator::with_runner(GhRunner::with_bin(bin.to_string_lossy()));
        // The first search item is malformed (no number): skipped,
        // not fatal (#8363). The landed comment is the second one on
        // issue 7; reconcile must find THAT comment's id and URL.
        let found = creator.find_by_marker(&outbox_comment_entry_8363("issue_comment"));
        assert_eq!(
            found,
            MarkerLookup::Found(
                6781256653,
                "https://github.com/developing-today/code/issues/7#issuecomment-6781256653"
                    .to_string()
            )
        );
        let argv_log = std::fs::read_to_string(&log).unwrap();
        assert!(
            argv_log.contains("issues/7/comments"),
            "reconcile must list the candidate issue's comments, argv was: {argv_log}"
        );
        assert!(
            !argv_log.contains("issues/comments/7"),
            "the issue number must not be read back as a comment id, argv was: {argv_log}"
        );
        let _ = std::fs::remove_dir_all(bin.parent().unwrap());
    }

    #[test]
    fn pr_comment_marker_resolves_the_same_way() {
        let _gh_stub_guard = GH_STUB_LOCK.lock().unwrap();
        let (bin, _log) = stub_gh_8361(
            "prcomment8363",
            r#"case "$*" in
  *search/issues*) printf '%s' '{"total_count":1,"items":[{"number":7,"html_url":"https://github.com/developing-today/code/pull/7"}]}' ;;
  *issues/7/comments*) printf '%s' '[{"id":555,"html_url":"https://github.com/developing-today/code/pull/7#issuecomment-555","body":"landed <!-- fleet-outbox:OB-test-1 -->"}]' ;;
  *) echo "unexpected argv: $*" >&2; exit 3 ;;
esac
"#,
        );
        let mut creator = GhCreator::with_runner(GhRunner::with_bin(bin.to_string_lossy()));
        assert_eq!(
            creator.find_by_marker(&outbox_comment_entry_8363("pr_comment")),
            MarkerLookup::Found(
                555,
                "https://github.com/developing-today/code/pull/7#issuecomment-555".to_string()
            )
        );
        let _ = std::fs::remove_dir_all(bin.parent().unwrap());
    }

    #[test]
    fn comment_list_failure_is_could_not_check_not_absent() {
        let _gh_stub_guard = GH_STUB_LOCK.lock().unwrap();
        let (bin, _log) = stub_gh_8361(
            "commentfail8363",
            r#"case "$*" in
  *search/issues*) printf '%s' '{"total_count":1,"items":[{"number":7,"html_url":"https://github.com/developing-today/code/issues/7"}]}' ;;
  *issues/7/comments*) echo 'gh: Forbidden (HTTP 403)' >&2; exit 1 ;;
  *) echo "unexpected argv: $*" >&2; exit 3 ;;
esac
"#,
        );
        let mut creator = GhCreator::with_runner(GhRunner::with_bin(bin.to_string_lossy()));
        assert_eq!(
            creator.find_by_marker(&outbox_comment_entry_8363("issue_comment")),
            MarkerLookup::CouldNotCheck("http_403".to_string())
        );
        let _ = std::fs::remove_dir_all(bin.parent().unwrap());
    }

    #[test]
    fn marker_search_404_is_could_not_check_not_absent() {
        let _gh_stub_guard = GH_STUB_LOCK.lock().unwrap();
        let (bin, _log) = stub_gh_8361("e404", "echo 'gh: Not Found (HTTP 404)' >&2\nexit 1\n");
        let mut creator = GhCreator::with_runner(GhRunner::with_bin(bin.to_string_lossy()));
        assert_eq!(
            creator.find_by_marker(&outbox_entry_8361()),
            MarkerLookup::CouldNotCheck("http_404".to_string())
        );
        let _ = std::fs::remove_dir_all(bin.parent().unwrap());
    }

    #[test]
    fn marker_search_403_is_could_not_check_not_absent() {
        let _gh_stub_guard = GH_STUB_LOCK.lock().unwrap();
        let (bin, _log) = stub_gh_8361("e403", "echo 'gh: Forbidden (HTTP 403)' >&2\nexit 1\n");
        let mut creator = GhCreator::with_runner(GhRunner::with_bin(bin.to_string_lossy()));
        assert_eq!(
            creator.find_by_marker(&outbox_entry_8361()),
            MarkerLookup::CouldNotCheck("http_403".to_string())
        );
        let _ = std::fs::remove_dir_all(bin.parent().unwrap());
    }

    #[test]
    fn marker_search_empty_result_is_checked_absent() {
        let _gh_stub_guard = GH_STUB_LOCK.lock().unwrap();
        let (bin, _log) = stub_gh_8361(
            "absent",
            r#"case "$*" in
  *search/issues*) printf '%s' '{"total_count":0,"items":[]}' ;;
  *issues?state=all*) printf '%s' '[]' ;;
  *) echo "unexpected argv: $*" >&2; exit 3 ;;
esac
"#,
        );
        let mut creator = GhCreator::with_runner(GhRunner::with_bin(bin.to_string_lossy()));
        assert_eq!(
            creator.find_by_marker(&outbox_entry_8361()),
            MarkerLookup::Absent
        );
        let _ = std::fs::remove_dir_all(bin.parent().unwrap());
    }

    #[test]
    fn marker_search_unparseable_is_could_not_check() {
        let _gh_stub_guard = GH_STUB_LOCK.lock().unwrap();
        let (bin, _log) = stub_gh_8361("garbage", "printf '%s' 'not json'\n");
        let mut creator = GhCreator::with_runner(GhRunner::with_bin(bin.to_string_lossy()));
        assert_eq!(
            creator.find_by_marker(&outbox_entry_8361()),
            MarkerLookup::CouldNotCheck("unparseable_json".to_string())
        );
        let _ = std::fs::remove_dir_all(bin.parent().unwrap());
    }

    #[test]
    fn marker_search_failed_candidate_readback_is_could_not_check() {
        let _gh_stub_guard = GH_STUB_LOCK.lock().unwrap();
        let (bin, _log) = stub_gh_8361(
            "readback404",
            r#"case "$*" in
  *search/issues*) printf '%s' '{"total_count":1,"items":[{"number":42,"html_url":"https://github.com/developing-today/code/issues/42"}]}' ;;
  *view*) echo 'gh: Not Found (HTTP 404)' >&2; exit 1 ;;
  *) echo "unexpected argv: $*" >&2; exit 3 ;;
esac
"#,
        );
        let mut creator = GhCreator::with_runner(GhRunner::with_bin(bin.to_string_lossy()));
        assert_eq!(
            creator.find_by_marker(&outbox_entry_8361()),
            MarkerLookup::CouldNotCheck("http_404".to_string())
        );
        let _ = std::fs::remove_dir_all(bin.parent().unwrap());
    }

    // ---- #8331: REST issues-list scan before trusting search absence ----
    // Oracle: search indexing lags, so a landed issue can be absent
    // from search results while already present in the repo's issue
    // list. Reconcile must consult the list before concluding
    // Absent for issue-kind entries; a list failure is
    // CouldNotCheck, never Absent.

    #[test]
    fn marker_in_list_but_not_search_reconciles_found() {
        let _gh_stub_guard = GH_STUB_LOCK.lock().unwrap();
        let (bin, log) = stub_gh_8361(
            "listhit8331",
            r#"case "$*" in
  *search/issues*) printf '%s' '{"total_count":0,"items":[]}' ;;
  *issues?state=all*) printf '%s' '[{"number":99,"html_url":"https://github.com/developing-today/code/issues/99","body":"landed <!-- fleet-outbox:OB-test-1 -->"},{"number":98,"html_url":"https://github.com/developing-today/code/issues/98","body":"unrelated"}]' ;;
  *) echo "unexpected argv: $*" >&2; exit 3 ;;
esac
"#,
        );
        let mut creator = GhCreator::with_runner(GhRunner::with_bin(bin.to_string_lossy()));
        assert_eq!(
            creator.find_by_marker(&outbox_entry_8361()),
            MarkerLookup::Found(
                99,
                "https://github.com/developing-today/code/issues/99".to_string()
            )
        );
        let argv_log = std::fs::read_to_string(&log).unwrap();
        assert!(
            argv_log.contains("issues?state=all"),
            "reconcile must scan the REST issues list, argv was: {argv_log}"
        );
        let _ = std::fs::remove_dir_all(bin.parent().unwrap());
    }

    #[test]
    fn marker_absent_from_search_and_list_is_checked_absent() {
        let _gh_stub_guard = GH_STUB_LOCK.lock().unwrap();
        let (bin, _log) = stub_gh_8361(
            "bothabsent8331",
            r#"case "$*" in
  *search/issues*) printf '%s' '{"total_count":0,"items":[]}' ;;
  *issues?state=all*) printf '%s' '[{"number":98,"html_url":"https://github.com/developing-today/code/issues/98","body":"unrelated"}]' ;;
  *) echo "unexpected argv: $*" >&2; exit 3 ;;
esac
"#,
        );
        let mut creator = GhCreator::with_runner(GhRunner::with_bin(bin.to_string_lossy()));
        assert_eq!(
            creator.find_by_marker(&outbox_entry_8361()),
            MarkerLookup::Absent
        );
        let _ = std::fs::remove_dir_all(bin.parent().unwrap());
    }

    #[test]
    fn issues_list_failure_is_could_not_check_not_absent() {
        let _gh_stub_guard = GH_STUB_LOCK.lock().unwrap();
        let (bin, _log) = stub_gh_8361(
            "listfail8331",
            r#"case "$*" in
  *search/issues*) printf '%s' '{"total_count":0,"items":[]}' ;;
  *issues?state=all*) echo 'gh: Forbidden (HTTP 403)' >&2; exit 1 ;;
  *) echo "unexpected argv: $*" >&2; exit 3 ;;
esac
"#,
        );
        let mut creator = GhCreator::with_runner(GhRunner::with_bin(bin.to_string_lossy()));
        assert_eq!(
            creator.find_by_marker(&outbox_entry_8361()),
            MarkerLookup::CouldNotCheck("http_403".to_string())
        );
        let _ = std::fs::remove_dir_all(bin.parent().unwrap());
    }

    #[test]
    fn issues_list_unexpected_shape_is_could_not_check() {
        let _gh_stub_guard = GH_STUB_LOCK.lock().unwrap();
        let (bin, _log) = stub_gh_8361(
            "listshape8331",
            r#"case "$*" in
  *search/issues*) printf '%s' '{"total_count":0,"items":[]}' ;;
  *issues?state=all*) printf '%s' '{"total_count":0,"items":[]}' ;;
  *) echo "unexpected argv: $*" >&2; exit 3 ;;
esac
"#,
        );
        let mut creator = GhCreator::with_runner(GhRunner::with_bin(bin.to_string_lossy()));
        assert_eq!(
            creator.find_by_marker(&outbox_entry_8361()),
            MarkerLookup::CouldNotCheck("unexpected_issues_list_shape".to_string())
        );
        let _ = std::fs::remove_dir_all(bin.parent().unwrap());
    }

    // --- #4919 completion telemetry defaults -------------------------

    #[test]
    fn completion_telemetry_reads_model_and_effort_from_the_claim_stamp() {
        let claim = json!({"claim_model": "claude-sonnet-4-5", "claim_variant": "high"});
        let derived = derive_completion_telemetry(&claim, None);
        assert_eq!(derived["model"], json!("claude-sonnet-4-5"));
        assert_eq!(derived["effort"], json!("high"));
        assert_eq!(derived["pool"], Value::Null);
        assert_eq!(derived["cost_usd"], Value::Null);
        assert_eq!(derived.len(), TELEMETRY_FIELDS.len());
    }

    #[test]
    fn completion_telemetry_falls_back_to_the_session_serving_leaf_and_route() {
        // A routed session: the declared model names the POOL; the
        // serving-leaf readout names the model and effort.
        let session = json!({"data": {
            "model": {"providerID": "opencode-route", "id": "workhorse"},
            "route_usage": {"lastServing": {
                "providerID": "anthropic", "modelID": "claude-opus-4-1", "variant": "medium"
            }},
        }});
        let derived = derive_completion_telemetry(&json!({}), Some(&session));
        assert_eq!(derived["model"], json!("claude-opus-4-1"));
        assert_eq!(derived["effort"], json!("medium"));
        assert_eq!(derived["pool"], json!("workhorse"));
        // A concrete declared model names no pool.
        let concrete = json!({"model": {"providerID": "anthropic", "id": "claude-haiku-4-5"}});
        assert_eq!(session_declared_pool(&concrete), None);
    }

    #[test]
    fn completion_telemetry_claim_wins_and_default_variant_is_absence() {
        let claim = json!({"claim_model": "claim-model", "claim_variant": "default"});
        let session =
            json!({"model": {"providerID": "anthropic", "id": "leaf-model", "variant": "low"}});
        let derived = derive_completion_telemetry(&claim, Some(&session));
        assert_eq!(derived["model"], json!("claim-model"));
        // "default" is the server saying no variant was chosen, so
        // the session's real variant supplies effort.
        assert_eq!(derived["effort"], json!("low"));
    }

    #[test]
    fn completion_telemetry_route_claim_model_is_the_pool_not_the_model() {
        // A transfer-stamped claim through the route provider: its
        // claim_model holds the route id (the pool), never a serving
        // leaf, so the session's leaf supplies the model — and with
        // no session record the model is an honest null rather than
        // the route alias.
        let claim = json!({"claim_provider": "opencode-route", "claim_model": "workhorse"});
        let session = json!({"data": {
            "model": {"providerID": "opencode-route", "id": "workhorse"},
            "route_usage": {"lastServing": {
                "providerID": "anthropic", "modelID": "claude-opus-4-1", "variant": "medium"
            }},
        }});
        let derived = derive_completion_telemetry(&claim, Some(&session));
        assert_eq!(derived["model"], json!("claude-opus-4-1"));
        assert_eq!(derived["effort"], json!("medium"));
        assert_eq!(derived["pool"], json!("workhorse"));
        let derived = derive_completion_telemetry(&claim, None);
        assert_eq!(derived["model"], Value::Null);
    }

    #[test]
    fn completion_telemetry_always_carries_all_four_keys() {
        let derived = derive_completion_telemetry(&json!({}), None);
        for field in TELEMETRY_FIELDS {
            assert!(derived.contains_key(field), "{field} must be present");
            assert_eq!(derived[field], Value::Null, "{field} has no source here");
        }
    }

    #[test]
    fn completion_telemetry_merge_never_overwrites_the_caller() {
        let mut result = json!({"model": "caller-model", "outcome": "pr_opened"});
        let derived = derive_completion_telemetry(&json!({"claim_model": "claim-model"}), None);
        merge_completion_telemetry(&mut result, derived);
        assert_eq!(result["model"], json!("caller-model"));
        assert_eq!(result["effort"], Value::Null);
        assert_eq!(result["outcome"], json!("pr_opened"));
    }

    #[test]
    fn completion_session_id_prefers_env_then_the_claim_row() {
        let claim = json!({"progress": {"session_id": "ses_row"}});
        assert_eq!(
            completion_session_id(Some("ses_env"), &claim).as_deref(),
            Some("ses_env")
        );
        assert_eq!(
            completion_session_id(Some("   "), &claim).as_deref(),
            Some("ses_row")
        );
        assert_eq!(completion_session_id(None, &json!({})), None);
    }

    #[test]
    fn drop_dep_list_parses_the_block_ref_forms() {
        // #12777 oracle (client half): --drop-dep takes the
        // refs block's --blocked-by-* flags take, kind-prefixed,
        // and refuses anything else before a byte is sent.
        let deps = parse_drop_dep_list(
            "issue:developing-today/code#9513, task:20408, ci:developing-today/code#777, pr:developing-today/code#9513",
        )
        .expect("the four ref forms parse");
        assert_eq!(
            deps,
            vec![
                json!({"kind": "issue", "repo": "developing-today/code", "number": 9513}),
                json!({"kind": "task", "task_id": 20408}),
                json!({"kind": "ci", "repo": "developing-today/code", "run_id": 777}),
                json!({"kind": "pr", "repo": "developing-today/code", "number": 9513}),
            ]
        );
        assert_eq!(parse_drop_dep_list("").expect("empty"), Vec::<Value>::new());
        assert!(parse_drop_dep_list("issue:9513").is_err());
        assert!(parse_drop_dep_list("issue:developing-today/code").is_err());
        assert!(parse_drop_dep_list("bogus:developing-today/code#1").is_err());
        assert!(parse_drop_dep_list("task:abc").is_err());
        assert!(parse_drop_dep_list("9513").is_err());
    }

    // ---- #10838: per-verb --help, usage-on-error, bare-list note ----

    #[test]
    fn usage_covers_the_acceptance_verbs() {
        // Acceptance minimum: claim, list, enqueue, checkpoint, park,
        // block, complete, health, fallback each print a verb usage
        // that names the verb's own flags.
        for verb in [
            "claim",
            "list",
            "enqueue",
            "checkpoint",
            "park",
            "block",
            "complete",
            "health",
            "fallback",
        ] {
            let u = usage(verb);
            assert!(
                u.starts_with("usage: repo-queue"),
                "{verb} usage header: {u:?}"
            );
            assert!(u.contains(verb), "{verb} usage names the verb: {u:?}");
        }
    }

    #[test]
    fn usage_never_prints_nothing() {
        // Unknown verbs (and the top level) fall back to TOP_USAGE,
        // so a usage error always teaches something.
        for verb in ["", "bogus-verb"] {
            assert!(!usage(verb).is_empty(), "{verb:?} must have usage text");
        }
        assert!(TOP_USAGE.contains("repo-queue"));
    }

    #[test]
    fn usage_error_text_carries_message_and_usage() {
        let text = usage_error_text("claim", "claim: unrecognized argument: --bogus");
        assert!(text.contains("--bogus"), "message kept: {text:?}");
        assert!(text.contains(&usage("claim").to_string()), "usage appended");
        // Exact form: the bare verb usage, unadorned, for the
        // `<verb> --help` path to print.
        assert!(usage("claim").starts_with("usage: repo-queue claim"));
    }

    #[test]
    fn help_flag_matching_covers_long_short_and_bare() {
        assert!(is_help_arg("--help"));
        assert!(is_help_arg("-h"));
        assert!(is_help_arg("help"));
        assert!(!is_help_arg("--bogus"));
        assert!(!is_help_arg("claim"));
    }

    #[test]
    fn a_help_word_given_as_a_flag_value_is_not_a_help_request() {
        let flags = ["worker", "error", "reason"];
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // `fail 5 --error help` must fail the task with error "help".
        assert!(!wants_help(&v(&["5", "--error", "help"]), &flags));
        assert!(!wants_help(&v(&["5", "--reason", "-h"]), &flags));
        // A real help token still wins anywhere else.
        assert!(wants_help(&v(&["5", "--error", "boom", "--help"]), &flags));
        assert!(wants_help(&v(&["5", "--error=help", "-h"]), &flags));
        // `--error --help`: the parser never takes a `--` token as a value.
        assert!(wants_help(&v(&["5", "--error", "--help"]), &flags));
        assert!(wants_help(&v(&["help"]), &flags));
    }

    /// Every verb's usage names exactly the flags its parser accepts, and as
    /// many leading `<positional>`s as it allows (#10838). Read from this
    /// file's own `parse_verb_args` / `reject_extra_pos` calls, so a flag
    /// added to a parser without its usage (or the reverse) fails here.
    #[test]
    fn every_verb_usage_matches_its_parser() {
        let src = [
            include_str!("client/transport.rs"),
            include_str!("client/support.rs"),
            include_str!("client/commands_query.rs"),
            include_str!("client/commands_lifecycle.rs"),
            include_str!("client/fallback_stack.rs"),
            include_str!("client.rs"),
        ]
        .concat();
        let list = |text: &str| -> Vec<String> {
            text.split('"')
                .skip(1)
                .step_by(2)
                .map(str::to_string)
                .collect()
        };
        let resolve = |arg: &str| -> Vec<String> {
            let arg = arg.trim();
            if arg.starts_with("&[") {
                return list(arg);
            }
            let decl = format!("const {arg}: &[&str] = &[");
            let start = src.find(&decl).unwrap_or_else(|| panic!("no const {arg}")) + decl.len();
            list(&src[start..start + src[start..].find(']').unwrap()])
        };
        let mut checked = 0;
        // Split so this test's own source doesn't match the marker.
        let marker = ["parse_verb", "_args("].concat();
        for (at, _) in src.match_indices(marker.as_str()) {
            // rustfmt may put the verb on its own line.
            let Some(call) = src[at + marker.len()..].trim_start().strip_prefix('"') else {
                continue;
            };
            let verb = &call[..call.find('"').unwrap()];
            if verb.is_empty() || verb.contains('{') {
                continue;
            }
            let after_rest = &call[call.find("rest,").unwrap() + "rest,".len()..];
            let (flags_arg, tail) = match after_rest.trim_start().starts_with("&[") {
                true => {
                    let t = after_rest.trim_start();
                    let end = t.find(']').unwrap() + 1;
                    (&t[..end], &t[end..])
                }
                false => {
                    let t = after_rest.trim_start();
                    let end = t.find(',').unwrap();
                    (&t[..end], &t[end..])
                }
            };
            let tail = tail.trim_start().trim_start_matches(',').trim_start();
            let bools_arg = if tail.starts_with("&[") {
                &tail[..tail.find(']').unwrap() + 1]
            } else {
                &tail[..tail.find(|c: char| c == ',' || c == ')').unwrap()]
            };
            let mut want: Vec<String> = resolve(flags_arg);
            want.extend(resolve(bools_arg));
            want.sort();
            let text = usage(verb);
            let synopsis = text.lines().next().unwrap();
            let mut got: Vec<String> = synopsis
                .split(|c: char| c == ' ' || c == '[' || c == ']' || c == '|')
                .filter_map(|w| w.strip_prefix("--"))
                .map(|w| w.split('=').next().unwrap().to_string())
                .collect();
            got.sort();
            got.dedup();
            assert_eq!(
                got, want,
                "usage for {verb:?} must list exactly its flags: {synopsis}"
            );
            let reject = format!("reject_extra_pos(\"{verb}\", &args, ");
            if let Some(r) = src.find(&reject) {
                let n: usize = src[r + reject.len()..]
                    .split(')')
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap();
                // Leading positionals only: the `<...>` words before the
                // first flag (a flag's own `<value>` is not a positional).
                let positionals = synopsis
                    .split_whitespace()
                    .skip_while(|w| {
                        !w.starts_with('<') && !w.starts_with('[') && !w.starts_with("--")
                    })
                    .take_while(|w| w.starts_with('<'))
                    .count();
                assert_eq!(positionals, n, "usage for {verb:?} positionals: {synopsis}");
            }
            // Required-vs-optional is part of the contract (#13174): a
            // flag the handler dies without must be written bare in the
            // synopsis, and a flag written bare must be one the handler
            // dies without. The requirement is read from the handlers'
            // own die(2, ...) messages — "<verb>: --<flag> ... is
            // required" and the "<verb> requires --<flag>" form — so a
            // requirement added without its usage (or the reverse)
            // fails here. Conditional requirements are phrased
            // differently in the source (claim --bind's --session-id,
            // lane's --limit-reason) and stay out of this set.
            let mut source_required: Vec<String> = want
                .iter()
                .filter(|flag| {
                    let colon_form = format!("{verb}: --{flag}");
                    let requires_form = format!("{verb} requires --{flag}");
                    if src.contains(&requires_form) {
                        return true;
                    }
                    src.match_indices(&colon_form).any(|(at, _)| {
                        let rest = &src[at + colon_form.len()..];
                        let literal = &rest[..rest.find('"').unwrap_or(rest.len())];
                        literal.contains("is required")
                    })
                })
                .cloned()
                .collect();
            source_required.sort();
            // Flags written outside any [...] group in the synopsis.
            let mut depth = 0i32;
            let mut documented_required: Vec<String> = Vec::new();
            let mut chars = synopsis.chars().peekable();
            while let Some(c) = chars.next() {
                match c {
                    '[' => depth += 1,
                    ']' => depth -= 1,
                    '-' if depth == 0 && chars.peek() == Some(&'-') => {
                        chars.next();
                        let name: String = chars
                            .by_ref()
                            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                            .collect();
                        if !name.is_empty() {
                            documented_required.push(name);
                        }
                    }
                    _ => {}
                }
            }
            documented_required.sort();
            documented_required.dedup();
            assert_eq!(
                documented_required, source_required,
                "usage for {verb:?} must write required flags bare and optional flags bracketed: {synopsis}"
            );
            checked += 1;
        }
        assert!(checked >= 30, "scanned only {checked} verbs");
    }

    /// #13175: the parity test above cross-checks the per-verb
    /// usages against the parsers, but TOP_USAGE — the text a cold
    /// session reads first — drifted on its own (its fallback line
    /// omitted `complete`; its lifecycle lines omitted `[--lane
    /// <id>]`). Every single-verb TOP_USAGE line must list exactly
    /// the flags of that verb's own usage synopsis. Group lines
    /// (ending in `...`) summarize multi-verb surfaces whose own
    /// usages carry the flags, and stay out of this check.
    #[test]
    fn top_usage_lines_match_per_verb_usage() {
        let flags = |text: &str| -> Vec<String> {
            let mut out: Vec<String> = text
                .split(|c: char| c == ' ' || c == '[' || c == ']' || c == '|')
                .filter_map(|w| w.strip_prefix("--"))
                .map(|w| w.split('=').next().unwrap().to_string())
                .collect();
            out.sort();
            out.dedup();
            out
        };
        let mut checked = 0;
        for line in TOP_USAGE.lines() {
            let line = line.trim();
            if line.is_empty() || line.ends_with("...") {
                continue;
            }
            let verb = line.split_whitespace().next().unwrap_or("");
            let text = usage(verb);
            if text == TOP_USAGE {
                // Not a verb with its own usage (header lines).
                continue;
            }
            let synopsis = text.lines().next().unwrap();
            assert_eq!(
                flags(line),
                flags(synopsis),
                "TOP_USAGE line for {verb:?} must list exactly the flags of its usage: {line}"
            );
            checked += 1;
        }
        assert!(checked >= 25, "checked only {checked} TOP_USAGE lines");
    }

    #[test]
    fn bare_list_note_fires_only_on_unfiltered_list() {
        let bare = parse_args_inner(&[], &["queue", "status", "limit"], &[]).unwrap();
        let note = bare_list_note(&bare, Some("repo-queue-code-pilot"));
        assert!(note.is_some(), "bare list must carry the scope note");
        let note = note.unwrap();
        assert!(
            note.contains("repo-queue-code-pilot"),
            "note names the queue: {note:?}"
        );
        assert!(
            note.contains("--status"),
            "note points at filters: {note:?}"
        );
        // Any scope filter silences the note.
        for flag in ["queue", "status", "lane", "worker", "task"] {
            let filtered = parse_args_inner(
                &[format!("--{flag}"), "x".to_string()],
                &["queue", "status", "limit", "lane", "worker", "task"],
                &[],
            )
            .unwrap();
            assert!(
                bare_list_note(&filtered, Some("repo-queue-code-pilot")).is_none(),
                "--{flag} must silence the note"
            );
        }
    }

    /// #7358: with no --queue and no REPO_QUEUE_DEFAULT_QUEUE, claim
    /// and list are unscoped; they never fall back to the shadow queue.
    #[test]
    fn claim_and_list_scope_never_defaults_to_the_shadow_queue() {
        assert_eq!(queue_scope_from(None, None), None);
        assert_eq!(queue_scope_from(None, Some("  ")), None);
        assert_eq!(
            queue_scope_from(None, Some(" repo-health-code-pilot ")).as_deref(),
            Some("repo-health-code-pilot")
        );
        assert_eq!(
            queue_scope_from(Some("q-flag".to_string()), Some("q-env")).as_deref(),
            Some("q-flag")
        );
        // The shadow queue is reachable only by naming it.
        assert_eq!(
            queue_scope_from(Some(DEFAULT_QUEUE.to_string()), None).as_deref(),
            Some(DEFAULT_QUEUE)
        );
        let bare = parse_args_inner(&[], &["queue", "status", "limit"], &[]).unwrap();
        let note = bare_list_note(&bare, None).unwrap();
        assert!(note.contains("every queue"), "{note:?}");
        assert!(!note.contains(DEFAULT_QUEUE), "{note:?}");
    }

    #[test]
    fn extra_positional_reports_the_first_extra() {
        assert_eq!(extra_positional(&[], 1), None);
        assert_eq!(extra_positional(&["a".into()], 1), None);
        assert_eq!(extra_positional(&["a".into(), "b".into()], 1), Some("b"));
    }
}
