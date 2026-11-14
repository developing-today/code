use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

fn read_request(stream: &mut TcpStream) {
    let mut request = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = stream.read(&mut buffer).expect("read fake-broker request");
        assert_ne!(n, 0, "client closed before sending a full request");
        request.extend_from_slice(&buffer[..n]);
        let Some(header_end) = request.windows(4).position(|w| w == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length").then(|| {
                    value
                        .trim()
                        .parse::<usize>()
                        .expect("request content length")
                })
            })
            .expect("request has Content-Length");
        if request.len() >= header_end + 4 + content_length {
            return;
        }
    }
}

fn fake_broker(response: Vec<u8>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake broker");
    let address = listener.local_addr().expect("fake broker address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept queue request");
        read_request(&mut stream);
        stream.write_all(&response).expect("write fake response");
    });
    (format!("http://{address}"), server)
}

fn temp_home() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after Unix epoch")
        .as_nanos();
    let home = std::env::temp_dir().join(format!(
        "fleet-queue-chunked-list-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&home).expect("create isolated HOME");
    home
}

fn run_list(home: &Path, broker_url: &str) -> Output {
    let token_file = home.join("broker-token");
    std::fs::write(&token_file, "test-token\n").expect("write fake token");
    Command::new(env!("CARGO_BIN_EXE_repo-queue"))
        .args(["list", "--queue", "fake-queue", "--limit", "100"])
        .env("HOME", home)
        .env("REPO_QUEUE_BROKER_URL", broker_url)
        .env("REPO_QUEUE_TOKEN_FILE", token_file)
        .output()
        .expect("run repo-queue list")
}

fn chunked_response(body: &[u8]) -> Vec<u8> {
    let mut response =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
    for chunk in body.chunks(4096) {
        response.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
        response.extend_from_slice(chunk);
        response.extend_from_slice(b"\r\n");
    }
    response.extend_from_slice(b"0\r\n\r\n");
    response
}

#[test]
fn list_prints_all_rows_from_a_chunked_response_over_32_kib_and_rejects_bad_json() {
    let rows: Vec<Value> = (1..=100)
        .map(|id| {
            json!({
                "id": id,
                "priority": 50,
                "status": "queued",
                "dedupe_key": format!("fixture:{id}"),
                "title": format!("row-{id}-{}", "x".repeat(500)),
            })
        })
        .collect();
    let body = serde_json::to_vec(&json!({"ok": true, "rows": rows})).unwrap();
    assert!(body.len() > 32 * 1024, "fixture is {} bytes", body.len());
    let (url, server) = fake_broker(chunked_response(&body));
    let home = temp_home();
    let output = run_list(&home, &url);
    server.join().expect("fake server completes");
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let task_rows = stdout
        .lines()
        .filter(|line| {
            line.strip_prefix("task ")
                .and_then(|rest| rest.split_whitespace().next())
                .is_some_and(|id| id.parse::<u64>().is_ok())
        })
        .count();
    assert_eq!(
        task_rows, 100,
        "expected all rows, got {task_rows}: {stdout}"
    );
    for id in [1, 50, 100] {
        assert!(
            stdout
                .lines()
                .any(|line| line.starts_with(&format!("task {id} "))),
            "missing task {id}: {stdout}"
        );
    }
    std::fs::remove_dir_all(&home).expect("remove isolated HOME");

    let (url, server) = fake_broker(
        b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nnot-json".to_vec(),
    );
    let home = temp_home();
    let output = run_list(&home, &url);
    server.join().expect("fake server completes");
    assert!(
        !output.status.success(),
        "bad JSON must not succeed: {output:?}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid JSON body"), "{stderr}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("(no tasks)"));
    std::fs::remove_dir_all(&home).expect("remove isolated HOME");
}
