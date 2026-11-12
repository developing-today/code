// Tests for the pure parsing core of meta-quota-export.mjs.
// Run: node --test pm-stack/meta-exporter/meta-quota-export.test.mjs
import assert from "node:assert/strict";
import { test } from "node:test";
import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { mkdtemp, readFile, symlink } from "node:fs/promises";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  ExportError,
  attemptBoundedRestart,
  failureDocument,
  PAGE_NOT_READY_EXIT,
  probeCdpReadiness,
  restartAllowed,
  parseBillingBanner,
  parsePaygBalance,
  parseSubscriptionQuota,
  successDocument,
} from "./meta-quota-export.mjs";

// The live payload captured 2026-10-27 (#1167): 1.02% / 8.83%.
const QUOTA = {
  subscription_quota: {
    tier_id: "27681527378179523",
    tier: "Muse Code High Usage",
    as_of: 1793077399,
    window_weighted_limit: "100000000000",
    window_duration_secs: 18000,
    window_weighted_used: "1023570000",
    window_resets_at: 1793090405,
    weekly_weighted_limit: "300000000000",
    weekly_weighted_used: "26484231500",
    weekly_resets_at: 1793577600,
  },
};

// A not-yet-started short window (#1301): zero usage, no window reset.
const QUOTA_INACTIVE_SHORT = {
  subscription_quota: {
    tier_id: "27681527378179523",
    tier: "Muse Code High Usage",
    as_of: 1793328541,
    window_weighted_limit: "100000000000",
    window_duration_secs: 18000,
    weekly_weighted_limit: "300000000000",
    weekly_resets_at: 1793577600,
    window_weighted_used: "0",
    weekly_weighted_used: "70873201100",
  },
};

test("parseSubscriptionQuota: percentages and resets match the console", () => {
  const parsed = parseSubscriptionQuota(QUOTA);
  assert.equal(parsed.tier, "Muse Code High Usage");
  assert.deepEqual(parsed.windows, [
    { name: "5h", usedPct: 1.02, resetAt: "2026-10-27T08:40:05.000Z" },
    { name: "weekly", usedPct: 8.83, resetAt: "2026-11-02T00:00:00.000Z" },
  ]);
  assert.equal(parsed.consoleAsOf, "2026-10-27T05:03:19.000Z"); // 00:03:19 CDT
});

test("parseSubscriptionQuota: shape drift fails closed", () => {
  assert.throws(() => parseSubscriptionQuota({}), /unparseable|missing/);
  assert.throws(() => parseSubscriptionQuota({ subscription_quota: { window_weighted_used: "1" } }));
  const zeroLimit = structuredClone(QUOTA);
  zeroLimit.subscription_quota.window_weighted_limit = "0";
  assert.throws(() => parseSubscriptionQuota(zeroLimit), /zero limit/);
});

test("parseBillingBanner: minor units to USD, currency checked", () => {
  assert.equal(parseBillingBanner({ kind: "free_partial", remaining_minor_units: 14857, currency: "USD" }), 148.57);
  assert.throws(() => parseBillingBanner({ remaining_minor_units: 100, currency: "EUR" }), /currency/);
  assert.throws(() => parseBillingBanner({}));
  // Fractional minor units are shape drift: fail closed, never export a fractional dollar amount (#2510).
  assert.throws(() => parseBillingBanner({ remaining_minor_units: 1050.5, currency: "USD" }), /missing\/invalid/);
  assert.throws(() => parseBillingBanner({ remaining_minor_units: 0.1, currency: "USD" }), /missing\/invalid/);
  assert.equal(parseBillingBanner({ remaining_minor_units: 0, currency: "USD" }), 0);
  // Unsafe integers and negatives reject; MAX_SAFE_INTEGER itself is the
  // representation limit, not a business cap.
  assert.throws(() => parseBillingBanner({ remaining_minor_units: Number.MAX_SAFE_INTEGER + 1, currency: "USD" }), /missing\/invalid/);
  assert.throws(() => parseBillingBanner({ remaining_minor_units: -100, currency: "USD" }), /missing\/invalid/);
});

