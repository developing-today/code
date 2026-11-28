// Queue transport alert Worker (code#6369).
//
// Every 10 minutes (cron) it probes the repo-queue broker's public
// route, GET <BROKER_URL>/healthz, through Cloudflare Access with a
// service token. That route is a Cloudflare tunnel to the broker's
// Tailscale listener, so it fails when the broker host is off, when
// tailscaled is stopped (the 2026-11-18 outage) or when the broker is
// down. Two failed probes in a row file one GitHub incident (or
// comment on an open one); the first passing probe after that comments
// once. It never closes the incident (#10136): a human does.
//
// State lives in one SQLite-backed Durable Object so passes never race.
// GET / is a status page that can turn on phone alerts (Web Push), and
// GET /status is the same view as JSON (no secrets). GET /version names
// the deployed commit, unauthenticated (#15793 criterion 1).
import { DurableObject } from "cloudflare:workers";
import { makeGitHub } from "./github.js";
import { classifyHealthz, emptyState } from "./logic.js";
import { makeTelegramNotifier } from "./notify.js";
import { makeBrokerEnqueuer } from "./queue.js";
import { runPass } from "./run.js";
import { publicStatus, renderStatusPage } from "./status.js";
import { ICON_SVG, MANIFEST, SERVICE_WORKER } from "./pwa.js";
import { pushAll, validEndpoint, vapidKeys } from "./webpush.js";
import { versionAnswer } from "./version.js";

const MAX_SUBS = 20;
const TEST_EVERY_MS = 30 * 1000;

const UA =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36 queue-transport-alert";

export async function probeBroker(env, fetchImpl = fetch) {
  if (!env.ACCESS_CLIENT_ID || !env.ACCESS_CLIENT_SECRET) {
    return { ok: true, detail: "", skipped: "Access service token not configured" };
  }
  const url = `${String(env.BROKER_URL).replace(/\/+$/, "")}/healthz`;
  try {
    const res = await fetchImpl(url, {
      headers: {
        "CF-Access-Client-Id": env.ACCESS_CLIENT_ID,
        "CF-Access-Client-Secret": env.ACCESS_CLIENT_SECRET,
        "User-Agent": UA,
      },
      redirect: "manual",
      signal: AbortSignal.timeout(Number(env.PROBE_TIMEOUT_MS || 10000)),
    });
    return classifyHealthz(res.status, await res.text());
  } catch (err) {
    const name = err && err.name;
    if (name === "TimeoutError" || name === "AbortError") {
      return { ok: false, detail: `no response from ${url} within the probe timeout` };
    }
    return { ok: false, detail: `request to ${url} failed: ${String(err && err.message ? err.message : err)}` };
  }
}

export class TransportState extends DurableObject {
  async fetch(request) {
    const { pathname } = new URL(request.url);
    const storage = this.ctx.storage;
    if (pathname === "/state") {
      const state = (await storage.get("state")) || emptyState();
      return Response.json({ ...state, last_push: (await storage.get("last_push")) || null });
    }
    if (pathname === "/run") {
      return Response.json(await this.run());
    }
    if (pathname === "/vapid") {
      return Response.json({ publicKey: (await vapidKeys(storage)).publicKey });
    }
    const body = request.method === "POST" ? await request.json().catch(() => ({})) : {};
    const endpoint = validEndpoint(body.endpoint);
    const subs = (await storage.get("push_subs")) || [];
    if (pathname === "/subscribe") {
      if (!endpoint) return new Response("not a browser push endpoint\n", { status: 400 });
      if (subs.some((s) => s.endpoint === endpoint)) return Response.json({ ok: true, subscribed: subs.length });
      if (subs.length >= MAX_SUBS) return new Response("too many devices subscribed\n", { status: 409 });
      subs.push({ endpoint, added: new Date().toISOString() });
      await storage.put("push_subs", subs);
      return Response.json({ ok: true, subscribed: subs.length });
    }
    if (pathname === "/unsubscribe") {
      const left = subs.filter((s) => s.endpoint !== endpoint);
      await storage.put("push_subs", left);
      return Response.json({ ok: true, subscribed: left.length });
    }
    if (pathname === "/test-push") {
      const sub = subs.find((s) => s.endpoint === endpoint);
      if (!sub) return new Response("this device is not subscribed\n", { status: 404 });
      const now = Date.now();
      if (sub.last_test && now - Date.parse(sub.last_test) < TEST_EVERY_MS) {
        return new Response("wait a few seconds between tests\n", { status: 429 });
      }
      sub.last_test = new Date(now).toISOString();
      await storage.put("push_subs", subs);
      const res = await this.push(this.testMessage(), [sub]);
      return Response.json(res, { status: res.sent ? 200 : 502 });
    }
    if (pathname === "/test-all") {
      return Response.json(await this.push(this.testMessage(), subs));
    }
    return new Response("not found", { status: 404 });
  }

  testMessage() {
    return { title: "Queue broker test", body: "Phone alerts work. You'll get one when the broker goes down and one when it's back." };
  }

  /** Sends one Web Push to `subs` (default: every subscribed device),
   * after recording the message the service worker will show. */
  async push(msg, subs = null) {
    const storage = this.ctx.storage;
    const all = (await storage.get("push_subs")) || [];
    const targets = subs || all;
    await storage.put("last_push", { ...msg, at: new Date().toISOString() });
    if (!targets.length) return { sent: 0, gone: [], failed: [], devices: 0 };
    const subject = this.env.PUBLIC_URL || "https://queue-transport-alert.developing-today.workers.dev";
    const res = await pushAll(targets, await vapidKeys(storage), subject, Date.now());
    if (res.gone.length) {
      await storage.put("push_subs", all.filter((s) => !res.gone.includes(s.endpoint)));
    }
    return { ...res, devices: targets.length };
  }

