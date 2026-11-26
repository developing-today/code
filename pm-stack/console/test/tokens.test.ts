// When the store tokens run out (#14255): read from the token itself,
// shown on the Platform screen, and told once per crossing.
//
// The tokens here are stand-ins built in this file: a JWT whose `exp`
// is set to a known time. That no token value ever leaves is checked
// by looking for the stand-in's own text in everything that does.
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { tokenSaid } from "../public/app.js";
import { readConfig } from "../src/config.ts";
import { changes, NOTHING_SEEN, type Seen } from "../src/notify.ts";
import { Ops, type Run, TOKEN_EXPIRY_KEPT_MS } from "../src/ops.ts";
import { storesList } from "../src/stores.ts";
import { expiryState, heldTokens, jwtExpiry, SOON_MS, type TokenExpiry } from "../src/tokens.ts";

const DAY = 86_400_000;
const NOW = Date.parse("2026-11-25T12:00:00Z");
const b64url = (s: string) => btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
/** A stand-in Turso token: a JWT with these claims. */
const standIn = (claims: Record<string, unknown>) =>
  `${b64url(JSON.stringify({ alg: "EdDSA", typ: "JWT" }))}.${b64url(JSON.stringify(claims))}.c2lnbmF0dXJlLW5vdC1jaGVja2Vk`;

test("when a token runs out is read from its exp claim, and nothing else is taken for one", () => {
  expect(jwtExpiry(standIn({ exp: 1_800_000_000, a: "rw" }))).toBe(1_800_000_000_000);
  // Surrounding blanks, as a secret is printed with a newline.
  expect(jwtExpiry(`  ${standIn({ exp: 1_800_000_000 })}\n`)).toBe(1_800_000_000_000);
  // A JWT that carries no date does not run out by itself.
  expect(jwtExpiry(standIn({ a: "rw" }))).toBeNull();
  expect(jwtExpiry(standIn({ exp: null }))).toBeNull();
  for (const [why, odd] of [
    ["not a JWT at all", "plain-secret-value"],
    ["two parts", "abc.def"],
    ["four parts", "a.b.c.d"],
    ["an empty payload", "a..c"],
    ["a payload that is not base64url", "a.b+c/d.e"],
    ["a payload that is not JSON", `a.${b64url("not json")}.c`],
    ["a payload that is a list", `a.${b64url("[1]")}.c`],
    ["an exp that is text", standIn({ exp: "soon" })],
    ["an exp of zero", standIn({ exp: 0 })],
    ["an exp below zero", standIn({ exp: -5 })],
  ] as const) {
    expect([why, jwtExpiry(odd)]).toEqual([why, undefined]);
  }
});

test("how a token stands: fine, within two weeks, run out, undated or unread", () => {
  expect(expiryState(NOW + SOON_MS + 1, NOW)).toBe("fine");
  expect(expiryState(NOW + SOON_MS, NOW)).toBe("soon");
  expect(expiryState(NOW + 1, NOW)).toBe("soon");
  expect(expiryState(NOW, NOW)).toBe("expired");
  expect(expiryState(NOW - DAY, NOW)).toBe("expired");
  expect(expiryState(null, NOW)).toBe("none");
  expect(expiryState(undefined, NOW)).toBe("unread");
});

test("the tokens the platform holds are the secrets the list of stores names, each once", () => {
  const file = JSON.parse(readFileSync(join(import.meta.dir, "..", "..", "platform", "stores.json"), "utf8")) as {
    stores: { name: string; token: string }[];
  };
  const held = heldTokens(storesList(file).stores);
  // The oracle is the file: every token that is a secret's name, and
  // none that is said in words (a Worker secret set by its deploy).
  const named = file.stores.filter((s) => /^[a-z0-9][a-z0-9._-]*$/.test(s.token));
  expect(held.map((t) => t.name)).toEqual([...new Set(named.map((s) => s.token))]);
  expect(held.length).toBeGreaterThanOrEqual(3);
  expect(held.some((t) => t.name.includes(" "))).toBe(false);
  // One token two stores use is one token, used by both.
  const row = storesList(file).stores[0];
  expect(heldTokens([row, { ...row, name: "its-twin" }, row])).toEqual([
    { name: row.token, stores: [row.name, "its-twin"] },
  ]);
});

