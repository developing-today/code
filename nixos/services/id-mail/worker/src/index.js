import email from "./transports/email.js";

export { Limiter } from "./limiter.js";

const TRANSPORTS = { email };

export default {
  async fetch(request, env) {
    if (request.method !== "POST") {
      return json({ error: "method not allowed" }, 405);
    }
    if (!env.CALLERS || !env.MAIL_CONFIG) {
      return json({ error: "worker is not configured" }, 500);
    }
    const config = JSON.parse(env.MAIL_CONFIG);
    const caller = await authenticate(request, env.CALLERS);
    if (!caller) {
      return json({ error: "unauthorized" }, 401);
    }

    let body;
    try {
      body = await request.json();
    } catch {
      return json({ error: "body is not JSON" }, 400);
    }
    if (!body || typeof body !== "object" || Array.isArray(body)) {
      return json({ error: "body must be a JSON object" }, 400);
    }
    const name = body.transport ?? config.defaultTransport;
    if (typeof name !== "string" || !config.transports.includes(name) || !Object.hasOwn(TRANSPORTS, name)) {
      return json({ error: "unknown transport" }, 400);
    }
    if (!Object.hasOwn(caller.transports, name)) {
      return json({ error: "caller may not use this transport" }, 403);
    }
    const transport = TRANSPORTS[name];
    const grant = caller.transports[name];

    const refusal = transport.check(body, grant, config);
    if (refusal) {
      return json({ error: refusal.error }, refusal.status);
    }

    const windowMs = config.limits.windowSeconds * 1000;
    const callerAllowed = await take(env, `caller:${caller.name}`, config.limits.callerPerWindow, windowMs);
    const recipientChecks = await Promise.all(
      transport
        .rateKeys(body)
        .map((key) => take(env, `target:${name}:${key}`, config.limits.recipientPerWindow, windowMs)),
    );
    if (!callerAllowed || !recipientChecks.every(Boolean)) {
      return json({ error: "rate limited" }, 429);
    }

    try {
      return json(await transport.deliver(body, grant, env), 200);
    } catch (error) {
      return json({ error: error.code ?? "send_failed", message: error.message }, 502);
    }
  },
};

async function take(env, key, limit, windowMs) {
  const stub = env.LIMITER.get(env.LIMITER.idFromName(key));
  const response = await stub.fetch("https://limiter.internal/", {
    method: "POST",
    body: JSON.stringify({ limit, windowMs }),
  });
  return (await response.json()).success;
}

async function authenticate(request, callersJson) {
  const match = /^Bearer (.+)$/.exec(request.headers.get("authorization") ?? "");
  if (!match) {
    return null;
  }
  const digest = await sha256Hex(match[1]);
  for (const [name, caller] of Object.entries(JSON.parse(callersJson))) {
    if (sameText(digest, caller.sha256)) {
      return { name, ...caller };
    }
  }
  return null;
}

async function sha256Hex(text) {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function sameText(a, b) {
  if (a.length !== b.length) {
    return false;
  }
  let diff = 0;
  for (let i = 0; i < a.length; i++) {
    diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  }
  return diff === 0;
}

function json(value, status = 200) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });
}
