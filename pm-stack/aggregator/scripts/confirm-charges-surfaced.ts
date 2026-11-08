#!/usr/bin/env bun
// Confirm only the charges in the exact feed successfully installed remotely.
// The push script holds the state-specific flock across collection and confirmation.
import { readFile, rename, unlink, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { unmatchedKey } from "../src/charges-collect.ts";
import type { UnmatchedCharge } from "../src/charges-match.ts";
import { isFiniteNumber, isRecord } from "../src/validate.ts";

const feedPath = process.argv[2];
if (!feedPath) throw new Error("usage: confirm-charges-surfaced.ts <delivered-feed>");
const statePath =
  process.env.CHARGES_SURFACED_STATE_PATH ??
  join(homedir(), "workspace/goals/keep-the-four-repos-healthy/hidden_files/charges-surfaced-state.json");
const feed: unknown = JSON.parse(await readFile(feedPath, "utf8"));
// The feed arrives from disk after delivery, so it is untrusted input:
// a non-object top level (null, array, string, number) is an invalid
// feed, not something to dereference blindly or silently accept
// (#3681).
if (!isRecord(feed)) throw new Error("invalid feed: expected a JSON object");
if (feed.chargesAttention !== undefined && !Array.isArray(feed.chargesAttention))
  throw new Error("invalid chargesAttention");
// An element is confirmable only when a surfaced key can actually be
// built from it: unmatchedKey needs a string normalized, a finite
// numeric amountUsd, and a string at. Anything else is dropped loudly
// instead of crashing confirmation (which would re-alert every
// already-delivered charge forever) or persisting a bogus
// "undefined|…" key (#3681).
type ConfirmableCharge = Pick<UnmatchedCharge, "normalized" | "amountUsd" | "at">;
function isConfirmableCharge(v: unknown): v is ConfirmableCharge {
  if (!isRecord(v)) return false;
  return typeof v.normalized === "string" && isFiniteNumber(v.amountUsd) && typeof v.at === "string";
}
const attention: ConfirmableCharge[] = [];
let dropped = 0;
for (const element of feed.chargesAttention ?? []) {
  if (isConfirmableCharge(element)) attention.push(element);
  else dropped += 1;
}
if (dropped > 0) console.error(`confirm-charges-surfaced: dropped ${dropped} malformed chargesAttention element(s)`);
if (attention.length) {
  let state: { surfaced?: unknown } = {};
  try {
    state = JSON.parse(await readFile(statePath, "utf8"));
    if (!state || !Array.isArray(state.surfaced) || !state.surfaced.every((key) => typeof key === "string"))
      throw new Error("invalid surfaced state");
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== "ENOENT") throw err;
  }
  const keys = new Set<string>((state.surfaced as string[] | undefined) ?? []);
  for (const charge of attention) keys.add(unmatchedKey(charge));
  const temp = `${statePath}.${process.pid}.${crypto.randomUUID()}.tmp`;
  try {
    await writeFile(
      temp,
      JSON.stringify({ version: 1, surfaced: [...keys], updatedAt: new Date().toISOString() }, null, 2),
      { flag: "wx" },
    );
    await rename(temp, statePath);
  } finally {
    await unlink(temp).catch(() => {});
  }
}
