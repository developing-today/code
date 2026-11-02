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
 *   no-console-page  browser is up but no dev.meta.ai tab is open.
 *                    Exit 5.
 *   unparseable      an endpoint answered something whose shape this
 *                    parser does not recognise. Exit 6.
 *   fetch-failed     network/other error inside the page. Exit 7.
 * "laptop-unreachable" is deliberately NOT produced here: if the
 * laptop is down nothing runs, so consumers derive it from the age of
 * latest.json (> 2 ticks = > 30 min without a fresh document).
 */
import { appendFile, mkdir, rename, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

export const SOURCE = "file-export/meta-laptop";
export const DEFAULT_TEAM_ID = "1073226932208501"; // "My Team" (#1167)

export class ExportError extends Error {
  constructor(status, exitCode, detail) {
    super(detail);
    this.status = status;
    this.exitCode = exitCode;
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
  if (typeof minor !== "number" || !Number.isFinite(minor) || minor < 0) {
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
  const m = iframeText.match(/Current balance\s*\n?\s*\$\s*([\d,]+(?:\.\d{2})?)(?![\d.,])/);
  if (!m) return null;
  const v = Number(m[1].replace(/,/g, ""));
  return Number.isFinite(v) ? v : null;
}

export function successDocument({ measuredAt, parsed, freeCreditsUsd, paygBalanceUsd }) {
  return {
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
}

export function failureDocument({ attemptedAt, status, detail }) {
  return {
    service: "meta",
    source: SOURCE,
    status,
    measuredAt: null,
    attemptedAt,
    detail,
  };
}

// ---------- CDP plumbing (only used by the main run) ----------

const DEFAULT_TIMEOUTS = Object.freeze({
  httpMs: 10_000,
  websocketOpenMs: 10_000,
  commandMs: 30_000,
});

async function httpJson(url, timeoutMs = DEFAULT_TIMEOUTS.httpMs) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const res = await fetch(url, { signal: controller.signal });
    if (!res.ok) throw new Error(`HTTP ${res.status} for ${url}`);
    return await res.json();
  } catch (error) {
    if (controller.signal.aborted) {
      throw new Error(`HTTP request timed out after ${timeoutMs}ms`);
    }
    throw error;
  } finally {
    clearTimeout(timer);
  }
}

export function connectWs(
  wsUrl,
  {
    openTimeoutMs = DEFAULT_TIMEOUTS.websocketOpenMs,
    commandTimeoutMs = DEFAULT_TIMEOUTS.commandMs,
    WebSocketImpl = WebSocket,
  } = {},
) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocketImpl(wsUrl);
    const pending = new Map();
    let id = 0;
    let state = "connecting";

    const rejectPending = (error) => {
      for (const [pendingId, call] of pending) {
        clearTimeout(call.timer);
        pending.delete(pendingId);
        call.reject(error);
      }
    };
    const closeSocket = () => {
      try {
        ws.close();
      } catch {
        // Closing a socket that is already closing is harmless.
      }
    };
    const fail = (error) => {
      if (state === "connecting") {
        state = "closed";
        clearTimeout(openTimer);
        reject(error);
      } else if (state === "open") {
        state = "closed";
        rejectPending(error);
      }
    };
    const openTimer = setTimeout(() => {
      fail(new Error(`websocket open timed out after ${openTimeoutMs}ms`));
      closeSocket();
    }, openTimeoutMs);

    ws.onmessage = (ev) => {
      let message;
      try {
        message = JSON.parse(ev.data);
      } catch (error) {
        fail(new Error(`invalid websocket response: ${error.message}`));
        closeSocket();
        return;
      }
      if (message.id != null && pending.has(message.id)) {
        const call = pending.get(message.id);
        pending.delete(message.id);
        clearTimeout(call.timer);
        if (message.error) call.reject(new Error(JSON.stringify(message.error)));
        else call.resolve(message.result);
      }
    };
    ws.onopen = () => {
      if (state !== "connecting") return;
      state = "open";
      clearTimeout(openTimer);
      resolve({
        send: (method, params = {}) =>
          new Promise((res, rej) => {
            if (state !== "open") {
              rej(new Error("websocket is not open"));
              return;
            }
            const requestId = ++id;
            const timer = setTimeout(() => {
              pending.delete(requestId);
              rej(new Error(`websocket ${method} timed out after ${commandTimeoutMs}ms`));
            }, commandTimeoutMs);
            pending.set(requestId, { resolve: res, reject: rej, timer });
            try {
              ws.send(JSON.stringify({ id: requestId, method, params }));
            } catch (error) {
              pending.delete(requestId);
              clearTimeout(timer);
              rej(error);
            }
          }),
        close: () => {
          if (state === "open") {
            state = "closed";
            rejectPending(new Error("websocket closed by client"));
          }
          closeSocket();
        },
      });
    };
    ws.onerror = (event) => {
      fail(new Error(`websocket error: ${event.message ?? "unknown"}`));
      closeSocket();
    };
    ws.onclose = () => {
      fail(new Error("websocket closed before response"));
    };
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
async function fetchPaygBalance(cdpBase, { timeouts, WebSocketImpl } = {}) {
  try {
    const targets = await httpJson(`${cdpBase}/json/list`, timeouts.httpMs);
    const frame = targets.find(
      (t) => t.type === "iframe" && typeof t.url === "string" && t.url.includes("billing.dev.meta.ai"),
    );
    if (!frame?.webSocketDebuggerUrl) return null;
    const client = await connectWs(frame.webSocketDebuggerUrl, {
      openTimeoutMs: timeouts.websocketOpenMs,
      commandTimeoutMs: timeouts.commandMs,
      WebSocketImpl,
    });
    try {
      const text = await evaluate(client, "document.body ? document.body.innerText : ''");
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

/** Run one export and always write either its success or failure document. */
export async function exportAndWrite({ outDir, ...runOptions } = {}) {
  try {
    const doc = await run(runOptions);
    await writeOutputs(outDir, doc);
    return { doc, exitCode: 0, error: null, outputError: null };
  } catch (error) {
    const err =
      error instanceof ExportError
        ? error
        : new ExportError("fetch-failed", 7, `unexpected: ${error.message ?? String(error)}`);
    const doc = failureDocument({
      attemptedAt: new Date().toISOString(),
      status: err.status,
      detail: err.message,
    });
    let outputError = null;
    try {
      await writeOutputs(outDir, doc);
    } catch (writeError) {
      outputError = writeError;
    }
    return { doc, exitCode: err.exitCode, error: err, outputError };
  }
}

export async function run({
  cdpPort = 19222,
  outDir,
  teamId = DEFAULT_TEAM_ID,
  timeouts = {},
  WebSocketImpl = WebSocket,
} = {}) {
  const deadlines = { ...DEFAULT_TIMEOUTS, ...timeouts };
  const attemptedAt = new Date().toISOString();
  const cdpBase = `http://127.0.0.1:${cdpPort}`;
  let targets;
  try {
    targets = await httpJson(`${cdpBase}/json/list`, deadlines.httpMs);
  } catch (e) {
    throw new ExportError("cdp-unavailable", 4, `CDP ${cdpBase}/json/list unreachable: ${e.message}`);
  }
  const page = targets.find(
    (t) => t.type === "page" && typeof t.url === "string" && t.url.startsWith("https://dev.meta.ai"),
  );
  if (!page?.webSocketDebuggerUrl) {
    throw new ExportError("no-console-page", 5, "browser is up but no dev.meta.ai tab is open");
  }
  const effectiveTeam =
    new URL(page.url).searchParams.get("team_id") ?? teamId;
  const client = await connectWs(page.webSocketDebuggerUrl, {
    openTimeoutMs: deadlines.websocketOpenMs,
    commandTimeoutMs: deadlines.commandMs,
    WebSocketImpl,
  });
  let bodies;
  try {
    bodies = await fetchQuotaPayloads(client, effectiveTeam);
  } finally {
    client.close();
  }
  const parsed = parseSubscriptionQuota(bodies.quota);
  const freeCreditsUsd = parseBillingBanner(bodies.banner);
  const paygBalanceUsd = await fetchPaygBalance(cdpBase, {
    timeouts: deadlines,
    WebSocketImpl,
  });
  return successDocument({
    measuredAt: new Date().toISOString(),
    parsed,
    freeCreditsUsd,
    paygBalanceUsd,
  });
}

async function main() {
  const args = process.argv.slice(2);
  const opt = (name, fallback) => {
    const i = args.indexOf(name);
    return i >= 0 && args[i + 1] ? args[i + 1] : fallback;
  };
  const outDir = opt("--out-dir", join(homedir(), "atlas", "meta-quota-export"));
  const cdpPort = Number(opt("--cdp-port", "19222"));
  const teamId = opt("--team-id", process.env.META_TEAM_ID ?? DEFAULT_TEAM_ID);
  const outcome = await exportAndWrite({ cdpPort, outDir, teamId });
  if (outcome.exitCode === 0) {
    const { doc } = outcome;
    console.log(JSON.stringify({ status: doc.status, measuredAt: doc.measuredAt, outDir }));
    return;
  }
  if (outcome.outputError) {
    console.error(`meta-quota-export: could not write outputs: ${outcome.outputError.message}`);
  }
  console.error(`meta-quota-export: ${outcome.error.status}: ${outcome.error.message}`);
  process.exit(outcome.exitCode);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
