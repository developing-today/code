//! #10394 Q1: what one request/response costs across each fleet<->ID boundary shape.
//! Dependency-free so it builds with a bare `rustc -O` on any runner:
//!   rustc -O --crate-type cdylib ipc_plugin.rs -o libipc_plugin.so
//!   rustc -O ipc_bench.rs -o ipc_bench && ./ipc_bench ./libipc_plugin.so
//! Every leg runs the same echo: send N bytes, get the same N bytes back.
use std::ffi::{CString, c_char, c_int, c_void};
use std::hint::black_box;
use std::io::{BufReader, BufWriter, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}
const RTLD_NOW: c_int = 2;
type EchoFn = extern "C" fn(*const u8, usize, *mut u8) -> usize;

fn write_frame(w: &mut impl Write, buf: &[u8]) {
    w.write_all(&(buf.len() as u32).to_le_bytes()).unwrap();
    w.write_all(buf).unwrap();
    w.flush().unwrap();
}
fn read_frame(r: &mut impl Read, buf: &mut Vec<u8>) -> bool {
    let mut len = [0u8; 4];
    if r.read_exact(&mut len).is_err() {
        return false;
    }
    buf.resize(u32::from_le_bytes(len) as usize, 0);
    r.read_exact(buf).unwrap();
    true
}
fn serve_stream(s: impl Read + Write + Clone) {
    let mut r = BufReader::new(s.clone());
    let mut w = BufWriter::new(s);
    let mut buf = Vec::new();
    while read_frame(&mut r, &mut buf) {
        write_frame(&mut w, &buf);
    }
}
// A tiny HTTP/1.1 keep-alive echo: the shape of mcpx's daemon API over its unix socket.
fn serve_http(s: UnixStream) {
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut w = BufWriter::new(s);
    loop {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if r.read_exact(&mut byte).is_err() {
                return;
            }
            head.push(byte[0]);
        }
        let text = String::from_utf8_lossy(&head);
        let len: usize = text
            .lines()
            .find_map(|l| {
                l.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .map(|v| v.trim().parse().unwrap())
            })
            .unwrap_or(0);
        let mut body = vec![0u8; len];
        r.read_exact(&mut body).unwrap();
        write!(w, "HTTP/1.1 200 OK\r\ncontent-type: application/octet-stream\r\ncontent-length: {len}\r\n\r\n").unwrap();
        w.write_all(&body).unwrap();
        w.flush().unwrap();
    }
}

#[derive(Clone)]
struct Uds(std::sync::Arc<UnixStream>);
impl Read for Uds {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        (&*self.0).read(b)
    }
}
impl Write for Uds {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        (&*self.0).write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        (&*self.0).flush()
    }
}
#[derive(Clone)]
struct Tcp(std::sync::Arc<TcpStream>);
impl Read for Tcp {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        (&*self.0).read(b)
    }
}
impl Write for Tcp {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        (&*self.0).write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        (&*self.0).flush()
    }
}

fn server_mode(mode: &str, addr: &str) {
    match mode {
        "stdio" => {
            let stdin = std::io::stdin().lock();
            let stdout = std::io::stdout().lock();
            let mut r = BufReader::new(stdin);
            let mut w = BufWriter::new(stdout);
            let mut buf = Vec::new();
            while read_frame(&mut r, &mut buf) {
                write_frame(&mut w, &buf);
            }
        }
        "once" => {
            // Spawn-per-call: read one frame from stdin, echo it, exit.
            let mut buf = Vec::new();
            read_frame(&mut std::io::stdin().lock(), &mut buf);
            write_frame(&mut std::io::stdout().lock(), &buf);
        }
        "uds" => {
            let l = UnixListener::bind(addr).unwrap();
            println!("ready");
            let (s, _) = l.accept().unwrap();
            serve_stream(Uds(std::sync::Arc::new(s)));
        }
        "http" => {
            let l = UnixListener::bind(addr).unwrap();
            println!("ready");
            let (s, _) = l.accept().unwrap();
            serve_http(s);
        }
        "tcp" => {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            println!("{}", l.local_addr().unwrap().port());
            let (s, _) = l.accept().unwrap();
            s.set_nodelay(true).unwrap();
            serve_stream(Tcp(std::sync::Arc::new(s)));
        }
        _ => panic!("unknown mode {mode}"),
    }
}

fn stats(name: &str, size: usize, mut d: Vec<Duration>) {
    d.sort();
    let us = |x: Duration| x.as_secs_f64() * 1e6;
    let p = |q: f64| us(d[((d.len() as f64 - 1.0) * q).round() as usize]);
    let mean = d.iter().map(|x| us(*x)).sum::<f64>() / d.len() as f64;
    println!(
        "| {name} | {size} | {} | {:.3} | {:.3} | {:.3} | {:.3} |",
        d.len(),
        us(d[0]),
        p(0.5),
        p(0.99),
        mean
    );
}

fn time_loop(n: usize, mut f: impl FnMut()) -> Vec<Duration> {
    for _ in 0..(n / 10).max(1) {
        f(); // warm-up
    }
    (0..n)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect()
}

