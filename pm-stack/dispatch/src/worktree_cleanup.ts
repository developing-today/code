// Dispatch worktree cleanup lifecycle (code#12217).
//
// createDispatchWorktree gives each launch its own detached tree under
// the fleet worktree root, and nothing removed those trees afterwards.
// The generic worktree-reaper deliberately retains detached trees
// (detached-v1 policy), so dispatch owns the lifecycle of the trees it
// creates. This module is that lifecycle:
//
//   - a registry file records the exact tree created for a launch
//     (task, session, primary, base commit). A cleanup target is ONLY
//     a registered path — never a directory scan, never a name guess;
//   - a tree is retired only after confirmed session termination: the
//     recorded session is gone (404 from the OpenCode server, or
//     deleted by dispatch itself), and no other session listed by the
//     server occupies the tree. An unreachable server, a transport
//     failure or an unreadable session list is AMBIGUOUS state — the
//     tree is retained (fail closed);
//   - before removal the tree must be a registered worktree of its
//     primary, unlocked, unoccupied (no process cwd inside it),
//     exactly clean, and still at the base commit it was created at.
//     Anything else — uncommitted changes, untracked files, commits
//     beyond base, a lock, an occupant, a git error — retains the
//     tree: user work is never discarded;
//   - removal is idempotent (an already-absent tree simply completes
//     its record) and every cleanup failure is reported as an outcome,
//     never thrown past the caller, so it can never mask a
//     dispatch/session result.
import { existsSync, mkdirSync, readFileSync, readdirSync, readlinkSync, realpathSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { runGit, type GitRunner } from "./worktree.ts";

export class WorktreeCleanupError extends Error {}

const msg = (e: unknown): string => (e instanceof Error ? e.message : String(e));
const nowIso = (): string => new Date().toISOString();

// ---------------------------------------------------------------------------
// Registry: the durable worktree <-> task/session association
// ---------------------------------------------------------------------------

/** One launch's worktree record. `state` is "active" until the tree is
 * confirmed gone; retention reasons are recorded on `lastCheck`, never
 * acted on destructively. */
export interface WorktreeRecord {
  taskId: number;
  /** The OpenCode session bound to the tree, or null when the launch
   * died before the session id was recorded (the session-list check
   * still protects that tree — see worktreeSessionEvidence). */
  sessionId: string | null;
  repo: string;
  /** The primary clone the tree was created from (git -C target). */
  primary: string;
  worktree: string;
  /** The commit the tree was created at; HEAD must still equal it. */
  base: string;
  createdAt: string;
  state: "active" | "removed";
  lastCheck: { at: string; outcome: "removed" | "retained"; reason: string } | null;
}

export type NewWorktreeRecord = Omit<WorktreeRecord, "state" | "lastCheck">;

function parseRecords(raw: string, file: string): WorktreeRecord[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    throw new WorktreeCleanupError(`worktree registry ${file} is not valid JSON`);
  }
  if (!Array.isArray(parsed)) {
    throw new WorktreeCleanupError(`worktree registry ${file} does not hold a JSON array`);
  }
  const str = (v: unknown): string | null => (typeof v === "string" ? v : null);
  const records: WorktreeRecord[] = [];
  for (const entry of parsed) {
    if (typeof entry !== "object" || entry === null) {
      throw new WorktreeCleanupError(`worktree registry ${file} holds a non-object entry`);
    }
    const e = entry as Record<string, unknown>;
    const worktree = str(e.worktree);
    const primary = str(e.primary);
    const base = str(e.base);
    if (worktree === null || primary === null || base === null) {
      throw new WorktreeCleanupError(`worktree registry ${file} holds an entry without worktree/primary/base`);
    }
    const state = e.state === "removed" ? "removed" : "active";
    const last = (e.lastCheck as Record<string, unknown> | null) ?? null;
    records.push({
      taskId: typeof e.taskId === "number" ? e.taskId : 0,
      sessionId: str(e.sessionId),
      repo: str(e.repo) ?? "unknown",
      primary,
      worktree,
      base,
      createdAt: str(e.createdAt) ?? "",
      state,
      lastCheck:
        last !== null && typeof last === "object" && (last.outcome === "removed" || last.outcome === "retained")
          ? { at: str(last.at) ?? "", outcome: last.outcome, reason: str(last.reason) ?? "" }
          : null,
    });
  }
  return records;
}

