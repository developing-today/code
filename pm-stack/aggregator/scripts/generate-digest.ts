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

function arg(name: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : undefined;
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
