// The page's own small functions, run as the browser runs them.
//
// Oracles: the elapsed-time phrases and the device labels are written
// out here by hand for real inputs (two real browser names).
import { expect, test } from "bun:test";
import {
  ago,
  androidSource,
  awayState,
  band,
  capital,
  deliveryNote,
  deviceLabel,
  deviceState,
  endpointMark,
  prCheck,
  subscriptionState,
  testSaid,
} from "../public/app.js";
import { androidRelease } from "../src/edge-proto.ts";
import { contact, endpointMark as serverMark } from "../src/push.ts";

test("elapsed time reads as a phrase on its own, never 'just now ago'", () => {
  const now = Date.parse("2026-11-22T12:00:00Z");
  const at = (seconds: number) => new Date(now - seconds * 1000).toISOString();
  expect(ago(at(0), now)).toBe("just now");
  expect(ago(at(89), now)).toBe("just now");
  expect(ago(at(90), now)).toBe("2 min ago");
  expect(ago(at(45 * 60), now)).toBe("45 min ago");
  expect(ago(at(5400), now)).toBe("2 h ago");
  expect(ago(at(26 * 3600), now)).toBe("26 h ago");
  expect(ago(at(3 * 86400), now)).toBe("3 d ago");
  // The queue store writes times without a zone: they are UTC.
  expect(ago("2026-11-22T11:00:00", now)).toBe("60 min ago");
  // A time in the future (clock skew) is now, and no time is nothing.
  expect(ago(at(-500), now)).toBe("just now");
  expect([ago("", now), ago(null, now), ago("not a time", now)]).toEqual(["", "", ""]);
});

test("a device is labelled by what it runs on, not by the engine's name", () => {
  expect(
    deviceLabel(
      "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Mobile Safari/537.36",
    ),
  ).toBe("Linux; Android 10; K");
  expect(deviceLabel("Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0")).toBe(
    "X11; Linux x86_64; rv:140.0",
  );
  expect(deviceLabel("curl/8.5.0")).toBe("curl/8.5.0");
  expect(deviceLabel(`(${"x".repeat(100)})`)).toHaveLength(60);
});

test("priority bands in words", () => {
  expect([10, 20, 30, 40, 50, 70, 90].map(band)).toEqual(["Top", "High", "Normal", "Low", "Low", "Later", "Later"]);
  expect([band(null), band(undefined), band("")]).toEqual(["Normal", "Normal", "Normal"]);
});

test("a device with notifications on tells whether the sender still has it, and for which key", async () => {
  // The app and the sender must compute the same mark, or every device
  // would look lost on every load.
  const endpoint = "https://fcm.googleapis.com/fcm/send/abc:DEF-123";
  expect(await endpointMark(endpoint)).toBe(await serverMark(endpoint));
  // Worked by hand: SHA-256 of the text, base64url, first 16 characters.
  expect(await endpointMark("a")).toBe("ypeBEsobvcr6wjGz");
  const status = { key: "KEY-NOW", known: ["mine"] };
  expect(subscriptionState("KEY-NOW", "mine", status)).toBe("keep");
  // The sender has lost it (a new store, a restore from an old copy).
  expect(subscriptionState("KEY-NOW", "other", status)).toBe("again");
  expect(subscriptionState("KEY-NOW", "mine", { key: "KEY-NOW" })).toBe("again");
  // It was made for another sender's key: the app moved from the laptop to the edge.
  expect(subscriptionState("KEY-OLD", "mine", status)).toBe("renew");
  // A browser that does not say which key it was made for is judged by the mark alone.
  expect(subscriptionState("", "mine", status)).toBe("keep");
  expect(subscriptionState("", "other", status)).toBe("again");
  // No answer worth acting on: leave it.
  expect(subscriptionState("KEY-OLD", "mine", { key: "" })).toBe("keep");
  expect(subscriptionState("KEY-OLD", "mine", null)).toBe("keep");
});

test("the contact given to push services is the project's page, never a person's address", () => {
  expect(contact("developing-today/code")).toBe("https://github.com/developing-today/code");
});

