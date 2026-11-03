#!/usr/bin/env bun
// Record a Muse launch in the identity registry (#906 §3).
//
// Muse sessions run inside the assistant runtime, which exposes
// no session list for the collector to scrape — so a Muse session
// registers itself (or its launcher does) by appending ONE JSON line
// to the launch-record file the collector reads. The record carries
// the session's stable id + display name, its parent (the spawning
// session, when known), and the task refs it was launched for; that is
// what makes the dashboard's task ↔ session linkage work for Muse
// sessions exactly as it does for OpenCode sessions.
//
// Usage:
//   bun run scripts/record-claude-launch.ts --label "Slice 3 builder" \
//     [--id claude-<unique>] [--parent <sessionId>] [--task owner/repo#123] \
//     [--model <id>] [--status launched] [--file <path.jsonl>]
//
// Default file: ~/workspace/goals/keep-the-four-repos-healthy/
// hidden_files/claude-launch-records.jsonl (the collector's default;
// override with CLAUDE_LAUNCH_RECORDS_PATH on the collector side, or
// --file here).
import { appendFile, mkdir } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

function arg(name: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

const file =
  arg("file") ??
  process.env.CLAUDE_LAUNCH_RECORDS_PATH ??
  join(homedir(), "workspace/goals/keep-the-four-repos-healthy/hidden_files/claude-launch-records.jsonl");

const label = arg("label");
if (!label) {
  console.error("record-claude-launch: --label is required (the session's human display name)");
  process.exit(2);
}

const now = new Date();
const id = arg("id") ?? `claude-${now.getTime().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
const taskRefs = process.argv
  .map((a, i) => (a === "--task" ? process.argv[i + 1] : undefined))
  .filter((v): v is string => typeof v === "string");

const record = {
  id,
  label,
  parentId: arg("parent") ?? null,
  startedAt: now.toISOString(),
  lastSeenAt: now.toISOString(),
  status: arg("status") ?? "launched",
  ...(arg("model") ? { model: arg("model") } : {}),
  ...(taskRefs.length ? { taskRefs } : {}),
};

await mkdir(dirname(file), { recursive: true });
await appendFile(file, `${JSON.stringify(record)}\n`);
console.log(`record-claude-launch: recorded ${id} ("${label}") in ${file}`);
