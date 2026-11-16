// #10609: a fetch stub installed by a test file and never restored leaks into
// every test file that runs after it in the same Bun process. server.test.ts
// used to overwrite globalThis.fetch at module scope to make any non-JWKS
// network access throw; when opencode.test.ts ran after it, its real
// http://127.0.0.1:<port>/api/session* requests hit that guard and 4
// OpenCodeClient tests failed — only under the file order that happened to
// put server.test.ts first (CI run 43807605419; the same head was green on a
// re-dispatch, and the dispatch tree was identical to a green earlier head).
//
// Bun runs test files in filename order by default, so the zz_ prefix puts
// this file after every stubbing one. Under `--randomize` the check is
// best-effort: it still fails whenever it runs after a leaked stub, which is
// the property that matters. It cannot weaken server.test.ts's own restore
// assertion (that lives in the file's afterAll).
import { describe, expect, test } from "bun:test";

describe("global fetch hygiene (#10609)", () => {
  test("globalThis.fetch is the runtime's native fetch, not a leaked test stub", () => {
    const source = Function.prototype.toString.call(globalThis.fetch);
    expect(source).toContain("[native code]");
  });
});
