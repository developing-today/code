// What the owner did, kept on the device until the platform has it.
//
// Oracles: the rules are written out here by hand for each case: which
// actions wait, which are sent only while fresh, which are never kept;
// what each kind of answer does to an entry; in what order things are
// sent, and when sending stops. The store is the file's own in-memory
// one, and the sender is a stand-in that answers as a test says. That a
// real browser keeps these across a reload and a real platform takes
// each once is checked in a browser and in test/edge.test.ts.
import { expect, test } from "bun:test";
import "../public/outbox.js";

interface Entry {
  id: string;
  kind: string;
  args: Record<string, unknown>;
  said: string;
  at: number;
  state: string;
  note: string;
  tries: number;
  sure?: boolean;
  endedAt?: number;
  triedAt?: number;
  result?: unknown;
}
type Answer = { network?: boolean; signin?: boolean; status?: number; body?: unknown };
interface Store {
  all(): Promise<Entry[]>;
  put(entry: Entry): Promise<void>;
  remove(id: string): Promise<void>;
}
const O = (globalThis as unknown as { PMOutbox: Record<string, unknown> }).PMOutbox as {
  TIMELY_MS: number;
  rule(kind: string): "late" | "timely" | "now";
  sayOf(kind: string, args?: unknown): string;
  make(kind: string, args: unknown, now: number, said?: string, id?: string): Entry;
  classify(answer: Answer): { state: string; note: string; stop?: boolean; signin?: boolean; result?: unknown };
  due(entry: Entry, now: number): boolean;
  summary(entries: Entry[]): Record<string, number>;
  expired(entries: Entry[], now: number): string[];
  flush(
    store: Store,
    send: (entry: Entry) => Promise<Answer>,
    now?: () => number,
  ): Promise<{ changed: Entry[]; signin: boolean; offline: boolean }>;
  settle(store: Store, acts: unknown, now?: () => number): Promise<Entry[]>;
  httpSend(entry: Entry, fetchImpl: (url: string, init: RequestInit) => Promise<unknown>): Promise<Answer>;
  memoryStore(): Store;
};

const T0 = 1_800_000_000_000;
const MIN = 60_000;
const ok = (result: unknown = {}): Answer => ({ status: 200, body: { ok: true, result } });

test("what may wait, what is sent only while it is fresh, and what needs a connection", () => {
  // What the owner says or decides does not go stale.
  for (const kind of [
    "ask.send",
    "ask.reply",
    "ask.close",
    "question.answer",
    "question.defer",
    "questions.accept",
    "hands.done",
    "need.comment",
    "issue.comment",
    "issue.rank",
    "task.move",
    "message.send",
    "intake.send",
  ]) {
    expect([kind, O.rule(kind)]).toEqual([kind, "late"]);
  }
  // These act on something that may have changed by the time they go.
  for (const kind of ["pr.merge", "deploy.start", "task.cancel", "task.unblock", "unit.act"]) {
    expect([kind, O.rule(kind)]).toEqual([kind, "timely"]);
  }
  // These set the connection up or carry a secret, and anything this
  // file has not heard of is not kept on a guess.
  for (const kind of [
    "sso.google",
    "push.on",
    "push.off",
    "push.class",
    "push.test",
    "device.approve",
    "device.remove",
    "edge.direct",
    "edge.retry",
    "something.new",
    "",
  ]) {
    expect([kind, O.rule(kind)]).toEqual([kind, "now"]);
  }
  expect(O.TIMELY_MS).toBe(10 * MIN);
});

