// DNS snapshot publisher (#14968 slice 2): the oracle is a fake
// Cloudflare D1 REST endpoint that records every request — it
// decides the endpoint, the bearer credential, the exact statements
// and parameters, and that the credential never reaches an error.
import { describe, expect, test } from "bun:test";
import {
  createDnsPublisher,
  createDnsPublisherGetter,
  DNS_SNAPSHOT_CREATE_SQL,
  DNS_SNAPSHOT_ID,
  DNS_SNAPSHOT_UPSERT_SQL,
  dnsSnapshotStatements,
} from "../src/dns-publisher.ts";
import type { DnsSection } from "../src/model.ts";

const ACCOUNT_ID = "0123456789abcdef0123456789abcdef";
const DATABASE_ID = "57d15925-b0b8-4466-a92c-c200a6af4c45";
const TOKEN = "test-token-never-logged";
const RECEIVED_AT = "2026-11-26T12:05:00.000Z";

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

interface RecordedCall {
  url: string;
  method?: string;
  authorization?: string;
  body: { sql: string; params: unknown[] };
}

/** A fake D1 REST endpoint: records calls, answers like D1. */
function fakeD1Endpoint(respond?: (call: RecordedCall) => Response) {
  const calls: RecordedCall[] = [];
  const fetchFn = (async (url: string | URL | Request, init?: RequestInit) => {
    const call: RecordedCall = {
      url: String(url),
      method: init?.method,
      authorization: (init?.headers as Record<string, string>)?.Authorization,
      body: JSON.parse(String(init?.body)),
    };
    calls.push(call);
    if (respond) return respond(call);
    return new Response(JSON.stringify({ success: true, result: [{ success: true, results: [] }] }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  }) as typeof fetch;
  return { calls, fetchFn };
}

describe("dnsSnapshotStatements", () => {
  test("create-then-upsert, with the exact section as the stored row", () => {
    const statements = dnsSnapshotStatements(SECTION, RECEIVED_AT);
    expect(statements).toHaveLength(2);
    expect(statements[0]).toEqual({ sql: DNS_SNAPSHOT_CREATE_SQL, params: [] });
    expect(statements[1].sql).toBe(DNS_SNAPSHOT_UPSERT_SQL);
    expect(statements[1].sql).toContain("on conflict(id) do update");
    expect(statements[1].params).toEqual([
      DNS_SNAPSHOT_ID,
      JSON.stringify(SECTION),
      SECTION.generatedAt,
      RECEIVED_AT,
      SECTION.source,
    ]);
  });

  test("refuses a section the shared gate would not serve", () => {
    expect(() => dnsSnapshotStatements({ ...SECTION, entries: [] }, RECEIVED_AT)).toThrow("invalid DNS section");
    expect(() => dnsSnapshotStatements({ ...SECTION, totalAnnualListUsd: -1 }, RECEIVED_AT)).toThrow(
      "invalid DNS section",
    );
  });

  test("refuses an unparseable receivedAt", () => {
    expect(() => dnsSnapshotStatements(SECTION, "not a timestamp")).toThrow("valid receivedAt");
  });
});

describe("createDnsPublisher", () => {
  test("posts both statements to the account's D1 endpoint with the bearer token", async () => {
    const { calls, fetchFn } = fakeD1Endpoint();
    const publisher = createDnsPublisher({ accountId: ACCOUNT_ID, databaseId: DATABASE_ID, token: TOKEN, fetchFn });
    await publisher.publish(SECTION, RECEIVED_AT);
    expect(calls).toHaveLength(2);
    for (const call of calls) {
      expect(call.url).toBe(
        `https://api.cloudflare.com/client/v4/accounts/${ACCOUNT_ID}/d1/database/${DATABASE_ID}/query`,
      );
      expect(call.method).toBe("POST");
      expect(call.authorization).toBe(`Bearer ${TOKEN}`);
    }
    expect(calls[0].body).toEqual({ sql: DNS_SNAPSHOT_CREATE_SQL, params: [] });
    expect(calls[1].body.sql).toBe(DNS_SNAPSHOT_UPSERT_SQL);
    expect(calls[1].body.params[1]).toBe(JSON.stringify(SECTION));
  });

  test("re-publishing the same snapshot sends byte-identical requests (idempotent)", async () => {
    const { calls, fetchFn } = fakeD1Endpoint();
    const publisher = createDnsPublisher({ accountId: ACCOUNT_ID, databaseId: DATABASE_ID, token: TOKEN, fetchFn });
    await publisher.publish(SECTION, RECEIVED_AT);
    await publisher.publish(SECTION, RECEIVED_AT);
    expect(calls).toHaveLength(4);
    expect(calls[2]).toEqual(calls[0]);
    expect(calls[3]).toEqual(calls[1]);
  });

  test("an invalid section is refused before any request is made", async () => {
    const { calls, fetchFn } = fakeD1Endpoint();
    const publisher = createDnsPublisher({ accountId: ACCOUNT_ID, databaseId: DATABASE_ID, token: TOKEN, fetchFn });
    await expect(publisher.publish({ ...SECTION, entries: [] }, RECEIVED_AT)).rejects.toThrow("invalid DNS section");
    expect(calls).toHaveLength(0);
  });

  test("an authentication refusal fails without the token reaching the error", async () => {
    const { fetchFn } = fakeD1Endpoint(
      () =>
        new Response(JSON.stringify({ success: false, errors: [{ message: `bad token ${TOKEN}` }] }), { status: 401 }),
    );
    const publisher = createDnsPublisher({ accountId: ACCOUNT_ID, databaseId: DATABASE_ID, token: TOKEN, fetchFn });
    const err = await publisher.publish(SECTION, RECEIVED_AT).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(Error);
    expect((err as Error).message).toContain("HTTP 401");
    expect((err as Error).message).not.toContain(TOKEN);
  });

  test("a success:false body fails generically, without echoing the response", async () => {
    const { fetchFn } = fakeD1Endpoint(
      () =>
        new Response(JSON.stringify({ success: false, result: [{ success: false }], errors: [{ message: TOKEN }] }), {
          status: 200,
        }),
    );
    const publisher = createDnsPublisher({ accountId: ACCOUNT_ID, databaseId: DATABASE_ID, token: TOKEN, fetchFn });
    const err = await publisher.publish(SECTION, RECEIVED_AT).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(Error);
    expect((err as Error).message).not.toContain(TOKEN);
  });

  test("bad configuration is refused at construction", () => {
    const { fetchFn } = fakeD1Endpoint();
    expect(() => createDnsPublisher({ accountId: "nope", databaseId: DATABASE_ID, token: TOKEN, fetchFn })).toThrow(
      "invalid Cloudflare account ID",
    );
    expect(() => createDnsPublisher({ accountId: ACCOUNT_ID, databaseId: DATABASE_ID, token: " ", fetchFn })).toThrow(
      "missing Cloudflare API token",
    );
  });
});

describe("createDnsPublisherGetter", () => {
  test("no D1 env names no publisher (the collector logs the skip)", async () => {
    const get = createDnsPublisherGetter(async () => TOKEN, {});
    expect(await get()).toBeNull();
  });

  test("no token by name names no publisher", async () => {
    const get = createDnsPublisherGetter(async () => null, {
      CLOUDFLARE_ACCOUNT_ID: ACCOUNT_ID,
      PM_AGG_D1_DATABASE_ID: DATABASE_ID,
    });
    expect(await get()).toBeNull();
  });

  test("configured env and token build a publisher", async () => {
    const get = createDnsPublisherGetter(async () => TOKEN, {
      CLOUDFLARE_ACCOUNT_ID: ACCOUNT_ID,
      PM_AGG_D1_DATABASE_ID: DATABASE_ID,
    });
    expect(await get()).not.toBeNull();
  });
});
