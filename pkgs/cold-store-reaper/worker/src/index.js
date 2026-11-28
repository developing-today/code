// Cold-store rehydrator Worker (design section 6, v1).
//
//   GET /health          -> { ok: true }                         (no auth)
//   GET /fetch/<sha256>  -> the object's original bytes          (Bearer auth)
//
// The Worker is a thin front door: it looks the object up in the D1
// manifest (binding MANIFEST), then hands the job to a Durable Object
// (binding BUNDLE_FLIGHT) keyed by bundle id, which single-flights all
// concurrent requests for the same bundle. The DO exchanges the Google
// refresh token for an access token (cached in DO storage), Range-pulls
// exactly the member's zstd frame out of the bundle file in Drive,
// decompresses it, and verifies the plaintext sha256 before anything is
// returned. Unverified bytes are never served: any Drive, decompression,
// or hash failure is a 502.
//
// Secrets (set by the operator, never committed): REHYDRATOR_API_KEY,
// GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET, GOOGLE_REFRESH_TOKEN.

import { decompressFrame, ExpansionLimitError } from "./decoder.js";

const TOKEN_URL = "https://oauth2.googleapis.com/token";
const DRIVE_FILES_URL = "https://www.googleapis.com/drive/v3/files";
const FETCH_ROUTE = /^\/fetch\/([0-9a-fA-F]{64})$/;

class HttpError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

