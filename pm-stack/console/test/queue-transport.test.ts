// The console's own watch on the queue transport's health (#13503
// slice 2, #16242).
//
// Oracles: the retired queue-transport-alert Worker's logic, run and
// status modules (pkgs/queue-transport-alert/worker/src/*.js, removed
// with the Worker in this slice; the contract is quoted in
// queue-transport.ts) — the state machine's threshold/recovery
// semantics, publicStatus's verdict mapping, and runPass's retry
// discipline, exercised with stand-in fetchers and clients. The store
// is the real Edge over a map: a probe pass the stateless part asks
// for is what its health-facing read gives back.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";

// Oracle: Cloudflare versions upload/deploy does not apply Cron Triggers;
// the Actions deploy must explicitly apply the configured schedule afterwards.
test("the Actions version release also applies the probe's Cron Trigger", () => {
  const workflow = readFileSync(new URL("../../../.github/workflows/pm-console-edge-deploy.yml", import.meta.url), "utf8");
  expect(workflow).toMatch(/node release\.mjs --sha "\$GITHUB_SHA"[\s\S]*npx wrangler triggers deploy/);
});
import { Edge } from "../edge/src/edge.ts";
import type { KV } from "../edge/src/edge-shared.ts";
import {
  buildOutagePacket,
  classifyHealthz,
  emptyProbeState,
  INCIDENT_LABELS,
  INCIDENT_TITLE_PREFIX,
  makeIncidentGitHub,
  parseProbeState,
  parseQueueTransport,
  probeBroker,
  QUEUE_TRANSPORT_KEY,
  QUEUE_TRANSPORT_PROBE_KEY,
  QUEUE_TRANSPORT_TOLD_KEY,
  runTransportPass,
  stepProbe,
  transportMessage,
  transportNeedsAttention,
  transportNotifyDecision,
  transportToldMarker,
  transportVerdict,
  type IncidentGitHub,
  type QueueTransport,
  type TransportProbeState,
} from "../edge/src/queue-transport.ts";

const AT = 1_800_000_000_000;
const MIN = 60_000;

describe("parseQueueTransport", () => {
  test("a kept verdict reads back whole", () => {
    const verdict: QueueTransport = {
      status: "down",
      consecutiveFailures: 2,
      failingSince: "2026-11-27T04:40:00Z",
      lastFailure: "HTTP 530",
      lastCheck: "2026-11-27T05:00:00Z",
      lastOk: "2026-11-27T04:30:00Z",
      incident: { number: 7001, url: "https://github.com/developing-today/code/issues/7001" },
      checkedAt: AT,
    };
    expect(parseQueueTransport(JSON.parse(JSON.stringify(verdict)))).toEqual(verdict);
  });

  test("anything else is not a verdict", () => {
    expect(parseQueueTransport(null)).toBeNull();
    expect(parseQueueTransport({})).toBeNull();
    expect(parseQueueTransport({ status: "down", consecutiveFailures: 2 })).toBeNull();
    expect(parseQueueTransport({ status: "down", consecutiveFailures: 2, checkedAt: "now" })).toBeNull();
  });
});