  async run() {
    const env = this.env;
    const probe = await probeBroker(env);
    if (probe.skipped) {
      console.log(JSON.stringify({ event: "skipped", reason: probe.skipped }));
      return { event: "skipped", reason: probe.skipped };
    }
    const apply = String(env.APPLY || "").toLowerCase() === "true";
    const github = apply && env.GITHUB_TOKEN ? makeGitHub(env.GITHUB_TOKEN, env.REPO) : null;
    const notify =
      apply && env.TELEGRAM_BOT_TOKEN && env.TELEGRAM_CHAT_ID
        ? makeTelegramNotifier(env.TELEGRAM_BOT_TOKEN, env.TELEGRAM_CHAT_ID)
        : null;
    const enqueue =
      apply && env.BROKER_AGENT_TOKEN && env.QUEUE
        ? makeBrokerEnqueuer({
            brokerUrl: env.BROKER_URL,
            agentToken: env.BROKER_AGENT_TOKEN,
            accessId: env.ACCESS_CLIENT_ID,
            accessSecret: env.ACCESS_CLIENT_SECRET,
            queue: env.QUEUE,
            repo: env.REPO,
          })
        : null;
    // Web Push to every subscribed device. Throws (so run.js keeps the
    // item and retries) only when no device got it for a reason other
    // than the subscription being gone.
    const push = apply
      ? async (msg) => {
          const res = await this.push(msg);
          if (res.failed.length && !res.sent) throw new Error(`web push: ${res.failed.join("; ")}`);
        }
      : null;
    const prev = await this.ctx.storage.get("state");
    const result = await runPass(
      prev,
      probe,
      github,
      Date.now(),
      Number(env.THRESHOLD || 2),
      `GET ${env.BROKER_URL}/healthz through Cloudflare Access, from a Cloudflare Worker cron`,
      { notify, enqueue, push, repo: env.REPO, statusUrl: env.PUBLIC_URL ? `${env.PUBLIC_URL}/` : "" },
    );
    result.state.last_errors = result.errors;
    await this.ctx.storage.put("state", result.state);
    const log = {
      event: result.event,
      mode: apply ? "apply" : "observe",
      ok: probe.ok,
      detail: probe.detail,
      failures: result.state.consecutive_failures,
      incident: result.incident,
      message: result.message,
      errors: result.errors,
    };
    console.log(JSON.stringify(log));
    return log;
  }
}

function stub(env) {
  return env.TRANSPORT_STATE.get(env.TRANSPORT_STATE.idFromName("broker"));
}

export default {
  async scheduled(_controller, env, ctx) {
    ctx.waitUntil(stub(env).fetch("https://transport-state/run"));
  },
  async fetch(request, env) {
    const { pathname } = new URL(request.url);
    const text = (body, type) =>
      new Response(body, { headers: { "Content-Type": type, "Cache-Control": "no-cache" } });
    if (pathname === "/version") {
      // #15793 criterion 1: unauthenticated, ahead of every other route
      // — it reads no state and no secret.
      if (request.method !== "GET") return new Response("method not allowed\n", { status: 405 });
      return Response.json(versionAnswer(env));
    }
    if (request.method === "POST") {
      if (["/subscribe", "/unsubscribe", "/test-push"].includes(pathname)) {
        const body = await request.text();
        if (body.length > 2048) return new Response("too large\n", { status: 413 });
        return stub(env).fetch(`https://transport-state${pathname}`, { method: "POST", body });
      }
      if (pathname === "/admin/test-push") {
        // Pushes a test to every device; for the operator, authenticated
        // with the probe's Access client secret.
        const given = request.headers.get("X-Admin-Token") || "";
        if (!env.ACCESS_CLIENT_SECRET || !(await sameSecret(given, env.ACCESS_CLIENT_SECRET))) {
          return new Response("forbidden\n", { status: 403 });
        }
        return stub(env).fetch("https://transport-state/test-all", { method: "POST", body: "{}" });
      }
      return new Response("method not allowed\n", { status: 405 });
    }
    if (request.method !== "GET" && request.method !== "HEAD") {
      return new Response("method not allowed\n", { status: 405 });
    }
    if (pathname === "/sw.js") return text(SERVICE_WORKER, "text/javascript; charset=utf-8");
    if (pathname === "/manifest.webmanifest") return text(MANIFEST, "application/manifest+json");
    if (pathname === "/icon.svg") return text(ICON_SVG, "image/svg+xml");
    if (pathname === "/vapid.json") return stub(env).fetch("https://transport-state/vapid");
    if (pathname === "/status" || pathname === "/status.json" || pathname === "/") {
      const res = await stub(env).fetch("https://transport-state/state");
      const view = publicStatus(await res.json(), env, Date.now());
      if (pathname === "/") {
        return new Response(renderStatusPage(view), {
          headers: { "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" },
        });
      }
      return Response.json(view, {
        headers: { "Access-Control-Allow-Origin": "*", "Cache-Control": "no-store" },
      });
    }
    return new Response("queue-transport-alert: GET / or /status\n", { status: 404 });
  },
};

/** Constant-time string comparison via HMAC digests. */
async function sameSecret(a, b) {
  const key = await crypto.subtle.importKey("raw", crypto.getRandomValues(new Uint8Array(32)), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const e = new TextEncoder();
  const [x, y] = await Promise.all([crypto.subtle.sign("HMAC", key, e.encode(a)), crypto.subtle.sign("HMAC", key, e.encode(b))]);
  const u = new Uint8Array(x);
  const v = new Uint8Array(y);
  let diff = 0;
  for (let i = 0; i < u.length; i += 1) diff |= u[i] ^ v[i];
  return diff === 0;
}