test("parsePaygBalance: reads the iframe text, absence stays null", () => {
  assert.equal(parsePaygBalance("Pay as you go\nCurrent balance\n$0.00\nYour balance won't increase"), 0);
  assert.equal(parsePaygBalance("Current balance\n$1,234.56"), 1234.56);
  assert.equal(parsePaygBalance("nothing relevant here"), null);
  assert.equal(parsePaygBalance(undefined), null);
});

test("documents: success carries the spec fields; failure carries none", () => {
  const ok = successDocument({
    measuredAt: "2026-10-27T05:00:00.000Z",
    parsed: parseSubscriptionQuota(QUOTA),
    freeCreditsUsd: 148.57,
    paygBalanceUsd: 0,
  });
  assert.equal(ok.service, "meta");
  assert.equal(ok.source, "file-export/meta-laptop");
  assert.equal(ok.status, "ok");
  assert.equal(ok.windows.length, 2);

  const bad = failureDocument({ attemptedAt: "2026-10-27T05:00:00.000Z", status: "session-expired", detail: "HTTP 401" });
  assert.equal(bad.measuredAt, null);
  assert.equal("windows" in bad, false);
  assert.equal("freeCreditsUsd" in bad, false);
});

// ---------- Readiness probe fixtures (issue #5487) ----------
// A fixture CDP endpoint: HTTP answers /json/new like a real browser,
// and the websocket either never replies (wedged renderer) or answers
// CDP calls (healthy renderer). The wedge case is the regression: the
// probe must report not-ready instead of hanging or passing.

function wsAccept(key) {
  return createHash("sha1").update(key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest("base64");
}

function wsFrame(text) {
  const payload = Buffer.from(text);
  let header;
  if (payload.length < 126) {
    header = Buffer.from([0x81, payload.length]);
  } else if (payload.length < 65536) {
    header = Buffer.alloc(4);
    header[0] = 0x81; header[1] = 126; header.writeUInt16BE(payload.length, 2);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x81; header[1] = 127; header.writeBigUInt64BE(BigInt(payload.length), 2);
  }
  return Buffer.concat([header, payload]);
}

function startFixtureCdp({ wedged }) {
  const sockets = new Set();
  const track = (sock) => { sockets.add(sock); sock.on("close", () => sockets.delete(sock)); };
  const server = createServer((req, res) => {
    if (req.url.startsWith("/json/new")) {
      res.setHeader("content-type", "application/json");
      res.end(JSON.stringify({
        id: "probe-target-1",
        type: "page",
        url: "about:blank",
        webSocketDebuggerUrl: `ws://127.0.0.1:${server.address().port}/ws`,
      }));
    } else if (req.url.startsWith("/json/close")) {
      res.end("Target is closing");
    } else if (req.url.startsWith("/json/list")) {
      res.setHeader("content-type", "application/json");
      res.end("[]");
    } else {
      res.statusCode = 404;
      res.end("nope");
    }
  });
  server.on("connection", track);
  server.on("upgrade", (req, socket) => {
    track(socket);
    socket.write(
      "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n" +
      `Sec-WebSocket-Accept: ${wsAccept(req.headers["sec-websocket-key"])}\r\n\r\n`,
    );
    if (wedged) return; // accept the socket, never reply — the wedge
    let buf = Buffer.alloc(0);
    socket.on("data", (chunk) => {
      buf = Buffer.concat([buf, chunk]);
      for (;;) {
        if (buf.length < 2) return;
        const masked = (buf[1] & 0x80) !== 0;
        let len = buf[1] & 0x7f;
        let off = 2;
        if (len === 126) { if (buf.length < 4) return; len = buf.readUInt16BE(2); off = 4; }
        else if (len === 127) { if (buf.length < 10) return; len = Number(buf.readBigUInt64BE(2)); off = 10; }
        const maskOff = off;
        if (masked) off += 4;
        if (buf.length < off + len) return;
        let payload = buf.subarray(off, off + len);
        if (masked) {
          const mask = buf.subarray(maskOff, maskOff + 4);
          payload = Buffer.from(payload.map((b, i) => b ^ mask[i % 4]));
        }
        buf = buf.subarray(off + len);
        let msg;
        try { msg = JSON.parse(payload.toString()); } catch { continue; }
        if (!msg.id) continue;
        const result = msg.method === "Runtime.evaluate" ? { result: { value: 1 } } : {};
        socket.write(wsFrame(JSON.stringify({ id: msg.id, result })));
      }
    });
  });
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      resolve({
        base: `http://127.0.0.1:${server.address().port}`,
        close: () => new Promise((r) => { for (const sock of sockets) sock.destroy(); server.close(r); }),
      });
    });
  });
}

