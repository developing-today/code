// AA last-good store (code#3332, moved to Supabase in #6815).
//
// The collector keeps the last good AA section so a transient Supabase
// failure degrades to the previous payload instead of blanking the
// explorer. In file mode it is a JSON file beside the feed; in supabase
// mode its canonical copy is public.agg_store_documents keyed
// "aa-last-good" (the file is not read or written).
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import { AA_LAST_GOOD_DOCUMENT } from "./store-backend.ts";

/** Minimal remote document contract; implemented over Supabase in
 *  remote-stores.ts and faked in tests. */
export interface DocumentRemote {
  load(store: string): Promise<{ version: number; document: unknown } | null>;
  save(store: string, version: number, document: unknown): Promise<void>;
}

export class AaLastGoodStore {
  constructor(
    private readonly file: string,
    private readonly remote?: DocumentRemote,
  ) {}

  /** The stored document, or null when absent/unparseable. Validation of
   *  the payload stays with the caller (the collector's isAaSection). */
  async read(): Promise<unknown | null> {
    if (this.remote) {
      const stored = await this.remote.load(AA_LAST_GOOD_DOCUMENT);
      return stored ? stored.document : null;
    }
    try {
      return JSON.parse(await readFile(this.file, "utf8")) as unknown;
    } catch {
      return null;
    }
  }

  async write(document: unknown): Promise<void> {
    if (this.remote) {
      await this.remote.save(AA_LAST_GOOD_DOCUMENT, 1, document);
      return;
    }
    await mkdir(dirname(this.file), { recursive: true });
    const tmp = `${this.file}.tmp`;
    await writeFile(tmp, JSON.stringify(document));
    await rename(tmp, this.file);
  }
}
