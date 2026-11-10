//! Contract tests for the `fleet serve search` role (#5604).
//!
//! The role is the Atlas service rehomed, so its contract is the
//! service's job protocol. These tests drive the role's own wiring
//! ([`fleet_search::serve::build_engine`] into the service's
//! `start_server`) over real loopback HTTP against a stub backend, and
//! name their oracle each time:
//!
//!   * request handling — the protocol itself (statuses and shapes the
//!     atlas-proto contract fixes);
//!   * dedupe — a counting backend (an identical *effective* request
//!     executes the backend exactly once; a different effective
//!     ceiling never coalesces);
//!   * recovery listing — set semantics over client identities;
//!   * shared embedder — a counting embedder (one instance serves every
//!     job; `/health` reports its observable state);
//!   * read-only posture — a source-level invariant (the role's only
//!     backend constructor is the read-only Turso chain);
//!   * argument surface — properties (never panics; round-trips).
#![cfg(feature = "serve")]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use atlas_core::types::{BranchHit, BranchPools, ChunkDetail, MessageRow, NeighborRow};
use atlas_core::{BranchQuery, Error as CoreError, Filters, QueryEmbedder, SearchBackend};
use fleet_search::serve::{self, Parsed, ServeArgs};
use fleet_search::server::{
    ClientConfig, RuntimeLimits, ServeSettings, ServerHandle, start_server,
};
use proptest::prelude::*;
use serde_json::{Value as Json, json};

const PERSONAL_TOKEN: &str = "contract-personal-token";
const SECRET_TOKEN: &str = "contract-secret-token";

// ------------------------------------------------------------- stubs --

struct StubBackend {
    calls: Arc<AtomicUsize>,
}

fn detail(id: i64, text: &str, sensitivity: &str) -> ChunkDetail {
    ChunkDetail {
        chunk_id: id,
        text: text.to_string(),
        chunk_idx: 0,
        message_id: format!("m{id}"),
        m_role: Some("user".to_string()),
        session_id: format!("s{id}"),
        s_title: Some("stub".to_string()),
        s_client: Some("stub".to_string()),
        s_created_at: Some("2026-11-06T00:00:00Z".to_string()),
        l_kind: Some("chat".to_string()),
        l_topics: Some("[]".to_string()),
        l_projects: Some("[]".to_string()),
        eff_sensitivity: Some(sensitivity.to_string()),
    }
}

impl SearchBackend for StubBackend {
    fn name(&self) -> &'static str {
        "turso"
    }
    fn vec_table(&self) -> &str {
        atlas_core::DEFAULT_VEC_TABLE
    }
    fn notes(&self) -> Vec<String> {
        Vec::new()
    }
    fn branch_pools(&self, q: &BranchQuery, filters: &Filters) -> Result<BranchPools, CoreError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        // A secret-bearing chunk exists only above the personal ceiling.
        let ids: Vec<i64> = if filters.max_sensitivity == atlas_core::Sensitivity::SecretBearing {
            vec![1, 3]
        } else {
            vec![1, 2]
        };
        let mut pools = BranchPools::default();
        if q.match_expr.is_some() {
            pools.fts = ids
                .iter()
                .map(|id| BranchHit {
                    chunk_id: *id,
                    dist: None,
                })
                .collect();
        }
        if q.query_vector.is_some() {
            pools.vector = ids
                .iter()
                .enumerate()
                .map(|(i, id)| BranchHit {
                    chunk_id: *id,
                    dist: Some(0.01 * (i as f64 + 1.0)),
                })
                .collect();
        }
        Ok(pools)
    }
    fn chunk_details(&self, ids: &[i64]) -> Result<Vec<ChunkDetail>, CoreError> {
        Ok(ids
            .iter()
            .filter_map(|id| match id {
                1 => Some(detail(1, "alpha one", "personal")),
                2 => Some(detail(2, "alpha two", "personal")),
                3 => Some(detail(3, "secret canary", "secret-bearing")),
                _ => None,
            })
            .collect())
    }
    fn neighbors(&self, _m: &str, _i: i64, _n: i64) -> Result<Vec<NeighborRow>, CoreError> {
        Ok(Vec::new())
    }
    fn message(&self, _m: &str) -> Result<Option<MessageRow>, CoreError> {
        Ok(None)
    }
}

