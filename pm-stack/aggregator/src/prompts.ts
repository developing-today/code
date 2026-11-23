// PM-stack prompt submissions (#10032 S1, #10035 S6 chat/session surface, design #9539).
// A submission is a prompt typed into a prompt widget, stored as one record in
// a JSON file in the aggregator's data directory (PM_AGG_PROMPTS_FILE). Writes
// are atomic (temp file + rename), like the comments store. This slice stores
// submissions only; enqueueing them to the fleet queue is a later slice.
//
// S6 adds the chat/session surface on top of the same store: threads (replyTo),
// cross-references (refs), archive toggle, read/unread tracking for the
// notification count, and an opt-in push flag (notify).
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { dirname } from "node:path";

export const PROMPT_SCOPES = ["section", "page", "stack", "agent"] as const;
export const PROMPT_TARGETS = ["claude", "muse", "pool:auto"] as const;
export const PROMPT_MODES = ["async"] as const;
export const PROMPT_STATUSES = ["queued", "running", "answered", "failed"] as const;

export type PromptScope = (typeof PROMPT_SCOPES)[number];
export type PromptTarget = (typeof PROMPT_TARGETS)[number];
export type PromptMode = (typeof PROMPT_MODES)[number];
export type PromptStatus = (typeof PROMPT_STATUSES)[number];

export interface NewPrompt {
  scope: PromptScope;
  target: PromptTarget;
  mode: PromptMode;
  page: string;
  sectionRef: string | null;
  prompt: string;
  replyTo?: string | null;
  refs?: string[];
  notify?: boolean;
}

export interface PromptRecord extends NewPrompt {
  id: string;
  threadId: string;
  status: PromptStatus;
  createdAt: string;
  answer?: string;
  error?: string;
  archived: boolean;
  read: boolean;
  replyTo: string | null;
  refs: string[];
  notify: boolean;
}

export interface PromptsFile {
  version: 1;
  prompts: PromptRecord[];
}

const MAX_PROMPT_CHARS = 8000;
const PAGE_ID = /^[a-z][a-z0-9-]{0,63}$/;

function normalizeRecord(raw: Partial<PromptRecord> & Record<string, unknown>): PromptRecord {
  const replyTo = typeof raw.replyTo === "string" ? raw.replyTo : null;
  const refs = Array.isArray(raw.refs) ? raw.refs.filter((x): x is string => typeof x === "string") : [];
  return {
    scope: (PROMPT_SCOPES.includes(raw.scope as PromptScope) ? raw.scope : "page") as PromptScope,
    target: (PROMPT_TARGETS.includes(raw.target as PromptTarget) ? raw.target : "claude") as PromptTarget,
    mode: (PROMPT_MODES.includes(raw.mode as PromptMode) ? raw.mode : "async") as PromptMode,
    page: typeof raw.page === "string" && PAGE_ID.test(raw.page) ? raw.page : "queue",
    sectionRef: typeof raw.sectionRef === "string" ? raw.sectionRef : null,
    prompt: typeof raw.prompt === "string" ? raw.prompt : "",
    replyTo,
    refs,
    notify: raw.notify === true,
    id: typeof raw.id === "string" ? raw.id : `prompt:${randomUUID()}`,
    threadId:
      typeof raw.threadId === "string" ? raw.threadId : typeof raw.id === "string" ? raw.id : `thread:${randomUUID()}`,
    status: (PROMPT_STATUSES.includes(raw.status as PromptStatus) ? raw.status : "queued") as PromptStatus,
    createdAt: typeof raw.createdAt === "string" ? raw.createdAt : new Date().toISOString(),
    answer: typeof raw.answer === "string" ? raw.answer : undefined,
    error: typeof raw.error === "string" ? raw.error : undefined,
    archived: raw.archived === true,
    read: raw.read === true,
  };
}

