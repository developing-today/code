import { appendFile, mkdir, readFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { NotificationHistoryEntry } from "./notifications.ts";

/** Durable append-only history used by the notification transition policy. */
export interface NotificationHistoryStore {
  read(): Promise<NotificationHistoryEntry[]>;
  append(entry: NotificationHistoryEntry): Promise<void>;
}

export class JsonlNotificationHistory implements NotificationHistoryStore {
  private writes: Promise<void> = Promise.resolve();

  constructor(private readonly file: string) {}

  async read(): Promise<NotificationHistoryEntry[]> {
    const raw = await readFile(this.file, "utf8").catch((error: NodeJS.ErrnoException) => {
      if (error.code === "ENOENT") return "";
      throw error;
    });
    return raw
      .split("\n")
      .filter(Boolean)
      .map((line) => JSON.parse(line) as NotificationHistoryEntry);
  }

  append(entry: NotificationHistoryEntry): Promise<void> {
    this.writes = this.writes.then(async () => {
      await mkdir(dirname(this.file), { recursive: true });
      await appendFile(this.file, `${JSON.stringify(entry)}\n`, "utf8");
    });
    return this.writes;
  }
}