/** Durable registry of launch worktrees. Mutations are serialized
 * inside this process (load-modify-save with an atomic tmp+rename), so
 * concurrent launches can never clobber each other's records. A load
 * failure THROWS (WorktreeCleanupError): callers fail closed — a sweep
 * that cannot read its records touches nothing, and a launch that
 * cannot record its tree still launches (the tree is simply never
 * auto-removed). */
export class WorktreeRegistry {
  private chain: Promise<unknown> = Promise.resolve();

  constructor(readonly file: string) {}

  async load(): Promise<WorktreeRecord[]> {
    let raw: string;
    try {
      raw = readFileSync(this.file, "utf8");
    } catch {
      return []; // no file yet: nothing was ever registered under this path
    }
    return parseRecords(raw, this.file);
  }

  /** Register a freshly created tree. A record at the same path is
   * replaced wholesale: the path names a brand-new tree (the creator
   * refuses to reuse an existing directory), so any old record for it
   * describes a tree that no longer exists. */
  async add(record: NewWorktreeRecord): Promise<WorktreeRecord> {
    return this.serialized(async () => {
      const records = await this.load();
      const full: WorktreeRecord = { ...record, state: "active", lastCheck: null };
      const existing = records.findIndex((r) => r.worktree === full.worktree);
      if (existing >= 0) records[existing] = full;
      else records.push(full);
      this.save(records);
      return full;
    });
  }

  /** Bind the launch's session id onto the record (called right after
   * session creation; a crash before this leaves sessionId null, which
   * the session-list evidence check still handles conservatively). */
  async setSessionId(worktree: string, sessionId: string): Promise<void> {
    await this.serialized(async () => {
      const records = await this.load();
      const record = records.find((r) => r.worktree === worktree);
      if (!record) return;
      record.sessionId = sessionId;
      this.save(records);
    });
  }

  /** Record the outcome of one cleanup check. */
  async mark(worktree: string, outcome: "removed" | "retained", reason: string, at: string = nowIso()): Promise<void> {
    await this.serialized(async () => {
      const records = await this.load();
      const record = records.find((r) => r.worktree === worktree);
      if (!record) return;
      record.lastCheck = { at, outcome, reason };
      if (outcome === "removed") record.state = "removed";
      this.save(records);
    });
  }

  private serialized<T>(fn: () => Promise<T>): Promise<T> {
    const run = this.chain.then(fn, fn);
    this.chain = run.catch(() => {});
    return run;
  }

  private save(records: WorktreeRecord[]): void {
    mkdirSync(dirname(this.file), { recursive: true });
    const tmp = `${this.file}.tmp-${process.pid}-${Math.random().toString(36).slice(2)}`;
    writeFileSync(tmp, JSON.stringify(records, null, 2) + "\n");
    renameSync(tmp, this.file);
  }
}

// ---------------------------------------------------------------------------
// Session termination evidence
// ---------------------------------------------------------------------------

export interface SessionEvidence {
  /** True only when termination is CONFIRMED; every unknown is retained. */
  terminated: boolean;
  /** How the decision was reached — recorded on the record either way. */
  reason: string;
}

/** The directory a raw session object runs in (the v2 envelope is
 * already unwrapped by the client; the object carries either
 * `directory` or `location.directory`, like sessionStatusView). */
function sessionDirectory(raw: unknown): string | null {
  if (typeof raw !== "object" || raw === null) return null;
  const obj = raw as Record<string, unknown>;
  if (typeof obj.directory === "string") return obj.directory;
  const loc = obj.location as Record<string, unknown> | undefined;
  return typeof loc?.directory === "string" ? loc.directory : null;
}

function sessionIdOf(raw: unknown): string | null {
  if (typeof raw === "object" && raw !== null && typeof (raw as Record<string, unknown>).id === "string") {
    return (raw as Record<string, unknown>).id as string;
  }
  return null;
}

function containsPath(dir: string, treeReal: string): boolean {
  let d: string;
  try {
    d = realpathSync(dir);
  } catch {
    d = dir; // the session's directory may be recorded but gone; compare literally
  }
  return d === treeReal || d.startsWith(treeReal + "/");
}

