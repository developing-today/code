// The edge's held actions: what the owner asked to be done later,
// taken by this side or handed to a machine, and the acts it did itself.
//
// Split out of `edge.ts` for the production-line guard (#14547):
// moves only, no behaviour change. `Edge` (in `edge.ts`) extends this
// class; the state both sides touch is declared here and owned there.
import { actions, type Action } from "../../src/actions.ts";
import { DIRECT_ACTIONS, DIRECT_READ_SECTIONS, type DirectClasses, type GitHubMemory, QUIET_MOST_MS } from "./direct.ts";
import { json } from "../../src/http.ts";
import { type Act, type Docs } from "../../src/docs.ts";
import { type EdgeConfig, HELD_FOR_MS, type Held, type HeldFor, machineReport, type MachineReport, type MachinesAnswer, subjectOf } from "../../src/edge-proto.ts";
import { settingsDifferences } from "../../src/settings.ts";
import { unread } from "../../src/timed.ts";
import type { EdgeOptions, KV } from "./edge.ts";

/** A rank set twice ends at the last one: an older held rank never
 * runs when a newer rank for the same issue is done. Anything else
 * held runs first, in the order it was done. */
export const LAST_WINS = new Set(["issue.rank"]);

/** With no report for this long, the laptop is not there to ask. */
export const HOST_QUIET_MS = 60_000;

export const HELD_MAX = 100;
/** After a pass over what is held that could do none of it, how long
 * before another is tried. */
export const DRAIN_AGAIN_MS = 60_000;
/** Said of something begun by an object that was put away before it
 * ended. */
export const RESTARTED = "Cloudflare restarted while doing this; check whether it happened before trying again.";
/** What the owner is told of something held, by who it waits for. */
export const HELD_SAID: Record<HeldFor, string> = {
  either:
    "Held: it could not be done just now. Cloudflare does it as soon as it can again, or the laptop if it is back first.",
  machine: "Held: the laptop cannot be reached. It will be sent when it is back.",
  asked: "Held: the laptop did not answer. It may have got this, and says what became of it when it is back.",
};
export const ACTS_KEPT = 30;
export const AUDIT_KEPT = 300;

/** An action done here under the name the device sent it with. */
export interface Acted {
  at: number;
  kind: string;
  /** "given": handed to a machine under this name. Nothing was done
   * here, and only the machine knows what became of it. */
  state: "running" | "done" | "given";
  status?: number;
  body?: Record<string, unknown>;
  /** For "given": how many times it has set off toward a machine. Word
   * that one asking never arrived says the action reached no machine
   * only when that asking was the only one. (A record from before this
   * was counted has none, and is taken to have set off already.) */
  relays?: number;
}

/** What the stateless part of the Worker must do with a request this
 * part has checked: serve a file of the app, or pass the request to the
 * host. Anything else is an ordinary response. */
export function next(what: "asset" | "relay" | "key", detail: Record<string, unknown>): Response {
  return new Response(JSON.stringify(detail), {
    status: 200,
    headers: { "content-type": "application/json", "x-pm-next": what },
  });
}

/** An action this side tried and its token was refused for, with
 * nothing done: to be done by a machine instead. */
export class HandOver extends Error {}

export abstract class EdgeHeld {
  /** A pass over what is held is under way in this object. */
  protected draining = false;
  /** Both of those have been read back from storage. They are kept
   * there because this object is put away when idle: one that forgot
   * them would claim each section again on every start, and a machine,
   * told so, would never send its copy. */
  protected awayRead = false;
  /** Held actions already handed to a machine in a report answer and not
   * yet acknowledged: the machine may be doing them, so the edge never
   * does them itself. Anything held and not in here was never handed
   * out, and the edge may do it itself first. Kept in storage under
   * "handed", so an object that starts again knows. */
  protected handed: Set<string> | null = null;
  protected ownActs: Act[] | null = null;
  protected docAtRead = false;

