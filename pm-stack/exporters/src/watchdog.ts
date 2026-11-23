// Quota watchdog oneshot entry point (#6032, design #6031).
//
// Slice 1 (watchdog-state.ts) decides; the rung-4 module
// (watchdog-escalation.ts) plans hand-offs; THIS file owns everything
// with side effects, in one tick:
//
//   1. load adapters (adapters.ts — provider specifics live there);
//   2. read the persisted state file (never trust it: coerceHealth);
//   3. observe each source — producer document + gatherer heartbeat +
//      a live CDP liveness probe — and classify the observation,
//      including the stale-read case (producer silent while the
//      browser lives: the back half of the 2026-11-13→16 outage);
//   4. `decide` the ladder step, execute rungs 2/3 against the real
//      browser when live repairs are enabled, record outcomes;
//   5. run rung-4 escalation / recovery effects when the decision
//      layer asks (production effectors shell to the fleet CLI / gh,
//      and only when QUOTA_WATCHDOG_LIVE=1 — otherwise the plan is
//      journalled as a dry run and nothing is created);
//   6. write the state file back ATOMICALLY (tmp + rename) and append
//      one history line per source. Nothing decision-relevant lives
//      in process memory (#5928): the next tick is a fresh process
//      that re-reads all of it from disk.
//
// Quiet when healthy: a healthy tick prints nothing beyond the
// surfacing lines of incidents that need a human (surfacingLine).
//
// Usage (bun):
//   bun watchdog.ts [--adapters-dir DIR] [--state-file F]
//                   [--history-file F] [--heartbeat-file F]
// Defaults follow the laptop layout; every path is overridable so
// tests and fixture demos run against a scratch tree.

