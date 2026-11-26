use super::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const OWNER: &str = "oooooooooooooooooooo0123456789abcdefXYZ_-";
// node: crypto.createHmac("sha256", OWNER).update(label).digest("base64url"),
// the same value the bridge's deriveFleetToken gives (worker/src/fleet.js).
const OWNER_FLEET: &str = "BvfiQrmwOt0zBdk0Uy1b-eOVZhNdd1vsn3UGggDszX4";

struct MapSecrets {
    values: HashMap<String, String>,
    stored: std::cell::RefCell<HashMap<String, String>>,
}

impl Secrets for MapSecrets {
    fn resolve(&self, name: &str) -> Result<String, String> {
        self.values
            .get(name)
            .cloned()
            .ok_or_else(|| format!("no secret named {name}"))
    }

    fn store(&self, name: &str, value: &str) -> Result<(), String> {
        self.stored
            .borrow_mut()
            .insert(name.to_string(), value.to_string());
        Ok(())
    }
}

fn secrets(pairs: &[(&str, &str)]) -> MapSecrets {
    MapSecrets {
        values: pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        stored: std::cell::RefCell::new(HashMap::new()),
    }
}

fn sv(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| s.to_string()).collect()
}

fn settings(url: &str) -> Settings {
    Settings {
        url: url.to_string(),
        agent: None,
        groups: None,
        owner_token_name: DEFAULT_BRIDGE_OWNER_TOKEN_NAME.to_string(),
        fleet_token_name: DEFAULT_BRIDGE_FLEET_TOKEN_NAME.to_string(),
        notes_token_name: None,
    }
}

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    url: String,
    body: Option<Value>,
    auth: Option<String>,
}

/// A fake bridge: answers each request with `reply(method, path) -> (status, json)`.
fn fake_bridge(
    reply: impl Fn(&str, &str, &Option<Value>) -> (u16, String) + Send + 'static,
) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.server_addr().to_ip().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for mut req in server.incoming_requests() {
            let mut body = String::new();
            req.as_reader().read_to_string(&mut body).unwrap();
            let body: Option<Value> = serde_json::from_str(&body).ok();
            let method = req.method().to_string();
            let path = req.url().to_string();
            let auth = req
                .headers()
                .iter()
                .find(|h| h.field.as_str().to_ascii_lowercase() == "authorization")
                .map(|h| h.value.as_str().to_string());
            let (status, text) = reply(&method, &path, &body);
            log.lock().unwrap().push(Seen {
                method,
                url: path,
                body,
                auth,
            });
            let _ = req.respond(tiny_http::Response::from_string(text).with_status_code(status));
        }
    });
    (url, seen)
}

fn run_with(url: &str, s: &MapSecrets, args: &[&str], stdin: &str) -> R<Value> {
    let mut input = stdin.as_bytes();
    let sleep = |_: Duration| {};
    let mut ctx = Ctx {
        settings: settings(url),
        secrets: s,
        stdin: &mut input,
        sleep: &sleep,
    };
    execute(&mut ctx, &sv(args))
}

#[test]
fn hmac_and_base64url_match_the_bridge() {
    assert_eq!(derive_fleet_token(OWNER), OWNER_FLEET);
    // A key longer than the block is hashed first (node, same call).
    assert_eq!(
        b64url(&hmac_sha256("k".repeat(100).as_bytes(), b"abc")),
        "tYsraU_boN122j6-mRdPco0ydWDzbs4iTpCGeXJHmSI"
    );
    assert_eq!(b64url(b"f"), "Zg");
    assert_eq!(b64url(b"fo"), "Zm8");
    assert_eq!(b64url(b"foo"), "Zm9v");
    assert_eq!(derive_fleet_token(OWNER).len(), 43);
}

#[test]
fn urls_are_checked() {
    assert_eq!(
        check_url("https://bridge.security.cab/").unwrap(),
        "https://bridge.security.cab"
    );
    assert!(check_url("http://127.0.0.1:8787").is_ok());
    assert!(check_url("http://localhost:1").is_ok());
    for bad in [
        "http://bridge.security.cab",
        "ftp://x",
        "https://x/path",
        "https://x?q",
        "https://",
        "bridge.security.cab",
    ] {
        assert!(check_url(bad).is_err(), "{bad}");
    }
}

#[test]
fn settings_env_beats_file_beats_default() {
    let mut config = Config::default();
    let none = |_: &str| None;
    let s = Settings::resolve(&config, &none).unwrap();
    assert_eq!(s.url, DEFAULT_BRIDGE_URL);
    assert_eq!(s.agent, None);
    config.bridge.agent = Some("from-file".into());
    config.bridge.groups = Some(sv(&["muse"]));
    config.bridge.url = Some("https://file.example".into());
    let s = Settings::resolve(&config, &none).unwrap();
    assert_eq!(s.agent.as_deref(), Some("from-file"));
    assert_eq!(s.url, "https://file.example");
    let env = |k: &str| match k {
        "FLEET_AGENT" => Some("from-env".to_string()),
        "FLEET_AGENT_GROUPS" => Some("a, b,,".to_string()),
        "FLEET_BRIDGE_URL" => Some("https://env.example/".to_string()),
        _ => None,
    };
    let s = Settings::resolve(&config, &env).unwrap();
    assert_eq!(s.agent.as_deref(), Some("from-env"));
    assert_eq!(s.groups, Some(sv(&["a", "b"])));
    assert_eq!(s.url, "https://env.example");
    let bad = |k: &str| (k == "FLEET_BRIDGE_URL").then(|| "http://evil.example".to_string());
    assert_eq!(
        Settings::resolve(&config, &bad).unwrap_err().exit_code(),
        EXIT_USAGE
    );
}