test("each action is said in a line the owner will know it by", () => {
  const said = (kind: string, args: unknown) => O.sayOf(kind, args);
  expect(said("ask.send", { text: "Ship the   parser\nrework this week" })).toBe(
    "Prompt: “Ship the parser rework this week”",
  );
  expect(said("ask.send", { text: "x".repeat(200) })).toBe(`Prompt: “${"x".repeat(59)}…”`);
  expect(said("ask.reply", { issue: 201, text: "Yes" })).toBe("Reply on #201: “Yes”");
  expect(said("ask.close", { issue: 202 })).toBe("Mark prompt #202 done");
  expect(said("issue.comment", { issue: 301, text: "" })).toBe("Comment on #301");
  expect(said("issue.rank", { issue: 301, rank: "next" })).toBe("Rank #301 next");
  expect(said("need.comment", { ref: "developing-today/code#9", text: "Done" })).toBe(
    "Reply on developing-today/code#9: “Done”",
  );
  expect(said("hands.done", { ref: "developing-today/code#9" })).toBe("Mark developing-today/code#9 done");
  expect(said("task.move", { task: 21048, to: "first" })).toBe("Move task 21048 to the top");
  expect(said("task.move", { task: 21048, to: "later" })).toBe("Move task 21048 later");
  expect(said("task.cancel", { task: 7 })).toBe("Cancel task 7");
  expect(said("task.unblock", { task: 7 })).toBe("Unblock task 7");
  expect(said("pr.merge", { pr: 55 })).toBe("Merge #55");
  expect(said("deploy.start", { what: "site" })).toBe("Start the site's deploy");
  expect(said("deploy.start", { what: "app" })).toBe("Start the app's build");
  expect(said("intake.send", { kind: "bug", text: "It crashes" })).toBe("Bug: “It crashes”");
  expect(said("intake.send", { kind: "note", text: "Think about X" })).toBe("Note: “Think about X”");
  expect(said("intake.send", { kind: "change", text: "Move it" })).toBe("Change: “Move it”");
  expect(said("intake.send", { kind: "note", session: true, text: "Do it now" })).toBe("Session: “Do it now”");
  expect(said("question.answer", { answer: "yes" })).toBe("Answer: “yes”");
  expect(said("question.answer", { answer: ["a", "b"] })).toBe("Answer: “a, b”");
  expect(said("question.defer", {})).toBe("Put a question off");
  expect(said("questions.accept", { set: "s" })).toBe("Accept the recommended answers");
  expect(said("message.send", { to: ["xaltha"], text: "hello" })).toBe("Message to xaltha: “hello”");
  expect(said("unit.act", { unit: "pm-console.service", action: "restart" })).toBe("restart pm-console.service");
  // One it does not know is still said, as it came; and no arguments are no reason to fail.
  expect(said("something.new", { a: 1 })).toBe("something.new");
  expect(O.sayOf("ask.close")).toBe("Mark prompt #undefined done");
});

test("an entry is the action as it stood, under a name of its own", () => {
  const a = O.make("pr.merge", { pr: 55 }, T0);
  const b = O.make("pr.merge", { pr: 55 }, T0);
  expect(a).toEqual({
    id: a.id,
    kind: "pr.merge",
    args: { pr: 55 },
    said: "Merge #55",
    at: T0,
    state: "waiting",
    note: "",
    tries: 0,
  });
  // The name is the kind the platform takes: a UUID in lower case.
  expect(a.id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/);
  expect(a.id).not.toBe(b.id);
  expect(
    O.make("ask.send", { text: "x" }, T0, "Said by the caller", "0a000000-0000-4000-8000-000000000001"),
  ).toMatchObject({
    said: "Said by the caller",
    id: "0a000000-0000-4000-8000-000000000001",
  });
  expect(O.make("ask.close", undefined, T0).args).toEqual({});
});

