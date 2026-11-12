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
    const config = parseMailConfig(env.MAIL_CONFIG);
    const callers = parseCallers(env.CALLERS);
    if (!config || !callers) {
      return json({ error: "worker is not configured" }, 500);
    }
    const caller = await authenticate(request, callers);
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
    if (!callerAllowed) {
      // Caller denial is final (#5739): never resolve or mutate recipient
      // Durable Objects for a request that will not be delivered.
      return json({ error: "rate limited" }, 429);
    }
    const recipientChecks = await Promise.all(
      transport
        .rateKeys(body)
        .map((key) => take(env, `target:${name}:${key}`, config.limits.recipientPerWindow, windowMs)),
    );
    if (!recipientChecks.every(Boolean)) {
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

// The MAIL_CONFIG / CALLERS env values are operator-supplied JSON: a
// malformed or wrong-shape value is a configuration error, reported in
// the handler's own vocabulary (500 "worker is not configured"), never
// a raw SyntaxError/TypeError escaping fetch() and never a value the
// handler substring-matches its way into accepting (#6970). Grants are
// validated here too, so transports can trust their shape.
function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isLimit(value) {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function parseJsonObject(raw) {
  let value;
  try {
    value = JSON.parse(raw);
  } catch {
    return null;
  }
  return isPlainObject(value) ? value : null;
}

function parseMailConfig(raw) {
  const config = parseJsonObject(raw);
  if (!config) {
    return null;
  }
  if (
    !Array.isArray(config.transports) ||
    !config.transports.every((name) => typeof name === "string")
  ) {
    return null;
  }
  if (config.defaultTransport !== undefined && typeof config.defaultTransport !== "string") {
    return null;
  }
  if (config.templates !== undefined) {
    if (
      !Array.isArray(config.templates) ||
      !config.templates.every(
        (template) =>
          isPlainObject(template) &&
          typeof template.subject === "string" &&
          typeof template.body === "string",
      )
    ) {
      return null;
    }
  } else {
    config.templates = [];
  }
  if (
    !isPlainObject(config.limits) ||
    !isLimit(config.limits.windowSeconds) ||
    !isLimit(config.limits.callerPerWindow) ||
    !isLimit(config.limits.recipientPerWindow)
  ) {
    return null;
  }
  return config;
}

function parseCallers(raw) {
  const callers = parseJsonObject(raw);
  if (!callers) {
    return null;
  }
  for (const caller of Object.values(callers)) {
    if (!isPlainObject(caller) || typeof caller.sha256 !== "string" || !isPlainObject(caller.transports)) {
      return null;
    }
    for (const grant of Object.values(caller.transports)) {
      if (
        !isPlainObject(grant) ||
        typeof grant.mode !== "string" ||
        !Array.isArray(grant.from) ||
        !grant.from.every((address) => typeof address === "string") ||
        !isLimit(grant.maxRecipients)
      ) {
        return null;
      }
    }
  }
  return callers;
}

async function authenticate(request, callers) {
  const match = /^Bearer (.+)$/.exec(request.headers.get("authorization") ?? "");
  if (!match) {
    return null;
  }
  const digest = await sha256Hex(match[1]);
  for (const [name, caller] of Object.entries(callers)) {
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
