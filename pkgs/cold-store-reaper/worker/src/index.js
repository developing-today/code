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

import { decompressFrame } from "./decoder.js";

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
    if (cached && cached.expires_at_ms > Date.now() + 60_000) {
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
    const payload = await resp.json();
    const entry = {
      token: payload.access_token,
      expires_at_ms: Date.now() + (payload.expires_in ?? 3600) * 1000,
    };
    await this.state.storage.put("google_token", entry);
    return entry.token;
  }

  async rehydrate(job) {
    const { drive_file_id, offset, frame_length, sha256, bundle_id } = job;
    const token = await this.accessToken();
    const range = `bytes=${offset}-${offset + frame_length - 1}`;
    const resp = await fetch(`${DRIVE_FILES_URL}/${drive_file_id}?alt=media`, {
      headers: { Authorization: `Bearer ${token}`, Range: range },
    });
    if (resp.status !== 206 && resp.status !== 200) {
      throw new HttpError(502, `drive range download failed (HTTP ${resp.status})`);
    }
    let frame = new Uint8Array(await resp.arrayBuffer());
    if (resp.status === 200) {
      // Drive ignored the Range header and sent the whole bundle: slice out
      // exactly the member's frame before doing anything with it.
      frame = frame.slice(offset, offset + frame_length);
    }
    if (frame.length !== frame_length) {
      throw new HttpError(
        502,
        `drive range download short: got ${frame.length} of ${frame_length} bytes`,
      );
    }
    let plain;
    try {
      plain = decompressFrame(frame);
    } catch {
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
