import assert from "node:assert/strict";
import { test } from "node:test";
import { MemoryStore } from "../src/messages.js";
import { OWNER_COOKIE, seal } from "../src/oauth.js";
import { route } from "../src/routes.js";

const TOKEN = "o".repeat(20) + "0123456789abcdefXYZ_-";
const GPT = "g".repeat(20) + "0123456789abcdefXYZ_-";
const env = { ACCESS_TOKEN: TOKEN, CHATGPT_TOKEN: GPT };
const ORIGIN = "https://bridge.example";
const REDIRECT = "https://claude.ai/api/mcp/auth_callback";

const b64u = (buf) => Buffer.from(buf).toString("base64url");
const VERIFIER = "v".repeat(30) + "-._~0123456789abcdef";
const CHALLENGE = b64u(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(VERIFIER)));

function harness(e = env) {
  const store = new MemoryStore(() => "2026-11-20T00:00:00.000Z");
  const call = (path, init) => route(new Request(ORIGIN + path, init), e, () => store);
  return { store, call };
}

const form = (data) => ({
  method: "POST",
  headers: { "content-type": "application/x-www-form-urlencoded", origin: ORIGIN },
  body: new URLSearchParams(data).toString(),
});

async function registerClient(call, extra = {}) {
  const res = await call("/register", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ client_name: "Claude", redirect_uris: [REDIRECT], ...extra }),
  });
  assert.equal(res.status, 201);
  return res.json();
}

const authorizeQuery = (clientId, extra = {}) =>
  "/authorize?" +
  new URLSearchParams({
    response_type: "code",
    client_id: clientId,
    redirect_uri: REDIRECT,
    code_challenge: CHALLENGE,
    code_challenge_method: "S256",
    state: "st-1",
    ...extra,
  });

const hiddenReq = (html) => html.match(/name="req" value="([^"]+)"/)[1].replace(/&amp;/g, "&");

/** Runs register -> authorize (with owner cookie or pasted secret) -> token. */
async function signIn(call, { secret, cookie } = {}) {
  const client = await registerClient(call, { token_endpoint_auth_method: "none" });
  const page = await call(authorizeQuery(client.client_id), cookie ? { headers: { cookie } } : undefined);
  assert.equal(page.status, 200);
  const req = hiddenReq(await page.text());
  const post = form({ req, action: "allow", ...(secret ? { secret } : {}) });
  if (cookie) post.headers.cookie = cookie;
  const approved = await call("/authorize", post);
  assert.equal(approved.status, 302);
  const back = new URL(approved.headers.get("location"));
  assert.equal(back.origin + back.pathname, REDIRECT);
  assert.equal(back.searchParams.get("state"), "st-1");
  assert.equal(back.searchParams.get("iss"), ORIGIN);
  const tok = await call(
    "/token",
    form({
      grant_type: "authorization_code",
      code: back.searchParams.get("code"),
      redirect_uri: REDIRECT,
      client_id: client.client_id,
      code_verifier: VERIFIER,
    }),
  );
  assert.equal(tok.status, 200);
  return { client, tokens: await tok.json(), approved };
}

const mcp = (call, path, accessToken, method, params) =>
  call(path, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      accept: "application/json, text/event-stream",
      ...(accessToken ? { authorization: `Bearer ${accessToken}` } : {}),
    },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
  });

test("the bare address asks for OAuth and advertises its metadata", async () => {
  const { call } = harness();
  for (const path of ["/", "/mcp"]) {
    const res = await mcp(call, path, null, "initialize", {});
    assert.equal(res.status, 401, path);
    const suffix = path === "/mcp" ? "/mcp" : "";
    assert.equal(
      res.headers.get("www-authenticate"),
      `Bearer resource_metadata="${ORIGIN}/.well-known/oauth-protected-resource${suffix}"`,
    );
  }
  const prm = await (await call("/.well-known/oauth-protected-resource")).json();
  assert.equal(prm.resource, ORIGIN);
  assert.deepEqual(prm.authorization_servers, [ORIGIN]);
  assert.equal((await (await call("/.well-known/oauth-protected-resource/mcp")).json()).resource, `${ORIGIN}/mcp`);
  for (const p of ["/.well-known/oauth-authorization-server", "/.well-known/openid-configuration"]) {
    const as = await (await call(p)).json();
    assert.equal(as.issuer, ORIGIN);
    assert.equal(as.registration_endpoint, `${ORIGIN}/register`);
    assert.deepEqual(as.code_challenge_methods_supported, ["S256"]);
  }
  // No access token configured: nothing of this exists.
  const off = harness({});
  assert.equal((await off.call("/")).status, 404);
  assert.equal((await off.call("/.well-known/oauth-protected-resource")).status, 404);
  assert.equal((await off.call("/register", { method: "POST", body: "{}" })).status, 404);
});

