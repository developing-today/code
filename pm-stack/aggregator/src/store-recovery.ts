import { randomUUID } from "node:crypto";
import { rename, unlink, writeFile } from "node:fs/promises";

export interface InvalidStoreRecord {
  index: number;
  reason: string;
}

export type StoreArchive = (
  file: string,
  raw: Uint8Array,
  invalidRecords?: readonly InvalidStoreRecord[],
) => Promise<string>;

function archivePath(file: string): string {
  const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
  return `${file}.corrupt-${timestamp}-${randomUUID()}`;
}

/** Preserve the exact source bytes before a parsed store is rewritten. */
export const archiveStoreBytes: StoreArchive = async (file, raw, invalidRecords = []) => {
  const archived = archivePath(file);
  await writeFile(archived, raw, { flag: "wx" });
  if (invalidRecords.length > 0) {
    await writeFile(
      `${archived}.diagnostics.json`,
      JSON.stringify({ source: file, archived, invalidRecords }, null, 2),
      { flag: "wx" },
    );
  }
  return archived;
};

/** Preserve an unreadable file/directory by moving it without reading it. */
export async function quarantineUnreadableStore(file: string): Promise<string> {
  const archived = archivePath(file);
  await rename(file, archived);
  return archived;
}

/** Archive a whole malformed store, then remove its active path. */
export async function quarantineWholeStore(file: string, raw: Uint8Array, archive: StoreArchive): Promise<string> {
  const archived = await archive(file, raw);
  await unlink(file);
  return archived;
}