  protected declare kv: KV;
  protected declare now: () => number;
  protected declare direct?: EdgeOptions["direct"];
  protected declare docs: Docs;
  protected declare table: Record<string, Action>;
  protected declare github: GitHubMemory;
  protected declare refused: Map<string, number>;
  protected declare unread: Map<string, number>;
  protected declare doing: Set<string>;
  protected declare confirmed: Record<string, { hash: string; at: number; error: string | null }>;
  protected declare machine: MachineReport | null;
  protected declare hostSeen: number | null;
  protected declare hostSeq: number | null;
  protected declare hostVersion: string | null;
  protected declare held: Held[] | null;
  protected declare taken: Held[] | null;
  protected declare busy: Map<string, Promise<void>>;
  protected declare drainNotBefore: number;
  protected declare lastDocAt: number;
  protected declare lastKeyFetch: number;
  protected declare keyFetchRead: boolean;
  protected declare hostActs: Act[];

  protected abstract config(): Promise<EdgeConfig | null>;
  protected abstract toldConfig(): Promise<EdgeConfig | null>;
  protected abstract mine(cfg: EdgeConfig | null): (DirectClasses & { table: Record<string, Action> }) | null;
  protected abstract directAct(
    kind: string,
    args: Record<string, unknown>,
    by: string,
    via: string,
    id: string | null,
  ): Promise<Response>;
  protected abstract once(id: string | null, kind: string, work: () => Promise<Response>): Promise<Response>;
  protected abstract actedIndex(id: string): Promise<void>;
  protected abstract refusedKinds(): string[];

  protected async heldList(): Promise<Held[]> {
    if (this.held === null) this.held = ((await this.kv.get("held")) as Held[] | undefined) ?? [];
    return this.held;
  }

  protected async saveHeld(): Promise<void> {
    await this.kv.put("held", this.held ?? []);
    await this.showEdge();
  }

  /** Names of held actions already handed to a machine. */
  protected async handedList(): Promise<Set<string>> {
    if (this.handed === null) {
      const stored = ((await this.kv.get("handed")) as string[] | undefined) ?? [];
      this.handed = new Set(stored.filter((id) => typeof id === "string"));
    }
    return this.handed;
  }

  protected async saveHanded(): Promise<void> {
    await this.kv.put("handed", [...(this.handed ?? [])]);
  }

  /** Forget handed names that are no longer held (acknowledged, dropped,
   * expired, or done here). */
  protected async pruneHanded(): Promise<void> {
    const held = await this.heldList();
    const handed = await this.handedList();
    let changed = false;
    for (const id of [...handed]) {
      if (!held.some((h) => h.id === id)) {
        handed.delete(id);
        changed = true;
      }
    }
    if (changed) await this.saveHanded();
  }

  protected async markHanded(ids: string[]): Promise<void> {
    if (ids.length === 0) return;
    const handed = await this.handedList();
    let changed = false;
    for (const id of ids) {
      if (!handed.has(id)) {
        handed.add(id);
        changed = true;
      }
    }
    if (changed) await this.saveHanded();
  }

  protected async unhand(ids: string[]): Promise<void> {
    if (ids.length === 0) return;
    const handed = await this.handedList();
    let changed = false;
    for (const id of ids) {
      if (handed.delete(id)) changed = true;
    }
    if (changed) await this.saveHanded();
  }

  // ---- what is held, done here ------------------------------------------
  //
  // What the owner did while this side was turned away and no machine
  // could be reached is held for a machine. When this side can act
  // again it does what it holds itself, so that nothing waits for a
  // machine that may not come. The rule that keeps anything from being
  // done twice is that an action is one side's or the other's, never
  // both. It is a machine's once it may have reached one: handed out in
  // a report's answer, or sent toward one with nothing to say that it
  // never arrived (see withMachine). It is this side's once it has been
  // taken out of the held list, which is done before anything is begun,
  // and only ever with one that certainly reached no machine.

  /** A machine has this, or may have. It was handed out in a report's
   * answer; or it is not known to have reached no machine (it went
   * toward one and nothing said that it never arrived, or it was kept
   * before that was written down). It is that machine's by name, which
   * sorts out whether it did it. This side never does it itself, and
   * nothing newer about the same thing goes ahead of it. */
  protected withMachine(h: Held, handed: Set<string>): boolean {
    return handed.has(h.id) || h.unsent !== true;
  }

  /** What a held action is about, or itself when it stands alone. */
  protected heldAbout(h: Held): string {
    return subjectOf(h.kind, h.args) ?? `:${h.id}`;
  }