test("pasting the page link approves, sets the owner cookie, and the token runs MCP on Claude's channel", async () => {
  const { call, store } = harness();
  const { tokens, approved } = await signIn(call, { secret: `https://bridge.security.cab/p/${TOKEN}?x=1` });
  assert.match(approved.headers.get("set-cookie"), new RegExp(`^${OWNER_COOKIE}=[^;]+; Path=/; .*Secure; HttpOnly; SameSite=Lax`));
  assert.equal(tokens.token_type, "Bearer");
  assert.ok(tokens.refresh_token);
  assert.doesNotMatch(JSON.stringify(tokens), new RegExp(TOKEN));

  for (const path of ["/", "/mcp"]) {
    const init = await mcp(call, path, tokens.access_token, "initialize", {
      protocolVersion: "2025-06-18",
      capabilities: {},
      clientInfo: { name: "t", version: "1" },
    });
    assert.equal(init.status, 200, path);
    assert.match((await init.json()).result.instructions, /Claude/);
  }
  const sent = await mcp(call, "/", tokens.access_token, "tools/call", {
    name: "send_message",
    arguments: { body: "hello over oauth" },
  });
  assert.equal(sent.status, 200);
  const { messages } = await store.list({ direction: "all" });
  assert.equal(messages.at(-1).body, "hello over oauth");
  assert.equal(messages.at(-1).channel, "claude");

  // Browser-based clients can read authenticated answers.
  assert.equal(sent.headers.get("access-control-allow-origin"), "*");
  // GET on the authorized address is the usual 405 (no server stream).
  assert.equal((await call("/", { headers: { authorization: `Bearer ${tokens.access_token}` } })).status, 405);
  // A forged or wrong-key token is refused.
  const bad = await mcp(call, "/", `${tokens.access_token}x`, "ping", {});
  assert.equal(bad.status, 401);
  assert.match(bad.headers.get("www-authenticate"), /error="invalid_token"/);
  const otherKey = harness({ ACCESS_TOKEN: `${TOKEN}rotated` });
  assert.equal((await mcp(otherKey.call, "/", tokens.access_token, "ping", {})).status, 401, "rotating the token revokes");
});

