// Everything the owner can do, as one list of named actions.
//
// One endpoint takes `{ kind, args }`. Each action names the sections it
// changes, so the documents are read again before the answer goes back,
// and what it leaves in the audit log, which never includes the text the
// owner typed.
import type { Devices } from "./devices.ts";
import type { Docs } from "./docs.ts";
import { OpError, type Ops } from "./ops.ts";
import type { Push } from "./push.ts";
import type { Work } from "./work.ts";

type Args = Record<string, unknown>;
interface Action {
  run: (args: Args) => Promise<unknown>;
  /** Sections to read again afterwards. */
  touch: string[];
  /** What the audit log keeps of the arguments. */
  audit: (args: Args) => Args;
}

const ids =
  (...keys: string[]) =>
  (args: Args) =>
    Object.fromEntries(keys.map((k) => [k, args[k]]));

/** A refusal from the push module is the caller's mistake, not a failure of this service. */
const asked = async <T>(p: Promise<T>): Promise<T> =>
  p.catch((error: Error) => {
    throw error instanceof OpError ? error : new OpError(400, error.message);
  });

export function actions(ops: Ops, work: Work, push?: Push, devices?: Devices): Record<string, Action> {
  // A phone linked to read the summary its home-screen widget shows.
  const phones: Record<string, Action> = devices
    ? {
        "device.approve": { run: (a) => devices.approve(a), touch: [], audit: ids("name") },
        "device.remove": { run: (a) => devices.remove(a), touch: [], audit: ids("id") },
      }
    : {};
  const notify: Record<string, Action> = push
    ? {
        "push.on": { run: (a) => asked(push.subscribe(a.subscription, a.label)), touch: [], audit: () => ({}) },
        "push.off": { run: (a) => asked(push.unsubscribe(a.endpoint)), touch: [], audit: () => ({}) },
        "push.class": { run: (a) => asked(push.setClass(a.class, a.on)), touch: [], audit: ids("class", "on") },
        "push.test": {
          run: () =>
            push.send("test", {
              title: "PM",
              body: "Notifications are on for this device.",
              url: "/console/",
              tag: "test",
            }),
          touch: [],
          audit: () => ({}),
        },
      }
    : {};
  return {
    ...notify,
    ...phones,
    "ask.send": { run: (a) => work.ask(a), touch: ["asks", "board"], audit: () => ({}) },
    "ask.reply": { run: (a) => work.askReply(a), touch: ["asks", "board"], audit: ids("issue") },
    "ask.close": { run: (a) => work.askClose(a), touch: ["asks"], audit: ids("issue") },
    "question.answer": {
      run: (a) => work.answer(a),
      touch: ["questions", "questionCount"],
      audit: ids("set", "ledger", "qid", "option", "options"),
    },
    "question.defer": {
      run: (a) => work.defer(a),
      touch: ["questions", "questionCount"],
      audit: ids("set", "ledger", "qid", "lean"),
    },
    "questions.accept": {
      run: (a) => work.acceptRecommended(a.set),
      touch: ["questions", "questionCount"],
      audit: ids("set"),
    },
    "hands.done": { run: (a) => work.handsDone(a), touch: [], audit: ids("ref") },
    "need.comment": { run: (a) => work.needComment(a), touch: [], audit: ids("ref") },
    "task.move": { run: (a) => work.moveTask(a), touch: ["board"], audit: ids("task", "to") },
    "task.unblock": {
      run: (a) => ops.queueAct("unblock", a.task, a.reason),
      touch: ["board"],
      audit: ids("task", "reason"),
    },
    "task.cancel": {
      run: (a) => ops.queueAct("cancel", a.task, a.reason),
      touch: ["board"],
      audit: ids("task", "reason"),
    },
    "issue.rank": { run: (a) => ops.setRank(a.issue, a.rank), touch: ["priorities"], audit: ids("issue", "rank") },
    "issue.comment": { run: (a) => ops.comment(a.issue, a.text), touch: [], audit: ids("issue") },
    "pr.merge": { run: (a) => ops.mergePr(a.pr), touch: ["prs"], audit: ids("pr") },
    "unit.act": {
      run: (a) => ops.serviceAct(a.unit, a.action),
      touch: ["services"],
      audit: ids("unit", "action"),
    },
    "message.send": {
      run: (a) => ops.send(a.to, a.text, a.command),
      touch: [],
      audit: (a) => ({ to: a.to, command: a.command === true }),
    },
  };
}

/** Run one named action and read again what it changed. */
export async function perform(
  table: Record<string, Action>,
  docs: Docs,
  kind: unknown,
  args: unknown,
): Promise<unknown> {
  const action = typeof kind === "string" && Object.hasOwn(table, kind) ? table[kind] : undefined;
  if (!action) throw new OpError(400, "kind: not an action this console has");
  if (args === null || typeof args !== "object" || Array.isArray(args))
    throw new OpError(400, "args: expected an object");
  const result = await action.run(args as Args);
  // The census is the dashboard's and refreshes on its own clock; for the
  // rest, the caller's next read should already show the change.
  await docs.touch(action.touch);
  return result;
}
