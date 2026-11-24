import { afterEach, describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { needsDrewAnswerHandle, raiseNeedsDrew, recordAnswer } from "../src/needs-drew.ts";

const tempDirs: string[] = [];

afterEach(async () => {
  await Promise.all(tempDirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
});

async function storeFile(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), "needs-drew-writeback-"));
  tempDirs.push(dir);
  return join(dir, "needs-drew.json");
}

const question = (ledgerId: string) => ({
  ledgerId,
  question: "Approve the extension?",
  options: [{ id: "approve", label: "Approve" }],
  links: [],
});

describe("needs-drew answer-to-handle mapping (#14043 S2)", () => {
  test("an answered Actions-exec question resolves its stored broker handle", async () => {
    // Oracle: the raise must persist source/key/handle, and the answer resolver
    // returns that handle only after first-answer-wins records an answer.
    const file = await storeFile();
    const ledgerId = "actions-exec-ext:session:42";
    const raised = await raiseNeedsDrew(file, question(ledgerId), "fleet-actions-exec", ledgerId);
    expect(raised.brokerMapping).toEqual({
      source: "fleet-actions-exec",
      key: ledgerId,
      handle: "session:42",
    });

    const answered = await recordAnswer(file, ledgerId, { optionId: "approve", via: "test" });
    expect(answered).not.toBeNull();
    expect(needsDrewAnswerHandle(answered!)).toBe("session:42");

    const stored = JSON.parse(await readFile(file, "utf8"));
    expect(stored.items[0].brokerMapping).toEqual(raised.brokerMapping);
  });

  test("an answered non-broker question has no handle", async () => {
    // Oracle: non-broker sources never receive a broker mapping, even if a
    // key-shaped value is supplied, so their recorded answers resolve to none.
    const file = await storeFile();
    const ledgerId = "ordinary-question-7";
    const raised = await raiseNeedsDrew(file, question(ledgerId), "fleet-queue", "actions-exec-ext:session:7");
    const answered = await recordAnswer(file, ledgerId, { optionId: "approve", via: "test" });

    expect(raised.brokerMapping).toBeUndefined();
    expect(answered).not.toBeNull();
    expect(needsDrewAnswerHandle(answered!)).toBeNull();
  });
});
