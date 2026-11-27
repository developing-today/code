import assert from "node:assert/strict";
import test from "node:test";
import { main, parseArgs, INVALID_RP_TOKEN } from "../smoke/live-identity.mjs";

const token = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJwcm9iZSJ9.AA";
const health = { status: "ok", service: "fleet-hosted", version: "0.1.0", auth: { rp_configured: true, tenancy_configured: true } };
const capabilities = { build: { name: "fleet-hosted", invoked_as: "fleet-hosted", profile: "hosted", edition: "hosted", version: "0.1.0" } };
function harness(replies, env = { FLEET_HOSTED_RP_TOKEN: token }) {
  const calls = [], output = [];
  return {
    calls, output,
    run: (args = []) => main(args, {
      env, output: (line) => output.push(line),
      fetchImpl: async (url, options) => {
        calls.push({ url, options });
        const reply = replies[calls.length - 1];
        assert.ok(reply, "oracle: only the five declared read-only requests may run");
        return new Response(typeof reply[1] === "string" ? reply[1] : JSON.stringify(reply[1]), { status: reply[0] });
      },
    }),
  };
}
const success = () => [[200, health], [200, capabilities], [401, { error: "invalid_token" }], [200, { ok: true }], [401, { error: "missing_token" }]];

test("oracle: golden hosted identity and auth transcript is the only complete proof", async () => {
  const h = harness(success());
  assert.equal(await h.run(), 0);
  assert.equal(JSON.parse(h.output[0]).result, "passed");
  assert.deepEqual(h.calls.map((c) => [new URL(c.url).pathname, c.options.method ?? "GET"]), [
    ["/health", "GET"], ["/capabilities", "GET"], ["/v1/queue/health", "POST"], ["/v1/queue/health", "POST"], ["/v1/queue/health", "POST"],
  ]);
  for (const call of h.calls) assert.equal(call.options.redirect, "error");
  assert.equal(h.calls[2].options.headers.authorization, `Bearer ${INVALID_RP_TOKEN}`);
  assert.equal(h.calls[3].options.headers.authorization, `Bearer ${token}`);
  assert.equal(h.calls[3].options.body, "{}");
  assert.equal(h.calls[4].options.headers.authorization, undefined);
  assert.ok(!h.output.join("").includes(token), "oracle: reports never contain credentials");
});

test("oracle: strict argument grammar rejects every unknown/duplicate/valueless form before fetch", async () => {
  for (const args of [["--typo"], ["stray"], ["--token", token], ["--help", "--typo"], ["--help=true"], ["--base-url"], ["--base-url="], ["--base-url", "--help"], ["--help", "--help"], ["--base-url=x", "--base-url=y"]]) {
    const h = harness([]);
    assert.equal(await h.run(args), 2, JSON.stringify(args));
    assert.equal(h.calls.length, 0);
  }
});

test("oracle: HTTPS-origin invariant and equivalent value syntaxes", async () => {
  assert.deepEqual(parseArgs(["--base-url", "https://example.test"]), parseArgs(["--base-url=https://example.test/"]));
  for (const value of ["http://example.test", "https://user:pass@example.test", "https://example.test/path", "https://example.test/?x=1", "https://example.test/#fragment", "nonsense"]) {
    const h = harness([]);
    assert.equal(await h.run(["--base-url", value]), 2);
    assert.equal(h.calls.length, 0);
  }
  const h = harness([]);
  assert.equal(await h.run(["--help"]), 0);
  assert.equal(h.calls.length, 0);
});

test("oracle: absent valid token cannot pass even when public identity and rejection pass", async () => {
  const h = harness([...success().slice(0, 3), success()[4]], {});
  assert.equal(await h.run(), 1);
  assert.equal(h.calls.length, 4);
  assert.equal(JSON.parse(h.output[0]).checks.find((c) => c.name === "valid RP accepted").error, "missing_rp_token");
});

test("oracle: opaque or malformed environment credentials cause usage failure before network", async () => {
  for (const credential of ["opaque-service-token", "a.b.", "a.b.c\r\nAuthorization: x"]) {
    const h = harness([], { FLEET_HOSTED_RP_TOKEN: credential });
    assert.equal(await h.run(), 2);
    assert.equal(h.calls.length, 0);
    assert.ok(!h.output.join("").includes(credential));
  }
});

test("oracle: auth_not_configured is incomplete, never invalid-token or valid-token evidence", async () => {
  const replies = success();
  replies[2] = replies[3] = [503, { error: "auth_not_configured" }];
  const h = harness(replies);
  assert.equal(await h.run(), 1);
  for (const check of JSON.parse(h.output[0]).checks.slice(2, 4)) {
    assert.equal(check.result, "incomplete");
    assert.equal(check.error, "auth_not_configured");
  }
});

test("oracle: wrong hosted identity cannot pass despite successful auth transcript", async () => {
  for (const [index, body] of [[0, { ...health, service: "other" }], [1, { build: { ...capabilities.build, edition: "native" } }], [1, { build: { ...capabilities.build, profile: "native" } }], [0, { ...health, version: "" }]]) {
    const replies = success(); replies[index] = [200, body];
    assert.equal(await harness(replies).run(), 1);
  }
});

test("oracle: health and capabilities package versions must agree and are reported", async () => {
  const good = harness(success());
  assert.equal(await good.run(), 0);
  assert.equal(JSON.parse(good.output[0]).checks[0].version, "0.1.0");
  const replies = success();
  replies[1] = [200, { build: { ...capabilities.build, version: "0.2.0" } }];
  const bad = harness(replies);
  assert.equal(await bad.run(), 1);
  assert.ok(JSON.parse(bad.output[0]).checks.some((c) => c.error === "version_mismatch"));
});

test("oracle: rejection must be 401 invalid_token, not accepted/forbidden/unavailable/missing-token", async () => {
  for (const reply of [[200, { ok: true }], [403, { error: "invalid_token" }], [503, { error: "jwks_unavailable" }], [401, { error: "missing_token" }]]) {
    const replies = success(); replies[2] = reply;
    assert.equal(await harness(replies).run(), 1);
  }
});

test("oracle: valid caller requires successful broker envelope, not HTTP 200 alone", async () => {
  for (const reply of [[401, { error: "invalid_token" }], [403, { error: "unknown_subject" }], [503, { error: "queue_store_not_configured" }], [200, { ok: false }], [200, {}]]) {
    const replies = success(); replies[3] = reply;
    assert.equal(await harness(replies).run(), 1);
  }
});

test("oracle: explicit missing auth configuration overrides otherwise passing endpoint transcript", async () => {
  for (const flag of ["rp_configured", "tenancy_configured"]) {
    const replies = success();
    replies[0] = [200, { ...health, auth: { ...health.auth, [flag]: false } }];
    assert.equal(await harness(replies).run(), 1);
  }
});

test("oracle: absent credentials require missing_token refusal, not anonymous access", async () => {
  for (const reply of [[200, { ok: true }], [401, { error: "invalid_token" }], [503, { error: "auth_not_configured" }]]) {
    const replies = success(); replies[4] = reply;
    assert.equal(await harness(replies).run(), 1);
  }
});

test("oracle: WAF HTML and transport failure are incomplete proof, not RP rejection", async () => {
  const replies = success(); replies[2] = [403, "<html>Cloudflare 1010</html>"];
  assert.equal(await harness(replies).run(), 1);
  const lines = [];
  assert.equal(await main([], { env: {}, output: (s) => lines.push(s), fetchImpl: async () => { throw new Error(token); } }), 1);
  assert.ok(!lines.join("").includes(token));
});
