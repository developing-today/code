import { afterEach, describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { raiseNeedsDrew, recordAnswer } from "../src/needs-drew.ts";
import {
  answerIdFor,
  approveFromAnswer,
  readApplyLedger,
  runApplierTick,
  signedDecideMessage,
  signDecide,
  toHouseIso,
  type DecideResult,
} from "../src/actions-exec-apply.ts";

const tempDirs: string[] = [];
afterEach(async () => {
  await Promise.all(tempDirs.splice(0).map((d) => rm(d, { recursive: true, force: true })));
});

async function files() {
  const dir = await mkdtemp(join(tmpdir(), "actions-exec-apply-"));
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

async function answerApprove(file: string, ledgerId: string, source: string, key: string, optionId = "approve") {
  await raiseNeedsDrew(file, q(ledgerId), source, key);
  const updated = await recordAnswer(file, ledgerId, { optionId, via: "test" });
  if (!updated?.answer) throw new Error("answer not recorded");
  return updated;
}

describe("actions-exec applier (#14043 S3)", () => {
  test("canonical message and house ISO shape match the broker S1 contract", () => {
    expect(signedDecideMessage("h1", true, "ans", "2026-11-24T08:00:00Z")).toBe("h1|true|ans|2026-11-24T08:00:00Z");
    expect(signedDecideMessage("h1", false, "ans", "2026-11-24T08:00:00Z")).toBe("h1|false|ans|2026-11-24T08:00:00Z");
    expect(toHouseIso(Date.parse("2026-11-24T08:00:00.123Z"))).toBe("2026-11-24T08:00:00Z");
    // node:crypto HMAC-SHA256 hex is 64 lowercase hex chars.
    expect(signDecide("k", "h", true, "a", "2026-11-24T08:00:00Z")).toMatch(/^[0-9a-f]{64}$/);
  });

  test("approve mapping: approve->true, deny->false, free-text->null", async () => {
    const { needsDrewFile } = await files();
    const a = await answerApprove(
      needsDrewFile,
      "actions-exec-ext:h1:1",
      "fleet-actions-exec",
      "actions-exec-ext:h1:1",
      "approve",
    );
    expect(approveFromAnswer(a)).toBe(true);
    const d = await answerApprove(
      needsDrewFile,
      "actions-exec-ext:h2:1",
      "fleet-actions-exec",
      "actions-exec-ext:h2:1",
      "deny",
    );
    expect(approveFromAnswer(d)).toBe(false);
    await raiseNeedsDrew(needsDrewFile, q("actions-exec-ext:h3:1"), "fleet-actions-exec", "actions-exec-ext:h3:1");
    const f = await recordAnswer(needsDrewFile, "actions-exec-ext:h3:1", {
      freeText: "maybe?",
      via: "test",
    });
    expect(approveFromAnswer(f!)).toBeNull();
    expect(answerIdFor(a)).toBe(`actions-exec-ext:h1:1@${a.answer!.at}`);
  });

  test("§3 row 1: pending request decided -> applied", async () => {
    const { needsDrewFile, applyFile } = await files();
    await answerApprove(needsDrewFile, "actions-exec-ext:alpha:1", "fleet-actions-exec", "actions-exec-ext:alpha:1");
    const calls: unknown[] = [];
    const res = await runApplierTick({
      needsDrewFile,
      applyFile,
      brokerUrl: "http://broker",
      brokerTokenFile: "unused",
      readTokenFn: async () => "tok",
      resolveKeyFn: async () => "secret",
      decideFn: async (body) => {
        calls.push(body);
        return {
          ok: true,
          decided: true,
          approved: true,
          tier: 3,
          outcome: "approved",
          via: "signed-apply",
        };
      },
    });
    expect(res.applied).toBe(1);
    expect(calls).toHaveLength(1);
    const ledger = await readApplyLedger(applyFile);
    expect(ledger[0].state).toBe("applied");
    expect(ledger[0].answerId).toContain("actions-exec-ext:alpha:1@");
  });

  test("§3 row 2: silence-approved window -> superseded with note", async () => {
    const { needsDrewFile, applyFile } = await files();
    await answerApprove(
      needsDrewFile,
      "actions-exec-ext:beta:2",
      "fleet-actions-exec",
      "actions-exec-ext:beta:2",
      "deny",
    );
    const res = await runApplierTick({
      needsDrewFile,
      applyFile,
      brokerUrl: "http://broker",
      brokerTokenFile: "unused",
      readTokenFn: async () => "tok",
      resolveKeyFn: async () => "secret",
      decideFn: async () => ({
        ok: true,
        decided: false,
        approved: true,
        tier: 2,
        outcome: "silence-approved",
        via: "signed-apply",
        note: "the deny window lapsed",
      }),
    });
    expect(res.superseded).toBe(1);
    const ledger = await readApplyLedger(applyFile);
    expect(ledger[0].state).toBe("superseded");
    expect(ledger[0].note ?? "").toMatch(/lapsed/i);
  });

  test("§3 rows 3-4: fail 5 no-pending -> rejected; expired grace -> superseded", async () => {
    const { needsDrewFile, applyFile } = await files();
    await answerApprove(needsDrewFile, "actions-exec-ext:gamma:1", "fleet-actions-exec", "actions-exec-ext:gamma:1");
    await answerApprove(needsDrewFile, "actions-exec-ext:delta:1", "fleet-actions-exec", "actions-exec-ext:delta:1");
    const res = await runApplierTick({
      needsDrewFile,
      applyFile,
      brokerUrl: "http://broker",
      brokerTokenFile: "unused",
      readTokenFn: async () => "tok",
      resolveKeyFn: async () => "secret",
      decideFn: async (body): Promise<DecideResult> => {
        if (body.handle === "gamma")
          return {
            ok: false,
            exit: 5,
            error: "no extension request is pending on this session",
          };
        return {
          ok: false,
          exit: 5,
          error: "expired decision grace: session settled first",
        };
      },
    });
    expect(res.rejected).toBe(1);
    expect(res.superseded).toBe(1);
    const ledger = await readApplyLedger(applyFile);
    expect(ledger.find((r) => r.handle === "gamma")?.state).toBe("rejected");
    expect(ledger.find((r) => r.handle === "delta")?.state).toBe("superseded");
  });

  test("transient broker failure stays failed for retry (past the backoff)", async () => {
    const { needsDrewFile, applyFile } = await files();
    await answerApprove(needsDrewFile, "actions-exec-ext:eps:1", "fleet-actions-exec", "actions-exec-ext:eps:1");
    let nowMs = Date.parse("2026-11-27T03:00:00Z");
    const failing = async (): Promise<DecideResult> => ({
      ok: false,
      exit: 3,
      error: "broker unreachable: reset",
    });
    const first = await runApplierTick({
      needsDrewFile,
      applyFile,
      brokerUrl: "http://broker",
      brokerTokenFile: "unused",
      readTokenFn: async () => "tok",
      resolveKeyFn: async () => "secret",
      decideFn: failing,
      raiseAlertFn: async () => ({ created: true }),
      now: () => nowMs,
    });
    expect(first.failed).toBe(1);
    // S4 backoff: the immediate next tick does not re-call decide.
    nowMs += 61_000;
    const second = await runApplierTick({
      needsDrewFile,
      applyFile,
      brokerUrl: "http://broker",
      brokerTokenFile: "unused",
      readTokenFn: async () => "tok",
      resolveKeyFn: async () => "secret",
      decideFn: async () => ({
        ok: true,
        decided: true,
        approved: true,
        outcome: "approved",
        via: "signed-apply",
      }),
      raiseAlertFn: async () => ({ created: true }),
      now: () => nowMs,
    });
    expect(second.applied).toBe(1);
    const ledger = await readApplyLedger(applyFile);
    expect(ledger).toHaveLength(1);
    expect(ledger[0].state).toBe("applied");
  });

  test("double-tap: one decide call, second answer for same handle superseded", async () => {
    const { needsDrewFile, applyFile } = await files();
    await answerApprove(needsDrewFile, "actions-exec-ext:dbl:1", "fleet-actions-exec", "actions-exec-ext:dbl:1");
    let calls = 0;
    const deps = {
      needsDrewFile,
      applyFile,
      brokerUrl: "http://broker",
      brokerTokenFile: "unused",
      readTokenFn: async () => "tok",
      resolveKeyFn: async () => "secret",
      decideFn: async (): Promise<DecideResult> => {
        calls += 1;
        return {
          ok: true,
          decided: true,
          approved: true,
          outcome: "approved",
          via: "signed-apply",
        };
      },
    };
    await runApplierTick(deps);
    expect(calls).toBe(1);
    // A later extension on the same handle answers under a new ledger id.
    await answerApprove(
      needsDrewFile,
      "actions-exec-ext:dbl:2",
      "fleet-actions-exec",
      "actions-exec-ext:dbl:2",
      "deny",
    );
    const res = await runApplierTick(deps);
    expect(calls).toBe(1);
    expect(res.superseded).toBe(1);
    const ledger = await readApplyLedger(applyFile);
    expect(ledger).toHaveLength(2);
    expect(ledger.find((r) => r.ledgerId === "actions-exec-ext:dbl:2")?.state).toBe("superseded");
  });

  test("applied rows are never re-sent on the next tick", async () => {
    const { needsDrewFile, applyFile } = await files();
    await answerApprove(needsDrewFile, "actions-exec-ext:once:1", "fleet-actions-exec", "actions-exec-ext:once:1");
    let calls = 0;
    const deps = {
      needsDrewFile,
      applyFile,
      brokerUrl: "http://broker",
      brokerTokenFile: "unused",
      readTokenFn: async () => "tok",
      resolveKeyFn: async () => "secret",
      decideFn: async (): Promise<DecideResult> => {
        calls += 1;
        return {
          ok: true,
          decided: true,
          approved: false,
          outcome: "denied",
          via: "signed-apply",
        };
      },
    };
    await runApplierTick(deps);
    const res = await runApplierTick(deps);
    expect(calls).toBe(1);
    expect(res).toEqual({
      checked: 1,
      applied: 0,
      superseded: 0,
      rejected: 0,
      failed: 0,
      errors: [],
      alertsRaised: 0,
      alertsPending: 0,
    });
  });
});
