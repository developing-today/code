// What the owner did, kept on this device until the platform has it.
//
// Every action the app takes is given a name here (a UUID) and written
// down before it is sent. With a connection it is sent at once, as
// before. Without one it waits, and is sent when the connection is
// back: by the page if it is open, by the service worker if it is not.
// The platform does an action once under its name however often it
// arrives, so sending again after an answer that never came is safe.
//
// One file for the page and the service worker alike, so there is one
// rule for what is sent when. It is a plain script, not a module: the
// worker takes it with importScripts, the page with a script tag, and
// both find it as PMOutbox.
((root) => {
  /** Actions that may be done late: what the owner says or decides does
   * not go stale. (The same list the platform holds for a machine.) */
  const LATE = new Set([
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
    // The composer's broaden and offer are the owner's words too
    // (#14375): done late they still do what was meant.
    "message.broaden",
    "message.offer",
    // A revocation done late is still the right act (#14374).
    "agent.revoke",
    // Taking back unused enroll tokens, the same (#14376). The
    // provisioning mints are NOT here: their answer is a secret shown
    // once, so they are "now" — done with a connection or not at all,
    // never kept on this device to be done late.
    "agent.revoke-enroll",
    // Removing a wake hook, the same (#14544). Setting one is NOT
    // here: its arguments carry the hook's token, so it is "now",
    // never kept on this device to be done late.
    "agent.wake-hook-remove",
    "intake.send",
  ]);
  /** Actions on something that may have changed by the time they are
   * sent: a merge, a deploy, a task's or a unit's state. They are kept
   * too, and sent by themselves only while they are fresh; after that
   * they wait for the owner to look again and say so. */
  const TIMELY = new Set(["pr.merge", "deploy.start", "task.cancel", "task.unblock", "unit.act"]);
  /** How long a timely action stays fresh: a tunnel, a lift, a bad patch. */
  const TIMELY_MS = 10 * 60_000;
  /** Said of a timely action kept back for being too old, by this device
   * or by the platform. */
  const STALE_NOTE = "Not sent: it was a while ago, and this may have changed.";
  /** Finished entries are shown for a day, and no more than this many. */
  const DONE_FOR_MS = 24 * 60 * 60_000;
  const DONE_KEPT = 30;
  /** The platform keeps what it holds for a machine for a day, and its
   * record of what was done is short. An entry it took that long ago
   * has its outcome there, not here. */
  const HELD_FOR_MS = 26 * 60 * 60_000;

  /** How an action is treated when it cannot be sent: "late" (kept and
   * sent whenever), "timely" (kept, sent by itself only while fresh),
   * or "now" (needs a connection: it sets the connection up, or carries
   * a secret that is not written down). What is not known is "now". */
  const rule = (kind) => (LATE.has(kind) ? "late" : TIMELY.has(kind) ? "timely" : "now");

  /** What an action is about, where the order of two matters: a rank
   * set twice, a task moved twice. Two about one thing are sent in the
   * order they were done, and a later one is not sent while an earlier
   * one still waits. Null: it stands alone (a new prompt, a message).
   * (The platform keeps the same order for what it holds for a
   * machine: subjectOf in src/edge-proto.ts says the same.) */
  function subject(kind, args = {}) {
    const a = args ?? {};
    const of = (what, value) => {
      const said = typeof value === "string" || typeof value === "number" ? String(value) : "";
      return said === "" ? null : `${what}:${said}`;
    };
    switch (kind) {
      case "ask.reply":
      case "ask.close":
      case "issue.comment":
      case "issue.rank":
        return of("issue", a.issue);
      case "task.move":
      case "task.cancel":
      case "task.unblock":
        return of("task", a.task);
      case "hands.done":
      case "need.comment":
        return of("ref", a.ref);
      case "question.answer":
      case "question.defer":
      case "questions.accept":
        return of("questions", a.set);
      case "pr.merge":
        return of("pr", a.pr);
      case "unit.act":
        return of("unit", a.unit);
      case "agent.revoke": {
        const surface = typeof a.surface === "string" ? a.surface : "";
        const name = typeof a.name === "string" ? a.name : "";
        return surface === "" || name === "" ? null : `agent:${surface}:${name}`;
      }
      case "message.broaden":
        return of("batch", a.batch);
      case "agent.revoke-enroll":
        return of("enroll", a.agent);
      case "agent.wake-hook-remove":
        return of("wake-hook", a.address);
      default:
        return null;
    }
  }
  /** Said of an entry that was not sent because an earlier one about
   * the same thing has not been taken yet. */
  const BEHIND_NOTE = "It goes after an earlier one about the same thing, which has not been taken yet.";

  const short = (text, n = 60) => {
    const t = String(text ?? "")
      .replace(/\s+/g, " ")
      .trim();
    return t.length > n ? `${t.slice(0, n - 1)}…` : t;
  };

  /** One line saying what an action is, for the list of what is kept. */
  /** A setting the owner may change, as it is called on the Machines screen. */
  function settingName(name) {
    return name === "promptLabel" ? "the prompt label" : name === "needsRepos" ? "the repositories looked in" : "a setting";
  }

  function sayOf(kind, args = {}) {
    const a = args ?? {};
    const quoted = (text) => (short(text) ? `: “${short(text)}”` : "");
    switch (kind) {
      case "ask.send":
        // A question and a standing instruction are said as what they are.
        return `${a.intent === "question" ? "Question" : a.intent === "directive" ? "Standing instruction" : "Prompt"}${quoted(a.text)}`;
      case "ask.reply":
        return `Reply on #${a.issue}${quoted(a.text)}`;
      case "ask.close":
        return `Mark prompt #${a.issue} done`;
      case "issue.comment":
        return `Comment on #${a.issue}${quoted(a.text)}`;
      case "issue.rank":
        return `Rank #${a.issue} ${a.rank}`;
      case "need.comment":
        return `Reply on ${a.ref}${quoted(a.text)}`;
      case "hands.done":
        return `Mark ${a.ref} done`;
      case "task.move":
        return `Move task ${a.task} ${a.to === "first" ? "to the top" : a.to}`;
      case "task.cancel":
        return `Cancel task ${a.task}`;
      case "task.unblock":
        return `Unblock task ${a.task}`;
      case "pr.merge":
        return `Merge #${a.pr}`;
      case "deploy.start":
        return a.what === "app" ? "Start the app's build" : "Start the site's deploy";
      case "intake.send": {
        const what = a.session ? "Session" : a.kind === "bug" ? "Bug" : a.kind === "change" ? "Change" : "Note";
        return `${what}${quoted(a.text)}`;
      }
      case "question.answer":
        return `Answer${quoted(Array.isArray(a.answer) ? a.answer.join(", ") : a.answer)}`;
      case "question.defer":
        return "Put a question off";
      case "questions.accept":
        return "Accept the recommended answers";
      case "message.send":
        return `Message to ${Array.isArray(a.to) ? a.to.join(", ") : a.to}${quoted(a.text)}`;
      case "message.broaden":
        return `Broaden the send on batch ${a.batch ?? "?"}`;
      case "message.offer":
        return `Offer to ${Array.isArray(a.to) ? a.to.join(", ") : a.to} (${a.slots ?? "?"} slots)${quoted(a.text)}`;
      case "agent.revoke":
        return `Revoke ${a.name ?? "an agent"}'s ${a.surface === "notes" ? "notes tokens" : "bridge credentials"}`;
      case "agent.revoke-enroll":
        return `Take back ${a.agent ?? "an agent"}'s unused enroll tokens`;
      case "agent.wake-hook-set":
        return `Set the wake hook for ${a.address ?? "an address"}`;
      case "agent.wake-hook-remove":
        return `Remove the wake hook for ${a.address ?? "an address"}`;
      case "agent.mint-enroll":
        return `Mint an enroll token for ${a.agent ?? "a new agent"}`;
      case "agent.mint-credential":
        return `Mint a credential for ${a.agent ?? "an agent"}`;
      case "unit.act":
        return `${a.action} ${a.unit}`;
      // The platform's own second half of something it filed itself:
      // never kept here, and shown where what waits for a machine is.
      case "settings.set":
        return `Change ${settingName(a.name)}`;
      case "settings.clear":
        return a.name ? `Put ${settingName(a.name)} back to main's` : "Put the changed settings back to main's";
      case "prompt.enqueue":
        return `File the queue task for prompt #${a.issue}`;
      case "triage.enqueue":
        return `File the triage task for #${a.issue}`;
      default:
        return String(kind);
    }
  }

  const uuid = () => root.crypto.randomUUID();

  /** A new entry: an action, named, as it stood when the owner did it. */
  function make(kind, args, now, said, id = uuid()) {
    return { id, kind, args: args ?? {}, said: said || sayOf(kind, args), at: now, state: "waiting", note: "", tries: 0 };
  }

  /** What is said of something the platform is holding, by who it
   * waits for: either side, whichever can first; a machine only; or a
   * machine that may already have it. */
  const HELD_NOTES = {
    either: "Kept: Cloudflare does it as soon as it can again, or the laptop if it is back first.",
    machine: "Kept for the laptop, which will do it when it is back.",
    asked: "Kept: the laptop did not answer. It may have got this, and says what became of it when it is back.",
  };

  /** What became of one try at sending. `answer` is what came back:
   * `{ network: true }` when nothing did, else the status, whether the
   * body was JSON, and the body. Returns the entry's new state and what
   * to say of it; `stop` means the rest will fare no better just now. */
  function classify(answer) {
    if (answer.network) return { state: "waiting", note: "", stop: true };
    const { status, body } = answer;
    // Not the platform's own answer: a sign-in page, or something in
    // between. The action was not taken; it waits for a sign-in.
    if (answer.signin || body === null || typeof body !== "object") {
      return { state: "waiting", note: "Sign in again to send this.", stop: true, signin: true };
    }
    if (status === 200 && body.ok === true) {
      if (body.held === true) {
        // The platform says who it waits for; one that does not say is
        // older than the saying, and holds things for the laptop only.
        const waits = typeof body.waits === "string" && Object.hasOwn(HELD_NOTES, body.waits) ? body.waits : "machine";
        return { state: "held", note: HELD_NOTES[waits], result: null };
      }
      // Taken, and answered "not recorded" without an error: an answer
      // to a question that was settled in the meantime. That is not
      // "sent", and it is said as what it is (the laptop says the same
      // of one it does late).
      const refused = Array.isArray(body.result?.refused) ? body.result.refused : [];
      if (refused.length > 0) {
        const why = String(refused[0]?.why ?? "no reason given");
        const applied = Number(body.result?.applied) || 0;
        return {
          state: "failed",
          note: applied > 0 ? `${applied} recorded, ${refused.length} not recorded: ${why}` : `Not recorded: ${why}`,
          result: body.result,
        };
      }
      return { state: "done", note: "", result: body.result ?? null };
    }
    const why = typeof body.error === "string" && body.error !== "" ? body.error : `http ${status}`;
    // Being done right now under this name: ask again shortly.
    if (status === 409 && body.running === true) return { state: "waiting", note: "Being done now." };
    // Too old for the platform to do unasked (#13501): the platform's
    // own clock, or this device's, says more than ten minutes have
    // passed. It waits for the owner to look again, as one this device
    // kept back itself does.
    if (status === 409 && body.stale === true) return { state: "stale", note: STALE_NOTE };
    // Nothing could take it just now (no machine in reach, too much
    // waiting). Under its name it is safe to offer again, and it is:
    // said in this device's own words, because the platform's are
    // written for a caller that cannot ("check before trying again").
    if (status === 503) return { state: "waiting", note: "Nothing could take it just now. It is offered again." };
    return { state: "failed", note: why };
  }

  /** Whether an entry is sent by itself now. A timely one that has gone
   * stale is not: it is marked, and waits for the owner. */
  function due(entry, now) {
    if (entry.state !== "waiting") return false;
    if (rule(entry.kind) !== "timely" || entry.sure === true) return true;
    return now - entry.at <= TIMELY_MS;
  }

  /** Counts for the line that says what is kept. */
  function summary(entries) {
    const count = (state) => entries.filter((e) => e.state === state).length;
    return {
      waiting: count("waiting"),
      stale: count("stale"),
      held: count("held"),
      failed: count("failed"),
      done: count("done"),
      // What still needs something: a connection, a look, or a decision.
      open: count("waiting") + count("stale") + count("failed"),
    };
  }

  /** The entries no longer worth keeping: done ones older than a day,
   * the oldest done ones past a count, and ones the platform took for a
   * machine so long ago that their outcome is only on its own record.
   * Nothing the platform has not taken is ever dropped by this. */
  function expired(entries, now) {
    const done = entries.filter((e) => e.state === "done").sort((a, b) => (b.endedAt ?? b.at) - (a.endedAt ?? a.at));
    return [
      ...done.filter((e, i) => i >= DONE_KEPT || now - (e.endedAt ?? e.at) > DONE_FOR_MS),
      ...entries.filter((e) => e.state === "held" && now - (e.endedAt ?? e.at) > HELD_FOR_MS),
    ].map((e) => e.id);
  }

  /** Send what is due, oldest first. `store` keeps the entries (all,
   * put, remove); `send(entry)` makes one try and says what came back.
   * Returns what changed, and which entries this pass looked at. One
   * flush at a time per store: a second call while one runs joins it. */
  const running = new WeakMap();
  function flush(store, send, now = () => Date.now()) {
    const already = running.get(store);
    if (already) return already;
    const work = (async () => {
      const changed = [];
      let signin = false;
      let offline = false;
      const entries = (await store.all()).sort((a, b) => a.at - b.at);
      // What an earlier entry in this pass is about, when it still
      // waits after its try: a later one about the same thing is not
      // sent ahead of it.
      const stillWaits = new Set();
      for (const listed of entries) {
        if (listed.state !== "waiting") continue;
        // As it stands now, not as it stood when this began: one taken
        // back meanwhile ("do not send") is not sent, and one already
        // dealt with is not dealt with again.
        const entry = (await store.all()).find((e) => e.id === listed.id);
        if (!entry || entry.state !== "waiting") continue;
        if (!due(entry, now())) {
          // Too old to send unasked: kept, and said to be.
          const stale = { ...entry, state: "stale", note: STALE_NOTE };
          await store.put(stale);
          changed.push(stale);
          continue;
        }
        const about = subject(entry.kind, entry.args);
        if (about !== null && stillWaits.has(about)) {
          if (entry.note !== BEHIND_NOTE) {
            const behind = { ...entry, note: BEHIND_NOTE };
            await store.put(behind);
            changed.push(behind);
          }
          continue;
        }
        let answer;
        try {
          answer = await send(entry);
        } catch {
          answer = { network: true };
        }
        const { state, note, result, stop, signin: needs } = classify(answer);
        if (state === "waiting" && about !== null) stillWaits.add(about);
        const next = {
          ...entry,
          state,
          note,
          tries: entry.tries + 1,
          triedAt: now(),
          ...(state === "done" || state === "failed" || state === "held" ? { endedAt: now() } : {}),
          ...(result !== undefined ? { result } : {}),
        };
        // A try that changed nothing worth writing is not written.
        if (next.state !== entry.state || next.note !== entry.note) {
          await store.put(next);
          changed.push(next);
        }
        if (needs) signin = true;
        if (answer.network) offline = true;
        if (stop) break;
      }
      const left = await store.all();
      for (const id of expired(left, now())) await store.remove(id);
      // How many still wait to be sent: by their own doing (no answer,
      // nothing to take them just now), not for the owner to look at.
      return {
        changed,
        signin,
        offline,
        waiting: left.filter((e) => e.state === "waiting").length,
        looked: entries.map((e) => e.id),
      };
    })();
    running.set(store, work);
    const clear = () => running.delete(store);
    work.then(clear, clear);
    return work;
  }

  /** Send an entry that has just been written down, behind whatever is
   * already waiting, in the order things were done. Sent ahead of an
   * older one, the older would undo it when its turn came: a rank
   * changed twice would end as the first. Says what became of it (the
   * entry as it now stands, or null if it was taken back meanwhile),
   * and what else went in the same pass. One that failed is not kept:
   * the caller says so to the owner's face. */
  async function sendInOrder(store, id, send, now = () => Date.now()) {
    const mine = async () => (await store.all()).find((e) => e.id === id) ?? null;
    let out = await flush(store, send, now);
    let entry = await mine();
    // A send already under way was joined, and it began before this
    // was written down: once more, for this one.
    if (entry?.state === "waiting" && !out.looked.includes(id) && !out.offline && !out.signin) {
      out = await flush(store, send, now);
      entry = await mine();
    }
    if (entry?.state === "failed") await store.remove(id);
    return { entry, others: out.changed.filter((e) => e.id !== id), signin: out.signin, offline: out.offline };
  }

  /** Entries held for a machine learn how they came out from the
   * platform's own record of what was done, which carries their names. */
  async function settle(store, acts, now = () => Date.now()) {
    const byId = new Map((Array.isArray(acts) ? acts : []).map((a) => [a.id, a]));
    const changed = [];
    for (const entry of await store.all()) {
      if (entry.state !== "held") continue;
      const act = byId.get(entry.id);
      if (!act || act.status === "running") continue;
      const next =
        act.status === "done"
          ? { ...entry, state: "done", note: "", endedAt: now() }
          : { ...entry, state: "failed", note: act.note || "It could not be done.", endedAt: now() };
      await store.put(next);
      changed.push(next);
    }
    return changed;
  }

  /** How long one try may take before it is given up on. A request
   * that hung would otherwise hold up everything behind it, in every
   * tab and in the worker, for as long as it hung. Given up on is not
   * "not done": it is offered again under its name, and the platform
   * does it once. */
  const SEND_WAIT_MS = 60_000;

  /** What is sent of an entry. The platform is told when the owner did
   * it and what this device's clock reads as it is sent, so it can hold
   * a timely one to the same ten minutes this device does (#13501),
   * and whether the owner looked again and said to send it anyway. */
  function sendBody(entry, now) {
    return {
      kind: entry.kind,
      args: entry.args,
      id: entry.id,
      ...(typeof entry.at === "number" && Number.isFinite(entry.at) ? { at: entry.at, sent: now } : {}),
      ...(entry.sure === true ? { sure: true } : {}),
    };
  }

  /** One try at sending an entry to the platform. */
  async function httpSend(entry, fetchImpl = root.fetch.bind(root), now = () => Date.now()) {
    let res;
    try {
      res = await fetchImpl("/console/api/act", {
        method: "POST",
        headers: { "Content-Type": "application/json", "X-PM-Console": "1" },
        body: JSON.stringify(sendBody(entry, now())),
        // A sign-in that has run out answers with a way to another
        // site. It is not followed: it is how this is told from no
        // connection at all.
        redirect: "manual",
        ...(root.AbortSignal?.timeout ? { signal: root.AbortSignal.timeout(SEND_WAIT_MS) } : {}),
      });
    } catch {
      return { network: true };
    }
    if (res.type === "opaqueredirect" || (res.status >= 300 && res.status < 400)) return { signin: true, status: 0, body: null };
    let body = null;
    try {
      body = await res.json();
    } catch (error) {
      // An answer that is not JSON is not the platform's own (a sign-in
      // page, something in between), and is read below as that. Any
      // other failure is the answer not arriving whole: the connection
      // went part-way through it, or the try was given up on. That is
      // no answer at all, and it is offered again.
      if (error?.name !== "SyntaxError") return { network: true };
      body = null;
    }
    // No sign-in, whoever says so; or refused by something in between
    // that is not the platform (it answers in its own words).
    if (res.status === 401 || (res.status === 403 && !(body && typeof body.error === "string"))) {
      return { signin: true, status: res.status, body: null };
    }
    return { status: res.status, body };
  }

  /** The entries, kept in the browser's own database so that the page
   * and the service worker see the same ones. */
  function idbStore(name = "pm-console") {
    const open = () =>
      new Promise((resolve, reject) => {
        const req = root.indexedDB.open(name, 1);
        req.onupgradeneeded = () => req.result.createObjectStore("outbox", { keyPath: "id" });
        req.onsuccess = () => resolve(req.result);
        req.onerror = () => reject(req.error);
      });
    const run = async (mode, work) => {
      const db = await open();
      try {
        return await new Promise((resolve, reject) => {
          const tx = db.transaction("outbox", mode);
          const req = work(tx.objectStore("outbox"));
          tx.oncomplete = () => resolve(req?.result);
          tx.onerror = () => reject(tx.error);
          tx.onabort = () => reject(tx.error);
        });
      } finally {
        db.close();
      }
    };
    return {
      all: async () => (await run("readonly", (s) => s.getAll())) ?? [],
      put: (entry) => run("readwrite", (s) => s.put(entry)),
      remove: (id) => run("readwrite", (s) => s.delete(id)),
    };
  }

  /** A store that lives only as long as the page: for a browser that
   * will not give this page a database. Nothing is kept across a
   * reload, and the app says so. */
  function memoryStore() {
    const entries = new Map();
    return {
      all: async () => [...entries.values()].map((e) => ({ ...e })),
      put: async (entry) => void entries.set(entry.id, { ...entry }),
      remove: async (id) => void entries.delete(id),
    };
  }

  root.PMOutbox = {
    TIMELY: [...TIMELY],
    TIMELY_MS,
    STALE_NOTE,
    SEND_WAIT_MS,
    rule,
    subject,
    sayOf,
    make,
    classify,
    due,
    summary,
    expired,
    flush,
    sendInOrder,
    settle,
    sendBody,
    httpSend,
    idbStore,
    memoryStore,
  };
})(typeof self !== "undefined" ? self : globalThis);