test("probeCdpReadiness: a CDP that answers /json/new but never replies to navigate/evaluate is wedged", async () => {
  const fx = await startFixtureCdp({ wedged: true });
  try {
    const r = await probeCdpReadiness({ cdpBase: fx.base, timeoutMs: 2000 });
    assert.equal(r.ok, false);
    assert.match(r.detail, /never replied|wedged/);
  } finally {
    await fx.close();
  }
});

test("probeCdpReadiness: a healthy renderer passes", async () => {
  const fx = await startFixtureCdp({ wedged: false });
  try {
    const r = await probeCdpReadiness({ cdpBase: fx.base, timeoutMs: 4000 });
    assert.deepEqual(r, { ok: true });
  } finally {
    await fx.close();
  }
});

test("restartAllowed: at most once per 15 minutes", () => {
  const now = Date.parse("2026-11-05T12:00:00.000Z");
  assert.equal(restartAllowed(null, now), true);
  assert.equal(restartAllowed("2026-11-05T11:50:00.000Z", now), false);
  assert.equal(restartAllowed("2026-11-05T11:45:00.000Z", now), true);
  assert.equal(restartAllowed("not-a-date", now), true);
});

test("attemptBoundedRestart: restarts once, then the persisted bound skips", async () => {
  const outDir = await mkdtemp(join(tmpdir(), "meta-exporter-restart-"));
  let calls = 0;
  const restart = async () => { calls += 1; };
  const first = await attemptBoundedRestart({ outDir, now: new Date("2026-11-05T12:00:00.000Z"), restart });
  assert.equal(first.result, "restarted");
  assert.equal(calls, 1);
  const state = JSON.parse(await readFile(join(outDir, ".helium-restart.json"), "utf8"));
  assert.equal(state.lastRestartAt, "2026-11-05T12:00:00.000Z");
  const second = await attemptBoundedRestart({ outDir, now: new Date("2026-11-05T12:05:00.000Z"), restart });
  assert.equal(second.result, "skipped-cooldown");
  assert.equal(calls, 1);
});

test("attemptBoundedRestart: a failed restart still persists the cooldown bound", async () => {
  const outDir = await mkdtemp(join(tmpdir(), "meta-exporter-restart-fail-"));
  let calls = 0;
  const restart = async () => { calls += 1; throw new Error("restart boom"); };
  const first = await attemptBoundedRestart({ outDir, now: new Date("2026-11-05T12:00:00.000Z"), restart });
  assert.equal(first.result, "restart-failed");
  assert.equal(calls, 1);
  const state = JSON.parse(await readFile(join(outDir, ".helium-restart.json"), "utf8"));
  assert.equal(state.lastRestartAt, "2026-11-05T12:00:00.000Z");
  const second = await attemptBoundedRestart({ outDir, now: new Date("2026-11-05T12:05:00.000Z"), restart });
  assert.equal(second.result, "skipped-cooldown");
  assert.equal(calls, 1);
});