describe("transportNotifyDecision", () => {
  const down: QueueTransport = {
    status: "down",
    consecutiveFailures: 3,
    failingSince: "2026-11-27T04:40:00Z",
    lastFailure: "HTTP 530",
    lastCheck: "2026-11-27T05:00:00Z",
    lastOk: "2026-11-27T04:30:00Z",
    incident: { number: 7001, url: "https://github.com/developing-today/code/issues/7001" },
    checkedAt: AT,
  };
  const up: QueueTransport = { ...down, status: "up", consecutiveFailures: 0, failingSince: null, incident: null };

  test("down and untold notifies; the same outage once told stays quiet", () => {
    expect(transportNeedsAttention(down)).toBe(true);
    expect(transportNotifyDecision(down, null)).toBe("notify");
    expect(transportNotifyDecision(down, transportToldMarker(down))).toBe("quiet");
  });

  test("degraded and stale are attention-worthy; up is not", () => {
    expect(transportNeedsAttention({ ...down, status: "degraded" })).toBe(true);
    expect(transportNeedsAttention({ ...down, status: "stale" })).toBe(true);
    expect(transportNeedsAttention(up)).toBe(false);
    expect(transportNeedsAttention(null)).toBe(false);
  });

  test("well again after told is a recovery; never told and well is quiet", () => {
    expect(transportNotifyDecision(up, transportToldMarker(down))).toBe("recovered");
    expect(transportNotifyDecision(up, null)).toBe("quiet");
  });

  test("missing and unknown verdicts cannot prove recovery", () => {
    const told = transportToldMarker(down);
    expect(transportNotifyDecision(null, told)).toBe("quiet");
    expect(transportNotifyDecision({ ...down, status: "unknown" }, told)).toBe("quiet");
    expect(transportNotifyDecision({ ...down, status: "unexpected" }, told)).toBe("quiet");
  });

  test("the down message names the outage and the incident", () => {
    const message = transportMessage(down);
    expect(message.title).toBe("Queue broker is down");
    expect(message.tag).toBe("queue-transport");
    expect(message.body).toContain("3 failed checks");
    expect(message.body).toContain("#7001");
  });
});

describe("classifyHealthz (the retired Worker's classifyHealthz)", () => {
  test("a 200 whose JSON says ok:true is healthy", () => {
    expect(classifyHealthz(200, JSON.stringify({ ok: true }))).toEqual({ ok: true, detail: "" });
  });

  test("a 200 that does not say ok:true fails, with the body clipped in", () => {
    const res = classifyHealthz(200, JSON.stringify({ ok: false }));
    expect(res.ok).toBe(false);
    expect(res.detail).toContain("not ok:true");
    expect(classifyHealthz(200, "hello").detail).toContain("non-JSON");
  });

  test("an Access refusal reads differently from the tunnel being down", () => {
    expect(classifyHealthz(302, "").detail).toContain("Cloudflare Access refused");
    expect(classifyHealthz(530, "").detail).toContain("tunnel could not reach the broker");
    expect(classifyHealthz(500, "boom").detail).toContain("HTTP 500");
  });
});

describe("probeBroker", () => {
  const config = { brokerUrl: "https://broker.example", accessId: "id", accessSecret: "secret" };

  test("without the Access token the probe is skipped, never an alert", async () => {
    const fetchImpl = (async () => {
      throw new Error("must not be called");
    }) as unknown as typeof fetch;
    expect(await probeBroker(fetchImpl, { brokerUrl: "https://broker.example" })).toEqual({
      skipped: "Access service token not configured",
    });
  });

  test("a healthy answer is ok; the Access headers went with the ask", async () => {
    let seen: Record<string, string> = {};
    const fetchImpl = (async (_url: string, init: { headers: Record<string, string> }) => {
      seen = init.headers;
      return new Response(JSON.stringify({ ok: true }), { status: 200 });
    }) as unknown as typeof fetch;
    expect(await probeBroker(fetchImpl, config)).toEqual({ ok: true, detail: "" });
    expect(seen["CF-Access-Client-Id"]).toBe("id");
    expect(seen["CF-Access-Client-Secret"]).toBe("secret");
  });

  test("a tunnel answer is a failed probe with its detail", async () => {
    const fetchImpl = (async () => new Response("error 1033", { status: 530 })) as unknown as typeof fetch;
    const probe = await probeBroker(fetchImpl, config);
    expect(probe).toMatchObject({ ok: false });
    expect("detail" in probe ? probe.detail : "").toContain("HTTP 530");
  });
});