test("a browser that opened the history page approves with one tap", async () => {
  const { call } = harness();
  const page = await call(`/p/${TOKEN}`);
  assert.equal(page.status, 200);
  const cookie = page.headers.get("set-cookie").split(";")[0];
  const firstPage = await call(authorizeQuery((await registerClient(call)).client_id), { headers: { cookie } });
  const html = await firstPage.text();
  assert.match(html, /signed in as the bridge's owner/);
  assert.doesNotMatch(html, /name="secret"/);
  assert.match(firstPage.headers.get("content-security-policy"), /form-action 'self' https:\/\/claude\.ai/);
  const { tokens } = await signIn(call, { cookie });
  assert.equal((await mcp(call, "/", tokens.access_token, "ping", {})).status, 200);
  // A cookie signed for another typ doesn't count.
  const fake = `${OWNER_COOKIE}=${await seal(env, { typ: "at", ch: "claude", exp: 9e9 })}`;
  const noTap = await (await call(authorizeQuery((await registerClient(call)).client_id), { headers: { cookie: fake } })).text();
  assert.match(noTap, /name="secret"/);
});

test("the ChatGPT token approves ChatGPT's channel only", async () => {
  const { call } = harness();
  const { tokens, approved } = await signIn(call, { secret: GPT });
  assert.equal(approved.headers.get("set-cookie"), null);
  const init = await mcp(call, "/", tokens.access_token, "initialize", {
    protocolVersion: "2025-06-18",
    capabilities: {},
    clientInfo: { name: "t", version: "1" },
  });
  assert.match((await init.json()).result.instructions, /ChatGPT/);
  // It still opens nothing but MCP.
  assert.equal((await call(`/p/${GPT}`)).status, 404);
});

test("refresh tokens rotate and stay bound to their client", async () => {
  const { call } = harness();
  const { client, tokens } = await signIn(call, { secret: TOKEN });
  const refreshed = await call("/token", form({ grant_type: "refresh_token", refresh_token: tokens.refresh_token, client_id: client.client_id }));
  assert.equal(refreshed.status, 200);
  const next = await refreshed.json();
  assert.equal((await mcp(call, "/", next.access_token, "ping", {})).status, 200);
  const other = await registerClient(call, { token_endpoint_auth_method: "none", client_name: "other" });
  const stolen = await call("/token", form({ grant_type: "refresh_token", refresh_token: tokens.refresh_token, client_id: other.client_id }));
  assert.equal(stolen.status, 400);
  assert.equal((await stolen.json()).error, "invalid_grant");
  // Rotation: the used refresh token is dead, and replaying it revokes
  // the whole family, the newest refresh token included.
  const replay = await call("/token", form({ grant_type: "refresh_token", refresh_token: tokens.refresh_token, client_id: client.client_id }));
  assert.match((await replay.json()).error_description, /already used/);
  const afterReplay = await call("/token", form({ grant_type: "refresh_token", refresh_token: next.refresh_token, client_id: client.client_id }));
  assert.equal(afterReplay.status, 400, "replay revokes the family");
  // An access token is not a refresh token.
  const swapped = await call("/token", form({ grant_type: "refresh_token", refresh_token: tokens.access_token, client_id: client.client_id }));
  assert.equal(swapped.status, 400);
});

test("approval refuses wrong secrets, foreign origins, bad PKCE and unregistered redirects", async () => {
  const { call } = harness();
  const client = await registerClient(call, { token_endpoint_auth_method: "none" });
  const req = hiddenReq(await (await call(authorizeQuery(client.client_id))).text());

  const wrong = await call("/authorize", form({ req, action: "allow", secret: "not-the-token" }));
  assert.equal(wrong.status, 401);
  assert.match(await wrong.text(), /isn&#39;t this bridge/);
  const noProof = await call("/authorize", form({ req, action: "allow" }));
  assert.equal(noProof.status, 401);

  const crossSite = form({ req, action: "allow", secret: TOKEN });
  crossSite.headers.origin = "https://evil.example";
  assert.equal((await call("/authorize", crossSite)).status, 403);

  const denied = await call("/authorize", form({ req, action: "deny" }));
  assert.equal(denied.status, 302);
  assert.equal(new URL(denied.headers.get("location")).searchParams.get("error"), "access_denied");

  // Redirect not registered: an error page, never a redirect.
  const elsewhere = await call(authorizeQuery(client.client_id, { redirect_uri: "https://evil.example/cb" }));
  assert.equal(elsewhere.status, 400);
  assert.equal(elsewhere.headers.get("location"), null);
  // A made-up client id.
  assert.equal((await call(authorizeQuery("forged.client"))).status, 400);
  // PKCE is required, S256 only.
  const plain = await call(authorizeQuery(client.client_id, { code_challenge_method: "plain" }));
  assert.equal(new URL(plain.headers.get("location")).searchParams.get("error"), "invalid_request");

  // Wrong verifier: the code is useless.
  const ok = await call("/authorize", form({ req, action: "allow", secret: TOKEN }));
  const code = new URL(ok.headers.get("location")).searchParams.get("code");
  const badVerifier = await call(
    "/token",
    form({ grant_type: "authorization_code", code, redirect_uri: REDIRECT, client_id: client.client_id, code_verifier: "w".repeat(50) }),
  );
  assert.equal((await badVerifier.json()).error, "invalid_grant");
  // Another client can't redeem it.
  const other = await registerClient(call, { token_endpoint_auth_method: "none", client_name: "other" });
  const theft = await call(
    "/token",
    form({ grant_type: "authorization_code", code, redirect_uri: REDIRECT, client_id: other.client_id, code_verifier: VERIFIER }),
  );
  assert.equal((await theft.json()).error, "invalid_grant");
  // Codes are single-use.
  const first = await call(
    "/token",
    form({ grant_type: "authorization_code", code, redirect_uri: REDIRECT, client_id: client.client_id, code_verifier: VERIFIER }),
  );
  assert.equal(first.status, 200);
  const second = await call(
    "/token",
    form({ grant_type: "authorization_code", code, redirect_uri: REDIRECT, client_id: client.client_id, code_verifier: VERIFIER }),
  );
  assert.match((await second.json()).error_description, /already used/);

  // Registration only takes https (or localhost) redirects.
  const http = await call("/register", {
    method: "POST",
    body: JSON.stringify({ redirect_uris: ["http://evil.example/cb"] }),
  });
  assert.equal(http.status, 400);
});

test("confidential clients must present their secret", async () => {
  const { call } = harness();
  const client = await registerClient(call); // no method given: client_secret_post, basic accepted too
  assert.equal(client.token_endpoint_auth_method, "client_secret_post");
  assert.ok(client.client_secret);
  const twin = await registerClient(call);
  assert.notEqual(twin.client_id, client.client_id, "identical metadata still gets its own client");
  assert.notEqual(twin.client_secret, client.client_secret);
  const newCode = async () => {
    const req = hiddenReq(await (await call(authorizeQuery(client.client_id))).text());
    const ok = await call("/authorize", form({ req, action: "allow", secret: TOKEN }));
    return new URL(ok.headers.get("location")).searchParams.get("code");
  };
  const body = { grant_type: "authorization_code", code: await newCode(), redirect_uri: REDIRECT, code_verifier: VERIFIER };
  const noSecret = await call("/token", form({ ...body, client_id: client.client_id }));
  assert.equal(noSecret.status, 401);
  const basic = Buffer.from(`${encodeURIComponent(client.client_id)}:${encodeURIComponent(client.client_secret)}`).toString("base64");
  const withBasic = await call("/token", { ...form(body), headers: { ...form(body).headers, authorization: `Basic ${basic}` } });
  assert.equal(withBasic.status, 200);
  const withPost = await call(
    "/token",
    form({ ...body, code: await newCode(), client_id: client.client_id, client_secret: client.client_secret }),
  );
  assert.equal(withPost.status, 200);
});

test("client ids are short, stored, and sign-ins leave a secret-free trail for the owner", async () => {
  const { call } = harness();
  const { client, tokens } = await signIn(call, { secret: `${ORIGIN}/p/${TOKEN}` });
  assert.match(client.client_id, /^mbc_[A-Za-z0-9_-]{22}$/);
  assert.ok(tokens.access_token.length < 200, "tokens stay compact");
  // A made-up or old-style (signed blob) client id is unknown.
  for (const id of ["mbc_" + "A".repeat(22), "eyJ0eXAiOiJjbGllbnQifQ.abc"]) {
    const page = await call(authorizeQuery(id));
    assert.equal(page.status, 400);
    assert.match(await page.text(), /unknown app/);
  }
  const res = await call(`/api/${TOKEN}/oauth-events`);
  assert.equal(res.status, 200);
  const { events } = await res.json();
  const kinds = events.map((e) => e.kind);
  for (const k of ["registered", "approval_shown", "approved", "token_issued", "authorize_refused"]) assert.ok(kinds.includes(k), k);
  const reg = events.find((e) => e.kind === "registered");
  assert.equal(reg.method, "none");
  assert.deepEqual(reg.redirects, [REDIRECT]);
  const dump = JSON.stringify(events);
  for (const secret of [TOKEN, client.client_id, tokens.access_token, tokens.refresh_token]) assert.ok(!dump.includes(secret));
  // Only the owner sees it.
  assert.equal((await call(`/api/${GPT}/oauth-events`)).status, 404);
});

test("a full registry refuses new clients instead of growing", async () => {
  const { call, store } = harness();
  for (let i = 0; i < 500; i++) await store.oauthClientAdd(`mbc_${String(i).padStart(22, "0")}`, { r: [REDIRECT], m: "none", n: "" });
  const res = await call("/register", { method: "POST", body: JSON.stringify({ redirect_uris: [REDIRECT] }) });
  assert.equal(res.status, 429);
});
