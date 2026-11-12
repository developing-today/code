#!/usr/bin/env node
/**
 * meta-quota-export — Meta (dev.meta.ai) quota exporter, Phase 1 of
 * developing-today/code#1301 (epic #1149; investigation #1167).
 *
 * Runs ON Drew's laptop, inside the reach of the laptop Helium
 * browser's live dev.meta.ai session (CDP debug port, default 19222).
 * It evaluates fetch() calls in the console page's own context, so the
 * requests carry the session cookie and leave from the laptop's own
 * egress/device — no cookie value is ever read into this process,
 * written to disk, or logged.
 *
 * Every run writes the export document atomically to
 *   <out-dir>/latest.json     (default ~/atlas/meta-quota-export/)
 * and appends the same document to <out-dir>/history.jsonl.
 *
 * Success document (the file-export format the #1151 quota gatherer
 * ingests; source label fixed as "file-export/meta-laptop"):
 *   { service: "meta", source: "file-export/meta-laptop",
 *     status: "ok", measuredAt: <ISO>, consoleAsOf: <ISO|null>,
 *     tier: "Muse Code High Usage",
 *     windows: [ { name: "5h", usedPct: 1.02, resetAt: <ISO> },
 *                { name: "weekly", usedPct: 8.83, resetAt: <ISO> } ],
 *     freeCreditsUsd: 148.57, paygBalanceUsd: 0 }
 *
 * Failure documents carry status + attemptedAt + detail and NO
 * windows/values — a failed tick can never be mistaken for a reading:
 *   session-expired  the console API answered 401/403 or bounced to
 *                    an auth page; the laptop session needs a human
 *                    re-login. Exit 3.
 *   cdp-unavailable  the browser debug port did not answer. Exit 4.
 *   no-console-page  browser is up, no dev.meta.ai tab is open, and
 *                    creating one via the CDP targets API failed.
 *                    Exit 5. (Since task 9492 / issue #5121 the exporter
 *                    owns its tab: it creates a dev.meta.ai target when
 *                    none exists and closes only the target it created.)
 *   page-not-ready   the browser (helium-cdp) answers CDP but its
 *                    renderer is wedged: the readiness probe's target
 *                    never navigates/evaluates, or a created tab never
 *                    finishes loading. Names the browser, not the tab.
 *                    Exit 8. Before failing, the exporter attempts ONE
 *                    bounded helium-cdp.service restart (max once per
 *                    15 min, recorded in the export document as
 *                    `recovery`) and re-probes. (issue #5487)
 *   unparseable      an endpoint answered something whose shape this
 *                    parser does not recognise. Exit 6.
 *   fetch-failed     network/other error inside the page. Exit 7.
 * "laptop-unreachable" is deliberately NOT produced here: if the
 * laptop is down nothing runs, so consumers derive it from the age of
 * latest.json (> 2 ticks = > 30 min without a fresh document).
 */
