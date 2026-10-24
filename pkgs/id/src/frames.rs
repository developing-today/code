//! Length-prefixed framing for bidirectional Iroh QUIC streams.
//!
//! One frame is a big-endian `u32` byte length followed by the payload bytes.
//! This is shared by the sync protocol (`crate::log_sync`), the envelope
//! protocol (`crate::envelope_net`), and the world session transport
//! (`crate::world_net`); it lives in its own ungated module so the crate
//! builds without the `world` feature.

use anyhow::{Context, Result, bail};
use iroh::endpoint::{ReadExactError, RecvStream, SendStream};

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
    if payload.len() > MAX_FRAME_BYTES {
        bail!("frame of {} bytes exceeds {MAX_FRAME_BYTES}", payload.len());
    }
    let len = u32::try_from(payload.len()).context("frame length")?;
    // One write so a frame is never split by a cancelled caller.
    let mut framed = Vec::with_capacity(4 + payload.len());
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(payload);
    send.write_all(&framed).await.context("write frame")?;
    Ok(())
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
