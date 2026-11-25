// Store migrations from the app (#14256).
//
// Oracle: GitHub's own shapes, written out by hand — the stores list
// as the repository holds it, and the migration workflow's runs as
// the runs API lists them (newest first, the run's name carrying its
// store). What the app starts is compared as the exact dispatch the
// deploy pattern makes; a failed run must come back as a failure.
import { describe, expect, test } from "bun:test";
import { actions } from "../src/actions.ts";
import { type Config, readConfig } from "../src/config.ts";
import { HOLDABLE } from "../src/edge-proto.ts";
import { lastMigrationRuns, runStore } from "../src/migrations.ts";
import { Ops, type Run } from "../src/ops.ts";
import { migrationSaid } from "../public/app.js";

const CFG: Config = readConfig({
  HOME: "/h",
  PM_CONSOLE_OWNER_EMAILS: "owner@example.com",
  PM_CONSOLE_ANDROID_SHA256: Array(32).fill("AB").join(":"),
});

const STORES = {
  checked: "2026-11-24",
  stores: [
    {
      name: "fleet-queue",
      title: "Fleet queue",
      does: "The one fleet queue.",
      used_by: ["pm-stack/console (queue reads)"],
      address: "https://fleet-queue-dezren39.aws-us-east-1.turso.io",
      token: "turso.fleet-queue-rw-token",
      tables: [],
      schema_version: null,
    },
    {
      name: "fleet-platform",
      title: "Fleet platform",
      does: "The PM plane.",
      used_by: ["pm-stack/aggregator (writes)"],
      address: "https://fleet-platform-dezren39.aws-us-east-1.turso.io",
      token: "turso.fleet-platform-rw-token",
      tables: ["incidents"],
      schema_version: null,
    },
  ],
};

function recorder(answers: (argv: string[]) => string = () => "") {
  const calls: { argv: string[]; stdin?: string }[] = [];
  const run: Run = async (argv, opts) => {
    calls.push({ argv, stdin: opts?.stdin });
    return { code: 0, stdout: answers(argv), stderr: "" };
  };
  return { run, calls };
}

/** The recorder's GitHub: the stores list, and the migration runs. */
function github(runs: unknown) {
  return recorder((argv) => {
    const path = argv.find((a) => a.startsWith("repos/")) ?? "";
    if (path.includes("/contents/")) return JSON.stringify({ content: btoa(JSON.stringify(STORES)) });
    if (path.includes("pm-store-migrate.yml/runs")) return JSON.stringify({ workflow_runs: runs });
    return "";
  });
}

const RUNS = [
  {
    display_title: "Migrate fleet-queue",
    head_sha: "ca5c04493289cde76fa0d7f287bbd46e0395c491",
    status: "completed",
    conclusion: "failure",
    created_at: "2026-11-25T05:51:40Z",
    html_url: "https://github.com/developing-today/code/actions/runs/99",
  },
  {
    display_title: "Migrate fleet-queue",
    head_sha: "326ac803beddb586530c6e48b76ce1db630e8aca",
    status: "completed",
    conclusion: "success",
    created_at: "2026-11-24T05:42:59Z",
    html_url: "https://github.com/developing-today/code/actions/runs/98",
  },
  {
    display_title: "Migrate test",
    head_sha: "111ac803beddb586530c6e48b76ce1db630e8aca",
    status: "in_progress",
    conclusion: null,
    created_at: "2026-11-25T06:00:00Z",
    html_url: "https://github.com/developing-today/code/actions/runs/100",
  },
];

describe("store migrations", () => {
  test("a run's store is read from its name, and nothing else is", () => {
    expect(runStore("Migrate fleet-queue")).toBe("fleet-queue");
    expect(runStore("Migrate test")).toBe("test");
    expect(runStore("pm-store migrate")).toBeNull();
    expect(runStore("Migrate ")).toBeNull();
    expect(runStore(42)).toBeNull();
  });

  test("the last run per store: newest wins, and a failure stays a failure", () => {
    const got = lastMigrationRuns(RUNS, [
      { name: "test", title: "Test store (stand-in)" },
      { name: "fleet-queue", title: "Fleet queue" },
      { name: "fleet-platform", title: "Fleet platform" },
    ]);
    expect(got).toEqual([
      {
        name: "test",
        title: "Test store (stand-in)",
        run: { state: "in_progress", commit: "111ac803bedd", at: "2026-11-25T06:00:00Z", url: "https://github.com/developing-today/code/actions/runs/100" },
      },
      {
        name: "fleet-queue",
        title: "Fleet queue",
        // The newer failure, not the older success behind it.
        run: { state: "failure", commit: "ca5c04493289", at: "2026-11-25T05:51:40Z", url: "https://github.com/developing-today/code/actions/runs/99" },
      },
      { name: "fleet-platform", title: "Fleet platform", run: null },
    ]);
  });

  test("the screen's line: a failed run reads as failed", () => {
    const now = Date.parse("2026-11-25T06:51:40Z");
    const failed = migrationSaid(
      { run: { state: "failure", commit: "ca5c04493289", at: "2026-11-25T05:51:40Z", url: "https://github.com/developing-today/code/actions/runs/99" } },
      now,
    );
    expect(failed.tone).toBe("warn");
    expect(failed.text).toContain("failed");
    expect(migrationSaid({ run: null }, now).text).toContain("No migration");
  });

  test("storeMigrations reads the list of record and the workflow's runs", async () => {
    const r = github(RUNS);
    const got = await new Ops(CFG, r.run).storeMigrations();
    expect(got.workflow).toBe("pm-store-migrate.yml");
    expect(got.stores.map((s) => s.name)).toEqual(["test", "fleet-queue", "fleet-platform"]);
    expect(got.stores[1].run?.state).toBe("failure");
    expect(got.stores[1].run?.url).toContain("/actions/runs/99");
  });

  test("store.migrate dispatches the migration workflow for a named store, from main", async () => {
    const r = github([]);
    const table = actions(new Ops(CFG, r.run), {} as never);
    expect(await table["store.migrate"].run({ store: "fleet-queue" })).toEqual({ started: "fleet-queue" });
    expect(await table["store.migrate"].run({ store: "test" })).toEqual({ started: "test" });
    const dispatches = r.calls.filter((c) => c.argv.some((a) => a.includes("/dispatches")));
    expect(dispatches.map((c) => [c.argv.slice(1), JSON.parse(c.stdin ?? "null")])).toEqual([
      [
        ["gh", "api", "-X", "POST", "repos/developing-today/code/actions/workflows/pm-store-migrate.yml/dispatches", "--input", "-"],
        { ref: "main", inputs: { store: "fleet-queue" } },
      ],
      [
        ["gh", "api", "-X", "POST", "repos/developing-today/code/actions/workflows/pm-store-migrate.yml/dispatches", "--input", "-"],
        { ref: "main", inputs: { store: "test" } },
      ],
    ]);
    // A store the list does not name is refused before any dispatch.
    for (const store of ["pm-dispatch.yml", "../x", "", null, "fleet-queue "]) {
      await expect(table["store.migrate"].run({ store })).rejects.toThrow(/store:/);
    }
    expect(r.calls.filter((c) => c.argv.some((a) => a.includes("/dispatches"))).length).toBe(2);
    expect(table["store.migrate"].audit({ store: "fleet-queue", extra: "x" })).toEqual({ store: "fleet-queue" });
    expect(HOLDABLE.has("store.migrate")).toBe(false);
  });
});
