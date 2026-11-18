//! F13.7 step 6 — end-to-end hardening for the first client-side
//! capability provider, `client.file.read` (design
//! `doc/2026-11-01_design_id789.md`, issue #5532).
//!
//! Steps 1–5 landed the pieces separately: the registry manifest
//! (step 2), the host broker (step 3), the shell's local
//! grant/disclosure adapter (step 4), and the file-read executor
//! plus the shipped shell's `ID_SHELL_FILE_READ_ROOT` opt-in
//! (step 5, PR #10925). This test assembles them the way the shipped
//! shell does — a live world host, the real registry manifest, the
//! real broker, a session running [`run_session_with_providers`],
//! and a [`ProviderAdapter`] answering through the session's
//! outbound channel — and exercises the whole path at once:
//!
//! - the happy path through BOTH gates (world ceiling + grant on
//!   the host, local grant + disclosure acknowledgment in the
//!   shell), with the result delivered as the ordinary journaled
//!   capability result event;
//! - the live refusal matrix (traversal, absolute path, symlink
//!   escape, missing file, directory, over-bound file) reaching the
//!   guest with the adapter's stable codes;
//! - the world gates refusing before any frame reaches the client;
//! - the default posture: no local opt-in means the provider is
//!   unavailable, and one session's offer never serves another
//!   session's request;
//! - the disclosure gate holding end-to-end even when an executor
//!   is registered through the raw adapter route;
//! - revocation at either gate refusing the next read.
//!
//! The audit assertions check the design's hard rule on both sides:
//! the host F12.6 records and the local adapter log carry decisions
//! (provider, outcome, stable codes, args digest) and never file
//! content, argument values, or local paths.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use id_native_shell::{
    FileReadExecutor, LocalBounds, ProviderAdapter, input_frame, run_session_with_providers,
};
use id_worlds::world::{
    WorldCore, WorldEvent, WorldEventKind, WorldHandle, WorldLimits, WorldProgram, WorldScopes,
};
use id_worlds::world_net::{WORLD_ALPN, WorldClient, WorldProtocol};
use id_worlds::world_session::WorldService;
use iroh::endpoint::{RelayMode, presets};
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointAddr, TransportAddr};
use serde_json::{Value, json};
use tokio::sync::mpsc;

const TIMEOUT: Duration = Duration::from_secs(30);
/// Distinctive file body: the audit assertions prove these bytes
/// (and the file's name and root path) never appear in either
/// audit trail even though the read itself succeeds.
const FILE_BODY: &[u8] = b"e2e file body for step 5532";

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);
/// The host's F12.6 audit records are crate-private values inside
/// id-worlds, but every record is also emitted publicly as a
/// tracing event on the `id::audit` target with the record's JSON
/// as its `audit_record` field — the same emission an operator's
/// telemetry collector consumes. The tests capture that stream
/// into one process-wide buffer and read back the windows around
/// their own world's records (each test hosts a uniquely named
/// world, so parallel tests never read each other's trail).
static AUDIT_CAPTURE: Mutex<Vec<u8>> = Mutex::new(Vec::new());
static AUDIT_INIT: std::sync::Once = std::sync::Once::new();

#[derive(Clone, Debug)]
struct AuditWriter;

