#!/usr/bin/env bun
// #9608 owner break-glass token CLI (design #1253 section 6).
// Command contract: issue-break-glass-token [--ttl-minutes N]
//   [--tokens PATH] [--events PATH]
// Run on the laptop only. Issues a single-use admin stack token that
// lives at most one hour (default: one hour). The issuance is written
// as an audit event before the token record is persisted — if the
// audit write fails, no token is issued. The token value is printed
// exactly once, to stdout; only its sha256 hash is stored. The
// credential never leaves the laptop.
//
// Paths default to the aggregator's own state locations (the same
// defaults loadConfig uses: PM_AGG_EVENTS_FILE / PM_AGG_BREAK_GLASS_FILE,
// the state dir under ~/.local/state/pm-aggregator). This CLI resolves
// them directly instead of calling loadConfig: it is a local operator
// tool, not the server, so the server's Source gate (#1195) must not
// stand between the owner and the recovery path during an outage.
import { join, resolve } from "node:path";
import { BREAK_GLASS_MAX_TTL_MS, BreakGlassTokenStore, issueBreakGlassToken } from "../src/break-glass.ts";
import { EventsStore } from "../src/events.ts";
import { requireHome } from "../src/home.ts";

interface Args {
  ttlMinutes?: number;
  tokensFile?: string;
  eventsFile?: string;
}

function parseArgs(argv: string[]): Args {
  const args: Args = {};
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]!;
    if (arg === "--ttl-minutes" || arg === "--tokens" || arg === "--events") {
      const value = argv[++i];
      if (!value || value.startsWith("--")) throw new Error(`${arg} requires a value`);
      if (arg === "--ttl-minutes") {
        const minutes = Number(value);
        if (!Number.isFinite(minutes) || minutes <= 0) {
          throw new Error("--ttl-minutes must be a positive number");
        }
        args.ttlMinutes = minutes;
      } else if (arg === "--tokens") {
        args.tokensFile = value;
      } else {
        args.eventsFile = value;
      }
      continue;
    }
    throw new Error(`unknown argument: ${arg}`);
  }
  return args;
}

async function main(): Promise<void> {
  const args = parseArgs(process.argv.slice(2));
  const tokensFile =
    args.tokensFile ??
    process.env.PM_AGG_BREAK_GLASS_FILE ??
    join(requireHome("PM_AGG_BREAK_GLASS_FILE"), ".local", "state", "pm-aggregator", "break-glass-tokens.json");
  const eventsFile = args.eventsFile ?? process.env.PM_AGG_EVENTS_FILE ?? join(process.cwd(), "events.json");
  const ttlMs = args.ttlMinutes === undefined ? BREAK_GLASS_MAX_TTL_MS : Math.round(args.ttlMinutes * 60 * 1000);

  const issued = await issueBreakGlassToken({
    store: new BreakGlassTokenStore(resolve(tokensFile)),
    events: new EventsStore(resolve(eventsFile)),
    ttlMs,
  });

  console.log(`break-glass token issued (audit event recorded first)`);
  console.log(`  id:      ${issued.id}`);
  console.log(`  expires: ${issued.expiresAt} (single-use, at most one hour)`);
  console.log(`  token:   ${issued.token}`);
  console.log(`Present the token once to consume it; it is not stored anywhere — only its hash is.`);
}

main().catch((error: unknown) => {
  console.error(`issue-break-glass-token: ${error instanceof Error ? error.message : String(error)}`);
  process.exit(1);
});