describe("stepProbe (the retired Worker's step)", () => {
  test("the threshold failure alerts; further failures stay down", () => {
    let s = emptyProbeState();
    let r = stepProbe(s, false, "HTTP 530", AT, 2);
    expect(r.event).toBe("failing");
    s = r.state;
    r = stepProbe(s, false, "HTTP 530", AT + 5 * MIN, 2);
    expect(r.event).toBe("alert");
    expect(r.state.alerted).toBe(true);
    expect(r.state.window_start).toBe(new Date(AT).toISOString().replace(/\.\d{3}Z$/, "Z"));
    r = stepProbe(r.state, false, "HTTP 530", AT + 10 * MIN, 2);
    expect(r.event).toBe("down");
  });

  test("a window that ends below the threshold ends quietly", () => {
    const failed = stepProbe(emptyProbeState(), false, "HTTP 530", AT, 2).state;
    const r = stepProbe(failed, true, "", AT + 5 * MIN, 2);
    expect(r.event).toBe("ok");
    expect(r.state.consecutive_failures).toBe(0);
    expect(r.state.window_start).toBeNull();
  });

  test("the first pass after an alerted window is the recovery", () => {
    let s = stepProbe(emptyProbeState(), false, "x", AT, 2).state;
    s = stepProbe(s, false, "x", AT + 5 * MIN, 2).state;
    const r = stepProbe(s, true, "", AT + 10 * MIN, 2);
    expect(r.event).toBe("recovered");
    expect(r.state.alerted).toBe(true); // the pass itself finishes the recovery
  });
});

function standInGitHub(existing: number | null = null, fail: Partial<Record<"find" | "file" | "comment", boolean>> = {}) {
  const calls: { filed: { title: string; body: string }[]; commented: { number: number; body: string }[] } = {
    filed: [],
    commented: [],
  };
  const github: IncidentGitHub = {
    async find() {
      if (fail.find) throw new Error("search refused");
      return existing;
    },
    async file(title, body) {
      if (fail.file) throw new Error("file refused");
      calls.filed.push({ title, body });
      return 7001;
    },
    async comment(number, body) {
      if (fail.comment) throw new Error("comment refused");
      calls.commented.push({ number, body });
    },
  };
  return { github, calls };
}

const fail = { ok: false, detail: "HTTP 530: the tunnel could not reach the broker" };
const well = { ok: true, detail: "" };

async function outage(github: IncidentGitHub | null, enqueue: ((packet: unknown) => Promise<void>) | null = null) {
  let state: TransportProbeState | null = null;
  state = (await runTransportPass(state, fail, { github, enqueue }, AT)).state;
  return runTransportPass(state, fail, { github, enqueue }, AT + 5 * MIN);
}

