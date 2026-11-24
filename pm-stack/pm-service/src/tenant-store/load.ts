/**
 * Loads the tenant store's SQL migrations from a directory of
 * `NNNN_name.sql` files (the DDL of record lives in those files, per
 * the pm-stack migration convention — see pm-stack/external-comms).
 * Files are returned in version order; a filename that does not parse
 * or a duplicated version is an error, never a silent skip.
 */
import type { Migration } from "./migrate.ts";

export async function loadMigrations(dir: string): Promise<Migration[]> {
  const files: string[] = [];
  for await (const file of new Bun.Glob("*.sql").scan({ cwd: dir })) files.push(file);
  files.sort();
  const migrations: Migration[] = [];
  const seen = new Set<number>();
  for (const file of files) {
    const match = /^(\d+)_([a-z0-9_]+)\.sql$/.exec(file);
    if (!match) throw new Error(`migration filename must be NNNN_name.sql: ${file}`);
    const version = Number(match[1]);
    if (seen.has(version)) throw new Error(`duplicate migration version ${version}: ${file}`);
    seen.add(version);
    migrations.push({ version, name: match[2] as string, sql: await Bun.file(`${dir}/${file}`).text() });
  }
  return migrations.sort((a, b) => a.version - b.version);
}

/** The migrations shipped with this package, from ./migrations. */
export function tenantStoreMigrations(): Promise<Migration[]> {
  return loadMigrations(new URL("./migrations", import.meta.url).pathname);
}