/** Evidence that no live or resumable session occupies the worktree.
 *
 * - recorded session id: the OpenCode server must report it gone (the
 *   client's getSession resolves null on 404); a live object or any
 *   error (server down, auth, 5xx) retains the tree;
 * - `confirmedTerminated` skips that first lookup when the caller itself
 *   deleted the session (the launch-failure path) — the occupancy
 *   cross-check below still runs;
 * - the session-list cross-check then proves no OTHER session (one
 *   dispatch never recorded, e.g. created by a human) runs in the tree;
 *   a failed or unreadable list is ambiguous state and retains it.
 *   For records with no session id at all (a launch that died between
 *   worktree creation and session creation) this list is the ONLY
 *   evidence either way. */
export async function worktreeSessionEvidence(args: {
  record: Pick<WorktreeRecord, "sessionId" | "worktree">;
  getSession: (sessionId: string) => Promise<unknown>;
  listSessions: () => Promise<unknown>;
  confirmedTerminated?: boolean;
}): Promise<SessionEvidence> {
  if (args.confirmedTerminated !== true && args.record.sessionId != null) {
    let session: unknown;
    try {
      session = await args.getSession(args.record.sessionId);
    } catch (e) {
      return { terminated: false, reason: `session ${args.record.sessionId} state unknown: ${msg(e)} — retained` };
    }
    if (session != null) {
      return { terminated: false, reason: `session ${args.record.sessionId} exists (live or resumable)` };
    }
  }
  if (args.confirmedTerminated !== true && args.record.sessionId == null) {
    // No session id recorded: a crash between worktree creation and
    // session creation may still have left a live session in the tree,
    // so the session list below decides — nothing else can.
  }
  try {
    const sessions = await args.listSessions();
    if (!Array.isArray(sessions)) {
      return { terminated: false, reason: "session list returned an ambiguous shape; retained" };
    }
    const list = sessions;
    let treeReal = args.record.worktree;
    try {
      treeReal = realpathSync(args.record.worktree);
    } catch {
      /* tree already gone: any session directory still inside the
       * recorded path (compared literally) counts as an occupant */
    }
    for (const raw of list) {
      const dir = sessionDirectory(raw);
      if (dir !== null && containsPath(dir, treeReal)) {
        const id = sessionIdOf(raw) ?? "(unknown id)";
        return { terminated: false, reason: `session ${id} still occupies the worktree` };
      }
    }
    return {
      terminated: true,
      reason: "recorded session confirmed gone; no session occupies the worktree",
    };
  } catch (e) {
    return {
      terminated: false,
      reason: `session list unavailable; cannot rule out a live session in the worktree: ${msg(e)} — retained`,
    };
  }
}

// ---------------------------------------------------------------------------
// Filesystem, process-occupancy and Git safety gates
// ---------------------------------------------------------------------------

export interface CleanupOutcome {
  worktree: string;
  removed: boolean;
  reason: string;
}

function isProcessUsingPath(worktree: string): boolean {
  let treeReal: string;
  try {
    treeReal = realpathSync(worktree);
  } catch {
    return false;
  }
  let pids: string[];
  try {
    pids = readdirSync("/proc").filter((entry) => /^\d+$/.test(entry));
  } catch {
    // Cannot prove unoccupied on non-Linux or an inaccessible procfs.
    return true;
  }
  for (const pid of pids) {
    try {
      const cwd = readlinkSync(`/proc/${pid}/cwd`);
      if (containsPath(cwd, treeReal)) return true;
    } catch {
      // A process exiting during the scan is harmless; unreadable cwd
      // for an existing PID is handled by the next pass / session list.
    }
  }
  return false;
}

function isLocked(primary: string, worktree: string): boolean {
  const r = Bun.spawnSync(["git", "-C", primary, "worktree", "list", "--porcelain"]);
  if (r.exitCode !== 0) throw new WorktreeCleanupError(`git worktree list failed: ${r.stderr.toString().trim()}`);
  const lines = r.stdout.toString().split("\n");
  let selected = false;
  let found = false;
  let target = worktree;
  try { target = realpathSync(worktree); } catch { /* compare literal path */ }
  for (const line of lines) {
    if (line.startsWith("worktree ")) {
      let listed = line.slice(9);
      try { listed = realpathSync(listed); } catch { /* compare literal path */ }
      selected = listed === target;
      found ||= selected;
    }
    else if (selected && line === "locked") return true;
  }
  return !found; // not registered with this primary: fail closed
}