import { appendFile, mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { execFile } from "node:child_process";
import { homedir } from "node:os";
import { join } from "node:path";
import { realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";

export const SOURCE = "file-export/meta-laptop";
export const DEFAULT_TEAM_ID = "1073226932208501"; // "My Team" (#1167)

export class ExportError extends Error {
  constructor(status, exitCode, detail, recovery = null) {
    super(detail);
    this.status = status;
    this.exitCode = exitCode;
    this.recovery = recovery;
  }
}

const round2 = (n) => Math.round(n * 100) / 100;
const MIN_EPOCH_SECONDS = 1_577_836_800; // 2020-01-01T00:00:00Z
const MAX_EPOCH_SECONDS = 4_102_444_800; // 2100-01-01T00:00:00Z

/** Parse a subscription-quota payload into export windows. Throws on
 * any shape drift — fail-closed, never partial zeros. The one tolerated
 * absence is a not-yet-started window (used = 0) whose reset the
 * console returns as null or omits: that window is emitted with
 * resetAt: null instead of failing the whole export (#1301). */
export function parseSubscriptionQuota(payload) {
  const q = payload?.subscription_quota;
  if (!q || typeof q !== "object") {
    throw new ExportError("unparseable", 6, "subscription-quota: subscription_quota object missing");
  }
  // Plain decimal spellings only (#2789). Number() also accepts "",
  // whitespace-only, hex ("0x10" -> 16), and scientific ("1e2" -> 100)
  // strings, which turned drifted fields into real measurements (a ""
  // used value exported as a measured 0% window). Reject every
  // non-decimal spelling here so drift fails closed at the parse;
  // surrounding whitespace is drift too, not a spelling.
  const DECIMAL = /^[0-9]+(\.[0-9]+)?$/;
  const num = (v) => {
    if (typeof v === "number") return v;
    if (typeof v === "string" && DECIMAL.test(v)) return Number(v);
    return NaN;
  };
  const windowUsed = num(q.window_weighted_used);
  const windowLimit = num(q.window_weighted_limit);
  const weeklyUsed = num(q.weekly_weighted_used);
  const weeklyLimit = num(q.weekly_weighted_limit);
  const durationSecs = num(q.window_duration_secs);
  const fields = { windowUsed, windowLimit, weeklyUsed, weeklyLimit };
  for (const [k, v] of Object.entries(fields)) {
    if (!Number.isFinite(v) || v < 0) {
      throw new ExportError("unparseable", 6, `subscription-quota: field ${k} not a finite number`);
    }
  }
  const epochSecondsField = (v, label) => {
    if (!Number.isFinite(v) || v < 0) {
      throw new ExportError("unparseable", 6, `subscription-quota: field ${label} not a finite number`);
    }
    if (v < MIN_EPOCH_SECONDS || v > MAX_EPOCH_SECONDS) {
      throw new ExportError(
        "unparseable",
        6,
        `subscription-quota: field ${label} outside the supported epoch-seconds range`,
      );
    }
    return v;
  };
  // A window the console has not started yet (no usage recorded in it)
  // has no reset: window_resets_at / weekly_resets_at comes back null
  // or the field is omitted entirely (observed live 2026-10-29, #1301:
  // after the 16:15Z short-window roll the absent reset poisoned every
  // export, including the healthy weekly window and free credits).
  // Tolerate exactly that shape — emit the window with resetAt: null.
  // A missing reset on a window WITH recorded usage, and any malformed
  // reset value, still fail closed. An empty/whitespace-only reset
  // string is treated like an absent reset: it carries no reading, so
  // on a zero-usage window it is tolerated, and on a window with usage
  // it fails closed exactly like the absent case.
  const resetField = (raw, used, label) => {
    if (raw === null || raw === undefined || (typeof raw === "string" && raw.trim() === "")) {
      if (used === 0) return null;
      throw new ExportError(
        "unparseable",
        6,
        `subscription-quota: field ${label} absent but the window has recorded usage`,
      );
    }
    return epochSecondsField(num(raw), label);
  };
  const windowReset = resetField(q.window_resets_at, windowUsed, "windowReset");
  const weeklyReset = resetField(q.weekly_resets_at, weeklyUsed, "weeklyReset");
  if (windowLimit === 0 || weeklyLimit === 0) {
    throw new ExportError("unparseable", 6, "subscription-quota: zero limit");
  }
  if (!Number.isFinite(durationSecs) || durationSecs <= 0) {
    throw new ExportError("unparseable", 6, "subscription-quota: field window_duration_secs not a finite positive number");
  }
  const shortName = durationSecs === 18_000 ? "5h" : durationSecs === 3_600 ? "1h" : `window-${durationSecs}s`;
  const iso = (epochSecs) => new Date(epochSecs * 1000).toISOString();
  // as_of is console metadata, not a measurement: an absent or
  // empty value means "the console didn't say when" and stays null,
  // while any other non-decimal spelling fails closed — never the
  // 1970 epoch that Number("") used to produce.
  const asOfRaw = q.as_of;
  let consoleAsOf = null;
  if (!(asOfRaw === null || asOfRaw === undefined || (typeof asOfRaw === "string" && asOfRaw.trim() === ""))) {
    consoleAsOf = iso(epochSecondsField(num(asOfRaw), "as_of"));
  }
  return {
    tier: typeof q.tier === "string" ? q.tier : null,
    consoleAsOf,
    windows: [
      { name: shortName, usedPct: round2((windowUsed / windowLimit) * 100), resetAt: windowReset === null ? null : iso(windowReset) },
      { name: "weekly", usedPct: round2((weeklyUsed / weeklyLimit) * 100), resetAt: weeklyReset === null ? null : iso(weeklyReset) },
    ],
  };
}

/** Parse a billing-banner payload into remaining free credits (USD). */
export function parseBillingBanner(payload) {
  const minor = payload?.remaining_minor_units;
  // Minor units are a count of cents: only a nonnegative safe integer is a
  // well-shaped value. A fractional value (1050.5) is shape drift, not
  // a fractional dollar amount of credit — fail closed (#2510). No domain-specific maximum is
  // imposed: there is no authoritative upstream bound, and
  // Number.MAX_SAFE_INTEGER (via isSafeInteger) is a representation-safety
  // limit, not a business cap.
  if (typeof minor !== "number" || !Number.isSafeInteger(minor) || minor < 0) {
    throw new ExportError("unparseable", 6, "billing-banner: remaining_minor_units missing/invalid");
  }
  if (payload.currency !== "USD") {
    throw new ExportError("unparseable", 6, `billing-banner: unexpected currency ${String(payload.currency)}`);
  }
  return minor / 100;
}

/** Parse the PAYG current balance out of the billing iframe's text.
 * Returns null (not 0) when the text does not contain the balance —
 * an absent read must stay absent, never become a fake $0. */
export function parsePaygBalance(iframeText) {
  if (typeof iframeText !== "string") return null;
  const m = iframeText.match(/Current balance\s*\n?\s*\$\s*([\d,]+(?:\.\d{2})?)/);
  if (!m) return null;
  const v = Number(m[1].replace(/,/g, ""));
  return Number.isFinite(v) ? v : null;
}

export function successDocument({ measuredAt, parsed, freeCreditsUsd, paygBalanceUsd, recovery = null }) {
  const doc = {
    service: "meta",
    source: SOURCE,
    status: "ok",
    measuredAt,
    consoleAsOf: parsed.consoleAsOf,
    tier: parsed.tier,
    windows: parsed.windows,
    freeCreditsUsd,
    paygBalanceUsd,
  };
  if (recovery) doc.recovery = recovery;
  return doc;
}

export function failureDocument({ attemptedAt, status, detail, recovery = null }) {
  const doc = {
    service: "meta",
    source: SOURCE,
    status,
    measuredAt: null,
    attemptedAt,
    detail,
  };
  if (recovery) doc.recovery = recovery;
  return doc;
}

// ---------- CDP plumbing (only used by the main run) ----------

async function httpJson(url) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`HTTP ${res.status} for ${url}`);
  return res.json();
}