test("what each kind of answer does to an entry", () => {
  // No answer at all: it waits, and so will the rest.
  expect(O.classify({ network: true })).toEqual({ state: "waiting", note: "", stop: true });
  // Done, with what it answered.
  expect(O.classify(ok({ issue: 500 }))).toEqual({ state: "done", note: "", result: { issue: 500 } });
  expect(O.classify({ status: 200, body: { ok: true } })).toEqual({ state: "done", note: "", result: null });
  // Taken, and kept by the platform for a machine.
  expect(O.classify({ status: 200, body: { ok: true, held: true, result: null } })).toEqual({
    state: "held",
    note: "Kept for the laptop, which will do it when it is back.",
    result: null,
  });
  // Refused or failed, in the platform's own words: not tried again by itself.
  expect(O.classify({ status: 404, body: { error: "no such issue" } })).toEqual({
    state: "failed",
    note: "no such issue",
  });
  expect(O.classify({ status: 502, body: { error: "GitHub said no" } })).toEqual({
    state: "failed",
    note: "GitHub said no",
  });
  expect(O.classify({ status: 400, body: {} })).toEqual({ state: "failed", note: "http 400" });
  // A 200 that does not say it worked did not work.
  expect(O.classify({ status: 200, body: { error: "odd" } })).toEqual({ state: "failed", note: "odd" });
  // Being done now under this name, or nothing to take it just now: it
  // waits, and the others are still tried.
  expect(O.classify({ status: 409, body: { error: "This is still being done.", running: true } })).toEqual({
    state: "waiting",
    note: "Being done now.",
  });
  expect(O.classify({ status: 409, body: { error: "conflict" } })).toEqual({ state: "failed", note: "conflict" });
  expect(
    O.classify({
      status: 503,
      body: {
        error:
          "The laptop did not answer, and this is not something to do late, so it is not kept. Check whether it happened before trying again.",
      },
    }),
  ).toEqual({ state: "waiting", note: "Nothing could take it just now. It is offered again." });
  // Not the platform's answer (a sign-in page, something in between):
  // it waits for a sign-in, and so will the rest.
  for (const answer of [
    { signin: true, status: 0, body: null },
    { status: 200, body: null },
    { status: 200, body: "html" },
  ]) {
    expect(O.classify(answer)).toEqual({
      state: "waiting",
      note: "Sign in again to send this.",
      stop: true,
      signin: true,
    });
  }
});

test("a timely action is sent by itself only while it is fresh, or once the owner has looked again", () => {
  const merge = O.make("pr.merge", { pr: 55 }, T0);
  expect(O.due(merge, T0)).toBe(true);
  expect(O.due(merge, T0 + 10 * MIN)).toBe(true);
  expect(O.due(merge, T0 + 10 * MIN + 1)).toBe(false);
  expect(O.due({ ...merge, sure: true }, T0 + 24 * 60 * MIN)).toBe(true);
  // What may wait is due however old it is.
  expect(O.due(O.make("ask.send", { text: "x" }, T0), T0 + 30 * 24 * 60 * MIN)).toBe(true);
  // What is not waiting is not due.
  for (const state of ["done", "failed", "held", "stale"]) {
    expect(O.due({ ...O.make("ask.send", {}, T0), state }, T0)).toBe(false);
  }
});

test("what is waiting is sent oldest first, each once, and the list says what became of each", async () => {
  const store = O.memoryStore();
  const first = O.make("ask.send", { text: "first" }, T0);
  const second = O.make("issue.rank", { issue: 301, rank: "next" }, T0 + 1_000);
  const third = O.make("issue.comment", { issue: 13, text: "x" }, T0 + 2_000);
  const fourth = O.make("ask.reply", { issue: 201, text: "held" }, T0 + 3_000);
  // Put out of order: the order is when they were done.
  for (const e of [third, first, fourth, second]) await store.put(e);
  const sent: string[] = [];
  const send = async (e: Entry): Promise<Answer> => {
    sent.push(e.kind);
    if (e.kind === "issue.comment") return { status: 404, body: { error: "no such issue" } };
    if (e.kind === "ask.reply") return { status: 200, body: { ok: true, held: true, result: null } };
    return ok({ n: sent.length });
  };
  const now = T0 + 60_000;
  const out = await O.flush(store, send, () => now);
  expect(sent).toEqual(["ask.send", "issue.rank", "issue.comment", "ask.reply"]);
  expect(out.changed.map((e) => [e.kind, e.state, e.note])).toEqual([
    ["ask.send", "done", ""],
    ["issue.rank", "done", ""],
    ["issue.comment", "failed", "no such issue"],
    ["ask.reply", "held", "Kept for the laptop, which will do it when it is back."],
  ]);
  expect([out.signin, out.offline]).toEqual([false, false]);
  const kept = (await store.all()).sort((a, b) => a.at - b.at);
  expect(kept.map((e) => [e.state, e.tries, e.triedAt, e.endedAt])).toEqual([
    ["done", 1, now, now],
    ["done", 1, now, now],
    ["failed", 1, now, now],
    ["held", 1, now, now],
  ]);
  expect(kept[0].result).toEqual({ n: 1 });
  // Nothing is waiting now: a second flush sends nothing.
  expect((await O.flush(store, send, () => now)).changed).toEqual([]);
  expect(sent).toHaveLength(4);
  expect(O.summary(kept)).toEqual({ waiting: 0, stale: 0, held: 1, failed: 1, done: 2, open: 1 });
});

