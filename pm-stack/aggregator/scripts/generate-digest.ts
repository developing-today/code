#!/usr/bin/env bun
// Generate the scheduled PM digest (#1278): build it from the canonical
// projection (the same Source the dashboard serves), archive it as
// JSON in the digests dir — the /digest/<date> page renders that file
// verbatim — and deliver it to chat via Web Push to the stored push
// subscriptions (the stack's delivery surface; class "digest").
//
// Usage:
//   bun run scripts/generate-digest.ts [--kind daily|weekly] [--no-deliver]
//
// The weekly timer passes --kind weekly on the configured day
// (systemd OnCalendar=Mon, see systemd/digest-weekly.timer and
// nixos/services/pm-stack). Delivery failure never fails the archive:
// the digest is written first and the exit code reports delivery only
// when --strict-deliver is passed.
import { loadConfig } from "../src/config.ts";
import { buildDigest, digestText, writeDigestArchive, type DigestKind } from "../src/digest.ts";
import { fixtureSource } from "../src/sources/fixtures.ts";
import { liveSource } from "../src/sources/live.ts";
import { classEnabled, getAlertsStore, inQuietHours } from "../src/alerts.ts";
import { sendPush, type PushTarget } from "../src/push.ts";

const USAGE = "usage: generate-digest.ts [--kind daily|weekly] [--no-deliver] [--strict-deliver]";

// A flag's value is the next argv token — but never another flag and
// never nothing (#3723, the post-alert.ts guard). The unconditional
// read used to make a present-but-valueless --kind indistinguishable
// from an absent one: the ?? default in main() then silently archived
// a daily digest for an invocation that named --kind and supplied no
// value, and a flag token in the value slot was blamed as a bad kind
// (#6926).
// Unknown or duplicated arguments are usage errors, never silently
// ignored (#7097): argv is probed token-by-token below, so a typo'd
// --knd weekly fell through to the daily default and a typo'd
// --no-delivre made the includes() check false and delivered anyway.
// Mirrors rejectUnknownArgs in append-usage-snapshots.ts (#6961) /
// compact-usage-snapshots.ts (#6967). Runs before any config load,
// archive write, or delivery attempt.
function rejectUnknownArgs(): void {
  const args = process.argv.slice(2);
  const seen = new Set<string>();
  for (let i = 0; i < args.length; i++) {
    const token = args[i] as string;
    if (token === "--kind") {
      if (seen.has(token)) {
        console.error("generate-digest: duplicate --kind");
        console.error(USAGE);
        process.exit(2);
      }
      seen.add(token);
      i++; // skip the value; arg() validates it
      continue;
    }
    if (token === "--no-deliver" || token === "--strict-deliver") {
      if (seen.has(token)) {
        console.error(`generate-digest: duplicate ${token}`);
        console.error(USAGE);
        process.exit(2);
      }
      seen.add(token);
      continue;
    }
    console.error(`generate-digest: unknown argument ${JSON.stringify(token)}`);
    console.error(USAGE);
    process.exit(2);
  }
}
rejectUnknownArgs();

function arg(name: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  if (i < 0) return undefined;
  const value = process.argv[i + 1];
  if (value === undefined || value === "" || value.startsWith("--")) {
    console.error(
      `generate-digest: --${name} requires a value that is not a flag (got ${value === undefined ? "no value" : JSON.stringify(value)})`,
    );
    console.error(USAGE);
    process.exit(2);
  }
  return value;
}

async function main(): Promise<void> {
  const kind = (arg("kind") ?? "daily") as DigestKind;
  if (kind !== "daily" && kind !== "weekly") {
    console.error(`generate-digest: --kind must be daily|weekly, got ${kind}`);
    process.exit(2);
  }
  const cfg = loadConfig();
  const source = cfg.source === "live" ? liveSource(cfg) : fixtureSource();
  // Prime the items facet first (#1267 pattern): items() records the
  // pool's feed freshness into provenance; projects() alone does not.
  await source.items([]).catch(() => null);
  const [pool, needsDrew, quota, poolAvailable] = await Promise.all([
    source.projects(),
    source.needsDrew ? source.needsDrew().catch(() => null) : Promise.resolve(null),
    source.quota().catch(() => null),
    source.projectsAvailable ? source.projectsAvailable().catch(() => undefined) : Promise.resolve(undefined),
  ]);
  const digest = buildDigest({
    pool,
    needsDrew,
    quota,
    provenanceDetail: source.provenanceDetail?.() ?? null,
    poolAvailable,
    sourceKind: cfg.source,
    now: new Date(),
    kind,
  });
  const file = await writeDigestArchive(cfg.digestsDir, digest);
  console.log(`generate-digest: archived ${kind} digest for ${digest.date} → ${file}`);
  console.log(digestText(digest));

  if (process.argv.includes("--no-deliver")) return;
  // Delivery: Web Push to every stored subscription, mirroring the
  // alert throat's send bookkeeping (gone subscriptions are noted,
  // sends are logged under class "digest"). No store (Supabase
  // unavailable) = no delivery surface; the archive still stands.
  const store = await getAlertsStore();
  if (!store) {
    console.error("generate-digest: no alerts store — delivery skipped, archive stands");
    return;
  }
  const subs = await store.listSubscriptions();
  const payload = {
    class: "digest",
    objectRef: digest.archiveRoute,
    url: digest.archiveRoute,
    title: `PM ${kind} digest — ${digest.date}`,
    body: digest.sections.map((s) => `${s.title}: ${s.items.length}${s.stale ? " (stale)" : ""}`).join(" · "),
  };
  let sent = 0;
  const now = new Date();
  for (const sub of subs) {
    // Subscription preferences are promises (#3676): a class the
    // subscriber turned off gets nothing, and a non-breakthrough class
    // like digest honors quiet hours (suppression is ledgered).
    if (!classEnabled(sub, "digest")) continue;
    if (inQuietHours(sub.quiet, now)) {
      await store.logSend({ class: "digest", objectRef: digest.archiveRoute, outcome: "suppressed-quiet" });
      continue;
    }
    const target: PushTarget = { endpoint: sub.endpoint, p256dh: sub.p256dh, auth: sub.auth };
    const outcome = await sendPush(target, payload);
    if (outcome.ok) {
      sent += 1;
      await store.logSend({ class: "digest", objectRef: digest.archiveRoute, outcome: "sent" });
      await store.noteSubscriptionAlive(sub.endpoint);
    } else if (outcome.gone) {
      await store.logSend({ class: "digest", objectRef: digest.archiveRoute, outcome: "gone" });
      await store.noteSubscriptionGone(sub.endpoint);
    } else {
      await store.logSend({ class: "digest", objectRef: digest.archiveRoute, outcome: "error" });
    }
  }
  console.log(`generate-digest: delivered to ${sent}/${subs.length} subscriptions`);
  if (process.argv.includes("--strict-deliver") && sent === 0 && subs.length > 0) process.exit(1);
}

await main();
