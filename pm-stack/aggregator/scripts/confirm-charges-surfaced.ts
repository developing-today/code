#!/usr/bin/env bun
// Confirm only the charges in the exact feed successfully installed remotely.
// The push script holds the state-specific flock across collection and confirmation.
import { readFile, rename, unlink, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { unmatchedKey } from "../src/charges-collect.ts";

const feedPath = process.argv[2];
if (!feedPath) throw new Error("usage: confirm-charges-surfaced.ts <delivered-feed>");
const statePath = process.env.CHARGES_SURFACED_STATE_PATH ?? join(homedir(), "workspace/goals/keep-the-four-repos-healthy/hidden_files/charges-surfaced-state.json");
const feed = JSON.parse(await readFile(feedPath, "utf8"));
if (feed.chargesAttention !== undefined && !Array.isArray(feed.chargesAttention)) throw new Error("invalid chargesAttention");
const attention = feed.chargesAttention ?? [];
if (attention.length) {
  let state: { surfaced?: unknown } = {};
  try {
    state = JSON.parse(await readFile(statePath, "utf8"));
    if (!state || !Array.isArray(state.surfaced) || !state.surfaced.every((key) => typeof key === "string")) throw new Error("invalid surfaced state");
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== "ENOENT") throw err;
  }
  const keys = new Set<string>((state.surfaced as string[] | undefined) ?? []);
  for (const charge of attention) keys.add(unmatchedKey(charge));
  const temp = `${statePath}.${process.pid}.${crypto.randomUUID()}.tmp`;
  try {
    await writeFile(temp, JSON.stringify({ version: 1, surfaced: [...keys], updatedAt: new Date().toISOString() }, null, 2), { flag: "wx" });
    await rename(temp, statePath);
  } finally {
    await unlink(temp).catch(() => {});
  }
}