test("with no connection nothing is lost and nothing is tried twice in a row; with one, it goes", async () => {
  const store = O.memoryStore();
  await store.put(O.make("ask.send", { text: "one" }, T0));
  await store.put(O.make("ask.send", { text: "two" }, T0 + 1));
  let tries = 0;
  const down = async (): Promise<Answer> => {
    tries++;
    throw new TypeError("Failed to fetch");
  };
  const out = await O.flush(store, down, () => T0 + 5);
  // The first got no answer: the second is not tried, it would fare no better.
  expect(tries).toBe(1);
  expect([out.changed, out.offline, out.signin]).toEqual([[], true, false]);
  expect((await store.all()).map((e) => [e.state, e.tries])).toEqual([
    ["waiting", 0],
    ["waiting", 0],
  ]);
  // The sign-in ran out: the same, and it is said why.
  const page = await O.flush(
    store,
    async () => ({ signin: true, status: 0, body: null }),
    () => T0 + 6,
  );
  expect([page.signin, page.offline, page.changed.map((e) => [e.state, e.note])]).toEqual([
    true,
    false,
    [["waiting", "Sign in again to send this."]],
  ]);
  // Back: both go, in order.
  const texts: unknown[] = [];
  const up = await O.flush(
    store,
    async (e) => {
      texts.push(e.args.text);
      return ok();
    },
    () => T0 + 7,
  );
  expect(texts).toEqual(["one", "two"]);
  expect(up.changed.map((e) => e.state)).toEqual(["done", "done"]);
});

test("one that nothing can take just now does not hold up the ones behind it", async () => {
  const store = O.memoryStore();
  await store.put(O.make("message.send", { to: ["a"], text: "needs the laptop" }, T0));
  await store.put(O.make("ask.send", { text: "needs nobody" }, T0 + 1));
  const out = await O.flush(
    store,
    async (e) =>
      e.kind === "message.send"
        ? { status: 503, body: { error: "The laptop cannot be reached, and too much is already waiting for it." } }
        : ok(),
    () => T0 + 2,
  );
  expect(out.changed.map((e) => [e.kind, e.state, e.note])).toEqual([
    ["message.send", "waiting", "Nothing could take it just now. It is offered again."],
    ["ask.send", "done", ""],
  ]);
  // Tried again and told the same, nothing is written again.
  const again = await O.flush(
    store,
    async () => ({
      status: 503,
      body: { error: "The laptop cannot be reached, and too much is already waiting for it." },
    }),
    () => T0 + 3,
  );
  expect(again.changed).toEqual([]);
});

test("a merge asked for a while ago is kept back, not sent, until the owner says it still stands", async () => {
  const store = O.memoryStore();
  const merge = O.make("pr.merge", { pr: 55 }, T0);
  const cancel = O.make("task.cancel", { task: 7, reason: "dup" }, T0 + 9 * MIN);
  await store.put(merge);
  await store.put(cancel);
  const sent: string[] = [];
  const send = async (e: Entry) => {
    sent.push(e.kind);
    return ok();
  };
  // Eleven minutes after the merge was asked for, two after the cancel.
  const out = await O.flush(store, send, () => T0 + 11 * MIN);
  expect(sent).toEqual(["task.cancel"]);
  expect(out.changed.map((e) => [e.kind, e.state, e.note])).toEqual([
    ["pr.merge", "stale", "Not sent: it was a while ago, and this may have changed."],
    ["task.cancel", "done", ""],
  ]);
  // Kept back is not waiting: it is not sent by a later flush either.
  await O.flush(store, send, () => T0 + 12 * MIN);
  expect(sent).toEqual(["task.cancel"]);
  // The owner looks again and says it still stands.
  const stale = (await store.all()).find((e) => e.kind === "pr.merge") as Entry;
  await store.put({ ...stale, state: "waiting", sure: true, note: "" });
  await O.flush(store, send, () => T0 + 5 * 60 * MIN);
  expect(sent).toEqual(["task.cancel", "pr.merge"]);
});

