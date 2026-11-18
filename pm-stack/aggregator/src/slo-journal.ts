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
    this.writes = this.writes.then(async () => {
      await mkdir(dirname(this.file), { recursive: true });
      await appendFile(this.file, payload, "utf8");
    });
    return this.writes;
  }

  async readAll(): Promise<SloJournalLine[]> {
    await this.writes;
    const raw = await readFile(this.file, "utf8").catch((error: NodeJS.ErrnoException) => {
      if (error.code === "ENOENT") return "";
      throw error;
    });
    return raw
      .split("\n")
      .filter(Boolean)
      .map((line) => JSON.parse(line) as SloJournalLine);
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
