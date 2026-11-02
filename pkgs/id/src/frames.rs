//! Length-prefixed framing for bidirectional Iroh QUIC streams and SSH
//! exec channels.
//!
//! One frame is a big-endian `u32` byte length followed by the payload bytes.
//! This is shared by the sync protocol (`crate::log_sync`), the envelope
//! protocol (`crate::envelope_net`), and the world session transports
//! (`crate::world_net` over iroh, `crate::world_ssh` over an SSH exec
//! channel); it lives in its own ungated module so the crate builds without
//! the `world` feature. Every writer goes through [`encode_frame`], so a frame
//! has the same envelope bytes on every transport that uses it.

use anyhow::{Context, Result, bail};
use iroh::endpoint::{ReadExactError, RecvStream, SendStream};

use crate::abuse::{AbuseEvaluator, EndpointId, RefusalError};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

/// Largest accepted or emitted frame, in bytes (1 MiB).
///
/// Both transports enforce it: the Iroh frame reader refuses longer frames
/// and the WebSocket has `max_message_size(MAX_FRAME_BYTES)`.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Write one frame.
///
/// # Errors
///
/// Fails if the payload exceeds [`MAX_FRAME_BYTES`] or the stream is closed.
pub async fn write_frame(send: &mut SendStream, payload: &[u8]) -> Result<()> {
    let framed = encode_frame(payload)?;
    // One write so a frame is never split by a cancelled caller.
    send.write_all(&framed).await.context("write frame")?;
    Ok(())
}

/// The envelope bytes of one frame: its big-endian `u32` length, then `payload`.
///
/// # Errors
///
/// Fails if the payload exceeds [`MAX_FRAME_BYTES`].
pub fn encode_frame(payload: &[u8]) -> Result<Vec<u8>> {
    if payload.len() > MAX_FRAME_BYTES {
        bail!("frame of {} bytes exceeds {MAX_FRAME_BYTES}", payload.len());
    }
    let len = u32::try_from(payload.len()).context("frame length")?;
    let mut framed = Vec::with_capacity(4 + payload.len());
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(payload);
    Ok(framed)
}

/// Write one frame to any byte stream (an SSH channel, a test pipe).
///
/// # Errors
///
/// Fails if the payload exceeds [`MAX_FRAME_BYTES`] or the stream is closed.
pub async fn write_frame_to<W: AsyncWrite + Unpin>(send: &mut W, payload: &[u8]) -> Result<()> {
    let framed = encode_frame(payload)?;
    send.write_all(&framed).await.context("write frame")?;
    send.flush().await.context("flush frame")?;
    Ok(())
}

