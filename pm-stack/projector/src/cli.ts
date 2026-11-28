// CLI: one-off jobs against the live stores.
//   bun run src/cli.ts reconcile
//   bun run src/cli.ts queue-view --tasks <tasks.json> [--window-hours 24]
//                                 [--consumer queue-view] [--no-ack]
//                                 [--format json|html]
//
// queue-view (#7078, #2072 slice 3 first piece) is the read-only
// integrated queue/PM view, machine face: it joins fleet queue task
// records (JSON exported from the queue broker — an array, or an object
// with a `tasks` array) with the slice-2 hot list consumed through a
// consumer checkpoint, and prints the view as JSON on stdout. The
// checkpoint is acked only after the view has been printed, unless
// --no-ack is passed.
//
// Strict argv parsing (#3352, #8650). The whole command line is declared
// per subcommand below and parsed single-pass before anything else runs:
// an unknown flag, a bare positional, or a value flag with no value (or
// with a flag token where the value belongs) is a usage error naming the
// offending argument, printed to stderr with exit 2 — before the DB is
// opened or migrated, before any fetch, and before any ack. The old
// indexOf/includes idiom silently ignored unknown flags and resolved a
// flag token as another flag's value.
import { readFileSync } from "node:fs";
import { openDb, migrate } from "./store";
import { buildQueueView, consumeHotList, defaultTrendLinks, type QueueTaskRecord } from "./queue-view";
import { renderQueueViewHtml } from "./queue-view-html";

const USAGE = `usage:
  cli.ts reconcile
  cli.ts queue-view --tasks <file> [--window-hours <hours>] [--consumer <name>]
                    [--no-ack] [--format json|html]`;

/** One declared subcommand: which flags take a value, which are booleans,
 * which value flags are required, and whether bare positionals are allowed
 * (none are, everywhere). */
type Spec = {
  value: readonly string[];
  boolean: readonly string[];
  required: readonly string[];
};

const COMMANDS: Record<string, Spec> = {
  reconcile: { value: [], boolean: [], required: [] },
  "queue-view": {
    value: ["--tasks", "--window-hours", "--consumer", "--format"],
    boolean: ["--no-ack"],
    required: ["--tasks"],
  },
};

export type CliArgs =
  | { cmd: "reconcile" }
  | {
      cmd: "queue-view";
      tasksPath: string;
      format: "json" | "html";
      windowHours: number;
      consumer: string;
      ack: boolean;
    };

function usageError(message: string): never {
  console.error(`projector: ${message}`);
  console.error(USAGE);
  console.error("refusing to run: no store opened, nothing fetched, nothing acked");
  process.exit(2);
}

/** Parse and validate the whole command line. Exported for the probe tests. */
export function parseCliArgv(argv: string[]): CliArgs {
  const cmd = argv[0] ?? "reconcile";
  const spec = COMMANDS[cmd];
  if (!spec) {
    usageError(`unknown command ${JSON.stringify(cmd)}`);
  }
  const values: Record<string, string> = {};
  let ack = true;
  for (let i = 1; i < argv.length; i++) {
    const token = argv[i];
    if (spec.boolean.includes(token)) {
      if (token === "--no-ack") ack = false;
      continue;
    }
    if (!spec.value.includes(token)) {
      if (token.startsWith("-")) usageError(`unknown flag ${JSON.stringify(token)} for ${cmd}`);
      usageError(`unexpected argument ${JSON.stringify(token)} (${cmd} takes only declared flags)`);
    }
    const value = argv[i + 1];
    if (typeof value !== "string" || value === "" || value.startsWith("-")) {
      usageError(
        `${token} requires a value that is not another flag (got ${value === undefined ? "nothing" : JSON.stringify(value)})`,
      );
    }
    values[token] = value;
    i++; // consume the value: no token is read twice
  }
  for (const flag of spec.required) {
    if (!(flag in values)) usageError(`${flag} <file> is required (queue task records as JSON)`);
  }
  if (cmd === "reconcile") return { cmd: "reconcile" };

  const format = values["--format"] ?? "json";
  if (format !== "json" && format !== "html") {
    usageError(`--format must be json or html, got ${format}`);
  }
  const windowHours = Number(values["--window-hours"] ?? "24");
  if (!Number.isFinite(windowHours) || windowHours <= 0) {
    usageError(`--window-hours must be a positive number, got ${values["--window-hours"]}`);
  }
  return {
    cmd: "queue-view",
    tasksPath: values["--tasks"],
    format,
    windowHours,
    consumer: values["--consumer"] ?? "queue-view",
    ack,
  };
}

// Validate argv before touching the store: a usage error must never leave a
// migrated DB or a moved consumer checkpoint behind.
const args = parseCliArgv(process.argv.slice(2));
const env = process.env as Record<string, string | undefined>;

const db = openDb(env);
await migrate(db);

if (args.cmd === "reconcile") {
  const { reconcileRepos } = await import("./reconcile");
  console.log("[projector] reconcile start:", env.PM_REPOS);
  const result = await reconcileRepos(db, env);
  console.log(JSON.stringify(result, null, 2));
} else {
  const { tasksPath, format, windowHours, consumer, ack } = args;
  const parsed = JSON.parse(readFileSync(tasksPath, "utf8")) as QueueTaskRecord[] | { tasks: QueueTaskRecord[] };
  const tasks = Array.isArray(parsed) ? parsed : parsed.tasks;
  if (!Array.isArray(tasks)) {
    console.error("queue-view: tasks file must be a JSON array or { tasks: [...] }");
    process.exit(2);
  }
  // Consume BEFORE capturing the window end: consumeHotList pins the
  // hot-list high-water, and the window end must be at or after that
  // pin. Capturing `until` first would let an event appended after
  // `until` but before the pin be consumed, filtered out of the view
  // by the window, and then acked past — lost for good. With this
  // order every consumed event has received_at <= pin <= until, and
  // anything appended after the pin is neither consumed nor acked.
  // (Codex finding on PR #8481, 2026-11-11.)
  const consumed = await consumeHotList(db, consumer);
  const until = new Date();
  const since = new Date(until.getTime() - windowHours * 3_600_000);
  const view = buildQueueView(tasks, consumed.events, {
    since: since.toISOString(),
    until: until.toISOString(),
    consumer,
    fromSeq: consumed.fromSeq,
    highWater: consumed.highWater,
    links: defaultTrendLinks(env),
  });
  // One read model, two faces: --format json is the machine face
  // (unchanged shape); --format html is the human face (#7078 second
  // piece), the same view rendered as a page/report.
  console.log(format === "html" ? renderQueueViewHtml(view) : JSON.stringify(view, null, 2));
  if (ack) {
    const cursor = await consumed.ack();
    console.error(`[projector] queue-view: consumer ${consumer} checkpoint acked at ${cursor}`);
  }
}
