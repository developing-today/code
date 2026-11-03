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
