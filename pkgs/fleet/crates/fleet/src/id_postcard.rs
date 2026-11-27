//! The `fleet id` initiator framing, build slice 3 of design #10391
//! (pass 2, §5 slice 3; issue #15435): postcard-native exchange
//! framing (mode 3, values) without the live iroh dial yet.
//!
//! Design §4.4 contract implemented here:
//!
//! - **Postcard-native (fleet ↔ ID):** `postcard::to_allocvec` /
//!   `take_from_bytes` with length-prefixed framing on the stream
//!   (4-byte big-endian length + postcard body; `take_from_bytes`
//!   rejects trailing/padded/concatenated payloads).
//! - **Per-stream 64 KiB default read cap** inherited from the meta
//!   protocol's `read_to_end` bound.
//! - **Enum-position compatibility rule:** postcard encodes enum
//!   variants by position, so variant order is wire-significant.
//!   Adding or reordering variants is a wire break requiring an ALPN
//!   bump (the `/iroh-meta/3` precedent). This module pins the ALPN
//!   string and proves the rule with a reorder fixture.
//!
//! The live dial (`List`/`Find`/tag round-trips against a real test
//! ID node, concurrent-stream multiplexing) rides on top of this
//! framing in the next step; this slice is the strict frame layer
//! plus its tests, independently revertible.

/// The ALPN the initiator (mode 3) speaks. Mirrors
/// [`crate::id_proxy::KNOWN_ALPNS`]'s `/iroh-meta/3` entry and the
/// ID meta protocol's `META_ALPN` (`pkgs/id/crates/id-sync`).
pub const POSTCARD_ALPN: &str = "/iroh-meta/3";

/// Default per-stream read cap: 64 KiB, inherited from the meta
/// protocol's `read_to_end` bound (design §4.3).
pub const PER_STREAM_CAP: usize = 64 * 1024;

/// Length-prefix width: one 4-byte big-endian `u32` before the
/// postcard body.
pub const FRAME_PREFIX_LEN: usize = 4;