#[test]
fn every_verb_has_its_own_help() {
    // #13197: the domain status line advertises `fleet bridge <command>
    // --help`; every verb execute() dispatches must answer with its own
    // usage — its own usage line, the shared footer, and never the
    // whole-domain USAGE text.
    for verb in [
        "send",
        "offer",
        "offers",
        "agents",
        "log",
        "pipeline",
        "directory",
        "credentials",
        "revoke",
        "mint-enroll",
        "enroll-tokens",
        "revoke-enroll",
        "mint-credential",
        "wake-hooks",
        "hello",
        "inbox",
        "post",
        "ack",
        "work",
        "health",
        "exchange",
        "notes",
    ] {
        let text = verb_usage(verb).unwrap_or_else(|| panic!("no per-verb help for {verb}"));
        assert!(
            text.starts_with(&format!("usage: fleet bridge {verb}")),
            "{verb}: {text}"
        );
        assert!(text.ends_with(USAGE_FOOTER), "{verb}: missing the footer");
        assert_ne!(text, USAGE, "{verb}: fell back to the shared text");
    }
    assert!(verb_usage("bogus").is_none());
    // The footer is USAGE's own tail, so the two cannot drift apart.
    assert!(USAGE.ends_with(USAGE_FOOTER));
}

#[test]
fn usage_errors_exit_2_before_any_request() {
    let s = secrets(&[]);
    // Port 9 (discard): nothing should ever connect for these.
    let url = "http://127.0.0.1:9";
    for args in [
        vec![],
        vec!["bogus"],
        vec!["send", "hi"],
        vec!["send", "--to", "all"],
        vec!["send", "--to"],
        vec!["send", "--nope", "x"],
        vec!["send", "--broaden", "b1", "leftover text"],
        vec!["send", "--limit", "many", "--to", "all", "x"],
        vec!["offer", "--to", "all", "x"],
        vec!["offer", "--slots", "2", "x"],
        vec!["offer", "--to", "all", "--slots", "many", "x"],
        vec!["offers", "extra"],
        vec!["agents", "extra"],
        vec!["inbox"],
        vec!["inbox", "--agent", "a", "--commands", "--messages"],
        vec!["inbox", "--agent", "a", "--wait", "x"],
        vec!["inbox", "--agent", "a", "--wait", "999999"],
        vec!["inbox", "--agent", "a", "--take=yes"],
        vec!["post", "--agent", "a"],
        vec!["post", "--agent", "a", "--reply-to", "-3", "x"],
        vec!["post", "--agent", "a", "--instruction", "x"],
        vec![
            "post", "--agent", "a", "--to", "claude", "--kind", "shout", "x",
        ],
        vec!["ack", "--agent", "a"],
        vec!["ack", "--agent", "a", "0"],
        vec!["work", "--agent", "a"],
        vec!["work", "--agent", "a", "--state", "done"],
        vec!["work", "--agent", "a", "--state", "sideways", "3"],
        vec!["work", "--agent", "a", "--state", "received", "3"],
        vec!["work", "--agent", "a", "--state", "queued", "3"],
        vec!["work", "--agent", "a", "--state", "done", "0"],
        vec![
            "work", "--agent", "a", "--state", "done", "--task", "x", "3",
        ],
        vec!["log", "--limit", "many"],
        vec!["pipeline", "--limit", "many"],
        vec!["pipeline", "--bogus", "x"],
        vec!["directory", "extra"],
        vec!["credentials", "extra"],
        vec!["revoke"],
        vec!["revoke", "--agent", "a", "--identity", "muse/a"],
        vec!["revoke", "felix"],
        vec!["mint-enroll"],
        vec!["mint-enroll", "--agent", "a", "--ttl", "soon"],
        vec!["mint-enroll", "--agent", "a", "extra"],
        vec!["enroll-tokens", "extra"],
        vec!["revoke-enroll"],
        vec!["revoke-enroll", "--agent", "a", "--token-hash", "ab12"],
        vec!["mint-credential"],
        vec!["wake-hooks", "extra"],
        vec!["wake-hooks", "--set", "hook.json"],
        vec!["wake-hooks", "--remove", "--address"],
    ] {
        let e = run_with(url, &s, &args, "").unwrap_err();
        assert_eq!(e.exit_code(), EXIT_USAGE, "{args:?}: {e}");
    }
}

#[test]
fn missing_credentials_exit_3_and_name_the_secret() {
    let s = secrets(&[]);
    let url = "http://127.0.0.1:9";
    let e = run_with(url, &s, &["agents"], "").unwrap_err();
    assert_eq!(e.exit_code(), EXIT_TRANSPORT);
    assert!(
        e.to_string().contains(DEFAULT_BRIDGE_OWNER_TOKEN_NAME),
        "{e}"
    );
    let e = run_with(url, &s, &["inbox", "--agent", "a"], "").unwrap_err();
    assert_eq!(e.exit_code(), EXIT_TRANSPORT);
    assert!(
        e.to_string().contains(DEFAULT_BRIDGE_FLEET_TOKEN_NAME),
        "{e}"
    );
}

#[test]
fn central_send_uses_the_owner_token_and_shapes_the_body() {
    let (url, seen) = fake_bridge(|_, _, _| {
        (
            201,
            r#"{"batch":"b1","messages":[],"unregistered":[]}"#.into(),
        )
    });
    let s = secrets(&[(DEFAULT_BRIDGE_OWNER_TOKEN_NAME, OWNER)]);
    let out = run_with(
        &url,
        &s,
        &[
            "send",
            "--to",
            "group:muse,agent:aster",
            "--to=all",
            "--command",
            "--sender",
            "drew",
            "run",
            "the",
            "check",
        ],
        "",
    )
    .unwrap();
    assert_eq!(out["batch"], "b1");
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("/api/{OWNER}/fleet/send"));
    assert_eq!(
        seen[0].body.as_ref().unwrap(),
        &json!({
            "body": "run the check",
            "to": ["group:muse", "agent:aster", "all"],
            "kind": "command",
            "sender": "drew"
        })
    );
}

#[test]
fn a_pattern_send_with_a_limit_carries_both_and_returns_the_match_report() {
    // #14375: the console composer sends agent:f* with a limit and
    // shows the bridge's match report; the CLI carries the limit
    // through and hands the report back untouched.
    let (url, seen) = fake_bridge(|_, _, _| {
        (
            201,
            r#"{"batch":"b9","messages":[],"unregistered":[],"matched":2,"delivered":1,"recipients":["felix-1"],"remaining":1}"#
                .into(),
        )
    });
    let s = secrets(&[(DEFAULT_BRIDGE_OWNER_TOKEN_NAME, OWNER)]);
    let out = run_with(
        &url,
        &s,
        &["send", "--to", "agent:f*", "--limit", "1", "--command", "-"],
        "do the thing",
    )
    .unwrap();
    assert_eq!(out["matched"], 2);
    assert_eq!(out["delivered"], 1);
    assert_eq!(out["recipients"], json!(["felix-1"]));
    assert_eq!(out["remaining"], 1);
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].url, format!("/api/{OWNER}/fleet/send"));
    assert_eq!(
        seen[0].body.as_ref().unwrap(),
        &json!({ "body": "do the thing", "to": ["agent:f*"], "kind": "command", "limit": 1 })
    );
}

