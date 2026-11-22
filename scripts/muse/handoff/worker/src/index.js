const TTL_MS = 15 * 60 * 1000;
const HEADERS = {
  "Cache-Control": "no-store, max-age=0",
  "Pragma": "no-cache",
  "Referrer-Policy": "no-referrer",
  "X-Content-Type-Options": "nosniff",
  "Content-Security-Policy": "default-src 'none'; frame-ancestors 'none'"
};

function reply(status, body) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { ...HEADERS, "Content-Type": "application/json; charset=utf-8" }
  });
}

function decode64(value) {
  const padded = value.replace(/-/g, "+").replace(/_/g, "/");
  return Uint8Array.from(atob(padded + "=".repeat((4 - padded.length % 4) % 4)), c => c.charCodeAt(0));
}

function encode64(value) {
  let binary = "";
  for (const b of new Uint8Array(value)) binary += String.fromCharCode(b);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

async function sha256(text) {
  return encode64(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text)));
}

function equal(a, b) {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

async function readJson(request, limit = 8192) {
  const raw = await request.text();
  if (raw.length > limit) throw new Error("payload too large");
  return JSON.parse(raw);
}

async function cipherKey(env) {
  if (!env.HANDOFF_ENCRYPTION_KEY) throw new Error("not configured");
  const raw = decode64(env.HANDOFF_ENCRYPTION_KEY);
  if (raw.length !== 32) throw new Error("not configured");
  return crypto.subtle.importKey("raw", raw, "AES-GCM", false, ["encrypt", "decrypt"]);
}

async function encrypt(env, value) {
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const key = await cipherKey(env);
  const plain = new TextEncoder().encode(JSON.stringify(value));
  const encrypted = await crypto.subtle.encrypt({ name: "AES-GCM", iv }, key, plain);
  return { iv: encode64(iv), data: encode64(encrypted) };
}

async function decrypt(env, record) {
  const key = await cipherKey(env);
  const plain = await crypto.subtle.decrypt(
    { name: "AES-GCM", iv: decode64(record.iv) },
    key,
    decode64(record.data)
  );
  return JSON.parse(new TextDecoder().decode(plain));
}

function adminAuthorized(request, env) {
  const given = request.headers.get("Authorization") || "";
  const prefix = "Bearer ";
  if (!env.ADMIN_TOKEN || !given.startsWith(prefix)) return false;
  return equal(given.slice(prefix.length), env.ADMIN_TOKEN);
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (request.method === "GET" && url.pathname === "/") {
      return new Response("Muse Fleet handoff service. Use the local handoff generator; never submit credentials in a browser form.", {
        headers: { ...HEADERS, "Content-Type": "text/plain; charset=utf-8" }
      });
    }
    if (request.method !== "POST") return reply(404, { error: "not found" });

    if (url.pathname === "/api/create") {
      if (!adminAuthorized(request, env)) return reply(401, { error: "unauthorized" });
      try {
        const body = await readJson(request);
        const bws = body.bws_access_token;
        const op = body.op_service_account_token;
        const name = body.name;
        if (typeof name !== "string" || !/^[a-z0-9][a-z0-9._-]{0,62}$/.test(name) ||
            typeof bws !== "string" || !bws.trim() || bws.length > 4096 ||
            typeof op !== "string" || !op.trim() || op.length > 4096) {
          return reply(400, { error: "agent name and both credentials are required" });
        }
        const id = crypto.randomUUID();
        const claim = encode64(crypto.getRandomValues(new Uint8Array(32)));
        const record = {
          claim_hash: await sha256(id + ":" + claim),
          expires_at: Date.now() + TTL_MS,
          ...(await encrypt(env, { agent_name: name, bws_access_token: bws, op_service_account_token: op }))
        };
        const stub = env.HANDOFFS.get(env.HANDOFFS.idFromName(id));
        const saved = await stub.fetch("https://handoff.internal/put", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(record)
        });
        if (!saved.ok) return reply(503, { error: "could not store handoff" });
        return reply(201, {
          claim_url: url.origin + "/#" + id + "." + claim,
          expires_in_seconds: TTL_MS / 1000
        });
      } catch {
        return reply(400, { error: "could not create handoff" });
      }
    }

    if (url.pathname === "/api/claim") {
      try {
        const body = await readJson(request, 2048);
        if (typeof body.id !== "string" || !/^[0-9a-f-]{36}$/.test(body.id) ||
            typeof body.claim !== "string" || !/^[A-Za-z0-9_-]{40,48}$/.test(body.claim)) {
          return reply(400, { error: "invalid handoff" });
        }
        const stub = env.HANDOFFS.get(env.HANDOFFS.idFromName(body.id));
        const used = await stub.fetch("https://handoff.internal/claim", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ hash: await sha256(body.id + ":" + body.claim) })
        });
        if (!used.ok) return reply(410, { error: "handoff expired, invalid, or already used" });
        const record = await used.json();
        return reply(200, await decrypt(env, record));
      } catch {
        return reply(410, { error: "handoff expired, invalid, or already used" });
      }
    }

    return reply(404, { error: "not found" });
  }
};

export class Handoff {
  constructor(state) {
    this.state = state;
  }

  async fetch(request) {
    const path = new URL(request.url).pathname;
    if (request.method === "POST" && path === "/put") {
      try {
        const record = await request.json();
        await this.state.storage.transaction(async tx => {
          await tx.put("record", record);
        });
        await this.state.storage.setAlarm(record.expires_at);
        return new Response("stored", { status: 201 });
      } catch {
        return new Response("failed", { status: 500 });
      }
    }
    if (request.method === "POST" && path === "/claim") {
      try {
        const body = await request.json();
        const record = await this.state.storage.transaction(async tx => {
          const current = await tx.get("record");
          if (!current || current.expires_at <= Date.now() ||
              !equal(current.claim_hash, body.hash)) {
            if (current && current.expires_at <= Date.now()) await tx.delete("record");
            return null;
          }
          await tx.delete("record");
          return current;
        });
        if (!record) return new Response("gone", { status: 410 });
        await this.state.storage.deleteAll();
        await this.state.storage.deleteAlarm();
        delete record.claim_hash;
        delete record.expires_at;
        return Response.json(record, { status: 200 });
      } catch {
        return new Response("failed", { status: 410 });
      }
    }
    return new Response("not found", { status: 404 });
  }

  async alarm() {
    await this.state.storage.deleteAll();
    await this.state.storage.deleteAlarm();
  }
}
