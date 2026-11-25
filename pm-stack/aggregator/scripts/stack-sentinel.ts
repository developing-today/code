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
// The intake bearer <redacted> is resolved BY NAME from Bitwarden Secrets
// Manager (`pm-aggregator.alerts-intake-token`, via src/secrets.ts)
// and is only ever sent to the aggregator as the Authorization
// header. It is never printed.
//
// Flap damping (#9049, src/sentinel-damping.ts): a service flips to
// "down" only after 3 consecutive failed probes (~90s at this
// cadence) — a single slow probe under load never flips state and
// never posts. A recovery posts only after the corresponding down
// was actually posted. Baseline rule: the first run records current
// states WITHOUT posting (no false transitions at startup); a failed
// first probe starts the failure streak like any other, so a service
// that is genuinely down at startup is declared after 3 probes. A
// recorded transition whose post has not yet succeeded
// (postedState !== state) is retried every round; the throat dedupes
// by (source, key).
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { resolveBwsSecret } from "../src/secrets.ts";
import { applySentinelProbe, normalizeSentinelEntry, type SentinelServiceEntry } from "../src/sentinel-damping.ts";
import { checkPrimaryCheckout } from "../src/primary-checkout.ts";
import {
  normalizeHookDispatchState,
  shouldDispatchHealthHook,
  type HealthHookState,
} from "../src/health-hook-dispatch.ts";
import { requireHome } from "../src/home.ts";

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
// Timeouts raised 3s -> 10s (#9049): at load average 8-10 a healthy
// service answers far slower than 3s, and the 3s timeout turned that
// slowness into down/up flapping. 10s gives a slow-but-alive service
// room to answer; the 3-consecutive-failure damping in
// src/sentinel-damping.ts is the primary flap guard.
const PROBES: Probe[] = [
  { id: "aggregator", name: "Aggregator", url: "http://127.0.0.1:4870/board", expect: [200], timeoutMs: 15_000 },
  { id: "projector", name: "Projector", url: "http://127.0.0.1:7801/health", expect: [200], timeoutMs: 10_000 },
  { id: "kaneo", name: "Kaneo", url: "http://127.0.0.1:5173/api/health", expect: [200], timeoutMs: 10_000 },
  { id: "dispatch", name: "Dispatch", url: "http://127.0.0.1:4879/", expect: [200, 401, 403, 404], timeoutMs: 10_000 },
  { id: "opencode", name: "OpenCode", url: "http://127.0.0.1:13420/global/health", expect: [200], timeoutMs: 10_000 },
  { id: "queue-broker", name: "Queue Broker", url: "http://100.99.113.103:4801/", expect: "any", timeoutMs: 10_000 },
];

type ServiceState = "down" | "up";

/** A sentinel target: an HTTP probe or a local check. Transitions post identically. */
interface Target {
  id: string;
  name: string;
}

/**
 * Local (non-HTTP) checks, folded through the same damping and
 * posting machinery as the service probes. The primary-checkout
 * guard (#9849): ~/git/code is a deploy surface kept on a clean main
 * by pm-git-sync; a lane working in it directly blocked deploys
 * twice (#8170 and its recurrence) before anyone noticed. The check
 * names the dirt in its incident detail.
 */
interface LocalCheck extends Target {
  run: () => { state: ServiceState; detail: string };
}

const LOCAL_CHECKS: LocalCheck[] = [
  {
    id: "primary-checkout",
    name: "Primary checkout",
    run: () =>
      checkPrimaryCheckout(
        process.env.PM_PRIMARY_CHECKOUT_DIR?.trim() || join(requireHome("PM_PRIMARY_CHECKOUT_DIR"), "git", "code"),
      ),
  },
];

interface SentinelStateFile {
  services: Record<string, SentinelServiceEntry>;
  /** Last C3 hook transition successfully spawned, independent of alert delivery. */
  hookDispatchedStates: Record<string, HealthHookState>;
}

const STATE_FILE =
  process.env.PM_AGG_STACK_SENTINEL_STATE_FILE?.trim() ||
  join(
    requireHome("PM_AGG_STACK_SENTINEL_STATE_FILE"),
    ".local",
    "state",
    "pm-aggregator",
    "stack-sentinel-state.json",
  );

let writeSeq = 0;