function connectWs(wsUrl) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(wsUrl);
    const pending = new Map();
    let id = 0;
    ws.onmessage = (ev) => {
      const m = JSON.parse(ev.data);
      if (m.id && pending.has(m.id)) {
        const p = pending.get(m.id);
        pending.delete(m.id);
        if (m.error) p.reject(new Error(JSON.stringify(m.error)));
        else p.resolve(m.result);
      }
    };
    ws.onopen = () =>
      resolve({
        send: (method, params = {}) =>
          new Promise((res, rej) => {
            const i = ++id;
            pending.set(i, { resolve: res, reject: rej });
            ws.send(JSON.stringify({ id: i, method, params }));
          }),
        close: () => ws.close(),
      });
    ws.onerror = (e) => reject(new Error(`websocket error: ${e.message ?? "unknown"}`));
  });
}

async function evaluate(client, expression) {
  const r = await client.send("Runtime.evaluate", {
    expression,
    returnByValue: true,
    awaitPromise: true,
  });
  if (r.exceptionDetails) {
    throw new Error(`page eval exception: ${JSON.stringify(r.exceptionDetails).slice(0, 300)}`);
  }
  return r.result.value;
}

/** Fetch both quota endpoints inside the console page's context.
 * Returns { quota, banner } parsed JSON bodies; throws ExportError
 * with the right failure state otherwise. */
