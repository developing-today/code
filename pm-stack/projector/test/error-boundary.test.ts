// #10540: a store failure a route does not handle itself must still
// answer JSON. Before the boundary in handleRequest, these routes let
// the exception escape and Bun answered a default non-JSON 500.
import { describe, test, expect } from "bun:test";
import type { Client } from "@libsql/client";
import { handleRequest, type ServerEnv } from "../src/server";

const env: ServerEnv = {};

/** A client whose every statement fails, as under DB contention. */
function brokenClient(): Client {
  const fail = async () => {
    throw new Error("SQLITE_BUSY: database is locked");
  };
  return { execute: fail, batch: fail, transaction: fail, executeMultiple: fail } as unknown as Client;
}

describe("projector error boundary (#10540)", () => {
  test("a store failure on a read route answers a JSON 500", async () => {
    for (const path of ["/items", "/comments?subject=github:o/r%2312", "/links?subject=github:o/r%2312", "/hot"]) {
      const res = await handleRequest(new Request(`http://127.0.0.1${path}`), brokenClient(), env);
      expect(res.status, path).toBe(500);
      expect(res.headers.get("content-type") ?? "", path).toContain("application/json");
      const body = (await res.json()) as { error?: string; message?: string };
      expect(body.error, path).toBe("internal_error");
      expect(body.message, path).toContain("database is locked");
    }
  });

  test("a route's own status is kept", async () => {
    const res = await handleRequest(new Request("http://127.0.0.1/no-such-route"), brokenClient(), env);
    expect(res.status).toBe(404);
  });
});
