//! Shared broker TCP admission and request dispatch (#14330).

use crate::uds_http::{HttpRequest, InFlightSlot, MAX_BODY_BYTES};
use std::io::Read;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

pub(crate) fn serve<F, G>(listener: TcpListener, precheck: G, handler: F)
where
    F: Fn(&HttpRequest) -> (u16, Vec<u8>) + Send + Sync + 'static,
    G: Fn(&HttpRequest) -> Option<(u16, Vec<u8>)> + Send + Sync + 'static,
{
    let server = tiny_http::Server::from_listener(listener, None).unwrap();
    let precheck = Arc::new(precheck);
    let handler = Arc::new(handler);
    let in_flight = Arc::new(AtomicUsize::new(0));
    for mut req in server.incoming_requests() {
        let Some(slot) = InFlightSlot::try_acquire(&in_flight) else {
            let _ =
                req.respond(tiny_http::Response::from_string("overloaded").with_status_code(503));
            continue;
        };
        let precheck = Arc::clone(&precheck);
        let handler = Arc::clone(&handler);
        std::thread::spawn(move || {
            let _slot = slot;
            let mut parsed = HttpRequest {
                method: req.method().as_str().to_string(),
                target: req.url().to_string(),
                headers: req
                    .headers()
                    .iter()
                    .map(|h| (h.field.to_string(), h.value.to_string()))
                    .collect(),
                body: Vec::new(),
            };
            let (status, body) = if let Some(response) = precheck(&parsed) {
                response
            } else {
                let _ = req
                    .as_reader()
                    .take((MAX_BODY_BYTES + 1) as u64)
                    .read_to_end(&mut parsed.body);
                if parsed.body.len() > MAX_BODY_BYTES {
                    (413, b"{}".to_vec())
                } else {
                    handler(&parsed)
                }
            };
            let _ = req.respond(tiny_http::Response::from_data(body).with_status_code(status));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uds_http::MAX_CONNECTIONS;
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpStream};
    use std::sync::atomic::Ordering;
    use std::sync::{Condvar, Mutex, mpsc};
    use std::time::Duration;

    #[test]
    fn unauthenticated_withheld_body_burst_is_bounded_and_recovers() {
        // Oracle: at most MAX_CONNECTIONS worker prechecks may be entered;
        // overload sockets close promptly, and a healthy request completes
        // after capacity frees while rejected peers still hold their sockets.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let entered = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (tx, rx) = mpsc::channel();
        let worker_entered = Arc::clone(&entered);
        let worker_gate = Arc::clone(&gate);
        std::thread::spawn(move || {
            serve(
                listener,
                move |head| {
                    if head.target == "/hold" {
                        worker_entered.fetch_add(1, Ordering::SeqCst);
                        tx.send(()).unwrap();
                        let (lock, cv) = &*worker_gate;
                        let _released = cv
                            .wait_while(lock.lock().unwrap(), |released| !*released)
                            .unwrap();
                        return Some((401, b"{}".to_vec()));
                    }
                    None
                },
                |_| (200, b"ok".to_vec()),
            );
        });
        let connect = |target: &str| {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            write!(
                stream,
                "POST {target} HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2048\r\n\r\n"
            )
            .unwrap();
            stream
        };
        let held: Vec<_> = (0..MAX_CONNECTIONS).map(|_| connect("/hold")).collect();
        for _ in 0..MAX_CONNECTIONS {
            rx.recv_timeout(Duration::from_secs(10))
                .expect("worker admitted");
        }
        let mut rejected: Vec<_> = (0..32).map(|_| connect("/hold")).collect();
        let mut all_closed = true;
        for peer in rejected.iter_mut().take(1) {
            let mut response = Vec::new();
            match peer.read_to_end(&mut response) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
                Err(_) => all_closed = false,
            }
        }
        // Always release test workers even on failure, to avoid hanging the
        // suite on the very regression this test detects.
        let (lock, cv) = &*gate;
        *lock.lock().unwrap() = true;
        cv.notify_all();
        for peer in held {
            let _ = peer.shutdown(Shutdown::Both);
        }
        assert!(
            all_closed,
            "saturation must close without draining withheld bodies"
        );
        assert_eq!(entered.load(Ordering::SeqCst), MAX_CONNECTIONS);
        let mut healthy = TcpStream::connect(addr).unwrap();
        healthy
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        healthy
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        healthy.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        // Keep these alive through the recovery assertion.
        drop(rejected);
    }
}