test("documents: page-not-ready is a distinct status/exit and recovery is recorded", () => {
  assert.notEqual(PAGE_NOT_READY_EXIT, 5);
  const recovery = { action: "restart", unit: "helium-cdp.service", at: "2026-11-05T12:00:00.000Z", result: "restarted" };
  const bad = failureDocument({ attemptedAt: "2026-11-05T12:00:00.000Z", status: "page-not-ready", detail: "wedged", recovery });
  assert.equal(bad.status, "page-not-ready");
  assert.deepEqual(bad.recovery, recovery);
  assert.equal("windows" in bad, false);
  const plain = failureDocument({ attemptedAt: "2026-11-05T12:00:00.000Z", status: "session-expired", detail: "HTTP 401" });
  assert.equal("recovery" in plain, false);
});

// ---------- Entry-point guard (issue #6343) ----------
// Regression: the re-homed exporter guarded main() with the raw-path
// comparison `import.meta.url === pathToFileURL(process.argv[1]).href`,
// so any symlinked invocation silently skipped main() and exited 0 —
// the #2350 bug, restored by the #6333 re-home. The guard now resolves
// symlinks on both sides (realpathSync, mirroring
// supabase-quota-export.mjs / cloudflare-quota-export.mjs). Oracle: a run
// against an unreachable CDP port must produce the cdp-unavailable
// failure document with exit 4, identically for direct and symlinked
// invocation; a silent exit 0 is the counterexample.

const execFileAsync = promisify(execFile);
const EXPORTER_PATH = fileURLToPath(new URL("./meta-quota-export.mjs", import.meta.url));

async function runExporter(entryPath, outDir) {
  try {
    const { stdout, stderr } = await execFileAsync(
      process.execPath,
      [entryPath, "--cdp-port", "1", "--out-dir", outDir],
      { timeout: 30000 },
    );
    return { code: 0, stdout, stderr };
  } catch (err) {
    return { code: err.code, stdout: err.stdout ?? "", stderr: err.stderr ?? "" };
  }
}

async function assertCdpUnavailableRun(entryPath, label) {
  const base = await mkdtemp(join(tmpdir(), `meta-exporter-entry-${label}-`));
  const outDir = join(base, "out");
  const r = await runExporter(entryPath, outDir);
  assert.equal(r.code, 4, `${label} invocation must exit 4, got ${r.code} (stderr: ${r.stderr})`);
  assert.match(r.stderr, /cdp-unavailable/);
  const latest = JSON.parse(await readFile(join(outDir, "latest.json"), "utf8"));
  assert.equal(latest.status, "cdp-unavailable");
  const history = await readFile(join(outDir, "history.jsonl"), "utf8");
  assert.equal(JSON.parse(history.trim()).status, "cdp-unavailable");
}

test("entry-point guard: direct invocation runs main (cdp-unavailable, exit 4)", async () => {
  await assertCdpUnavailableRun(EXPORTER_PATH, "direct");
});

test("entry-point guard: symlinked invocation runs main, matching direct (#6343)", async () => {
  const base = await mkdtemp(join(tmpdir(), "meta-exporter-entry-link-"));
  const linkPath = join(base, "meta-quota-export-link.mjs");
  await symlink(EXPORTER_PATH, linkPath);
  await assertCdpUnavailableRun(linkPath, "symlink");
});
// #2789: num() used to be a bare Number() coercion, so drifted
// spellings became real measurements ("" -> 0% window, "0x10" -> 16,
// "1e2" -> 100). Only plain decimal spellings are accepted now.
test("parseSubscriptionQuota: non-decimal numeric spellings fail closed", () => {
  for (const bad of ["", "   ", "0x10", "0b101", "0o17", "1e2", "1E2", "abc", "12,000", " 123", "123 ", "1023570000n"]) {
    for (const field of ["window_weighted_used", "window_weighted_limit", "window_duration_secs"]) {
      const payload = structuredClone(QUOTA);
      payload.subscription_quota[field] = bad;
      assert.throws(() => parseSubscriptionQuota(payload), /not a finite|window_duration_secs/);
    }
  }
  // The same spellings are rejected on the weekly fields too.
  for (const bad of ["", "0x10", "1e2"]) {
    const payload = structuredClone(QUOTA);
    payload.subscription_quota.weekly_weighted_used = bad;
    assert.throws(() => parseSubscriptionQuota(payload), /weeklyUsed/);
  }
});

