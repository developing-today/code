import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { OpenCodeClient, OpenCodeError } from "../src/opencode.ts";

// Pinning tests for #7085: the OpenCode client parses two JSON inputs —
// the server state/auth file and every 2xx response body. Both parses
// are boundary validations: wrong-shape or malformed input must surface
// as OpenCodeError, never a raw Type/SyntaxError, and a non-string
// password must never be coerced into the Basic credential.

let mode: "ok" | "nonjson" = "ok";
let lastAuth = "";
let server: ReturnType<typeof Bun.serve>;
let baseUrl = "";
let dir = "";

function authFile(name: string, content: string): string {
  const path = join(dir, name);
  writeFileSync(path, content);
  return path;
}

beforeAll(() => {
  dir = mkdtempSync(join(tmpdir(), "dispatch-opencode-test-"));
  server = Bun.serve({
    port: 0,
    fetch(req) {
      lastAuth = req.headers.get("authorization") ?? "";
      if (mode === "nonjson") {
        return new Response("<html>not json</html>", { status: 200, headers: { "content-type": "text/html" } });
      }
      return Response.json({ data: { id: "sess-control" } });
    },
  });
  baseUrl = `http://127.0.0.1:${server.port}`;
});

afterAll(() => {
  server.stop(true);
  rmSync(dir, { recursive: true, force: true });
});

async function create(file: string) {
  const client = new OpenCodeClient(baseUrl, file);
  return client.createSession({ directory: "/tmp", title: "t", model: null });
}

describe("OpenCodeClient auth file boundary (#7085)", () => {
  test("control: a valid auth file resolves and sends the password verbatim", async () => {
    mode = "ok";
    const session = await create(authFile("valid.json", JSON.stringify({ password: "pw123" })));
    expect(session.id).toBe("sess-control");
    expect(Buffer.from(lastAuth.replace("Basic ", ""), "base64").toString()).toBe("opencode:pw123");
  });

  test("control: an object without a password yields the no-password OpenCodeError", async () => {
    await expect(create(authFile("empty.json", "{}"))).rejects.toThrow(OpenCodeError);
    await expect(create(authFile("empty.json", "{}"))).rejects.toThrow(/carries no password/);
  });

  test("a null auth file is an OpenCodeError, not a raw TypeError", async () => {
    const err = await create(authFile("null.json", "null")).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(OpenCodeError);
    expect(String((err as Error).message)).toContain("does not hold a JSON object");
  });

  test("an array auth file is an OpenCodeError, not a raw error", async () => {
    const err = await create(authFile("array.json", "[]")).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(OpenCodeError);
  });

  test("a malformed auth file is an OpenCodeError, not a raw SyntaxError", async () => {
    const err = await create(authFile("bad.json", "{not json")).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(OpenCodeError);
    expect(String((err as Error).message)).toContain("not valid JSON");
  });

  test("a numeric password is rejected, never coerced into the credential", async () => {
    lastAuth = "";
    const err = await create(authFile("num.json", JSON.stringify({ password: 12345 }))).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(OpenCodeError);
    expect(lastAuth).toBe("");
  });

  test("an object password is rejected, never sent as [object Object]", async () => {
    lastAuth = "";
    const err = await create(authFile("obj.json", JSON.stringify({ password: { x: 1 } }))).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(OpenCodeError);
    expect(lastAuth).toBe("");
  });
});

describe("OpenCodeClient response body boundary (#7085)", () => {
  test("a non-JSON 200 body is an OpenCodeError, not a raw SyntaxError", async () => {
    mode = "nonjson";
    const err = await create(authFile("valid2.json", JSON.stringify({ password: "pw123" }))).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(OpenCodeError);
    expect(String((err as Error).message)).toContain("non-JSON response");
    mode = "ok";
  });
});

describe("OpenCodeClient.getSession (#5555)", () => {
  let stub: ReturnType<typeof Bun.serve>;
  let stubUrl = "";
  beforeAll(() => {
    stub = Bun.serve({
      port: 0,
      fetch(req) {
        const path = new URL(req.url).pathname;
        if (path === "/api/session/ses_ok") return Response.json({ data: { id: "ses_ok", title: "t" } });
        if (path === "/api/session/ses_boom") return new Response("boom", { status: 500 });
        return new Response("nope", { status: 404 });
      },
    });
    stubUrl = `http://127.0.0.1:${stub.port}`;
  });
  afterAll(() => {
    stub.stop(true);
  });

  test("returns the session object on 200 (envelope unwrapped)", async () => {
    const client = new OpenCodeClient(stubUrl, authFile("gs.json", JSON.stringify({ password: "pw" })));
    const raw = (await client.getSession("ses_ok")) as { id: string };
    expect(raw.id).toBe("ses_ok");
  });

  test("returns null on 404 — the session does not exist", async () => {
    const client = new OpenCodeClient(stubUrl, authFile("gs.json", JSON.stringify({ password: "pw" })));
    expect(await client.getSession("ses_missing")).toBeNull();
  });

  test("a 500 stays an OpenCodeError — gone is not conflated with unknown", async () => {
    const client = new OpenCodeClient(stubUrl, authFile("gs.json", JSON.stringify({ password: "pw" })));
    await expect(client.getSession("ses_boom")).rejects.toBeInstanceOf(OpenCodeError);
  });
});