/** An Ops whose GitHub answers with the repository's list of stores and
 * whose secrets are the stand-ins given. */
function opsWith(secrets: Record<string, string>) {
  const file = readFileSync(join(import.meta.dir, "..", "..", "platform", "stores.json"), "utf8");
  const fetched: string[] = [];
  const run: Run = async (argv) => {
    if (argv[1] === "gh" && argv[3] === "repos/developing-today/code/contents/pm-stack/platform/stores.json") {
      return {
        code: 0,
        stdout: JSON.stringify({ content: (btoa(file).match(/.{1,60}/g) ?? []).join("\n") }),
        stderr: "",
      };
    }
    if (argv[1] === "secrets" && argv[2] === "get") {
      fetched.push(argv[3]);
      const value = secrets[argv[3]];
      if (value === undefined) return { code: 1, stdout: "", stderr: `no secret ${argv[3]}` };
      return { code: 0, stdout: `${value}\n`, stderr: "" };
    }
    return { code: 1, stdout: "", stderr: "not here" };
  };
  return { ops: new Ops(readConfig({ HOME: "/h" }), run), fetched };
}

test("read through the fleet binary: each held token's date, never its value, and not fetched every minute", async () => {
  const soon = standIn({ exp: (NOW + 3 * DAY) / 1000 });
  const later = standIn({ exp: (NOW + 90 * DAY) / 1000 });
  const { ops, fetched } = opsWith({
    "turso.fleet-queue-rw-token": later,
    "turso.fleet-platform-rw-token": soon,
    // The mirror's is not there: it cannot be read.
  });
  const got = await ops.tokenExpiries(NOW);
  const byName = new Map(got.tokens.map((t) => [t.name, t]));
  expect(byName.get("turso.fleet-queue-rw-token")).toMatchObject({
    stores: ["fleet-queue"],
    state: "fine",
    expiresAt: new Date(NOW + 90 * DAY).toISOString(),
  });
  expect(byName.get("turso.fleet-platform-rw-token")).toMatchObject({ state: "soon" });
  expect(byName.get("turso.fleet-mirror-rw-token")).toEqual({
    name: "turso.fleet-mirror-rw-token",
    stores: ["fleet-mirror"],
    expiresAt: null,
    state: "unread",
  });
  // Nothing of a token leaves: not its value, nor any part of it.
  const said = JSON.stringify(got);
  for (const value of [soon, later]) for (const part of value.split(".")) expect(said).not.toContain(part);
  // Asked again within the time it is kept: no token is fetched again.
  const first = fetched.length;
  expect(first).toBe(got.tokens.length);
  await ops.tokenExpiries(NOW + 60_000);
  expect(fetched).toHaveLength(first);
  // After it: fetched again, so a rotation shows.
  await ops.tokenExpiries(NOW + TOKEN_EXPIRY_KEPT_MS);
  expect(fetched).toHaveLength(2 * first);
});

const token = (name: string, state: TokenExpiry["state"], expiresAt: string | null = "2026-12-01T00:00:00.000Z") => ({
  name,
  stores: ["fleet-platform"],
  expiresAt,
  state,
});
const primed: Seen = { ...NOTHING_SEEN, primed: true };