import { appendFile, mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { homedir } from "node:os";
import {
  coerceHealth,
  decide,
  emptyState,
  initialHealth,
  recordRungOutcome,
  sanitizeDetail,
  type FailureClass,
  type Observation,
  type Rung,
  type SourceHealth,
  type WatchdogState,
} from "./watchdog-state.ts";
import {
  applyEscalationActions,
  buildQueueTaskWrapper,
  EscalationPartialError,
  planEscalation,
  planRecovery,
  recordEscalation,
  recordReEscalation,
  surfacingLine,
  type EscalationContext,
  type EscalationEffectors,
} from "./watchdog-escalation.ts";
import { loadAdapters, type WatchdogAdapter } from "./adapters.ts";

// ---------------------------------------------------------------------------
// Effects — everything the tick can do to the world, behind one seam so
// tests drive the whole tick with fakes and no browser, queue, or network.
// ---------------------------------------------------------------------------

export interface RungOutcome {
  outcome: "succeeded" | "failed" | "skipped";
  detail: string;
}

export interface WatchdogEffects {
  /** Live CDP liveness: does the browser's /json/version answer? */
  probeCdp(port: number): Promise<{ ok: boolean; detail: string }>;
  /** Rung 2: re-validate the CDP session (browser alive, an allowlist
   *  page open AND its renderer answering — the wedge probe — and the
   *  adapter cookies present BY NAME). */
  rung2(adapter: WatchdogAdapter): Promise<RungOutcome>;
  /** Rung 3: re-login driver via the provider's own SSO flow. Stops
   *  dead at a human-only challenge. */
  rung3(adapter: WatchdogAdapter): Promise<RungOutcome>;
  /** Rung 4 effectors (#6033). */
  escalation: EscalationEffectors;
  log(line: string): void;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function fetchJson(url: string, timeoutMs: number, init?: RequestInit): Promise<unknown> {
  const ctl = new AbortController();
  const t = setTimeout(() => ctl.abort(), timeoutMs);
  try {
    const res = await fetch(url, { ...init, signal: ctl.signal });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    return await res.json();
  } finally {
    clearTimeout(t);
  }
}

/** Cookie NAMES present on any of the adapter's origins, via the CDP
 *  browser websocket. Values are never read into this process: the
 *  protocol reply is reduced to a name set before anything is kept. */
async function cdpCookieNames(port: number, origins: string[], timeoutMs = 5000): Promise<string[] | null> {
  let version: { webSocketDebuggerUrl?: string };
  try {
    version = (await fetchJson(`http://127.0.0.1:${port}/json/version`, timeoutMs)) as typeof version;
  } catch {
    return null;
  }
  if (!version.webSocketDebuggerUrl) return null;
  return await new Promise<string[] | null>((resolve) => {
    const ws = new WebSocket(version.webSocketDebuggerUrl!);
    const timer = setTimeout(() => {
      try {
        ws.close();
      } catch {
        /* already closed */
      }
      resolve(null);
    }, timeoutMs);
    ws.onopen = () => {
      ws.send(JSON.stringify({ id: 1, method: "Storage.getCookies" }));
    };
    ws.onmessage = (ev) => {
      try {
        const msg = JSON.parse(String(ev.data)) as {
          id?: number;
          result?: { cookies?: { name: string; domain: string }[] };
        };
        if (msg.id !== 1) return;
        clearTimeout(timer);
        ws.close();
        const cookies = msg.result?.cookies ?? [];
        const names = new Set<string>();
        for (const c of cookies) {
          // Keep only cookies scoped to an allowlist origin's host.
          if (
            origins.some((o) => {
              try {
                return (
                  new URL(o).hostname.endsWith(c.domain.replace(/^\./, "")) || c.domain.endsWith(new URL(o).hostname)
                );
              } catch {
                return false;
              }
            })
          )
            names.add(c.name);
        }
        resolve([...names]);
      } catch {
        clearTimeout(timer);
        resolve(null);
      }
    };
    ws.onerror = () => {
      clearTimeout(timer);
      resolve(null);
    };
  });
}

/** One /json/list target, reduced to the fields the watchdog uses. */
interface CdpTarget {
  type?: string;
  url?: string;
  webSocketDebuggerUrl?: string;
}

/** The allowlist page target rung 2 should talk to: a page (not a
 *  background page, service worker, or other non-page target) on one
 *  of the adapter's origins, with a debugger websocket. Pure — the
 *  wedge probe's target choice is testable without a browser. */
export function pickAllowlistTarget(targets: CdpTarget[], origins: string[]): CdpTarget | null {
  return (
    targets.find(
      (t) =>
        (t.type === undefined || t.type === "page") &&
        typeof t.webSocketDebuggerUrl === "string" &&
        t.webSocketDebuggerUrl.length > 0 &&
        origins.some((o) => (t.url ?? "").startsWith(o)),
    ) ?? null
  );
}

/** Wedge probe (#10630): does the page's renderer answer a trivial
 *  Runtime.evaluate over its own debugger websocket? A hung renderer
 *  leaves every browser-level endpoint (/json/version, /json/list,
 *  Storage.getCookies) answering while the page itself never replies
 *  — the silent shape of the 2026-11-13→16 outage. The reply's value
 *  is discarded; answering at all is the signal. */
export async function cdpPageResponsive(
  webSocketDebuggerUrl: string,
  timeoutMs = 5000,
): Promise<{ ok: boolean; detail: string }> {
  return await new Promise((resolve) => {
    let settled = false;
    const done = (r: { ok: boolean; detail: string }) => {
      if (!settled) {
        settled = true;
        resolve(r);
      }
    };
    let ws: WebSocket;
    try {
      ws = new WebSocket(webSocketDebuggerUrl);
    } catch (e) {
      done({ ok: false, detail: `page websocket unusable: ${e instanceof Error ? e.message : String(e)}` });
      return;
    }
    const timer = setTimeout(() => {
      try {
        ws.close();
      } catch {
        /* already closed */
      }
      done({ ok: false, detail: `page unresponsive: Runtime.evaluate not answered within ${timeoutMs} ms` });
    }, timeoutMs);
    ws.onopen = () => {
      ws.send(
        JSON.stringify({ id: 1, method: "Runtime.evaluate", params: { expression: "1", returnByValue: true } }),
      );
    };
    ws.onmessage = (ev) => {
      try {
        const msg = JSON.parse(String(ev.data)) as { id?: number; error?: { message?: string } };
        if (msg.id !== 1) return;
        clearTimeout(timer);
        try {
          ws.close();
        } catch {
          /* already closed */
        }
        if (msg.error) done({ ok: false, detail: `page evaluate errored: ${msg.error.message ?? "unknown CDP error"}` });
        else done({ ok: true, detail: "page renderer answered Runtime.evaluate" });
      } catch {
        /* not our reply frame — keep waiting for the reply or the timeout */
      }
    };
    ws.onerror = () => {
      clearTimeout(timer);
      done({ ok: false, detail: "page websocket errored before answering" });
    };
  });
}

async function systemctlUser(args: string[], timeoutMs = 30000): Promise<{ ok: boolean; detail: string }> {
  try {
    const proc = Bun.spawn(["systemctl", "--user", ...args], { stdout: "pipe", stderr: "pipe" });
    const timer = setTimeout(() => proc.kill(), timeoutMs);
    const code = await proc.exited;
    clearTimeout(timer);
    const err = (await new Response(proc.stderr).text()).trim().slice(0, 200);
    return { ok: code === 0, detail: err || `exit ${code}` };
  } catch (e) {
    return { ok: false, detail: e instanceof Error ? e.message : String(e) };
  }
}

/** Production effects. Rung 2/3 touch the browser only when
 *  QUOTA_WATCHDOG_LIVE_REPAIRS=1 (the systemd unit sets it); without
 *  it they report `skipped`, which the ladder records honestly. */
export function productionEffects(env: Record<string, string | undefined> = process.env): WatchdogEffects {
  const liveRepairs = env.QUOTA_WATCHDOG_LIVE_REPAIRS === "1";
  const liveEscalation = env.QUOTA_WATCHDOG_LIVE === "1";
  const probeCdp = async (port: number) => {
    try {
      const v = (await fetchJson(`http://127.0.0.1:${port}/json/version`, 3000)) as { Browser?: string };
      return { ok: true, detail: `CDP answered (${v.Browser ?? "browser"})` };
    } catch (e) {
      return { ok: false, detail: `CDP /json/version unreachable: ${e instanceof Error ? e.message : String(e)}` };
    }
  };
  return {
    probeCdp,
    async rung2(adapter) {
      if (!adapter.cdpPort) return { outcome: "skipped", detail: "adapter declares no CDP port" };
      if (!liveRepairs)
        return { outcome: "skipped", detail: "live repairs not enabled (QUOTA_WATCHDOG_LIVE_REPAIRS != 1)" };
      const probe = await probeCdp(adapter.cdpPort);
      if (!probe.ok) {
        // Browser down: one bounded supervised restart, then re-probe.
        if (adapter.browserUnit) {
          const rs = await systemctlUser(["restart", adapter.browserUnit]);
          if (rs.ok) {
            await sleep(3000);
            const again = await probeCdp(adapter.cdpPort);
            if (again.ok)
              return { outcome: "succeeded", detail: `browser unit ${adapter.browserUnit} restarted; ${again.detail}` };
            return { outcome: "failed", detail: `browser unit restarted but CDP still unreachable: ${again.detail}` };
          }
          return {
            outcome: "failed",
            detail: `CDP unreachable and restart of ${adapter.browserUnit} failed: ${rs.detail}`,
          };
        }
        return { outcome: "failed", detail: probe.detail };
      }
      // Browser answers at the endpoint. Validate the session in
      // layers: an allowlist page is open, its renderer answers an
      // evaluate (the wedge probe — browser-level endpoints answer
      // even when the page is hung), and the adapter's cookies are
      // present by name.
      const validate = async (): Promise<{ ok: boolean; wedged: boolean; detail: string }> => {
        let targets: CdpTarget[] = [];
        try {
          targets = (await fetchJson(`http://127.0.0.1:${adapter.cdpPort}/json/list`, 3000)) as CdpTarget[];
        } catch (e) {
          return {
            ok: false,
            wedged: false,
            detail: `CDP /json/list failed: ${e instanceof Error ? e.message : String(e)}`,
          };
        }
        if (adapter.origins.length > 0) {
          const target = pickAllowlistTarget(targets, adapter.origins);
          if (!target) {
            const anyOpen = targets.some((t) => adapter.origins.some((o) => (t.url ?? "").startsWith(o)));
            return {
              ok: false,
              wedged: false,
              detail: anyOpen
                ? "allowlist page is open but exposes no debugger websocket — cannot probe it"
                : "no tab on an adapter allowlist origin",
            };
          }
          const page = await cdpPageResponsive(target.webSocketDebuggerUrl!);
          if (!page.ok) return { ok: false, wedged: true, detail: `browser wedged: ${page.detail}` };
        }
        if (adapter.cookieNames.length > 0) {
          const names = await cdpCookieNames(adapter.cdpPort, adapter.origins);
          if (names === null)
            return { ok: false, wedged: false, detail: "cookie presence check could not run (CDP websocket)" };
          const missing = adapter.cookieNames.filter((n) => !names.includes(n));
          if (missing.length > 0)
            return { ok: false, wedged: false, detail: `cookies missing by name: ${missing.join(", ")}` };
        }
        return {
          ok: true,
          wedged: false,
          detail: "CDP session re-validated: browser live, allowlist page responsive, cookies present by name",
        };
      };
      let v = await validate();
      if (!v.ok && v.wedged && adapter.browserUnit) {
        // Wedged browser: one bounded supervised restart, then the
        // FULL validation again — a restart that leaves the page
        // hung is a failure, not a repair. (Recovery is only real
        // when a later tick observes a fresh producer read; that is
        // the recovery path's job, not this rung's claim.)
        const rs = await systemctlUser(["restart", adapter.browserUnit]);
        if (!rs.ok)
          return { outcome: "failed", detail: `${v.detail}; restart of ${adapter.browserUnit} failed: ${rs.detail}` };
        await sleep(3000);
        const again = await probeCdp(adapter.cdpPort);
        if (!again.ok)
          return { outcome: "failed", detail: `browser unit restarted but CDP still unreachable: ${again.detail}` };
        v = await validate();
        if (v.ok)
          return {
            outcome: "succeeded",
            detail: `browser wedged; unit ${adapter.browserUnit} restarted; ${v.detail}`,
          };
        return { outcome: "failed", detail: `browser still wedged after unit restart: ${v.detail}` };
      }
      return v.ok ? { outcome: "succeeded" as const, detail: v.detail } : { outcome: "failed" as const, detail: v.detail };
    },
    async rung3(adapter) {
      if (!adapter.cdpPort || !adapter.loginUrl)
        return { outcome: "skipped", detail: "adapter declares no CDP port or login URL" };
      if (!liveRepairs)
        return { outcome: "skipped", detail: "live repairs not enabled (QUOTA_WATCHDOG_LIVE_REPAIRS != 1)" };
      const originOk = adapter.origins.some((o) => adapter.loginUrl!.startsWith(o));
      if (!originOk)
        return { outcome: "failed", detail: "login URL is outside the adapter origin allowlist — refusing" };
      let targetId: string | null = null;
      try {
        const created = (await fetchJson(
          `http://127.0.0.1:${adapter.cdpPort}/json/new?${encodeURIComponent(adapter.loginUrl)}`,
          5000,
          { method: "PUT" },
        )) as { id?: string };
        targetId = created.id ?? null;
        await sleep(5000);
        const targets = (await fetchJson(`http://127.0.0.1:${adapter.cdpPort}/json/list`, 3000)) as {
          id?: string;
          url?: string;
          title?: string;
        }[];
        const tab =
          targets.find((t) => t.id === targetId) ?? targets.find((t) => (t.url ?? "").startsWith(adapter.loginUrl!));
        const where = `${tab?.url ?? ""} ${tab?.title ?? ""}`;
        const marker = adapter.challengeMarkers.find((m) => where.toLowerCase().includes(m.toLowerCase()));
        if (marker) {
          return {
            outcome: "failed",
            detail: `human-only challenge at login (${marker}) — stopping; a human must complete it`,
          };
        }
        if (adapter.cookieNames.length > 0) {
          const names = await cdpCookieNames(adapter.cdpPort, adapter.origins);
          if (names !== null) {
            const missing = adapter.cookieNames.filter((n) => !names.includes(n));
            if (missing.length === 0)
              return { outcome: "succeeded", detail: "re-login restored the adapter cookies (present by name)" };
            return {
              outcome: "failed",
              detail: `re-login did not restore cookies: ${missing.join(", ")} still missing`,
            };
          }
        }
        return {
          outcome: "failed",
          detail: `login flow landed on ${tab?.url ?? "unknown page"} without confirming a session`,
        };
      } catch (e) {
        return { outcome: "failed", detail: `re-login driver error: ${e instanceof Error ? e.message : String(e)}` };
      } finally {
        if (targetId) {
          try {
            await fetchJson(`http://127.0.0.1:${adapter.cdpPort}/json/close/${targetId}`, 2000);
          } catch {
            /* tab already gone */
          }
        }
      }
    },
    escalation: productionEscalationEffectors(liveEscalation),
    log: (line) => console.log(line),
  };
}

/** Rung-4 production effectors (#6033): fleet queue + gh, behind the
 *  QUOTA_WATCHDOG_LIVE gate. Dry-run mode plans are journalled by the
 *  caller and these effectors are never invoked. */
function productionEscalationEffectors(live: boolean): EscalationEffectors {
  const run = async (cmd: string[], timeoutMs = 60000): Promise<string> => {
    const proc = Bun.spawn(cmd, { stdout: "pipe", stderr: "pipe" });
    const timer = setTimeout(() => proc.kill(), timeoutMs);
    const code = await proc.exited;
    clearTimeout(timer);
    const out = await new Response(proc.stdout).text();
    if (code !== 0) {
      const err = (await new Response(proc.stderr).text()).trim().slice(0, 300);
      throw new Error(`${cmd[0]} ${cmd[1] ?? ""} exit ${code}: ${err}`);
    }
    return out;
  };
  const guard = () => {
    if (!live) throw new Error("escalation effectors invoked without QUOTA_WATCHDOG_LIVE=1");
  };
  return {
    async enqueueTask(packet) {
      guard();
      // The validated repo-task/v1 envelope (task 20118): the packet
      // names the incident issue created earlier in this hand-off —
      // buildQueueTaskWrapper throws when it does not, rather than
      // enqueueing a packet the queue would reject.
      const wrapper = buildQueueTaskWrapper(packet, new Date().toISOString());
      const file = join("/tmp", `watchdog-escalation-${packet.source}-${Date.now()}.json`);
      await writeFile(file, JSON.stringify(wrapper));
      const out = await run(["fleet", "queue", "enqueue", "--queue", "repo-health-code-pilot", file]);
      const m = out.match(/"id"\s*:\s*(\d+)/);
      if (!m) throw new Error(`enqueue output carried no task id: ${out.slice(0, 200)}`);
      return m[1];
    },
    async openIssue(action) {
      guard();
      await run(["gh", "label", "create", "watchdog-incident", "--repo", action.repo, "--force"]).catch(() => "");
      const file = join("/tmp", `watchdog-issue-${Date.now()}.md`);
      await writeFile(file, action.body);
      const out = await run([
        "gh",
        "issue",
        "create",
        "--repo",
        action.repo,
        "--title",
        action.title,
        "--body-file",
        file,
        "--label",
        "watchdog-incident",
      ]);
      const m = out.match(/\/issues\/(\d+)/);
      if (!m) throw new Error(`issue create output carried no number: ${out.slice(0, 200)}`);
      return Number(m[1]);
    },
    async commentIssue(action) {
      guard();
      const file = join("/tmp", `watchdog-comment-${Date.now()}.md`);
      await writeFile(file, action.body);
      await run(["gh", "issue", "comment", String(action.issueNumber), "--repo", action.repo, "--body-file", file]);
    },
  };
}

// ---------------------------------------------------------------------------
// Observation + classification — the watchdog owns the classes the
// producer cannot see: missing document, stale read, browser state.
// ---------------------------------------------------------------------------

interface ProducerDoc {
  status?: string;
  measuredAt?: string;
  detail?: string;
  error?: string;
  failureClass?: string;
}

const STATUS_CLASS: Record<string, FailureClass> = {
  "cdp-unavailable": "cdp-unreachable",
  "cdp-unreachable": "cdp-unreachable",
  "browser-down": "browser-down",
  "session-expired": "session-expired",
  "login-required": "session-expired",
  "cookie-missing": "cookie-missing",
  "endpoint-changed": "endpoint-changed",
  "rate-limited": "rate-limited",
  rate_limited: "rate-limited",
};

function classifyText(text: string): FailureClass | null {
  const t = text.toLowerCase();
  if (t.includes("rate-limit") || t.includes("rate limit") || t.includes("429")) return "rate-limited";
  if (t.includes("cdp-unavailable") || t.includes("cdp unavailable") || t.includes("cdp-unreachable"))
    return "cdp-unreachable";
  if (t.includes("session") && (t.includes("expired") || t.includes("login"))) return "session-expired";
  if (t.includes("cookie") && t.includes("missing")) return "cookie-missing";
  if (t.includes("login page") || t.includes("sign-in") || t.includes("sign in")) return "session-expired";
  return null;
}

export interface ObservationInputs {
  doc: ProducerDoc | null;
  docError: string | null;
  heartbeatEntry: { state?: string; error?: string } | null;
  heartbeatStale: boolean;
  cdp: { ok: boolean; detail: string } | null;
  nowMs: number;
}

export function buildObservation(adapter: WatchdogAdapter, inp: ObservationInputs): Observation {
  const at = new Date(inp.nowMs).toISOString();
  const { doc } = inp;

  if (doc && typeof doc.status === "string") {
    if (doc.status === "ok") {
      const measured = doc.measuredAt ? Date.parse(doc.measuredAt) : NaN;
      if (Number.isFinite(measured) && inp.nowMs - measured > adapter.staleAfterMs) {
        // Stale read: the document claims health but is old — the
        // producer has gone silent while everything looks alive.
        // A dead CDP endpoint at the same moment names the culprit.
        if (inp.cdp && !inp.cdp.ok) {
          return {
            kind: "failed",
            at,
            class: "cdp-unreachable",
            detail: `stale read (${Math.round((inp.nowMs - measured) / 60000)} min old) and ${inp.cdp.detail}`,
          };
        }
        return {
          kind: "failed",
          at,
          class: "unknown",
          detail: `stale read: producer document is ${Math.round((inp.nowMs - measured) / 60000)} min old (limit ${Math.round(adapter.staleAfterMs / 60000)} min)`,
        };
      }
      return { kind: "healthy", at, detail: "producer ok" };
    }
    // Producer pre-classified its own failure.
    const detail = sanitizeDetail(doc.detail ?? doc.error ?? doc.status);
    const cls =
      (doc.failureClass && (FAILURE_CLASS_SET.has(doc.failureClass) ? (doc.failureClass as FailureClass) : null)) ||
      adapter.classMap[doc.status] ||
      STATUS_CLASS[doc.status] ||
      classifyText(detail) ||
      "unknown";
    return { kind: "failed", at, class: cls, detail: detail || doc.status };
  }

  if (inp.docError) {
    // Missing/unreadable document — a class the producer cannot see.
    if (inp.cdp && !inp.cdp.ok) {
      return {
        kind: "failed",
        at,
        class: "browser-down",
        detail: `producer document missing (${inp.docError}); ${inp.cdp.detail}`,
      };
    }
    return { kind: "failed", at, class: "unknown", detail: `producer document missing or unreadable: ${inp.docError}` };
  }

  // No document configured for this source: the gatherer heartbeat is
  // the observation.
  if (adapter.heartbeatKey) {
    if (inp.heartbeatStale) {
      return { kind: "failed", at, class: "unknown", detail: "stale read: gatherer heartbeat is stale" };
    }
    const entry = inp.heartbeatEntry;
    if (!entry)
      return { kind: "failed", at, class: "unknown", detail: `no heartbeat entry for ${adapter.heartbeatKey}` };
    if (entry.state === "ok") return { kind: "healthy", at, detail: "gatherer heartbeat ok" };
    const detail = sanitizeDetail(entry.error ?? `heartbeat state ${entry.state ?? "?"}`);
    return { kind: "failed", at, class: classifyText(detail) ?? "unknown", detail };
  }

  return { kind: "failed", at, class: "unknown", detail: "no observation source produced data" };
}

const FAILURE_CLASS_SET = new Set<string>([
  "session-expired",
  "cookie-missing",
  "endpoint-changed",
  "browser-down",
  "cdp-unreachable",
  "rate-limited",
  "unknown",
]);

// ---------------------------------------------------------------------------
// One tick over all sources.
// ---------------------------------------------------------------------------

export interface TickOptions {
  adaptersDir: string;
  stateFile: string;
  historyFile: string;
  heartbeatFile?: string;
  effects: WatchdogEffects;
  liveEscalation: boolean;
  now?: Date;
}

export interface HistoryEntry {
  at: string;
  source: string;
  observation: string;
  class: FailureClass | null;
  decision: string;
  surfacing: string | null;
}

export async function runTick(
  opts: TickOptions,
): Promise<{ state: WatchdogState; history: HistoryEntry[]; errors: string[] }> {
  const now = opts.now ?? new Date();
  const at = now.toISOString();
  const { adapters, errors } = await loadAdapters(opts.adaptersDir);

  let state: WatchdogState = emptyState(at);
  try {
    const raw = JSON.parse(await readFile(opts.stateFile, "utf8")) as Partial<WatchdogState>;
    if (raw && typeof raw === "object" && typeof raw.sources === "object" && raw.sources !== null) {
      state = {
        version: 1,
        updatedAt: typeof raw.updatedAt === "string" ? raw.updatedAt : at,
        sources: Object.fromEntries(Object.entries(raw.sources).map(([k, v]) => [k, coerceHealth(k, v)])),
      };
    }
  } catch {
    // Missing or corrupt state file: start fresh (fail open to
    // re-observe, never to assume healthy).
    state = emptyState(at);
  }

  let heartbeat: { tickAt?: string; perProvider?: Record<string, { state?: string; error?: string }> } | null = null;
  if (opts.heartbeatFile) {
    try {
      heartbeat = JSON.parse(await readFile(opts.heartbeatFile, "utf8"));
    } catch {
      heartbeat = null;
    }
  }
  const heartbeatStale =
    heartbeat?.tickAt !== undefined ? now.getTime() - Date.parse(heartbeat.tickAt) > 30 * 60_000 : false;

  const history: HistoryEntry[] = [];

  for (const adapter of adapters) {
    const prev = state.sources[adapter.name] ?? initialHealth(adapter.name);

    // --- observe ---
    let doc: ProducerDoc | null = null;
    let docError: string | null = null;
    if (adapter.latestPath) {
      try {
        doc = JSON.parse(await readFile(adapter.latestPath, "utf8")) as ProducerDoc;
      } catch (e) {
        docError = e instanceof Error ? e.message.split("\n")[0] : String(e);
      }
    }
    const cdp = adapter.cdpPort ? await opts.effects.probeCdp(adapter.cdpPort) : null;
    const obs = buildObservation(adapter, {
      doc,
      docError,
      heartbeatEntry: adapter.heartbeatKey ? (heartbeat?.perProvider?.[adapter.heartbeatKey] ?? null) : null,
      heartbeatStale,
      cdp,
      nowMs: now.getTime(),
    });

    // --- recovery effects for an escalated incident that just healed ---
    if (obs.kind === "healthy" && prev.incident?.escalation) {
      const recovery = planRecovery(prev, at);
      if (recovery.length > 0) {
        if (opts.liveEscalation) {
          await applyEscalationActions(recovery, opts.effects.escalation).catch((e) => {
            opts.effects.log(
              `watchdog: recovery effects for ${adapter.name} failed: ${e instanceof Error ? e.message : String(e)}`,
            );
          });
        } else {
          opts.effects.log(
            `watchdog: dry-run recovery plan for ${adapter.name}: ${recovery.map((a) => a.type).join(", ")}`,
          );
        }
      }
    }

    // --- decide + execute the chosen rung ---
    const { decision, next } = decide(prev, obs, { classMap: adapter.rungMap, cooldownMs: adapter.cooldownMs });
    let health = next;

    if (decision.action === "repair") {
      const rung = decision.rung;
      const res =
        rung === 2
          ? await opts.effects.rung2(adapter)
          : rung === 3
            ? await opts.effects.rung3(adapter)
            : { outcome: "skipped" as const, detail: `rung ${rung} has no executor` };
      health = recordRungOutcome(health, at, rung, res.outcome, res.detail);
    }

    // --- rung 4: escalation hand-off ---
    if (decision.action === "escalate" || health.incident?.surfacing === "escalate") {
      const ctx: EscalationContext = {
        issueRepo: adapter.issueRepo,
        stateFilePath: opts.stateFile,
        adapterRef: `adapters/${adapter.name}.json`,
        historyRef: opts.historyFile,
      };
      const plan = planEscalation(health, ctx, now.getTime());
      if (plan.actions.length > 0) {
        if (opts.liveEscalation) {
          try {
            const refs = await applyEscalationActions(plan.actions, opts.effects.escalation);
            health = recordEscalation(health, ctx, refs, at);
            if (plan.reEscalated) health = recordReEscalation(health, at);
          } catch (e) {
            if (e instanceof EscalationPartialError) {
              health = recordEscalation(health, ctx, e.refs, at);
            }
            opts.effects.log(
              `watchdog: escalation for ${adapter.name} failed: ${e instanceof Error ? e.message : String(e)}`,
            );
          }
        } else {
          opts.effects.log(
            `watchdog: dry-run escalation plan for ${adapter.name}: ${plan.actions.map((a) => a.type).join(", ")} (set QUOTA_WATCHDOG_LIVE=1 to execute)`,
          );
        }
      }
    }

    state.sources[adapter.name] = health;
    const line = surfacingLine(health, now.getTime());
    if (line) opts.effects.log(line);
    history.push({
      at,
      source: adapter.name,
      observation: obs.kind,
      class: obs.kind === "failed" ? obs.class : null,
      decision: decision.action,
      surfacing: health.incident?.surfacing ?? null,
    });
  }

  // --- persist: atomic state write + history append ---
  state.updatedAt = at;
  await mkdir(dirname(opts.stateFile), { recursive: true });
  const tmp = `${opts.stateFile}.tmp-${process.pid}`;
  await writeFile(tmp, `${JSON.stringify(state, null, 2)}\n`);
  await rename(tmp, opts.stateFile);
  if (history.length > 0) {
    await mkdir(dirname(opts.historyFile), { recursive: true });
    await appendFile(opts.historyFile, history.map((h) => JSON.stringify(h)).join("\n") + "\n");
  }
  return { state, history, errors };
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

function argValue(args: string[], flag: string): string | undefined {
  const i = args.indexOf(flag);
  return i >= 0 ? args[i + 1] : undefined;
}

export async function main(
  args: string[] = process.argv.slice(2),
  env: Record<string, string | undefined> = process.env,
): Promise<number> {
  const home = homedir();
  const stateDir = join(home, ".local", "state", "pm-aggregator");
  const adaptersDir =
    argValue(args, "--adapters-dir") ??
    env.QUOTA_WATCHDOG_ADAPTERS_DIR ??
    join(home, ".config", "quota-watchdog", "adapters");
  const stateFile = argValue(args, "--state-file") ?? join(stateDir, "quota-watchdog-state.json");
  const historyFile = argValue(args, "--history-file") ?? join(stateDir, "quota-watchdog-history.jsonl");
  const heartbeatFile = argValue(args, "--heartbeat-file") ?? join(stateDir, "quota-gatherer-heartbeat.json");
  const effects = productionEffects(env);
  const { errors } = await runTick({
    adaptersDir,
    stateFile,
    historyFile,
    heartbeatFile,
    effects,
    liveEscalation: env.QUOTA_WATCHDOG_LIVE === "1",
  });
  for (const e of errors) effects.log(`watchdog: adapter error: ${e}`);
  return errors.length > 0 ? 1 : 0;
}

if (import.meta.main) {
  process.exit(await main());
}