test("two flushes at once are one flush: nothing is sent twice", async () => {
  const store = O.memoryStore();
  await store.put(O.make("ask.send", { text: "once" }, T0));
  let open: () => void = () => {};
  const gate = new Promise<void>((r) => {
    open = r;
  });
  let sends = 0;
  const send = async () => {
    sends++;
    await gate;
    return ok();
  };
  const a = O.flush(store, send, () => T0 + 1);
  const b = O.flush(store, send, () => T0 + 1);
  open();
  expect(await a).toBe(await b);
  expect(sends).toBe(1);
  // And when it is over, the next is a flush of its own.
  await store.put(O.make("ask.send", { text: "again" }, T0 + 2));
  await O.flush(store, send, () => T0 + 3);
  expect(sends).toBe(2);
  // Another store is another matter.
  const other = O.memoryStore();
  await other.put(O.make("ask.send", { text: "elsewhere" }, T0));
  await O.flush(other, send, () => T0 + 3);
  expect(sends).toBe(3);
});

test("what is finished is shown for a day and then dropped; what is not finished is never dropped", async () => {
  const done = (n: number, endedAt: number): Entry => ({ ...O.make("ask.send", { n }, T0), state: "done", endedAt });
  const day = 24 * 60 * MIN;
  const entries = [
    done(1, T0),
    done(2, T0 + day),
    { ...O.make("ask.send", {}, T0 - 30 * day), state: "waiting" },
    { ...O.make("pr.merge", {}, T0 - 30 * day), state: "stale" },
    { ...O.make("ask.send", {}, T0 - 30 * day), state: "failed" },
    { ...O.make("ask.reply", {}, T0), state: "held", endedAt: T0 },
    { ...O.make("ask.reply", {}, T0), state: "held", endedAt: T0 + 3 * 60 * MIN },
  ];
  // A day and a bit after the first was done: it goes, the rest stay.
  expect(O.expired(entries, T0 + day + 1)).toEqual([entries[0].id]);
  // What the platform took for a machine more than 26 hours ago has its outcome on the platform's record.
  expect(O.expired(entries, T0 + 26 * 60 * MIN + 1)).toEqual([entries[0].id, entries[5].id]);
  // No more than thirty finished ones, newest kept.
  const many = Array.from({ length: 33 }, (_, i) => done(i, T0 + i));
  expect(O.expired(many, T0 + 100)).toEqual([many[2].id, many[1].id, many[0].id]);
  // A flush does the dropping.
  const store = O.memoryStore();
  for (const e of entries) await store.put(e);
  await O.flush(
    store,
    async () => ({ network: true }),
    () => T0 + 26 * 60 * MIN + 1,
  );
  expect((await store.all()).map((e) => e.state).sort()).toEqual(["done", "failed", "held", "stale", "waiting"]);
});

