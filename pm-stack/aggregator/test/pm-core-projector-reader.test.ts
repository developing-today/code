import { expect, test } from "bun:test";
import { mkdtemp, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { openDb, migrate, upsertItem, type ItemRow } from "../../projector/src/store.ts";

const ref = "developing-today/code#8570";
const item = (): Omit<ItemRow, "fetched_at"> => ({
  ref,
  repo: "developing-today/code",
  number: 8570,
  kind: "issue",
  url: "https://github.com/developing-today/code/issues/8570",
  api_url: "https://api.github.com/repos/developing-today/code/issues/8570",
  title: "Reader served from the pm-core projector store",
  state: "open",
  state_reason: null,
  draft: 0,
  merged: 0,
  author: "test",
  labels: "[]",
  assignees: "[]",
  created_at: "2026-11-27T00:00:00Z",
  updated_at: "2026-11-27T00:00:00Z",
  closed_at: null,
  merged_at: null,
  source_updated_at: "2026-11-27T00:00:00Z",
});

test("live pm-core reads projector items from its in-process store without an HTTP projector", async () => {
  const dir = mkdtemp(join(tmpdir(), "pm-core-projector-reader-"));
  const databaseUrl = `file:${join(dir, "projector.db")}`;
  const db = openDb({ DATABASE_URL: databaseUrl });
  try {
    await migrate(db);
    await upsertItem(db, item());

    const script = `
      const { source } = await import("./src/server-wiring.ts");
      const items = await source.items([${JSON.stringify(ref)}]);
      console.log(JSON.stringify(items.get(${JSON.stringify(ref)}) ?? null));
    `;
    const child = Bun.spawnSync([process.execPath, "-e", script], {
      cwd: join(import.meta.dir, ".."),
      env: {
        ...process.env,
        PM_AGG_SOURCE: "live",
        PM_AGG_FEED_FILE: join(dir, "missing-feed.json"),
        PM_PROJECTOR_URL: "off",
        PM_CORE_PROJECTOR_DATABASE_URL: databaseUrl,
      },
      stdout: "pipe",
      stderr: "pipe",
    });

    expect(child.exitCode).toBe(0);
    expect(child.stderr.toString()).toBe("");
    expect(JSON.parse(child.stdout.toString())).toMatchObject({
      ref,
      title: "Reader served from the pm-core projector store",
      resolution: "projector",
    });
  } finally {
    db.close();
    rmSync(dir, { recursive: true, force: true });
  }
});