/// Why a received frame was refused. Every variant is a hard error:
/// silent fallback or best-effort decode is never attempted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// Fewer than 4 bytes: no length prefix present.
    TooShort,
    /// The length prefix promises more bytes than the buffer holds.
    Truncated { declared: usize, present: usize },
    /// Bytes follow the declared frame: padded, concatenated, or
    /// trailing garbage. Rejected per `take_from_bytes` strictness.
    TrailingBytes { declared: usize, present: usize },
    /// The declared body exceeds [`PER_STREAM_CAP`].
    TooLarge { declared: usize, cap: usize },
    /// Postcard rejected the body (unknown discriminant, bad
    /// encoding, or trailing bytes inside the body).
    Decode(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::TooShort => write!(f, "frame too short: missing 4-byte length prefix"),
            FrameError::Truncated { declared, present } => write!(
                f,
                "frame truncated: prefix declares {declared} body bytes, buffer holds {present}"
            ),
            FrameError::TrailingBytes { declared, present } => write!(
                f,
                "frame has trailing bytes: prefix declares {declared} body bytes, buffer holds {present} (padded/concatenated payloads rejected)"
            ),
            FrameError::TooLarge { declared, cap } => write!(
                f,
                "frame too large: declares {declared} bytes, per-stream cap is {cap}"
            ),
            FrameError::Decode(msg) => write!(f, "postcard decode failed: {msg}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Encode `value` as one length-prefixed frame: 4-byte big-endian
/// body length followed by the postcard body.
pub fn encode_frame<T: serde::Serialize>(value: &T) -> Vec<u8> {
    let body = postcard::to_allocvec(value).expect("postcard encode of Serialize value");
    let mut out = Vec::with_capacity(FRAME_PREFIX_LEN + body.len());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

/// Decode one exact frame (length prefix + body) with no trailing
/// bytes allowed. The body itself must also decode with no trailing
/// bytes (`take_from_bytes` strictness).
pub fn decode_frame_exact<T: serde::de::DeserializeOwned>(frame: &[u8]) -> Result<T, FrameError> {
    if frame.len() < FRAME_PREFIX_LEN {
        return Err(FrameError::TooShort);
    }
    let declared = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    if declared > PER_STREAM_CAP {
        return Err(FrameError::TooLarge {
            declared,
            cap: PER_STREAM_CAP,
        });
    }
    let present = frame.len() - FRAME_PREFIX_LEN;
    if present < declared {
        return Err(FrameError::Truncated { declared, present });
    }
    if present > declared {
        return Err(FrameError::TrailingBytes { declared, present });
    }
    decode_strict(&frame[FRAME_PREFIX_LEN..])
}

/// Decode one bare postcard body with no trailing bytes allowed.
/// Mirrors `id-sync`'s `decode_meta_exact`: `take_from_bytes` plus
/// an explicit empty-remainder check, so padded, concatenated, and
/// trailing-byte payloads all fail instead of silently decoding a
/// prefix.
pub fn decode_strict<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, FrameError> {
    let (value, rest) =
        postcard::take_from_bytes::<T>(bytes).map_err(|e| FrameError::Decode(e.to_string()))?;
    if !rest.is_empty() {
        return Err(FrameError::TrailingBytes {
            declared: bytes.len() - rest.len(),
            present: bytes.len(),
        });
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    /// Minimal stand-in for the meta protocol's value shape: enough
    /// to prove framing strictness without linking `id-sync` (fleet
    /// stays decoupled from ID's protocol crate; the live dial will
    /// reuse this framing against the real `MetaRequest` type).
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    enum Probe {
        List,
        Find { query: String, prefer_name: bool },
        GetTags { subject: Option<Vec<u8>> },
    }

    /// Same payload names in a different order: postcard encodes the
    /// variant by position, so `List` (index 0 here) collides with
    /// `Find` (index 0 there). Decoding across the reorder must NOT
    /// round-trip silently — the mismatch case errors loudly, which
    /// is the failure the ALPN bump exists to force.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    enum Reordered {
        Find { query: String, prefer_name: bool },
        List,
        GetTags { subject: Option<Vec<u8>> },
    }

    /// Same-shape swap: two unit variants in opposite order. Postcard
    /// encodes both as a bare index, so `OrderA::List` ([0]) decodes
    /// as `OrderB::Ping` — silently the WRONG variant. This is the
    /// corruption an ALPN bump prevents: without it, peers would
    /// act on a misread discriminant with no error at all.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    enum OrderA {
        List,
        Ping,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    enum OrderB {
        Ping,
        List,
    }

    #[test]
    fn round_trip_each_variant() {
        for value in [
            Probe::List,
            Probe::Find {
                query: "config".to_string(),
                prefer_name: true,
            },
            Probe::GetTags {
                subject: Some(b"report.txt".to_vec()),
            },
        ] {
            let frame = encode_frame(&value);
            let back: Probe = decode_frame_exact(&frame).expect("exact frame decodes");
            assert_eq!(back, value);
        }
    }

    #[test]
    fn alpn_is_pinned_to_meta_v3() {
        assert_eq!(POSTCARD_ALPN, "/iroh-meta/3");
        assert!(crate::id_proxy::KNOWN_ALPNS.contains(&POSTCARD_ALPN));
    }

    #[test]
    fn padded_body_is_rejected() {
        let mut body = postcard::to_allocvec(&Probe::List).unwrap();
        body.push(0x00);
        assert!(decode_strict::<Probe>(&body).is_err());
    }

    #[test]
    fn concatenated_bodies_are_rejected() {
        let a = postcard::to_allocvec(&Probe::List).unwrap();
        let b = postcard::to_allocvec(&Probe::List).unwrap();
        let mut both = a;
        both.extend_from_slice(&b);
        assert!(decode_strict::<Probe>(&both).is_err());
    }

    #[test]
    fn trailing_byte_inside_frame_is_rejected() {
        let mut frame = encode_frame(&Probe::List);
        frame.push(0x00);
        assert!(matches!(
            decode_frame_exact::<Probe>(&frame),
            Err(FrameError::TrailingBytes { .. })
        ));
    }

    #[test]
    fn truncated_frame_is_rejected() {
        let frame = encode_frame(&Probe::Find {
            query: "config".to_string(),
            prefer_name: false,
        });
        let cut = &frame[..frame.len() - 1];
        assert!(matches!(
            decode_frame_exact::<Probe>(cut),
            Err(FrameError::Truncated { .. })
        ));
    }

    #[test]
    fn over_cap_frame_is_rejected_before_decode() {
        let mut frame = vec![0xFF; FRAME_PREFIX_LEN];
        frame[0] = 0x00;
        frame[1] = 0x02;
        frame[2] = 0x00;
        frame[3] = 0x00;
        assert!(matches!(
            decode_frame_exact::<Probe>(&frame),
            Err(FrameError::TooLarge { .. })
        ));
    }

    #[test]
    fn variant_reorder_does_not_round_trip() {
        // Same-shape swap: silent corruption, caught only because the
        // test asserts the discriminant mapping.
        let bytes = postcard::to_allocvec(&OrderA::List).unwrap();
        let back: OrderB = postcard::from_bytes(&bytes).expect("positional decode succeeds");
        assert_eq!(
            back,
            OrderB::Ping,
            "positional encoding turns List bytes into another variant: ALPN bump required"
        );
        let bytes = postcard::to_allocvec(&OrderA::Ping).unwrap();
        let back: OrderB = postcard::from_bytes(&bytes).expect("positional decode succeeds");
        assert_eq!(
            back,
            OrderB::List,
            "positional encoding turns Ping bytes into another variant: ALPN bump required"
        );
        // Shape mismatch across the reorder: loud decode error, never
        // a silent prefix decode (`Probe::List` is one byte; decoding
        // it as `Reordered::Find` runs out of bytes).
        let bytes = postcard::to_allocvec(&Probe::List).unwrap();
        assert!(postcard::from_bytes::<Reordered>(&bytes).is_err());
    }

    #[test]
    fn frames_decode_independently_for_multiplex_use() {
        let a = encode_frame(&Probe::List);
        let b = encode_frame(&Probe::Find {
            query: "x".to_string(),
            prefer_name: true,
        });
        let back_a: Probe = decode_frame_exact(&a).unwrap();
        let back_b: Probe = decode_frame_exact(&b).unwrap();
        assert_eq!(back_a, Probe::List);
        assert_eq!(
            back_b,
            Probe::Find {
                query: "x".to_string(),
                prefer_name: true,
            }
        );
    }

    // --- Live transport: real iroh QUIC round-trips against a test
    // node. Acceptance for #15435: List/Find/tag-shaped exchanges
    // succeed over ALPN `/iroh-meta/3`, and three concurrent
    // streams over one connection multiplex correctly. Mirrors the
    // `id-sync` meta_client test harness (Minimal endpoints, relay
    // disabled, loopback dial) without linking `id-sync`, so the
    // fleet workspace stays self-contained.

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    enum ProbeResp {
        Entries(Vec<String>),
        Tags(Vec<(Vec<u8>, Vec<u8>)>),
    }

    use std::net::Ipv4Addr;
    use std::time::Duration;

    async fn test_endpoint(alpns: Vec<Vec<u8>>) -> iroh::Endpoint {
        iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .alpns(alpns)
            .relay_mode(iroh::RelayMode::Disabled)
            .bind()
            .await
            .expect("bind test endpoint")
    }

    fn loopback_addr(ep: &iroh::Endpoint) -> iroh::EndpointAddr {
        iroh::EndpointAddr::from_parts(
            ep.id(),
            ep.bound_sockets().iter().map(|addr| {
                let ip = if addr.ip().is_unspecified() {
                    Ipv4Addr::LOCALHOST.into()
                } else {
                    addr.ip()
                };
                iroh::TransportAddr::Ip(std::net::SocketAddr::new(ip, addr.port()))
            }),
        )
    }

    /// Test-node responder: serves three framed `Probe` requests, one
    /// per bi-stream on a single connection, with canned answers.
    /// Returns the first error instead of panicking, so a server-side
    /// failure surfaces as its own cause rather than a client-side
    /// connection-lost.
    fn serve_probes(server: iroh::Endpoint) -> tokio::task::JoinHandle<Result<(), String>> {
        tokio::spawn(async move {
            let incoming = server.accept().await.ok_or("no incoming".to_string())?;
            let conn = incoming.await.map_err(|e| format!("connecting: {e}"))?;
            for i in 0..3 {
                let (mut send, mut recv) = conn
                    .accept_bi()
                    .await
                    .map_err(|e| format!("accept_bi {i}: {e}"))?;
                let bytes = recv
                    .read_to_end(PER_STREAM_CAP)
                    .await
                    .map_err(|e| format!("read request {i}: {e}"))?;
                let req: Probe =
                    decode_frame_exact(&bytes).map_err(|e| format!("decode request {i}: {e}"))?;
                let resp = match req {
                    Probe::List => {
                        ProbeResp::Entries(vec!["alpha".to_string(), "beta".to_string()])
                    }
                    Probe::Find { query, .. } => ProbeResp::Entries(vec![query]),
                    Probe::GetTags { subject } => {
                        ProbeResp::Tags(vec![(subject.unwrap_or_default(), b"seen".to_vec())])
                    }
                };
                send.write_all(&encode_frame(&resp))
                    .await
                    .map_err(|e| format!("write response {i}: {e}"))?;
                send.finish().map_err(|e| format!("finish {i}: {e:?}"))?;
            }
            // Hold the connection open until the peer closes it:
            // dropping `conn` here would race the client's reads with
            // an ApplicationClose and lose in-flight responses. (Same
            // shape as `id-sync`'s test `serve_once`.)
            conn.closed().await;
            Ok(())
        })
    }

    async fn round_trip_on(conn: iroh::endpoint::Connection, req: Probe) -> ProbeResp {
        let (mut send, mut recv) = conn.open_bi().await.expect("open_bi");
        send.write_all(&encode_frame(&req))
            .await
            .expect("write request");
        send.finish().expect("finish");
        let bytes = recv
            .read_to_end(PER_STREAM_CAP)
            .await
            .expect("read response");
        decode_frame_exact(&bytes).expect("strict response decodes")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn live_list_find_tag_round_trips_over_concurrent_streams() {
        tokio::time::timeout(Duration::from_secs(20), async {
            let server = test_endpoint(vec![POSTCARD_ALPN.as_bytes().to_vec()]).await;
            let client = test_endpoint(vec![]).await;
            let addr = loopback_addr(&server);
            let serving = serve_probes(server.clone());
            let conn = client
                .connect(addr, POSTCARD_ALPN.as_bytes())
                .await
                .expect("connect");
            let (list, found, tags) = tokio::join!(
                round_trip_on(conn.clone(), Probe::List),
                round_trip_on(
                    conn.clone(),
                    Probe::Find {
                        query: "config".to_string(),
                        prefer_name: true,
                    }
                ),
                round_trip_on(
                    conn.clone(),
                    Probe::GetTags {
                        subject: Some(b"report.txt".to_vec()),
                    }
                ),
            );
            assert_eq!(
                list,
                ProbeResp::Entries(vec!["alpha".to_string(), "beta".to_string()])
            );
            assert_eq!(found, ProbeResp::Entries(vec!["config".to_string()]));
            assert_eq!(
                tags,
                ProbeResp::Tags(vec![(b"report.txt".to_vec(), b"seen".to_vec())])
            );
            conn.close(0u32.into(), b"done");
            serving
                .await
                .expect("server task")
                .expect("test node served all three streams");
            client.close().await;
            server.close().await;
        })
        .await
        .expect("live round-trips complete within timeout");
    }
}
