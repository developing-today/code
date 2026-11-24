// What a machine and a device say to the platform, read by its parsers.
//
// Oracles: for a machine's name, the format is written out again here
// (a letter or digit first, only letters, digits, blanks, dots,
// underscores and hyphens after, no blank last, 63 at most) and held
// against whatever a generator feeds in. For a machine's account of
// itself, a generator builds accounts that keep every rule (so what is
// good is what the generator built, not what the parser says) and a
// fault is one change known to break one rule. For what an action is
// about, the platform's function and the device's are each the other's
// reference: they are two copies of one rule (src/edge-proto.ts and
// public/outbox.js), and the order of two actions is only kept if they
// agree on every action, so they are compared on generated ones.
import { expect, test } from "bun:test";
import "../public/outbox.js";
import { HOLDABLE, machineName, machineReport, subjectOf } from "../src/edge-proto.ts";
import { letters, pick, rng } from "./support.ts";

const O = (globalThis as unknown as { PMOutbox: { subject(kind: string, args?: unknown): string | null } }).PMOutbox;

const LOWER = "abcdefghijklmnopqrstuvwxyz";
const DIGIT = "0123456789";
const ALNUM = `${LOWER}${LOWER.toUpperCase()}${DIGIT}`;
/** The format of a name fit to show, said again. */
const fit = (name: string) =>
  name.length >= 1 &&
  name.length <= 63 &&
  ALNUM.includes(name[0]) &&
  [...name].every((ch) => `${ALNUM} ._-`.includes(ch)) &&
  name.trim() === name;

test("property: whatever a machine is called, its name comes out fit to show or empty, and stays as it is once it is", () => {
  const next = rng(13471);
  let fitted = 0;
  let empty = 0;
  for (let i = 0; i < 8000; i++) {
    const called: unknown =
      next() < 0.9
        ? letters(next, `${ALNUM}${ALNUM}   ..__--'’é日/\\:\t\n `, 0, pick(next, [3, 12, 70, 140]))
        : pick<unknown>(next, [null, undefined, 0, 7, -1.5, true, [], ["a", "b"], {}, { name: "x" }]);
    const name = machineName(called);
    if (name === "") empty++;
    else {
      fitted++;
      expect([called, name, fit(name)]).toEqual([called, name, true]);
    }
    // The words of what it is called are the words of the name, in
    // order, as far as the name goes (the last may be cut short): none
    // is lost before the cut, none joined to its neighbour, and a name
    // is empty only when there is no letter or digit to show.
    const words = (text: string) => text.match(/[A-Za-z0-9]+/g) ?? [];
    const [had, has] = [words(String(called ?? "")), words(name)];
    expect([called, name === ""]).toEqual([called, had.length === 0]);
    expect([called, has.slice(0, -1)]).toEqual([called, had.slice(0, Math.max(0, has.length - 1))]);
    if (has.length > 0)
      expect([called, name, had[has.length - 1].startsWith(has[has.length - 1])]).toEqual([called, name, true]);
    // Blanks around what it is called make no difference.
    if (typeof called === "string")
      expect([called, machineName(`${letters(next, " \t\n", 0, 3)}${called}${letters(next, " \t\n", 0, 3)}`)]).toEqual([
        called,
        name,
      ]);
    // Said again of its own answer, it answers the same.
    expect([called, machineName(name)]).toEqual([called, name]);
    // And a name that is already fit is left exactly as it is.
    if (typeof called === "string" && fit(called)) expect(name).toBe(called);
  }
  expect([fitted > 1000, empty > 100]).toEqual([true, true]);
  // The cut at 63 that lands after a blank, by hand: this is the case
  // the property found.
  expect(machineName(`${"x".repeat(62)} yz`)).toBe("x".repeat(62));
});

const kind = (next: () => number) =>
  `${pick(next, [...LOWER, ...LOWER.toUpperCase()])}${letters(next, `${ALNUM}._-`, 0, 47)}`;
const kinds = (next: () => number, most: number) =>
  Array.from({ length: Math.floor(next() * (most + 1)) }, () => kind(next));
const sortedOnce = (list: string[]) => [...new Set(list)].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));

/** One change to a good account, each known to break one rule. */
const FAULTS: [string, (said: Record<string, unknown>, next: () => number) => unknown][] = [
  ["no name", ({ name: _name, ...rest }) => rest],
  ["a name that is not text", (said, next) => ({ ...said, name: pick<unknown>(next, [null, 7, ["nixos"], {}]) })],
  [
    "a name not fit to show",
    (said, next) => ({
      ...said,
      name: pick(next, ["", " nixos", "nixos ", "Drew's laptop", "-nixos", "x".repeat(64), "a/b"]),
    }),
  ],
  ["no list of what it can do", ({ actions: _actions, ...rest }) => rest],
  ["no list of what it reads", ({ sections: _sections, ...rest }) => rest],
  [
    "a list that is not one",
    (said, next) => ({
      ...said,
      [pick(next, ["actions", "sections"])]: pick<unknown>(next, [null, "task.move", 7, { 0: "task.move" }]),
    }),
  ],
  [
    "something in a list that is not the name of a thing",
    (said, next) => {
      const which = pick(next, ["actions", "sections"]);
      const list = [...(said[which] as string[])];
      list.splice(
        Math.floor(next() * (list.length + 1)),
        0,
        pick<unknown>(next, ["", "1st", "task move", ".hidden", "x".repeat(49), 7, null, ["a"], "a/b", "é"]) as string,
      );
      return { ...said, [which]: list };
    },
  ],
  [
    "a list longer than any machine's",
    (said, next) => ({
      ...said,
      [pick(next, ["actions", "sections"])]: Array.from({ length: 201 + Math.floor(next() * 20) }, (_, n) => `k${n}`),
    }),
  ],
];

