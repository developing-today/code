import { describe, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import {
  mkdir, mkdtemp, readFile, readdir, rm, writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  AGENT, CRON, MODEL_ID, PROVIDER_ID, TASK_ID,
  buildTask, mergeTasks, parseArgs,
} from "../install.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const prompt = await readFile(join(here, "..", "prompt.md"), "utf8");
const recoveryMigration = await readFile(
  join(here, "..", "migration-0022.sql"), "utf8",
);
const installer = join(here, "..", "install.mjs");

async function makeStores() {
  const home = await mkdtemp(join(tmpdir(), "opencode-claimant-install-"));
  const defaultDir = join(
    home, ".config", "openchamber", "projects",
  );
  await mkdir(defaultDir, { recursive: true });
  const defaultStore = join(
    defaultDir, "path_L2hvbWUvdXNlci9naXQvY29kZQ.json",
  );
  const explicitStore = join(home, "explicit-store.json");
  const original = JSON.stringify({
    version: 1,
    scheduledTasks: [
      { id: TASK_ID, name: "existing", enabled: false },
      { id: "preserve", enabled: true },
    ],
  }, null, 2) + "\n";
  await writeFile(defaultStore, original);
  await writeFile(explicitStore, original);
  return { home, defaultDir, defaultStore, explicitStore, original };
}

function runInstaller(home, args) {
  return spawnSync(process.execPath, [installer, ...args], {
    encoding: "utf8",
    env: {
      ...process.env,
      HOME: home,
      BUN_INSTALL: join(tmpdir(), "opencode-claimant-bun-install"),
    },
  });
}

async function expectNoWrites(stores) {
  expect(await readFile(stores.defaultStore, "utf8")).toBe(stores.original);
  expect(await readFile(stores.explicitStore, "utf8")).toBe(stores.original);
  expect(await readdir(stores.home)).toEqual([
    ".config", "explicit-store.json",
  ]);
  expect(await readdir(stores.defaultDir)).toEqual([
    "path_L2hvbWUvdXNlci9naXQvY29kZQ.json",
  ]);
}

async function rejectsBeforeWrites(args, offendingArg) {
  const stores = await makeStores();
  try {
    const result = runInstaller(stores.home, args(stores));
    expect(result.error).toBeUndefined();
    expect(result.status).toBe(2);
    expect(result.stderr).toContain(offendingArg);
    expect(result.stderr).toContain("Usage:");
    await expectNoWrites(stores);
  } finally {
    await rm(stores.home, { recursive: true, force: true });
  }
}

describe("buildTask", () => {
  test("builds a disabled-by-default OpenChamber scheduled task", () => {
    const task = buildTask(prompt);
    expect(task.id).toBe(TASK_ID);
    expect(task.enabled).toBe(false);
    expect(task.schedule).toEqual({
      kind: "cron", cron: CRON, timezone: "America/Chicago",
    });
    expect(task.execution.providerID).toBe(PROVIDER_ID);
    expect(task.execution.modelID).toBe(MODEL_ID);
    expect(task.execution.agent).toBe(AGENT);
    expect(task.execution.prompt).toBe(prompt);
  });

  test("embeds the cap, routing, and shadow rules from the prompt", () => {
    for (const needle of ["12", "workhorse", "shadow",
                          "opencode-claimant", "poll.py"]) {
      expect(prompt).toContain(needle);
    }
  });

  test("recovery clears both bindings atomically before relaunch", () => {
    const deletion = prompt.indexOf("DELETE /api/session/<id>");
    const recovery = prompt.indexOf(
      "opencode-claimant-recovery:zero-message-deleted",
    );
    expect(deletion).toBeGreaterThanOrEqual(0);
    expect(recovery).toBeGreaterThan(deletion);
    expect(prompt).toContain("do not fall back to a progress-only checkpoint");
    expect(recoveryMigration).toContain(
      "t.progress ->> 'session_id' is distinct from p_session_id",
    );
    expect(recoveryMigration).toContain(
      "row_out.session_id is distinct from p_session_id",
    );
    expect(recoveryMigration).toContain(
      "recovery attempts exhausted for task %",
    );
    expect(recoveryMigration).toContain(
      "set session_id = null, updated_at = now()",
    );
    expect(recoveryMigration).toContain(
      "set attempts = coalesce(attempts, 0) + 1",
    );
    expect(recoveryMigration).toContain("'stage', 'launch_failed'");
    expect(recoveryMigration).toContain(
      "'launch_failed_session_id', p_session_id",
    );
  });

  test("rejects a foreign prompt", () => {
    expect(() => buildTask("unrelated prompt")).toThrow();
  });
});

describe("mergeTasks", () => {
  const other = { id: "other-task", name: "keep me", enabled: true };

  test("inserts without disturbing other tasks", () => {
    const store = { version: 1, scheduledTasks: [other] };
    const merged = mergeTasks(store, buildTask(prompt));
    expect(merged.scheduledTasks).toHaveLength(2);
    expect(merged.scheduledTasks[0]).toEqual(other);
    expect(merged.scheduledTasks[1].id).toBe(TASK_ID);
  });

  test("is idempotent: merging twice deep-equals merging once", () => {
    const store = { version: 1, scheduledTasks: [other] };
    const task = buildTask(prompt);
    const once = mergeTasks(store, task);
    const twice = mergeTasks(once, task);
    expect(twice).toEqual(once);
  });

  test("updates in place instead of duplicating", () => {
    const store = { version: 1, scheduledTasks: [buildTask("old #3026 x")] };
    const merged = mergeTasks(store, buildTask(prompt));
    expect(merged.scheduledTasks).toHaveLength(1);
    expect(merged.scheduledTasks[0].execution.prompt).toBe(prompt);
  });

  test("refuses a store without a scheduledTasks array", () => {
    expect(() => mergeTasks({}, buildTask(prompt))).toThrow();
    expect(() => mergeTasks(null, buildTask(prompt))).toThrow();
  });
});

describe("parseArgs", () => {
  test("keeps documented options in either order", () => {
    expect(parseArgs(["--dry-run", "--store", "/tmp/store", "--enable"])).toEqual({
      storePath: "/tmp/store",
      dryRun: true,
      enable: true,
    });
    expect(parseArgs(["--store", "/tmp/store", "--enable", "--dry-run"])).toEqual({
      storePath: "/tmp/store",
      dryRun: true,
      enable: true,
    });
  });

  test("rejects a repeated store option", () => {
    expect(() => parseArgs(["--store", "/tmp/a", "--store", "/tmp/b"]))
      .toThrow("--store may be specified only once");
  });
});

describe("installer CLI argument validation", () => {
  test("rejects typoed and unknown options before reading or writing stores", async () => {
    await rejectsBeforeWrites(
      ({ explicitStore }) => ["--dry-rnu", "--enable", "--store", explicitStore],
      "--dry-rnu",
    );
    await rejectsBeforeWrites(
      ({ explicitStore }) => ["--bogus-flag", "--enable", "--store", explicitStore],
      "--bogus-flag",
    );
    await rejectsBeforeWrites(
      ({ explicitStore }) => ["--stor", explicitStore, "--enable"],
      "--stor",
    );
  });

  test("rejects positionals and missing --store values before I/O", async () => {
    await rejectsBeforeWrites(
      ({ explicitStore }) => ["unexpected", "--store", explicitStore],
      "unexpected",
    );
    await rejectsBeforeWrites(() => ["--store"], "--store");
    await rejectsBeforeWrites(() => ["--store", ""], "--store");
    await rejectsBeforeWrites(() => ["--store", "--dry-run"], "--store");
  });

  test("valid --dry-run with an explicit store stays read-only", async () => {
    const stores = await makeStores();
    try {
      const result = runInstaller(stores.home, [
        "--store", stores.explicitStore, "--dry-run", "--enable",
      ]);
      expect(result.error).toBeUndefined();
      expect(result.status).toBe(0);
      expect(result.stdout).toContain('"dry_run": true');
      expect(result.stdout).toContain('"enabled": true');
      await expectNoWrites(stores);
    } finally {
      await rm(stores.home, { recursive: true, force: true });
    }
  });
});