  /** Who each held action waits for, as the owner is told it. The same
   * rule heldMine goes by: this side does the oldest about a thing, and
   * then the next, for as long as each is a kind it does and nothing
   * about the thing is, or may be, with a machine. */
  protected async heldWaits(cfg: EdgeConfig | null): Promise<Map<string, HeldFor>> {
    const held = await this.heldList();
    const handed = await this.handedList();
    const can = this.primary(cfg) && this.mine(cfg) !== null;
    const withMachine = new Set(held.filter((h) => this.withMachine(h, handed)).map((h) => this.heldAbout(h)));
    // Things with something ahead that only a machine can do.
    const behind = new Set<string>();
    const waits = new Map<string, HeldFor>();
    for (const h of held) {
      const about = this.heldAbout(h);
      if (this.withMachine(h, handed)) waits.set(h.id, "asked");
      else if (can && this.does(h.kind) && !withMachine.has(about) && !behind.has(about)) {
        waits.set(h.id, "either");
      } else {
        waits.set(h.id, "machine");
        behind.add(about);
      }
    }
    return waits;
  }

  /** A held action put where its time says it goes: oldest first. */
  protected insertHeld(held: Held[], item: Held): void {
    const at = held.findIndex((h) => h.at > item.at);
    if (at < 0) held.push(item);
    else held.splice(at, 0, item);
  }

  /** What this side has taken, read back when an object starts, and each
   * settled by what its record says. One that is in the held list again
   * (or never left it) is not taken. One whose record says it ended,
   * ended. One begun and not ended was being done by an object that was
   * put away: whether it happened is not known, and neither side does
   * it again; the owner is told to look. One never begun waits again,
   * where it was. */
  protected async recoverTaken(): Promise<void> {
    const stored = (await this.kv.get("taken")) as unknown;
    this.taken = [];
    if (!Array.isArray(stored) || stored.length === 0) return;
    const held = await this.heldList();
    let back = false;
    for (const item of stored as Partial<Held>[]) {
      if (
        !item ||
        typeof item.id !== "string" ||
        typeof item.kind !== "string" ||
        typeof item.at !== "string" ||
        typeof item.who !== "string" ||
        item.args === null ||
        typeof item.args !== "object"
      ) {
        continue;
      }
      const was = item as Held;
      if (held.some((h) => h.id === was.id)) continue;
      const key = `a:${was.id}`;
      const record = ((await this.kv.get(key)) as Acted | undefined) ?? null;
      if (record?.state === "done") continue;
      if (record?.state === "running") {
        await this.kv.put(key, {
          at: this.now(),
          kind: was.kind,
          state: "done",
          status: 502,
          body: { error: RESTARTED },
        } satisfies Acted);
        await this.logAct({ id: was.id, kind: was.kind, at: was.at, by: was.who, status: "failed", note: RESTARTED });
        continue;
      }
      this.insertHeld(held, was);
      back = true;
      await this.kv.put(key, { at: this.now(), kind: was.kind, state: "given" } satisfies Acted);
    }
    if (back) await this.saveHeld();
    await this.kv.delete("taken");
  }

  /** Take a held action to do here: out of the held list before anything
   * is begun, so that a machine reporting meanwhile is not handed it.
   * Null if it is not this side's to take: gone from the list, or a
   * machine's (see withMachine), which may be doing it. */
  protected async takeHeld(id: string): Promise<{ item: Held; ended: () => void } | null> {
    await this.ready();
    const held = await this.heldList();
    const handed = await this.handedList();
    // From here until the lists are changed nothing is waited for: a
    // report answered meanwhile sees the list with it or without it,
    // and a newer action about the same thing finds this one being done.
    const at = held.findIndex((h) => h.id === id);
    if (at < 0 || this.withMachine(held[at], handed)) return null;
    const [item] = held.splice(at, 1);
    const taken = (this.taken ??= []);
    taken.push(item);
    const about = this.heldAbout(item);
    let over: () => void = () => {};
    const doing = new Promise<void>((resolve) => {
      over = resolve;
    });
    this.busy.set(about, doing);
    const ended = () => {
      if (this.busy.get(about) === doing) this.busy.delete(about);
      over();
    };
    try {
      // Written down as taken before it leaves the held list: an object
      // put away between the two finds it in both, and it was not begun.
      await this.kv.put("taken", taken);
      await this.saveHeld();
    } catch (error) {
      // It could not be written down: it was not taken, and waits as
      // it did (nothing was handed out meanwhile that is about the
      // same thing, since it was this side's for as long as this took).
      this.taken = (this.taken ?? []).filter((t) => t.id !== item.id);
      if (!held.some((h) => h.id === item.id)) this.insertHeld(held, item);
      ended();
      throw error;
    }
    return { item, ended };
  }