test("a token near its end is told once as it gets near, once as it runs out, and again only after a rotation", () => {
  const tell = (seen: Seen, tokens: TokenExpiry[] | null) => changes(seen, { tokens }, NOW);
  // Fine: nothing.
  let step = tell(primed, [token("t", "fine")]);
  expect(step.messages).toEqual([]);
  // Within two weeks: told, with its name, what uses it and the day.
  step = tell(step.seen, [token("t", "soon")]);
  expect(step.messages).toHaveLength(1);
  expect(step.messages[0]).toMatchObject({
    cls: "trouble",
    title: "A store token runs out soon",
    body: "t (fleet-platform) runs out on 2026-12-01.",
    tag: "token-t",
    url: "/console/#more/platform",
  });
  // Read again, still near: not told again, however often it is read.
  for (let i = 0; i < 3; i++) {
    step = tell(step.seen, [token("t", "soon")]);
    expect(step.messages).toEqual([]);
  }
  // Could not be read this time: that changes nothing either way.
  step = tell(step.seen, [token("t", "unread", null)]);
  expect(step.messages).toEqual([]);
  step = tell(step.seen, null);
  expect(step.messages).toEqual([]);
  // Run out: told once more.
  step = tell(step.seen, [token("t", "expired", "2026-11-24T00:00:00.000Z")]);
  expect(step.messages.map((m) => [m.title, m.body])).toEqual([
    ["A store token has run out", "t (fleet-platform) ran out on 2026-11-24."],
  ]);
  step = tell(step.seen, [token("t", "expired", "2026-11-24T00:00:00.000Z")]);
  expect(step.messages).toEqual([]);
  // Rotated: fine again, forgotten; the new one is told of in its turn.
  step = tell(step.seen, [token("t", "fine")]);
  expect(step.messages).toEqual([]);
  expect(step.seen.tokens).toEqual({});
  step = tell(step.seen, [token("t", "soon")]);
  expect(step.messages).toHaveLength(1);
});

test("a token already run out when first seen is told as run out, and not also as near", () => {
  const step = changes(primed, { tokens: [token("t", "expired")] }, NOW);
  expect(step.messages.map((m) => m.title)).toEqual(["A store token has run out"]);
  expect(changes(step.seen, { tokens: [token("t", "soon")] }, NOW).messages).toEqual([]);
});

test("turning notifications on only takes stock of tokens already near their end", () => {
  const first = changes(NOTHING_SEEN, { tokens: [token("t", "soon")] }, NOW);
  expect(first.messages).toEqual([]);
  expect(first.seen.tokens).toEqual({ t: "soon" });
  // A state kept from before tokens were watched has none of them.
  expect(changes(primed, {}, NOW).seen.tokens).toBeUndefined();
});

test("the near-expiry notification, end to end from a short-lived stand-in token", async () => {
  const shortLived = standIn({ exp: (NOW + 2 * DAY) / 1000 });
  const { ops } = opsWith({ "turso.fleet-platform-rw-token": shortLived });
  const { tokens } = await ops.tokenExpiries(NOW);
  const { messages } = changes(primed, { tokens }, NOW);
  expect(messages.map((m) => m.body)).toEqual([
    `turso.fleet-platform-rw-token (fleet-platform) runs out on ${new Date(NOW + 2 * DAY).toISOString().slice(0, 10)}.`,
  ]);
  for (const part of shortLived.split(".")) expect(JSON.stringify(messages)).not.toContain(part);
});

test("the Platform screen's line for a token", () => {
  const at = (days: number) => new Date(NOW + days * DAY).toISOString();
  expect(tokenSaid({ state: "fine", expiresAt: at(90) }, NOW)).toEqual({
    text: "Runs out on 2027-02-23.",
    tone: "good",
  });
  expect(tokenSaid({ state: "soon", expiresAt: at(3) }, NOW)).toEqual({
    text: "Runs out on 2026-11-28, in 3 days. Rotate it before then.",
    tone: "warn",
  });
  expect(tokenSaid({ state: "soon", expiresAt: at(1.5) }, NOW).text).toContain("in 1 day.");
  expect(tokenSaid({ state: "soon", expiresAt: at(0.5) }, NOW).text).toContain("in less than a day.");
  expect(tokenSaid({ state: "expired", expiresAt: at(-1) }, NOW).tone).toBe("warn");
  expect(tokenSaid({ state: "none", expiresAt: null }, NOW).text).toBe(
    "Carries no date: it does not run out by itself.",
  );
  expect(tokenSaid({ state: "unread", expiresAt: null }, NOW).tone).toBe("");
});
