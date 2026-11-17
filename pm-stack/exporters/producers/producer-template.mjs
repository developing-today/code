#!/usr/bin/env node
// Producer template for browser-dependent quota sources (#6032).
//
// Fork this file to add a source. It carries the plumbing discipline
// distilled from the retired Meta producer
// (~/atlas/meta-quota-export/meta-quota-export.mjs):
//
//   - CDP plumbing with a per-call timeout on EVERY browser call —
//     a wedged renderer must never hang the producer past its unit's
//     TimeoutStartSec;
//   - tab ownership: a producer closes only tabs it created
//     (code#5121), never the browser's standing tabs;
//   - failure-class exit codes: the producer pre-classifies its own
//     failures into the watchdog's vocabulary and writes a failure
//     DOCUMENT (not just a log line), so the watchdog can classify
//     without scraping journals;
//   - atomic latest.json (tmp + rename) + append-only history.jsonl,
//     so readers never observe a torn document;
//   - no secret value in any document, log, or exit message: cookie
//     checks are presence-by-name, extracted tokens go straight to
//     fleet secrets under their registered name.
//
// What a fork fills in: ADAPTER (origins, cookie names, the read
// itself in `readQuota`), and its adapter JSON (see
// ../adapters/fixture.json for the shape). Register the fork with the
// gatherer by adding its document to the aggregator's file-export
// inputs (the `FILE_EXPORTS` list in the pm-aggregator package) —
// until that lands, the watchdog still watches the document directly.
//
// Exit codes (failure classes the watchdog maps, watchdog-state.ts):
//   0 ok · 3 session-expired · 4 cdp-unreachable · 5 rate-limited
//   6 cookie-missing · 7 endpoint-changed · 8 unknown

import { appendFile, mkdir, rename, writeFile } from "node:fs/promises";
import { join } from "node:path";

const ADAPTER = {
  name: "template",
  cdpPort: 19222,
  origins: ["https://example.invalid"],
  cookieNames: [],
  outDir: process.env.PRODUCER_OUT_DIR ?? "",
};

export class ExportError extends Error {
  constructor(status, exitCode, detail) {
    super(detail);
    this.status = status;
    this.exitCode = exitCode;
  }
}

const EXIT = {
  ok: 0,
  "session-expired": 3,
  "cdp-unreachable": 4,
  "rate-limited": 5,
  "cookie-missing": 6,
  "endpoint-changed": 7,
  unknown: 8,
};

/** One CDP HTTP call, timeout-bounded. */
async function cdpJson(cdpBase, path, { method = "GET", timeoutMs = 10000 } = {}) {
  const ctl = new AbortController();
  const t = setTimeout(() => ctl.abort(), timeoutMs);
  try {
    const res = await fetch(`${cdpBase}${path}`, { method, signal: ctl.signal });
    if (!res.ok) throw new Error(`CDP ${path}: HTTP ${res.status}`);
    return await res.json();
  } finally {
    clearTimeout(t);
  }
}

export function failureDocument({ attemptedAt, status, detail }) {
  return {
    service: ADAPTER.name,
    source: `file-export/${ADAPTER.name}`,
    status,
    measuredAt: attemptedAt,
    detail,
    windows: [],
  };
}

async function publish(outDir, doc) {
  await mkdir(outDir, { recursive: true });
  const latest = join(outDir, "latest.json");
  const tmp = `${latest}.tmp-${process.pid}`;
  await writeFile(tmp, `${JSON.stringify(doc, null, 2)}\n`);
  await rename(tmp, latest);
  await appendFile(join(outDir, "history.jsonl"), `${JSON.stringify({ at: doc.measuredAt, status: doc.status })}\n`);
}

/** The fork's actual read: open (and OWN) a tab on an allowlist
 *  origin, scrape the quota windows, return the success document.
 *  Throw ExportError with a pre-classified status on any failure. */
async function readQuota(cdpBase) {
  let target = null;
  try {
    target = await cdpJson(cdpBase, `/json/new?${encodeURIComponent(ADAPTER.origins[0])}`, { method: "PUT" });
    // TODO(fork): drive the page, extract windows, build the doc.
    return {
      service: ADAPTER.name,
      source: `file-export/${ADAPTER.name}`,
      status: "ok",
      measuredAt: new Date().toISOString(),
      windows: [],
    };
  } finally {
    if (target?.id) {
      // Tab ownership: close only the tab this run created.
      await cdpJson(cdpBase, `/json/close/${target.id}`).catch(() => {});
    }
  }
}

export async function run({ cdpPort = ADAPTER.cdpPort, outDir = ADAPTER.outDir } = {}) {
  if (!outDir) throw new Error("PRODUCER_OUT_DIR is required");
  const attemptedAt = new Date().toISOString();
  const cdpBase = `http://127.0.0.1:${cdpPort}`;
  try {
    await cdpJson(cdpBase, "/json/version", { timeoutMs: 5000 });
  } catch (e) {
    const doc = failureDocument({
      attemptedAt,
      status: "cdp-unavailable",
      detail: `CDP ${cdpBase}/json/version unreachable: ${e.message}`,
    });
    await publish(outDir, doc);
    return EXIT["cdp-unreachable"];
  }
  try {
    const doc = await readQuota(cdpBase);
    await publish(outDir, doc);
    return EXIT.ok;
  } catch (e) {
    const status = e instanceof ExportError ? e.status : "unknown";
    const doc = failureDocument({
      attemptedAt,
      status,
      detail: e instanceof Error ? e.message : String(e),
    });
    await publish(outDir, doc);
    return e instanceof ExportError ? e.exitCode : EXIT.unknown;
  }
}

import { fileURLToPath } from "node:url";
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  process.exit(await run());
}
