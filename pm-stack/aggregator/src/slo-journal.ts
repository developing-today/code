import { appendFile, mkdir, readFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { SloSample } from "./slo.ts";

/** One append-only line in the SLO journal (#9988): one facet's sample for
 * one minute. The journal never rewrites or compacts a line. */
export interface SloJournalLine extends SloSample {
  facet: string;
}

export class SloJournal {
  private writes: Promise<void> = Promise.resolve();

  constructor(private readonly file: string) {}

  append(lines: readonly SloJournalLine[]): Promise<void> {
    if (lines.length === 0) return this.writes;
    const payload = lines.map((line) => `${JSON.stringify(line)}\n`).join("");
    // A failed append must not poison the chain: each write starts from
    // the previous one settled, so one bad write does not fail every
    // later append and read.
    this.writes = this.writes.catch(() => undefined).then(async () => {
      await mkdir(dirname(this.file), { recursive: true });
      await appendFile(this.file, payload, "utf8");
    });
    return this.writes;
  }

  async readAll(): Promise<SloJournalLine[]> {
    await this.writes.catch(() => undefined);
    const raw = await readFile(this.file, "utf8").catch((error: NodeJS.ErrnoException) => {
      if (error.code === "ENOENT") return "";
      throw error;
    });
    return raw
      .split("\n")
      .filter(Boolean)
      .flatMap((line) => {
        // A line cut short by a crash mid-append is skipped, not fatal:
        // one torn line must not blank every budget on /health.
        try {
          return [JSON.parse(line) as SloJournalLine];
        } catch {
          return [];
        }
      });
  }
}

export function groupSamplesByFacet(lines: readonly SloJournalLine[]): Map<string, SloSample[]> {
  const byFacet = new Map<string, SloSample[]>();
  for (const { facet, atMs, bad, value } of lines) {
    const list = byFacet.get(facet);
    const sample: SloSample = { atMs, bad, value };
    if (list) list.push(sample);
    else byFacet.set(facet, [sample]);
  }
  return byFacet;
}
