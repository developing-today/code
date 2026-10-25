#!/usr/bin/env bun
// Fleet feed collector (#890): runs on the sandbox host, where the
// quota sources of record live, and emits fleet-feed.json — the file
// the laptop aggregator renders for its quota (#851) and sandbox
// attention (#852) facets.
//
// Sources (all read-only, all fail-soft):
//   - the stewardship quota view (dated per-pool reads) — extracted by
//     src/quota-extract.ts; a pool whose read pattern stops matching
//     renders with identity + a "no current read" note.
//   - the sandbox OpenCode server API — sandbox session feed entries
//     (shared mapper, src/sources/opencode.ts) + live per-pool burn
//     attribution for the quota view.
//   - jules-review-state.json — keeper supervision entries annotate
//     matching sessions as evidence.
//
// Usage: bun run scripts/collect-fleet-feed.ts [--out <path>]
// Env overrides: QUOTA_VIEW_PATH, JULES_STATE_PATH, SANDBOX_OPENCODE_URL,
// OPENCODE_SERVER_PASSWORD_FILE.
import { readFile, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import type { FleetFeed } from "../src/feed.ts";
import type { QuotaPool, SessionFeedEntry } from "../src/model.ts";
import { extractQuotaPools } from "../src/quota-extract.ts";
import { computeChoices } from "../src/quota-guidance.ts";
import { mapServerSessions, poolBurn } from "../src/sources/opencode.ts";

const PKG = join(dirname(fileURLToPath(import.meta.url)), "..");
const HOME = homedir();
const HIDDEN = join(HOME, "workspace/goals/keep-the-four-repos-healthy/hidden_files");

const outArg = process.argv.indexOf("--out");
const outPath =
  outArg >= 0 ? process.argv[outArg + 1] : (process.env.FLEET_FEED_OUT ?? join(PKG, "fleet-feed.json"));
const quotaViewPath =
  process.env.QUOTA_VIEW_PATH ??
  join(
    HOME,
    "workspace/goals/stewardship-commission-recurring-review-and-dispatch/hidden_files/visibility/quota-view.md",
  );
const julesStatePath = process.env.JULES_STATE_PATH ?? join(HIDDEN, "jules-review-state.json");
const serverUrl = process.env.SANDBOX_OPENCODE_URL ?? "http://127.0.0.1:4096";
const passwordFile =
  process.env.OPENCODE_SERVER_PASSWORD_FILE ?? join(HOME, "tools/secrets.d/opencode-server-password");

/** Provider id (OpenCode config) -> quota pool id, for burn attribution. */
function poolFor(provider: string, model: string): string | null {
  switch (provider) {
    case "meta":
      return "meta:high-usage";
    case "claude-code":
      return "anthropic:claude-pro";
    case "openai":
    case "openai-codex":
    case "chatgpt":
      return "openai:codex";
    case "cline-pass":
      return "cline-pass:cline-pass";
    case "opendesign":
      return "opendesign:go";
    case "deepseek":
      return "deepseek:api";
    case "antigravity-ls":
    case "antigravity-agy":
    case "google":
      return model.startsWith("gemini") ? "antigravity:gemini" : "antigravity:claude-gpt";
    default:
      // opencode-go is deliberately unattributed in slice 1: which
      // workspace (Personal vs Lab) a sandbox session burns is not
      // visible from the session list alone.
      return null;
  }
}

/** sessionId -> "keeper <action> <at>" from jules-review-state.json. */
async function supervisionNotes(): Promise<Map<string, string>> {
  const notes = new Map<string, string>();
  try {
    const state = JSON.parse(await readFile(julesStatePath, "utf8")) as {
      opencode_supervision?: { at?: string; action?: string; error?: string; outcome?: string }[];
    };
    for (const entry of state.opencode_supervision ?? []) {
      const text = `${entry.error ?? ""}\n${entry.outcome ?? ""}`;
      for (const m of text.matchAll(/ses_[A-Za-z0-9]+/g)) {
        notes.set(m[0], `keeper ${entry.action ?? "assessed"} ${entry.at ?? ""}`.trim());
      }
    }
  } catch {
    /* annotations are best-effort */
  }
  return notes;
}

async function main(): Promise<void> {
  const now = new Date();

  // --- quota -------------------------------------------------------------
  let pools: QuotaPool[] = [];
  try {
    const quotaView = await readFile(quotaViewPath, "utf8");
    pools = extractQuotaPools(quotaView, now);
  } catch (err) {
    console.error(`collector: quota view unreadable (${err}); emitting no pools`);
  }

  // --- sandbox attention + burn -------------------------------------------
  let attention: SessionFeedEntry[] = [];
  try {
    const password = (await readFile(passwordFile, "utf8")).trim();
    const mapped = await mapServerSessions({ baseUrl: serverUrl, password }, now.getTime());
    if (mapped) {
      attention = mapped.entries;
      const notes = await supervisionNotes();
      for (const e of attention) {
        const note = notes.get(e.sessionId);
        if (note) e.supervisionNote = note;
      }
      // Burn attribution: sessions the server holds active right now.
      const simple = mapped.entries.map((e) => ({
        id: e.sessionId,
        provider: e.provider,
        model: e.model,
      }));
      const burn = poolBurn(simple, mapped.tokensBySession, mapped.activeIds);
      for (const pool of pools) {
        const sessions: { session: string; model: string; tokens: number }[] = [];
        for (const [provider, list] of burn) {
          for (const s of list) {
            if (poolFor(provider, s.model) === pool.id) sessions.push(s);
          }
        }
        if (sessions.length) pool.activeSessions = sessions;
      }
    } else {
      console.error("collector: sandbox OpenCode server unreachable; attention empty");
    }
  } catch (err) {
    console.error(`collector: sandbox server read failed (${err}); attention empty`);
  }

  const feed: FleetFeed = {
    version: 1,
    generatedAt: now.toISOString(),
    quota: { pools, burn: [], choices: computeChoices(pools, now.getTime()) },
    attention,
  };
  await writeFile(outPath, JSON.stringify(feed, null, 2));
  const withReads = pools.filter((p) => p.windows.length > 0).length;
  console.log(
    `collector: wrote ${outPath} — ${pools.length} pools (${withReads} with reads), ${attention.length} sandbox sessions`,
  );
}

await main();
