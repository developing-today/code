// The page's own small functions, run as the browser runs them.
//
// Oracles: the elapsed-time phrases and the device labels are written
// out here by hand for real inputs (two real browser names).
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  plainWords,
  ASK_KINDS,
  ago,
  askArgs,
  askKind,
  askSent,
  androidSource,
  answeredKeys,
  awayNotice,
  censusSaid,
  differsSaid,
  doneKindsSaid,
  doneSaid,
  doneTaskSaid,
  laneSaid,
  mergedBySaid,
  acceptAllSaid,
  applySaid,
  awayState,
  blockedSaid,
  band,
  capital,
  changeAsked,
  deliveryNote,
  deviceLabel,
  deviceState,
  endpointMark,
  enrollCommand,
  everyRead,
  unmetered,
  flushSaid,
  freshSaid,
  intakeArgs,
  intakeSaid,
  KEEP_ALL_EVERY_MS,
  keepAllDue,
  keptGroups,
  keptSaid,
  heldWaitsSaid,
  levelHeadings,
  listed,
  liveDelay,
  liveMessage,
  liveUrl,
  machineSaid,
  needsLead,
  recommendedSets,
  machinesMark,
  reposTyped,
  settingSaid,
  settingsTakenSaid,
  matchReportSaid,
  mergeMatchReport,
  offerSaid,
  phaseHeading,
  platformCounts,
  prCheck,
  runSaid,
  sharedGoal,
  sharedGoalSaid,
  sentSaid,
  servingSaid,
  subscriptionState,
  taskAgent,
  testSaid,
  unitStateSaid,
  whyShown,
  unreadSaid,
  whySaid,
  workerSaid,
} from "../public/app.js";
import { DIRECT_ACTIONS, DIRECT_READ_SECTIONS } from "../edge/src/direct.ts";
import { QUEUE_WHY_CHARS } from "../src/ops.ts";
import { ASK_INTENTS } from "../src/work.ts";
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

