// A Store that is a file on this machine: private to the user, and
// written whole to a new file that is then moved into place, so a crash
// never leaves half of it.
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { Store } from "./store.ts";

export function fileStore(path: string): Store {
  let writing: Promise<void> = Promise.resolve();
  return {
    read: async () => {
      try {
        return await readFile(path, "utf8");
      } catch {
        return null; // no file yet
      }
    },
    // One write at a time, in the order asked.
    write: (text) => {
      const next = writing.then(async () => {
        await mkdir(dirname(path), { recursive: true, mode: 0o700 });
        const tmp = `${path}.${process.pid}.tmp`;
        await writeFile(tmp, text, { mode: 0o600 });
        await rename(tmp, path);
      });
      writing = next.catch(() => {});
      return next;
    },
  };
}