/// Counts embeds; the role must route every job through ONE instance.
struct CountingEmbedder {
    embeds: Arc<AtomicUsize>,
}

impl QueryEmbedder for CountingEmbedder {
    fn embed_query(&self, _text: &str, _trunc: usize) -> Result<Vec<f32>, CoreError> {
        self.embeds.fetch_add(1, Ordering::SeqCst);
        Ok(vec![1.0, 0.0])
    }
}

struct Harness {
    handle: ServerHandle,
    backend_calls: Arc<AtomicUsize>,
    embeds: Arc<AtomicUsize>,
}

fn start() -> Harness {
    let backend_calls = Arc::new(AtomicUsize::new(0));
    let embeds = Arc::new(AtomicUsize::new(0));
    let engine = serve::build_engine(
        StubBackend {
            calls: backend_calls.clone(),
        },
        Arc::new(CountingEmbedder {
            embeds: embeds.clone(),
        }),
    );
    let settings = ServeSettings {
        listen: "127.0.0.1:0".parse().unwrap(),
        clients: vec![
            ClientConfig {
                name: "sandbox-scripts".to_string(),
                token: PERSONAL_TOKEN.to_string(),
                ceiling_cap: atlas_proto_sensitivity("personal"),
            },
            ClientConfig {
                name: "drew-cli".to_string(),
                token: SECRET_TOKEN.to_string(),
                ceiling_cap: atlas_proto_sensitivity("secret-bearing"),
            },
        ],
        vec_table: atlas_core::DEFAULT_VEC_TABLE.to_string(),
        limits: RuntimeLimits::default(),
    };
    let handle = start_server(settings, engine).expect("service starts on loopback");
    Harness {
        handle,
        backend_calls,
        embeds,
    }
}

fn atlas_proto_sensitivity(name: &str) -> atlas_proto::Sensitivity {
    atlas_proto::Sensitivity::from_name(name).unwrap()
}

// -------------------------------------------------------------- http --

struct Resp {
    status: u16,
    body: Json,
}

fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Json>,
) -> Resp {
    let mut stream = TcpStream::connect(addr).unwrap();
    let payload = body.map(|b| b.to_string()).unwrap_or_default();
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        payload.len()
    );
    if let Some(t) = token {
        req.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(&payload);
    stream.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    let status = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    Resp {
        status,
        body: serde_json::from_str(body).unwrap_or(Json::Null),
    }
}

fn submit(addr: SocketAddr, token: &str, body: Json) -> Resp {
    http(addr, "POST", "/v1/search/jobs", Some(token), Some(body))
}