test("the Cloudflare screen says where its configuration was last applied from (#13495)", () => {
  const now = Date.parse("2026-11-25T07:00:00Z");
  // Fixture provenance, as the cloudflare-apply read returns it: the
  // sentence names the source, the short commit and when it finished.
  expect(
    applySaid(
      {
        source: "pm-console-cloudflare config, dispatched by dezren39",
        commit: "c58cf1540b1488c9c642e8b424eeed3471e62740",
        at: "2026-11-25T04:00:00Z",
        url: "https://github.com/developing-today/code/actions/runs/3",
      },
      now,
    ),
  ).toBe("Last applied from pm-console-cloudflare config, dispatched by dezren39, at commit c58cf1540b14, 3 h ago.");
  // No successful apply on record: the screen says provenance is
  // unavailable, and never presents a failed or running one instead.
  expect(applySaid(null, now)).toBe("Apply provenance is unavailable: no successful apply is on record.");
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
  const levels = (tags: string[]): (string | null)[] => {
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
  expect(
    changeAsked({ action: "move", to: "now", itemTitle: "left over from before" }) as {
      action: string;
      to?: string;
      title?: string;
    },
  ).toEqual({
    action: "move",
    to: "now",
  });
  expect(
    changeAsked({ action: "add-under", to: "now", itemTitle: "Tiles" }) as {
      action: string;
      to?: string;
      title?: string;
    },
  ).toEqual({
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
    "a machine's first report, which says who may sign in",
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
  expect(carrying.text).toContain("what is labelled as needing you are current");
  expect(carrying.text).toContain("Sessions and questions are as the laptop last reported them, 2 h ago");
  const waiting = awayNotice(false, "2 h ago");
  expect(waiting.title).toBe("The laptop is out of reach");
  expect(waiting.text).toBe(
    "It last reported 2 h ago; this is the state as of then. You can keep working: what is safe to do late is held and sent when it is back.",
  );
});

test("lists of what needs the owner that are not whole say so, and name what could not be read when that is known", () => {
  const lists = { hands: [], reviews: [], other: [] };
  // Whole: nothing is said, with or without the word.
  expect(censusSaid({ ...lists, complete: true, unread: [] })).toBe("");
  expect(censusSaid(lists)).toBe("");
  expect(censusSaid(null)).toBe("");
  // Not whole, and the reader names what is missing.
  expect(censusSaid({ ...lists, complete: false, unread: ["a/one"] })).toBe(
    "Not the whole list: a/one could not be read just now.",
  );
  expect(censusSaid({ ...lists, complete: false, unread: ["a/one", "a/two (user/review)", "a/three"] })).toBe(
    "Not the whole list: a/one, a/two (user/review) and a/three could not be read just now.",
  );
  // Not whole, from a reader that does not say what is missing (the laptop's dashboard).
  for (const unread of [undefined, [], [""], "a/one", [3]]) {
    expect([unread, censusSaid({ ...lists, complete: false, unread })]).toEqual([
      unread,
      "Not the whole list: some of it could not be read just now.",
    ]);
  }
});

test("text on a status colour can be read, on the dark page and the light one", () => {
  // The oracle is the WCAG 2 contrast formula, written out here; 4.5 to
  // 1 is what it asks of text this size. (The toasts and the danger
  // button are not on screen when the screens are scanned, which is how
  // three of these went unread for a while.)
  const css = readFileSync(new URL("../public/app.css", import.meta.url), "utf8");
  const block = (from: number) => css.slice(css.indexOf("{", from) + 1, css.indexOf("}", from));
  const vars = (text: string) =>
    Object.fromEntries([...text.matchAll(/(--[a-z0-9-]+):\s*(#[0-9a-fA-F]{6})\s*;/g)].map((m) => [m[1], m[2]]));
  const dark = vars(block(css.indexOf(":root")));
  const lightAt = css.indexOf("@media (prefers-color-scheme: light)");
  expect(lightAt).toBeGreaterThan(0);
  const light = { ...dark, ...vars(block(css.indexOf(":root", lightAt))) };
  const luminance = (hex: string) => {
    const channel = (i: number) => {
      const c = Number.parseInt(hex.slice(1 + 2 * i, 3 + 2 * i), 16) / 255;
      return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * channel(0) + 0.7152 * channel(1) + 0.0722 * channel(2);
  };
  const contrast = (a: string, b: string) => {
    const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
  };
  // The formula itself, on the two pairs everybody knows.
  expect(contrast("#000000", "#ffffff")).toBeCloseTo(21, 5);
  expect(contrast("#777777", "#ffffff")).toBeCloseTo(4.48, 2);
  for (const [name, theme] of [
    ["dark", dark],
    ["light", light],
  ] as const) {
    for (const [ink, on] of [
      ["--accent-ink", "--accent"],
      ["--good-ink", "--good"],
      ["--bad-ink", "--bad"],
      ["--text", "--bg"],
      ["--text", "--panel"],
      ["--quiet", "--panel"],
      ["--good", "--panel"],
      ["--warn", "--panel"],
      ["--bad", "--panel"],
    ]) {
      const ratio = contrast(theme[ink], theme[on]);
      expect(`${name} ${ink} on ${on}: ${ratio >= 4.5 ? "readable" : ratio.toFixed(2)}`).toBe(
        `${name} ${ink} on ${on}: readable`,
      );
    }
  }
  // And the stylesheet uses those inks where it puts text on the colours.
  for (const rule of [".toast {", ".toast.bad {", "button.danger {"]) {
    const at = css.indexOf(`\n${rule}`);
    expect(at).toBeGreaterThan(0);
    const body = block(at);
    const background = /background:\s*var\((--[a-z-]+)\)/.exec(body)?.[1];
    expect(body).toContain(`color: var(${background}-ink)`);
  }
  // A button on a toast (Undo) is written in the toast's ink, not the page's.
  expect(block(css.indexOf("\n.toast button {"))).toContain("color: inherit");
});

test("the corner of the header says live, how old, offline or signed out", () => {
  const now = Date.parse("2026-11-23T12:00:00Z");
  const at = (secondsAgo: number) => now - secondsAgo * 1000;
  expect(freshSaid({ asOf: at(10) }, now)).toEqual({ text: "live", tone: "" });
  expect(freshSaid({ asOf: at(120) }, now)).toEqual({ text: "as of 2 min ago", tone: "" });
  expect(freshSaid({ asOf: at(600) }, now)).toEqual({ text: "as of 10 min ago", tone: "stale" });
  // With no connection it never says "live", however recent the copy.
  expect(freshSaid({ asOf: at(10), offline: true }, now)).toEqual({ text: "offline · just now", tone: "stale" });
  expect(freshSaid({ asOf: at(7200), offline: true }, now)).toEqual({ text: "offline · 2 h ago", tone: "stale" });
  expect(freshSaid({ offline: true }, now)).toEqual({ text: "offline", tone: "stale" });
  // Signed out is said before anything else: it is what to act on.
  expect(freshSaid({ asOf: at(10), offline: true, signedOut: true }, now)).toEqual({
    text: "signed out",
    tone: "down",
  });
  expect(freshSaid({}, now)).toEqual({ text: "", tone: "" });
});

test("the line under the header says there is no connection and what is kept, and nothing when there is nothing to say", () => {
  expect(keptSaid({})).toBeNull();
  expect(
    keptSaid({ offline: false, signedOut: false, summary: { waiting: 0, stale: 0, failed: 0, held: 2, done: 5 } }),
  ).toBeNull();
  expect(keptSaid({ offline: true })).toEqual({
    text: "No connection: this is what the device last saw.",
    tone: "",
    to: "more/device",
  });
  expect(keptSaid({ offline: true, summary: { waiting: 1 } })).toEqual({
    text: "No connection: this is what the device last saw · 1 thing kept to send.",
    tone: "",
    to: "more/device",
  });
  // Connected, and something still waits (it is being sent, or waits for a machine).
  expect(keptSaid({ summary: { waiting: 3 } })).toEqual({
    text: "3 things kept to send.",
    tone: "",
    to: "more/device",
  });
  // What needs the owner is said as needing them.
  expect(keptSaid({ summary: { waiting: 1, stale: 2, failed: 1 } })).toEqual({
    text: "1 thing kept to send · 2 things kept back for you to look at · 1 thing could not be done.",
    tone: "warn",
    to: "more/device",
  });
  // Signed out: a tap signs in again, unless there is something to look at first.
  expect(keptSaid({ signedOut: true, offline: true, summary: { waiting: 2 } })).toEqual({
    text: "Signed out: tap to sign in again · 2 things kept to send.",
    tone: "warn",
    to: "reload",
  });
  expect(keptSaid({ signedOut: true, summary: { failed: 1 } })?.to).toBe("more/device");
});

test("what a send did is said in one line, and nothing is said of nothing", () => {
  const e = (state: string) => ({ state });
  expect(flushSaid([])).toBe("");
  expect(flushSaid([e("waiting")])).toBe("");
  expect(flushSaid([e("done")])).toBe("What you did offline: 1 sent.");
  // Taken by the platform for a machine counts as sent: it has it.
  expect(flushSaid([e("done"), e("held"), e("failed"), e("stale"), e("stale")])).toBe(
    "What you did offline: 2 sent, 1 could not be done, 2 kept back for you to look at.",
  );
});

test("what is kept is listed by what it needs: waiting, kept back, failed, with the platform, sent", () => {
  const e = (id: string, state: string, at: number) => ({ id, state, at });
  const groups = keptGroups([
    e("d1", "done", 1),
    e("w1", "waiting", 2),
    e("w2", "waiting", 5),
    e("f1", "failed", 3),
    e("s1", "stale", 4),
    e("h1", "held", 6),
  ]);
  expect(groups.map((g) => [g.title, g.can, g.entries.map((x: { id: string }) => x.id)])).toEqual([
    ["Waiting to send", ["drop"], ["w2", "w1"]],
    ["Kept back for you to look at", ["send", "drop"], ["s1"]],
    ["Could not be done", ["again", "drop"], ["f1"]],
    ["With the platform, waiting to be done", [], ["h1"]],
    ["Sent", [], ["d1"]],
  ]);
  // A group with nothing in it is not shown.
  expect(keptGroups([e("w1", "waiting", 1)]).map((g) => g.state)).toEqual(["waiting"]);
  expect(keptGroups([])).toEqual([]);
});

test("keeping a copy of every screen reads each under the address its screen asks for it by", () => {
  const caps = [{ id: "id" }, { id: "pm-stack" }, { id: "repo-and-ci" }];
  expect(everyRead(caps, true)).toEqual([
    "doc/core",
    "doc/questions",
    "doc/ops",
    "me",
    "roadmaps",
    "roadmap?area=id",
    "roadmap?area=pm-stack",
    "roadmap?area=repo-and-ci",
    "bugs",
    "intake",
    "queue?status=blocked&q=",
    "queue?status=queued&q=",
    "queue?status=claimed&q=",
    "queue?status=parked&q=",
    "releases",
    "platform",
    "tokens",
    "messages?limit=15",
    "push",
    "devices",
    "machines",
    "signin",
    "android",
  ]);
  // Served straight from a machine there is no kept Android build to ask about.
  expect(everyRead(caps, false)).not.toContain("android");
  // The list of capabilities could not be read: the rest still is.
  expect(everyRead(undefined, false)).toEqual([
    "doc/core",
    "doc/questions",
    "doc/ops",
    "me",
    "roadmaps",
    "bugs",
    "intake",
    "queue?status=blocked&q=",
    "queue?status=queued&q=",
    "queue?status=claimed&q=",
    "queue?status=parked&q=",
    "releases",
    "platform",
    "tokens",
    "messages?limit=15",
    "push",
    "devices",
    "machines",
    "signin",
  ]);
});

test("what the owner is told of something just done, from what became of it when it was sent in its turn", () => {
  const id = "0c000000-0000-4000-8000-000000000001";
  const older = [{ id: "x", state: "done" }];
  const quiet = { others: [], signin: false, offline: false };
  // Done: its result, and what else went with it.
  expect(sentSaid({ ...quiet, entry: { state: "done", result: { rank: "next" } }, others: older }, id)).toEqual({
    state: "done",
    result: { rank: "next" },
    others: older,
  });
  expect(sentSaid({ ...quiet, entry: { state: "done" } }, id)).toEqual({ state: "done", result: null, others: [] });
  // Taken by the platform for a machine.
  // Taken by the platform to be done later, with what it said of who will.
  expect(sentSaid({ ...quiet, entry: { state: "held", note: "Kept: who will" }, others: older }, id)).toEqual({
    state: "held",
    note: "Kept: who will",
    others: older,
  });
  expect(sentSaid({ ...quiet, entry: { state: "held" }, others: older }, id)).toEqual({
    state: "held",
    note: "",
    others: older,
  });
  // Refused: said, and nothing more.
  expect(sentSaid({ ...quiet, entry: { state: "failed", note: "no such issue" } }, id)).toEqual({
    state: "failed",
    note: "no such issue",
  });
  // Kept on the device: with no connection, with no sign-in, and when
  // nothing could take it or it goes behind another, each said as itself.
  expect(sentSaid({ ...quiet, offline: true, entry: { state: "waiting", note: "" } }, id)).toEqual({
    state: "kept",
    id,
    note: "Kept on this device. It is sent when you are connected.",
  });
  expect(
    sentSaid({ ...quiet, signin: true, entry: { state: "waiting", note: "Sign in again to send this." } }, id),
  ).toEqual({
    state: "kept",
    id,
    note: "Kept on this device. Sign in again to send it.",
  });
  expect(
    sentSaid(
      { ...quiet, entry: { state: "waiting", note: "Nothing could take it just now. It is offered again." } },
      id,
    ),
  ).toEqual({
    state: "kept",
    id,
    note: "Kept on this device: Nothing could take it just now. It is offered again.",
  });
  // Taken back by the owner while it was out.
  expect(sentSaid({ ...quiet, entry: null }, id)).toEqual({
    state: "refused",
    note: "It was taken back before it was sent.",
  });
});

test("a question is left out of the list while its answer is kept and not yet recorded, and is shown again when the answer is taken back", () => {
  const answer = (qid: string, state: string, kind = "question.answer") => ({
    kind,
    state,
    args: { set: "s1", ledger: "l1", qid, option: "a" },
  });
  // Waiting for a connection, or with the platform for a machine: left out.
  expect([
    ...answeredKeys([answer("q1", "waiting"), answer("q2", "held"), answer("q3", "waiting", "question.defer")]),
  ]).toEqual(["s1|l1|q1", "s1|l1|q2", "s1|l1|q3"]);
  // Recorded, refused, or kept back: nothing to leave out any more.
  expect([...answeredKeys([answer("q1", "done"), answer("q2", "failed"), answer("q3", "stale")])]).toEqual([]);
  // Taken back by the owner, the entry is gone, and so is the reason to hide the question.
  expect([...answeredKeys([])]).toEqual([]);
  expect([...answeredKeys(null)]).toEqual([]);
  // Other things that are kept are not answers.
  expect([
    ...answeredKeys([
      { kind: "ask.send", state: "waiting", args: { text: "x" } },
      { kind: "questions.accept", state: "waiting", args: { set: "s1" } },
    ]),
  ]).toEqual([]);
  // One being sent this moment is left out too, with what is kept.
  expect([...answeredKeys([answer("q1", "waiting")], ["s1|l1|q9"])]).toEqual(["s1|l1|q9", "s1|l1|q1"]);
});

test("a machine, as the Machines screen says it: what only it reports and does, beside what the platform does itself", () => {
  const now = Date.parse("2026-11-23T12:00:00Z");
  const machine = {
    name: "nixos",
    seen: now - 12_000,
    quiet: false,
    version: "0123abcd4567",
    actions: ["ask.send", "issue.rank", "question.answer", "task.cancel", "unit.act", "push.on", "something.new"],
    sections: ["board", "sessions", "questionCount", "questions", "services", "asks"],
  };
  // The platform does the core itself: only what is left is the machine's.
  const platform = {
    primary: true,
    actions: ["ask.send", "issue.rank", "push.on"],
    sections: ["board", "asks"],
    refused: [],
  };
  expect(machineSaid(machine, platform, now)).toEqual({
    name: "nixos",
    reporting: true,
    state: "Reporting. Last heard from just now.",
    version: "0123abc",
    // The count of questions is part of the questions, not a thing of its own.
    reports: ["sessions", "questions", "its units"],
    // A kind this screen has no words for is said as it is named.
    does: ["record an answer", "cancel a task", "start, stop or restart a unit", "something.new"],
    said: true,
    // Nothing said of the code Cloudflare runs, and it is on main's settings.
    otherCode: "",
    differs: [],
    // The owner has changed no setting: nothing to take.
    settings: "",
  });
  // Cloudflare's own commit, where it is known: said when the machine runs another, and only then.
  expect(machineSaid(machine, platform, now, "0123abcd4567").otherCode).toBe("");
  expect(machineSaid(machine, platform, now, "fedcba987654").otherCode).toBe(
    "Cloudflare runs fedcba9: the two are on different code.",
  );
  expect(machineSaid({ ...machine, version: null }, platform, now, "fedcba987654").otherCode).toBe("");
  // On other settings than main's: each is said, in the order given.
  const differs = [
    { name: "queue", machine: "another-queue", platform: "repo-health-code-pilot", goesBy: "platform" },
    { name: "needsRepos", machine: ["a/b", "c/d"], platform: ["a/b", "e/f"], goesBy: "machine" },
  ];
  expect(machineSaid({ ...machine, differs }, platform, now).differs).toEqual(differs.map(differsSaid));
  // An answer whose list is not one says nothing of it.
  expect(machineSaid({ ...machine, differs: "many" }, platform, now).differs).toEqual([]);
  // With nothing in front of it, everything is the machine's.
  const alone = machineSaid(machine, null, now);
  expect(alone.reports).toEqual(["the board", "sessions", "questions", "its units", "your prompts"]);
  expect(alone.does).toContain("send a prompt");
  // Away, and for how long.
  expect(machineSaid({ ...machine, seen: now - 2 * 3600_000, quiet: true }, platform, now)).toMatchObject({
    reporting: false,
    state: "Not reporting. Last heard from 2 h ago.",
  });
  expect(machineSaid({ ...machine, seen: null, quiet: true }, platform, now)).toMatchObject({
    reporting: false,
    state: "It has never reported.",
  });
  // A console older than this screen: a machine that has not said what it can do.
  expect(
    machineSaid(
      { name: "A machine", seen: now, quiet: false, version: null, actions: [], sections: [] },
      platform,
      now,
    ),
  ).toMatchObject({
    version: "",
    reports: [],
    does: [],
    said: false,
  });
  // Lists in words.
  expect([listed([]), listed(["a"]), listed(["a", "b"]), listed(["a", "b", "c"])]).toEqual([
    "",
    "a",
    "a and b",
    "a, b and c",
  ]);
});

test("the Machines screen is drawn again when what it shows has changed, and not for a report that changed nothing", () => {
  const now = Date.parse("2026-11-23T12:00:00Z");
  const machine = {
    name: "nixos",
    seen: now - 5_000,
    quiet: false,
    version: "0123abcd4567",
    actions: ["a.b"],
    sections: ["board"],
  };
  const answer = (more: Record<string, unknown> = {}) => ({
    served: "edge",
    platform: { primary: true, actions: ["ask.send"], sections: ["board"], refused: [] },
    machines: [machine],
    held: [{ id: "h1", kind: "task.move" }],
    ...more,
  });
  const mark = machinesMark(answer(), now);
  // Another report from a machine that is still reporting changes nothing that is shown.
  expect(machinesMark(answer({ machines: [{ ...machine, seen: now }] }), now + 5_000)).toBe(mark);
  // Each of these does.
  for (const [why, changed] of [
    ["it went quiet", answer({ machines: [{ ...machine, quiet: true }] })],
    ["it runs another commit", answer({ machines: [{ ...machine, version: "fedcba987654" }] })],
    [
      "it runs with another setting",
      answer({
        machines: [{ ...machine, differs: [{ name: "queue", machine: "q", platform: "p", goesBy: "platform" }] }],
      }),
    ],
    ["it can do something more", answer({ machines: [{ ...machine, actions: ["a.b", "c.d"] }] })],
    ["it is called something else", answer({ machines: [{ ...machine, name: "amd" }] })],
    ["there is another", answer({ machines: [machine, { ...machine, name: "amd" }] })],
    ["it took the owner's settings", answer({ machines: [{ ...machine, settingsRev: 2 }] })],
    ["there is none", answer({ machines: [] })],
    ["something else waits", answer({ held: [{ id: "h1" }, { id: "h2" }] })],
    ["what waits now waits for someone else", answer({ held: [{ id: "h1", kind: "task.move", waits: "asked" }] })],
    ["nothing waits", answer({ held: [] })],
    [
      "the platform stopped doing the core",
      answer({ platform: { primary: false, actions: [], sections: [], refused: [] } }),
    ],
    [
      "the platform was turned away for something",
      answer({ platform: { primary: true, actions: [], sections: ["board"], refused: ["pr.merge"] } }),
    ],
    [
      "the owner changed a setting",
      answer({ platform: { primary: true, actions: ["ask.send"], sections: ["board"], refused: [], settingsRev: 3 } }),
    ],
    ["nothing is in front any more", answer({ platform: null })],
  ] as const) {
    expect([why, machinesMark(changed, now) === mark]).toEqual([why, false]);
  }
  // Gone quiet, it is drawn again as the minutes pass (the screen says how long), not with every look.
  const quiet = answer({ machines: [{ ...machine, quiet: true, seen: now - 90_000 }] });
  expect(machinesMark(quiet, now + 20_000)).toBe(machinesMark(quiet, now));
  expect(machinesMark(quiet, now + 60_000)).not.toBe(machinesMark(quiet, now));
  // An answer that is not one is not a crash.
  expect(machinesMark(null, now)).toBe(machinesMark({}, now));
});

// The oracle is the Worker's own lists: whatever it does or reads
// itself it can be turned away for, and then the app says so. A kind
// with no words of its own is shown by its name ("hands.done").
// Seen on the live board on 2026-11-24: "Why 272 tasks are blocked"
// above six lines that added up to fifteen. Almost every blocked task
// carries a reason of its own, so the commonest six said nothing of the
// rest. The oracle is arithmetic: what is said has to add up to what is
// blocked, whatever the reasons are.
test("why tasks are blocked is said by the kind of reason, and adds up to all of them", () => {
  const kinds = [
    { kind: "reason", n: 231 },
    { kind: "time-past", n: 9 },
    { kind: "time", n: 2 },
    { kind: "tasks", n: 12 },
    { kind: "none", n: 21 },
  ];
  const said = blockedSaid(kinds);
  expect(said.map((line) => line.n).reduce((a, b) => a + b, 0)).toBe(275);
  expect(said.map((line) => `${line.n} ${line.text}`)).toEqual([
    "231 have a reason written on them",
    "12 wait on other tasks",
    "11 wait for a time: for 9 of them it has already passed",
    "21 have no reason recorded",
  ]);
  // One, and the words follow.
  expect(
    blockedSaid([
      { kind: "reason", n: 1 },
      { kind: "time", n: 1 },
      { kind: "tasks", n: 1 },
      { kind: "none", n: 1 },
    ]).map((l) => `${l.n} ${l.text}`),
  ).toEqual([
    "1 has a reason written on it",
    "1 waits on other tasks",
    "1 waits for a time",
    "1 has no reason recorded",
  ]);
  expect(blockedSaid([{ kind: "time-past", n: 1 }]).map((l) => `${l.n} ${l.text}`)).toEqual([
    "1 waits for a time: it has already passed",
  ]);
  // A kind this screen has no words for is still counted, and said as it is named.
  expect(blockedSaid([{ kind: "something-new", n: 3 }]).map((l) => `${l.n} ${l.text}`)).toEqual(["3 something-new"]);
  // Counts as the queue store gives them (text), and nothing at all.
  expect(blockedSaid([{ kind: "none", n: "4" }]).map((l) => l.n)).toEqual([4]);
  expect(blockedSaid([])).toEqual([]);
  expect(blockedSaid(null)).toEqual([]);
  // The reasons as an older console gives them (each its own text, the commonest six): said as they were.
  expect(blockedSaid([{ why: "waiting on native issue blockers", n: "7" }]).map((l) => `${l.n} ${l.text}`)).toEqual([
    "7 waiting on native issue blockers",
  ]);
});

// What was finished lately, as the app says it. The oracle is the
// document's own numbers: what is said has to be them, with "at least"
// where GitHub's page was full and there may be more than were counted.
test("what was finished in the last day is said in numbers, with 'at least' where there may be more", () => {
  const none = { day: 0, more: false, latest: [] };
  expect(
    doneSaid({
      tasks: { day: 12, latest: [] },
      merged: { day: 100, more: true, latest: [] },
      closed: { day: 1, more: false, latest: [] },
    }),
  ).toEqual({
    tiles: [
      { n: "12", label: "tasks finished" },
      { n: "100+", label: "pull requests merged" },
      { n: "1", label: "issue closed" },
    ],
    line: "In the last day: 12 tasks finished, at least 100 pull requests merged, 1 issue closed.",
  });
  // One of each, and none of some: said as it is, the nothings left out of the line.
  expect(doneSaid({ tasks: { day: 1, latest: [] }, merged: { ...none, day: 1 }, closed: none })).toEqual({
    tiles: [
      { n: "1", label: "task finished" },
      { n: "1", label: "pull request merged" },
      { n: "0", label: "issues closed" },
    ],
    line: "In the last day: 1 task finished, 1 pull request merged.",
  });
  expect(doneSaid({ tasks: { day: 0, latest: [] }, merged: none, closed: none })?.line).toBe(
    "Nothing was finished in the last day.",
  );
  // None counted, but the page was full of older things: there may still be some.
  expect(
    doneSaid({ tasks: { day: 0, latest: [] }, merged: { ...none, more: true }, closed: { ...none, more: true } }),
  ).toEqual({
    tiles: [
      { n: "0", label: "tasks finished" },
      { n: "0+", label: "pull requests merged" },
      { n: "0+", label: "issues closed" },
    ],
    line: "In the last day: nothing counted; pull requests merged and issues closed could not all be counted.",
  });
  // Not read yet, or a copy from a console that does not read it: nothing is said, not zeros.
  expect(doneSaid(null)).toBeNull();
  expect(doneSaid(undefined)).toBeNull();
  expect(doneSaid({})).toBeNull();
  expect(doneSaid({ tasks: { day: "many" }, merged: none, closed: none })).toBeNull();
});

// What the day's finished tasks were, by kind. The oracle is the counts
// themselves: each is said with its kind's name, the most first, and
// what is said adds up to what was counted.
test("the day's finished tasks are said by kind, the most first", () => {
  expect(
    doneKindsSaid([
      { kind: "second_pass", n: 650 },
      { kind: "small_fix", n: 40 },
      { kind: null, n: 3 },
      { kind: "triage", n: 1 },
    ]),
  ).toBe("650 second pass · 40 small fix · 3 of no kind · 1 triage");
  // Nothing counted, a copy without them, and counts that are not counts: nothing is said.
  expect(doneKindsSaid([])).toBe("");
  expect(doneKindsSaid(undefined)).toBe("");
  expect(doneKindsSaid([{ kind: "triage", n: 0 }, { kind: "x", n: "many" }, null])).toBe("");
});

// A lane the pool made up for itself is a long run of letters that is
// nobody's name (seen live: "any-occodecpanaakhknhcenhhbmkcflkicffigpl"
// across two lines of a card). It is said as what it is.
test("a lane the pool made up is said as a pooled lane; a named one by its name", () => {
  expect(laneSaid("any-occodecpanaakhknhcenhhbmkcflkicffigpl")).toBe("a pooled lane");
  expect(laneSaid("any-o")).toBe("any-o");
  expect(laneSaid("code-i")).toBe("code-i");
  expect(laneSaid("oc-code-06")).toBe("oc-code-06");
  expect(laneSaid(null)).toBe("");
  expect(laneSaid(undefined)).toBe("");
});

// Who merged is said only where it tells something. Seen live: every
// one of forty pull requests said the same name, the owner's, because
// his agents work under it.
test("who a pull request is by is said only when they are not all by the same one", () => {
  const prs = (...by: (string | null)[]) => by.map((who, i) => ({ number: i + 1, by: who }));
  expect(mergedBySaid(prs("dezren39", "dezren39", "dezren39"))).toBe(false);
  expect(mergedBySaid(prs("dezren39", "dependabot[bot]", "dezren39"))).toBe(true);
  // One alone, or none: there is nothing to tell apart.
  expect(mergedBySaid(prs("dezren39"))).toBe(false);
  expect(mergedBySaid([])).toBe(false);
  expect(mergedBySaid(undefined)).toBe(false);
});

// A finished task says what came of it. The oracle is the queue's own
// list of verdicts (fleet-queue's disposition.rs: VERDICTS), each given
// words here; one this app has no words for is said as it is named.
test("a finished task says what came of it, where it goes, and who did it", () => {
  const task = {
    id: 21001,
    title: "Fix the claim scan",
    at: "2026-11-24T10:00:00",
    lane: "code-a",
    model: "m-1",
    kind: "small_fix",
    issue: 5,
    pr: 77,
    verdict: "merged_verified",
    url: "https://github.com/o/r/issues/5",
    prUrl: "https://github.com/o/r/pull/77",
  };
  const now = Date.parse("2026-11-24T12:00:00Z");
  expect(doneTaskSaid(task, now)).toEqual({
    chips: ["small fix", "merged"],
    links: [
      { text: "#77", url: "https://github.com/o/r/pull/77" },
      { text: "issue #5", url: "https://github.com/o/r/issues/5" },
    ],
    meta: "task 21001 · code-a · m-1 · finished 2 h ago",
  });
  const said = (verdict: string | null) => doneTaskSaid({ ...task, verdict }, now).chips.slice(1);
  expect(
    ["merged_verified", "closed_verified", "open_draft_with_followup", "open_draft_superseded", "invalid_no_work"].map(
      (v) => said(v)[0],
    ),
  ).toEqual([
    "merged",
    "issue closed",
    "left as a draft, with a follow-up",
    "its draft was superseded",
    "no work came of it",
  ]);
  expect(said("something_new")).toEqual(["something new"]);
  expect(said(null)).toEqual([]);
  // Done in a lane the pool made up: said as that, not by its run of letters.
  expect(doneTaskSaid({ ...task, lane: "any-occodecpanaakhknhcenhhbmkcflkicffigpl" }, now).meta).toBe(
    "task 21001 · a pooled lane · m-1 · finished 2 h ago",
  );
  // With nothing known but that it was finished.
  expect(
    doneTaskSaid(
      {
        id: 7,
        title: "Task 7",
        at: "",
        lane: null,
        model: null,
        kind: null,
        issue: null,
        pr: null,
        verdict: null,
        url: null,
        prUrl: null,
      },
      now,
    ),
  ).toEqual({
    chips: [],
    links: [],
    meta: "task 7",
  });
});

// What he is sending is his to say: something to do, a question, or
// how he wants work done from now on. The oracle is the three kinds
// the platform takes (ASK_INTENTS in src/work.ts): each has words here,
// and anything else is taken as something to do.
test("each kind of prompt has its own words, and the platform takes every kind the app offers", () => {
  expect(ASK_KINDS.map((k) => k.intent)).toEqual([...ASK_INTENTS]);
  expect(ASK_KINDS.map((k) => [k.label, k.send, k.chip])).toEqual([
    ["Do this", "Send", ""],
    ["Answer this", "Ask", "Question"],
    ["From now on", "Set", "Standing instruction"],
  ]);
  for (const kind of ASK_KINDS) expect(kind.placeholder.length).toBeGreaterThan(40);
  expect(askKind("question")).toBe(ASK_KINDS[1]);
  expect(askKind("directive")).toBe(ASK_KINDS[2]);
  for (const other of ["task", undefined, null, "rant", 7]) expect(askKind(other)).toBe(ASK_KINDS[0]);
  // What is sent with it: nothing more for something to do (as before), the kind for the others.
  expect(askArgs("  Do the docs  ", "task")).toEqual({ text: "Do the docs" });
  expect(askArgs("Why?", "question")).toEqual({ text: "Why?", intent: "question" });
  expect(askArgs("No red merges", "directive")).toEqual({ text: "No red merges", intent: "directive" });
  expect(askArgs("x", "rant")).toEqual({ text: "x" });
  // And what he is told when it has gone.
  expect(askSent("task", { issue: 7, queued: true })).toBe("Sent as #7; first in the queue");
  expect(askSent("question", { issue: 7, queued: true })).toBe("Asked as #7; first in the queue");
  expect(askSent("directive", { issue: 7, queued: true })).toBe("Set as #7; first in the queue");
  expect(askSent("question", { issue: 7, queued: false, note: "filed, not queued: the queue is down" })).toBe(
    "Filed as #7. filed, not queued: the queue is down",
  );
  expect(askSent("task", { issue: 7, queued: false })).toBe("Filed as #7.");
});

// An enroll command shared to the phone is read back for the
// provisioning form (#14377): the binding the mint put in it. The
// oracle is the command the form itself shows at a mint (name, kind,
// groups, the bootstrap token); the token is opaque, so a command is
// known by its flags, and text without one is not a command.
test("a shared enroll command gives its binding back, and other text does not", () => {
  expect(
    enrollCommand("fleet agent enroll --name scout-1 --kind muse --groups muse,fleet --bootstrap tok-abc123"),
  ).toEqual({
    token: "tok-abc123",
    name: "scout-1",
    kind: "muse",
    groups: ["muse", "fleet"],
    notesIdentity: undefined,
  });
  expect(enrollCommand("fleet agent enroll --name solo --bootstrap tok-xyz")).toEqual({
    token: "tok-xyz",
    name: "solo",
    kind: undefined,
    groups: [],
    notesIdentity: undefined,
  });
  expect(enrollCommand("fleet agent enroll --name n --kind codex --notes-identity muse/n --bootstrap t")).toEqual({
    token: "t",
    name: "n",
    kind: "codex",
    groups: [],
    notesIdentity: "muse/n",
  });
  // No bootstrap token, no enroll step, or not text at all: not a command.
  expect(enrollCommand("fleet agent enroll --name scout-1 --kind muse")).toBeNull();
  expect(enrollCommand("please enroll the new laptop when it arrives")).toBeNull();
  expect(enrollCommand("run --bootstrap the servers")).toBeNull();
  expect(enrollCommand(null)).toBeNull();
  expect(enrollCommand(42)).toBeNull();
});

// A goal most of a list shares is said once, not on every card. Seen
// live: sixty queued tasks, fifty-nine of them triage with the same
// three lines of goal each. The oracle is counting: the goal that more
// than half of the tasks carry, where there are at least three of them.
test("a goal most of the tasks share is found, to be said once; where there is none, each card keeps its own", () => {
  const t = (goal: string | null, step: string | null = null) => ({ goal, step });
  const same = "Record a verdict on the issue";
  expect(sharedGoal([t(same), t(same), t(same), t("Another")])).toEqual({ goal: same, n: 3 });
  // Exactly half is not most, however many that is.
  expect(sharedGoal([t(same), t(same), t(same), t("A"), t("B"), t("C")])).toBeNull();
  expect(sharedGoal([t(same), t(same), t(same), t(same), t("A"), t("B"), t("C")])).toEqual({ goal: same, n: 4 });
  // Two alike say nothing of a list.
  expect(sharedGoal([t(same), t(same)])).toBeNull();
  // A card that shows its next step does not show its goal, so it is not counted as carrying one.
  expect(sharedGoal([t(same, "doing it"), t(same, "doing it"), t(same, "doing it"), t(same)])).toBeNull();
  expect(sharedGoal([t(same), t(same), t(same), t(same, "doing it")])).toEqual({ goal: same, n: 3 });
  // No goals at all, nothing at all, and something that is not a list.
  expect(sharedGoal([t(null), t(""), t(null)])).toBeNull();
  expect(sharedGoal([])).toBeNull();
  expect(sharedGoal(undefined)).toBeNull();
  // What is said of it.
  expect(sharedGoalSaid({ goal: same, n: 59 }, 60)).toBe(
    "59 of these 60 have the same goal, said here once: Record a verdict on the issue",
  );
  expect(sharedGoalSaid({ goal: same, n: 3 }, 3)).toBe(
    "All 3 have the same goal, said here once: Record a verdict on the issue",
  );
});

// The composer's match report, said as the bridge reported it
// (#14375, acceptance 1-2): matched and delivered counts with the
// recipients named, the remainder while any are left, and a broaden
// folded in until none remain — the screen then shows no Broaden.
test("a match report is said with its recipients, and a broaden folds into it", () => {
  const sent = { batch: "b9", matched: 2, delivered: 1, recipients: ["felix-1"], remaining: 1 };
  expect(matchReportSaid(sent)).toBe("Matched 2 · sent to 1 (felix-1) · 1 more matched and not yet reached");
  const widened = mergeMatchReport(sent, {
    batch: "b9",
    matched: 2,
    delivered: 1,
    recipients: ["felix-2"],
    remaining: 0,
  });
  expect(widened.delivered).toBe(2);
  expect(widened.recipients).toEqual(["felix-1", "felix-2"]);
  expect(widened.remaining).toBe(0);
  expect(matchReportSaid(widened)).toBe("Matched 2 · sent to 2 (felix-1, felix-2)");
  // An answer that is not a report (a send held for a machine) is
  // not said as one.
  expect(matchReportSaid({ held: true })).toBeNull();
  expect(matchReportSaid(null)).toBeNull();
  expect(mergeMatchReport(sent, { held: true })).toBe(sent);
  // An offer is said by its state and its claimed slots.
  expect(offerSaid({ state: "open", claimed: 1, slots: 2, claims: [{ agent: "felix-1" }] })).toBe(
    "open · 1 of 2 claimed",
  );
  expect(offerSaid({ state: "filled", slots: 2, claims: [{ agent: "a" }, { agent: "b" }] })).toBe(
    "filled · 2 of 2 claimed",
  );
});

// A reason that was cut where the queue's store cuts it says that it
// was. Seen live: "...and the sandbox CLI, t". The oracle is the length
// the read asks the store for (QUEUE_WHY_CHARS in src/ops.ts).
test("a blocked task's reason that was cut short ends by saying so", () => {
  expect(QUEUE_WHY_CHARS).toBe(300);
  expect(whyShown("Short reason.")).toBe("Short reason.");
  expect(whyShown("x".repeat(299))).toBe("x".repeat(299));
  expect(whyShown("x".repeat(300))).toBe(`${"x".repeat(300)}…`);
  expect(whyShown(`${"word ".repeat(59)}and t`)).toBe(`${"word ".repeat(59)}and t…`);
  expect(whyShown("")).toBe("");
  expect(whyShown(null)).toBe("");
});

// What needs the owner is what only he can do. Seen live: "394 need
// you" on the first screen, of which 347 were questions the fleet had
// already recommended an answer to. The number that is his alone leads.
test("what only the owner can do leads: the count on the first screen, the badge, and the list the Needs screen opens on", () => {
  const counts = { questions: 347, hands: 29, reviews: 19, other: 9 };
  expect(needsLead(counts)).toEqual({ badge: 48, tile: 29, tileLabel: "only you", opens: "hands" });
  // Nothing for his hands: reviews asked of him, then questions.
  expect(needsLead({ ...counts, hands: 0 })).toMatchObject({ badge: 19, tile: 0, opens: "reviews" });
  expect(needsLead({ questions: 5, hands: 0, reviews: 0, other: 2 })).toMatchObject({
    badge: 0,
    tile: 0,
    opens: "questions",
  });
  // Nothing at all still opens somewhere.
  expect(needsLead({ questions: 0, hands: 0, reviews: 0, other: 0 })).toMatchObject({ badge: 0, opens: "hands" });
});

test("the sets with a recommendation to accept, each once, with how many questions that answers", () => {
  const open = [
    { set: "s1", setTitle: "One", recommended: ["a"] },
    { set: "s1", setTitle: "One", recommended: [] },
    { set: "s2", setTitle: "Two", recommended: [] },
    { set: "s3", setTitle: "Three", recommended: ["x"] },
    { set: "s1", setTitle: "One", recommended: ["b"] },
  ];
  expect(recommendedSets(open)).toEqual({ sets: ["s1", "s3"], questions: 3 });
  expect(recommendedSets([])).toEqual({ sets: [], questions: 0 });
  expect(recommendedSets(null)).toEqual({ sets: [], questions: 0 });
});

test("accepting every recommendation says what it came to: answered, waiting to go, and not accepted", () => {
  const done = (applied: number) => ({ state: "done", result: { applied } });
  expect(acceptAllSaid([done(2), done(5)])).toBe("7 answered");
  expect(acceptAllSaid([done(2), { state: "held" }, { state: "kept" }])).toBe(
    "2 answered; 2 sets wait to be sent or done",
  );
  expect(acceptAllSaid([{ state: "kept" }])).toBe("1 set waits to be sent or done");
  expect(acceptAllSaid([done(1), { state: "refused", note: "no" }, null])).toBe(
    "1 answered; 2 sets could not be accepted",
  );
  expect(acceptAllSaid([{ state: "refused" }])).toBe("1 set could not be accepted");
  // A set whose questions were all answered meanwhile is accepted, and answers nothing.
  expect(acceptAllSaid([done(0)])).toBe("0 answered");
  expect(acceptAllSaid([])).toBe("0 answered");
});

test("a unit's state is said once where systemd's two words are the same", () => {
  expect(unitStateSaid({ active: "failed", sub: "failed" })).toBe("failed");
  expect(unitStateSaid({ active: "active", sub: "running" })).toBe("active · running");
  expect(unitStateSaid({ active: "inactive", sub: "dead" })).toBe("inactive · dead");
  expect(unitStateSaid({ active: "active" })).toBe("active");
  expect(unitStateSaid(null)).toBe("");
});

// Seen on the live board: a task's lane is its agent's own name
// ("code-f", "oc-code-02") or one of the pool's ("any-n", "any-occode…"),
// and its worker ("repo-worker-code", "xaltha") is always an agent's.
test("a task's agent, to message it: its lane where that is an agent's name, else its worker", () => {
  // With the registered names in hand, only a registered one is offered.
  const names = new Set(["code-f", "any-n", "xaltha", "repo-worker-code", "conductor"]);
  expect(taskAgent({ lane: "code-f", worker: "repo-worker-code" }, names)).toBe("code-f");
  expect(taskAgent({ lane: "any-n", worker: "repo-worker-code" }, names)).toBe("any-n");
  expect(taskAgent({ lane: "any-occodefihdbkklhacimchhigmnffadedgjjjdi", worker: "xaltha" }, names)).toBe("xaltha");
  expect(taskAgent({ lane: "somewhere-else", worker: "nobody-known" }, names)).toBeNull();
  // Without them: the lane, unless it is one of the pool's made-up ones.
  expect(taskAgent({ lane: "code-f", worker: "repo-worker-code" })).toBe("code-f");
  expect(taskAgent({ lane: "any-n", worker: "repo-worker-code" })).toBe("any-n");
  expect(taskAgent({ lane: "any-occodefihdbkklhacimchhigmnffadedgjjjdi", worker: "xaltha" })).toBe("xaltha");
  expect(taskAgent({ lane: null, worker: "conductor" })).toBe("conductor");
  expect(taskAgent({ lane: "", worker: "" })).toBeNull();
  expect(taskAgent(null)).toBeNull();
});

// The app is told when something changes (a WebSocket the Worker
// holds) and reads again at once. What it is told is the Worker's to
// word (edge.ts `changed`); the oracle for the wire is that message.
test("what the Worker says over the live connection: which documents to read again, and nothing else is acted on", () => {
  expect(liveMessage('{"t":"changed","docs":["core"]}')).toEqual({ docs: ["core"] });
  expect(liveMessage('{"t":"changed","docs":["core","ops","questions"]}')).toEqual({
    docs: ["core", "ops", "questions"],
  });
  // The platform's own answer to "ping", a kind this app does not know, and anything that is not a message.
  for (const text of [
    "pong",
    '{"t":"something-new","docs":["core"]}',
    '{"t":"changed"}',
    '{"t":"changed","docs":"core"}',
    "{",
    "",
    null,
    7,
  ]) {
    expect([text, liveMessage(text)]).toEqual([text, { docs: [] }]);
  }
  // A document this app does not have is not asked for.
  expect(liveMessage('{"t":"changed","docs":["core","secrets",7]}')).toEqual({ docs: ["core"] });
});

test("a live connection that drops is tried again, soon at first and then no more often than every half minute", () => {
  const delays = [1, 2, 3, 4, 5, 6, 7, 20].map((tries) => liveDelay(tries));
  expect(delays[0]).toBe(1_000);
  // Never sooner than the time before, never more than half a minute.
  for (let i = 1; i < delays.length; i++) expect(delays[i]).toBeGreaterThanOrEqual(delays[i - 1]);
  expect(Math.max(...delays)).toBe(30_000);
  expect(liveDelay(0)).toBe(1_000);
});

test("where the live connection is: the same host, secure where the page is", () => {
  expect(liveUrl({ protocol: "https:", host: "pm.example.com" })).toBe("wss://pm.example.com/console/api/events");
  expect(liveUrl({ protocol: "http:", host: "127.0.0.1:4872" })).toBe("ws://127.0.0.1:4872/console/api/events");
});

test("the header says the screen is live only while it is: connected, or read within the last look", () => {
  const now = Date.parse("2026-11-24T09:00:00Z");
  // Connected: live, however long ago the last read was, since a change would have been said.
  expect(freshSaid({ asOf: now - 5 * 60_000, offline: false, signedOut: false, live: true }, now)).toEqual({
    text: "live",
    tone: "",
  });
  // Not connected: by how old what is shown is, as before.
  expect(freshSaid({ asOf: now - 10_000, offline: false, signedOut: false, live: false }, now).text).toBe("live");
  expect(freshSaid({ asOf: now - 5 * 60_000, offline: false, signedOut: false, live: false }, now).text).toBe(
    "as of 5 min ago",
  );
  // No connection at all, or signed out, is said first.
  expect(freshSaid({ asOf: now - 5 * 60_000, offline: true, signedOut: false, live: true }, now).text).toBe(
    "offline · 5 min ago",
  );
  expect(freshSaid({ asOf: now, offline: false, signedOut: true, live: true }, now).text).toBe("signed out");
});

test("everything Cloudflare does or reads itself has words for being turned away for it", () => {
  const kinds = [...DIRECT_ACTIONS, ...DIRECT_READ_SECTIONS.map((name) => `read.${name}`)];
  const said = awayState({ edge: true, directEnabled: true, primary: true, refused: kinds }).refused ?? [];
  expect(said).toHaveLength(kinds.length);
  expect(kinds.filter((kind, i) => said[i] === kind)).toEqual([]);
  // Each in words of its own.
  expect(new Set(said).size).toBe(kinds.length);
});

test("a setting a machine runs with another value of is said in a sentence: what it says, what main says, and which Cloudflare goes by", () => {
  expect(
    differsSaid({ name: "queue", machine: "another-queue", platform: "repo-health-code-pilot", goesBy: "platform" }),
  ).toBe("For the queue it says another-queue. Cloudflare goes by main's: repo-health-code-pilot.");
  // The list of repositories is the one Cloudflare still goes by the machine's for, and why is said.
  expect(
    differsSaid({ name: "needsRepos", machine: ["a/b", "c/d"], platform: ["a/b", "e/f"], goesBy: "machine" }),
  ).toBe(
    "For the repositories looked in for what needs you it says a/b and c/d, and main says a/b and e/f. Cloudflare goes by this machine's until it runs main's: a machine takes a reply only for a repository on its own list.",
  );
  // What it does not say at all, and a list with nothing in it.
  expect(differsSaid({ name: "promptLabel", machine: null, platform: "owner/prompt", goesBy: "platform" })).toBe(
    "For the label that marks an instruction it says nothing. Cloudflare goes by main's: owner/prompt.",
  );
  expect(differsSaid({ name: "queueDbUrl", machine: "", platform: "https://x.example", goesBy: "platform" })).toBe(
    "For where the queue's store answers it says nothing. Cloudflare goes by main's: https://x.example.",
  );
  expect(differsSaid({ name: "needsRepos", machine: [], platform: ["a/b"], goesBy: "machine" })).toStartWith(
    "For the repositories looked in for what needs you it says none, and main says a/b.",
  );
  // Each setting has words of its own, and one this screen has none for is said by its name.
  const said = ["repo", "queue", "queueDbUrl", "queueTokenName", "androidPackage", "promptLabel", "needsRepos"].map(
    (name) => differsSaid({ name, machine: "x", platform: "y", goesBy: "platform" }),
  );
  expect(new Set(said).size).toBe(said.length);
  expect(said.filter((line) => /For (repo|queue[A-Z]|android|prompt|needs)/.test(line))).toEqual([]);
  expect(differsSaid({ name: "somethingNew", machine: "x", platform: "y", goesBy: "platform" })).toBe(
    "For somethingNew it says x. Cloudflare goes by main's: y.",
  );
  // A setting gone by the machine's that is not the list is said without a reason that is not its own.
  expect(differsSaid({ name: "queue", machine: "x", platform: "y", goesBy: "machine" })).toBe(
    "For the queue it says x, and main says y. Cloudflare goes by this machine's until it runs main's.",
  );
  // Something that is not a difference is not a crash.
  expect(differsSaid(null)).toBe("For a setting it says nothing. Cloudflare goes by main's: nothing.");
});

test("who something held waits for is said in the owner's words, and not at all when the platform did not say", () => {
  // Each of the three says something of its own.
  const said = ["either", "machine", "asked"].map((waits) => heldWaitsSaid(waits));
  expect(said.every((line) => line.length > 20)).toBe(true);
  expect(new Set(said).size).toBe(3);
  expect(said[0]).toContain("Cloudflare does this as soon as it can again");
  expect(said[2]).toContain("may have got it");
  // An answer older than the saying, or one that says something else: nothing is made up.
  for (const waits of [undefined, null, "", "someone", "constructor", "toString", 3, {}]) {
    expect([waits, heldWaitsSaid(waits)]).toEqual([waits, ""]);
  }
});

test("a Worker's line on the Platform screen: how it gets into service, and how its last deploy went", () => {
  const now = Date.parse("2026-11-23T12:00:00Z");
  const run = (state: string, more: Record<string, unknown> = {}) => ({
    commit: "0123456789ab",
    state,
    at: "2026-11-23T10:00:00Z",
    url: "https://github.com/x/y/actions/runs/1",
    ...more,
  });
  // One whose deploy workflow puts what it ran on into service when it succeeds.
  const from = (more: Record<string, unknown>) => ({
    state: "in service",
    deploy: "actions",
    runDeploys: true,
    run: null,
    ...more,
  });
  // One that deploys in steps: a run that succeeded may only have uploaded a version.
  const steps = (more: Record<string, unknown>) => ({ ...from(more), runDeploys: false });
  expect(workerSaid(steps({ run: run("success") }), now)).toEqual({
    text: "In service. The last run of its deploy workflow, 2 h ago, succeeded (0123456). It deploys in steps, so that run may not be what is serving.",
    tone: "",
  });
  expect(workerSaid(steps({ run: run("failure") }), now)).toEqual({
    text: "In service. The last run of its deploy workflow, 2 h ago, did not finish (failure) (0123456). It deploys in steps, so that run may not be what is serving.",
    tone: "warn",
  });
  expect(workerSaid(steps({ run: run("in_progress") }), now).text).toContain("2 h ago, is in progress (0123456)");
  expect(workerSaid(steps({ run: run("success") }), now).text).not.toContain("Last deploy");
  expect(workerSaid(from({ run: run("success") }), now)).toEqual({
    text: "In service. Last deploy 2 h ago, of 0123456.",
    tone: "good",
  });
  // One that did not finish is said, and marked.
  expect(workerSaid(from({ run: run("failure") }), now)).toEqual({
    text: "In service. Its last deploy, 2 h ago, did not finish (failure).",
    tone: "warn",
  });
  expect(workerSaid(from({ run: run("timed_out") }), now).text).toContain("did not finish (timed out)");
  // One under way is neither good nor bad yet.
  expect(workerSaid(from({ run: run("in_progress") }), now)).toEqual({
    text: "In service. A deploy of 0123456 is in progress, started 2 h ago.",
    tone: "",
  });
  expect(workerSaid(from({ run: { error: "rate limited" } }), now)).toEqual({
    text: "In service. Deployed from Actions; its last run could not be read (rate limited).",
    tone: "warn",
  });
  expect(workerSaid(from({}), now).text).toBe("In service. Deployed from Actions; it has not run on main yet.");
  // By hand, from a machine: in service, and marked as what is left to do.
  expect(workerSaid({ state: "in service", deploy: "by hand", run: null }, now)).toEqual({
    text: "In service. Deployed by hand, from a machine.",
    tone: "warn",
  });
  expect(workerSaid({ state: "not deployed", deploy: "none", run: null }, now)).toEqual({
    text: "Not deployed.",
    tone: "",
  });
  // The counts the screen opens with.
  expect(
    platformCounts([
      { state: "in service", deploy: "actions" },
      { state: "in service", deploy: "by hand" },
      { state: "in service", deploy: "by hand" },
      { state: "not deployed", deploy: "none" },
    ]),
  ).toEqual({ all: 4, live: 3, actions: 1, byHand: 2, idle: 1 });
  expect(platformCounts([])).toEqual({ all: 0, live: 0, actions: 0, byHand: 0, idle: 0 });
});

test("why something could not be read is said in the owner's words where the reason is a known one", () => {
  // As GitHub says it, through the fleet binary: seen in service on 2026-11-23.
  const limited =
    'fleet gh failed (exit 1): {"message":"API rate limit exceeded for user ID 11225574. If you reach out to GitHub Support for help, please include the request ID 312C:3E98FD:129EC6:13D870:6B047542","documentation_url":"https://docs.github.com/rest/overview/rate-limits-for-the-rest-api","status":"403"}';
  const said = whySaid(limited);
  expect(said).toBe(
    "GitHub's allowance for the hour is used up on this account, which the agents share; it is asked again every few minutes and answers when the hour turns",
  );
  // None of the raw answer is shown: no JSON, no request number, no user number.
  expect(/[{}"]|request ID|11225574/.test(said)).toBe(false);
  // Turned away for going too fast is not the hour's allowance, and is not said to be.
  const fast = "GitHub turned this account away for asking too often just now";
  expect(whySaid("You have exceeded a secondary rate limit")).toBe(`${fast}; it is asked again every few minutes`);
  // Where Cloudflare was the one turned away, why starts with the time GitHub gave, and that
  // time is said as the clock on this device shows it.
  const now = Date.UTC(2026, 10, 23, 17, 20, 0);
  const back = Date.UTC(2026, 10, 23, 18, 0, 0);
  const clock = (at: number) => new Date(at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const then = (at: number) => `GitHub says it answers again at ${clock(at)}, and it is not asked before then`;
  const asked = (at: number, why: string) => `GitHub says to ask again at ${new Date(at).toISOString()}: ${why}`;
  const hour = "GitHub's allowance for the hour is used up on this account, which the agents share";
  const timed = `fleet gh failed (exit 1): ${asked(back, limited)}`;
  expect(whySaid(timed, now)).toBe(`${hour}; ${then(back)}`);
  expect(/[{}"]|request ID|11225574|2026-11-23T/.test(whySaid(timed, now))).toBe(false);
  // What Cloudflare says of a call it did not make, GitHub having said not to ask yet: the same.
  expect(whySaid(asked(back, `not asked again since GitHub answered: ${limited}`), now)).toBe(whySaid(timed, now));
  // Read after the time has come (the notice is older than the wait): no time already past is promised.
  expect(whySaid(timed, back + 1)).toBe(
    `${hour}; the time GitHub gave for asking again has come, and it is being asked`,
  );
  expect(whySaid(timed, back)).toBe(whySaid(timed, back + 1));
  // Going too fast, with a time.
  expect(whySaid(asked(back, "You have exceeded a secondary rate limit"), now)).toBe(`${fast}; ${then(back)}`);
  // GitHub says "rate limit exceeded" of searches too, which it counts by the minute: a wait
  // of a minute or two is not the hour's allowance, whatever the words.
  expect(whySaid(asked(now + 40_000, limited), now)).toBe(`${fast}; ${then(now + 40_000)}`);
  expect(whySaid(asked(now + 120_000, limited), now)).toBe(`${fast}; ${then(now + 120_000)}`);
  expect(whySaid(asked(now + 120_001, limited), now)).toBe(`${hour}; ${then(now + 120_001)}`);
  // Turned away with a time and no words at all.
  expect(whySaid(asked(back, "GitHub answered http 429"), now)).toBe(`${fast}; ${then(back)}`);
  // A time that is not one is not read as one.
  expect(whySaid("GitHub says to ask again at teatime: API rate limit exceeded", now)).toBe(said);
  // Anything else is said as it came, and nothing is said as "no reason given".
  expect(whySaid("connection refused")).toBe("connection refused");
  expect(whySaid("  timed out \n")).toBe("timed out");
  for (const none of ["", null, undefined, "   "]) expect([none, whySaid(none)]).toEqual([none, "no reason given"]);
});

test("each list that could not be read is said on the screens that show it, with why and how old the copy is", () => {
  const now = Date.UTC(2026, 10, 23, 17, 20, 0);
  const limited =
    'fleet gh failed (exit 1): {"message":"API rate limit exceeded for user ID 11225574.","status":"403"}';
  const doc = {
    health: {
      prs: { ok: false, error: limited },
      priorities: { ok: true, error: null },
      agents: { ok: false, error: "bridge: connection refused" },
      services: { ok: false, error: "systemctl: timed out" },
      // Not read, and no reason given: nothing is made up for it.
      more: { ok: false, error: null },
    },
    // When each was last tried: just now, and failing. Not how old what is shown is.
    read: {
      prs: new Date(now - 20_000).toISOString(),
      agents: new Date(now - 20_000).toISOString(),
      services: new Date(now - 20_000).toISOString(),
    },
    // When what is shown of each was read.
    good: {
      prs: new Date(now - 12 * 60_000).toISOString(),
      agents: null,
      services: new Date(now - 2 * 3600_000).toISOString(),
    },
  };
  const hour =
    "GitHub's allowance for the hour is used up on this account, which the agents share; it is asked again every few minutes and answers when the hour turns";
  // A screen that shows all of a document's lists is told of each, in the document's order.
  expect(unreadSaid(doc, null, now)).toEqual([
    `Could not read pull requests just now: ${hour}. Showing the last good copy, read 12 min ago.`,
    "Could not read agents just now: bridge: connection refused. Showing the last good copy.",
    "Could not read the machine's units just now: systemctl: timed out. Showing the last good copy, read 2 h ago.",
  ]);
  // A screen that shows one list is told of that one, and of no other's trouble.
  expect(unreadSaid(doc, ["prs"], now)).toEqual([
    `Could not read pull requests just now: ${hour}. Showing the last good copy, read 12 min ago.`,
  ]);
  expect(unreadSaid(doc, ["priorities"], now)).toEqual([]);
  expect(unreadSaid(doc, ["agents", "services"], now)).toHaveLength(2);
  // A copy of the document from before it said when each list was read (one kept on a
  // device) gives no age, and none is made up from when reading was last tried.
  expect(unreadSaid({ health: doc.health, read: doc.read }, ["prs"], now)).toEqual([
    `Could not read pull requests just now: ${hour}. Showing the last good copy.`,
  ]);
  // A document with nothing wrong, or none yet, says nothing.
  expect(unreadSaid({ health: { prs: { ok: true, error: null } } }, null, now)).toEqual([]);
  for (const none of [null, undefined, {}]) expect(unreadSaid(none, ["prs"], now)).toEqual([]);
  // Every screen drawn from the operations document names its own list, so that none shows
  // an old copy with nothing said (the Pull requests screen did, until this was written).
  const source = readFileSync(new URL("../public/app.js", import.meta.url), "utf8");
  for (const [view, list] of [
    ["viewPrs", "prs"],
    ["viewIssues", "priorities"],
    ["viewAgents", "agents"],
    ["viewServices", "services"],
  ]) {
    const at = source.indexOf(`function ${view}(`);
    const body = source.slice(
      at,
      source.indexOf("\nfunction ", at + 10) > 0 ? source.indexOf("\nfunction ", at + 10) : undefined,
    );
    expect([view, at > 0 && body.includes(`...notices(docs.ops, ["${list}"])`)]).toEqual([view, true]);
  }
});

test("words written for a Markdown file are shown as plain words", () => {
  // A row of the PM stack's roadmap, as its file has it (seen raw on the Roadmap screen on 2026-11-24).
  expect(
    plainWords(
      "The app managed from Cloudflare. Design record: [`doc/2026-11-23T06-50-00Z_architecture_pm_platform_cloudflare_core/`](../../doc/2026-11-23T06-50-00Z_architecture_pm_platform_cloudflare_core/). What is left is tracked in one place.",
    ),
  ).toBe(
    "The app managed from Cloudflare. Design record: doc/2026-11-23T06-50-00Z_architecture_pm_platform_cloudflare_core/. What is left is tracked in one place.",
  );
  expect(plainWords("See [the plan](https://example.com/a_b) and **this** and `fleet queue list`.")).toBe(
    "See the plan and this and fleet queue list.",
  );
  // Brackets that are not a link, an issue number, and nothing at all are left as they are.
  expect(plainWords("Done [x] for #13497 (see the file)")).toBe("Done [x] for #13497 (see the file)");
  expect(plainWords("a * b * c")).toBe("a * b * c");
  expect([plainWords(""), plainWords(null), plainWords(undefined)]).toEqual(["", "", ""]);
  // And a roadmap row's words are shown that way.
  expect(readFileSync(new URL("../public/app.js", import.meta.url), "utf8")).toContain(
    'h("p", {}, plainWords(r.intent))',
  );
});

test("every screen is kept in the background only on a connection that is not metered, and only once in a while (#13501)", () => {
  // Named by the browser as Wi-Fi or a cable.
  expect(unmetered({ type: "wifi" })).toBe(true);
  expect(unmetered({ type: "ethernet", effectiveType: "2g" })).toBe(true);
  // Named as anything else: left alone, however fast.
  expect(unmetered({ type: "cellular", effectiveType: "4g" })).toBe(false);
  expect(unmetered({ type: "bluetooth" })).toBe(false);
  expect(unmetered({ type: "none" })).toBe(false);
  // Not named: a fast one only.
  expect(unmetered({ effectiveType: "4g" })).toBe(true);
  expect(unmetered({ type: "unknown", effectiveType: "4g" })).toBe(true);
  expect(unmetered({ effectiveType: "3g" })).toBe(false);
  // The owner asked to save data, or the browser says nothing at all.
  expect(unmetered({ type: "wifi", saveData: true })).toBe(false);
  expect(unmetered(undefined)).toBe(false);
  expect(unmetered(null)).toBe(false);
  // Never kept, or not for twelve hours: due. Since then: not.
  const now = 1_800_000_000_000;
  expect(KEEP_ALL_EVERY_MS).toBe(12 * 60 * 60_000);
  expect(keepAllDue(null, now)).toBe(true);
  expect(keepAllDue("", now)).toBe(true);
  expect(keepAllDue(String(now - KEEP_ALL_EVERY_MS), now)).toBe(true);
  expect(keepAllDue(String(now - KEEP_ALL_EVERY_MS + 1), now)).toBe(false);
  expect(keepAllDue(String(now - 60_000), now)).toBe(false);
  // A time that cannot be read, or one ahead of the clock, is taken as never.
  expect(keepAllDue("soon", now)).toBe(true);
  expect(keepAllDue(String(now + 60_000), now)).toBe(true);
});

// The oracle is the design's §5 (doc/2026-11-25T08-48-36Z_design_pm-owner-settings-at-edge.md):
// each setting with its value, whose it is and since when, main's beside
// an owner's change, and a way to change only the two the owner may.
test("a platform setting is said with its value, whose it is and since when, and whether the owner may change it", () => {
  const now = Date.parse("2026-11-25T12:00:00Z");
  expect(
    settingSaid(
      {
        name: "promptLabel",
        source: "owner",
        since: now - 20 * 60_000,
        value: "owner/now",
        repository: "owner/prompt",
        by: "owner@example.com",
        rev: 2,
      },
      now,
    ),
  ).toEqual({
    name: "promptLabel",
    what: "The label that marks an instruction",
    value: "owner/now",
    whose: "Yours: changed 20 min ago by owner@example.com.",
    main: "Main says owner/prompt.",
    changeable: true,
    changed: true,
  });
  // Main's, and changeable: no main's beside it, since it is main's.
  expect(
    settingSaid({ name: "needsRepos", source: "repository", since: null, value: ["a/b", "c/d"], repository: ["a/b", "c/d"] }, now),
  ).toMatchObject({ value: "a/b and c/d", whose: "Main's.", main: null, changeable: true, changed: false });
  // A machine's list, for now.
  expect(settingSaid({ name: "needsRepos", source: "machine", since: null, value: ["a/b"] }, now).whose).toBe(
    "The machine's: Cloudflare goes by the list it reports until it runs main's.",
  );
  // Main's alone decides it: nothing to press.
  expect(settingSaid({ name: "queue", source: "repository", since: null, value: "repo-health-code-pilot" }, now)).toMatchObject({
    what: "The queue",
    changeable: false,
    changed: false,
  });
  // What is not one is not a crash.
  expect(settingSaid(null, now)).toMatchObject({ what: "A setting", value: "nothing", changeable: false });
});

test("a list of repositories is read as typed: commas, spaces or lines between them", () => {
  expect(reposTyped("a/b, c/d\ne/f  g/h,,")).toEqual(["a/b", "c/d", "e/f", "g/h"]);
  expect(reposTyped("")).toEqual([]);
  expect(reposTyped(undefined)).toEqual([]);
});

test("a machine that has not taken the owner's latest change to the settings is said to, and one that has is not", () => {
  // The owner has changed none: nothing to say, whatever the machine says.
  expect(settingsTakenSaid(0, 0)).toBe("");
  expect(settingsTakenSaid(null, 0)).toBe("");
  expect(settingsTakenSaid(undefined, undefined)).toBe("");
  // It runs with the latest.
  expect(settingsTakenSaid(3, 3)).toBe("");
  // It is behind: it takes it with its next report.
  expect(settingsTakenSaid(2, 3)).toBe("It has not taken your latest change to the settings yet: it will with its next report.");
  expect(settingsTakenSaid(0, 1)).toBe("It has not taken your latest change to the settings yet: it will with its next report.");
  // Its console does not take them at all.
  expect(settingsTakenSaid(null, 1)).toBe("Its console is older than changing the settings from here: it runs with its own.");
  expect(settingsTakenSaid(undefined, 1)).toBe("Its console is older than changing the settings from here: it runs with its own.");
});
