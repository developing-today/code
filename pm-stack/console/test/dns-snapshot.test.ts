// Hosted /dns snapshot reader (#14968 slice 2): the oracle is a fake
// D1 adapter standing in for the LEDGER_DB binding — the rows it
// stores are read back through the real D1 query path
// (createD1LedgerQuery), so exact snapshot storage, stale/missing
// fail-closed behavior, and HTML/JSON parity are all decided here.
import { describe, expect, test } from "bun:test";
import {
  createD1LedgerQuery,
  type LedgerD1Database,
  type LedgerPagesIo,
} from "../edge/src/ledger-pages.ts";
import {
  DNS_SNAPSHOT_MAX_AGE_MS,
  DNS_SNAPSHOT_SQL,
  dnsPage,
  dnsSnapshotFromRow,
  type DnsSection,
} from "../edge/src/dns-pages.ts";

const NOW = Date.parse("2026-11-26T12:10:00.000Z");
const now = () => NOW;

const SECTION: DnsSection = {
  generatedAt: "2026-11-26T12:00:00.000Z",
  source: "register.json (#902 pipeline) via fleet feed collector",
  totalAnnualListUsd: 23.16,
  entries: [
    {
      domain: "security.cab",
      registrar: "Porkbun",
      dnsHost: "Cloudflare",
      renewal: "2027-01-15",
      autoRenew: "ON",
      priceUsd: 12.08,
      priorYearCost: "$11.08",
      used: "PM stack",
      plan: "Active — pm.security.cab",
      review: "",
      status: "active",
    },
    {
      domain: "example.dev",
      registrar: "Porkbun",
      dnsHost: "Porkbun",
      renewal: "2027-03-01",
      autoRenew: "OFF",
      priceUsd: 11.08,
      priorYearCost: "",
      used: "",
      plan: "",
      review: "",
      status: "unused",
    },
  ],
};

function snapshotRow(receivedAt: string, section: DnsSection = SECTION): Record<string, unknown> {
  return {
    section_json: JSON.stringify(section),
    generated_at: section.generatedAt,
    received_at: receivedAt,
    source: section.source,
  };
}

/** A fake D1 database: prepare() only accepts the snapshot SQL and
 * answers the rows it was given, as the binding would. */
function fakeD1(rows: Record<string, unknown>[] | Error): LedgerD1Database {
  return {
    prepare: (sql: string) => {
      if (sql !== DNS_SNAPSHOT_SQL) throw new Error(`unexpected sql: ${sql}`);
      return {
        all: async () => {
          if (rows instanceof Error) throw rows;
          return { results: rows };
        },
      };
    },
  };
}

function ioWith(rows: Record<string, unknown>[] | Error): LedgerPagesIo {
  return { now, query: createD1LedgerQuery(fakeD1(rows)) };
}

const FRESH = "2026-11-26T12:05:00.000Z"; // 5 minutes before NOW

describe("dnsSnapshotFromRow", () => {
  test("decodes the stored row exactly", () => {
    const snapshot = dnsSnapshotFromRow(snapshotRow(FRESH));
    expect(snapshot?.section).toEqual(SECTION);
    expect(snapshot?.generatedAt).toBe(SECTION.generatedAt);
    expect(snapshot?.receivedAt).toBe(FRESH);
    expect(snapshot?.source).toBe(SECTION.source);
  });

  test("refuses malformed JSON, a gated-out section, and disagreeing provenance", () => {
    expect(dnsSnapshotFromRow({ ...snapshotRow(FRESH), section_json: "{nope" })).toBeNull();
    expect(
      dnsSnapshotFromRow(snapshotRow(FRESH, { ...SECTION, entries: [] })),
    ).toBeNull();
    expect(dnsSnapshotFromRow({ ...snapshotRow(FRESH), generated_at: "2026-11-26T11:00:00.000Z" })).toBeNull();
    expect(dnsSnapshotFromRow({ ...snapshotRow(FRESH), received_at: "not a timestamp" })).toBeNull();
  });
});