async function fetchQuotaPayloads(client, teamId) {
  const expression = `(async () => {
    const team = ${JSON.stringify(teamId)};
    const get = async (path) => {
      const r = await fetch(path, { headers: { Accept: "application/json" }, credentials: "same-origin" });
      return { status: r.status, url: r.url, text: await r.text() };
    };
    const quota = await get("/api/portal/teams/" + team + "/subscription-quota");
    const banner = await get("/api/portal/teams/" + team + "/billing-banner");
    return JSON.stringify({ quota, banner });
  })()`;
  let raw;
  try {
    raw = await evaluate(client, expression);
  } catch (e) {
    throw new ExportError("fetch-failed", 7, `in-page fetch failed: ${e.message}`);
  }
  let outer;
  try {
    outer = JSON.parse(raw);
  } catch {
    throw new ExportError("fetch-failed", 7, "in-page fetch returned a non-JSON envelope");
  }
  const bodies = {};
  for (const key of ["quota", "banner"]) {
    const r = outer[key];
    if (!r || typeof r.status !== "number") {
      throw new ExportError("fetch-failed", 7, `${key}: no response captured`);
    }
    if (r.status === 401 || r.status === 403 || (typeof r.url === "string" && r.url.includes("auth"))) {
      throw new ExportError(
        "session-expired",
        3,
        `${key}: HTTP ${r.status}${r.url ? ` (landed on ${new URL(r.url).pathname})` : ""} — laptop dev.meta.ai session needs re-login`,
      );
    }
    if (r.status !== 200) {
      throw new ExportError("fetch-failed", 7, `${key}: HTTP ${r.status}`);
    }
    try {
      bodies[key] = JSON.parse(r.text);
    } catch {
      throw new ExportError("unparseable", 6, `${key}: HTTP 200 but body is not JSON`);
    }
  }
  return bodies;
}

/** Best-effort PAYG balance from the billing iframe's DOM text.
 * Absence is reported as null, never invented. */
async function fetchPaygBalance(cdpBase) {
  try {
    const targets = await httpJson(`${cdpBase}/json/list`);
    const frame = targets.find(
      (t) => t.type === "iframe" && typeof t.url === "string" && t.url.includes("billing.dev.meta.ai"),
    );
    if (!frame?.webSocketDebuggerUrl) return null;
    const client = await connectWs(frame.webSocketDebuggerUrl);
    try {
      // Bound the read (Codex review on PR #6333): a frame that accepts
      // CDP but never answers Runtime.evaluate must not hang the whole
      // export — withTimeout resolves null, which parses to a null
      // (absent) balance, never an invented one.
      const text = await withTimeout(
        evaluate(client, "document.body ? document.body.innerText : ''"),
        10000,
      );
      return parsePaygBalance(text);
    } finally {
      client.close();
    }
  } catch {
    return null;
  }
}

async function writeOutputs(outDir, doc) {
  await mkdir(outDir, { recursive: true });
  const latest = join(outDir, "latest.json");
  const tmp = join(outDir, `.latest.json.tmp-${process.pid}`);
  await writeFile(tmp, `${JSON.stringify(doc, null, 2)}\n`, "utf8");
  await rename(tmp, latest);
  await appendFile(join(outDir, "history.jsonl"), `${JSON.stringify(doc)}\n`, "utf8");
}


async function createTarget(cdpBase, url) {
  const res = await fetch(`${cdpBase}/json/new?${encodeURIComponent(url)}`, { method: "PUT" });
  if (!res.ok) throw new Error(`HTTP ${res.status} creating target`);
  return res.json();
}

async function closeTarget(cdpBase, id) {
  try {
    await fetch(`${cdpBase}/json/close/${id}`);
  } catch {
    // best effort: a target that is already gone needs no closing
  }
}

/** Race a promise against a timeout; resolves null on timeout.
 * CDP calls into a busy renderer can hang indefinitely, and the
 * systemd unit kills the whole run at 2 minutes, so no single CDP
 * call in the created-tab path may block unbounded. */
function withTimeout(promise, ms) {
  return Promise.race([
    promise,
    new Promise((resolve) => setTimeout(() => resolve(null), ms)),
  ]);
}


// ---------- Readiness probe + bounded recovery (issue #5487) ----------

export const HELIUM_UNIT = "helium-cdp.service";
export const RESTART_MIN_INTERVAL_MS = 15 * 60 * 1000;
export const PAGE_NOT_READY_EXIT = 8;

/** Readiness probe: create a throwaway target, navigate it to a data:
 * URL, and evaluate `1` on it — each step timeout-raced. A healthy
 * renderer answers in ~1 s; a wedged one (CDP answers /json/new but
 * Page.navigate / Runtime.evaluate never reply) fails the probe.
 * Returns { ok: true } or { ok: false, detail }. Never throws. */
