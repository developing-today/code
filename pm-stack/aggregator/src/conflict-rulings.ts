// Per-class trust rulings for the source-conflict inbox (#1282, slice 2).
//
// Slice 1 (conflicts.ts) projects disagreements and never resolves. This
// module persists the class-level ruling that suppresses a class: for the
// Project-vs-derived class only, "trust project" or "trust derived" is
// recorded as data (this JSON file), never as code. Renders that
// understand the class apply the trusted value and stop emitting the
// class's entries until the ruling is revoked.
//
// Scope: project-vs-derived only. Kaneo-vs-GitHub is a separately scoped
// slice of #1282 and is rejected here, so a ruling for an unknown class
// can never be smuggled in as data.
//
// Storage follows the views store discipline (views.ts): one JSON file
// (PM_AGG_CONFLICT_RULINGS_FILE), atomic writes (unique temp + rename),
// serialized through a per-store promise chain; a missing file is the
// only silent empty start — an unparseable or wrongly-shaped file is
// quarantined to <file>.corrupt-<timestamp>, never silently overwritten.
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import { isIsoDate } from "./validate.ts";

/** The only class slice 2 rules on. */
export const RULING_CLASS_ID = "project-vs-derived" as const;
export type RulingClassId = typeof RULING_CLASS_ID;

/** Which source the class trusts while the ruling stands. */
export type TrustedSource = "project" | "derived";

export interface ConflictRuling {
  classId: RulingClassId;
  trusted: TrustedSource;
  createdAt: string;
  createdBy: string;
  reason: string | null;
}

export interface ConflictRulingsFile {
  version: 1;
  rulings: ConflictRuling[];
}

export function validateRulingInput(body: unknown):
  | { ok: true; value: { classId: RulingClassId; trusted: TrustedSource; reason: string | null } }
  | { ok: false; error: string } {
  if (typeof body !== "object" || body === null) return { ok: false, error: "body must be an object" };
  const b = body as Record<string, unknown>;
  if (b.classId !== RULING_CLASS_ID)
    return { ok: false, error: `classId must be "${RULING_CLASS_ID}" (slice 2 rules only this class)` };
  if (b.trusted !== "project" && b.trusted !== "derived")
    return { ok: false, error: 'trusted must be "project" or "derived"' };
  let reason: string | null = null;
  if (b.reason !== undefined && b.reason !== null) {
    if (typeof b.reason !== "string" || b.reason.trim() === "" || b.reason.length > 500)
      return { ok: false, error: "reason must be a non-empty string (max 500 chars) when present" };
    reason = b.reason.trim();
  }
  return { ok: true, value: { classId: RULING_CLASS_ID, trusted: b.trusted, reason } };
}

function isRuling(value: unknown): value is ConflictRuling {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const r = value as Record<string, unknown>;
  return (
    r.classId === RULING_CLASS_ID &&
    (r.trusted === "project" || r.trusted === "derived") &&
    isIsoDate(r.createdAt) &&
    typeof r.createdBy === "string" &&
    (r.createdBy as string).length > 0 &&
    (r.reason === null || (typeof r.reason === "string" && (r.reason as string).length <= 500))
  );
}

export class ConflictRulingsStore {
  private rulings: ConflictRuling[] = [];
  private loadPromise: Promise<void> | null = null;
  private writeChain: Promise<void> = Promise.resolve();
  private writeSeq = 0;

  constructor(private readonly file: string) {}

  async load(): Promise<void> {
    if (!this.loadPromise) this.loadPromise = this.doLoad();
    return this.loadPromise;
  }

  private async doLoad(): Promise<void> {
    let raw: string;
    try {
      raw = await readFile(this.file, "utf8");
    } catch (err) {
      if ((err as { code?: unknown })?.code === "ENOENT") {
        this.rulings = [];
        return;
      }
      this.rulings = [];
      await this.quarantine(`could not be read (${err instanceof Error ? err.message : String(err)})`);
      return;
    }
    try {
      const parsed: unknown = JSON.parse(raw);
      if (typeof parsed !== "object" || parsed === null || !Array.isArray((parsed as ConflictRulingsFile).rulings)) {
        throw new Error("store file does not have the expected { rulings: [...] } shape");
      }
      const rows = (parsed as { rulings: unknown[] }).rulings;
      const bad = rows.findIndex((r) => !isRuling(r));
      if (bad !== -1) throw new Error(`ruling row ${bad} does not have the expected ConflictRuling shape`);
      this.rulings = rows.filter(isRuling);
    } catch (err) {
      this.rulings = [];
      await this.quarantine(`is unparseable (${err instanceof Error ? err.message : String(err)})`);
    }
  }

  private async quarantine(why: string): Promise<void> {
    const quarantined = `${this.file}.corrupt-${new Date().toISOString().replace(/[:.]/g, "-")}`;
    try {
      await rename(this.file, quarantined);
      console.error(
        `[conflict-rulings] WARNING: rulings store ${this.file} ${why}; quarantined the original bytes to ${quarantined} and started empty.`,
      );
    } catch (renameErr) {
      console.error(
        `[conflict-rulings] WARNING: rulings store ${this.file} ${why} and could not be quarantined (${renameErr instanceof Error ? renameErr.message : String(renameErr)}); starting empty — the next persist will overwrite it, so copy it aside now.`,
      );
    }
  }

  private persist(): Promise<void> {
    const run = this.writeChain.then(async () => {
      const data: ConflictRulingsFile = { version: 1, rulings: this.rulings };
      await mkdir(dirname(this.file), { recursive: true });
      const tmp = `${this.file}.tmp-${process.pid}-${++this.writeSeq}`;
      await writeFile(tmp, JSON.stringify(data, null, 2));
      await rename(tmp, this.file);
    });
    this.writeChain = run.catch(() => {});
    return run;
  }

  async all(): Promise<ConflictRuling[]> {
    await this.load();
    return [...this.rulings].sort((a, b) => a.createdAt.localeCompare(b.createdAt));
  }

  async getForClass(classId: string): Promise<ConflictRuling | null> {
    await this.load();
    return this.rulings.find((r) => r.classId === classId) ?? null;
  }

  /** Record (or replace) the ruling for the class. One ruling per class. */
  async set(
    input: { classId: RulingClassId; trusted: TrustedSource; reason: string | null },
    createdBy: string,
  ): Promise<ConflictRuling> {
    await this.load();
    const now = new Date().toISOString();
    const ruling: ConflictRuling = {
      classId: input.classId,
      trusted: input.trusted,
      createdAt: now,
      createdBy,
      reason: input.reason,
    };
    this.rulings = [...this.rulings.filter((r) => r.classId !== input.classId), ruling];
    await this.persist();
    return ruling;
  }

  /** Revoke the ruling for the class; false when none stood. */
  async revoke(classId: string): Promise<boolean> {
    await this.load();
    const before = this.rulings.length;
    this.rulings = this.rulings.filter((r) => r.classId !== classId);
    if (this.rulings.length === before) return false;
    await this.persist();
    return true;
  }
}