describe("hosted /dns", () => {
  test("json twin returns the stored section exactly, with register + publisher freshness", async () => {
    const res = await dnsPage(new Request("https://pm.security.cab/api/dns.json"), ioWith([snapshotRow(FRESH)]));
    expect(res?.status).toBe(200);
    const body = (await res!.json()) as any;
    expect(body.entries).toEqual(SECTION.entries);
    expect(body.generatedAt).toBe(SECTION.generatedAt);
    expect(body.totalAnnualListUsd).toBe(SECTION.totalAnnualListUsd);
    expect(body.freshness.registerGeneratedAt).toBe(SECTION.generatedAt);
    expect(body.freshness.publisherReceivedAt).toBe(FRESH);
    expect(body.freshness.source).toBe(SECTION.source);
    expect(body.freshness.maxAgeMs).toBe(DNS_SNAPSHOT_MAX_AGE_MS);
  });

  test("html page renders the same register the json twin returns (parity)", async () => {
    const res = await dnsPage(new Request("https://pm.security.cab/dns"), ioWith([snapshotRow(FRESH)]));
    expect(res?.status).toBe(200);
    const html = await res!.text();
    for (const entry of SECTION.entries) {
      expect(html).toContain(entry.domain);
      expect(html).toContain(entry.renewal);
    }
    expect(html).toContain(SECTION.generatedAt);
    expect(html).toContain(FRESH);
    expect(html).toContain(SECTION.source);
    expect(html).toContain("2 domains — 1 active, 0 earmarked, 1 unused");
    expect(html).toContain("$23.16/yr");
  });

  test("a missing snapshot is a visible 503, never an empty register", async () => {
    const html = await dnsPage(new Request("https://pm.security.cab/dns"), ioWith([]));
    expect(html?.status).toBe(503);
    expect(await html!.text()).toContain("No DNS snapshot has been published");
    const api = await dnsPage(new Request("https://pm.security.cab/api/dns.json"), ioWith([]));
    expect(api?.status).toBe(503);
    expect(((await api!.json()) as any).error).toBe("dns snapshot missing");
  });

  test("a stale snapshot (older than 15 minutes) fails visibly on both twins", async () => {
    const stale = new Date(NOW - DNS_SNAPSHOT_MAX_AGE_MS - 60_000).toISOString();
    const html = await dnsPage(new Request("https://pm.security.cab/dns"), ioWith([snapshotRow(stale)]));
    expect(html?.status).toBe(503);
    const htmlText = await html!.text();
    expect(htmlText).toContain("stale");
    expect(htmlText).toContain(stale);
    const api = await dnsPage(new Request("https://pm.security.cab/api/dns.json"), ioWith([snapshotRow(stale)]));
    expect(api?.status).toBe(503);
    const body = (await api!.json()) as any;
    expect(body.error).toBe("dns snapshot stale");
    expect(body.freshness.publisherReceivedAt).toBe(stale);
  });

  test("a snapshot exactly at the freshness limit still serves", async () => {
    const atLimit = new Date(NOW - DNS_SNAPSHOT_MAX_AGE_MS).toISOString();
    const res = await dnsPage(new Request("https://pm.security.cab/api/dns.json"), ioWith([snapshotRow(atLimit)]));
    expect(res?.status).toBe(200);
  });

  test("a future-dated snapshot is malformed, not maximally fresh", async () => {
    const future = new Date(NOW + 60 * 60_000).toISOString();
    const res = await dnsPage(new Request("https://pm.security.cab/api/dns.json"), ioWith([snapshotRow(future)]));
    expect(res?.status).toBe(503);
  });

  test("a failed read (no snapshot table yet) is a 503", async () => {
    const res = await dnsPage(
      new Request("https://pm.security.cab/dns"),
      ioWith(new Error("no such table: dns_snapshots")),
    );
    expect(res?.status).toBe(503);
    expect(await res!.text()).toContain("did not answer");
  });

  test("other paths and methods are not this handler's", async () => {
    expect(await dnsPage(new Request("https://pm.security.cab/alerts"), ioWith([snapshotRow(FRESH)]))).toBeNull();
    expect(
      await dnsPage(new Request("https://pm.security.cab/dns", { method: "POST" }), ioWith([snapshotRow(FRESH)])),
    ).toBeNull();
  });
});