test("property: a machine's account of itself is read back in order with each thing once, and one fault refuses it", () => {
  const next = rng(20261123);
  for (let i = 0; i < 1500; i++) {
    const said = {
      name: `${pick(next, [...ALNUM])}${next() < 0.5 ? "" : `${letters(next, `${ALNUM} ._-`, 0, 61)}${pick(next, [...ALNUM, ".", "_", "-"])}`}`,
      // Some twice over, and in no order.
      actions: kinds(next, 30).flatMap((k) => (next() < 0.2 ? [k, k] : [k])),
      sections: kinds(next, 12).flatMap((k) => (next() < 0.2 ? [k, k] : [k])),
    };
    expect(fit(said.name)).toBe(true); // the generator's names are fit by the format said above
    const want = { name: said.name, actions: sortedOnce(said.actions), sections: sortedOnce(said.sections) };
    expect([said, machineReport(said)]).toEqual([said, want]);
    expect(machineReport(want)).toEqual(want);
    // More said than is asked for is not carried along.
    expect(machineReport({ ...said, secret: "x", version: 7 })).toEqual(want);
    for (const [why, fault] of FAULTS) {
      const bad = fault(said, next);
      expect([why, bad, machineReport(bad)]).toEqual([why, bad, null]);
    }
  }
  // At the limit and one over it, exactly.
  const full = Array.from({ length: 200 }, (_, n) => `k${n}`);
  expect(machineReport({ name: "nixos", actions: full, sections: [] })?.actions.length).toBe(200);
  expect(machineReport({ name: "nixos", actions: [...full, "one.more"], sections: [] })).toBeNull();
  // The limit is on what is sent, not on what is left once each is counted once.
  expect(machineReport({ name: "nixos", actions: [...full, "k0"], sections: [] })).toBeNull();
  for (const odd of [null, undefined, 7, "nixos", true, [], [["nixos"]]])
    expect([odd, machineReport(odd)]).toEqual([odd, null]);
});

test("property: the device and the platform say the same of what any action is about", () => {
  const next = rng(13398);
  const known = [
    "ask.reply",
    "ask.close",
    "issue.comment",
    "issue.rank",
    "task.move",
    "task.cancel",
    "task.unblock",
    "hands.done",
    "need.comment",
    "question.answer",
    "question.defer",
    "questions.accept",
    "pr.merge",
    "unit.act",
    ...HOLDABLE,
    "ask.send",
    "intake.send",
    "prompt.enqueue",
    "push.subscribe",
    "held.drop",
  ];
  const value = (): unknown =>
    pick<unknown>(next, [
      undefined,
      null,
      "",
      " ",
      0,
      7,
      201,
      -3,
      1.5,
      Number.NaN,
      "7",
      "201",
      "developing-today/code#3",
      "s1",
      "fleet-queue.service",
      true,
      false,
      [],
      [7],
      {},
      { toString: () => "7" },
      letters(next, `${ALNUM} #/.-`, 1, 20),
    ]);
  const about = new Map<string, number>();
  let nothing = 0;
  for (let i = 0; i < 20000; i++) {
    const what =
      next() < 0.9
        ? pick(next, known)
        : next() < 0.5
          ? kind(next)
          : pick(next, ["", "toString", "__proto__", "constructor"]);
    const args: unknown =
      next() < 0.92
        ? Object.fromEntries(
            ["issue", "task", "ref", "set", "pr", "unit", "text", "qid"]
              .filter(() => next() < 0.5)
              .map((key) => [key, value()]),
          )
        : pick<unknown>(next, [undefined, null, "issue", 7, [], [{ issue: 7 }], true]);
    const here = O.subject(what, args);
    const there = subjectOf(what, args as Record<string, unknown>);
    expect([what, args, here]).toEqual([what, args, there]);
    if (there === null) nothing++;
    else {
      // What it is about is said as a kind of thing and which one, and
      // which one is never empty.
      expect(/^(issue|task|ref|questions|pr|unit):.+$/s.test(there)).toBe(true);
      about.set(there.split(":")[0], (about.get(there.split(":")[0]) ?? 0) + 1);
    }
  }
  // A number and the same number written as text are one thing.
  expect([subjectOf("issue.rank", { issue: 201 }), O.subject("issue.rank", { issue: "201" })]).toEqual([
    "issue:201",
    "issue:201",
  ]);
  // The generator reaches every kind of thing, and actions about nothing.
  expect([...about.keys()].sort()).toEqual(["issue", "pr", "questions", "ref", "task", "unit"]);
  expect([[...about.entries()].filter(([, n]) => n < 100), nothing > 1000]).toEqual([[], true]);
  // Everything the platform may hold for a machine is something both sides know how to place.
  for (const held of HOLDABLE) expect([held, O.subject(held, {}), subjectOf(held, {})]).toEqual([held, null, null]);
});