export function validateNewPrompt(body: unknown): { ok: true; value: NewPrompt } | { ok: false; error: string } {
  if (typeof body !== "object" || body === null || Array.isArray(body)) {
    return { ok: false, error: "body must be a JSON object" };
  }
  const b = body as Record<string, unknown>;
  if (!PROMPT_SCOPES.includes(b.scope as PromptScope))
    return { ok: false, error: "scope must be section, page, stack or agent" };
  if (!PROMPT_TARGETS.includes(b.target as PromptTarget))
    return { ok: false, error: "target must be claude, muse or pool:auto" };
  if (!PROMPT_MODES.includes(b.mode as PromptMode)) return { ok: false, error: "mode must be async" };
  if (typeof b.page !== "string" || !PAGE_ID.test(b.page)) return { ok: false, error: "page must be a page id" };
  if (b.sectionRef !== null && b.sectionRef !== undefined && typeof b.sectionRef !== "string") {
    return { ok: false, error: "sectionRef must be a string or null" };
  }
  if (b.scope === "section" && (typeof b.sectionRef !== "string" || b.sectionRef === "")) {
    return { ok: false, error: "section scope requires sectionRef" };
  }
  if (typeof b.prompt !== "string" || b.prompt.trim() === "")
    return { ok: false, error: "prompt must be non-empty text" };
  if (b.prompt.length > MAX_PROMPT_CHARS) return { ok: false, error: `prompt exceeds ${MAX_PROMPT_CHARS} characters` };

  // S6 optional fields
  let replyTo: string | null | undefined;
  if (b.replyTo !== undefined && b.replyTo !== null) {
    if (typeof b.replyTo !== "string" || b.replyTo.trim() === "") {
      return { ok: false, error: "replyTo must be a string or null" };
    }
    replyTo = b.replyTo;
  } else if (b.replyTo === null) {
    replyTo = null;
  }
  let refs: string[] | undefined;
  if (b.refs !== undefined) {
    if (!Array.isArray(b.refs) || !b.refs.every((x) => typeof x === "string")) {
      return { ok: false, error: "refs must be an array of strings" };
    }
    refs = b.refs as string[];
  }
  let notify: boolean | undefined;
  if (b.notify !== undefined) {
    if (typeof b.notify !== "boolean") return { ok: false, error: "notify must be a boolean" };
    notify = b.notify;
  }

  const value: NewPrompt = {
    scope: b.scope as PromptScope,
    target: b.target as PromptTarget,
    mode: b.mode as PromptMode,
    page: b.page as string,
    sectionRef: (b.sectionRef as string | null | undefined) ?? null,
    prompt: b.prompt as string,
  };
  if (replyTo !== undefined) value.replyTo = replyTo;
  if (refs !== undefined) value.refs = refs;
  if (notify !== undefined) value.notify = notify;
  return { ok: true, value };
}

export class PromptsStore {
  private prompts: PromptRecord[] = [];
  private loadPromise: Promise<void> | null = null;
  private writeChain: Promise<void> = Promise.resolve();
  private writeSeq = 0;

  constructor(private readonly path: string) {}

  async load(): Promise<void> {
    if (!this.loadPromise) this.loadPromise = this.doLoad();
    return this.loadPromise;
  }

  private async doLoad(): Promise<void> {
    try {
      const parsed = JSON.parse(await readFile(this.path, "utf8")) as PromptsFile | { prompts?: unknown[] };
      const raw = Array.isArray((parsed as { prompts?: unknown[] }).prompts)
        ? (parsed as { prompts: unknown[] }).prompts
        : [];
      this.prompts = raw.map((r) => normalizeRecord((r ?? {}) as Partial<PromptRecord> & Record<string, unknown>));
    } catch (err) {
      if ((err as NodeJS.ErrnoException).code !== "ENOENT") throw err;
    }
  }

  async add(input: NewPrompt, now = new Date()): Promise<PromptRecord> {
    await this.load();
    const replyTo = input.replyTo ?? null;
    // Resolve thread: inherit from referenced prompt if present.
    let threadId: string;
    if (replyTo) {
      const parent = this.prompts.find((p) => p.id === replyTo);
      threadId = parent ? parent.threadId : `thread:${randomUUID()}`;
    } else {
      threadId = `thread:${randomUUID()}`;
    }
    const record: PromptRecord = {
      scope: input.scope,
      target: input.target,
      mode: input.mode,
      page: input.page,
      sectionRef: input.sectionRef,
      prompt: input.prompt,
      replyTo,
      refs: input.refs ?? [],
      notify: input.notify ?? false,
      id: `prompt:${randomUUID()}`,
      threadId,
      status: "queued",
      createdAt: now.toISOString(),
      archived: false,
      read: false,
    };
    // top-level thread id = its own id? Use record.id for root to be intuitive.
    if (!replyTo) record.threadId = record.id;
    await this.enqueue(async () => {
      this.prompts.unshift(record);
      try {
        await this.write();
      } catch (err) {
        this.prompts = this.prompts.filter((p) => p.id !== record.id);
        throw err;
      }
    });
    return record;
  }

  async get(id: string): Promise<PromptRecord | null> {
    await this.load();
    return this.prompts.find((p) => p.id === id) ?? null;
  }