test("parseSubscriptionQuota: empty-string reset on a used window fails closed", () => {
  for (const bad of ["", "   "]) {
    const shortUsed = structuredClone(QUOTA);
    shortUsed.subscription_quota.window_resets_at = bad;
    assert.throws(() => parseSubscriptionQuota(shortUsed), /windowReset/);
    const weeklyUsed = structuredClone(QUOTA);
    weeklyUsed.subscription_quota.weekly_resets_at = bad;
    assert.throws(() => parseSubscriptionQuota(weeklyUsed), /weeklyReset/);
  }
});

test("parseSubscriptionQuota: empty-string reset on a zero-usage window is tolerated like absent", () => {
  const payload = structuredClone(QUOTA_INACTIVE_SHORT);
  payload.subscription_quota.window_resets_at = "";
  assert.equal(parseSubscriptionQuota(payload).windows[0].resetAt, null);
});

test("parseSubscriptionQuota: decimal spellings still parse, including fractions", () => {
  const frac = structuredClone(QUOTA);
  frac.subscription_quota.window_weighted_used = "1023570000.5";
  assert.equal(parseSubscriptionQuota(frac).windows[0].usedPct, 1.02);
});

test("parseSubscriptionQuota: empty as_of stays null; non-decimal as_of fails closed", () => {
  const empty = structuredClone(QUOTA);
  empty.subscription_quota.as_of = "";
  assert.equal(parseSubscriptionQuota(empty).consoleAsOf, null);
  const blank = structuredClone(QUOTA);
  delete blank.subscription_quota.as_of;
  assert.equal(parseSubscriptionQuota(blank).consoleAsOf, null);
  for (const bad of ["0x10", "1e2", "soon"]) {
    const payload = structuredClone(QUOTA);
    payload.subscription_quota.as_of = bad;
    assert.throws(() => parseSubscriptionQuota(payload), /unparseable|finite/);
  }
});

test("parseSubscriptionQuota: epoch fields reject values outside the pinned seconds window", () => {
  const firstAllowed = 1_577_836_800; // 2020-01-01T00:00:00Z
  const lastAllowed = 4_102_444_800; // 2100-01-01T00:00:00Z
  const cases = [
    ["window_resets_at", [firstAllowed - 1, lastAllowed + 1, 0, 1_793_090_405_000, 1e300]],
    ["weekly_resets_at", [firstAllowed - 1, lastAllowed + 1, 0, 1_793_090_405_000, 1e300]],
    ["as_of", [firstAllowed - 1, lastAllowed + 1, 0, 1_793_000_000_000, 1e300]],
  ];
  for (const [field, values] of cases) {
    for (const value of values) {
      const payload = structuredClone(QUOTA);
      payload.subscription_quota[field] = value;
      assert.throws(
        () => parseSubscriptionQuota(payload),
        (error) => {
          assert.ok(error instanceof ExportError);
          assert.equal(error.status, "unparseable");
          assert.equal(error.exitCode, 6);
          return true;
        },
        `${field}=${value} should fail as unparseable`,
      );
    }
  }
});

test("parseSubscriptionQuota: epoch window endpoints are inclusive", () => {
  const endpoints = [
    [1_577_836_800, "2020-01-01T00:00:00.000Z"],
    [4_102_444_800, "2100-01-01T00:00:00.000Z"],
  ];
  for (const [epoch, expected] of endpoints) {
    for (const field of ["window_resets_at", "weekly_resets_at"]) {
      const payload = structuredClone(QUOTA);
      payload.subscription_quota[field] = epoch;
      const parsed = parseSubscriptionQuota(payload);
      assert.equal(parsed.windows[field === "window_resets_at" ? 0 : 1].resetAt, expected);
    }
    const payload = structuredClone(QUOTA);
    payload.subscription_quota.as_of = epoch;
    assert.equal(parseSubscriptionQuota(payload).consoleAsOf, expected);
  }
});
