// PM-stack prompt submissions (#10032, design #9539 S1). A submission is
// a prompt typed into a prompt widget, stored as one record in a JSON file
// in the aggregator's data directory (PM_AGG_PROMPTS_FILE). Writes are
// atomic (temp file + rename), like the comments store. This slice stores
// submissions only; enqueueing them to the fleet queue is a later slice.
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
}

export interface PromptRecord extends NewPrompt {
  id: string;
  status: PromptStatus;
  createdAt: string;
  answer?: string;
  error?: string;
}

export interface PromptsFile {
  version: 1;
  prompts: PromptRecord[];
}

const MAX_PROMPT_CHARS = 8000;
const PAGE_ID = /^[a-z][a-z0-9-]{0,63}$/;

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
  return {
    ok: true,
    value: {
      scope: b.scope as PromptScope,
      target: b.target as PromptTarget,
      mode: b.mode as PromptMode,
      page: b.page,
      sectionRef: (b.sectionRef as string | null | undefined) ?? null,
      prompt: b.prompt,
    },
  };
}

export class PromptsStore {
  private prompts: PromptRecord[] = [];
  private loadPromise: Promise<void> | null = null;
  private writeChain: Promise<void> = Promise.resolve();
  private writeSeq = 0;

  constructor(private readonly path: string) {}

  // Memoized as one promise (#12222, same pattern as comments.ts #1509):
  // a boolean flag let concurrent first callers each run the read.
  async load(): Promise<void> {
    if (!this.loadPromise) this.loadPromise = this.doLoad();
    return this.loadPromise;
  }

  private async doLoad(): Promise<void> {
    try {
      const parsed = JSON.parse(await readFile(this.path, "utf8")) as PromptsFile;
      this.prompts = Array.isArray(parsed.prompts) ? parsed.prompts : [];
    } catch (err) {
      if ((err as NodeJS.ErrnoException).code !== "ENOENT") throw err;
    }
  }

  async add(input: NewPrompt, now = new Date()): Promise<PromptRecord> {
    await this.load();
    const record: PromptRecord = {
      ...input,
      id: `prompt:${randomUUID()}`,
      status: "queued",
      createdAt: now.toISOString(),
    };
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

  async list(filter: { page?: string; status?: PromptStatus } = {}): Promise<PromptRecord[]> {
    await this.load();
    return this.prompts.filter(
      (p) =>
        (filter.page === undefined || p.page === filter.page) &&
        (filter.status === undefined || p.status === filter.status),
    );
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
