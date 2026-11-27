//! Mode-1 process acceptance tests. Oracle: exact bytes at a stock
//! TCP peer plus bounded process termination, with stdin held open.
#![cfg(feature = "id")]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const LIMIT: Duration = Duration::from_secs(10);

struct Proxy(Child);

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn proxy(listener: &TcpListener) -> Proxy {
    Proxy(
        Command::new(env!("CARGO_BIN_EXE_fleet"))
            .args([
                "id",
                "proxy",
                &format!("tcp://{}", listener.local_addr().unwrap()),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn proxy"),
    )
}

fn accept(listener: TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let start = Instant::now();
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(start.elapsed() < LIMIT, "proxy did not connect");
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("accept failed: {e}"),
        }
    };
    stream.set_read_timeout(Some(LIMIT)).unwrap();
    stream.set_write_timeout(Some(LIMIT)).unwrap();
    stream
}

#[test]
fn id_proxy_stdio_exits_on_peer_eof_with_stdin_open() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut child = proxy(&listener);
    let mut peer = accept(listener);
    peer.write_all(b"final\0bytes").unwrap();
    drop(peer);
    // Deliberately retain child.stdin: EOF on the target, not caller
    // stdin, must terminate the process.
    let start = Instant::now();
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success(), "{status}");
            break;
        }
        assert!(start.elapsed() < LIMIT, "proxy hung on open stdin");
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut output = Vec::new();
    child
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut output)
        .unwrap();
    assert_eq!(output, b"final\0bytes");
}

#[test]
fn id_proxy_stdio_delivers_binary_response_before_either_side_closes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut child = proxy(&listener);
    let mut peer = accept(listener);
    let mut output = child.0.stdout.take().unwrap();
    let (sent, received) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut response = [0; 4];
        output.read_exact(&mut response).unwrap();
        sent.send(response).unwrap();
    });
    peer.write_all(b"\0\x01\xff\x02").unwrap();
    assert_eq!(
        received
            .recv_timeout(LIMIT)
            .expect("binary response buffered until EOF"),
        [0, 1, 255, 2]
    );
    child.0.stdin.as_mut().unwrap().write_all(b"ack").unwrap();
    let mut ack = [0; 3];
    peer.read_exact(&mut ack).unwrap();
    assert_eq!(&ack, b"ack");
    reader.join().unwrap();
}
