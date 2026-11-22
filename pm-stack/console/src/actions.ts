// Everything the owner can do, as one list of named actions.
//
// One endpoint takes `{ kind, args }`. Each action names the sections it
// changes, so the documents are read again before the answer goes back,
// and what it leaves in the audit log, which never includes the text the
// owner typed.
import type { Devices } from "./devices.ts";
import type { Docs } from "./docs.ts";
import { OpError } from "./errors.ts";
import type { Ops } from "./ops.ts";
import type { Push } from "./push.ts";
import { promptPacket, type Work } from "./work.ts";

type Args = Record<string, unknown>;
export interface Action {
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

/** The actions that need nothing from the host: this device's
 * notifications, and phones linked to read the summary. They are the
 * ones the edge runs itself. */
export function ownActions(push?: Push, devices?: Devices): Record<string, Action> {
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
  return { ...notify, ...phones };
}

export function actions(ops: Ops, work: Work, push?: Push, devices?: Devices): Record<string, Action> {
  return {
    ...ownActions(push, devices),
    "ask.send": { run: (a) => work.ask(a), touch: ["asks", "board"], audit: () => ({}) },
    "ask.reply": { run: (a) => work.askReply(a), touch: ["asks", "board"], audit: ids("issue") },
    "ask.close": { run: (a) => work.askClose(a), touch: ["asks"], audit: ids("issue") },
    // The second half of a prompt the edge filed while the laptop was
    // away: the issue already exists, so only its queue task is left.
    // The edge holds this, never the whole ask, so the issue is never
    // filed twice. The arguments are checked again here: the edge is
    // trusted for the held actions it sends, and a bad one is refused
    // rather than enqueued.
    "prompt.enqueue": {
      run: async (a) => {
        const issue = Number(a.issue);
        if (!Number.isSafeInteger(issue) || issue <= 0) throw new OpError(400, "issue: a positive whole number");
        if (typeof a.text !== "string" || a.text.trim() === "" || a.text.length > 8000)
          throw new OpError(400, "text: required, at most 8000 characters");
        if (typeof a.at !== "string" || a.at === "") throw new OpError(400, "at: required");
        const queued = await ops.enqueue(
          promptPacket(ops.cfg.repo, ops.cfg.queue, issue, a.text, a.at),
        );
        return { issue, queued: queued.code === 0 };
      },
      touch: ["board"],
      audit: ids("issue"),
    },
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
    // The client's two values are in the arguments: nothing of them is
    // written to the record of what was done, and the action is never
    // held anywhere to be done late.
    "sso.google": { run: (a) => ops.googleSignin(a.clientId, a.clientSecret), touch: [], audit: () => ({}) },
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
