const SENDER = "id@security.cab";

export default {
  async fetch(request, env) {
    if (request.method !== "POST") {
      return json({ error: "method not allowed" }, 405);
    }
    if (!env.MAIL_TOKEN) {
      return json({ error: "worker has no MAIL_TOKEN secret" }, 500);
    }
    if (!sameText(request.headers.get("authorization") ?? "", `Bearer ${env.MAIL_TOKEN}`)) {
      return json({ error: "unauthorized" }, 401);
    }

    let body;
    try {
      body = await request.json();
    } catch {
      return json({ error: "body is not JSON" }, 400);
    }
    const { to, subject, text } = body ?? {};
    if (typeof to !== "string" || typeof subject !== "string" || typeof text !== "string") {
      return json({ error: "to, subject and text must be strings" }, 400);
    }
    if (/[\r\n]/.test(subject) || /[\r\n]/.test(to)) {
      return json({ error: "to and subject must be one line" }, 400);
    }

    try {
      const sent = await env.EMAIL.send({ from: SENDER, to, subject, text });
      return json({ messageId: sent.messageId }, 200);
    } catch (error) {
      return json({ error: error.code ?? "send_failed", message: error.message }, 502);
    }
  },
};

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

function json(value, status) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });
}