/// Read one frame from any byte stream. `Ok(None)` means the peer finished
/// cleanly between frames.
///
/// This is **not** cancel-safe; run it in its own task, not inside `select!`.
///
/// # Errors
///
/// Fails on a truncated frame, a stream error, or a length above
/// [`MAX_FRAME_BYTES`] (the oversized body is never read).
pub async fn read_frame_from<R: AsyncRead + Unpin>(recv: &mut R) -> Result<Option<Vec<u8>>> {
    let mut len = [0_u8; 4];
    let mut filled = 0;
    while filled < len.len() {
        let read = recv
            .read(&mut len[filled..])
            .await
            .context("read frame length")?;
        if read == 0 {
            if filled == 0 {
                return Ok(None);
            }
            bail!("stream ended inside a frame length");
        }
        filled += read;
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME_BYTES {
        bail!("frame of {len} bytes exceeds {MAX_FRAME_BYTES}");
    }
    let mut body = vec![0_u8; len];
    recv.read_exact(&mut body)
        .await
        .context("read frame body")?;
    Ok(Some(body))
}

/// Splits a byte stream that arrives in arbitrary pieces back into frames,
/// for transports that deliver bytes by callback (the SSH server).
#[derive(Debug, Default)]
pub struct FrameBuffer {
    pending: Vec<u8>,
}

/// One result of [`FrameBuffer::next_frame`].
#[derive(Debug, PartialEq, Eq)]
pub enum Decoded {
    /// A complete frame body.
    Frame(Vec<u8>),
    /// The next frame claims more than [`MAX_FRAME_BYTES`]; the stream is unusable.
    Oversized,
}

impl FrameBuffer {
    /// Append bytes as they arrive.
    pub fn push(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
    }

    /// The next complete frame, if one has fully arrived.
    pub fn next_frame(&mut self) -> Option<Decoded> {
        let header: [u8; 4] = self.pending.get(..4)?.try_into().ok()?;
        let len = u32::from_be_bytes(header) as usize;
        if len > MAX_FRAME_BYTES {
            return Some(Decoded::Oversized);
        }
        if self.pending.len() < 4 + len {
            return None;
        }
        let body = self.pending[4..4 + len].to_vec();
        self.pending.drain(..4 + len);
        Some(Decoded::Frame(body))
    }

    /// Bytes received but not yet returned as a frame.
    #[must_use]
    pub const fn buffered(&self) -> usize {
        self.pending.len()
    }
}

/// Charge one frame of `payload_len` bytes against `endpoint`'s
/// shared bucket in `abuse` (the #2244 P0-A evaluator): one frame
/// token plus the byte cost, admitted or refused as a unit. This is
/// the admission seam every charged transport funnels through — the
/// read path calls it after the length prefix is known and before
/// the body is allocated, the write path before the frame is
/// encoded. A refusal sheds this frame/stream only; nothing here
/// touches the peer connection.
///
/// # Errors
///
/// Returns the evaluator's structured [`RefusalError`] (reason +
/// bounded retry value) when the bucket cannot admit the frame.
pub fn charge_frame(
    abuse: &mut AbuseEvaluator,
    endpoint: EndpointId,
    authorized: bool,
    payload_len: usize,
    now_ms: u64,
) -> Result<(), RefusalError> {
    abuse
        .check_frame_and_bytes(endpoint, authorized, payload_len as u64, now_ms)
        .map_err(RefusalError::from)
}

/// Read the length prefix of one frame from an Iroh stream.
/// `Ok(None)` means the peer finished cleanly between frames. Split
/// from [`read_frame_body`] so a charged reader can charge the frame
/// (see [`charge_frame`]) after the prefix and before allocating the
/// body, without holding any lock across the body read.
///
/// # Errors
///
/// Fails on a stream error or a length above [`MAX_FRAME_BYTES`].
pub async fn read_frame_len(recv: &mut RecvStream) -> Result<Option<u32>> {
    let mut len = [0_u8; 4];
    match recv.read_exact(&mut len).await {
        Ok(()) => {}
        Err(ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(e) => return Err(e).context("read frame length"),
    }
    let len = u32::from_be_bytes(len);
    if len as usize > MAX_FRAME_BYTES {
        bail!("frame of {len} bytes exceeds {MAX_FRAME_BYTES}");
    }
    Ok(Some(len))
}

/// Read the body of one frame whose length prefix ([`read_frame_len`])
/// has already been consumed.
///
/// # Errors
///
/// Fails on a truncated frame or a stream error.
pub async fn read_frame_body(recv: &mut RecvStream, len: u32) -> Result<Vec<u8>> {
    let mut body = vec![0_u8; len as usize];
    recv.read_exact(&mut body)
        .await
        .context("read frame body")?;
    Ok(body)
}

/// Read one frame, charging it against `endpoint`'s shared bucket in
/// `abuse` after the length prefix and before the body is allocated.
/// `Ok(None)` means the peer finished cleanly between frames. The
/// caller owns `abuse` for the duration of the read; transports that
/// share one evaluator across streams charge via [`read_frame_len`] +
/// [`charge_frame`] + [`read_frame_body`] instead, so the shared lock
/// is never held across the body read.
///
/// # Errors
///
/// Fails with the structured [`RefusalError`] when the bucket refuses
/// the frame, and on a truncated frame, a stream error, or a length
/// above [`MAX_FRAME_BYTES`] (the oversized body is never read).
pub async fn read_frame_charged(
    recv: &mut RecvStream,
    abuse: &mut AbuseEvaluator,
    endpoint: EndpointId,
    authorized: bool,
    now_ms: u64,
) -> Result<Option<Vec<u8>>> {
    let Some(len) = read_frame_len(recv).await? else {
        return Ok(None);
    };
    charge_frame(abuse, endpoint, authorized, len as usize, now_ms)?;
    Ok(Some(read_frame_body(recv, len).await?))
}

/// Write one frame, charging it against `endpoint`'s shared bucket in
/// `abuse` before the frame is encoded. A refusal sheds this frame
/// only; the stream stays open.
///
/// # Errors
///
/// Fails with the structured [`RefusalError`] when the bucket refuses
/// the frame, if the payload exceeds [`MAX_FRAME_BYTES`], or if the
/// stream is closed.
pub async fn write_frame_charged(
    send: &mut SendStream,
    abuse: &mut AbuseEvaluator,
    endpoint: EndpointId,
    authorized: bool,
    now_ms: u64,
    payload: &[u8],
) -> Result<()> {
    charge_frame(abuse, endpoint, authorized, payload.len(), now_ms)?;
    write_frame(send, payload).await
}

/// Read one frame. `Ok(None)` means the peer finished cleanly between frames.
///
/// This is **not** cancel-safe; run it in its own task, not inside `select!`.
///
/// # Errors
///
/// Fails on a truncated frame, a stream error, or a length above
/// [`MAX_FRAME_BYTES`] (the oversized body is never read).
pub async fn read_frame(recv: &mut RecvStream) -> Result<Option<Vec<u8>>> {
    let mut len = [0_u8; 4];
    match recv.read_exact(&mut len).await {
        Ok(()) => {}
        Err(ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(e) => return Err(e).context("read frame length"),
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME_BYTES {
        bail!("frame of {len} bytes exceeds {MAX_FRAME_BYTES}");
    }
    let mut body = vec![0_u8; len];
    recv.read_exact(&mut body)
        .await
        .context("read frame body")?;
    Ok(Some(body))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_byte_stream_framing_round_trips_and_ends_cleanly() {
        let (mut client, mut server) = tokio::io::duplex(64);
        let writer = tokio::spawn(async move {
            write_frame_to(&mut client, br#"{"type":"info"}"#)
                .await
                .unwrap();
            write_frame_to(&mut client, b"").await.unwrap();
        });
        assert_eq!(
            read_frame_from(&mut server).await.unwrap().unwrap(),
            br#"{"type":"info"}"#
        );
        assert_eq!(read_frame_from(&mut server).await.unwrap().unwrap(), b"");
        writer.await.unwrap();
        assert!(read_frame_from(&mut server).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn truncated_and_oversized_frames_fail() {
        let (mut client, mut server) = tokio::io::duplex(64);
        client.write_all(&[0, 0]).await.unwrap();
        drop(client);
        assert!(read_frame_from(&mut server).await.is_err());

        let (mut client, mut server) = tokio::io::duplex(64);
        let too_long = u32::try_from(MAX_FRAME_BYTES + 1).unwrap().to_be_bytes();
        client.write_all(&too_long).await.unwrap();
        assert!(read_frame_from(&mut server).await.is_err());
        assert!(encode_frame(&vec![0; MAX_FRAME_BYTES + 1]).is_err());
    }

    #[tokio::test]
    async fn charged_frames_round_trip_and_count_both_directions() {
        use crate::abuse::{AbusePolicy, BucketLimits};
        let policy = AbusePolicy {
            authorized: BucketLimits {
                frames_per_sec: 1,
                frame_burst: 2,
                bytes_per_sec: 8,
                byte_burst: 16,
            },
            ..AbusePolicy::default()
        };
        let mut abuse = AbuseEvaluator::new(policy, [9; 32]);
        let endpoint = [7; 32];
        let (mut client, mut server) = tokio::io::duplex(64);
        // Write path charges via the encode funnel (write_frame_to on a
        // byte stream charges the same seam as write_frame_charged).
        charge_frame(&mut abuse, endpoint, true, 5, 0).unwrap();
        write_frame_to(&mut client, b"hello").await.unwrap();
        // Read path charges after the length prefix, before the body.
        let mut len = [0_u8; 4];
        server.read_exact(&mut len).await.unwrap();
        let len = u32::from_be_bytes(len);
        charge_frame(&mut abuse, endpoint, true, len as usize, 0).unwrap();
        let mut body = vec![0_u8; len as usize];
        server.read_exact(&mut body).await.unwrap();
        assert_eq!(body, b"hello");
        // Both directions recorded: 2 frames, 10 bytes for the pair.
        let snapshot = abuse.counters().snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].1.frames, 2);
        assert_eq!(snapshot[0].1.bytes, 10);
        // The burst (2 frames) is spent: the next charge is refused
        // with a machine-readable retry value.
        let refusal = charge_frame(&mut abuse, endpoint, true, 1, 0).unwrap_err();
        assert!(refusal.0.retry_after_ms.is_some());
    }

    #[test]
    fn byte_refusal_does_not_spend_a_frame_token_or_increment_counters() {
        use crate::abuse::{AbusePolicy, BucketLimits};
        let policy = AbusePolicy {
            authorized: BucketLimits {
                frames_per_sec: 1,
                frame_burst: 2,
                bytes_per_sec: 1,
                byte_burst: 4,
            },
            ..AbusePolicy::default()
        };
        let endpoint = [8; 32];
        let mut abuse = AbuseEvaluator::new(policy, [9; 32]);

        let refusal = charge_frame(&mut abuse, endpoint, true, 5, 0).unwrap_err();
        assert_eq!(
            refusal.0.reason,
            crate::abuse::RefusalReason::BudgetExhausted
        );
        assert!(abuse.counters().get(&[9; 32], &endpoint).is_none());

        // The refused frame did not consume the frame allowance.
        charge_frame(&mut abuse, endpoint, true, 4, 0).unwrap();
        assert_eq!(
            abuse.counters().get(&[9; 32], &endpoint),
            Some(crate::abuse::Counters {
                frames: 1,
                bytes: 4,
            })
        );
    }

    #[test]
    fn the_frame_buffer_reassembles_frames_from_any_split() {
        let mut wire = encode_frame(b"hello").unwrap();
        wire.extend(encode_frame(b"").unwrap());
        wire.extend(encode_frame(b"world").unwrap());
        for split in 0..=wire.len() {
            let mut buffer = FrameBuffer::default();
            let mut frames = Vec::new();
            for piece in [&wire[..split], &wire[split..]] {
                buffer.push(piece);
                while let Some(Decoded::Frame(frame)) = buffer.next_frame() {
                    frames.push(frame);
                }
            }
            assert_eq!(frames, [b"hello".to_vec(), Vec::new(), b"world".to_vec()]);
            assert_eq!(buffer.buffered(), 0);
        }
        let mut buffer = FrameBuffer::default();
        buffer.push(&u32::MAX.to_be_bytes());
        assert_eq!(buffer.next_frame(), Some(Decoded::Oversized));
    }
}
