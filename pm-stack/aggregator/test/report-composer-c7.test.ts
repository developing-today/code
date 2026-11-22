import { access, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, test } from "bun:test";
import { weeklyReportPeriod, dailyReportPeriod } from "../src/report-periods.ts";
import { composeReport, generateReportFromStoredRecords, MODAL_LIVING_ITEM_FIXTURE } from "../src/report-composer.ts";
import {
  appendReportRecord,
  makeReportRecord,
  readCurrentReportRecords,
  type ReportRecord,
} from "../src/report-records.ts";
import { parseArgs } from "../scripts/generate-report.ts";

const tempDirs: string[] = [];
async function storePath(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), "report-composer-c7-"));
  tempDirs.push(dir);
  return join(dir, "reports.jsonl");
}

afterEach(async () => {
  await Promise.all(tempDirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
});

function dailyRecord(
  date: string,
  text: string,
  classification?: "implemented" | "cancelled" | "duplicate" | "in-progress",
  extra?: { text: string; classification: "cancelled" | "duplicate" | "implemented" },
) {
  const period = dailyReportPeriod(date);
  const at = new Date(Date.parse(period.periodStart) + 12 * 60 * 60 * 1000).toISOString();
  return makeReportRecord(
    {
      period,
      asOf: at,
      templateVersion: "daily-source-v1",
      sourceCursor: `source:${date}:${text}`,
      sources: [{ id: `event:${date}`, uri: `gh:developing-today/code/issues/${date}`, observedAt: at }],
      blocks: [
        {
          id: `activity:${date}`,
          text,
          relatedAt: at,
          links: [`gh:developing-today/code/issues/${date}`],
          classification,
        },
        ...(extra
          ? [
              {
                id: `activity:${date}:late`,
                text: extra.text,
                relatedAt: new Date(Date.parse(at) + 60_000).toISOString(),
                links: [`gh:developing-today/code/issues/${date}:late`],
                classification: extra.classification,
              },
            ]
          : []),
      ],
    },
    at,
  );
}

describe("C7 report composition", () => {
  test("report generation CLI requires an explicit, canonical period before doing I/O", () => {
    expect(parseArgs(["--kind", "weekly", "--period", "2026-11-16", "--as-of", "2026-11-22T12:00:00.000Z"])).toEqual({
      kind: "weekly",
      identity: "2026-11-16",
      asOf: "2026-11-22T12:00:00.000Z",
    });
    expect(() => parseArgs(["--kind", "weekly", "--as-of", "2026-11-22T12:00:00.000Z"])).toThrow(/--period/);
    expect(() =>
      parseArgs(["--kind", "weekly", "--period", "next-week", "--as-of", "2026-11-22T12:00:00.000Z"]),
    ).toThrow(/Monday/);
    expect(() =>
      parseArgs(["--kind", "daily", "--period", "2026-11-21", "--as-of", "2026-11-22T05:00:00Z", "--surprise", "x"]),
    ).toThrow(/unknown argument/);
    expect(() =>
      parseArgs([
        "--kind",
        "daily",
        "--period",
        "2026-11-21",
        "--as-of",
        "2026-11-22T05:00:00.000Z",
        "--kind",
        "weekly",
      ]),
    ).toThrow(/duplicate argument/);
  });

  test("report CLI does not silently use a file when the configured backend is Turso", async () => {
    const file = await storePath();
    const script = new URL("../scripts/generate-report.ts", import.meta.url).pathname;
    const child = Bun.spawn(
      [
        process.execPath,
        script,
        "--kind",
        "daily",
        "--period",
        "2026-11-21",
        "--as-of",
        "2026-11-21T18:00:00.000Z",
        "--store",
        file,
      ],
      { env: { ...Bun.env, PM_AGG_STORE_BACKEND: "turso" }, stdout: "pipe", stderr: "pipe" },
    );
    const [exitCode, stderr] = await Promise.all([child.exited, new Response(child.stderr).text()]);
    expect(exitCode).toBe(2);
    expect(stderr).toContain("--store is only valid with PM_AGG_STORE_BACKEND=file");
    await expect(access(file)).rejects.toThrow();
  });

  test("generates weekly rollups from stored daily records with deterministic ordering and classified links", async () => {
    const file = await storePath();
    const inputs = [
      dailyRecord("2026-11-13", "Modal re-embed began.", "in-progress", {
        text: "Late result disproved the earlier hypothesis.",
        classification: "cancelled",
      }),
      dailyRecord("2026-11-21", "Late result: implementation confirmed.", "implemented"),
      dailyRecord("2026-11-20", "An issue was cancelled as duplicate.", "duplicate"),
      dailyRecord("2026-11-16", "Weekly work started.", "implemented"),
    ];
    for (const record of inputs) await appendReportRecord(file, record);

    const daily = await generateReportFromStoredRecords(file, dailyReportPeriod("2026-11-21"), {
      asOf: "2026-11-21T18:00:00.000Z",
      generatedAt: "2026-11-21T18:01:00.000Z",
    });

    const period = weeklyReportPeriod("2026-11-16");
    const generated = await generateReportFromStoredRecords(file, period, {
      asOf: "2026-11-22T12:00:00.000Z",
      generatedAt: "2026-11-22T12:01:00.000Z",
    });
    const persisted = await readCurrentReportRecords(file);
    const weekly = persisted.find((record) => record.id === generated.record.id);

    expect(generated.inserted).toBe(true);
    expect(daily.record.period.kind).toBe("daily");
    expect(weekly?.period.id).toBe("weekly:2026-11-16:America/Chicago");
    const eventBlocks = weekly?.blocks.filter((block) => block.id.startsWith("report:")) ?? [];
    expect(eventBlocks.map((block) => block.relatedAt)).toEqual(
      [...eventBlocks.map((block) => block.relatedAt)].sort(),
    );
    expect(weekly?.blocks.some((block) => block.text.startsWith("[Prior-Friday reach-back] Modal"))).toBe(true);
    expect(
      weekly?.blocks.some((block) => block.text.startsWith("[Prior-Friday reach-back] Late result disproved")),
    ).toBe(true);
    expect(weekly?.blocks.some((block) => block.text.includes("Late result"))).toBe(true);
    expect(
      generated.record.blocks.some((block) => block.id === "activity:implemented" && block.links?.length === 2),
    ).toBe(true);
    expect(
      generated.record.blocks.some((block) => block.id === "activity:duplicate" && block.links?.length === 1),
    ).toBe(true);

    const dailyReadBack = (await readCurrentReportRecords(file)).filter(
      (record) => record.period.id === "daily:2026-11-21:America/Chicago",
    );
    expect(dailyReadBack.some((record) => record.id === daily.record.id)).toBe(true);

    const rerun = await generateReportFromStoredRecords(file, period, {
      asOf: "2026-11-22T12:00:00.000Z",
      generatedAt: "2026-11-22T12:05:00.000Z",
    });
    expect(rerun.inserted).toBe(false);
    expect(rerun.record.id).toBe(generated.record.id);
  });

  test("keeps generation dependent on explicit period identity and uses the amendment's Modal example", () => {
    const daily = dailyRecord("2026-11-21", MODAL_LIVING_ITEM_FIXTURE[0]);
    const monthlyPeriod = dailyReportPeriod("2026-11-21");
    const result = composeReport(monthlyPeriod, [daily as ReportRecord], { asOf: "2026-11-22T05:00:00.000Z" });
    expect(result.blocks.some((block) => block.text.includes("2-day re-embed project using Modal"))).toBe(true);
    expect(MODAL_LIVING_ITEM_FIXTURE[1]).toContain("Purchased $10");
  });

  test("includes prior Friday through calendar boundaries across a DST transition", () => {
    const friday = dailyRecord("2026-03-06", "Friday before the spring DST transition.");
    const result = composeReport(weeklyReportPeriod("2026-03-09"), [friday], { asOf: "2026-03-10T12:00:00.000Z" });
    expect(result.blocks.some((block) => block.text.startsWith("[Prior-Friday reach-back] Friday before"))).toBe(true);
  });
});