test("what the platform kept for a machine learns how it came out from the platform's own record", async () => {
  const store = O.memoryStore();
  const a = { ...O.make("ask.send", { text: "a" }, T0), state: "held" };
  const b = { ...O.make("ask.reply", { issue: 1, text: "b" }, T0 + 1), state: "held" };
  const c = { ...O.make("issue.rank", { issue: 2, rank: "now" }, T0 + 2), state: "held" };
  const d = { ...O.make("ask.send", { text: "d" }, T0 + 3), state: "waiting" };
  for (const e of [a, b, c, d]) await store.put(e);
  const acts = [
    { id: a.id, kind: "ask.send", status: "done" },
    { id: b.id, kind: "ask.reply", status: "failed", note: "Sent late, and the laptop could not do it: no such issue" },
    { id: c.id, kind: "issue.rank", status: "running", note: "Held" },
    // Its name is on the record, and it was never taken: not this function's to touch.
    { id: d.id, kind: "ask.send", status: "done" },
    { id: "someone-else", kind: "pr.merge", status: "done" },
  ];
  const changed = await O.settle(store, acts, () => T0 + 99);
  expect(changed.map((e) => [e.kind, e.state, e.note, e.endedAt])).toEqual([
    ["ask.send", "done", "", T0 + 99],
    ["ask.reply", "failed", "Sent late, and the laptop could not do it: no such issue", T0 + 99],
  ]);
  expect((await store.all()).map((e) => e.state).sort()).toEqual(["done", "failed", "held", "waiting"]);
  // A record that cannot be read changes nothing.
  expect(await O.settle(store, undefined)).toEqual([]);
  expect(await O.settle(store, "nonsense")).toEqual([]);
});

test("one try at sending: what goes out, and how each kind of answer is read", async () => {
  const entry = O.make("ask.send", { text: "hello" }, T0, undefined, "0a000000-0000-4000-8000-000000000009");
  const calls: { url: string; init: RequestInit }[] = [];
  const answering =
    (res: unknown) =>
    async (url: string, init: RequestInit): Promise<unknown> => {
      calls.push({ url, init });
      return res;
    };
  const res = (status: number, body: unknown, type = "basic") => ({
    status,
    type,
    json: async () => {
      if (body === undefined) throw new Error("not json");
      return body;
    },
  });
  expect(await O.httpSend(entry, answering(res(200, { ok: true, result: { issue: 5 } })))).toEqual({
    status: 200,
    body: { ok: true, result: { issue: 5 } },
  });
  // It goes as the app's own writes do, under its name, and the way to
  // a sign-in page is not followed.
  expect(calls[0].url).toBe("/console/api/act");
  expect(calls[0].init).toEqual({
    method: "POST",
    headers: { "Content-Type": "application/json", "X-PM-Console": "1" },
    body: JSON.stringify({ kind: "ask.send", args: { text: "hello" }, id: entry.id }),
    redirect: "manual",
  });
  // No answer.
  expect(
    await O.httpSend(entry, async () => {
      throw new TypeError("Failed to fetch");
    }),
  ).toEqual({ network: true });
  // Sent towards a sign-in page, in either of the ways a browser shows it.
  expect(await O.httpSend(entry, answering(res(0, undefined, "opaqueredirect")))).toEqual({
    signin: true,
    status: 0,
    body: null,
  });
  expect(await O.httpSend(entry, answering(res(302, undefined)))).toEqual({ signin: true, status: 0, body: null });
  // No sign-in, whoever says so.
  expect(await O.httpSend(entry, answering(res(401, { error: "sign in" })))).toEqual({
    signin: true,
    status: 401,
    body: null,
  });
  expect(await O.httpSend(entry, answering(res(403, undefined)))).toEqual({ signin: true, status: 403, body: null });
  // Refused by the platform in its own words: that is an answer.
  expect(await O.httpSend(entry, answering(res(403, { error: "this request did not come from the console" })))).toEqual(
    {
      status: 403,
      body: { error: "this request did not come from the console" },
    },
  );
  // An answer that is not JSON is passed on as none, for classify to read.
  expect(await O.httpSend(entry, answering(res(200, undefined)))).toEqual({ status: 200, body: null });
});

test("the store that lives with the page keeps copies, not the caller's own objects", async () => {
  const store = O.memoryStore();
  const entry = O.make("ask.send", { text: "x" }, T0);
  await store.put(entry);
  entry.state = "changed by the caller";
  expect((await store.all())[0].state).toBe("waiting");
  (await store.all())[0].state = "changed by a reader";
  expect((await store.all())[0].state).toBe("waiting");
  await store.remove(entry.id);
  await store.remove("not there");
  expect(await store.all()).toEqual([]);
});
