import { describe, expect, test } from "bun:test";
import { createD1StoreClient, createD1StoreGetter, D1_API_TOKEN_NAME } from "./d1-stores.ts";

const ACCOUNT = "0123456789abcdef0123456789abcdef";
const DATABASE = "01234567-89ab-cdef-0123-456789abcdef";

describe("D1 store client", () => {
  test("sends translated SQL and bound parameters to the Cloudflare query API", async () => {
    let request: Request | undefined;
    const client = createD1StoreClient(ACCOUNT, DATABASE, "test-token", async (input, init) => {
      request = new Request(input, init);
      return Response.json({
        success: true,
        result: [{ success: true, results: [{ store: "report", document: '{"count":3}' }] }],
      });
    });

    const rows = await client.query<{ store: string; document: { count: number } }>(
      "select store, document from public.agg_store_documents where store = 'report'",
    );

    expect(client.ref).toBe(`d1:${DATABASE}`);
    expect(request?.url).toBe(`https://api.cloudflare.com/client/v4/accounts/${ACCOUNT}/d1/database/${DATABASE}/query`);
    expect(request?.method).toBe("POST");
    expect(request?.headers.get("authorization")).toBe("Bearer test-token");
    expect(await request?.json()).toEqual({
      sql: "select store, document from agg_store_documents where store = ?",
      params: ["report"],
    });
    expect(rows).toEqual([{ store: "report", document: { count: 3 } }]);
  });

  test("keeps a failed HTTP response and malformed API result fail-closed", async () => {
    const unavailable = createD1StoreClient(
      ACCOUNT,
      DATABASE,
      "test-token",
      async () => new Response("private error body", { status: 403 }),
    );
    const malformed = createD1StoreClient(ACCOUNT, DATABASE, "test-token", async () =>
      Response.json({ success: true, result: [{ success: false, results: [] }] }),
    );

    await expect(unavailable.query("select 1")).rejects.toThrow("HTTP 403");
    await expect(malformed.query("select 1")).rejects.toThrow("malformed result");
  });

  test("resolves the account token only when the D1 database is configured", async () => {
    const names: string[] = [];
    const env = { CLOUDFLARE_ACCOUNT_ID: ACCOUNT, PM_AGG_D1_DATABASE_ID: DATABASE };
    const getClient = createD1StoreGetter(async (name) => {
      names.push(name);
      return "test-token";
    }, env);

    expect(await getClient()).not.toBeNull();
    expect(names).toEqual([D1_API_TOKEN_NAME]);
    expect(await createD1StoreGetter(async () => "test-token", {} as NodeJS.ProcessEnv)()).toBeNull();
  });
});