fn ready_line(child: &mut std::process::Child) -> String {
    let mut out = child.stdout.take().unwrap();
    let mut line = Vec::new();
    let mut b = [0u8; 1];
    loop {
        out.read_exact(&mut b).unwrap();
        if b[0] == b'\n' {
            break;
        }
        line.push(b[0]);
    }
    String::from_utf8(line).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "--serve" {
        return server_mode(&args[2], args.get(3).map(String::as_str).unwrap_or(""));
    }
    let plugin = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "./libipc_plugin.so".into());
    let exe = std::env::current_exe().unwrap();
    let tmp = std::env::temp_dir();
    let sizes = [64usize, 16 * 1024];
    let scale: usize = std::env::var("IPC_BENCH_SCALE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);

    println!("| boundary | payload bytes | samples | min us | p50 us | p99 us | mean us |");
    println!("|---|---|---|---|---|---|---|");
    for &size in &sizes {
        let payload = vec![0xA5u8; size];
        let mut out = vec![0u8; size];

        // 1. Static embed: an ordinary in-process call (the copy is the work).
        stats(
            "in-process call (static embed)",
            size,
            time_loop(200_000 * scale, || {
                out.copy_from_slice(black_box(&payload));
                black_box(&out);
            }),
        );

        // 2. Dynamic library: first load once, then calls through the dlsym pointer.
        let path = CString::new(plugin.clone()).unwrap();
        let t = Instant::now();
        let handle = unsafe { dlopen(path.as_ptr(), RTLD_NOW) };
        assert!(!handle.is_null(), "dlopen {plugin} failed");
        let sym = CString::new("plugin_echo").unwrap();
        let f: EchoFn = unsafe { std::mem::transmute(dlsym(handle, sym.as_ptr())) };
        if size == sizes[0] {
            stats("dlopen + dlsym (one-time load)", 0, vec![t.elapsed()]);
        }
        stats(
            "dylib call via dlsym pointer",
            size,
            time_loop(200_000 * scale, || {
                black_box(f(payload.as_ptr(), payload.len(), out.as_mut_ptr()));
            }),
        );

        // 3. Persistent child over stdin/stdout pipes (an exec'd helper kept alive; MCP stdio shape).
        let mut child = Command::new(&exe)
            .args(["--serve", "stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut w = BufWriter::new(child.stdin.take().unwrap());
        let mut r = BufReader::new(child.stdout.take().unwrap());
        let mut buf = Vec::new();
        stats(
            "persistent child, stdio pipes",
            size,
            time_loop(20_000 * scale, || {
                write_frame(&mut w, &payload);
                read_frame(&mut r, &mut buf);
            }),
        );
        drop(w);
        child.wait().unwrap();

        // 4. Unix domain socket to a separate server process.
        let sock = tmp.join(format!("ipc-bench-{}-{size}.sock", std::process::id()));
        let _ = std::fs::remove_file(&sock);
        let mut child = Command::new(&exe)
            .args(["--serve", "uds", sock.to_str().unwrap()])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        ready_line(&mut child);
        let s = Uds(std::sync::Arc::new(UnixStream::connect(&sock).unwrap()));
        let (mut r, mut w) = (BufReader::new(s.clone()), BufWriter::new(s));
        stats(
            "unix socket, framed",
            size,
            time_loop(20_000 * scale, || {
                write_frame(&mut w, &payload);
                read_frame(&mut r, &mut buf);
            }),
        );
        drop((r, w));
        child.wait().unwrap();
        let _ = std::fs::remove_file(&sock);

        // 5. HTTP/1.1 keep-alive over a unix socket (mcpx daemon API shape).
        let mut child = Command::new(&exe)
            .args(["--serve", "http", sock.to_str().unwrap()])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        ready_line(&mut child);
        let s = UnixStream::connect(&sock).unwrap();
        let mut r = BufReader::new(s.try_clone().unwrap());
        let mut w = BufWriter::new(s);
        stats(
            "unix socket, HTTP/1.1 keep-alive",
            size,
            time_loop(20_000 * scale, || {
                write!(
                    w,
                    "POST /echo HTTP/1.1\r\nhost: x\r\ncontent-length: {}\r\n\r\n",
                    payload.len()
                )
                .unwrap();
                w.write_all(&payload).unwrap();
                w.flush().unwrap();
                let mut head = Vec::new();
                let mut b = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    r.read_exact(&mut b).unwrap();
                    head.push(b[0]);
                }
                buf.resize(payload.len(), 0);
                r.read_exact(&mut buf).unwrap();
            }),
        );
        drop((r, w));
        child.wait().unwrap();
        let _ = std::fs::remove_file(&sock);

        // 6. TCP loopback to a separate server process (the remote-transport floor).
        let mut child = Command::new(&exe)
            .args(["--serve", "tcp"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let port = ready_line(&mut child);
        let s = TcpStream::connect(("127.0.0.1", port.parse::<u16>().unwrap())).unwrap();
        s.set_nodelay(true).unwrap();
        let s = Tcp(std::sync::Arc::new(s));
        let (mut r, mut w) = (BufReader::new(s.clone()), BufWriter::new(s));
        stats(
            "tcp loopback, framed",
            size,
            time_loop(20_000 * scale, || {
                write_frame(&mut w, &payload);
                read_frame(&mut r, &mut buf);
            }),
        );
        drop((r, w));
        child.wait().unwrap();

        // 7. Spawn per call: exec a fresh process for every request (`fleet` shelling out to `id`).
        stats(
            "spawn per call (fork+exec, this binary)",
            size,
            time_loop(300 * scale, || {
                let mut c = Command::new(&exe)
                    .args(["--serve", "once"])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .spawn()
                    .unwrap();
                write_frame(&mut c.stdin.take().unwrap(), &payload);
                let mut o = c.stdout.take().unwrap();
                read_frame(&mut o, &mut buf);
                c.wait().unwrap();
            }),
        );
    }
}