#[test]
fn broaden_carries_the_batch_and_no_text() {
    let (url, seen) = fake_bridge(|_, _, _| {
        (
            201,
            r#"{"batch":"b9","messages":[],"unregistered":[],"matched":2,"delivered":1,"recipients":["felix-2"],"remaining":0}"#
                .into(),
        )
    });
    let s = secrets(&[(DEFAULT_BRIDGE_OWNER_TOKEN_NAME, OWNER)]);
    let out = run_with(&url, &s, &["send", "--broaden", "b9"], "").unwrap();
    assert_eq!(out["remaining"], 0);
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("/api/{OWNER}/fleet/send"));
    assert_eq!(seen[0].body.as_ref().unwrap(), &json!({ "broaden": "b9" }));
}

#[test]
fn offer_posts_its_slots_and_ttl_and_offers_lists() {
    let (url, seen) = fake_bridge(|_, _, body| {
        if body.is_some() {
            (
                201,
                r#"{"offer":{"id":7,"state":"open","claimed":0,"remaining":2}}"#.into(),
            )
        } else {
            (
                200,
                r#"{"offers":[{"id":7,"state":"open","claims":[]}]}"#.into(),
            )
        }
    });
    let s = secrets(&[(DEFAULT_BRIDGE_OWNER_TOKEN_NAME, OWNER)]);
    let out = run_with(
        &url,
        &s,
        &[
            "offer",
            "--to",
            "agent:f*",
            "--slots",
            "2",
            "--ttl",
            "3600",
            "--command",
            "-",
        ],
        "take the next task",
    )
    .unwrap();
    assert_eq!(out["offer"]["state"], "open");
    let out = run_with(&url, &s, &["offers"], "").unwrap();
    assert_eq!(out["offers"][0]["id"], 7);
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].url, format!("/api/{OWNER}/fleet/offer"));
    assert_eq!(
        seen[0].body.as_ref().unwrap(),
        &json!({
            "body": "take the next task",
            "to": ["agent:f*"],
            "kind": "command",
            "slots": 2,
            "ttl_seconds": 3600
        })
    );
    assert_eq!(seen[1].method, "GET");
    assert_eq!(seen[1].url, format!("/api/{OWNER}/fleet/offers"));
}

#[test]
fn text_can_come_from_stdin() {
    let (url, seen) = fake_bridge(|_, _, _| (201, r#"{"message":{}}"#.into()));
    let s = secrets(&[(DEFAULT_BRIDGE_FLEET_TOKEN_NAME, "f".repeat(40).as_str())]);
    run_with(
        &url,
        &s,
        &["post", "--agent", "Muse-1", "--reply-to", "7", "-"],
        "line one\nline two\n",
    )
    .unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].url, format!("/fleet/{}/send", "f".repeat(40)));
    assert_eq!(
        seen[0].body.as_ref().unwrap(),
        &json!({"agent": "muse-1", "body": "line one\nline two\n", "reply_to": 7})
    );
}

#[test]
fn agent_verbs_derive_the_fleet_token_from_the_owner_token() {
    let (url, seen) = fake_bridge(|_, _, _| (200, r#"{"agent":{"name":"m"}}"#.into()));
    let s = secrets(&[(DEFAULT_BRIDGE_OWNER_TOKEN_NAME, OWNER)]);
    run_with(
        &url,
        &s,
        &["hello", "--agent", "m", "--groups", "muse, code-lanes"],
        "",
    )
    .unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].url, format!("/fleet/{OWNER_FLEET}/hello"));
    assert_eq!(
        seen[0].body.as_ref().unwrap(),
        &json!({"agent": "m", "groups": ["muse", "code-lanes"]})
    );
}

