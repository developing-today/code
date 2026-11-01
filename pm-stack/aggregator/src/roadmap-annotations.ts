// Append-only, attributed annotations for roadmap rows. Row subjects are
// feed/source identifiers; aliases let a reshuffled Grist row continue to
// resolve to the original stable subject without rewriting annotation history.
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import { randomUUID } from "node:crypto";
import { isObjectRef, validateAuthor } from "./comments.ts";
import type { DashAuthor } from "./model.ts";

export type RoadmapAnnotationKind = "decision" | "outcome" | "context";

export interface RoadmapAnnotation {
  id: string;
  /** Canonical stable ref after resolving any source-row aliases. */
  stableRef: string;
  kind: RoadmapAnnotationKind;
  /** User-supplied calendar date; separate from the immutable write time. */
  date: string;
  author: DashAuthor;
  body: string;
  createdAt: string;
}

export interface NewRoadmapAnnotation {
  objectRef: string;
  kind: RoadmapAnnotationKind;
  date: string;
  author: DashAuthor;
  body: string;
}

interface RoadmapAnnotationsFile {
  version: 1;
  annotations: RoadmapAnnotation[];
  aliases: Record<string, string>;
}

export function validRoadmapDate(value: unknown): value is string {
  if (typeof value !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(value)) return false;
  const date = new Date(`${value}T00:00:00.000Z`);
  return !Number.isNaN(date.getTime()) && date.toISOString().slice(0, 10) === value;
}

export function validateNewRoadmapAnnotation(
  input: unknown,
): { ok: true; value: NewRoadmapAnnotation } | { ok: false; error: string } {
  if (typeof input !== "object" || input === null) return { ok: false, error: "body must be a JSON object" };
  const value = input as Record<string, unknown>;
  if (!isObjectRef(value.objectRef)) return { ok: false, error: "objectRef must be a non-empty ref string" };
  if (value.kind !== "decision" && value.kind !== "outcome" && value.kind !== "context") {
    return { ok: false, error: "kind must be decision | outcome | context" };
  }
  if (!validRoadmapDate(value.date)) return { ok: false, error: "date must be a valid YYYY-MM-DD calendar date" };
  const author = validateAuthor(value.author);
  if (!author) return { ok: false, error: "author must be {kind: drew|muse|agent, id, label}" };
  if (typeof value.body !== "string" || value.body.trim() === "") {
    return { ok: false, error: "body must be non-empty text" };
  }
  return {
    ok: true,
    value: {
      objectRef: value.objectRef,
      kind: value.kind,
      date: value.date,
      author,
      body: value.body.trim().slice(0, 20_000),
    },
  };
}

function isAnnotation(value: unknown): value is RoadmapAnnotation {
  if (typeof value !== "object" || value === null) return false;
  const row = value as Partial<RoadmapAnnotation>;
  return (
    typeof row.id === "string" &&
    isObjectRef(row.stableRef) &&
    (row.kind === "decision" || row.kind === "outcome" || row.kind === "context") &&
    validRoadmapDate(row.date) &&
    validateAuthor(row.author) !== null &&
    typeof row.body === "string" &&
    typeof row.createdAt === "string"
  );
}

export class RoadmapAnnotationsStore {
  private annotations: RoadmapAnnotation[] = [];
  private aliases: Record<string, string> = {};
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
      if ((err as { code?: unknown })?.code === "ENOENT") return;
      throw err;
    }
    try {
      const parsed: unknown = JSON.parse(raw);
      if (typeof parsed !== "object" || parsed === null) throw new Error("not an object");
      const file = parsed as Partial<RoadmapAnnotationsFile>;
      if (file.version !== 1 || !Array.isArray(file.annotations) || typeof file.aliases !== "object" || file.aliases === null) {
        throw new Error("unexpected roadmap annotation store shape");
      }
      this.annotations = file.annotations.filter(isAnnotation);
      this.aliases = Object.fromEntries(
        Object.entries(file.aliases).filter(([alias, stable]) => isObjectRef(alias) && isObjectRef(stable)),
      );
    } catch (err) {
      const quarantined = `${this.file}.corrupt-${new Date().toISOString().replace(/[:.]/g, "-")}`;
      await rename(this.file, quarantined).catch(() => {});
      console.error(
        `[roadmap-annotations] invalid store ${this.file}; quarantined original bytes to ${quarantined}: ${err instanceof Error ? err.message : String(err)}`,
      );
      this.annotations = [];
      this.aliases = {};
    }
  }

  private persist(): Promise<void> {
    const run = this.writeChain.then(async () => {
      const data: RoadmapAnnotationsFile = { version: 1, annotations: this.annotations, aliases: this.aliases };
      await mkdir(dirname(this.file), { recursive: true });
      const tmp = `${this.file}.tmp-${process.pid}-${++this.writeSeq}`;
      await writeFile(tmp, JSON.stringify(data, null, 2));
      await rename(tmp, this.file);
    });
    this.writeChain = run.catch(() => {});
    return run;
  }

  private resolveRef(ref: string): string {
    const seen = new Set<string>();
    let current = ref;
    while (this.aliases[current]) {
      if (seen.has(current)) throw new Error("roadmap annotation alias cycle");
      seen.add(current);
      current = this.aliases[current];
    }
    return current;
  }

  async all(): Promise<RoadmapAnnotation[]> {
    await this.load();
    return [...this.annotations].sort((a, b) => a.date.localeCompare(b.date) || a.createdAt.localeCompare(b.createdAt));
  }

  async forObject(objectRef: string): Promise<RoadmapAnnotation[]> {
    await this.load();
    const stableRef = this.resolveRef(objectRef);
    return (await this.all()).filter((annotation) => annotation.stableRef === stableRef);
  }

  async add(input: NewRoadmapAnnotation): Promise<RoadmapAnnotation> {
    await this.load();
    const annotation: RoadmapAnnotation = {
      id: `ra-${randomUUID()}`,
      stableRef: this.resolveRef(input.objectRef),
      kind: input.kind,
      date: input.date,
      author: input.author,
      body: input.body,
      createdAt: new Date().toISOString(),
    };
    this.annotations.push(annotation);
    await this.persist();
    return annotation;
  }

  /** Map a current source ref to an existing canonical ref; history stays immutable. */
  async addAlias(aliasRef: string, stableRef: string): Promise<void> {
    await this.load();
    const canonical = this.resolveRef(stableRef);
    if (aliasRef === canonical) throw new Error("aliasRef and stableRef already identify the same item");
    if (this.annotations.some((annotation) => annotation.stableRef === aliasRef)) {
      throw new Error("aliasRef already owns annotation history; choose it as stableRef instead");
    }
    if (this.aliases[aliasRef] && this.resolveRef(aliasRef) !== canonical) {
      throw new Error("aliasRef already points to a different stable ref");
    }
    this.aliases[aliasRef] = canonical;
    await this.persist();
  }

  async aliasEntries(): Promise<Record<string, string>> {
    await this.load();
    return { ...this.aliases };
  }
}