  /** Do here a held action that never reached a machine. "done": it
   * ended here, well or badly, and its record and "What you did" say
   * how. "back": this side was turned away before anything was written,
   * and it waits again where it was, a machine's by name as before.
   * "gone": it was not this side's to take. */
  protected async doHeld(item: Held): Promise<"done" | "back" | "gone"> {
    const took = await this.takeHeld(item.id);
    if (!took) return "gone";
    const { item: taken, ended } = took;
    try {
      let outcome: "done" | "back" = "done";
      try {
        await this.once(taken.id, taken.kind, () =>
          this.directAct(taken.kind, taken.args, taken.who, "edge", taken.id),
        );
      } catch (error) {
        // Anything else is a failure of this side's own (its storage):
        // it stays taken, and the next object settles it by its record.
        if (!(error instanceof HandOver)) throw error;
        outcome = "back";
      }
      if (outcome === "back") {
        await this.kv.put(`a:${taken.id}`, { at: this.now(), kind: taken.kind, state: "given" } satisfies Acted);
        await this.actedIndex(taken.id);
        this.insertHeld(await this.heldList(), taken);
        await this.saveHeld();
      }
      this.taken = (this.taken ?? []).filter((t) => t.id !== taken.id);
      await this.kv.put("taken", this.taken);
      return outcome;
    } finally {
      ended();
    }
  }

  /** A held action that a newer one of its kind, about the same thing,
   * makes pointless (a rank set twice ends at the last): dropped, and
   * said to have been replaced. */
  protected async replaceHeld(old: Held, by: string): Promise<void> {
    const list = await this.heldList();
    const at = list.findIndex((h) => h.id === old.id);
    if (at >= 0) list.splice(at, 1);
    await this.saveHeld();
    await this.unhand([old.id]);
    await this.audit({
      at: new Date(this.now()).toISOString(),
      who: old.who,
      via: "edge",
      action: old.kind,
      phase: "done",
      ok: false,
      error: `replaced by a newer ${by}: the last one wins`,
    });
    await this.logAct({
      id: old.id,
      kind: old.kind,
      at: old.at,
      by: old.who,
      status: "failed",
      note: `Replaced by a newer ${by}: the last one wins.`,
    });
    await this.kv.put(`a:${old.id}`, {
      at: this.now(),
      kind: old.kind,
      state: "done",
      status: 409,
      body: { error: "This was replaced by a newer action about the same thing." },
    } satisfies Acted);
    await this.actedIndex(old.id);
  }

  /** For each thing held actions are about (one that stands alone is
   * its own), the oldest of them, where it is one this side could do
   * itself: a kind it runs, that certainly reached no machine, and
   * nothing else about the same thing with a machine either. Only the
   * oldest about a thing can be done next, so these are all a pass can
   * begin with. */
  protected async heldMine(cfg: EdgeConfig | null): Promise<Held[]> {
    if (!this.primary(cfg) || !this.mine(cfg)) return [];
    const held = await this.heldList();
    const handed = await this.handedList();
    const withMachine = new Set(held.filter((h) => this.withMachine(h, handed)).map((h) => this.heldAbout(h)));
    const seen = new Set<string>();
    const first: Held[] = [];
    for (const h of held) {
      const about = this.heldAbout(h);
      if (seen.has(about)) continue;
      seen.add(about);
      if (this.does(h.kind) && !withMachine.has(about)) first.push(h);
    }
    return first;
  }

  /** When this side may next try one of its kind: not while it is
   * turned away for the kind, nor before the time GitHub gave. (Every
   * kind is kept to GitHub's time, though one or two ask only the queue
   * store: tried sooner, the rest would only be turned away again.) */
  protected async heldReadyAt(kind: string): Promise<number> {
    this.refusedKinds();
    return Math.max(this.refused.get(kind) ?? 0, (await this.github.quiet()).core?.until ?? 0);
  }