export async function probeCdpReadiness({ cdpBase, timeoutMs = 10000, deps = {} } = {}) {
  const create = deps.createTarget ?? createTarget;
  const connect = deps.connectWs ?? connectWs;
  const close = deps.closeTarget ?? closeTarget;
  const stepMs = Math.max(1000, Math.floor(timeoutMs / 2));
  let target = null;
  let client = null;
  try {
    target = await withTimeout(create(cdpBase, "about:blank"), stepMs);
    if (!target?.id || !target?.webSocketDebuggerUrl) {
      return { ok: false, detail: "probe: creating a target returned no usable target" };
    }
    client = await withTimeout(connect(target.webSocketDebuggerUrl), stepMs);
    if (!client) return { ok: false, detail: "probe: websocket to the probe target never opened" };
    const nav = await withTimeout(
      client.send("Page.navigate", { url: "data:text/html,<title>probe</title>ok" }),
      stepMs,
    );
    if (nav === null) {
      return { ok: false, detail: `probe: Page.navigate never replied within ${stepMs}ms — renderer wedged` };
    }
    const val = await withTimeout(evaluate(client, "1"), stepMs);
    if (val !== 1) {
      return { ok: false, detail: `probe: Runtime.evaluate never replied within ${stepMs}ms — renderer wedged` };
    }
    return { ok: true };
  } catch (e) {
    return { ok: false, detail: `probe: ${e.message}` };
  } finally {
    try { client?.close(); } catch { /* best effort */ }
    if (target?.id) await close(cdpBase, target.id);
  }
}

/** Pure cooldown check for the bounded restart. */
export function restartAllowed(lastRestartAtIso, nowMs) {
  if (!lastRestartAtIso) return true;
  const last = Date.parse(lastRestartAtIso);
  if (!Number.isFinite(last)) return true;
  return nowMs - last >= RESTART_MIN_INTERVAL_MS;
}

function execFileAsync(file, args) {
  return new Promise((resolve, reject) => {
    execFile(file, args, { timeout: 30000 }, (err) => (err ? reject(err) : resolve()));
  });
}

/** Attempt ONE bounded helium-cdp restart, at most once per
 * RESTART_MIN_INTERVAL_MS, with the attempt persisted next to the
 * export outputs so the bound holds across ticks. Returns a recovery
 * record for the export document either way. */
export async function attemptBoundedRestart({ outDir, now = new Date(), restart } = {}) {
  const at = now.toISOString();
  const statePath = outDir ? join(outDir, ".helium-restart.json") : null;
  let last = null;
  if (statePath) {
    try { last = JSON.parse(await readFile(statePath, "utf8")).lastRestartAt ?? null; } catch { last = null; }
  }
  if (!restartAllowed(last, now.getTime())) {
    return { action: "restart", unit: HELIUM_UNIT, at, result: "skipped-cooldown", lastRestartAt: last };
  }
  const doRestart = restart ?? (() => execFileAsync("systemctl", ["--user", "restart", HELIUM_UNIT]));
  // Persist the attempt BEFORE calling restart (Codex review on
  // PR #6333): a restart that rejects or times out must still obey the
  // once-per-interval bound — callers such as the Meta session keeper
  // can re-invoke the exporter independently of its timer.
  if (statePath) {
    try {
      await mkdir(outDir, { recursive: true });
      await writeFile(statePath, `${JSON.stringify({ lastRestartAt: at })}\n`, "utf8");
    } catch { /* the record in the export document still stands */ }
  }
  try {
    await doRestart();
  } catch (e) {
    return { action: "restart", unit: HELIUM_UNIT, at, result: "restart-failed", detail: e.message };
  }
  return { action: "restart", unit: HELIUM_UNIT, at, result: "restarted" };
}

/** Drive the created page to dev.meta.ai and wait until its document
 * is usable on that origin; returns the final href. Throws
 * no-console-page when the tab does not finish loading in time.
 * Notes from live debugging (issue #5121, 2026-11-04): a target made
 * via PUT /json/new lists the requested URL but its document stays
 * about:blank until an explicit Page.navigate; and under laptop load
 * the renderer can take tens of seconds to answer Runtime.evaluate,
 * so every poll is timeout-raced and treated as still-loading. */