fn wait_done(addr: SocketAddr, id: &str, job_token: &str) {
    for _ in 0..500 {
        let r = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{id}"),
            Some(job_token),
            None,
        );
        assert_eq!(r.status, 200, "poll: {}", r.body);
        match r.body["status"].as_str().unwrap() {
            "done" => return,
            "failed" | "expired" => panic!("job ended {}", r.body),
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    panic!("job {id} did not finish");
}

fn ids(page: &Json) -> Vec<i64> {
    page["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["chunk_id"].as_i64().unwrap())
        .collect()
}

fn run_job(addr: SocketAddr, token: &str, body: Json) -> (String, String) {
    let created = submit(addr, token, body);
    assert_eq!(created.status, 202, "{}", created.body);
    let id = created.body["job_id"].as_str().unwrap().to_string();
    let jt = created.body["job_token"].as_str().unwrap().to_string();
    wait_done(addr, &id, &jt);
    (id, jt)
}

// ------------------------------------------------- request handling --

#[test]
fn health_is_public_and_reports_the_shared_embedder_state() {
    let h = start();
    let addr = h.handle.addr;
    let r = http(addr, "GET", "/health", None, None);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.body["status"], "ok");
    assert_eq!(r.body["embedder_state"], "cold", "no vector query yet");
    // One vector-bearing job warms the (single) embedder.
    run_job(addr, PERSONAL_TOKEN, json!({"query": "alpha"}));
    let r = http(addr, "GET", "/health", None, None);
    assert_eq!(r.body["embedder_state"], "ready");
}

#[test]
fn submit_poll_and_page_results_with_the_protocol_shapes() {
    let h = start();
    let addr = h.handle.addr;
    let created = submit(addr, PERSONAL_TOKEN, json!({"query": "alpha"}));
    assert_eq!(created.status, 202, "{}", created.body);
    for key in ["job_id", "job_token", "status"] {
        assert!(created.body.get(key).is_some(), "202 body lacks {key}");
    }
    let id = created.body["job_id"].as_str().unwrap();
    let jt = created.body["job_token"].as_str().unwrap();
    wait_done(addr, id, jt);
    let page = http(
        addr,
        "GET",
        &format!("/v1/search/jobs/{id}/results"),
        Some(jt),
        None,
    );
    assert_eq!(page.status, 200, "{}", page.body);
    assert_eq!(ids(&page.body), vec![1, 2]);
    // Deleting releases the job: the capability then 404s.
    let del = http(
        addr,
        "DELETE",
        &format!("/v1/search/jobs/{id}"),
        Some(jt),
        None,
    );
    assert_eq!(del.status, 200, "{}", del.body);
    let gone = http(
        addr,
        "GET",
        &format!("/v1/search/jobs/{id}"),
        Some(jt),
        None,
    );
    assert_eq!(gone.status, 404);
}

#[test]
fn unauthenticated_and_malformed_requests_are_refused_without_running_the_backend() {
    let h = start();
    let addr = h.handle.addr;
    assert_eq!(
        submit(addr, "not-a-client-token", json!({"query": "alpha"})).status,
        401
    );
    let anon = http(
        addr,
        "POST",
        "/v1/search/jobs",
        None,
        Some(json!({"query": "alpha"})),
    );
    assert_eq!(anon.status, 401);
    // Validation: k below 1 and pool above the design maximum are 400s.
    assert_eq!(
        submit(addr, PERSONAL_TOKEN, json!({"query": "alpha", "k": 0})).status,
        400
    );
    assert_eq!(
        submit(addr, PERSONAL_TOKEN, json!({"query": "alpha", "pool": 401})).status,
        400
    );
    assert_eq!(h.backend_calls.load(Ordering::SeqCst), 0);
    assert_eq!(h.embeds.load(Ordering::SeqCst), 0);
}

#[test]
fn the_server_clamps_the_requested_ceiling_to_the_clients_cap() {
    let h = start();
    let addr = h.handle.addr;
    // sandbox-scripts is capped at personal: asking for secret-bearing
    // is clamped, and the secret chunk stays invisible.
    let (id, jt) = run_job(
        addr,
        PERSONAL_TOKEN,
        json!({"query": "secret", "ceiling": "secret-bearing"}),
    );
    let status = http(
        addr,
        "GET",
        &format!("/v1/search/jobs/{id}"),
        Some(&jt),
        None,
    );
    assert_eq!(
        status.body["effective_ceiling"], "personal",
        "{}",
        status.body
    );
    let page = http(
        addr,
        "GET",
        &format!("/v1/search/jobs/{id}/results"),
        Some(&jt),
        None,
    );
    assert!(!ids(&page.body).contains(&3), "{}", page.body);
    // drew-cli may see it.
    let (id2, jt2) = run_job(
        addr,
        SECRET_TOKEN,
        json!({"query": "secret", "ceiling": "secret-bearing"}),
    );
    let page2 = http(
        addr,
        "GET",
        &format!("/v1/search/jobs/{id2}/results"),
        Some(&jt2),
        None,
    );
    assert!(ids(&page2.body).contains(&3), "{}", page2.body);
}

// ------------------------------------------------------------ dedupe --

#[test]
fn identical_effective_requests_execute_the_backend_once() {
    let h = start();
    let addr = h.handle.addr;
    let (id1, jt1) = run_job(addr, PERSONAL_TOKEN, json!({"query": "alpha"}));
    let after_first = h.backend_calls.load(Ordering::SeqCst);
    assert_eq!(
        after_first, 2,
        "one hybrid computation = FTS + vector branch"
    );
    let second = submit(addr, PERSONAL_TOKEN, json!({"query": "alpha"}));
    assert_eq!(second.status, 202);
    let id2 = second.body["job_id"].as_str().unwrap().to_string();
    assert_ne!(id1, id2, "each submitter keeps its own job id");
    assert_ne!(jt1, second.body["job_token"].as_str().unwrap());
    assert_eq!(
        second.body["status"], "done",
        "coalesced onto the finished run"
    );
    assert_eq!(h.backend_calls.load(Ordering::SeqCst), after_first);
    assert_eq!(h.embeds.load(Ordering::SeqCst), 1, "one embed for both");
}

#[test]
fn dedupe_is_keyed_by_the_effective_request_not_the_spelled_one() {
    let h = start();
    let addr = h.handle.addr;
    // personal client asking for personal vs asking for secret-bearing:
    // different spellings, the SAME effective ceiling -> one execution.
    run_job(
        addr,
        PERSONAL_TOKEN,
        json!({"query": "alpha", "ceiling": "personal"}),
    );
    let first = h.backend_calls.load(Ordering::SeqCst);
    let again = submit(
        addr,
        PERSONAL_TOKEN,
        json!({"query": "alpha", "ceiling": "secret-bearing"}),
    );
    assert_eq!(again.status, 202);
    assert_eq!(again.body["status"], "done");
    assert_eq!(h.backend_calls.load(Ordering::SeqCst), first);
    // A client whose effective ceiling differs never shares the run.
    run_job(
        addr,
        SECRET_TOKEN,
        json!({"query": "alpha", "ceiling": "secret-bearing"}),
    );
    assert!(
        h.backend_calls.load(Ordering::SeqCst) > first,
        "a different effective ceiling must execute afresh"
    );
}

// --------------------------------------------------- recovery listing --

#[test]
fn recovery_listing_is_per_client_metadata_only() {
    let h = start();
    let addr = h.handle.addr;
    let (id, jt) = run_job(addr, PERSONAL_TOKEN, json!({"query": "alpha"}));
    // The owning client recovers its job from the client token alone.
    let mine = http(addr, "GET", "/v1/search/jobs", Some(PERSONAL_TOKEN), None);
    assert_eq!(mine.status, 200, "{}", mine.body);
    let jobs = mine.body["jobs"].as_array().unwrap();
    let own: Vec<&Json> = jobs.iter().filter(|j| j["job_id"] == id.as_str()).collect();
    assert_eq!(own.len(), 1, "{}", mine.body);
    assert_eq!(own[0]["status"], "done");
    assert_eq!(own[0]["query"], "alpha");
    for forbidden in ["results", "job_token", "text"] {
        assert!(own[0].get(forbidden).is_none(), "list leaks {forbidden}");
    }
    // Another client's list never contains it.
    let theirs = http(addr, "GET", "/v1/search/jobs", Some(SECRET_TOKEN), None);
    assert_eq!(theirs.status, 200);
    assert!(
        theirs.body["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|j| j["job_id"] != id.as_str())
    );
    // A job capability is not a client credential; neither is nothing.
    assert_eq!(
        http(addr, "GET", "/v1/search/jobs", Some(&jt), None).status,
        401
    );
    assert_eq!(http(addr, "GET", "/v1/search/jobs", None, None).status, 401);
    // And the recovered id + its capability still reads the ranking.
    let page = http(
        addr,
        "GET",
        &format!("/v1/search/jobs/{id}/results"),
        Some(&jt),
        None,
    );
    assert_eq!(ids(&page.body), vec![1, 2]);
}

// -------------------------------------------------- read-only posture --

#[test]
fn the_roles_only_backend_constructor_is_the_read_only_turso_chain() {
    // Oracle: a source-level invariant. The role has no write path
    // because atlas-core's backends issue only SELECTs AND this role
    // can only construct the read-only-token Turso backend: no local
    // backend, no explicit-token constructor, no --backend flag.
    let src = include_str!("../src/serve.rs");
    let code: String = src
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(code.contains("TursoBackend::new("));
    for forbidden in [
        "LocalBackend",
        "with_token(",
        "TURSO_HOST",
        "\"--backend\"",
        "\"--db\"",
    ] {
        assert!(
            !code.contains(forbidden),
            "serve.rs must not reference {forbidden}: the role stays read-only by construction"
        );
    }
}

// ------------------------------------------------------ arg surface --

fn s(list: &[&str]) -> Vec<String> {
    list.iter().map(|x| x.to_string()).collect()
}

#[test]
fn args_parse_both_flag_spellings_and_reject_the_rest() {
    assert_eq!(
        serve::parse_args(&[]),
        Ok(Parsed::Run(ServeArgs::default()))
    );
    assert_eq!(serve::parse_args(&s(&["--help"])), Ok(Parsed::Help));
    let want = ServeArgs {
        config: Some("/tmp/c.json".into()),
        listen: Some("127.0.0.1:19999".parse().unwrap()),
        job_ttl_ms: Some(5),
        hard_ttl_ms: Some(9),
    };
    assert_eq!(
        serve::parse_args(&s(&[
            "--config",
            "/tmp/c.json",
            "--listen=127.0.0.1:19999",
            "--job-ttl-ms",
            "5",
            "--hard-ttl-ms=9"
        ])),
        Ok(Parsed::Run(want))
    );
    for bad in [
        s(&["--bogus"]),
        s(&["stray"]),
        s(&["--listen"]),
        s(&["--listen", "not-an-addr"]),
        s(&["--job-ttl-ms", "-1"]),
        s(&["--backend", "local"]),
    ] {
        assert!(serve::parse_args(&bad).is_err(), "{bad:?} must be refused");
    }
}

proptest! {
    // Invariant: parse_args is total — any argv yields Ok or Err,
    // never a panic.
    #[test]
    fn parse_args_is_total(argv in proptest::collection::vec(".{0,24}", 0..8)) {
        let _ = serve::parse_args(&argv);
    }

    // Invariant: a generated valid flag set parses to exactly the
    // values it spelled (model = the generator's own fields).
    #[test]
    fn valid_flag_sets_round_trip(
        cfg in "[a-z/._-]{1,16}",
        port in 1024u16..65535,
        job in any::<u64>(),
        hard in any::<u64>(),
        inline in any::<bool>(),
    ) {
        let listen = format!("127.0.0.1:{port}");
        let argv = if inline {
            vec![format!("--config={cfg}"), format!("--listen={listen}"),
                 format!("--job-ttl-ms={job}"), format!("--hard-ttl-ms={hard}")]
        } else {
            vec!["--config".into(), cfg.clone(), "--listen".into(), listen.clone(),
                 "--job-ttl-ms".into(), job.to_string(), "--hard-ttl-ms".into(), hard.to_string()]
        };
        let want = ServeArgs {
            config: Some(cfg.into()),
            listen: Some(listen.parse().unwrap()),
            job_ttl_ms: Some(job),
            hard_ttl_ms: Some(hard),
        };
        prop_assert_eq!(serve::parse_args(&argv), Ok(Parsed::Run(want)));
    }
}