test("the Laptop away screen reads the edge's own account of itself", () => {
  // Straight from the laptop, or no answer: there is no edge to ask.
  expect(awayState(null)).toEqual({ served: false });
  expect(awayState({ ok: true })).toEqual({ served: false });
  // Served from the edge, the switch never touched.
  expect(awayState({ ok: true, edge: true, directEnabled: false, directReady: false, selfCheck: null })).toEqual({
    served: true,
    on: false,
    ready: false,
    checkedAt: null,
    checks: [],
  });
  // On and holding its tokens: each kind of read it tried, in the owner's words.
  expect(
    awayState({
      edge: true,
      directEnabled: true,
      directReady: true,
      selfCheck: {
        at: 1_700_000_000_000,
        kinds: { board: { ok: true }, asks: { ok: false, error: "queue store answered http 401" } },
      },
    }),
  ).toEqual({
    served: true,
    on: true,
    ready: true,
    checkedAt: 1_700_000_000_000,
    checks: [
      { what: "The board", ok: true, error: "" },
      { what: "Your prompts", ok: false, error: "queue store answered http 401" },
    ],
  });
  // The flag is on but a token is missing: on, and not ready.
  expect(awayState({ edge: true, directEnabled: true, directReady: false }).ready).toBe(false);
  // Ready without the flag is not ready: the flag is what the owner turned.
  expect(awayState({ edge: true, directEnabled: false, directReady: true })).toMatchObject({ on: false, ready: false });
});

test("the Phones screen says which build a download is, and who made it", () => {
  const release = { name: "2026.11.23-a3a547e", code: 21755935, built: "2026-11-23T02:00:00.000Z" };
  // A build GitHub released is what is handed out, whatever the laptop has.
  expect(androidSource({ androidBuild: { at: 1, current: false } }, { release, copy: null })).toEqual({
    by: "github",
    name: "2026.11.23-a3a547e",
    at: "2026-11-23T02:00:00.000Z",
  });
  // No release: the copy at Cloudflare is worth a word only when it is not the laptop's newest.
  const at = Date.parse("2026-11-22T11:31:06Z");
  expect(androidSource({ androidBuild: { at, current: false } }, { release: null, copy: null })).toEqual({
    by: "copy",
    at: "2026-11-22T11:31:06.000Z",
  });
  expect(androidSource({ androidBuild: { at, current: true } }, { release: null, copy: null })).toBeNull();
  // Served by the laptop itself, which cannot say where a download comes from.
  expect(androidSource({ androidApp: true }, null)).toBeNull();
  expect(androidSource(null, null)).toBeNull();
});

test("a release's description is taken only when it is one", () => {
  const good = {
    name: "2026.11.23-a3a547e",
    code: 21755935,
    commit: "a".repeat(40),
    sha256: "b".repeat(64),
    size: 445978,
    built: "2026-11-23T02:00:00Z",
    certificate: "8B:82",
  };
  expect(androidRelease(good)).toEqual({
    name: "2026.11.23-a3a547e",
    code: 21755935,
    sha256: "b".repeat(64),
    size: 445978,
    built: "2026-11-23T02:00:00.000Z",
  });
  const bad: Record<string, unknown>[] = [
    { name: "../../etc/passwd" },
    { name: "<b>x</b>" },
    { name: "" },
    { name: 7 },
    { code: 0 },
    { code: -4 },
    { code: 1.5 },
    { code: "21755935" },
    { sha256: "B".repeat(64) },
    { sha256: "b".repeat(63) },
    { size: 0 },
    { size: 33 * 1024 * 1024 },
    { size: "445978" },
    { built: "yesterday" },
    { built: 1795400000 },
  ];
  for (const change of bad) expect(androidRelease({ ...good, ...change })).toBeNull();
  for (const not of [null, undefined, "", "version", 4, [], "<html>Not Found</html>"])
    expect(androidRelease(not)).toBeNull();
});