async function waitForPageReady(client, timeoutMs = 75000) {
  await withTimeout(client.send("Page.enable"), 5000);
  await withTimeout(
    client.send("Page.navigate", { url: "https://dev.meta.ai/" }),
    5000,
  );
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const state = await withTimeout(
      evaluate(client, "document.readyState + '|' + location.href"),
      4000,
    );
    if (typeof state === "string") {
      const [ready, href] = state.split("|");
      if (
        (ready === "complete" || ready === "interactive") &&
        href &&
        href.startsWith("https://dev.meta.ai")
      ) {
        return href;
      }
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new ExportError(
    "page-not-ready",
    PAGE_NOT_READY_EXIT,
    `helium-cdp page not ready: created dev.meta.ai tab did not finish loading within ${Math.round(timeoutMs / 1000)}s — the browser renderer, not the tab, is the culprit (issue #5487)`,
  );
}

export async function run({ cdpPort = 19222, outDir, teamId = DEFAULT_TEAM_ID, probeFn, restartFn } = {}) {
  const attemptedAt = new Date().toISOString();
  const cdpBase = `http://127.0.0.1:${cdpPort}`;
  let targets;
  try {
    targets = await httpJson(`${cdpBase}/json/list`);
  } catch (e) {
    throw new ExportError("cdp-unavailable", 4, `CDP ${cdpBase}/json/list unreachable: ${e.message}`);
  }
  // Readiness probe (issue #5487): CDP answering /json/list does NOT
  // mean a page can load — a wedged renderer answers every cheap
  // health signal while Page.navigate hangs forever. Probe first;
  // on a wedge, try ONE bounded helium-cdp restart and re-probe;
  // if still wedged, fail as page-not-ready naming the browser.
  const probe = probeFn ?? ((base) => probeCdpReadiness({ cdpBase: base }));
  let recovery = null;
  const firstProbe = await probe(cdpBase);
  if (!firstProbe.ok) {
    recovery = await attemptBoundedRestart({ outDir, restart: restartFn });
    if (recovery.result === "restarted") {
      const secondProbe = await probe(cdpBase);
      if (!secondProbe.ok) {
        throw new ExportError(
          "page-not-ready",
          PAGE_NOT_READY_EXIT,
          `helium-cdp wedged: ${secondProbe.detail}; still not ready after a bounded ${HELIUM_UNIT} restart`,
          recovery,
        );
      }
      // The pre-restart target list is stale (Codex review on
      // PR #6333): its dev.meta.ai entry carries the old debugger URL.
      // Re-fetch before selecting or creating the console page.
      try {
        targets = await httpJson(`${cdpBase}/json/list`);
      } catch (e) {
        throw new ExportError("cdp-unavailable", 4, `CDP ${cdpBase}/json/list unreachable after restart: ${e.message}`, recovery);
      }
    } else {
      throw new ExportError(
        "page-not-ready",
        PAGE_NOT_READY_EXIT,
        `helium-cdp wedged: ${firstProbe.detail}; bounded restart not performed (${recovery.result})`,
        recovery,
      );
    }
  }
  let page = targets.find(
    (t) => t.type === "page" && typeof t.url === "string" && t.url.startsWith("https://dev.meta.ai"),
  );
  // Own the tab (issue #5121): when no dev.meta.ai target exists and CDP
  // answers, create one, use it, and close only the target we created.
  // cdp-unavailable above stays a real failure (exit 4).
  let createdTargetId = null;
  try {
    if (!page?.webSocketDebuggerUrl) {
      let created;
      try {
        created = await createTarget(cdpBase, "https://dev.meta.ai/");
      } catch (e) {
        throw new ExportError(
          "no-console-page",
          5,
          `browser is up but no dev.meta.ai tab is open and creating one failed: ${e.message}`,
        );
      }
      if (!created?.id || !created?.webSocketDebuggerUrl) {
        throw new ExportError(
          "no-console-page",
          5,
          "browser is up but no dev.meta.ai tab is open and the created target has no debugger URL",
        );
      }
      createdTargetId = created.id;
      page = { ...created, url: created.url || "https://dev.meta.ai/" };
    }
    const client = await connectWs(page.webSocketDebuggerUrl);
    let bodies;
    try {
      let teamUrl = page.url;
      if (createdTargetId) {
        teamUrl = await waitForPageReady(client);
      }
      let teamParam = null;
      try {
        teamParam = new URL(teamUrl).searchParams.get("team_id");
      } catch {
        teamParam = null;
      }
      bodies = await fetchQuotaPayloads(client, teamParam ?? teamId);
    } finally {
      client.close();
    }
    const parsed = parseSubscriptionQuota(bodies.quota);
    const freeCreditsUsd = parseBillingBanner(bodies.banner);
    const paygBalanceUsd = await fetchPaygBalance(cdpBase);
    return successDocument({
      measuredAt: new Date().toISOString(),
      parsed,
      freeCreditsUsd,
      paygBalanceUsd,
      recovery,
    });
  } finally {
    if (createdTargetId) await closeTarget(cdpBase, createdTargetId);
  }
}

/** Strict single-pass argv parsing (#3469). Every value-taking flag must be
 * followed by a value token that is not itself a flag: a flag token (or
 * nothing) where a value belongs is a usage error naming the flag, raised
 * before any directory is created or any CDP call is made. The parse is
 * single-pass — each value token is consumed once — so a flag token can
 * never double as both a value and a flag. Unknown flags and bare
 * positionals are usage errors too: silently ignoring them is what used
 * to redirect output into a directory named after a flag. Throws
 * ExportError("usage", 2); main() turns it into stderr + exit 2. */
const VALUE_FLAGS = new Set(["--out-dir", "--cdp-port", "--team-id"]);

function usageError(message) {
  throw new ExportError("usage", 2, message);
}

function parseCdpPort(raw) {
  if (!/^\d+$/.test(raw)) {
    usageError(`--cdp-port must be a decimal integer in 1-65535 (got ${JSON.stringify(raw)})`);
  }
  const port = Number(raw);
  if (!Number.isSafeInteger(port) || port < 1 || port > 65535) {
    usageError(`--cdp-port must be a decimal integer in 1-65535 (got ${JSON.stringify(raw)})`);
  }
  return port;
}

export function parseArgs(argv) {
  const values = {};
  for (let i = 0; i < argv.length; i++) {
    const token = argv[i];
    if (!VALUE_FLAGS.has(token)) {
      if (token.startsWith("--")) usageError(`unknown flag ${JSON.stringify(token)}`);
      else usageError(`unexpected argument ${JSON.stringify(token)} (this exporter takes only --flag value pairs)`);
    }
    const value = argv[i + 1];
    if (typeof value !== "string" || value === "" || value.startsWith("--")) {
      usageError(
        `${token} requires a value that is not another flag (got ${value === undefined ? "nothing" : JSON.stringify(value)})`,
      );
    }
    values[token] = value;
    i++; // consume the value: no token is read twice
  }
  return {
    outDir: values["--out-dir"] ?? join(homedir(), "atlas", "meta-quota-export"),
    cdpPort: parseCdpPort(values["--cdp-port"] ?? "19222"),
    teamId: values["--team-id"] ?? process.env.META_TEAM_ID ?? DEFAULT_TEAM_ID,
  };
}

async function main() {

async function main() {
  let options;
  try {
    options = parseArgs(process.argv.slice(2));
  } catch (error) {
    if (error instanceof ExportError && error.status === "usage") {
      console.error(`meta-quota-export: ${error.message}; refusing to run, no output written`);
      process.exit(2);
    }
    throw error;
  }
  const { outDir, cdpPort, teamId } = options;
  try {
    const doc = await run({ cdpPort, outDir, teamId });
    await writeOutputs(outDir, doc);
    console.log(JSON.stringify({ status: doc.status, measuredAt: doc.measuredAt, outDir }));
  } catch (e) {
    const err =
      e instanceof ExportError
        ? e
        : new ExportError("fetch-failed", 7, `unexpected: ${e.message ?? String(e)}`);
    const doc = failureDocument({
      attemptedAt: new Date().toISOString(),
      status: err.status,
      detail: err.message,
      recovery: err.recovery ?? null,
    });
    try {
      await writeOutputs(outDir, doc);
    } catch (writeErr) {
      console.error(`meta-quota-export: could not write outputs: ${writeErr.message}`);
    }
    console.error(`meta-quota-export: ${err.status}: ${err.message}`);
    process.exit(err.exitCode);
  }
}

const isMain = (() => {
  try {
    if (!process.argv[1]) return false;
    return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
})();
if (isMain) {
  await main();
}