  async list(
    filter: {
      page?: string;
      status?: PromptStatus;
      archived?: boolean;
      threadId?: string;
      replyTo?: string;
      includeArchived?: boolean;
    } = {},
  ): Promise<PromptRecord[]> {
    await this.load();
    return this.prompts.filter((p) => {
      if (filter.page !== undefined && p.page !== filter.page) return false;
      if (filter.status !== undefined && p.status !== filter.status) return false;
      if (filter.threadId !== undefined && p.threadId !== filter.threadId) return false;
      if (filter.replyTo !== undefined && p.replyTo !== filter.replyTo) return false;
      const showArchived = filter.archived === true || filter.includeArchived === true;
      if (!showArchived && p.archived) return false;
      if (filter.archived === false && p.archived) return false;
      if (filter.archived === true && !p.archived) return false;
      return true;
    });
  }

  async thread(threadId: string): Promise<PromptRecord[]> {
    await this.load();
    return this.prompts
      .filter((p) => p.threadId === threadId)
      .slice()
      .sort((a, b) => (a.createdAt < b.createdAt ? -1 : a.createdAt > b.createdAt ? 1 : 0));
  }

  async threads(): Promise<{ threadId: string; prompts: PromptRecord[] }[]> {
    await this.load();
    const map = new Map<string, PromptRecord[]>();
    for (const p of this.prompts) {
      if (p.archived) continue;
      const arr = map.get(p.threadId) ?? [];
      arr.push(p);
      map.set(p.threadId, arr);
    }
    return [...map.entries()].map(([threadId, prompts]) => ({
      threadId,
      prompts: prompts.slice().sort((a, b) => (a.createdAt < b.createdAt ? -1 : 1)),
    }));
  }

  async update(
    id: string,
    patch: {
      status?: PromptStatus;
      answer?: string;
      error?: string;
      archived?: boolean;
      read?: boolean;
      refs?: string[];
      replyTo?: string | null;
    } = {},
  ): Promise<PromptRecord | null> {
    await this.load();
    const idx = this.prompts.findIndex((p) => p.id === id);
    if (idx === -1) return null;
    const rec = this.prompts[idx];
    if (patch.status !== undefined && PROMPT_STATUSES.includes(patch.status)) rec.status = patch.status;
    if (patch.answer !== undefined) rec.answer = patch.answer;
    if (patch.error !== undefined) rec.error = patch.error;
    if (patch.archived !== undefined) rec.archived = patch.archived === true;
    if (patch.read !== undefined) rec.read = patch.read === true;
    if (patch.refs !== undefined && Array.isArray(patch.refs))
      rec.refs = patch.refs.filter((x) => typeof x === "string");
    if (patch.replyTo !== undefined) {
      rec.replyTo = typeof patch.replyTo === "string" ? patch.replyTo : null;
    }
    await this.enqueue(async () => {
      await this.write();
    });
    return rec;
  }

  async markArchived(id: string, archived: boolean): Promise<PromptRecord | null> {
    return this.update(id, { archived });
  }

  async markRead(id: string, read = true): Promise<PromptRecord | null> {
    return this.update(id, { read });
  }

  async unreadCount(): Promise<number> {
    await this.load();
    return this.prompts.filter((p) => !p.read && !p.archived).length;
  }

  async count(filter: { status?: PromptStatus; archived?: boolean; read?: boolean } = {}): Promise<number> {
    await this.load();
    return this.prompts.filter((p) => {
      if (filter.status !== undefined && p.status !== filter.status) return false;
      if (filter.archived !== undefined && p.archived !== filter.archived) return false;
      if (filter.read !== undefined && p.read !== filter.read) return false;
      return true;
    }).length;
  }

  // Writes are serialized and each one uses a unique temp path (#12222):
  // concurrent adds shared one pid-only temp path, so most renames failed
  // with ENOENT while their records stayed in memory. A failed add is
  // rolled back inside its own serialized slot, so it rejects only its
  // caller and never reaches a later write.
  private enqueue(job: () => Promise<void>): Promise<void> {
    const run = this.writeChain.then(job);
    this.writeChain = run.catch(() => {});
    return run;
  }

  private async write(): Promise<void> {
    const file: PromptsFile = { version: 1, prompts: this.prompts };
    await mkdir(dirname(this.path), { recursive: true });
    const tmp = `${this.path}.tmp-${process.pid}-${++this.writeSeq}`;
    await writeFile(tmp, JSON.stringify(file, null, 2));
    await rename(tmp, this.path);
  }
}
