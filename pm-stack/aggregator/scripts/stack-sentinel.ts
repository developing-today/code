#!/usr/bin/env bun
// Stack sentinel (#8568, notifications Phase 1): a long-running probe
// loop over the PM-stack services. On a state transition it posts to
// the alert throat (POST /api/alerts, the post-alert protocol) with
// the `stackHealth` extension, so the aggregator's sentinel state —
// and therefore its stack-health notifications — tracks reality even
// when the aggregator's own /health probes disagree.
//
// Usage: bun run scripts/stack-sentinel.ts   (loops forever, 30s cadence)
//
// Env: ALERTS_INTAKE_URL (default http://127.0.0.1:4870, as post-alert),
// PM_AGG_STACK_SENTINEL_STATE_FILE (default
// ~/.local/state/pm-aggregator/stack-sentinel-state.json).
//
// The intake bearer is resolved BY NAME from Bitwarden Secrets
// Manager (`pm-aggregator.alerts-intake-token`, via src/secrets.ts)
// and is only ever sent to the aggregator as the Authorization
// header. It is never printed.
//
// Baseline rule: the first run records current states WITHOUT posting
// (no false transitions at startup) — unless a service is actually
// down at first probe, which is a real state and does post. A recorded
// transition whose post has not yet succeeded (postedState !== state)
// is retried every round; the throat dedupes by (source, key).
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { resolveBwsSecret } from "../src/secrets.ts";

const LOOP_MS = 30_000;

interface Probe {
  id: string;
  name: string;
  url: string;
  /** HTTP statuses that mean alive; "any" = any HTTP response at all. */
  expect: number[] | "any";
  timeoutMs: number;
}

// Alive-sets verified live 2026-11-11 (#8568 probe pass):
// - dispatch answers 404 on / — that IS its alive answer.
// - queue-broker binds ONLY the tailnet IP (100.99.113.103), NOT
//   loopback: probing 127.0.0.1:4801 would read permanently down.
//   It answers 401 (auth required) on /; any HTTP response = alive,
//   a connection failure = down.
// - aggregator /board took ~5s when sick, hence the 15s timeout.
const PROBES: Probe[] = [
  { id: "aggregator", name: "Aggregator", url: "http://127.0.0.1:4870/board", expect: [200], timeoutMs: 15_000 },
  { id: "projector", name: "Projector", url: "http://127.0.0.1:7801/health", expect: [200], timeoutMs: 3_000 },
  { id: "kaneo", name: "Kaneo", url: "http://127.0.0.1:5173/api/health", expect: [200], timeoutMs: 3_000 },
  { id: "dispatch", name: "Dispatch", url: "http://127.0.0.1:4879/", expect: [200, 401, 403, 404], timeoutMs: 3_000 },
  { id: "opencode", name: "OpenCode", url: "http://127.0.0.1:13420/global/health", expect: [200], timeoutMs: 3_000 },
  { id: "queue-broker", name: "Queue Broker", url: "http://100.99.113.103:4801/", expect: "any", timeoutMs: 3_000 },
];

type ServiceState = "down" | "up";

interface SentinelServiceState {
  state: ServiceState;
  since: string;
  /** The state last successfully posted to the throat (null = never). */
  postedState: ServiceState | null;
  detail?: string;
}

interface SentinelStateFile {
  services: Record<string, SentinelServiceState>;
}

const STATE_FILE =
  process.env.PM_AGG_STACK_SENTINEL_STATE_FILE?.trim() ||
  join(homedir(), ".local", "state", "pm-aggregator", "stack-sentinel-state.json");

let writeSeq = 0;

async function loadState(): Promise<SentinelStateFile> {
  try {
    const parsed: unknown = JSON.parse(await readFile(STATE_FILE, "utf8"));
    if (parsed && typeof parsed === "object" && (parsed as SentinelStateFile).services) {
      return parsed as SentinelStateFile;
    }
  } catch {
    // no state yet — first run
  }
  return { services: {} };
}