test("a pull request's check says what GitHub said, and nothing it did not", () => {
  expect(
    prCheck({
      mergeable: true,
      mergeable_state: "clean",
      additions: 120,
      deletions: 8,
      changed_files: 5,
      review_comments: 2,
    }),
  ).toBe("can merge (clean) · +120 −8 in 5 files · 2 review comments");
  expect(
    prCheck({
      mergeable: false,
      mergeable_state: "dirty",
      additions: 1,
      deletions: 0,
      changed_files: 1,
      review_comments: 1,
    }),
  ).toBe("cannot merge (dirty) · +1 −0 in 1 file · 1 review comment");
  // GitHub works mergeability out after the fact: not known is not "cannot".
  expect(
    prCheck({
      mergeable: null,
      mergeable_state: "unknown",
      additions: 3,
      deletions: 3,
      changed_files: 2,
      review_comments: 0,
    }),
  ).toBe("mergeability not known yet · +3 −3 in 2 files · 0 review comments");
  // An answer with parts missing never reads "undefined".
  expect(prCheck({ mergeable: true })).toBe("can merge");
  expect(prCheck({})).toBe("mergeability not known yet");
  for (const nothing of [null, undefined, [], "x"]) expect(prCheck(nothing)).not.toContain("undefined");
  expect(prCheck(null)).toBe("GitHub did not say.");
});

test("a button's word starts with a capital", () => {
  expect(["start", "restart", "stop", "", "Logs"].map(capital)).toEqual(["Start", "Restart", "Stop", "", "Logs"]);
});

test("the Notifications screen says 'on' only when something would arrive here", () => {
  // The browser being subscribed is half of it. The other half is the
  // sender holding this device: without it nothing arrives, and saying
  // "On" would be untrue (a refused address left it so before).
  expect(deviceState("granted", true, "keep")).toBe("on");
  expect(deviceState("granted", true, "again")).toBe("unfinished");
  expect(deviceState("granted", true, "renew")).toBe("unfinished");
  expect(deviceState("granted", false, "keep")).toBe("off");
  expect(deviceState("default", true, "keep")).toBe("off");
  expect(deviceState("denied", true, "keep")).toBe("off");
});

test("what came of the last notification is said in plain words, and never as 'delivered'", () => {
  const NOW = Date.parse("2026-11-23T05:10:00Z");
  expect(deliveryNote(null, NOW)).toBe("Nothing has been sent yet.");
  expect(
    deliveryNote({ at: "2026-11-23T05:05:00.000Z", class: "test", sent: 1, gone: 0, failed: 0, refused: [] }, NOW),
  ).toBe("Last sent 5 min ago: a test. The push service took it for 1 device.");
  expect(
    deliveryNote(
      {
        at: "2026-11-23T03:10:00.000Z",
        class: "replies",
        sent: 2,
        gone: 1,
        failed: 1,
        refused: [{ host: "fcm.googleapis.com", status: 403 }],
      },
      NOW,
    ),
  ).toBe(
    "Last sent 2 h ago: a reply from the fleet. The push service took it for 2 devices. Not taken for 1 device: fcm.googleapis.com answered 403. 1 device no longer subscribed was forgotten.",
  );
  // A service that could not be reached at all, and a class this page does not know.
  expect(
    deliveryNote(
      {
        at: "2026-11-23T05:09:30.000Z",
        class: "later",
        sent: 0,
        gone: 2,
        failed: 2,
        refused: [{ host: "updates.push.services.mozilla.com", status: 0 }],
      },
      NOW,
    ),
  ).toBe(
    "Last sent just now: a notification. Not taken for 2 devices: updates.push.services.mozilla.com did not answer. 2 devices no longer subscribed were forgotten.",
  );
  // An older sender's record without the list of refusals still reads.
  expect(deliveryNote({ at: "2026-11-23T05:09:30.000Z", class: "needs", sent: 0, gone: 0, failed: 1 }, NOW)).toBe(
    "Last sent just now: something that needs you. Not taken for 1 device.",
  );
  expect(testSaid({ sent: 2, gone: 0, failed: 0 })).toBe("Test sent to 2 devices");
  expect(testSaid({ sent: 1, gone: 0, failed: 1 })).toBe("Test sent to 1 device; not taken for 1 device");
  expect(testSaid({ sent: 0, gone: 0, failed: 1 })).toBe("The push service did not take the test");
  expect(testSaid({ sent: 0, gone: 1, failed: 0 })).toBe("No device took the test");
});
