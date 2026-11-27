// Oracle: an unaccepted answer retains the exact submitted words for its
// question identity; only an accepted/durably kept handoff clears it.
// Runs the real card click, dialog close, submit and sendAnswer functions.
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { El } from "./dom.ts";

class Input extends El {
  value = "";
  returnValue = "";
  get textContent() { return super.textContent; }
  set textContent(text: string) { this.replaceChildren(text); }
  showModal() {}
  focus() {}
  addEventListener(name: string, fn: any, options?: { once?: boolean }) {
    const wrapped = options?.once ? (event: any) => {
      this.listeners[name] = this.listeners[name].filter((f) => f !== wrapped);
      fn(event);
    } : fn;
    super.addEventListener(name, wrapped);
  }
}

function browser() {
  const frame = new Map(["ask", "ask-input", "ask-ok", "ask-title", "ask-text", "ask-label", "toast"].map((id) => [id, new Input(id)]));
  let result: any = { state: "refused", note: "No connection" };
  const sent: any[] = [];
  const completions: (() => void)[] = [];
  const document = {
    getElementById: (id: string) => frame.get(id) ?? null,
    createElement: (tag: string) => new Input(tag),
    createTextNode: (text: string) => text,
  };
  const ui = readFileSync(new URL("../public/ui.js", import.meta.url), "utf8").replaceAll("export ", "");
  const app = readFileSync(new URL("../public/app.js", import.meta.url), "utf8")
    .replace(/^import .*;$/gm, "").replaceAll("export ", "");
  const hooks = runInNewContext(`${ui}\n${app}\nloadDoc = async () => {}; draw = () => completeSend();\n({ questionCard, setStore: (store) => { outbox = store; } });`, {
    document, PAGES: [], navigator: {}, setTimeout: () => 0, clearTimeout: () => {},
    completeSend: () => completions.shift()?.(),
    self: { PMOutbox: {
      make: (kind: string, args: unknown) => ({ id: "answer-id", kind, args }),
      rule: () => "keep",
      httpSend: async (entry: any) => { sent.push(entry); return result; },
      classify: (answer: any) => answer,
      sendInOrder: async (_store: any, _id: any, send: any) => {
        const out = await send({ kind: "question.answer" });
        return { entry: { state: "waiting" }, others: [], signin: false, offline: false, ...out };
      },
    } },
  });
  const input = frame.get("ask-input")!;
  const dialog = frame.get("ask")!;
  function open(q: any) {
    const card = hooks.questionCard({ options: [], recommended: [], links: [], q: "Which?", ...q });
    card.find("button").find((b: El) => b.textContent === "Write an answer").fire("click");
  }
  async function settle() { await Promise.resolve(); }
  async function answer(q: any, words: string) {
    const done = new Promise<void>((resolve) => { completions.push(resolve); });
    open(q);
    input.value = words;
    dialog.returnValue = "ok";
    dialog.fire("close");
    // The real sendAnswer's final draw, not a sleep or a fixed number
    // of microtasks, determines when its handoff has finished.
    await done;
  }
  function cancel() { dialog.returnValue = ""; dialog.fire("close"); }
  return { open, answer, cancel, input, sent, hooks, settle, outcome: (out: any) => { result = out; } };
}

const first = { set: "board", ledger: "decisions", qid: "q1" };

test("answer draft: an older acceptance cannot erase a newer failed submission, even with identical words", async () => {
  const b = browser();
  let accept!: (out: any) => void;
  let refuse!: (out: any) => void;
  b.outcome(new Promise((resolve) => { accept = resolve; }));
  const older = b.answer(first, "same exact words");
  await b.settle();
  b.outcome(new Promise((resolve) => { refuse = resolve; }));
  const newer = b.answer(first, "same exact words");
  await b.settle();
  accept({ state: "done" });
  await older;
  refuse({ state: "failed", note: "Not accepted" });
  await newer;
  b.open(first);
  expect(b.input.value).toBe("same exact words");
  b.cancel();
});

test("answer draft: refused and failed sends reopen the exact words without crossing question keys", async () => {
  const b = browser();
  // Model invariant exercised over each identity dimension, Unicode and whitespace.
  const questions = [first, { ...first, set: "other" }, { ...first, ledger: "other" }, { ...first, qid: "other" }];
  const words = ["  Keep my exact words\nplease  ", "✓ 第二案", "different ledger", "last question"];
  for (let i = 0; i < questions.length; i++) {
    b.outcome({ state: i % 2 ? "failed" : "refused", note: "Not accepted" });
    await b.answer(questions[i], words[i]);
  }
  for (let i = 0; i < questions.length; i++) {
    b.open(questions[i]);
    expect(b.input.value).toBe(words[i]);
    b.cancel();
    await b.settle();
  }
  expect(b.sent.map((entry) => entry.args.text)).toEqual(words);
});

for (const state of ["done", "held", "kept"]) {
  test(`answer draft: ${state} handoff clears only that question`, async () => {
    const b = browser();
    const other = { ...first, qid: "q2" };
    await b.answer(first, "first draft");
    await b.answer(other, "other draft");
    const stored: any[] = [];
    if (state === "kept") b.hooks.setStore({ put: async (entry: any) => { stored.push(entry); }, all: async () => stored });
    b.outcome({ state: state === "kept" ? "waiting" : state, note: "Accepted" });
    await b.answer(first, "accepted draft");
    if (state === "kept") expect(stored[0].args).toEqual({ ...first, text: "accepted draft" });
    b.open(first);
    expect(b.input.value).toBe("");
    b.cancel();
    await b.settle();
    b.open(other);
    expect(b.input.value).toBe("other draft");
    b.cancel();
  });
}