describe("runTransportPass (the retired Worker's runPass)", () => {
  test("an alert files one incident, titled and labelled as the Worker's was", async () => {
    const { github, calls } = standInGitHub();
    const pass = await outage(github);
    expect(pass.event).toBe("alert");
    expect(pass.state.alerted).toBe(true);
    expect(pass.state.incident).toBe(7001);
    expect(pass.state.incident_filed_by_probe).toBe(true);
    expect(calls.filed).toHaveLength(1);
    expect(calls.filed[0].title).toStartWith(INCIDENT_TITLE_PREFIX);
    expect(calls.commented).toHaveLength(0);
  });

  test("an alert comments on an open incident instead of filing a second", async () => {
    const { github, calls } = standInGitHub(7009);
    const pass = await outage(github);
    expect(pass.state.incident).toBe(7009);
    expect(pass.state.incident_filed_by_probe).toBe(false);
    expect(calls.filed).toHaveLength(0);
    expect(calls.commented).toHaveLength(1);
    expect(calls.commented[0].number).toBe(7009);
  });

  test("a failed GitHub write leaves the window un-alerted, so the next tick retries", async () => {
    const { github } = standInGitHub(null, { file: true });
    const pass = await outage(github);
    expect(pass.event).toBe("alert");
    expect(pass.state.alerted).toBe(false);
    expect(pass.state.incident).toBeNull();
    expect(pass.errors.length).toBeGreaterThan(0);
  });

  test("with no GitHub client the window still alerts; only the record waits", async () => {
    const pass = await outage(null);
    expect(pass.state.alerted).toBe(true);
    expect(pass.state.incident).toBeNull();
    expect(pass.errors).toEqual([]);
  });

  test("recovery comments once, enqueues the anchored triage packet, and closes the window", async () => {
    const { github, calls } = standInGitHub();
    const packets: unknown[] = [];
    const enqueue = async (packet: unknown) => {
      packets.push(packet);
    };
    const down = await outage(github, enqueue);
    const back = await runTransportPass(down.state, well, { github, enqueue }, AT + 15 * MIN);
    expect(back.event).toBe("recovered");
    expect(calls.commented).toHaveLength(1);
    expect(calls.commented[0].number).toBe(7001);
    expect(calls.commented[0].body).toContain("reachable again");
    expect(packets).toHaveLength(1);
    const packet = packets[0] as { dedupe_key: string; payload: { issue: { number: number }; kind: string } };
    expect(packet.dedupe_key).toBe("repo-health:v1:triage:developing-today/code:issue:7001");
    expect(packet.payload.issue.number).toBe(7001);
    expect(packet.payload.kind).toBe("triage");
    expect(back.state.alerted).toBe(false);
    expect(back.state.last_outage?.incident).toBe(7001);
    expect(back.state.pending).toHaveLength(0);
  });

  test("a failed recovery comment stays alerted and retries on the next pass", async () => {
    const good = standInGitHub();
    const down = await outage(good.github);
    const bad = standInGitHub(null, { comment: true });
    const back = await runTransportPass(down.state, well, { github: bad.github, enqueue: null }, AT + 15 * MIN);
    expect(back.event).toBe("recovered");
    expect(back.state.alerted).toBe(true);
    expect(back.state.pending).toHaveLength(0);
  });

  test("a failed enqueue stays pending and is delivered by a later healthy pass", async () => {
    const { github } = standInGitHub();
    const down = await outage(github);
    const refusing = async () => {
      throw new Error("queue store refused");
    };
    const back = await runTransportPass(down.state, well, { github, enqueue: refusing }, AT + 15 * MIN);
    expect(back.state.pending).toHaveLength(1);
    const packets: unknown[] = [];
    const accepting = async (packet: unknown) => {
      packets.push(packet);
    };
    const later = await runTransportPass(back.state, well, { github, enqueue: accepting }, AT + 20 * MIN);
    expect(later.state.pending).toHaveLength(0);
    expect(packets).toHaveLength(1);
  });
});

describe("transportVerdict (the retired Worker's publicStatus, mapped)", () => {
  test("no probe yet is unknown; a fresh healthy state is up", () => {
    expect(transportVerdict(emptyProbeState(), AT).status).toBe("unknown");
    const s = stepProbe(emptyProbeState(), true, "", AT, 2).state;
    const verdict = transportVerdict(s, AT);
    expect(verdict.status).toBe("up");
    expect(verdict.lastCheck).toBe(s.last_probe);
    expect(verdict.checkedAt).toBe(AT);
  });

  test("failing below threshold is degraded; alerted is down, with its incident", async () => {
    const { github } = standInGitHub();
    const first = await runTransportPass(null, fail, { github, enqueue: null }, AT);
    expect(transportVerdict(first.state, AT).status).toBe("degraded");
    const second = await runTransportPass(first.state, fail, { github, enqueue: null }, AT + 5 * MIN);
    const verdict = transportVerdict(second.state, AT + 5 * MIN);
    expect(verdict.status).toBe("down");
    expect(verdict.incident).toEqual({ number: 7001, url: "https://github.com/developing-today/code/issues/7001" });
    expect(verdict.failingSince).toBe(second.state.window_start);
    expect(verdict.lastFailure).toContain("HTTP 530");
  });

  test("a verdict whose last probe is old reads stale", () => {
    const s = stepProbe(emptyProbeState(), true, "", AT, 2).state;
    expect(transportVerdict(s, AT + 26 * MIN).status).toBe("stale");
  });

  test("kept probe state reads back through parseProbeState", async () => {
    const { github } = standInGitHub();
    const pass = await outage(github);
    const back = parseProbeState(JSON.parse(JSON.stringify(pass.state)));
    expect(back).toEqual(pass.state);
    expect(parseProbeState(null)).toBeNull();
    expect(parseProbeState({ alerted: "yes" })).toBeNull();
  });
});

