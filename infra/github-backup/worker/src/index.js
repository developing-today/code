// hatch-repo-backup-webhook — GitHub App webhook receiver + laptop queue.
//
// POST /github        GitHub webhook. Verifies X-Hub-Signature-256 (HMAC
//                     SHA-256 with GITHUB_WEBHOOK_SECRET), stores
//                     {id, repo, event, action, ts} in KV (TTL 7 days).
// GET  /queue         Bearer DRAIN_BEARER. Returns pending events.
// POST /queue/ack     Bearer DRAIN_BEARER. Body {"ids":[...]} deletes them.
// GET  /health        Liveness.
//
// Events are a freshness signal only; the weekly reconcile sweep in
// ../bin/reconcile.py is the correctness floor, so a lost event is not
// lost data. Source of truth: developing-today/code infra/github-backup.

const enc = new TextEncoder();

async function hmacHex(secret, body) {
  const key = await crypto.subtle.importKey(
    "raw", enc.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const sig = await crypto.subtle.sign("HMAC", key, enc.encode(body));
  return [...new Uint8Array(sig)].map(b => b.toString(16).padStart(2, "0")).join("");
}

function authed(req, env) {
  const h = req.headers.get("Authorization") || "";
  return env.DRAIN_BEARER && h === `Bearer ${env.DRAIN_BEARER}`;
}

export default {
  async fetch(req, env) {
    const url = new URL(req.url);
    if (url.pathname === "/health") return new Response("ok");
    if (url.pathname === "/github" && req.method === "POST") {
      const body = await req.text();
      const given = (req.headers.get("X-Hub-Signature-256") || "").replace("sha256=", "");
      const expect = await hmacHex(env.GITHUB_WEBHOOK_SECRET, body);
      let diff = given.length ^ expect.length;
      for (let i = 0; i < expect.length; i++)
        diff |= (given.charCodeAt(i) || 0) ^ expect.charCodeAt(i);
      if (diff !== 0) return new Response("bad signature", { status: 401 });
      let payload = {};
      try { payload = JSON.parse(body); } catch { /* ping has form encoding sometimes */ }
      const event = req.headers.get("X-GitHub-Event") || "unknown";
      if (event === "ping") return new Response("pong");
      const repo = payload.repository && payload.repository.full_name;
      if (!repo) return new Response("no repository in payload", { status: 202 });
      const id = `${Date.now()}-${crypto.randomUUID()}`;
      await env.GHB_EVENTS.put("ev:" + id, JSON.stringify({
        id, repo, event, action: payload.action || null,
        ts: new Date().toISOString(),
      }), { expirationTtl: 7 * 24 * 3600 });
      return new Response("queued", { status: 202 });
    }
    if (url.pathname === "/queue" && req.method === "GET") {
      if (!authed(req, env)) return new Response("unauthorized", { status: 401 });
      const list = await env.GHB_EVENTS.list({ prefix: "ev:", limit: 500 });
      const events = [];
      for (const k of list.keys) {
        const v = await env.GHB_EVENTS.get(k.name);
        if (v) events.push(JSON.parse(v));
      }
      return Response.json({ events });
    }
    if (url.pathname === "/queue/ack" && req.method === "POST") {
      if (!authed(req, env)) return new Response("unauthorized", { status: 401 });
      const { ids } = await req.json();
      for (const id of ids || []) await env.GHB_EVENTS.delete("ev:" + id);
      return Response.json({ acked: (ids || []).length });
    }
    return new Response("not found", { status: 404 });
  },
};
