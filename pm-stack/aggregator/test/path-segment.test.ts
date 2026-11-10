// #2748: the two aggregator path-segment decodes (/discuss, /item) are
// guarded — malformed percent-encoding earns the shared contract's
// JSON 400 instead of a raw URIError escaping the handler (the #2747
// probe matrix), and every legitimate encoding behaves byte-identically
// to the unguarded decode it replaces.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";
import { decodePathSegment } from "../src/path-segment.ts";

describe("decodePathSegment", () => {
  const malformed = ["%", "%zz", "%E0%A4%A", "%2", "a%", "%u1234"];
  for (const raw of malformed) {
    test(`malformed ${JSON.stringify(raw)} -> null`, () => {
      expect(decodePathSegment(raw)).toBeNull();
    });
  }
  const controls: Array<[string, string]> = [
    ["plain-ref", "plain-ref"],
    ["a%20b", "a b"],
    ["a%2Fb", "a/b"], // %2F still decodes inside values (decode-once)
    ["100%25", "100%"], // escaped percent yields the literal
    ["gh%3Adeveloping-today%2Fcode%231", "gh:developing-today/code#1"],
    ["", ""],
  ];
  for (const [raw, want] of controls) {
    test(`control ${JSON.stringify(raw)} -> ${JSON.stringify(want)}`, () => {
      expect(decodePathSegment(raw)).toBe(want);
    });
  }
});

// Handler level: boot the real server from fixtures (the boot.test.ts
// pattern) and pin the probe matrix on both routes.
let proc: Subprocess<"ignore", "pipe", "inherit"> | null = null;
let port = 0;

beforeAll(async () => {
  port = 5100 + Math.floor(Math.random() * 1000);
  proc = spawn({
    cmd: ["bun", "run", "src/server.ts"],
    cwd: import.meta.dir + "/..",
    env: { ...Bun.env, PM_AGG_PORT: String(port), PM_AGG_SOURCE: "fixtures" },
    stdout: "pipe",
    stderr: "inherit",
  });
  const deadline = Date.now() + 8000;
  let ready = false;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/api/health`);
      if (res.ok) {
        ready = true;
        break;
      }
    } catch {
      /* not up yet */
    }
    await Bun.sleep(120);
  }
  expect(ready).toBe(true);
});

afterAll(() => {
  proc?.kill();
});

describe("malformed percent-encoding is a JSON 400 on both routes (#2747 matrix)", () => {
  const malformed = ["%", "%zz", "%E0%A4%A"];
  for (const route of ["/discuss/", "/item/"]) {
    for (const raw of malformed) {
      test(`${route}${raw} -> 400 naming the bad encoding`, async () => {
        const res = await fetch(`http://127.0.0.1:${port}${route}${raw}`);
        expect(res.status).toBe(400);
        expect(res.headers.get("content-type")).toContain("application/json");
        const body = (await res.json()) as { error: string };
        expect(body.error).toContain("malformed percent-encoding");
        expect(body.error).toContain(raw);
      });
    }
    test(`${route}plain value still routes (not a 400)`, async () => {
      const res = await fetch(`http://127.0.0.1:${port}${route}plain-ref`);
      expect(res.status).not.toBe(400);
    });
  }

  test("/item/<known board id> still resolves 200", async () => {
    const board = (await (
      await fetch(`http://127.0.0.1:${port}/api/board.json`)
    ).json()) as { board: { id: string }[] };
    const id = encodeURIComponent(board.board[0].id);
    const res = await fetch(`http://127.0.0.1:${port}/item/${id}`);
    expect(res.status).toBe(200);
  });

  test("/discuss/<encoded ref> still renders 200", async () => {
    const ref = encodeURIComponent("gh:developing-today/code#1");
    const res = await fetch(`http://127.0.0.1:${port}/discuss/${ref}`);
    expect(res.status).toBe(200);
  });
});