describe("makeIncidentGitHub", () => {
  function githubFetch(routes: (url: string, init: { method: string; body?: string }) => Response) {
    const seen: { url: string; method: string; body?: string }[] = [];
    const fetchImpl = (async (url: string, init: { method: string; body?: string }) => {
      seen.push({ url, method: init.method, body: init.body });
      return routes(url, init);
    }) as unknown as typeof fetch;
    return { fetchImpl, seen };
  }

  test("find returns the open incident whose title carries the prefix", async () => {
    const { fetchImpl } = githubFetch(() =>
      new Response(
        JSON.stringify({
          items: [
            { title: "Queue transport incident (old format)", number: 5 },
            { title: `${INCIDENT_TITLE_PREFIX} repo-queue broker unreachable since then`, number: 7009 },
          ],
        }),
        { status: 200 },
      ),
    );
    const github = makeIncidentGitHub("token", "developing-today/code", fetchImpl);
    expect(await github.find()).toBe(7009);
  });

  test("file posts the incident with the Worker's labels; comment posts to the issue", async () => {
    const { fetchImpl, seen } = githubFetch((url, init) =>
      init.method === "POST" && url.endsWith("/issues")
        ? new Response(JSON.stringify({ number: 7001 }), { status: 201 })
        : new Response("{}", { status: 201 }),
    );
    const github = makeIncidentGitHub("token", "developing-today/code", fetchImpl);
    expect(await github.file("a title", "a body")).toBe(7001);
    expect(JSON.parse(seen[0].body ?? "{}").labels).toEqual(INCIDENT_LABELS);
    await github.comment(7001, "recovered");
    expect(seen[1].url).toContain("/issues/7001/comments");
    // Deliberately no close: the client surface is find/file/comment only.
    expect(Object.keys(github).sort()).toEqual(["comment", "file", "find"]);
  });
});

describe("buildOutagePacket", () => {
  test("the packet is the Worker's: same dedupe key, priority and anchor", () => {
    const packet = buildOutagePacket(
      { type: "enqueue", incident: 7001, start: "2026-11-27T04:40:00Z", end: "2026-11-27T05:00:00Z" },
      "developing-today/code",
      AT,
    ) as { dedupe_key: string; priority: number; payload: { origin: string; schema: string } };
    expect(packet.dedupe_key).toBe("repo-health:v1:triage:developing-today/code:issue:7001");
    expect(packet.priority).toBe(20);
    expect(packet.payload.schema).toBe("repo-task/v1");
    expect(packet.payload.origin).toBe("queue-transport-alert");
  });
});