async function saveState(state: SentinelStateFile): Promise<void> {
  await mkdir(dirname(STATE_FILE), { recursive: true });
  const tmp = `${STATE_FILE}.tmp-${process.pid}-${++writeSeq}`;
  await writeFile(tmp, JSON.stringify(state, null, 2));
  await rename(tmp, STATE_FILE);
}

async function probeOne(probe: Probe): Promise<{ state: ServiceState; detail: string }> {
  try {
    const res = await fetch(probe.url, { signal: AbortSignal.timeout(probe.timeoutMs), redirect: "manual" });
    // Drain the body so the socket is released.
    await res.text().catch(() => "");
    if (probe.expect === "any" || probe.expect.includes(res.status)) {
      return { state: "up", detail: `HTTP ${res.status}` };
    }
    return { state: "down", detail: `HTTP ${res.status} — expected ${probe.expect.join(" or ")}` };
  } catch (err) {
    return { state: "down", detail: `probe failed: ${err instanceof Error ? err.message : String(err)}` };
  }
}

async function postTransition(
  probe: Probe,
  entry: SentinelServiceState,
  base: string,
  token: string,
): Promise<boolean> {
  const body = {
    source: "stack-sentinel",
    key: `service:${probe.id}`,
    severity: "warning",
    title:
      entry.state === "down" ? `Stack health: ${probe.name} is down` : `Stack health: ${probe.name} recovered`,
    body: entry.detail ?? "",
    stackHealth: {
      serviceId: probe.id,
      name: probe.name,
      state: entry.state,
      since: entry.since,
      ...(entry.detail ? { detail: entry.detail } : {}),
    },
  };
  try {
    const res = await fetch(`${base}/api/alerts`, {
      method: "POST",
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      body: JSON.stringify(body),
      signal: AbortSignal.timeout(30_000),
    });
    await res.text().catch(() => "");
    return res.ok;
  } catch {
    return false;
  }
}

async function round(state: SentinelStateFile): Promise<void> {
  const base = (process.env.ALERTS_INTAKE_URL ?? "http://127.0.0.1:4870").replace(/\/+$/, "");
  let token: string | null = null;
  let tokenResolved = false;
  for (const probe of PROBES) {
    const { state: current, detail } = await probeOne(probe);
    const now = new Date().toISOString();
    let entry = state.services[probe.id];
    if (!entry) {
      // First sighting: baseline without posting when up; a down
      // service at first probe is a real state and does post.
      entry = { state: current, since: now, postedState: current === "up" ? "up" : null, detail };
      state.services[probe.id] = entry;
    } else if (entry.state !== current) {
      entry = { state: current, since: now, postedState: entry.postedState, detail };
      state.services[probe.id] = entry;
    } else {
      entry.detail = detail;
    }
    if (entry.postedState !== entry.state) {
      if (!tokenResolved) {
        token = await resolveBwsSecret("pm-aggregator.alerts-intake-token");
        tokenResolved = true;
      }
      if (!token) {
        console.error(`stack-sentinel: intake token unavailable; ${probe.id} transition stays pending`);
      } else if (await postTransition(probe, entry, base, token)) {
        entry.postedState = entry.state;
        console.log(`stack-sentinel: posted ${probe.id} -> ${entry.state}`);
      } else {
        console.error(`stack-sentinel: post failed for ${probe.id} -> ${entry.state}; will retry next round`);
      }
    }
  }
  await saveState(state);
}

async function main(): Promise<void> {
  const state = await loadState();
  console.log(`stack-sentinel: probing ${PROBES.length} services every ${LOOP_MS / 1000}s (state: ${STATE_FILE})`);
  for (;;) {
    try {
      await round(state);
    } catch (err) {
      console.error(`stack-sentinel: round failed (${err instanceof Error ? err.message : "error"})`);
    }
    await new Promise((resolve) => setTimeout(resolve, LOOP_MS));
  }
}

await main();