async function loadState(): Promise<SentinelStateFile> {
  try {
    const parsed: unknown = JSON.parse(await readFile(STATE_FILE, "utf8"));
    if (parsed && typeof parsed === "object" && (parsed as SentinelStateFile).services) {
      const services: Record<string, SentinelServiceEntry> = {};
      for (const [id, raw] of Object.entries((parsed as SentinelStateFile).services)) {
        // Entries persisted before #9049 lack the streak fields;
        // normalize fills them in (an unparseable entry is dropped,
        // which re-baselines that service on the next probe).
        const entry = normalizeSentinelEntry(raw);
        if (entry) services[id] = entry;
      }
      return {
        services,
        hookDispatchedStates: normalizeHookDispatchState((parsed as Partial<SentinelStateFile>).hookDispatchedStates),
      };
    }
  } catch {
    // no state yet — first run
  }
  return { services: {}, hookDispatchedStates: {} };
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
  probe: Target,
  entry: SentinelServiceEntry,
  base: string,
  token: string,
): Promise<boolean> {
  const body = {
    source: "stack-sentinel",
    key: `service:${probe.id}`,
    severity: "warning",
    title: entry.state === "down" ? `Stack health: ${probe.name} is down` : `Stack health: ${probe.name} recovered`,
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

// A hook still running for a target holds that target's next dispatch:
// a slow down diagnostic must finish before its recovery runs, or the
// recovery finds no open incident and the late outage is never closed.
const runningHooks = new Map<string, { exitCode: number | null }>();

async function round(state: SentinelStateFile): Promise<void> {
  const base = (process.env.ALERTS_INTAKE_URL ?? "http://127.0.0.1:4870").replace(/\/+$/, "");
  let token: string | null = null;
  let tokenResolved = false;
  const fold = async (target: Target, verdict: { state: ServiceState; detail: string }): Promise<void> => {
    const now = new Date().toISOString();
    // #9049: the damping fold decides whether this verdict changes
    // the service's state at all — a lone failed probe only advances
    // the failure streak recorded on the entry.
    const entry = applySentinelProbe(state.services[target.id], verdict, now);
    state.services[target.id] = entry;
    // C3 health hooks: fire the matching failure-triggered hook on a
    // transition for the two monitored targets. Fire-and-forget; a hook
    // failure never blocks the sentinel round.
    const hookTarget =
      target.id === "aggregator" ? "pm-stack-reachable" : target.id === "opencode" ? "opencode-working" : null;
    if (hookTarget && state.hookDispatchedStates[hookTarget] === undefined && entry.state === "up") {
      // Baseline only; see shouldDispatchHealthHook.
      state.hookDispatchedStates[hookTarget] = "up";
    } else if (
      hookTarget &&
      shouldDispatchHealthHook(state.hookDispatchedStates, hookTarget, entry.state) &&
      runningHooks.get(hookTarget)?.exitCode === null
    ) {
      // Left undispatched, so a later round retries it once the
      // running hook has exited.
      console.log(`stack-sentinel: ${hookTarget} hook still running; ${entry.state} dispatch waits`);
    } else if (hookTarget && shouldDispatchHealthHook(state.hookDispatchedStates, hookTarget, entry.state)) {
      const caller = "stack-sentinel";
      const hookPath = join(
        requireHome("the health hooks directory"),
        "git",
        "code",
        "pm-stack",
        "health-hooks",
        `${hookTarget}.sh`,
      );
      try {
        const proc = Bun.spawn([hookPath, "--caller", caller, "--detail", entry.detail ?? ""], {
          stdout: "ignore",
          stderr: "ignore",
        });
        runningHooks.set(hookTarget, proc);
        state.hookDispatchedStates[hookTarget] = entry.state;
      } catch (err) {
        console.error(
          `stack-sentinel: failed to spawn C3 hook ${hookTarget}: ${err instanceof Error ? err.message : err}`,
        );
      }
    }
    if (entry.postedState !== entry.state) {
      if (!tokenResolved) {
        token = await resolveBwsSecret("pm-aggregator.alerts-intake-token");
        tokenResolved = true;
      }
      if (!token) {
        console.error(`stack-sentinel: intake token unavailable; ${target.id} transition stays pending`);
      } else if (await postTransition(target, entry, base, token)) {
        entry.postedState = entry.state;
        console.log(`stack-sentinel: posted ${target.id} -> ${entry.state}`);
      } else {
        console.error(`stack-sentinel: post failed for ${target.id} -> ${entry.state}; will retry next round`);
      }
    }
  };
  for (const probe of PROBES) {
    await fold(probe, await probeOne(probe));
  }
  for (const check of LOCAL_CHECKS) {
    await fold(check, check.run());
  }
  await saveState(state);
}

async function main(): Promise<void> {
  const state = await loadState();
  console.log(
    `stack-sentinel: probing ${PROBES.length} services + ${LOCAL_CHECKS.length} local checks every ${LOOP_MS / 1000}s (state: ${STATE_FILE})`,
  );
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
