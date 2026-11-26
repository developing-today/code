// fleet-git-rehost — read-only git smart-HTTP over Workers + R2
// (#8584, Drew's surface pick 2026-11-24: Workers + R2 smart-HTTP).
//
// The publisher (infra/github-backup/bin/rehost_publish.py) writes, per
// repo, one self-contained packfile and a refs.json index into the
// fleet-git-rehost R2 bucket. This Worker is a thin protocol shell
// over those two objects:
//
//   GET  /<owner>/<repo>/info/refs?service=git-upload-pack
//   POST /<owner>/<repo>/git-upload-pack
//
// There is no receive-pack: the rehost is a backup copy, read-only by
// construction. Protocol decisions live in protocol.js (unit-tested);
// this file is only routing, R2 access, and stream plumbing.

import { buildAdvertisement, knownOids, parseUploadPackRequest, pktLine } from "./protocol.js";

const SEGMENT = /^[A-Za-z0-9][A-Za-z0-9._-]*$/;
const MAX_REQUEST_BYTES = 1024 * 1024;
const PUBLISHED_REPOS = [
  "developing-today/code",
  "developing-today/fleet",
  "developing-today-forks/openchamber",
  "developing-today-forks/opencode",
];

const NO_STORE = { "cache-control": "no-store", "referrer-policy": "no-referrer" };

function text(body, status = 200) {
  return new Response(body, {
    status,
    headers: { "content-type": "text/plain; charset=utf-8", ...NO_STORE },
  });
}

function notFound() {
  return text("not found\n", 404);
}

function validRepoParts(parts) {
  return parts.every((p) => SEGMENT.test(p));
}

async function loadDoc(env, owner, repo) {
  const obj = await env.REHOST.get(`${owner}/${repo}/refs.json`);
  if (!obj) return null;
  try {
    return await obj.json();
  } catch {
    return null;
  }
}

async function infoRefs(request, env, owner, repo) {
  const service = new URL(request.url).searchParams.get("service");
  if (service === "git-receive-pack") {
    return text("this rehost is read-only: git-receive-pack is not served\n", 403);
  }
  if (service !== "git-upload-pack") {
    return text("expected ?service=git-upload-pack\n", 400);
  }
  const doc = await loadDoc(env, owner, repo);
  if (!doc) return notFound();
  return new Response(buildAdvertisement(doc), {
    headers: {
      "content-type": "application/x-git-upload-pack-advertisement",
      ...NO_STORE,
    },
  });
}

async function uploadPack(request, env, owner, repo) {
  const doc = await loadDoc(env, owner, repo);
  if (!doc) return notFound();
  // git's HTTP transport gzip-compresses larger upload-pack requests
  // (Content-Encoding: gzip); the pkt-lines live inside the encoding.
  let requestStream = request.body;
  const encoding = (request.headers.get("content-encoding") || "").toLowerCase();
  if (encoding === "gzip" || encoding === "deflate") {
    requestStream = requestStream.pipeThrough(new DecompressionStream(encoding));
  } else if (encoding && encoding !== "identity") {
    return text(`unsupported content-encoding: ${encoding}\n`, 400);
  }
  const body = await new Response(requestStream).arrayBuffer();
  if (body.byteLength > MAX_REQUEST_BYTES) {
    return text("upload-pack request too large\n", 400);
  }
  let parsed;
  try {
    parsed = parseUploadPackRequest(new TextDecoder().decode(body));
  } catch {
    return text("malformed upload-pack request\n", 400);
  }
  if (parsed.wants.length === 0) {
    return text("no wants in upload-pack request\n", 400);
  }
  const known = knownOids(doc);
  const unknown = parsed.wants.filter((oid) => !known.has(oid));
  if (unknown.length > 0) {
    // Protocol-level refusal, in-band, the way git servers report it.
    return new Response(pktLine(`ERR object not in this rehost: ${unknown[0]}\n`), {
      headers: { "content-type": "application/x-git-upload-pack-result", ...NO_STORE },
    });
  }
  const pack = await env.REHOST.get(`${owner}/${repo}/${doc.pack}`);
  if (!pack) {
    return text("published pack is missing; the publisher needs a rerun\n", 500);
  }
  // Response = one NAK pkt-line, then the pack bytes verbatim (no
  // side-band was advertised, so the pack is not framed).
  const nak = new TextEncoder().encode(pktLine("NAK\n"));
  const reader = pack.body.getReader();
  let nakSent = false;
  const stream = new ReadableStream({
    async pull(controller) {
      if (!nakSent) {
        controller.enqueue(nak);
        nakSent = true;
        return;
      }
      const { done, value } = await reader.read();
      if (done) controller.close();
      else controller.enqueue(value);
    },
    cancel() {
      return reader.cancel();
    },
  });
  return new Response(stream, {
    headers: { "content-type": "application/x-git-upload-pack-result", ...NO_STORE },
  });
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const parts = url.pathname.split("/").filter(Boolean);
    if (parts.length === 0) {
      const lines = [
        "fleet-git-rehost: read-only Cloudflare rehost of the fleet repos (#8584).",
        "",
        "Clone with: git clone https://git.security.cab/<owner>/<repo>",
        "",
        "Published repos:",
        ...PUBLISHED_REPOS.map((r) => `  ${r}`),
        "",
      ];
      return text(lines.join("\n"));
    }
    if (parts.length === 2 && validRepoParts(parts)) {
      const [owner, repo] = parts;
      const doc = await loadDoc(env, owner, repo);
      if (!doc) return notFound();
      return text(
        `${owner}/${repo}: HEAD ${doc.head} (${doc.head_ref}), ` +
          `${doc.refs.length} refs, published ${doc.generated_at}.\n` +
          `Clone: git clone https://git.security.cab/${owner}/${repo}\n`,
      );
    }
    if (parts.length === 4 && parts[2] === "info" && parts[3] === "refs") {
      const [owner, repo] = parts;
      if (!validRepoParts([owner, repo])) return notFound();
      if (request.method !== "GET") return text("method not allowed\n", 405);
      return infoRefs(request, env, owner, repo);
    }
    if (parts.length === 3 && parts[2] === "git-upload-pack") {
      const [owner, repo] = parts;
      if (!validRepoParts([owner, repo])) return notFound();
      if (request.method !== "POST") return text("method not allowed\n", 405);
      return uploadPack(request, env, owner, repo);
    }
    if (parts.length === 3 && parts[2] === "git-receive-pack") {
      return text("this rehost is read-only: git-receive-pack is not served\n", 403);
    }
    return notFound();
  },
};
