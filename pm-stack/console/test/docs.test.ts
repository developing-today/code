// The documents, and being told when one of them has changed: what a
// holder with someone connected (the Worker, a live connection open to
// the app) passes on, so the app reads again at once and not at its
// next look.
import { describe, expect, test } from "bun:test";
import { type DocName, Docs, Stamped } from "../src/docs.ts";

const SECTIONS = [
  "board",
  "sessions",
  "census",
  "asks",
  "questionCount",
  "questions",
  "questionSets",
  "prs",
  "priorities",
  "done",
  "agents",
  "services",
  "more",
];

function world() {
  const now = { t: 1_000_000 };
  const data: Record<string, unknown> = Object.fromEntries(SECTIONS.map((name) => [name, { n: 0 }]));
  const failing = new Set<string>();
  const docs = new Docs(
    Object.fromEntries(
      SECTIONS.map((name) => [
        name,
        {
          ttlMs: 10_000,
          load: async () => {
            if (failing.has(name)) throw new Error(`${name}: the source is down`);
            return structuredClone(data[name]);
          },
        },
      ]),
    ),
    () => now.t,
  );
  const told: DocName[][] = [];
  docs.onChange = (names) => told.push([...names].sort());
  return { now, data, failing, docs, told };
}

describe("being told when a document has changed", () => {
  // The oracle is the document's own tag: whenever a read changes it,
  // the holder was told of that document, and when it does not, it was not.
  test("a section read with other content: the documents it is part of, and no other", async () => {
    const w = world();
    await w.docs.touch(SECTIONS);
    w.told.length = 0;
    const before = Object.fromEntries(
      await Promise.all((["core", "questions", "ops"] as DocName[]).map(async (d) => [d, (await w.docs.doc(d)).etag])),
    );
    w.data.board = { n: 1 };
    w.data.prs = { n: 1 };
    await w.docs.reread(["board", "prs", "questions"]);
    expect(w.told.flat().sort()).toEqual(["core", "ops"]);
    const after = Object.fromEntries(
      await Promise.all((["core", "questions", "ops"] as DocName[]).map(async (d) => [d, (await w.docs.doc(d)).etag])),
    );
    expect([after.core !== before.core, after.ops !== before.ops, after.questions === before.questions]).toEqual([
      true,
      true,
      true,
    ]);
  });

  test("read again with the same content, nobody is told", async () => {
    const w = world();
    await w.docs.touch(SECTIONS);
    w.told.length = 0;
    await w.docs.reread(SECTIONS);
    await w.docs.reread(SECTIONS);
    expect(w.told).toEqual([]);
  });

  test("the first read of a section is a change: there was nothing, and now there is something", async () => {
    const w = world();
    await w.docs.reread(["questions"]);
    expect(w.told).toEqual([["questions"]]);
  });

  test("a read that fails, and one that works again, each change what the document says of its health", async () => {
    const w = world();
    await w.docs.touch(SECTIONS);
    w.told.length = 0;
    w.failing.add("asks");
    await w.docs.reread(["asks"]);
    expect(w.told).toEqual([["core"]]);
    // Failing still, with the same words: nothing new to say.
    await w.docs.reread(["asks"]);
    expect(w.told).toEqual([["core"]]);
    w.failing.delete("asks");
    await w.docs.reread(["asks"]);
    expect(w.told).toEqual([["core"], ["core"]]);
  });

  test("what travels beside the sections in the core document: a value set to something else, and what the owner did", async () => {
    const w = world();
    await w.docs.touch(SECTIONS);
    w.told.length = 0;
    w.docs.extra("edge", { held: 0 });
    w.docs.extra("edge", { held: 0 });
    expect(w.told).toEqual([["core"]]);
    w.docs.extra("edge", { held: 1 });
    w.docs.extra("edge", undefined);
    w.docs.extra("edge", undefined);
    expect(w.told).toHaveLength(3);
    w.told.length = 0;
    const act = {
      id: "a1",
      kind: "task.move",
      at: "2026-11-24T08:00:00Z",
      by: "o",
      status: "running" as const,
      note: "",
    };
    w.docs.logAct(act);
    w.docs.logAct(act);
    w.docs.logAct({ ...act, status: "done" });
    expect(w.told).toEqual([["core"], ["core"]]);
    w.docs.restoreActs(w.docs.recentActs());
    expect(w.told).toHaveLength(2);
    w.docs.restoreActs([]);
    expect(w.told).toHaveLength(3);
  });

  test("a copy read somewhere else and handed over is a change when it is another copy, whenever it was read", async () => {
    const now = { t: 5_000_000 };
    let copy: Stamped = new Stamped({ n: 1 }, 4_000_000);
    const docs = new Docs(
      Object.fromEntries(SECTIONS.map((name) => [name, { ttlMs: 10_000, load: async () => copy }])),
      () => now.t,
    );
    await docs.touch(SECTIONS);
    const told: DocName[][] = [];
    docs.onChange = (names) => told.push([...names]);
    // The same copy, said to be read later: the document says the same.
    copy = new Stamped({ n: 1 }, 4_500_000);
    await docs.reread(["board"]);
    expect(told).toEqual([]);
    copy = new Stamped({ n: 2 }, 4_500_000);
    await docs.reread(["board"]);
    expect(told).toEqual([["core"]]);
  });

  test("with nobody to tell, nothing is different", async () => {
    const w = world();
    w.docs.onChange = null;
    await w.docs.touch(SECTIONS);
    w.docs.extra("edge", { held: 1 });
    expect(JSON.parse((await w.docs.doc("core")).body).edge).toEqual({ held: 1 });
  });
});
