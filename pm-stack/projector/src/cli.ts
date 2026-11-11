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
import { readFileSync } from "node:fs";
import { openDb, migrate } from "./store";
import { buildQueueView, consumeHotList, defaultTrendLinks, type QueueTaskRecord } from "./queue-view";
import { renderQueueViewHtml } from "./queue-view-html";

const cmd = process.argv[2] ?? "reconcile";
const env = process.env as Record<string, string | undefined>;

function arg(name: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

const db = openDb(env);
await migrate(db);

if (cmd === "reconcile") {
  const { reconcileRepos } = await import("./reconcile");
  console.log("[projector] reconcile start:", env.PM_REPOS);
  const result = await reconcileRepos(db, env);
  console.log(JSON.stringify(result, null, 2));
} else if (cmd === "queue-view") {
  const tasksPath = arg("tasks");
  if (!tasksPath) {
    console.error("queue-view: --tasks <file> is required (queue task records as JSON)");
    process.exit(2);
  }
  const parsed = JSON.parse(readFileSync(tasksPath, "utf8")) as
    | QueueTaskRecord[]
    | { tasks: QueueTaskRecord[] };
  const tasks = Array.isArray(parsed) ? parsed : parsed.tasks;
  if (!Array.isArray(tasks)) {
    console.error("queue-view: tasks file must be a JSON array or { tasks: [...] }");
    process.exit(2);
  }
  const format = arg("format") ?? "json";
  if (format !== "json" && format !== "html") {
    console.error(`queue-view: --format must be json or html, got ${format}`);
    process.exit(2);
  }
  const windowHours = Number(arg("window-hours") ?? "24");
  const consumer = arg("consumer") ?? "queue-view";
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
  if (!process.argv.includes("--no-ack")) {
    const cursor = await consumed.ack();
    console.error(`[projector] queue-view: consumer ${consumer} checkpoint acked at ${cursor}`);
  }
} else {
  console.error(`unknown command: ${cmd}`);
  process.exit(2);
}