#[test]
fn inbox_builds_its_query_and_waits_until_something_arrives() {
    let calls = Arc::new(Mutex::new(0u32));
    let c = calls.clone();
    let (url, seen) = fake_bridge(move |_, _, _| {
        let mut n = c.lock().unwrap();
        *n += 1;
        if *n < 3 {
            (200, r#"{"messages":[],"next_since_id":0}"#.into())
        } else {
            (
                200,
                r#"{"messages":[{"id":5,"kind":"command"}],"next_since_id":5}"#.into(),
            )
        }
    });
    let s = secrets(&[(DEFAULT_BRIDGE_FLEET_TOKEN_NAME, "f".repeat(40).as_str())]);
    let slept = Arc::new(Mutex::new(Vec::new()));
    let sl = slept.clone();
    let sleep = move |d: Duration| sl.lock().unwrap().push(d.as_secs());
    let mut input: &[u8] = b"";
    let mut st = settings(&url);
    st.agent = Some("muse-3".into());
    st.groups = Some(sv(&["muse"]));
    let mut ctx = Ctx {
        settings: st,
        secrets: &s,
        stdin: &mut input,
        sleep: &sleep,
    };
    let out = execute(
        &mut ctx,
        &sv(&[
            "inbox",
            "--commands",
            "--take",
            "--wait",
            "600",
            "--interval",
            "2",
        ]),
    )
    .unwrap();
    assert_eq!(out["messages"][0]["id"], 5);
    assert_eq!(*slept.lock().unwrap(), vec![2, 2]);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(
        seen[0].url,
        format!(
            "/fleet/{}/inbox?agent=muse-3&groups=muse&kind=command&mark_read=true",
            "f".repeat(40)
        )
    );
}

#[test]
fn inbox_without_wait_returns_empty_at_once() {
    let (url, seen) = fake_bridge(|_, _, _| (200, r#"{"messages":[]}"#.into()));
    let s = secrets(&[(DEFAULT_BRIDGE_FLEET_TOKEN_NAME, "f".repeat(40).as_str())]);
    let out = run_with(
        &url,
        &s,
        &["inbox", "--agent", "a", "--all", "--since-id", "9"],
        "",
    )
    .unwrap();
    assert_eq!(out["messages"], json!([]));
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(
        seen[0]
            .url
            .ends_with("?agent=a&since_id=9&unread_only=false"),
        "{}",
        seen[0].url
    );
}

#[test]
fn ack_log_and_agents_shapes() {
    let (url, seen) = fake_bridge(|_, _, _| (200, r#"{"ok":true}"#.into()));
    let s = secrets(&[(DEFAULT_BRIDGE_OWNER_TOKEN_NAME, OWNER)]);
    run_with(&url, &s, &["ack", "--agent", "a", "3", "4"], "").unwrap();
    run_with(&url, &s, &["ack", "--agent", "a", "--up-to", "9"], "").unwrap();
    run_with(&url, &s, &["agents"], "").unwrap();
    run_with(
        &url,
        &s,
        &[
            "log",
            "--agent",
            "muse 1",
            "--direction",
            "outbound",
            "--limit",
            "5",
        ],
        "",
    )
    .unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(
        seen[0].body.as_ref().unwrap(),
        &json!({"agent": "a", "ids": [3, 4]})
    );
    assert_eq!(
        seen[1].body.as_ref().unwrap(),
        &json!({"agent": "a", "up_to_id": 9})
    );
    assert_eq!(seen[2].url, format!("/api/{OWNER}/fleet/agents"));
    assert_eq!(
        seen[3].url,
        format!("/api/{OWNER}/fleet/messages?agent=muse%201&direction=outbound&limit=5")
    );
}

#[test]
fn work_marks_the_agents_own_rows_and_names_the_queue_task() {
    let (url, seen) = fake_bridge(|_, _, _| {
        (
            200,
            r#"{"message":{"id":3,"work_state":"queued","work_task_id":23500}}"#.into(),
        )
    });
    let s = secrets(&[(DEFAULT_BRIDGE_FLEET_TOKEN_NAME, "f".repeat(40).as_str())]);
    let out = run_with(
        &url,
        &s,
        &[
            "work", "--agent", "muse-1", "--state", "queued", "--task", "23500", "3",
        ],
        "",
    )
    .unwrap();
    assert_eq!(out["message"]["work_state"], "queued");
    run_with(
        &url,
        &s,
        &["work", "--agent", "muse-1", "--state", "acted", "4", "5"],
        "",
    )
    .unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("/fleet/{}/work", "f".repeat(40)));
    assert_eq!(
        seen[0].body.as_ref().unwrap(),
        &json!({"agent": "muse-1", "id": 3, "state": "queued", "task_id": 23500})
    );
    // Several ids are one request each, in order.
    assert_eq!(seen.len(), 3);
    assert_eq!(
        seen[1].body.as_ref().unwrap(),
        &json!({"agent": "muse-1", "id": 4, "state": "acted"})
    );
    assert_eq!(
        seen[2].body.as_ref().unwrap(),
        &json!({"agent": "muse-1", "id": 5, "state": "acted"})
    );
}

#[test]
fn work_help_carries_the_task_it_or_queue_it_rule() {
    // #14372 acceptance: the help text and the book carry the same
    // rule, verbatim. The book half is pinned by the text itself
    // (pkgs/fleet/book/reference/bridge.md quotes WORK_RULE).
    let text = verb_usage("work").unwrap();
    assert!(text.contains(WORK_RULE), "{text}");
    assert!(WORK_RULE.contains("repo-task/v1"), "{WORK_RULE}");
    assert!(WORK_RULE.contains("context_refs"), "{WORK_RULE}");
}

#[test]
fn pipeline_directory_credentials_and_revoke_shapes() {
    // #14374: the console's unified views and its revoke control, as
    // owner-token calls on the bridge's own routes.
    let (url, seen) = fake_bridge(|_, _, _| (200, r#"{"ok":true}"#.into()));
    let s = secrets(&[(DEFAULT_BRIDGE_OWNER_TOKEN_NAME, OWNER)]);
    run_with(
        &url,
        &s,
        &[
            "pipeline", "--kind", "command", "--sender", "felix", "--unread", "--limit", "25",
            "--order", "desc",
        ],
        "",
    )
    .unwrap();
    run_with(&url, &s, &["directory"], "").unwrap();
    run_with(&url, &s, &["credentials"], "").unwrap();
    run_with(&url, &s, &["revoke", "--agent", "felix"], "").unwrap();
    run_with(&url, &s, &["revoke", "--identity", "muse/felix"], "").unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(
        seen[0].url,
        format!(
            "/api/{OWNER}/pipeline?kind=command&sender=felix&unread_only=true&limit=25&order=desc"
        )
    );
    assert_eq!(seen[1].url, format!("/api/{OWNER}/agents"));
    assert_eq!(seen[2].url, format!("/api/{OWNER}/fleet/credentials"));
    assert_eq!(
        seen[3].url,
        format!("/api/{OWNER}/fleet/credentials/revoke")
    );
    assert_eq!(seen[3].body.as_ref().unwrap(), &json!({"agent": "felix"}));
    assert_eq!(seen[4].url, format!("/api/{OWNER}/notes-tokens/revoke"));
    assert_eq!(
        seen[4].body.as_ref().unwrap(),
        &json!({"identity": "muse/felix"})
    );
}

#[test]
fn wake_hooks_verb_shapes() {
    // #14544: the console's wake-hook control, as owner-token calls
    // on the wake-hooks API. The set form reads its JSON from stdin,
    // so a hook token never travels as an argument; the remove form
    // posts the address alone, which is how the API removes.
    let (url, seen) = fake_bridge(|_, _, _| (200, r#"{"ok":true}"#.into()));
    let s = secrets(&[(DEFAULT_BRIDGE_OWNER_TOKEN_NAME, OWNER)]);
    run_with(&url, &s, &["wake-hooks"], "").unwrap();
    run_with(
        &url,
        &s,
        &["wake-hooks", "--set", "-"],
        r#"{"address":"agent:felix","url":"https://hooks.example/wake","token":"sekret"}"#,
    )
    .unwrap();
    run_with(
        &url,
        &s,
        &["wake-hooks", "--remove", "--address", "agent:felix"],
        "",
    )
    .unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].url, format!("/api/{OWNER}/wake-hooks"));
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[1].url, format!("/api/{OWNER}/wake-hooks"));
    assert_eq!(seen[1].method, "POST");
    assert_eq!(
        seen[1].body.as_ref().unwrap(),
        &json!({"address": "agent:felix", "url": "https://hooks.example/wake", "token": "sekret"})
    );
    assert_eq!(seen[2].url, format!("/api/{OWNER}/wake-hooks"));
    assert_eq!(
        seen[2].body.as_ref().unwrap(),
        &json!({"address": "agent:felix"})
    );
    // Usage errors are usage errors: they answer before any request.
    for args in [
        vec!["wake-hooks", "--set"],
        vec!["wake-hooks", "--remove"],
        vec![
            "wake-hooks",
            "--set",
            "--remove",
            "--address",
            "agent:felix",
        ],
        vec!["wake-hooks", "--address", "agent:felix"],
    ] {
        let e = run_with(&url, &s, &args, "").unwrap_err();
        assert_eq!(e.exit_code(), EXIT_USAGE, "{args:?}: {e}");
    }
    assert_eq!(seen.len(), 3, "the usage errors made no request");
}

#[test]
fn provisioning_verb_shapes() {
    // #14376: the console's provisioning verbs, as owner-token calls on
    // the bridge's own routes. The mint answers carry the values; the
    // CLI passes them through untouched and keeps nothing.
    let (url, seen) = fake_bridge(|_, _, _| (201, r#"{"ok":true}"#.into()));
    let s = secrets(&[(DEFAULT_BRIDGE_OWNER_TOKEN_NAME, OWNER)]);
    run_with(
        &url,
        &s,
        &[
            "mint-enroll",
            "--agent",
            "new-1",
            "--kind",
            "muse",
            "--groups",
            "muse,code-lanes",
            "--notes-identity",
            "muse/new-1",
            "--ttl",
            "3600",
        ],
        "",
    )
    .unwrap();
    run_with(&url, &s, &["enroll-tokens"], "").unwrap();
    run_with(&url, &s, &["revoke-enroll", "--token-hash", "ab12"], "").unwrap();
    run_with(&url, &s, &["revoke-enroll", "--agent", "new-1"], "").unwrap();
    run_with(
        &url,
        &s,
        &[
            "mint-credential",
            "--agent",
            "pm-stack",
            "--scopes",
            "send,read",
            "--addr",
            "service:pm-stack",
        ],
        "",
    )
    .unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("/api/{OWNER}/enroll-tokens"));
    assert_eq!(
        seen[0].body.as_ref().unwrap(),
        &json!({"agent": "new-1", "kind": "muse", "groups": ["muse", "code-lanes"], "notes_identity": "muse/new-1", "ttl_seconds": 3600})
    );
    assert_eq!(seen[1].method, "GET");
    assert_eq!(seen[1].url, format!("/api/{OWNER}/enroll-tokens"));
    assert_eq!(seen[2].url, format!("/api/{OWNER}/enroll-tokens/revoke"));
    assert_eq!(
        seen[2].body.as_ref().unwrap(),
        &json!({"token_hash": "ab12"})
    );
    assert_eq!(seen[3].url, format!("/api/{OWNER}/enroll-tokens/revoke"));
    assert_eq!(seen[3].body.as_ref().unwrap(), &json!({"agent": "new-1"}));
    assert_eq!(seen[4].url, format!("/api/{OWNER}/fleet/credentials/mint"));
    assert_eq!(
        seen[4].body.as_ref().unwrap(),
        &json!({"agent": "pm-stack", "scopes": ["send", "read"], "addr": "service:pm-stack"})
    );
}

#[test]
fn http_errors_exit_4_and_never_show_the_token() {
    let (url, _) = fake_bridge(|_, path, _| {
        if path.contains("/send") {
            (
                400,
                r#"{"error":"no registered agent is in group muse"}"#.into(),
            )
        } else {
            // A misbehaving proxy echoing the request path back.
            (502, format!("bad gateway for {path}"))
        }
    });
    let s = secrets(&[(DEFAULT_BRIDGE_OWNER_TOKEN_NAME, OWNER)]);
    let e = run_with(&url, &s, &["send", "--to", "group:muse", "x"], "").unwrap_err();
    assert_eq!(e.exit_code(), EXIT_HTTP);
    assert!(e.to_string().contains("no registered agent"), "{e}");
    let e = run_with(&url, &s, &["agents"], "").unwrap_err();
    assert_eq!(e.exit_code(), EXIT_HTTP);
    let text = e.to_string();
    assert!(!text.contains(OWNER), "{text}");
    assert!(text.contains("<token>"), "{text}");
}

#[test]
fn not_found_explains_itself() {
    let (url, _) = fake_bridge(|_, _, _| (404, "Not found".into()));
    let s = secrets(&[(DEFAULT_BRIDGE_FLEET_TOKEN_NAME, "f".repeat(40).as_str())]);
    let e = run_with(&url, &s, &["inbox", "--agent", "a"], "").unwrap_err();
    assert_eq!(e.exit_code(), EXIT_HTTP);
    assert!(e.to_string().contains("wrong token"), "{e}");
}

#[test]
fn transport_failure_exits_3_without_the_token() {
    // Nothing listens on port 9 here; the connect fails.
    let s = secrets(&[(DEFAULT_BRIDGE_OWNER_TOKEN_NAME, OWNER)]);
    let e = run_with("http://127.0.0.1:9", &s, &["agents"], "").unwrap_err();
    assert_eq!(e.exit_code(), EXIT_TRANSPORT);
    assert!(!e.to_string().contains(OWNER), "{e}");
}

#[test]
fn health_needs_no_token() {
    let (url, seen) = fake_bridge(|_, _, _| (200, r#"{"ok":true}"#.into()));
    let out = run_with(&url, &secrets(&[]), &["health"], "").unwrap();
    assert_eq!(out["ok"], true);
    assert_eq!(seen.lock().unwrap()[0].url, "/healthz");
}

#[test]
fn post_can_address_any_connector_or_agent() {
    let (url, seen) = fake_bridge(|_, _, _| (201, r#"{"messages":[]}"#.into()));
    let s = secrets(&[(DEFAULT_BRIDGE_FLEET_TOKEN_NAME, "f".repeat(40).as_str())]);
    run_with(
        &url,
        &s,
        &[
            "post",
            "--agent",
            "muse-1",
            "--to",
            "claude,agent:muse-2",
            "--to",
            "group:laptop",
            "--instruction",
            "rerun the check",
        ],
        "",
    )
    .unwrap();
    run_with(
        &url,
        &s,
        &[
            "post",
            "--agent",
            "muse-1",
            "--to",
            "chatgpt",
            "--reply-to",
            "9",
            "done",
        ],
        "",
    )
    .unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].url, format!("/fleet/{}/send", "f".repeat(40)));
    assert_eq!(
        seen[0].body.as_ref().unwrap(),
        &json!({
            "agent": "muse-1",
            "body": "rerun the check",
            "to": ["claude", "agent:muse-2", "group:laptop"],
            "kind": "instruction"
        })
    );
    assert_eq!(
        seen[1].body.as_ref().unwrap(),
        &json!({"agent": "muse-1", "body": "done", "to": ["chatgpt"], "reply_to": 9})
    );
}

// ---- #14370: profile-recorded credential names, exchange, notes ----

#[test]
fn settings_take_credential_names_from_the_agent_profile() {
    let dir = std::env::temp_dir().join(format!("fleet-bridge-profile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let profile = dir.join("agent.json");
    std::fs::write(
        &profile,
        r#"{"name":"new-1","kind":"muse","groups":["muse"],"fleet_token_name":"bridge.new-1-fleet-token","notes_token_name":"bridge.new-1-token"}"#,
    )
    .unwrap();
    let env = |k: &str| match k {
        "FLEET_AGENT_PROFILE" => Some(profile.to_str().unwrap().to_string()),
        _ => None,
    };
    let config = Config::default();
    let s = Settings::resolve(&config, &env).unwrap();
    assert_eq!(s.agent.as_deref(), Some("new-1"));
    assert_eq!(s.fleet_token_name, "bridge.new-1-fleet-token");
    assert_eq!(s.notes_token_name.as_deref(), Some("bridge.new-1-token"));
    // Explicit config beats the profile, and the notes name derives
    // from the agent name when nothing records one.
    let mut config = Config::default();
    config.bridge.fleet_token_name = Some("custom.fleet".into());
    let s = Settings::resolve(&config, &env).unwrap();
    assert_eq!(s.fleet_token_name, "custom.fleet");
    std::fs::write(&profile, r#"{"name":"new-2","kind":"muse","groups":[]}"#).unwrap();
    let s = Settings::resolve(&Config::default(), &env).unwrap();
    assert_eq!(s.fleet_token_name, DEFAULT_BRIDGE_FLEET_TOKEN_NAME);
    assert_eq!(s.notes_token_name.as_deref(), Some("bridge.new-2-token"));
    std::fs::remove_dir_all(&dir).ok();
}

const FIXTURE_FLEET_CRED: &str = "fixtureFleetCredValue00000000000000000001";
const FIXTURE_NOTES_TOKEN: &str = "fixtureNotesTokenValue0000000000000000002";
const FIXTURE_BOOTSTRAP: &str = "fixtureBootstrapTokenValue00000000000003";

fn exchange_bridge(status: u16) -> (String, Arc<Mutex<Vec<Seen>>>) {
    fake_bridge(move |_, path, _| {
        if path == "/enroll/exchange" && status == 201 {
            (
                201,
                json!({
                    "agent": "new-1",
                    "kind": "muse",
                    "groups": ["muse"],
                    "notes_identity": "muse/new-1",
                    "fleet_token": FIXTURE_FLEET_CRED,
                    "notes_token": FIXTURE_NOTES_TOKEN,
                })
                .to_string(),
            )
        } else {
            (status, "Not found".into())
        }
    })
}

#[test]
fn exchange_stores_credentials_by_name_and_prints_no_value() {
    let (url, seen) = exchange_bridge(201);
    let s = secrets(&[]);
    let out = run_with(
        &url,
        &s,
        &[
            "exchange",
            "--agent",
            "New-1",
            "--bootstrap",
            FIXTURE_BOOTSTRAP,
        ],
        "",
    )
    .unwrap();
    assert_eq!(out["agent"], "new-1");
    assert_eq!(out["kind"], "muse");
    assert_eq!(out["groups"], json!(["muse"]));
    assert_eq!(out["notes_identity"], "muse/new-1");
    assert_eq!(out["fleet_token_name"], "bridge.new-1-fleet-token");
    assert_eq!(out["notes_token_name"], "bridge.new-1-token");
    let printed = out.to_string();
    for value in [FIXTURE_FLEET_CRED, FIXTURE_NOTES_TOKEN, FIXTURE_BOOTSTRAP] {
        assert!(
            !printed.contains(value),
            "output must not carry a credential value"
        );
    }
    let stored = s.stored.borrow();
    assert_eq!(
        stored.get("bridge.new-1-fleet-token").map(String::as_str),
        Some(FIXTURE_FLEET_CRED)
    );
    assert_eq!(
        stored.get("bridge.new-1-token").map(String::as_str),
        Some(FIXTURE_NOTES_TOKEN)
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].url, "/enroll/exchange");
    assert_eq!(
        seen[0].body.as_ref().unwrap(),
        &json!({ "bootstrap_token": FIXTURE_BOOTSTRAP, "agent": "new-1" })
    );
}

#[test]
fn exchange_refusal_stores_nothing_and_says_why() {
    let (url, _) = exchange_bridge(404);
    let s = secrets(&[]);
    let e = run_with(
        &url,
        &s,
        &[
            "exchange",
            "--agent",
            "new-1",
            "--bootstrap",
            FIXTURE_BOOTSTRAP,
        ],
        "",
    )
    .unwrap_err();
    assert_eq!(e.exit_code(), EXIT_HTTP);
    let text = e.to_string();
    assert!(text.contains("bootstrap"), "{text}");
    assert!(
        !text.contains(FIXTURE_BOOTSTRAP),
        "the token is scrubbed: {text}"
    );
    assert!(
        s.stored.borrow().is_empty(),
        "a refused exchange stores nothing"
    );
}

#[test]
fn notes_verbs_carry_kind_and_reply_to_under_a_bearer_token() {
    let (url, seen) = fake_bridge(|_, _, _| (201, r#"{"delivered":1}"#.into()));
    let s = secrets(&[("bridge.new-1-token", FIXTURE_NOTES_TOKEN)]);
    let mut st = settings(&url);
    st.agent = Some("new-1".into());
    st.notes_token_name = Some("bridge.new-1-token".into());
    let mut input: &[u8] = b"";
    let sleep = |_: Duration| {};
    let mut ctx = Ctx {
        settings: st,
        secrets: &s,
        stdin: &mut input,
        sleep: &sleep,
    };
    execute(
        &mut ctx,
        &sv(&[
            "notes",
            "send",
            "--to",
            "muse/*",
            "--kind",
            "instruction",
            "do the thing",
        ]),
    )
    .unwrap();
    // A reply names only the note id: no --to, and no sender field
    // anywhere in the payload — the worker routes it store-side.
    execute(&mut ctx, &sv(&["notes", "send", "--reply-to", "7", "done"])).unwrap();
    execute(&mut ctx, &sv(&["notes", "inbox"])).unwrap();
    execute(&mut ctx, &sv(&["notes", "ack", "3", "4"])).unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].url, "/notes/send");
    assert_eq!(
        seen[0].body.as_ref().unwrap(),
        &json!({ "note": "do the thing", "kind": "instruction", "to": "muse/*" })
    );
    assert_eq!(
        seen[1].body.as_ref().unwrap(),
        &json!({ "note": "done", "kind": "note", "reply_to": 7 })
    );
    assert_eq!(seen[2].url, "/notes/inbox");
    assert_eq!(seen[2].method, "GET");
    assert_eq!(seen[3].body.as_ref().unwrap(), &json!({ "ids": [3, 4] }));
    for req in seen.iter() {
        assert_eq!(
            req.auth.as_deref(),
            Some(format!("Bearer {FIXTURE_NOTES_TOKEN}").as_str()),
            "notes calls authenticate with the identity's own token"
        );
    }
}

#[test]
fn notes_verbs_name_the_missing_token() {
    let s = secrets(&[]);
    let mut st = settings("http://127.0.0.1:9");
    st.agent = Some("new-1".into());
    st.notes_token_name = Some("bridge.new-1-token".into());
    let mut input: &[u8] = b"";
    let sleep = |_: Duration| {};
    let mut ctx = Ctx {
        settings: st,
        secrets: &s,
        stdin: &mut input,
        sleep: &sleep,
    };
    for verb in ["inbox", "watch"] {
        let e = execute(&mut ctx, &sv(&["notes", verb])).unwrap_err();
        assert_eq!(e.exit_code(), EXIT_TRANSPORT, "{verb}");
        assert!(e.to_string().contains("bridge.new-1-token"), "{verb}: {e}");
    }
    // Usage errors in the family exit 2 before any token is needed.
    for args in [
        vec!["notes"],
        vec!["notes", "bogus"],
        vec!["notes", "send", "x"],
        vec!["notes", "send", "--to", "muse/x", "--kind", "shout", "x"],
        vec!["notes", "ack"],
        vec!["notes", "ack", "0"],
        vec!["notes", "get"],
        vec!["notes", "get", "muse/new-1", "extra"],
        vec!["notes", "get", "muse/*"],
        vec!["notes", "get", "muse"],
        vec!["notes", "get", "muse/new-1/x"],
        vec!["notes", "get", "notes/inbox"],
        vec!["notes", "get", "api/x"],
        vec!["notes", "get", "-muse/x"],
        vec!["notes", "watch", "extra"],
        vec!["notes", "watch", "--wait", "999999"],
        vec!["notes", "watch", "--interval", "soon"],
    ] {
        let e = execute(&mut ctx, &sv(&args)).unwrap_err();
        assert_eq!(e.exit_code(), EXIT_USAGE, "{args:?}: {e}");
    }
}

#[test]
fn notes_get_reads_the_public_signal_without_a_token() {
    let (url, seen) = fake_bridge(|_, path, _| match path {
        "/muse/new-1" => (200, "6f1c0d3e-0000-4000-8000-000000000001\n".into()),
        _ => (404, "not found".into()),
    });
    let s = secrets(&[]);
    let got = run_with(&url, &s, &["notes", "get", "Muse/New-1"], "").unwrap();
    assert_eq!(
        got,
        json!({ "identity": "muse/new-1", "signal": "6f1c0d3e-0000-4000-8000-000000000001" })
    );
    let e = run_with(&url, &s, &["notes", "get", "muse/gone"], "").unwrap_err();
    assert_eq!(e.exit_code(), EXIT_HTTP);
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].url, "/muse/new-1");
    assert!(seen.iter().all(|r| r.auth.is_none()), "get sends no token");
}

/// A Ctx for `notes watch` whose sleep records each pause.
fn watch_ctx<'a>(
    url: &str,
    s: &'a MapSecrets,
    input: &'a mut dyn Read,
    sleep: &'a dyn Fn(Duration),
) -> Ctx<'a> {
    let mut st = settings(url);
    st.agent = Some("new-1".into());
    st.notes_token_name = Some("bridge.new-1-token".into());
    Ctx {
        settings: st,
        secrets: s,
        stdin: input,
        sleep,
    }
}

#[test]
fn notes_watch_returns_waiting_notes_at_once() {
    let (url, seen) = fake_bridge(|_, _, _| {
        (
            200,
            r#"{"identity":"muse/new-1","notes":[{"id":4,"note":"hi"}]}"#.into(),
        )
    });
    let s = secrets(&[("bridge.new-1-token", FIXTURE_NOTES_TOKEN)]);
    let paused = std::cell::RefCell::new(Vec::new());
    let sleep = |d: Duration| paused.borrow_mut().push(d.as_secs());
    let mut input: &[u8] = b"";
    let mut ctx = watch_ctx(&url, &s, &mut input, &sleep);
    let got = execute(&mut ctx, &sv(&["notes", "watch"])).unwrap();
    assert_eq!(got["woke"], json!("pending"));
    assert_eq!(got["waited"], json!(0));
    assert_eq!(got["notes"][0]["id"], json!(4));
    assert!(paused.borrow().is_empty());
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].url, "/notes/inbox");
}

#[test]
fn notes_watch_reads_the_inbox_only_when_the_signal_moves() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let signals = Arc::new(AtomicUsize::new(0));
    let inboxes = Arc::new(AtomicUsize::new(0));
    let (sig, inb) = (signals.clone(), inboxes.clone());
    let (url, seen) = fake_bridge(move |_, path, _| match path {
        "/notes/inbox" => {
            let n = inb.fetch_add(1, Ordering::SeqCst);
            let body = if n < 2 {
                r#"{"identity":"muse/new-1","notes":[]}"#
            } else {
                r#"{"identity":"muse/new-1","notes":[{"id":9,"note":"go"}]}"#
            };
            (200, body.into())
        }
        "/muse/new-1" => {
            // a, a, b (a rotation: the inbox is still empty), b, c.
            let n = sig.fetch_add(1, Ordering::SeqCst);
            let v = ["a", "a", "b", "b", "c"][n.min(4)];
            (200, format!("{v}\n"))
        }
        _ => (404, "{}".into()),
    });
    let s = secrets(&[("bridge.new-1-token", FIXTURE_NOTES_TOKEN)]);
    let paused = std::cell::RefCell::new(Vec::new());
    let sleep = |d: Duration| paused.borrow_mut().push(d.as_secs());
    let mut input: &[u8] = b"";
    let mut ctx = watch_ctx(&url, &s, &mut input, &sleep);
    let got = execute(
        &mut ctx,
        &sv(&["notes", "watch", "--wait", "600", "--interval", "15"]),
    )
    .unwrap();
    assert_eq!(got["woke"], json!("signal"));
    assert_eq!(got["notes"][0]["id"], json!(9));
    assert_eq!(*paused.borrow(), vec![15, 15, 15, 15]);
    assert_eq!(inboxes.load(Ordering::SeqCst), 3, "initial, rotation, note");
    assert_eq!(signals.load(Ordering::SeqCst), 5);
    let seen = seen.lock().unwrap();
    for req in seen.iter() {
        if req.url == "/notes/inbox" {
            assert_eq!(
                req.auth.as_deref(),
                Some(format!("Bearer {FIXTURE_NOTES_TOKEN}").as_str())
            );
        } else {
            assert!(req.auth.is_none(), "the signal is read without a token");
        }
    }
}

#[test]
fn notes_watch_gives_up_empty_after_wait() {
    let (url, _seen) = fake_bridge(|_, path, _| match path {
        "/notes/inbox" => (200, r#"{"identity":"muse/new-1","notes":[]}"#.into()),
        _ => (200, "same\n".into()),
    });
    let s = secrets(&[("bridge.new-1-token", FIXTURE_NOTES_TOKEN)]);
    let paused = std::cell::RefCell::new(Vec::new());
    let sleep = |d: Duration| paused.borrow_mut().push(d.as_secs());
    let mut input: &[u8] = b"";
    let mut ctx = watch_ctx(&url, &s, &mut input, &sleep);
    let got = execute(
        &mut ctx,
        &sv(&["notes", "watch", "--wait", "20", "--interval", "15"]),
    )
    .unwrap();
    assert_eq!(got["woke"], Value::Null);
    assert_eq!(got["waited"], json!(20));
    assert_eq!(got["notes"], json!([]));
    assert_eq!(
        *paused.borrow(),
        vec![15, 5],
        "the last pause stops at --wait"
    );
    let got = execute(&mut ctx, &sv(&["notes", "watch", "--wait", "0"])).unwrap();
    assert_eq!(got["woke"], Value::Null);
}

// ---- ScopedSecrets (#10358 S1, design record §6 and D11) ----

/// A store that records every name it is asked for and serves what was
/// written to it.
#[derive(Default)]
struct RecordingSecrets {
    asked: std::cell::RefCell<Vec<String>>,
    stored: std::cell::RefCell<HashMap<String, String>>,
}

impl Secrets for RecordingSecrets {
    fn resolve(&self, name: &str) -> Result<String, String> {
        self.asked.borrow_mut().push(name.to_string());
        self.stored
            .borrow()
            .get(name)
            .cloned()
            .ok_or_else(|| format!("no secret named {name}"))
    }

    fn store(&self, name: &str, value: &str) -> Result<(), String> {
        self.asked.borrow_mut().push(name.to_string());
        self.stored
            .borrow_mut()
            .insert(name.to_string(), value.to_string());
        Ok(())
    }
}

const SCOPE_MATRIX: [(&str, Option<&str>, Option<&str>); 9] = [
    ("alpha", None, Some("alpha")),
    ("42.alpha", None, Some("42.alpha")),
    ("alpha", Some("42"), Some("42.alpha")),
    ("42.alpha", Some("42"), Some("42.alpha")),
    ("42.x", Some("42"), Some("42.x")),
    ("42.42.x", Some("42"), Some("42.42.x")),
    ("42", Some("42"), Some("42.42")),
    ("42.", Some("42"), None),
    ("420.x", Some("42"), Some("42.420.x")),
];

fn scope_for(prefix: Option<&str>) -> fleet_secrets::Scope {
    match prefix {
        Some(p) => fleet_secrets::Scope::prefixed(p).unwrap(),
        None => fleet_secrets::Scope::none(),
    }
}

#[test]
fn scoped_secrets_store_and_resolve_the_full_name() {
    for (row, (name, prefix, stored)) in SCOPE_MATRIX.iter().enumerate() {
        let scoped = ScopedSecrets::new(scope_for(*prefix), RecordingSecrets::default());
        let wrote = scoped.store(name, "fixture-value-not-a-secret");
        let read = scoped.resolve(name);
        let asked = scoped.inner.asked.borrow().clone();
        match stored {
            Some(want) => {
                assert_eq!(wrote, Ok(()), "row {}", row + 1);
                // A scoped bridge reads what it wrote.
                assert_eq!(
                    read.as_deref(),
                    Ok("fixture-value-not-a-secret"),
                    "row {}",
                    row + 1
                );
                assert_eq!(asked, [*want, *want], "row {}", row + 1);
            }
            None => {
                assert!(wrote.is_err(), "row {}", row + 1);
                assert!(read.is_err(), "row {}", row + 1);
                assert!(asked.is_empty(), "row {}: inner never asked", row + 1);
            }
        }
    }
}