  /** The time of the next thing to do about what is held: the first of
   * it this side could do itself, and the day's end of the oldest of it
   * whoever's it is. Null when nothing is held. */
  protected async heldDue(cfg: EdgeConfig | null): Promise<number | null> {
    const held = await this.heldList();
    if (held.length === 0) return null;
    const times = held.map((h) => Date.parse(h.at) + HELD_FOR_MS + 1000).filter((t) => Number.isFinite(t));
    for (const h of await this.heldMine(cfg)) {
      times.push(Math.max(await this.heldReadyAt(h.kind), this.drainNotBefore));
    }
    return times.length > 0 ? Math.min(...times) : null;
  }

  /** What has waited more than a day is dropped, and said to have been. */
  protected async expireHeld(now: number): Promise<boolean> {
    const held = await this.heldList();
    const handed = await this.handedList();
    let dropped = false;
    for (const old of held.filter((h) => now - Date.parse(h.at) > HELD_FOR_MS)) {
      // Said before it leaves the list: whether a machine may have had it.
      const maybe = this.withMachine(old, handed);
      held.splice(held.indexOf(old), 1);
      dropped = true;
      await this.logAct({
        id: old.id,
        kind: old.kind,
        at: old.at,
        by: old.who,
        status: "failed",
        note: maybe
          ? "The laptop may have got this, and did not say what became of it within a day: check whether it happened before trying again."
          : "Not sent: the laptop was out of reach for more than a day.",
      });
    }
    return dropped;
  }

  /** What is held and never reached a machine is done here, now that
   * this side can: the oldest about each thing first, and nothing about
   * a thing behind one that could not be done. One pass at a time. */
  protected async drainHeld(cfg: EdgeConfig | null): Promise<void> {
    if (this.draining || this.now() < this.drainNotBefore) return;
    this.draining = true;
    let did = 0;
    let tried = false;
    try {
      await this.ready();
      // Left for another pass: what this one could not do, and
      // everything behind it about the same thing.
      const left = new Set<string>();
      for (let turn = 0; turn < 2 * HELD_MAX; turn++) {
        const now = this.now();
        let pick: Held | null = null;
        for (const h of await this.heldMine(cfg)) {
          const about = this.heldAbout(h);
          if (left.has(about)) continue;
          if (this.doing.has(h.id) || (await this.heldReadyAt(h.kind)) > now) {
            left.add(about);
            continue;
          }
          pick = h;
          break;
        }
        if (!pick) break;
        tried = true;
        const about = subjectOf(pick.kind, pick.args);
        const one = pick;
        if (
          LAST_WINS.has(one.kind) &&
          about !== null &&
          (await this.heldList()).some(
            (h) => h.id !== one.id && h.kind === one.kind && subjectOf(h.kind, h.args) === about,
          )
        ) {
          await this.replaceHeld(one, one.kind);
          did++;
          continue;
        }
        const went = await this.doHeld(one);
        if (went === "done") did++;
        else left.add(about ?? `:${one.id}`);
      }
    } finally {
      this.draining = false;
      // A pass that tried and could do nothing is not tried again at
      // once. (One that found nothing due yet sets no such pause.)
      if (tried && did === 0) this.drainNotBefore = this.now() + DRAIN_AGAIN_MS;
    }
  }

  /** The platform's own part, each machine that reports, and what is
   * held for one: what the Machines screen is drawn from. Answered
   * here, so it is there with every machine away. */
  protected async machinesSaid(cfg: EdgeConfig | null): Promise<MachinesAnswer> {
    await this.ready();
    const primary = this.primary(cfg);
    const refused = this.refusedKinds();
    const seen = this.hostSeen || null;
    const told = await this.toldConfig();
    const differs = told === null ? [] : settingsDifferences(told);
    return {
      served: "edge",
      platform: {
        primary,
        // What it does itself right now. Its own (notifications, linked
        // phones, what is held) always; the core when it has the means,
        // less what it was turned away for and a machine does for the hour.
        actions: [
          ...Object.keys(this.table).sort(),
          ...(primary ? this.kinds().filter((kind) => !refused.includes(kind)) : []),
        ],
        sections: primary ? this.owned(cfg) : [],
        refused: [...refused, ...this.refusedReads().map((name) => `read.${name}`)],
      },
      // One machine can report today: the one whose key this side holds.
      machines:
        seen === null
          ? []
          : [
              {
                name: this.machine?.name ?? "A machine",
                seen,
                quiet: this.now() - seen >= HOST_QUIET_MS,
                version: this.hostVersion,
                actions: this.machine?.actions ?? [],
                sections: this.machine?.sections ?? [],
                ...(differs.length > 0 ? { differs } : {}),
              },
            ],
      held: await this.heldSaid(cfg),
    };
  }

