import { describe, expect, test, afterEach } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { raiseNeedsDrew, recordAnswer } from "../src/needs-drew.ts";
import {
  applyAlertKey,
  applyRetryDelayMs,
  buildApplyFailureIncident,
  readApplyLedger,
  runApplierTick,
  type DecideResult,
} from "../src/actions-exec-apply.ts";

const tempDirs: string[] = [];
afterEach(async () => {
  await Promise.all(tempDirs.splice(0).map((d) => rm(d, { recursive: true, force: true })));
});

async function files() {
  const dir = await mkdtemp(join(tmpdir(), "actions-exec-apply-alerts-"));
  tempDirs.push(dir);
  return {
    needsDrewFile: join(dir, "needs-drew.json"),
    applyFile: join(dir, "apply.json"),
  };
}

const q = (ledgerId: string) => ({
  ledgerId,
  question: "Extend?",
  options: [
    { id: "approve", label: "Approve" },
    { id: "deny", label: "Deny" },
  ],
  links: [],
});

async function answerApprove(file: string, ledgerId: string) {
  await raiseNeedsDrew(file, q(ledgerId), "fleet-actions-exec", ledgerId);
  const updated = await recordAnswer(file, ledgerId, {
    optionId: "approve",
    via: "test",
  });
  if (!updated?.answer) throw new Error("answer not recorded");
  return updated;
}

