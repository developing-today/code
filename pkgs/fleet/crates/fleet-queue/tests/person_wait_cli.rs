//! `repo-queue park` and `block` refuse a wait on a person before the
//! broker is asked (FDR-0013), and let a hands item and an ordinary
//! wait through.
//!
//! Oracle: the rule in `doc/decisions/FDR-0013-no-owner-decision-gates.md`
//! — only a hands item may wait on a person. The fake broker counts
//! connections, so "refused before the broker" is observed, not
//! inferred from the exit code alone.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

struct FakeBroker {
    url: String,
    connections: Arc<AtomicUsize>,
}

/// A broker that answers every request `400` and counts connections.
fn fake_broker() -> FakeBroker {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake broker");
    let url = format!("http://{}", listener.local_addr().expect("local addr"));
    let connections = Arc::new(AtomicUsize::new(0));
    let seen = connections.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            seen.fetch_add(1, Ordering::SeqCst);
            let mut buffer = [0u8; 8192];
            let _ = stream.read(&mut buffer);
            let body = br#"{"error":"fake broker"}"#;
            let head = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body);
        }
    });
    FakeBroker { url, connections }
}

fn isolated_home(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let home = std::env::temp_dir().join(format!(
        "fleet-person-wait-{tag}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&home).expect("create isolated HOME");
    home
}

fn run(tag: &str, broker: &FakeBroker, args: &[&str]) -> Output {
    let home = isolated_home(tag);
    let token_file = home.join("broker-token");
    std::fs::write(&token_file, "test-token\n").expect("write fake token");
    let output = Command::new(env!("CARGO_BIN_EXE_repo-queue"))
        .args(args)
        .env("HOME", &home)
        .env("REPO_QUEUE_BROKER_URL", &broker.url)
        .env("REPO_QUEUE_TOKEN_FILE", &token_file)
        .output()
        .expect("run repo-queue");
    let _ = std::fs::remove_dir_all(&home);
    output
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn block_on_an_owner_class_is_refused_before_the_broker_is_asked() {
    let broker = fake_broker();
    let output = run(
        "block-refused",
        &broker,
        &[
            "block",
            "42",
            "--worker",
            "w",
            "--lane",
            "l",
            "--reason-class",
            "escalated-to-user",
            "--blocker",
            "which encoding should this use?",
        ],
    );
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    let text = stderr(&output);
    assert!(text.contains("refusing to wait on a person"), "{text}");
    assert!(text.contains("FDR-0013"), "{text}");
    assert_eq!(broker.connections.load(Ordering::SeqCst), 0);
}

#[test]
fn park_whose_reason_waits_on_the_owner_is_refused_before_the_broker_is_asked() {
    let broker = fake_broker();
    let output = run(
        "park-refused",
        &broker,
        &[
            "park",
            "42",
            "--worker",
            "w",
            "--lane",
            "l",
            "--reason",
            "awaiting owner decision on scope",
            "--delay-seconds",
            "600",
        ],
    );
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("refusing to wait on a person"),
        "{}",
        stderr(&output)
    );
    assert_eq!(broker.connections.load(Ordering::SeqCst), 0);
}

#[test]
fn a_hands_item_is_let_through_to_the_broker() {
    let broker = fake_broker();
    let output = run(
        "hands",
        &broker,
        &[
            "block",
            "42",
            "--worker",
            "w",
            "--lane",
            "l",
            "--reason-class",
            "escalated-to-user",
            "--blocker",
            "the gh login lacks the codespace scope",
            "--hands",
            "interactive-login",
            "--action",
            "Run gh auth refresh -s codespace on the laptop and approve the code.",
        ],
    );
    // The fake broker answers 400, so the verb fails — but for the
    // broker's reason, not the guard's.
    assert_ne!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        !stderr(&output).contains("refusing to wait on a person"),
        "{}",
        stderr(&output)
    );
    assert!(broker.connections.load(Ordering::SeqCst) >= 1);
}

#[test]
fn a_hands_item_without_its_one_action_is_refused() {
    let broker = fake_broker();
    let output = run(
        "hands-no-action",
        &broker,
        &[
            "block",
            "42",
            "--worker",
            "w",
            "--lane",
            "l",
            "--reason-class",
            "escalated-to-user",
            "--blocker",
            "needs a login",
            "--hands",
            "interactive-login",
        ],
    );
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert_eq!(broker.connections.load(Ordering::SeqCst), 0);
}

#[test]
fn an_ordinary_park_is_not_mistaken_for_a_person_wait() {
    // Positive control for the refusals above: a wait on CI reaches
    // the broker.
    let broker = fake_broker();
    let output = run(
        "ordinary-park",
        &broker,
        &[
            "park",
            "42",
            "--worker",
            "w",
            "--lane",
            "l",
            "--reason",
            "waiting for CI on the open PR",
            "--delay-seconds",
            "600",
        ],
    );
    assert!(
        !stderr(&output).contains("refusing to wait on a person"),
        "{}",
        stderr(&output)
    );
    assert!(broker.connections.load(Ordering::SeqCst) >= 1);
}