  /** How much is held, by who it waits for. */
  protected async heldCounts(cfg: EdgeConfig | null): Promise<Record<HeldFor, number>> {
    const counts: Record<HeldFor, number> = { either: 0, machine: 0, asked: 0 };
    for (const waits of (await this.heldWaits(cfg)).values()) counts[waits]++;
    return counts;
  }

  /** What is held, each with who it waits for. */
  protected async heldSaid(cfg: EdgeConfig | null): Promise<(Held & { waits: HeldFor })[]> {
    const waits = await this.heldWaits(cfg);
    return (await this.heldList()).map((h) => ({ ...h, waits: waits.get(h.id) ?? "machine" }));
  }

  /** What the app is told of this side, beside its state: how much is
   * waiting to be done, and whether this side does the core itself. */
  protected async showEdge(): Promise<void> {
    this.docs.extra("edge", { held: (this.held ?? []).length, primary: this.primary(await this.config()) });
  }

  /** What the owner did lately: this side's own record (what it ran or
   * is holding) together with the host's, newest first. */
  protected async showActs(): Promise<void> {
    if (this.ownActs === null) this.ownActs = ((await this.kv.get("acts")) as Act[] | undefined) ?? [];
    const merged = [...this.ownActs, ...this.hostActs.filter((h) => !this.ownActs?.some((o) => o.id === h.id))];
    merged.sort((a, b) => (a.at < b.at ? 1 : a.at > b.at ? -1 : 0));
    this.docs.restoreActs(merged);
  }

  protected async logAct(act: Act): Promise<void> {
    if (this.ownActs === null) this.ownActs = ((await this.kv.get("acts")) as Act[] | undefined) ?? [];
    this.ownActs = [act, ...this.ownActs.filter((a) => a.id !== act.id)].slice(0, ACTS_KEPT);
    await this.kv.put("acts", this.ownActs);
    await this.showActs();
  }

  protected async dropAct(id: string): Promise<void> {
    if (this.ownActs === null) this.ownActs = ((await this.kv.get("acts")) as Act[] | undefined) ?? [];
    this.ownActs = this.ownActs.filter((a) => a.id !== id);
    await this.kv.put("acts", this.ownActs);
    await this.showActs();
  }

  protected async audit(entry: Record<string, unknown>): Promise<void> {
    const log = ((await this.kv.get("audit")) as Record<string, unknown>[] | undefined) ?? [];
    log.push(entry);
    await this.kv.put("audit", log.slice(-AUDIT_KEPT));
  }

  protected async ready(): Promise<void> {
    // After a restart: the held count and the record of what was done
    // are read back before the first document is built.
    if (this.held === null) {
      await this.heldList();
      await this.showEdge();
    }
    if (this.taken === null) await this.recoverTaken();
    if (this.ownActs === null) await this.showActs();
    if (this.hostSeen === null) {
      const host = (await this.kv.get("host")) as
        | {
            seen: number;
            seq: number;
            confirmed?: Record<string, { hash: string; at: number; error: string | null }>;
            version?: string | null;
            machine?: unknown;
          }
        | undefined;
      this.hostSeen = host?.seen ?? 0;
      this.hostSeq = host?.seq ?? 0;
      this.confirmed = host?.confirmed ?? {};
      this.hostVersion = host?.version ?? null;
      this.machine = machineReport(host?.machine);
    }
    if (!this.awayRead) {
      this.awayRead = true;
      const kept = (await this.kv.get("away")) as
        | { kinds?: Record<string, unknown>; reads?: Record<string, unknown> }
        | undefined;
      const now = this.now();
      // Only what still stands, and nothing further off than it could
      // have been set for (the longest is GitHub's own wait): a time
      // like that is not one this side wrote, and is not believed at
      // all. (Cut down to the longest instead, as it once was, it was
      // cut down afresh by every object that started, each from its
      // own now, and never came.)
      const still = (from: Record<string, unknown> | undefined, into: Map<string, number>) => {
        for (const [name, until] of Object.entries(from ?? {})) {
          if (typeof until === "number" && until > now && until <= now + QUIET_MOST_MS && !into.has(name)) {
            into.set(name, until);
          }
        }
      };
      still(kept?.kinds, this.refused);
      still(kept?.reads, this.unread);
    }
    if (!this.docAtRead) {
      this.docAtRead = true;
      try {
        const at = (await this.kv.get("docAt")) as number | undefined;
        const now = this.now();
        if (typeof at === "number" && at > 0 && at <= now) this.lastDocAt = at;
      } catch {
        // Unread storage is no storage: nobody is looking until a read.
      }
    }
    if (!this.keyFetchRead) {
      this.keyFetchRead = true;
      try {
        const at = (await this.kv.get("keyFetch")) as number | undefined;
        const now = this.now();
        if (typeof at === "number" && at > 0 && at <= now) this.lastKeyFetch = at;
      } catch {
        // Unread storage is no storage: the next stranger fetches.
      }
    }
  }