describe("actions-exec applier failure alerts S4 (#14043 §4)", () => {
  test("alert key and incident shape: source/key/severity name the failure", () => {
    expect(applyAlertKey("h@2026-11-27T03:00:00Z")).toBe("actions-exec-apply:h@2026-11-27T03:00:00Z");
    const incident = buildApplyFailureIncident(
      {
        answerId: "actions-exec-ext:h1:1@2026-11-27T03:00:00Z",
        ledgerId: "actions-exec-ext:h1:1",
        handle: "h1",
        approve: true,
        answeredAt: "2026-11-27T03:00:00Z",
        state: "failed",
        attempts: 1,
        updatedAt: "2026-11-27T03:00:01Z",
      },
      "broker unreachable: reset",
    );
    expect(incident.source).toBe("fleet-actions-exec");
    expect(incident.key).toBe("actions-exec-apply:actions-exec-ext:h1:1@2026-11-27T03:00:00Z");
    // Design §4 says "high"; the throat vocabulary is critical |
    // warning | info and only critical pushes — so high maps to critical.
    expect(incident.severity).toBe("critical");
    expect(incident.title).toMatch(/h1/);
    expect(incident.body ?? "").toMatch(/broker unreachable/);
  });

  test("bounded backoff: attempts grow the delay and cap", () => {
    expect(applyRetryDelayMs(1)).toBe(60_000);
    expect(applyRetryDelayMs(2)).toBe(120_000);
    expect(applyRetryDelayMs(3)).toBe(240_000);
    expect(applyRetryDelayMs(100)).toBe(30 * 60_000);
  });

  test("forced broker failure raises exactly one alert and stays failed", async () => {
    const { needsDrewFile, applyFile } = await files();
    const answered = await answerApprove(needsDrewFile, "actions-exec-ext:fail1:1");
    const raised: unknown[] = [];
    const res = await runApplierTick({
      needsDrewFile,
      applyFile,
      brokerUrl: "http://broker",
      brokerTokenFile: "unused",
      readTokenFn: async () => "tok",
      resolveKeyFn: async () => "secret",
      decideFn: async (): Promise<DecideResult> => ({
        ok: false,
        exit: 3,
        error: "broker unreachable: reset",
      }),
      raiseAlertFn: async (incident) => {
        raised.push(incident);
        return { created: true };
      },
    });
    expect(res.failed).toBe(1);
    expect(raised).toHaveLength(1);
    const ledger = await readApplyLedger(applyFile);
    expect(ledger).toHaveLength(1);
    expect(ledger[0].state).toBe("failed");
    expect(ledger[0].attempts).toBe(1);
    expect(typeof ledger[0].nextRetryAt).toBe("string");
    expect(ledger[0].alertRaisedAt).toBeDefined();
    expect(answered.answer).toBeDefined();
  });

  test("backoff window skips the decide call but still reports the row", async () => {
    const { needsDrewFile, applyFile } = await files();
    await answerApprove(needsDrewFile, "actions-exec-ext:backoff:1");
    let calls = 0;
    const base = {
      needsDrewFile,
      applyFile,
      brokerUrl: "http://broker",
      brokerTokenFile: "unused",
      readTokenFn: async () => "tok",
      resolveKeyFn: async () => "secret",
      decideFn: async (): Promise<DecideResult> => {
        calls += 1;
        return { ok: false, exit: 3, error: "broker unreachable: reset" };
      },
      raiseAlertFn: async () => ({ created: true }),
    };
    let nowMs = Date.parse("2026-11-27T03:00:00Z");
    await runApplierTick({ ...base, now: () => nowMs });
    expect(calls).toBe(1);
    // 30s later the 60s backoff has not elapsed: no second decide call,
    // no duplicate alert.
    nowMs += 30_000;
    let raised = 0;
    await runApplierTick({
      ...base,
      now: () => nowMs,
      raiseAlertFn: async () => {
        raised += 1;
        return { created: false };
      },
    });
    expect(calls).toBe(1);
    expect(raised).toBe(0);
    // Past the backoff the retry fires.
    nowMs += 31_000;
    await runApplierTick({ ...base, now: () => nowMs });
    expect(calls).toBe(2);
  });

  test("terminal signature refusal never retries and raises no failure alert", async () => {
    const { needsDrewFile, applyFile } = await files();
    await answerApprove(needsDrewFile, "actions-exec-ext:termsig:1");
    let calls = 0;
    let raised = 0;
    const deps = {
      needsDrewFile,
      applyFile,
      brokerUrl: "http://broker",
      brokerTokenFile: "unused",
      readTokenFn: async () => "tok",
      resolveKeyFn: async () => "secret",
      decideFn: async (): Promise<DecideResult> => {
        calls += 1;
        return { ok: false, exit: 4, error: "bad signature" };
      },
      raiseAlertFn: async () => {
        raised += 1;
        return { created: true };
      },
    };
    const res = await runApplierTick(deps);
    expect(res.rejected).toBe(1);
    expect(raised).toBe(0);
    await runApplierTick(deps);
    expect(calls).toBe(1);
  });

  test("throat outage keeps the failure; the alert is re-raised on recovery", async () => {
    const { needsDrewFile, applyFile } = await files();
    await answerApprove(needsDrewFile, "actions-exec-ext:throat:1");
    const failing: DecideResult = {
      ok: false,
      exit: 3,
      error: "broker unreachable: reset",
    };
    let nowMs = Date.parse("2026-11-27T03:00:00Z");
    const base = {
      needsDrewFile,
      applyFile,
      brokerUrl: "http://broker",
      brokerTokenFile: "unused",
      readTokenFn: async () => "tok",
      resolveKeyFn: async () => "secret",
      decideFn: async (): Promise<DecideResult> => failing,
      now: () => nowMs,
    };
    // Throat down: the store getter returns null, so no alert goes out
    // but the failed row keeps the pending alert.
    await runApplierTick({ ...base, getAlertsStoreFn: async () => null });
    let ledger = await readApplyLedger(applyFile);
    expect(ledger[0].state).toBe("failed");
    expect(ledger[0].alertRaisedAt).toBeUndefined();
    // Recovery after the backoff: the pending alert goes out even though
    // the decide retry also fires — exactly one alert for the answer.
    nowMs += 61_000;
    const raised: unknown[] = [];
    await runApplierTick({
      ...base,
      getAlertsStoreFn: async () => ({
        postIncident: async (incident: unknown) => {
          raised.push(incident);
          return { created: true };
        },
      }),
    });
    expect(raised).toHaveLength(1);
    ledger = await readApplyLedger(applyFile);
    expect(ledger[0].alertRaisedAt).toBeDefined();
  });
});