impl std::io::Write for AuditWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        AUDIT_CAPTURE.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn ensure_audit_capture() {
    AUDIT_INIT.call_once(|| {
        let subscriber = tracing_subscriber::fmt::Subscriber::builder()
            .with_max_level(tracing::Level::INFO)
            .with_ansi(false)
            .with_writer(|| AuditWriter)
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
}

/// The captured audit text for one world: a window around each of
/// its records in the shared stream, wide enough to hold the whole
/// record JSON on either side of the `world_id` marker.
fn host_audit_text(world: &str) -> String {
    let captured = AUDIT_CAPTURE.lock().unwrap();
    let text = String::from_utf8_lossy(&captured);
    let marker = format!("\"world_id\":\"{world}\"");
    let mut out = String::new();
    for (idx, _) in text.match_indices(marker.as_str()) {
        let mut start = idx.saturating_sub(1500);
        while start > 0 && !text.is_char_boundary(start) {
            start -= 1;
        }
        let mut end = (idx + 1500).min(text.len());
        while end < text.len() && !text.is_char_boundary(end) {
            end += 1;
        }
        out.push_str(&text[start..end]);
        out.push('\n');
    }
    out
}

/// A unique temporary tree: `root/` is the user-selected file-read
/// root, and the base also holds an `outside.txt` sibling the root
/// must never serve.
struct TempTree {
    base: PathBuf,
    root: PathBuf,
}

fn temp_tree() -> TempTree {
    let unique = format!(
        "id-file-read-e2e-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    );
    let base = std::env::temp_dir().join(unique);
    let root = base.join("root");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("notes.txt"), FILE_BODY).unwrap();
    std::fs::write(root.join("big.bin"), vec![7_u8; 40_000]).unwrap();
    std::fs::write(base.join("outside.txt"), b"outside the root").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(base.join("outside.txt"), root.join("escape.txt")).unwrap();
    TempTree { base, root }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.base).ok();
    }
}

/// Guest program state shared with the test: the program turns each
/// input into one `client.file.read` want for the input text as the
/// path, and records every capability result event it hears.
#[derive(Default)]
struct ProgState {
    pending: Option<(String, String)>,
    results: Vec<Value>,
    counter: u32,
}

struct ReadProgram {
    state: Arc<Mutex<ProgState>>,
}