  /** Write down what this side is turned away for, so that an object
   * that starts again knows. */
  protected async saveAway(): Promise<void> {
    await this.kv.put("away", { kinds: Object.fromEntries(this.refused), reads: Object.fromEntries(this.unread) });
  }

  // ---- doing the core here ------------------------------------------------

  /** Whether this side holds the PM bridge credential and the bridge
   * binding (#14373): the two together make the bridge this side's. */
  protected bridgeReady(): boolean {
    return Boolean(this.direct?.bridgeToken) && Boolean(this.direct?.bridgeFetch);
  }

  /** The kinds this side does itself when it does the core. The
   * composer's kinds — a message, its broaden, its offer (#14375) —
   * are this side's only with the bridge held (bridgeReady): without
   * it each stays a machine's, as message.send was before the bridge
   * path existed, and is held for one like any other. */
  protected kinds(): readonly string[] {
    const base = this.direct?.kinds ?? DIRECT_ACTIONS;
    if (!this.bridgeReady()) return base;
    const composer = ["message.send", "message.broaden", "message.offer"].filter((k) => !base.includes(k));
    return composer.length ? [...base, ...composer] : base;
  }

  /** A kind this side does itself when it does the core; any other is
   * a machine's whenever one comes. */
  protected does(kind: string): boolean {
    return this.kinds().includes(kind);
  }

  /** The switch is on and both tokens are here. */
  protected directCapable(): boolean {
    return this.direct?.enabled === true && Boolean(this.direct.githubToken) && Boolean(this.direct.queueToken);
  }

  /** This side does the core itself: it has the switch and the tokens,
   * and a machine has said who may sign in. Where things are it knows
   * by itself (src/settings.ts). Whether a machine is reporting makes
   * no difference to this. */
  protected primary(cfg: EdgeConfig | null): boolean {
    return this.directCapable() && cfg !== null;
  }

  /** A section this side reads itself. */
  protected owns(name: string, cfg: EdgeConfig | null): boolean {
    return this.owned(cfg).includes(name);
  }

  /** The sections this side reads itself: all it can while it does the
   * core, less any its token was refused for within the hour. Those are
   * a machine's to read meanwhile, exactly as before this side read
   * anything: a token that may do less than was thought leaves the app
   * no worse off than it was. */
  protected owned(cfg: EdgeConfig | null): string[] {
    if (!this.primary(cfg)) return [];
    const refused = this.refusedReads();
    return DIRECT_READ_SECTIONS.filter(
      (name) =>
        !refused.includes(name) &&
        // What is labelled as needing the owner is read repository by
        // repository: a machine that has not said which they are (one
        // from before it did) goes on reporting the list itself. Read
        // from no repositories it would be an empty list, and whole.
        (name !== "census" || (cfg?.needsRepos.length ?? 0) > 0) &&
        // The agents list is the bridge's: read here only while this
        // side holds the PM bridge credential (#14373). Without it a
        // machine goes on reporting the list, as it did before.
        (name !== "agents" || this.bridgeReady()) &&
        // The pipeline count is the bridge's the same way (#14377).
        (name !== "pipeline" || this.bridgeReady()),
    );
  }

  /** Sections this side's token was turned away for, and still is. */
  protected refusedReads(): string[] {
    const now = this.now();
    for (const [name, until] of this.unread) if (now >= until) this.unread.delete(name);
    return [...this.unread.keys()].sort();
  }
}
