// install.mjs — install/refresh the #3026 opencode claimant as an
// OpenChamber scheduled task in the laptop's code-project store.
//
// The store (~/.config/openchamber/projects/<base64(abs project path)>
// .json) is runtime state, not source: this script is the versioned
// source of truth for the task definition, and installing is an
// idempotent merge by task id that preserves every other task in the
// store. The task installs DISABLED (shadow mode is also the prompt's
// own default); enabling is the parallel-run gate's decision, not
// the installer's — pass --enable only under that gate.
//
// Usage:
//   bun install.mjs [--store <path>] [--dry-run] [--enable]
//   node install.mjs ...           (same; no dependencies)
import fs from "node:fs";
import { readFile, writeFile, rename, copyFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const TASK_ID = "f1b4a33b-14d5-4a69-9ad6-d221e42e3ac9";
export const TASK_NAME = "Opencode lane claimant/executor - #3026";
export const CRON = "*/5 * * * *";
export const TIMEZONE = "America/Chicago";
export const PROVIDER_ID = "opencode-route";
export const MODEL_ID = "workhorse";
export const AGENT = "build";

export const DEFAULT_STORE = join(
  homedir(),
  ".config", "openchamber", "projects",
  "path_L2hvbWUvdXNlci9naXQvY29kZQ.json", // /home/user/git/code
);

export class UsageError extends Error {
  constructor(message) {
    super(`${message}\nUsage: bun install.mjs [--store <path>] [--dry-run] [--enable]`);
    this.name = "UsageError";
  }
}

export function parseArgs(argv) {
  const parsed = { storePath: DEFAULT_STORE, dryRun: false, enable: false };
  let storeSeen = false;

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--dry-run") {
      parsed.dryRun = true;
    } else if (arg === "--enable") {
      parsed.enable = true;
    } else if (arg === "--store") {
      if (storeSeen) throw new UsageError("--store may be specified only once");
      const value = argv[i + 1];
      if (value === undefined || value === "" || value.startsWith("--")) {
        throw new UsageError("--store requires a path value");
      }
      storeSeen = true;
      parsed.storePath = value;
      i += 1;
    } else if (arg.startsWith("--")) {
      throw new UsageError(`unrecognized argument: ${arg}`);
    } else {
      throw new UsageError(`unexpected positional argument: ${arg}`);
    }
  }

  return parsed;
}

export function buildTask(prompt, { enabled = false } = {}) {
  if (typeof prompt !== "string" || !prompt.includes("#3026")) {
    throw new Error("prompt.md does not look like the #3026 claimant prompt");
  }
  return {
    id: TASK_ID,
    name: TASK_NAME,
    enabled,
    schedule: { kind: "cron", cron: CRON, timezone: TIMEZONE },
    execution: {
      prompt,
      providerID: PROVIDER_ID,
      modelID: MODEL_ID,
      agent: AGENT,
    },
  };
}

// Pure merge: replace the task with the same id, else append. Every
// other task is preserved object-for-object; merging twice is a no-op
// the second time (deep-equal result).
export function mergeTasks(store, task) {
  if (!store || !Array.isArray(store.scheduledTasks)) {
    throw new Error("store has no scheduledTasks array — refusing to write");
  }
  const rest = store.scheduledTasks.filter((t) => t.id !== task.id);
  return { ...store, scheduledTasks: [...rest, task] };
}

async function main(argv) {
  const { storePath, dryRun, enable } = parseArgs(argv);
  const here = dirname(fileURLToPath(import.meta.url));
  const prompt = await readFile(join(here, "prompt.md"), "utf8");
  const raw = await readFile(storePath, "utf8");
  const store = JSON.parse(raw);
  const existing = (store.scheduledTasks || []).find((t) => t.id === TASK_ID);
  const task = buildTask(prompt, {
    enabled: enable ? true : (existing?.enabled ?? false),
  });
  const merged = mergeTasks(store, task);
  const summary = {
    store: storePath,
    task_id: TASK_ID,
    action: existing ? "updated" : "inserted",
    enabled: task.enabled,
    tasks_before: store.scheduledTasks.length,
    tasks_after: merged.scheduledTasks.length,
  };
  if (dryRun) {
    console.log(JSON.stringify({ ...summary, dry_run: true, task }, null, 1));
    return 0;
  }
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  await copyFile(storePath, `${storePath}.bak-${stamp}`);
  const tmp = `${storePath}.tmp-${process.pid}`;
  await writeFile(tmp, JSON.stringify(merged, null, 2) + "\n");
  await rename(tmp, storePath);
  console.log(JSON.stringify(summary, null, 1));
  return 0;
}

const isMain = (() => {
  try {
    if (!process.argv[1]) return false;
    return fs.realpathSync(process.argv[1]) === fs.realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
})();
if (isMain) {
  main(process.argv.slice(2)).then(
    (code) => process.exit(code),
    (err) => {
      console.error(`install: ${err.message}`);
      process.exit(err instanceof UsageError ? 2 : 1);
    },
  );
}
