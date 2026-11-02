import { describe, test, expect } from "bun:test";
import { generateKeyPairSync } from "node:crypto";
import { appJwt, apiTokenFor, listResource } from "../src/github";

function testKeyB64(type: "pkcs8" | "pkcs1"): string {
  const { privateKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
  return Buffer.from(privateKey.export({ type, format: "der" })).toString("base64");
}

function testKeyPem(type: "pkcs8" | "pkcs1"): string {
  const { privateKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
  return privateKey.export({ type, format: "pem" }) as string;
}

describe("appJwt key formats", () => {
  test("signs with a PKCS#8 key (control)", async () => {
    const jwt = await appJwt({ GITHUB_APP_ID: "123", GITHUB_APP_PRIVATE_KEY: testKeyB64("pkcs8") });
    expect(jwt.split(".")).toHaveLength(3);
  });

  test("signs with a PKCS#1 key — the format GitHub issues for Apps", async () => {
    const jwt = await appJwt({ GITHUB_APP_ID: "123", GITHUB_APP_PRIVATE_KEY: testKeyB64("pkcs1") });
    expect(jwt.split(".")).toHaveLength(3);
  });

  test("signs with raw PKCS#8 PEM text", async () => {
    const jwt = await appJwt({ GITHUB_APP_ID: "123", GITHUB_APP_PRIVATE_KEY: testKeyPem("pkcs8") });
    expect(jwt.split(".")).toHaveLength(3);
  });

  test("signs with raw PKCS#1 PEM text", async () => {
    const jwt = await appJwt({ GITHUB_APP_ID: "123", GITHUB_APP_PRIVATE_KEY: testKeyPem("pkcs1") });
    expect(jwt.split(".")).toHaveLength(3);
  });

  test("signs with base64-of-PEM PKCS#8 key", async () => {
    const b64 = Buffer.from(testKeyPem("pkcs8"), "utf8").toString("base64");
    const jwt = await appJwt({ GITHUB_APP_ID: "123", GITHUB_APP_PRIVATE_KEY: b64 });
    expect(jwt.split(".")).toHaveLength(3);
  });

  test("signs with base64-of-PEM PKCS#1 key", async () => {
    const b64 = Buffer.from(testKeyPem("pkcs1"), "utf8").toString("base64");
    const jwt = await appJwt({ GITHUB_APP_ID: "123", GITHUB_APP_PRIVATE_KEY: b64 });
    expect(jwt.split(".")).toHaveLength(3);
  });
});

describe("listResource pagination", () => {
  test("follows the Link header's absolute next URL as-is", async () => {
    const calls: string[] = [];
    const fetchFn = async (url: string) => {
      calls.push(url);
      if (url === "https://api.github.com/repos/o/r/issues?per_page=100") {
        return new Response(JSON.stringify([{ n: 1 }]), {
          status: 200,
          headers: { link: '<https://api.github.com/repos/o/r/issues?per_page=100&page=2>; rel="next"' },
        });
      }
      if (url === "https://api.github.com/repos/o/r/issues?per_page=100&page=2") {
        return new Response(JSON.stringify([{ n: 2 }]), { status: 200 });
      }
      return new Response("{}", { status: 404 });
    };
    const rows = await listResource<{ n: number }>(
      { kind: "token", token: "t" },
      "repos/o/r/issues?per_page=100",
      fetchFn as unknown as typeof fetch,
    );
    expect(rows).toEqual([{ n: 1 }, { n: 2 }]);
    expect(calls[1]).toBe("https://api.github.com/repos/o/r/issues?per_page=100&page=2");
  });
});

describe("apiTokenFor installation resolution", () => {
  test("discovers the installation per repo when no static id is configured", async () => {
    const env = { GITHUB_APP_ID: "123", GITHUB_APP_PRIVATE_KEY: testKeyB64("pkcs8") };
    const fetchFn = async (url: string) => {
      if (url === "https://api.github.com/repos/acme/widgets/installation") {
        return Response.json({ id: 777 });
      }
      if (url === "https://api.github.com/app/installations/777/access_tokens") {
        return Response.json({ token: "inst-token" });
      }
      return new Response("{}", { status: 404 });
    };
    const auth = await apiTokenFor(env, "acme/widgets", fetchFn as unknown as typeof fetch);
    expect(auth).toEqual({ kind: "app", token: "inst-token" });
  });

  test("rejects a present invalid static installation ID instead of falling back", async () => {
    for (const raw of ["abc", "12.5", "0", "-1", "9007199254740992"]) {
      const calls: string[] = [];
      const fetchFn = async (url: string) => {
        calls.push(url);
        return new Response("{}", { status: 404 });
      };
      const result = await apiTokenFor(
        { GITHUB_APP_INSTALL_ID: raw, GITHUB_TOKEN: "plain-token" },
        "acme/invalid-static",
        fetchFn as unknown as typeof fetch,
      ).catch((error: unknown) => error);
      expect(result).toBeInstanceOf(Error);
      expect((result as Error).message).toContain("GITHUB_APP_INSTALL_ID");
      expect((result as Error).message).toContain(JSON.stringify(raw));
      expect(calls).toEqual([]);
    }
  });

  test("keeps the plain-token fallback for absent or empty static IDs", async () => {
    for (const raw of [undefined, ""]) {
      const auth = await apiTokenFor(
        { GITHUB_APP_INSTALL_ID: raw, GITHUB_TOKEN: "plain-token" },
        "acme/no-app",
      );
      expect(auth).toEqual({ kind: "token", token: "plain-token" });
    }
  });

  test("uses a valid static installation ID when discovery is unavailable", async () => {
    const env = {
      GITHUB_APP_ID: "123",
      GITHUB_APP_PRIVATE_KEY: testKeyB64("pkcs8"),
      GITHUB_APP_INSTALL_ID: "42",
    };
    const calls: string[] = [];
    const fetchFn = async (url: string) => {
      calls.push(url);
      if (url === "https://api.github.com/repos/acme/static/installation") return new Response("{}", { status: 404 });
      if (url === "https://api.github.com/app/installations/42/access_tokens") return Response.json({ token: "static-token" });
      return new Response("{}", { status: 404 });
    };
    const auth = await apiTokenFor(env, "acme/static", fetchFn as unknown as typeof fetch);
    expect(auth).toEqual({ kind: "app", token: "static-token" });
    expect(calls).toContain("https://api.github.com/app/installations/42/access_tokens");
  });
});