impl WorldProgram for ReadProgram {
    fn update(&mut self, event: &WorldEvent) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        match &event.kind {
            WorldEventKind::Input(bytes) => {
                state.counter += 1;
                let id = format!("read-{}", state.counter);
                let path = String::from_utf8_lossy(bytes).into_owned();
                state.pending = Some((id, path));
            }
            WorldEventKind::System { event } => {
                if let Ok(parsed) = serde_json::from_str::<Value>(event)
                    && parsed["cap"] == "result"
                {
                    if let Some((id, _)) = &state.pending
                        && parsed["id"] == *id
                    {
                        state.pending = None;
                    }
                    state.results.push(parsed);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn wants(&mut self) -> Result<Option<String>> {
        let state = self.state.lock().unwrap();
        Ok(state.pending.as_ref().map(|(id, path)| {
            json!({
                "v": 1,
                "requests": [{
                    "id": id,
                    "cap": "client.file.read",
                    "args": {"path": path},
                }],
            })
            .to_string()
        }))
    }
}

/// A loopback endpoint that uses no relay or discovery (mirrors
/// `tests/live_session.rs`).
async fn endpoint() -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap()
}

async fn serve(service: WorldService) -> (Router, EndpointAddr) {
    let ep = endpoint().await;
    let router = Router::builder(ep.clone())
        .accept(WORLD_ALPN, WorldProtocol::new(service))
        .spawn();
    let addr = EndpointAddr::from_parts(
        ep.id(),
        ep.bound_sockets().iter().map(|socket| {
            let ip = if socket.ip().is_unspecified() {
                std::net::Ipv4Addr::LOCALHOST.into()
            } else {
                socket.ip()
            };
            TransportAddr::Ip(std::net::SocketAddr::new(ip, socket.port()))
        }),
    );
    (router, addr)
}

/// One hosted world running the read program, served over loopback.
struct HostedWorld {
    world: WorldHandle,
    name: String,
    state: Arc<Mutex<ProgState>>,
    #[allow(dead_code)]
    router: Router,
    addr: EndpointAddr,
    client_ep: Endpoint,
}

async fn hosted_world(name: &str) -> HostedWorld {
    ensure_audit_capture();
    let state = Arc::new(Mutex::new(ProgState::default()));
    // The host's server ceiling is the outermost host gate: it
    // defaults to the default ceiling, which excludes this
    // provider, so the operator's `serve --world-allowed-caps`
    // list must name it before any world ceiling or grant can
    // reach it. Mirror that here: the default ceiling plus the
    // provider.
    let mut server_caps = id_worlds::world_caps::default_ceiling_names();
    server_caps.insert("client.file.read".to_owned());
    let limits = WorldLimits {
        server_caps,
        ..WorldLimits::default()
    };
    let core = WorldCore::new(name, limits).unwrap();
    let world = WorldHandle::spawn_with_program(
        core,
        Box::new(ReadProgram {
            state: state.clone(),
        }),
    );
    let service = WorldService::new(world.clone(), Some("admin".to_owned()));
    let (router, addr) = serve(service).await;
    let client_ep = endpoint().await;
    HostedWorld {
        world,
        name: name.to_owned(),
        state,
        router,
        addr,
        client_ep,
    }
}

impl HostedWorld {
    /// Open both world gates for `client.file.read`: name it in the
    /// world's ceiling and grant it.
    async fn open_world_gates(&self) {
        self.world
            .set_caps_ceiling(vec!["client.file.read".to_owned()])
            .await
            .unwrap();
        self.world
            .update_caps(vec!["client.file.read".to_owned()], Vec::new())
            .await
            .unwrap();
    }

    async fn wait_results(&self, count: usize) -> Vec<Value> {
        let state = self.state.clone();
        tokio::time::timeout(TIMEOUT, async move {
            loop {
                {
                    let state = state.lock().unwrap();
                    if state.results.len() >= count {
                        return state.results.clone();
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "timed out waiting for {count} results; got {:?}",
                self.state.lock().unwrap().results
            )
        })
    }

    /// This world's slice of the captured host audit stream (see
    /// the free `host_audit_text`): the text the audit assertions
    /// check for decision records — and for the absence of
    /// content, paths, and argument values.
    fn host_audit_text(&self) -> String {
        host_audit_text(&self.name)
    }
}

/// One shell session wired exactly as the shipped binary wires it
/// (`crates/id-native-shell/src/main.rs`): the adapter answers
/// validated `provider_request` frames from the session loop and
/// its replies go back on the session's shared outbound channel.
/// The adapter sits behind a mutex so the test can inspect the
/// local audit log and revoke mid-session; the shipped shell owns
/// it inside the provider task instead, with the same call order.
struct Shell {
    outbound: mpsc::Sender<Value>,
    /// The provider task's reply sender, in a slot `finish` can
    /// empty: the session ends when the outbound channel's last
    /// sender drops, so the provider task must not hold a clone
    /// of it past the test's own sender. (The shipped shell drops
    /// both together at app exit; a test needs the same effect
    /// mid-process.)
    reply_slot: Arc<Mutex<Option<mpsc::Sender<Value>>>>,
    adapter: Arc<Mutex<ProviderAdapter>>,
    task: tokio::task::JoinHandle<Result<id_native_shell::SessionState>>,
}

async fn start_shell(host: &HostedWorld, capability: &str, adapter: ProviderAdapter) -> Shell {
    let adapter = Arc::new(Mutex::new(adapter));
    let client = WorldClient::connect(&host.client_ep, host.addr.clone())
        .await
        .unwrap();
    let (outbound_tx, outbound_rx) = mpsc::channel(32);
    let (views_tx, _views_rx) = mpsc::unbounded_channel();
    let (provider_tx, mut provider_rx) = mpsc::unbounded_channel::<Value>();
    // The shipped shell passes no `--world`, so its adapter world
    // key is the empty string and the offer is computed for it.
    let offer = adapter.lock().unwrap().offer_frame("");
    let task_adapter = adapter.clone();
    let reply_slot = Arc::new(Mutex::new(Some(outbound_tx.clone())));
    let task_reply_slot = reply_slot.clone();
    let provider_task = tokio::spawn(async move {
        while let Some(request) = provider_rx.recv().await {
            let reply = task_adapter
                .lock()
                .unwrap()
                .handle_request("", &request, now_ms());
            let sender = task_reply_slot.lock().unwrap().clone();
            let Some(sender) = sender else { break };
            if sender.send(reply).await.is_err() {
                break;
            }
        }
    });
    let capability = capability.to_owned();
    let task = tokio::spawn(async move {
        let result = run_session_with_providers(
            client,
            None,
            &capability,
            None,
            outbound_rx,
            views_tx,
            provider_tx,
            offer,
        )
        .await;
        provider_task.abort();
        result
    });
    Shell {
        outbound: outbound_tx,
        reply_slot,
        adapter,
        task,
    }
}

impl Shell {
    async fn input(&self, path: &str) {
        self.outbound
            .send(input_frame(path.as_bytes()))
            .await
            .unwrap();
    }

    async fn finish(self) {
        drop(self.outbound);
        *self.reply_slot.lock().unwrap() = None;
        tokio::time::timeout(TIMEOUT, self.task)
            .await
            .expect("shell session did not end")
            .unwrap()
            .unwrap();
    }
}

/// An adapter opted in the way the shipped shell's
/// `ID_SHELL_FILE_READ_ROOT` opt-in does it: one `grant_file_read`
/// call with the disclosure acknowledged, under the design-default
/// local bounds.
fn opted_in_adapter(root: &PathBuf) -> ProviderAdapter {
    let mut adapter = ProviderAdapter::new();
    adapter
        .grant_file_read("", root.clone(), LocalBounds::DESIGN_DEFAULT, true)
        .unwrap();
    adapter
}

#[tokio::test]
async fn happy_path_reads_through_both_gates_and_audits_carry_no_content() {
    let tree = temp_tree();
    let host = hosted_world("e2e-happy").await;
    host.open_world_gates().await;
    let (_, ann) = host.world.issue("ann", WorldScopes::GUEST).await.unwrap();
    let shell = start_shell(&host, ann.expose(), opted_in_adapter(&tree.root)).await;

    shell.input("notes.txt").await;
    let results = host.wait_results(1).await;
    assert_eq!(results[0]["ok"], true, "result: {}", results[0]);
    assert_eq!(results[0]["value"]["provider"], "client.file.read");
    assert_eq!(
        results[0]["value"]["result_hex"],
        json!(hex_encode(FILE_BODY))
    );

    // Local audit: exactly one entry, the execution, no reason.
    {
        let adapter = shell.adapter.lock().unwrap();
        let entries = adapter.audit_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].outcome, "executed");
        assert_eq!(entries[0].reason, None);
    }
    // Host audit: the broker recorded the allowed use and its
    // completion — and neither audit trail carries the file's
    // content, its name, or the local root path.
    let host_audit = host.host_audit_text();
    assert!(
        host_audit.contains("client_provider_use"),
        "host audit: {host_audit}"
    );
    assert!(host_audit.contains("allowed"), "host audit: {host_audit}");
    assert!(host_audit.contains("completed"), "host audit: {host_audit}");
    for forbidden in [
        String::from_utf8_lossy(FILE_BODY).into_owned(),
        "notes.txt".to_owned(),
        tree.root.to_string_lossy().into_owned(),
    ] {
        assert!(
            !host_audit.contains(&forbidden),
            "host audit leaked {forbidden:?}: {host_audit}"
        );
    }

    shell.finish().await;
    host.client_ep.close().await;
}

#[tokio::test]
async fn refusal_matrix_reaches_the_guest_with_stable_codes() {
    let tree = temp_tree();
    let host = hosted_world("e2e-refusals").await;
    host.open_world_gates().await;
    let (_, ann) = host.world.issue("ann", WorldScopes::GUEST).await.unwrap();
    let shell = start_shell(&host, ann.expose(), opted_in_adapter(&tree.root)).await;

    let absolute = tree.root.join("notes.txt").to_string_lossy().into_owned();
    let mut cases: Vec<(String, &str)> = vec![
        ("../outside.txt".to_owned(), "file_read_invalid_path"),
        ("sub/../../outside.txt".to_owned(), "file_read_invalid_path"),
        (absolute, "file_read_invalid_path"),
        ("missing.txt".to_owned(), "file_read_not_found"),
        ("sub".to_owned(), "file_read_not_a_file"),
        ("big.bin".to_owned(), "result_too_large"),
    ];
    #[cfg(unix)]
    cases.push(("escape.txt".to_owned(), "file_read_invalid_path"));

    for (index, (path, code)) in cases.iter().enumerate() {
        shell.input(path).await;
        let results = host.wait_results(index + 1).await;
        let result = &results[index];
        assert_eq!(result["ok"], false, "path {path:?}: {result}");
        assert_eq!(result["error"], "cap_denied", "path {path:?}: {result}");
        assert_eq!(result["reason"], *code, "path {path:?}: {result}");
    }

    // Every refusal was decided locally and logged with its code.
    let adapter = shell.adapter.lock().unwrap();
    let entries = adapter.audit_entries();
    assert_eq!(entries.len(), cases.len());
    for (entry, (_, code)) in entries.iter().zip(cases.iter()) {
        assert_eq!(entry.outcome, "refused");
        assert_eq!(entry.reason.as_deref(), Some(*code));
    }
    drop(adapter);

    shell.finish().await;
    host.client_ep.close().await;
}

#[tokio::test]
async fn world_gates_refuse_before_any_frame_reaches_the_client() {
    let tree = temp_tree();
    let host = hosted_world("e2e-gates").await;
    let (_, ann) = host.world.issue("ann", WorldScopes::GUEST).await.unwrap();
    let shell = start_shell(&host, ann.expose(), opted_in_adapter(&tree.root)).await;

    // Ceiling excludes the provider (grant present): refused by the
    // broker's ceiling step.
    host.world
        .set_caps_ceiling(vec!["chat.say".to_owned()])
        .await
        .unwrap();
    host.world
        .update_caps(vec!["client.file.read".to_owned()], Vec::new())
        .await
        .unwrap();
    shell.input("notes.txt").await;
    let results = host.wait_results(1).await;
    assert_eq!(results[0]["ok"], false, "result: {}", results[0]);
    assert_eq!(results[0]["error"], "outside_ceiling");

    // Ceiling admits it but the grant is gone: refused by the
    // broker's grant step (grant-on-use never applies).
    host.world
        .set_caps_ceiling(vec!["client.file.read".to_owned()])
        .await
        .unwrap();
    host.world
        .update_caps(Vec::new(), vec!["client.file.read".to_owned()])
        .await
        .unwrap();
    shell.input("notes.txt").await;
    let results = host.wait_results(2).await;
    assert_eq!(results[1]["ok"], false, "result: {}", results[1]);
    assert_eq!(results[1]["error"], "cap_denied");

    // Neither refusal reached the client: the local adapter saw no
    // request at all, while the host audit recorded both denials.
    assert!(shell.adapter.lock().unwrap().audit_entries().is_empty());
    let host_audit = host.host_audit_text();
    assert!(
        host_audit.contains("outside_ceiling"),
        "host audit: {host_audit}"
    );
    assert!(
        host_audit.contains("not_granted"),
        "host audit: {host_audit}"
    );

    // Opening both gates again makes the same request succeed,
    // proving the refusals were the gates and not the setup.
    host.world
        .update_caps(vec!["client.file.read".to_owned()], Vec::new())
        .await
        .unwrap();
    shell.input("notes.txt").await;
    let results = host.wait_results(3).await;
    assert_eq!(results[2]["ok"], true, "result: {}", results[2]);

    shell.finish().await;
    host.client_ep.close().await;
}

#[tokio::test]
async fn no_local_opt_in_is_unavailable_and_offers_never_cross_sessions() {
    let tree = temp_tree();
    let host = hosted_world("e2e-no-opt-in").await;
    host.open_world_gates().await;
    let (_, ann) = host.world.issue("ann", WorldScopes::GUEST).await.unwrap();
    let (_, bo) = host.world.issue("bo", WorldScopes::GUEST).await.unwrap();
    let ann_shell = start_shell(&host, ann.expose(), opted_in_adapter(&tree.root)).await;
    // Bo's shell never opted in: the shipped default, no grants, no
    // offer frame.
    let bo_shell = start_shell(&host, bo.expose(), ProviderAdapter::new()).await;

    // Bo's request cannot ride on Ann's offer: availability is the
    // originating session's offer only.
    bo_shell.input("notes.txt").await;
    let results = host.wait_results(1).await;
    assert_eq!(results[0]["ok"], false, "result: {}", results[0]);
    assert_eq!(results[0]["error"], "failed");
    assert_eq!(results[0]["reason"], "provider_unavailable");
    assert!(bo_shell.adapter.lock().unwrap().audit_entries().is_empty());

    // Ann's identical request, from the session that offered the
    // provider, succeeds.
    ann_shell.input("notes.txt").await;
    let results = host.wait_results(2).await;
    assert_eq!(results[1]["ok"], true, "result: {}", results[1]);
    assert_eq!(
        results[1]["value"]["result_hex"],
        json!(hex_encode(FILE_BODY))
    );

    bo_shell.finish().await;
    ann_shell.finish().await;
    host.client_ep.close().await;
}

#[tokio::test]
async fn unacknowledged_disclosure_refuses_end_to_end() {
    let tree = temp_tree();
    let host = hosted_world("e2e-disclosure").await;
    host.open_world_gates().await;
    let (_, ann) = host.world.issue("ann", WorldScopes::GUEST).await.unwrap();
    // Register an executor built WITHOUT the disclosure
    // acknowledgment through the adapter's raw route (the shipped
    // shell's grant_file_read refuses this combination up front;
    // the executor's own gate is the second line of defense).
    let mut adapter = ProviderAdapter::new();
    adapter.register_executor(
        "client.file.read",
        Box::new(FileReadExecutor::new(tree.root.clone(), false).unwrap()),
    );
    adapter.grant("", "client.file.read", LocalBounds::DESIGN_DEFAULT);
    let shell = start_shell(&host, ann.expose(), adapter).await;

    shell.input("notes.txt").await;
    let results = host.wait_results(1).await;
    assert_eq!(results[0]["ok"], false, "result: {}", results[0]);
    assert_eq!(results[0]["error"], "cap_denied");
    assert_eq!(results[0]["reason"], "content_disclosure_not_acknowledged");

    let adapter = shell.adapter.lock().unwrap();
    let entries = adapter.audit_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].outcome, "refused");
    assert_eq!(
        entries[0].reason.as_deref(),
        Some("content_disclosure_not_acknowledged")
    );
    drop(adapter);

