// #3124 (queue task 4590): live source usage() must consume the feed's
// charges + chargesAttention in EVERY return path, not only inside the
// saas gate. A feed carrying charges but no saas section — e.g. when
// the quota-view fallback serves usage — must still surface the charge
// rows and the unmatched-charge attention flags.
import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { extractSaasServices } from "../src/saas-extract.ts";
import type { ChargeRow } from "../src/model.ts";
import type { UnmatchedCharge } from "../src/charges-match.ts";
import { liveSource } from "../src/sources/live.ts";

const FEED_CHARGE: ChargeRow = {
  at: "2026-10-30",
  label: "Mystery AI Inc",
  amountUsd: 42.5,
  method: "Visa •7535",
  source: "plaid",
};
const FEED_UNMATCHED: UnmatchedCharge = {
  merchant: "Mystery AI Inc",
  normalized: "mystery ai",
  amountUsd: 42.5,
  at: "2026-10-30",
  source: "plaid",
};

const VIEW = `
## Summary (b) — subscriptions, infrastructure, account tooling

| Service | Plan / price | Allowance sketch | Usage / status | Next date recorded |
|---|---|---|---|---|
| Supabase | Free plan (muse-dev) | DB + egress + storage | manual dashboard read needed | unknown |
`;

async function sourceWith(opts: {
  feedOver: Record<string, unknown>;
  quotaView?: string;
}): Promise<ReturnType<typeof liveSource>> {
  const dir = await mkdtemp(join(tmpdir(), "pm-charges-gate-"));
  const feedFile = join(dir, "fleet-feed.json");
  await writeFile(
    feedFile,
    JSON.stringify({
      version: 1,
      generatedAt: new Date().toISOString(),
      quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
      attention: [],
      ...opts.feedOver,
    }),
  );
  const quotaViewFile = join(dir, "quota-view.md");
  if (opts.quotaView !== undefined) await writeFile(quotaViewFile, opts.quotaView);
  const cfg = loadConfig({
    PM_AGG_SOURCE: "live",
    PM_AGG_FEED_FILE: feedFile,
    PM_AGG_QUOTA_VIEW: quotaViewFile,
    PM_AGG_SUPABASE_EXPORT: join(dir, "no-supabase-export.json"),
    PM_AGG_CLOUDFLARE_EXPORT: join(dir, "no-cloudflare-export.json"),
    PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
    PM_AGG_COMMENTS_FILE: join(dir, "comments.json"),
    PM_AGG_SNAPSHOTS_DIR: join(dir, "snapshots"),
    PM_AGG_OPENCHAMBER_STATE: join(dir, "no-state.json"),
    PM_PROJECTOR_URL: "off",
    KANEO_ADMIN_EMAIL: "",
    KANEO_ADMIN_PASSWORD: "",
    GRIST_API_KEY: "",
  });
  return liveSource(cfg);
}

describe("live source usage(): feed charges outside the saas gate (#3124)", () => {
  test("quota-view fallback: feed charges + unmatched-charge flags survive with no saas section", async () => {
    const src = await sourceWith({
      feedOver: { charges: [FEED_CHARGE], chargesAttention: [FEED_UNMATCHED] },
      quotaView: VIEW,
    });
    const usage = await src.usage();
    expect(usage.charges).toContainEqual(FEED_CHARGE);
    expect(usage.chargesNote).toBeUndefined();
    const flags = usage.attention.filter((a) => a.kind === "unmatched-charge");
    expect(flags).toHaveLength(1);
    expect(flags[0]!.label).toContain("Mystery AI Inc");
  });

  test("malformed chargesAttention entries drop at load; the well-formed flag still renders (#2439)", async () => {
    const src = await sourceWith({
      feedOver: { chargesAttention: [null, { ...FEED_UNMATCHED, amountUsd: "42.50" }, FEED_UNMATCHED] },
      quotaView: VIEW,
    });
    const usage = await src.usage();
    const flags = usage.attention.filter((a) => a.kind === "unmatched-charge");
    expect(flags).toHaveLength(1);
    expect(flags[0]!.label).toContain("Mystery AI Inc");
  });

  test("fixtures fallback: feed charges are kept when the quota view is unreadable", async () => {
    const src = await sourceWith({
      feedOver: { charges: [FEED_CHARGE], chargesAttention: [FEED_UNMATCHED] },
      // no quotaView file written -> the direct read throws -> fixtures fallback
    });
    const usage = await src.usage();
    expect(usage.charges).toContainEqual(FEED_CHARGE);
    expect(usage.chargesNote).toBeUndefined();
    expect(usage.attention.filter((a) => a.kind === "unmatched-charge")).toHaveLength(1);
  });

  test("quota-view fallback: a charge recorded in both the feed and the prose appears once (feed row wins)", async () => {
    // The prose extractor builds its own row for this charge (label
    // "Meta PAYG spill", date 2026-10-25, descriptive source string);
    // the feed row is the same charge with different strings. The
    // charge-level merge must keep exactly one row — the feed's.
    const feedRow: ChargeRow = {
      at: "2026-10-27",
      label: "Meta PAYG spill",
      amountUsd: 42.5,
      method: "Visa •7535",
      source: "plaid",
    };
    const src = await sourceWith({
      feedOver: { charges: [feedRow] },
      quotaView: `${VIEW}\n\nVisa •7535 — $42.50 charged + $42.50 charged\n`,
    });
    const usage = await src.usage();
    expect(usage.charges).toEqual([feedRow]);
  });

  test("feed-saas path unchanged: charges + flags come from the feed as before", async () => {
    const src = await sourceWith({
      feedOver: {
        saas: extractSaasServices(VIEW),
        charges: [FEED_CHARGE],
        chargesAttention: [FEED_UNMATCHED],
      },
      quotaView: VIEW,
    });
    const usage = await src.usage();
    expect(usage.charges).toEqual([FEED_CHARGE]);
    expect(usage.services.some((s) => s.id === "supabase")).toBe(true);
    expect(usage.attention.filter((a) => a.kind === "unmatched-charge")).toHaveLength(1);
  });
});