describe("the Edge probes the transport itself", () => {
  function makeEdge(initial: "up" | "down" | "skipped") {
    const store = new Map<string, unknown>();
    const kv: KV = {
      get: async (key: string) => store.get(key),
      put: async (key: string, value: unknown) => {
        store.set(key, value);
      },
      delete: async (key: string) => {
        store.delete(key);
      },
    } as KV;
    const alarms: number[] = [];
    const now = { at: AT };
    const mode = { probe: initial };
    const fetchImpl = (async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
      if (url.endsWith("/healthz")) {
        return mode.probe === "up"
          ? new Response(JSON.stringify({ ok: true }), { status: 200 })
          : new Response("error 1033", { status: 530 });
      }
      throw new Error(`unexpected fetch ${url}`);
    }) as unknown as typeof fetch;
    const edge = new Edge({
      kv,
      now: () => now.at,
      setAlarm: async (at) => {
        alarms.push(at);
      },
      fetchImpl,
      transport:
        initial === "skipped"
          ? { brokerUrl: "https://broker.example" }
          : { brokerUrl: "https://broker.example", accessId: "id", accessSecret: "secret" },
    });
    return { edge, store, alarms, now, setProbe: (probe: "up" | "down") => { mode.probe = probe; } };
  }

  const probePass = (edge: Edge) =>
    edge.fetch(new Request("https://pm.internal/internal/queue-transport-probe", { method: "POST" }));

  // Oracle: a scheduled probe alone must drive the notification lifecycle;
  // no host report, owned document clock, connected app or other alarm exists.
  async function notificationEdge(probe: "up" | "down") {
    const w = makeEdge(probe);
    const status = await w.edge.push.status();
    w.edge.push.status = async () => ({ ...status, devices: 1 });
    const messages: string[] = [];
    let failed = false;
    w.edge.push.send = async (_cls, message) => {
      messages.push(message.title);
      return { sent: failed ? 0 : 1, gone: 0, failed: failed ? 1 : 0 };
    };
    return { ...w, messages, fail: (value: boolean) => { failed = value; } };
  }

  test("a healthy probe leaves an up verdict the health read gives back", async () => {
    const { edge, store } = makeEdge("up");
    const res = await probePass(edge);
    expect(res.status).toBe(200);
    const verdict = await edge.lastQueueTransport();
    expect(verdict?.status).toBe("up");
    expect(verdict?.checkedAt).toBe(AT);
    expect(store.get(QUEUE_TRANSPORT_KEY)).toEqual(verdict);
    expect(store.has(QUEUE_TRANSPORT_PROBE_KEY)).toBe(true);
  });

  test("a skipped probe keeps nothing: no verdict, no state", async () => {
    const { edge, store } = makeEdge("skipped");
    const res = await probePass(edge);
    expect(res.status).toBe(200);
    expect(await res.json()).toMatchObject({ skipped: "Access service token not configured" });
    expect(await edge.lastQueueTransport()).toBeNull();
    expect(store.has(QUEUE_TRANSPORT_PROBE_KEY)).toBe(false);
  });

  test("two failed probes tell the owner once; recovery tells him again", async () => {
    const w = await notificationEdge("down");
    await probePass(w.edge);
    expect((await w.edge.lastQueueTransport())?.status).toBe("degraded");
    w.now.at += 5 * MIN;
    await probePass(w.edge);
    const verdict = await w.edge.lastQueueTransport();
    expect(verdict?.status).toBe("down");
    expect(w.alarms.length).toBeGreaterThan(0);
    w.now.at = w.alarms[w.alarms.length - 1];
    w.alarms.length = 0;
    await w.edge.alarm();
    expect(w.messages).toEqual(["Queue broker is down"]);
    expect(w.store.get(QUEUE_TRANSPORT_TOLD_KEY)).toBe(transportToldMarker(verdict!));

    // The same outage probed again stays quiet.
    w.now.at += 5 * MIN;
    await probePass(w.edge);
    await w.edge.alarm();
    expect(w.messages).toEqual(["Queue broker is down"]);

    // The broker answers again: one more pass, and the ending is said too.
    w.setProbe("up");
    w.now.at += 5 * MIN;
    await probePass(w.edge);
    expect((await w.edge.lastQueueTransport())?.status).toBe("up");
    w.alarms.length = 0;
    await w.edge.alarm();
    expect(w.messages).toEqual(["Queue broker is down", "Queue broker is back"]);
    expect(w.store.has(QUEUE_TRANSPORT_TOLD_KEY)).toBe(false);
  });

  test("a failed outage push schedules its own retry", async () => {
    const w = await notificationEdge("down");
    await probePass(w.edge);
    w.now.at += 5 * MIN;
    await probePass(w.edge);
    w.alarms.length = 0;
    w.fail(true);
    await w.edge.alarm();
    expect(w.store.has(QUEUE_TRANSPORT_TOLD_KEY)).toBe(false);
    expect(w.alarms).toEqual([w.now.at + 60_000]);
    w.now.at = w.alarms.pop()!;
    w.fail(false);
    await w.edge.alarm();
    expect(w.store.get(QUEUE_TRANSPORT_TOLD_KEY)).toBeTruthy();
  });
});
