// cloudflare.config.ts (the cf deploy source) and wrangler.jsonc (local
// `wrangler dev`) describe the same Worker; keep them in step.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const dir = new URL("../", import.meta.url);
const cfConfig = readFileSync(new URL("cloudflare.config.ts", dir), "utf8");
const wrangler = JSON.parse(
  readFileSync(new URL("wrangler.jsonc", dir), "utf8")
    .split("\n")
    .filter((line) => !line.trim().startsWith("//"))
    .join("\n"),
);

test("name, compatibility date and cron schedule match", () => {
  assert.match(cfConfig, new RegExp(`name: "${wrangler.name}"`));
  assert.match(cfConfig, new RegExp(`compatibilityDate: "${wrangler.compatibility_date}"`));
  const schedules = [...cfConfig.matchAll(/triggers\.scheduled\(\{ schedule: "([^"]+)" \}\)/g)].map((m) => m[1]);
  assert.deepEqual(schedules, wrangler.triggers.crons);
});

test("every plain var matches", () => {
  const texts = Object.fromEntries([...cfConfig.matchAll(/(\w+): bindings\.text\("([^"]*)"\)/g)].map((m) => [m[1], m[2]]));
  assert.deepEqual(texts, wrangler.vars);
});

test("the Durable Object keeps its sqlite namespace", () => {
  const [binding] = wrangler.durable_objects.bindings;
  assert.match(cfConfig, new RegExp(`${binding.class_name}: exports\\.durableObject\\(\\{ storage: "sqlite" \\}\\)`));
  assert.match(cfConfig, new RegExp(`${binding.name}: bindings\\.durableObject\\(\\{ worker: "${wrangler.name}", exportName: "${binding.class_name}" \\}\\)`));
  assert.ok(wrangler.migrations.some((m) => m.new_sqlite_classes?.includes(binding.class_name)));
});

test("every secret the Worker reads is declared", () => {
  const declared = [...cfConfig.matchAll(/(\w+): bindings\.secret\(\)/g)].map((m) => m[1]).sort();
  assert.deepEqual(declared, ["ACCESS_CLIENT_ID", "ACCESS_CLIENT_SECRET", "BROKER_AGENT_TOKEN", "GITHUB_TOKEN", "TELEGRAM_BOT_TOKEN"]);
  const src = ["index.js", "run.js", "notify.js", "queue.js"]
    .map((f) => readFileSync(new URL(`src/${f}`, dir), "utf8"))
    .join("\n");
  for (const name of declared) assert.match(src, new RegExp(`env\\.${name}\\b`), name);
});
