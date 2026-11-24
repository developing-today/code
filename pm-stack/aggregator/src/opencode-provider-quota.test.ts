import { describe, expect, test } from "bun:test";
import { fetchOpenCodeProviderQuota } from "./opencode-provider-quota.ts";

describe("fetchOpenCodeProviderQuota", () => {
  test("uses server auth and returns only the provider quota answer", async () => {
    // Oracle: the authenticated request targets the provider-only route,
    // and the returned payload equals the server's quota-only response.
    let request: Request | undefined;
    const answer = {
      data: {
        providerId: "opendesign",
        providerName: "Open Design",
        ok: true,
        configured: true,
        usage: { windows: { "5h": { usedPercent: 25 } } },
        fetchedAt: 1_800_000_000_000,
      },
    };
    const result = await fetchOpenCodeProviderQuota({
      baseUrl: "http://127.0.0.1:13420/",
      auth: { username: "opencode", password: "fixture-password" },
      providerId: "opendesign",
      fetchImpl: (async (input, init) => {
        request = new Request(input, init);
        return Response.json(answer);
      }) as typeof fetch,
    });

    expect(request?.url).toBe("http://127.0.0.1:13420/api/experimental/credential/provider/opendesign/quota");
    expect(request?.headers.get("authorization")).toBe(
      `Basic ${Buffer.from("opencode:fixture-password").toString("base64")}`,
    );
    expect(result).toEqual(answer.data);
    expect(JSON.stringify(result)).not.toContain("fixture-password");
  });

  test("falls back when the server does not yet expose the route", async () => {
    const result = await fetchOpenCodeProviderQuota({
      baseUrl: "http://127.0.0.1:13420",
      auth: { username: "opencode", password: "fixture-password" },
      providerId: "opendesign",
      fetchImpl: (async () => new Response(null, { status: 404 })) as unknown as typeof fetch,
    });
    expect(result).toBeNull();
  });

  test("rejects malformed or mismatched provider responses", async () => {
    const result = await fetchOpenCodeProviderQuota({
      baseUrl: "http://127.0.0.1:13420",
      auth: { username: "opencode", password: "fixture-password" },
      providerId: "opendesign",
      fetchImpl: (async () =>
        Response.json({
          data: {
            providerId: "other",
            providerName: "Other",
            ok: true,
            configured: true,
            usage: null,
            fetchedAt: 1,
          },
        })) as unknown as typeof fetch,
    });
    expect(result).toBeNull();
  });
});
