// The page's own small functions, run as the browser runs them.
//
// Oracles: the elapsed-time phrases and the device labels are written
// out here by hand for real inputs (two real browser names).
import { expect, test } from "bun:test";
import {
  ago,
  androidSource,
  awayNotice,
  awayState,
  band,
  capital,
  changeAsked,
  deliveryNote,
  deviceLabel,
  deviceState,
  endpointMark,
  intakeArgs,
  intakeSaid,
  levelHeadings,
  phaseHeading,
  prCheck,
  runSaid,
  servingSaid,
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

test("the Cloudflare screen reads the edge's own account of itself", () => {
  // Straight from the laptop, or no answer: there is no edge to ask.
  expect(awayState(null)).toEqual({ served: false });
  expect(awayState({ ok: true })).toEqual({ served: false });
  // Served from the edge, the switch never touched.
  expect(awayState({ ok: true, edge: true, directEnabled: false, directReady: false, selfCheck: null })).toEqual({
    served: true,
    on: false,
    ready: false,
    lacks: [],
    refused: [],
    checkedAt: null,
    checks: [],
  });
  // On and holding its tokens: each kind of read it tried, in the owner's words.
  expect(
    awayState({
      edge: true,
      configured: true,
      directEnabled: true,
      githubTokenConfigured: true,
      queueTokenConfigured: true,
      primary: true,
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
    lacks: [],
    refused: [],
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

test("a card's title is not announced as a skipped heading level", () => {
  // The screen's title is level one. Card titles are h3: fine under a
  // section's h2, a skipped level when the screen has no section above them.
  const heading = (tagName: string) => {
    const attrs: Record<string, string> = {};
    return { tagName, attrs, setAttribute: (k: string, v: string) => void (attrs[k] = v) };
  };
  const levels = (tags: string[]) => {
    const els = tags.map(heading);
    levelHeadings(els);
    return els.map((e) => e.attrs["aria-level"] ?? null);
  };
  // No sections: every card title is said to be level two.
  expect(levels(["H3", "H3"])).toEqual(["2", "2"]);
  // Cards before the first section are level two; under a section they stay level three.
  expect(levels(["H3", "H2", "H3", "H3", "H2", "H3"])).toEqual(["2", null, null, null, null, null]);
  // Sections first: nothing is changed.
  expect(levels(["H2", "H3"])).toEqual([null, null]);
  expect(levels([])).toEqual([]);
});

test("a deploy or build is told in a line, and what is being served in a sentence", () => {
  const NOW = Date.parse("2026-11-23T06:00:00Z");
  const run = { state: "success", at: "2026-11-23T05:51:40Z", commit: "ca5c04493289", byHand: false };
  expect(runSaid(run, NOW)).toBe("Done · 8 min ago · ca5c044");
  expect(runSaid({ ...run, state: "in_progress", byHand: true, at: "2026-11-23T05:59:30Z" }, NOW)).toBe(
    "Running now · just now · ca5c044 · started by hand",
  );
  expect(runSaid({ ...run, state: "failure" }, NOW)).toBe("Failed · 8 min ago · ca5c044");
  // A state this page was never told of is still said, in words.
  expect(runSaid({ ...run, state: "action_required" }, NOW)).toBe("Action required · 8 min ago · ca5c044");
  expect(runSaid({ state: null, at: null, commit: null }, NOW)).toBe("Unknown");

  expect(servingSaid({ edge: true, version: "ca5c04493289", hostVersion: "ca5c04493289" })).toBe(
    "Cloudflare and the laptop are both on ca5c044.",
  );
  expect(servingSaid({ edge: true, version: "ca5c04493289", hostVersion: "326ac803bedd" })).toBe(
    "Cloudflare is serving ca5c044; the laptop runs 326ac80. After a merge the laptop follows within a minute or two.",
  );
  expect(servingSaid({ edge: true, version: "ca5c04493289", hostVersion: null })).toBe(
    "Cloudflare is serving ca5c044. The laptop has not said what it runs.",
  );
  // Not behind the edge at all: said plainly, never guessed.
  for (const health of [null, {}, { edge: false, version: "x" }, { edge: true }]) {
    expect(servingSaid(health)).toBe("This is served straight from the laptop.");
  }
});

test("what a form on a plan screen sends is the owner's words and only the choices made", () => {
  const WORDS = "do this  first —\n“before” the rest";
  // A note on a roadmap row, rank left alone, no session.
  expect(
    intakeArgs(
      { kind: "note", area: "id", about: { row: "Native TUI client", phase: "later" } },
      { text: WORDS, rank: "keep", session: false },
    ),
  ).toEqual({ kind: "note", area: "id", about: { row: "Native TUI client", phase: "later" }, text: WORDS });
  // A rank chosen, and a session asked for.
  expect(intakeArgs({ kind: "note", about: { issue: 557 } }, { text: "x", rank: "expedite", session: true })).toEqual({
    kind: "note",
    about: { issue: 557 },
    text: "x",
    rank: "expedite",
    mode: "session",
  });
  // "No rank" is a choice, and is sent.
  expect(intakeArgs({ kind: "note" }, { text: "x", rank: "none" }).rank).toBe("none");
  // The form's own capability is used; none chosen sends none, even
  // when what it started from had an empty one.
  expect(intakeArgs({ kind: "bug" }, { text: "x", area: "fleet", rank: "now" })).toEqual({
    kind: "bug",
    text: "x",
    area: "fleet",
    rank: "now",
  });
  expect(intakeArgs({ kind: "bug" }, { text: "x", area: "" })).toEqual({ kind: "bug", text: "x" });
  expect("area" in intakeArgs({ kind: "note", area: null, about: { issue: 3 } }, { text: "x" })).toBe(false);
  // What it started from is not changed.
  const base = { kind: "note", area: "id" };
  intakeArgs(base, { text: "x", rank: "now", session: true });
  expect(base).toEqual({ kind: "note", area: "id" });
});

test("a change form names a phase only when moving, and a name only when adding under", () => {
  expect(changeAsked({ action: "move", to: "now", itemTitle: "left over from before" })).toEqual({
    action: "move",
    to: "now",
  });
  expect(changeAsked({ action: "add-under", to: "now", itemTitle: "Tiles" })).toEqual({
    action: "add-under",
    title: "Tiles",
  });
  expect(changeAsked({ action: "add-under", to: "now", itemTitle: "" })).toEqual({ action: "add-under" });
  for (const action of ["built", "rewrite", "drop"]) {
    expect(changeAsked({ action, to: "next", itemTitle: "x" })).toEqual({ action });
  }
});

test("what was sent is said in a line: where it went, and whether its task is still to come", () => {
  expect(intakeSaid({ issue: 13001, commented: false, queued: true })).toBe("Filed as #13001");
  expect(intakeSaid({ issue: 557, commented: true, queued: true })).toBe("Note left on #557");
  expect(intakeSaid({ issue: 13001, commented: false, queued: false })).toBe(
    "Filed as #13001; its task follows when a machine is connected",
  );
  expect(intakeSaid({ issue: 557, commented: true, queued: false })).toBe(
    "Note left on #557; its task follows when a machine is connected",
  );
  expect(intakeSaid(undefined)).toBe("Sent");
});

test("a phase's heading says how many it shows, and of how many when there are more", () => {
  expect(phaseHeading("In progress", 0, 0)).toBe("In progress");
  expect(phaseHeading("In progress", 3, 0)).toBe("In progress (3)");
  expect(phaseHeading("Built", 30, 516, 14)).toBe("Built (30 of 546 in the last 14 days)");
  expect(phaseHeading("Open", 100, 12)).toBe("Open (100 of 112)");
  expect(phaseHeading("Built", 0, 5, 14)).toBe("Built (0 of 5 in the last 14 days)");
});

test("with the switch on and something missing, the Cloudflare screen says what by name", () => {
  const base = {
    edge: true,
    configured: true,
    directEnabled: true,
    githubTokenConfigured: true,
    queueTokenConfigured: true,
  };
  // Everything there, and the edge says it does the core: nothing lacking.
  expect(awayState({ ...base, primary: true }).lacks).toEqual([]);
  expect(awayState({ ...base, primary: true }).ready).toBe(true);
  // An edge from before `primary` says the same under its older name.
  expect(awayState({ ...base, directReady: true }).ready).toBe(true);
  // One token missing, the other, and a machine that has never reported.
  expect(awayState({ ...base, queueTokenConfigured: false, primary: false })).toMatchObject({
    ready: false,
    lacks: ["the queue store token"],
  });
  expect(awayState({ ...base, githubTokenConfigured: false, queueTokenConfigured: false }).lacks).toEqual([
    "the repository token",
    "the queue store token",
  ]);
  expect(awayState({ ...base, configured: false }).lacks).toEqual([
    "a machine's first report, which says where things are",
  ]);
  // With the switch off nothing is lacking: nothing was asked for.
  expect(awayState({ edge: true, directEnabled: false, primary: true })).toMatchObject({
    on: false,
    ready: false,
    lacks: [],
  });
});

test("what Cloudflare's token was refused for is said in the owner's words", () => {
  const base = { edge: true, configured: true, directEnabled: true, primary: true };
  expect(awayState({ ...base, refused: ["pr.merge", "deploy.start"] }).refused).toEqual([
    "merging",
    "starting a deploy",
  ]);
  // One this app does not know by name is still said, as it came.
  expect(awayState({ ...base, refused: ["something.new"] }).refused).toEqual(["something.new"]);
  // What it could not read is said the same way.
  expect(awayState({ ...base, refused: ["read.board", "read.prs", "read.priorities", "read.asks"] }).refused).toEqual([
    "reading the board",
    "reading the pull requests",
    "reading the ranked issues",
    "reading your prompts",
  ]);
  // An edge that says nothing of it, or something that is not a list, has refused nothing.
  expect(awayState(base).refused).toEqual([]);
  expect(awayState({ ...base, refused: "pr.merge" }).refused).toEqual([]);
});

test("the laptop being away is said differently when Cloudflare is carrying on", () => {
  const carrying = awayNotice(true, "2 h ago");
  expect(carrying.title).toBe("The laptop is not connected");
  expect(carrying.text).toContain("Cloudflare is carrying on");
  expect(carrying.text).toContain("as the laptop last reported them, 2 h ago");
  const waiting = awayNotice(false, "2 h ago");
  expect(waiting.title).toBe("The laptop is out of reach");
  expect(waiting.text).toBe(
    "It last reported 2 h ago; this is the state as of then. You can keep working: what is safe to do late is held and sent when it is back.",
  );
});
