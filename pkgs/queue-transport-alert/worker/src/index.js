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
// GET /status returns that state (no secrets) for a quick look.
import { DurableObject } from "cloudflare:workers";
import { makeGitHub } from "./github.js";
import { classifyHealthz, emptyState } from "./logic.js";
import { runPass } from "./run.js";

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
    if (pathname === "/status") {
      return Response.json((await this.ctx.storage.get("state")) || emptyState());
    }
    if (pathname === "/run") {
      return Response.json(await this.run());
    }
    return new Response("not found", { status: 404 });
  }

  async run() {
    const env = this.env;
    const probe = await probeBroker(env);
    if (probe.skipped) {
      console.log(JSON.stringify({ event: "skipped", reason: probe.skipped }));
      return { event: "skipped", reason: probe.skipped };
    }
    const apply = String(env.APPLY || "").toLowerCase() === "true" && Boolean(env.GITHUB_TOKEN);
    const github = apply ? makeGitHub(env.GITHUB_TOKEN, env.REPO) : null;
    const prev = await this.ctx.storage.get("state");
    const result = await runPass(
      prev,
      probe,
      github,
      Date.now(),
      Number(env.THRESHOLD || 2),
      `GET ${env.BROKER_URL}/healthz through Cloudflare Access, from a Cloudflare Worker cron`,
    );
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
    if (request.method === "GET" && pathname === "/status") {
      return stub(env).fetch("https://transport-state/status");
    }
    return new Response("queue-transport-alert: GET /status\n", { status: 404 });
  },
};