/** Clean only the exact registered target. The caller supplies
 * termination evidence and may pass `confirmedTerminated` when the
 * session was deleted by dispatch itself; all other uncertainty is a
 * retained outcome. This helper never throws cleanup errors. */
export async function cleanupDispatchWorktree(args: {
  record: WorktreeRecord;
  registry: WorktreeRegistry;
  getSession: (sessionId: string) => Promise<unknown>;
  listSessions: () => Promise<unknown>;
  confirmedTerminated?: boolean;
  git?: GitRunner;
  isOccupied?: (worktree: string) => boolean;
  at?: string;
}): Promise<CleanupOutcome> {
  const { record, registry } = args;
  const finish = async (removed: boolean, reason: string): Promise<CleanupOutcome> => {
    try {
      await registry.mark(record.worktree, removed ? "removed" : "retained", reason, args.at);
    } catch (e) {
      // Registry writes are best-effort after safety evaluation; report
      // the error but never throw into a launch/session caller.
      return { worktree: record.worktree, removed: false, reason: `${reason}; registry update failed: ${msg(e)}` };
    }
    return { worktree: record.worktree, removed, reason };
  };
  try {
    if (record.state === "removed") return { worktree: record.worktree, removed: true, reason: "already removed" };
    if (!existsSync(record.worktree)) return finish(true, "worktree already absent");

    const evidence = await worktreeSessionEvidence({
      record,
      getSession: args.getSession,
      listSessions: args.listSessions,
      confirmedTerminated: args.confirmedTerminated,
    });
    if (!evidence.terminated) return finish(false, evidence.reason);

    const occupied = args.isOccupied ?? isProcessUsingPath;
    if (occupied(record.worktree)) return finish(false, "a process has its cwd in the worktree");
    if (isLocked(record.primary, record.worktree)) return finish(false, "worktree is locked");

    const git = args.git ?? runGit;
    const head = await git(["-C", record.worktree, "rev-parse", "--verify", "HEAD"]);
    if (head.code !== 0 || head.stdout.trim() !== record.base) {
      return finish(false, `HEAD is not the recorded base commit (${head.stdout.trim() || "unavailable"}); retained`);
    }
    const status = await git(["-C", record.worktree, "status", "--porcelain", "--untracked-files=all"]);
    if (status.code !== 0) return finish(false, `git status failed: ${status.stderr.trim()}; retained`);
    if (status.stdout.trim()) return finish(false, `worktree has uncommitted or untracked work; retained`);

    const remove = await git(["-C", record.primary, "worktree", "remove", record.worktree]);
    if (remove.code !== 0) return finish(false, `git worktree remove failed: ${remove.stderr.trim().slice(0, 300)}; retained`);
    return finish(true, "session ended and worktree was clean, unoccupied, unlocked, and at its recorded base commit; removed");
  } catch (e) {
    return finish(false, `cleanup check failed: ${msg(e)} — retained`);
  }
}

/** Sweep every active registry record. Each result is isolated so one
 * malformed or unsafe tree cannot prevent later records being checked.
 * A registry that cannot be read surfaces as a single retained
 * outcome naming the problem — never a throw, never a removal. */
export async function sweepDispatchWorktrees(args: {
  registry: WorktreeRegistry;
  getSession: (sessionId: string) => Promise<unknown>;
  listSessions: () => Promise<unknown>;
  git?: GitRunner;
  isOccupied?: (worktree: string) => boolean;
}): Promise<CleanupOutcome[]> {
  let records: WorktreeRecord[];
  try {
    records = await args.registry.load();
  } catch (e) {
    return [{ worktree: args.registry.file, removed: false, reason: `registry unavailable: ${msg(e)}` }];
  }
  const outcomes: CleanupOutcome[] = [];
  for (const record of records) {
    if (record.state === "removed") continue;
    outcomes.push(
      await cleanupDispatchWorktree({
        record,
        registry: args.registry,
        getSession: args.getSession,
        listSessions: args.listSessions,
        git: args.git,
        isOccupied: args.isOccupied,
      }),
    );
  }
  return outcomes;
}
