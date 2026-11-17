// #8605: answering a needs-drew question closes the alert incident its
// raise opened. The raise side posts source "fleet-queue" / key
// `needs-drew:<ledgerId>` (pkgs/fleet/crates/fleet-queue/src/needs_drew.rs);
// the answer path must close with exactly that pair. These tests pin the
// close helper's contract and the answer handler's wiring: an answer is
// the primary outcome and a failed close never fails it.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  closeAnsweredNeedsDrewIncident,
  needsDrewCloseDiagnostics,
  needsDrewIncidentKey,
  NEEDS_DREW_DEFAULT_ALERT_SOURCE,
  type NeedsDrewIncidentCloser,
} from "../src/needs-drew.ts";

const AGG_DIR = join(import.meta.dir, "..");

describe("closeAnsweredNeedsDrewIncident", () => {
  test("closes with the item's stored source and the raise side's key", async () => {
    const calls: { source: string; key: string }[] = [];
    const closer: NeedsDrewIncidentCloser = {
      async closeOpenIncidentByKey(source, key) {
        calls.push({ source, key });
        return { transitioned: true };
      },
    };
    const out = await closeAnsweredNeedsDrewIncident(
      { ledgerId: "q-1", source: "fleet-queue" },
      async () => closer,
    );
    expect(out).toEqual({ ok: true, source: "fleet-queue", key: "needs-drew:q-1", transitioned: true });
    expect(calls).toEqual([{ source: "fleet-queue", key: "needs-drew:q-1" }]);
  });

  test("an item raised without a source closes under the raise default", async () => {
    let seen = "";
    const out = await closeAnsweredNeedsDrewIncident({ ledgerId: "q-2", source: "" }, async () => ({
      async closeOpenIncidentByKey(source) {
        seen = source;
        return { transitioned: false };
      },
    }));
    expect(out).toEqual({
      ok: true,
      source: NEEDS_DREW_DEFAULT_ALERT_SOURCE,
      key: "needs-drew:q-2",
      transitioned: false,
    });
    expect(seen).toBe("fleet-queue");
  });

  test("a throwing store returns a failure and never throws", async () => {
    const out = await closeAnsweredNeedsDrewIncident({ ledgerId: "q-3", source: "fleet-queue" }, async () => ({
      async closeOpenIncidentByKey() {
        throw new Error("boom");
      },
    }));
    expect(out).toEqual({ ok: false, source: "fleet-queue", key: "needs-drew:q-3", error: "boom" });
  });

  test("an unavailable store is a recorded failure, not a crash", async () => {
    const before = needsDrewCloseDiagnostics().count;
    const out = await closeAnsweredNeedsDrewIncident({ ledgerId: "q-4", source: "fleet-queue" }, async () => null);
    expect(out.ok).toBe(false);
    if (!out.ok) expect(out.error).toBe("alerts store unavailable");
    const after = needsDrewCloseDiagnostics();
    expect(after.count).toBe(before + 1);
    expect(after.failures.at(-1)).toMatchObject({ ledgerId: "q-4", key: "needs-drew:q-4" });
  });

  test("the key spelling matches the raise side exactly", () => {
    expect(needsDrewIncidentKey("ledger-42")).toBe("needs-drew:ledger-42");
  });
});

// --- The answer handler wiring ---------------------------------------------
// Boots the real server against a seeded needs-drew file. The alerts
// store is whatever this host resolves; either way the answer must be
// 200 and carry the incidentClose outcome, and the recorded answer must
// survive a close failure.

const LEDGER = "probe-8605-answer-close";
const TOKEN = "tok-8605-answer-close";
let dir: string;
let proc: ReturnType<typeof Bun.spawn> | null = null;
let base = "";

function freePort(): number {
  const s = Bun.serve({ port: 0, fetch: () => new Response("x") });
  const p = s.port;
  s.stop();
  if (p === undefined) throw new Error("freePort: Bun.serve did not report a port");
  return p;
}

beforeAll(async () => {
  dir = await mkdtemp(join(tmpdir(), "needs-drew-close-8605-"));
  const file = join(dir, "needs-drew.json");
  await writeFile(
    file,
    JSON.stringify({
      version: 1,
      items: [
        {
          ledgerId: LEDGER,
          question: "Close the incident when this is answered?",
          options: [{ id: "seen", label: "Seen" }],
          links: [],
          token: TOKEN,
          raisedAt: new Date().toISOString(),
          source: "fleet-queue",
        },
      ],
    }),
  );
  const port = freePort();
  base = `http://127.0.0.1:${port}`;
  proc = Bun.spawn(["bun", "run", "src/server.ts"], {
    cwd: AGG_DIR,
    env: {
      ...process.env,
      PM_AGG_SOURCE: "fixtures",
      PM_AGG_DEV: "1",
      PM_AGG_PORT: String(port),
      PM_AGG_HOSTS: "127.0.0.1",
      PM_AGG_FRESHNESS_PORT: String(freePort()),
      PM_AGG_NEEDS_DREW_FILE: file,
      PM_AGG_DECISIONS_FILE: join(dir, "decisions.json"),
      PM_AGG_DECISION_EVENTS_FILE: join(dir, "decision-events.jsonl"),
      PM_AGG_DECISION_OUTBOX_FILE: join(dir, "decision-outbox.jsonl"),
    },
    stdout: "ignore",
    stderr: "ignore",
  });
  const deadline = Date.now() + 30_000;
  for (;;) {
    try {
      const res = await fetch(`${base}/api/health`);
      if (res.status === 200) break;
    } catch {
      /* not up yet */
    }
    if (Date.now() > deadline) throw new Error("aggregator did not come up");
    await Bun.sleep(200);
  }
}, 60_000);

afterAll(async () => {
  proc?.kill();
  if (dir) await rm(dir, { recursive: true, force: true });
});

describe("POST /api/needs-drew/:ledgerId/answer closes the incident", () => {
  test("a valid answer returns 200 with the incidentClose outcome", async () => {
    const res = await fetch(`${base}/api/needs-drew/${encodeURIComponent(LEDGER)}/answer`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ token: TOKEN, optionId: "seen" }),
    });
    expect(res.status).toBe(200);
    const body = (await res.json()) as {
      answered: boolean;
      answer: { optionId: string };
      incidentClose: { source: string; key: string; ok: boolean; closed: boolean; error?: string };
    };
    // The answer is the primary outcome and is always recorded.
    expect(body.answered).toBe(true);
    expect(body.answer.optionId).toBe("seen");
    // The close targets the raise side's exact pair.
    expect(body.incidentClose.source).toBe("fleet-queue");
    expect(body.incidentClose.key).toBe(`needs-drew:${LEDGER}`);
    expect(typeof body.incidentClose.closed).toBe("boolean");
    // A failed close is reported, never silent, and never a failed answer.
    if (!body.incidentClose.ok) expect(body.incidentClose.error).toBeTruthy();
  }, 60_000);

  test("the recorded answer survives and a repeat answer is idempotent", async () => {
    const res = await fetch(`${base}/answer/${encodeURIComponent(LEDGER)}?t=${TOKEN}`);
    expect(res.status).toBe(200);
    const page = await res.text();
    expect(page).toContain("Answered");
  }, 30_000);
});