function jsonResponse(obj, status = 200) {
  return new Response(JSON.stringify(obj), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function errorResponse(message, status) {
  return jsonResponse({ error: message }, status);
}

async function sha256Hex(bytes) {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

// Read exactly frameLength bytes starting at offset from a response body
// that ignored the Range header (HTTP 200, the whole bundle). Bytes before
// offset are discarded chunk by chunk and the stream is cancelled the
// moment the frame is complete; only the frame itself is ever retained.
// A stream that ends early yields a short array, which the caller's
// exact-length check rejects (fail closed).
export async function readStreamedFrame(resp, offset, frameLength) {
  if (!resp.body || typeof resp.body.getReader !== "function") {
    // No readable stream to bound: fall back to buffering and slicing.
    const whole = new Uint8Array(await resp.arrayBuffer());
    return whole.slice(offset, offset + frameLength);
  }
  const reader = resp.body.getReader();
  const parts = [];
  let skip = offset;
  let remaining = frameLength;
  try {
    while (remaining > 0) {
      const { done, value } = await reader.read();
      if (done) break;
      if (skip >= value.length) {
        skip -= value.length;
        continue;
      }
      const take = value.subarray(skip, skip + remaining);
      parts.push(take);
      remaining -= take.length;
      skip = 0;
    }
  } finally {
    // Stop the download here: never pull the rest of the bundle.
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
  const frame = new Uint8Array(frameLength - remaining);
  let pos = 0;
  for (const part of parts) {
    frame.set(part, pos);
    pos += part.length;
  }
  return frame;
}

// ---------------------------------------------------------------------------
// Durable Object: one instance per bundle id. Concurrent fetches for the
// same bundle funnel through here; an in-flight map joins identical member
// requests onto one Drive pull instead of stampeding Drive per request.
// ---------------------------------------------------------------------------
export class BundleFlight {
  constructor(state, env) {
    this.state = state;
    this.env = env;
    this.inflight = new Map(); // sha256 -> Promise<{bytes, bundleId, sha256}>
  }

  async fetch(request) {
    let job;
    try {
      job = await request.json();
    } catch {
      return errorResponse("invalid JSON job body", 400);
    }
    try {
      let pending = this.inflight.get(job.sha256);
      if (!pending) {
        pending = this.rehydrate(job).finally(() => {
          this.inflight.delete(job.sha256);
        });
        this.inflight.set(job.sha256, pending);
      }
      const result = await pending;
      // A fresh Response per caller: a Response body can be consumed once,
      // so joined callers must not share one instance.
      return new Response(result.bytes, {
        status: 200,
        headers: {
          "Content-Type": "application/octet-stream",
          "Content-Length": String(result.bytes.length),
          "X-Cold-Store-Sha256": result.sha256,
          "X-Cold-Store-Bundle": result.bundleId,
        },
      });
    } catch (err) {
      if (err instanceof HttpError) {
        return errorResponse(err.message, err.status);
      }
      return errorResponse(`rehydration failed: ${err.message}`, 502);
    }
  }

  async accessToken() {
    const cached = await this.state.storage.get("google_token");
    // A cached entry is trusted only with a usable token (#6989): an
    // entry poisoned before this validation existed (or written by
    // anything else) is treated as absent, so it self-heals by
    // re-exchanging instead of serving the bogus token until expiry.
    if (
      cached &&
      typeof cached.token === "string" &&
      cached.token !== "" &&
      cached.expires_at_ms > Date.now() + 60_000
    ) {
      return cached.token;
    }
    const form = new URLSearchParams({
      client_id: this.env.GOOGLE_CLIENT_ID,
      client_secret: this.env.GOOGLE_CLIENT_SECRET,
      refresh_token: this.env.GOOGLE_REFRESH_TOKEN,
      grant_type: "refresh_token",
    });
    const resp = await fetch(TOKEN_URL, {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: form,
    });
    if (!resp.ok) {
      throw new HttpError(502, `google token exchange failed (HTTP ${resp.status})`);
    }
    // Validate the 200 body BEFORE caching it (#6989): the parsed
    // result is written to Durable Object storage and trusted by the
    // early return above, so one wrong-shape 200 (proxy error-page
    // JSON, a Google-side shape change) would otherwise poison every
    // rehydration on this bundle DO until the bogus expiry passed.
    // A rejected response writes nothing, so the next call
    // re-exchanges. The Python sibling shaped this same boundary in
    // #6846.
    let payload;
    try {
      payload = await resp.json();
    } catch {
      throw new HttpError(
        502,
        "google token exchange failed (response body was not JSON)",
      );
    }
    if (
      typeof payload !== "object" ||
      payload === null ||
      Array.isArray(payload) ||
      typeof payload.access_token !== "string" ||
      payload.access_token === ""
    ) {
      throw new HttpError(
        502,
        "google token exchange failed (response did not contain an access_token)",
      );
    }
    const expiresIn =
      typeof payload.expires_in === "number" &&
      Number.isFinite(payload.expires_in) &&
      payload.expires_in > 0
        ? payload.expires_in
        : 3600;
    const entry = {
      token: payload.access_token,
      expires_at_ms: Date.now() + expiresIn * 1000,
    };
    await this.state.storage.put("google_token", entry);
    return entry.token;
  }

  async rehydrate(job) {
    const { drive_file_id, offset, frame_length, sha256, bundle_id } = job;
    // The manifest's expected plaintext size (selected as o.size by the
    // entry point) bounds decompression (#6590). A job without a usable
    // size fails closed here, before any fetch: falling back to an
    // unbounded decompress is exactly the failure being fixed.
    const expectedSize =
      typeof job.size === "string" && job.size.trim() !== "" ? Number(job.size) : job.size;
    if (!Number.isSafeInteger(expectedSize) || expectedSize < 0) {
      throw new HttpError(
        502,
        "manifest entry has no usable expected size — refusing unbounded decompression",
      );
    }
    const token = await this.accessToken();
    const range = `bytes=${offset}-${offset + frame_length - 1}`;
    const resp = await fetch(`${DRIVE_FILES_URL}/${drive_file_id}?alt=media`, {
      headers: { Authorization: `Bearer ${token}`, Range: range },
    });
    if (resp.status !== 206 && resp.status !== 200) {
      throw new HttpError(502, `drive range download failed (HTTP ${resp.status})`);
    }
    let frame;
    if (resp.status === 200) {
      // Drive ignored the Range header and sent the whole bundle: stream
      // it, discarding everything before the frame and stopping as soon
      // as the frame is complete, so peak memory tracks frame_length
      // rather than the bundle size (issue #6182).
      frame = await readStreamedFrame(resp, offset, frame_length);
    } else {
      frame = new Uint8Array(await resp.arrayBuffer());
    }
    if (frame.length !== frame_length) {
      throw new HttpError(
        502,
        `drive range download short: got ${frame.length} of ${frame_length} bytes`,
      );
    }
    let plain;
    try {
      plain = decompressFrame(frame, expectedSize);
    } catch (err) {
      if (err instanceof ExpansionLimitError) {
        // The decoder's message names the exact bound that fired
        // (declared content size, declared window, produced output).
        throw new HttpError(502, err.message);
      }
      throw new HttpError(502, "zstd frame decompression failed");
    }
    const digest = await sha256Hex(plain);
    if (digest !== String(sha256).toLowerCase()) {
      throw new HttpError(502, "sha256 mismatch — refusing to return unverified bytes");
    }
    return { bytes: plain, bundleId: bundle_id, sha256: digest };
  }
}

// ---------------------------------------------------------------------------
// Worker entry point.
// ---------------------------------------------------------------------------
export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    if (request.method === "GET" && url.pathname === "/health") {
      return jsonResponse({ ok: true });
    }

    const match = url.pathname.match(FETCH_ROUTE);
    if (request.method === "GET" && match) {
      if (
        !env.REHYDRATOR_API_KEY ||
        request.headers.get("Authorization") !== `Bearer ${env.REHYDRATOR_API_KEY}`
      ) {
        return errorResponse("unauthorized", 401);
      }
      const sha = match[1].toLowerCase();
      const row = await env.MANIFEST.prepare(
        "SELECT o.hash AS sha256, o.size AS size, m.offset AS offset," +
          " m.length AS frame_length, m.bundle_id AS bundle_id," +
          " b.drive_file_id AS drive_file_id" +
          " FROM objects o" +
          " JOIN members m ON m.object_hash = o.hash" +
          " JOIN bundles b ON b.bundle_id = m.bundle_id" +
          " WHERE o.hash = ? AND b.state = 'verified'" +
          " ORDER BY b.uploaded_at DESC LIMIT 1",
      )
        .bind(sha)
        .first();
      if (!row) {
        return errorResponse("unknown object", 404);
      }
      const stub = env.BUNDLE_FLIGHT.get(env.BUNDLE_FLIGHT.idFromName(row.bundle_id));
      return stub.fetch("https://bundle-flight/rehydrate", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(row),
      });
    }

    return errorResponse("not found", 404);
  },
};