    shell.finish().await;
    host.client_ep.close().await;
}

#[tokio::test]
async fn revocation_at_either_gate_refuses_the_next_read() {
    let tree = temp_tree();
    let host = hosted_world("e2e-revocation").await;
    host.open_world_gates().await;
    let (_, ann) = host.world.issue("ann", WorldScopes::GUEST).await.unwrap();
    let shell = start_shell(&host, ann.expose(), opted_in_adapter(&tree.root)).await;

    shell.input("notes.txt").await;
    let results = host.wait_results(1).await;
    assert_eq!(results[0]["ok"], true, "result: {}", results[0]);

    // World-side revoke: the broker refuses before routing, so the
    // local adapter sees nothing new.
    host.world
        .update_caps(Vec::new(), vec!["client.file.read".to_owned()])
        .await
        .unwrap();
    shell.input("notes.txt").await;
    let results = host.wait_results(2).await;
    assert_eq!(results[1]["ok"], false, "result: {}", results[1]);
    assert_eq!(results[1]["error"], "cap_denied");
    assert_eq!(shell.adapter.lock().unwrap().audit_entries().len(), 1);

    // Re-granting restores the read.
    host.world
        .update_caps(vec!["client.file.read".to_owned()], Vec::new())
        .await
        .unwrap();
    shell.input("notes.txt").await;
    let results = host.wait_results(3).await;
    assert_eq!(results[2]["ok"], true, "result: {}", results[2]);

    // Local revoke: the offer was registered at join, so the broker
    // still routes — and the adapter refuses with provider_revoked.
    assert!(shell.adapter.lock().unwrap().revoke("", "client.file.read"));
    shell.input("notes.txt").await;
    let results = host.wait_results(4).await;
    assert_eq!(results[3]["ok"], false, "result: {}", results[3]);
    assert_eq!(results[3]["error"], "cap_denied");
    assert_eq!(results[3]["reason"], "provider_revoked");
    let adapter = shell.adapter.lock().unwrap();
    let last = adapter.audit_entries().last().unwrap();
    assert_eq!(last.outcome, "refused");
    assert_eq!(last.reason.as_deref(), Some("provider_revoked"));
    drop(adapter);

    shell.finish().await;
    host.client_ep.close().await;
}
