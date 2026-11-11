//! Byte-transparent, bidirectional TCP/Unix stream relay.
//!
//! The explicit read/write loop is intentional: `std::io::copy` may
//! use `splice(2)`, which has stalled mid-stream in the fleet sandbox.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::os::unix::net::UnixStream;

pub trait RelayStream: Read + Write + Send + 'static {
    fn try_clone_stream(&self) -> io::Result<Self>
    where
        Self: Sized;
    fn shutdown_write(&self) -> io::Result<()>;
    fn shutdown_both(&self) -> io::Result<()>;
}

impl RelayStream for TcpStream {
    fn try_clone_stream(&self) -> io::Result<Self> {
        self.try_clone()
    }

    fn shutdown_write(&self) -> io::Result<()> {
        self.shutdown(Shutdown::Write)
    }

    fn shutdown_both(&self) -> io::Result<()> {
        self.shutdown(Shutdown::Both)
    }
}

impl RelayStream for UnixStream {
    fn try_clone_stream(&self) -> io::Result<Self> {
        self.try_clone()
    }

    fn shutdown_write(&self) -> io::Result<()> {
        self.shutdown(Shutdown::Write)
    }

    fn shutdown_both(&self) -> io::Result<()> {
        self.shutdown(Shutdown::Both)
    }
}

fn pump<R: Read, W: Write>(src: &mut R, dst: &mut W) -> io::Result<()> {
    let mut buf = [0u8; 16 * 1024];
    loop {
        match src.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                dst.write_all(&buf[..n])?;
                dst.flush()?;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
}

/// Relay both directions until both peers half-close. An error in
/// either direction shuts down both sockets to unblock the other pump.
pub fn relay<A: RelayStream, B: RelayStream>(a: A, b: B) -> io::Result<()> {
    let mut a_read = a.try_clone_stream()?;
    let mut b_write = b.try_clone_stream()?;
    let mut b_read = b.try_clone_stream()?;
    let mut a_write = a.try_clone_stream()?;
    let a_error = a.try_clone_stream()?;
    let b_error = b.try_clone_stream()?;

    let upstream = std::thread::spawn(move || {
        let result = pump(&mut a_read, &mut b_write);
        let _ = b_write.shutdown_write();
        if result.is_err() {
            let _ = a_error.shutdown_both();
            let _ = b_error.shutdown_both();
        }
        result
    });

    let downstream = pump(&mut b_read, &mut a_write);
    let _ = a_write.shutdown_write();
    if downstream.is_err() {
        let _ = a.shutdown_both();
        let _ = b.shutdown_both();
    }
    let upstream = upstream
        .join()
        .map_err(|_| io::Error::other("raw relay pump panicked"))?;
    downstream.and(upstream)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (UnixStream, UnixStream, UnixStream, UnixStream) {
        let (left, left_peer) = UnixStream::pair().unwrap();
        let (right, right_peer) = UnixStream::pair().unwrap();
        (left, right, left_peer, right_peer)
    }

    #[test]
    fn relay_preserves_bytes_both_ways_and_propagates_half_close() {
        let (left, right, mut client, mut server) = pair();
        let relay = std::thread::spawn(move || relay(left, right));

        client.write_all(b"client-first\0\xff").unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let mut from_client = Vec::new();
        server.read_to_end(&mut from_client).unwrap();
        assert_eq!(from_client, b"client-first\0\xff");

        server.write_all(b"server-reply\xfe\0").unwrap();
        server.shutdown(Shutdown::Write).unwrap();
        let mut from_server = Vec::new();
        client.read_to_end(&mut from_server).unwrap();
        assert_eq!(from_server, b"server-reply\xfe\0");
        relay.join().unwrap().unwrap();
    }

    #[test]
    fn independent_splices_run_concurrently() {
        let (left_a, right_a, mut client_a, mut server_a) = pair();
        let (left_b, right_b, mut client_b, mut server_b) = pair();
        let relay_a = std::thread::spawn(move || relay(left_a, right_a));
        let relay_b = std::thread::spawn(move || relay(left_b, right_b));

        client_a.write_all(b"A").unwrap();
        client_a.shutdown(Shutdown::Write).unwrap();
        client_b.write_all(b"B").unwrap();
        client_b.shutdown(Shutdown::Write).unwrap();
        let mut a = Vec::new();
        let mut b = Vec::new();
        server_a.read_to_end(&mut a).unwrap();
        server_b.read_to_end(&mut b).unwrap();
        assert_eq!(a, b"A");
        assert_eq!(b, b"B");
        server_a.shutdown(Shutdown::Write).unwrap();
        server_b.shutdown(Shutdown::Write).unwrap();
        let mut ignored = Vec::new();
        client_a.read_to_end(&mut ignored).unwrap();
        client_b.read_to_end(&mut ignored).unwrap();
        relay_a.join().unwrap().unwrap();
        relay_b.join().unwrap().unwrap();
    }
}
