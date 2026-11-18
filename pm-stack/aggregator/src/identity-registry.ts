import { randomUUID } from "node:crypto";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { NewAuditEvent } from "./events.ts";

export const IDENTITY_ROLES = ["owner", "orchestrator", "agent", "read-only"] as const;
export type IdentityRole = (typeof IDENTITY_ROLES)[number];

export interface IdentityMapping {
  identity: string;
  role: IdentityRole;
  updatedAt: string;
  updatedBy: string;
}

export interface IdentityRegistryFile {
  version: 1;
  revision: number;
  mappings: IdentityMapping[];
}

export const IDENTITY_MAPPING_CACHE_TTL_MS = 5 * 60 * 1000;

export function identityMappingAuditEvent(input: {
  identity: string;
  previousRole: IdentityRole | null;
  role: IdentityRole | null;
  revision: number;
  actor: string;
}): NewAuditEvent {
  return {
    actor: input.actor,
    kind: "identity-mapping-changed",
    ref: `identity:${input.identity}`,
    summary: `Identity role ${input.previousRole ?? "unset"} -> ${input.role ?? "unset"}; registry revision ${input.revision}`,
    attribution: { confidence: "authenticated", asserted: true },
    source: { identity: "pm-aggregator", link: "#9604" },
  };
}

export function validateIdentityMapping(
  input: unknown,
): { ok: true; identity: string; role: IdentityRole } | { ok: false; error: string } {
  if (typeof input !== "object" || input === null || Array.isArray(input)) {
    return { ok: false, error: "mapping must be an object" };
  }
  const value = input as Record<string, unknown>;
  if (
    typeof value.identity !== "string" ||
    !/^(?:[^\s:][^\s]{0,199}|agent:[^\s]{1,190})$/.test(value.identity.trim())
  ) {
    return { ok: false, error: "identity must be a non-empty identity or agent:<id> (max 200 chars)" };
  }
  if (!IDENTITY_ROLES.includes(value.role as IdentityRole)) return { ok: false, error: "invalid role" };
  return { ok: true, identity: value.identity.trim(), role: value.role as IdentityRole };
}

export class IdentityRegistryStore {
  private state: IdentityRegistryFile | null = null;
  private cachedAt = 0;
  private chain: Promise<void> = Promise.resolve();

  constructor(private readonly file: string, private readonly cacheTtlMs = IDENTITY_MAPPING_CACHE_TTL_MS) {
    if (!Number.isFinite(cacheTtlMs) || cacheTtlMs < 0 || cacheTtlMs > IDENTITY_MAPPING_CACHE_TTL_MS) {
      throw new Error("identity mapping cache TTL must be between 0 and 5 minutes");
    }
  }

  async list(): Promise<IdentityRegistryFile> {
    if (!this.state || Date.now() - this.cachedAt >= this.cacheTtlMs) {
      this.state = await this.load();
      this.cachedAt = Date.now();
    }
    return structuredClone(this.state);
  }

  async roleFor(identity: string): Promise<IdentityRole> {
    return (await this.list()).mappings.find((row) => row.identity === identity)?.role ?? "read-only";
  }

  async set(
    identity: string,
    role: IdentityRole,
    actor: string,
  ): Promise<{ registry: IdentityRegistryFile; previousRole: IdentityRole | null }> {
    return this.serialize(async () => {
      const current = await this.load();
      const previousRole = current.mappings.find((row) => row.identity === identity)?.role ?? null;
      const mappings = current.mappings.filter((row) => row.identity !== identity);
      mappings.push({ identity, role, updatedAt: new Date().toISOString(), updatedBy: actor });
      const registry = {
        version: 1 as const,
        revision: current.revision + 1,
        mappings: mappings.sort((a, b) => a.identity.localeCompare(b.identity)),
      };
      await this.persist(registry);
      this.state = registry;
      this.cachedAt = Date.now();
      return { registry: structuredClone(registry), previousRole };
    });
  }

  async remove(identity: string): Promise<{ registry: IdentityRegistryFile; previousRole: IdentityRole | null }> {
    return this.serialize(async () => {
      const current = await this.load();
      const previousRole = current.mappings.find((row) => row.identity === identity)?.role ?? null;
      if (previousRole === null) return { registry: current, previousRole };
      const registry = {
        version: 1 as const,
        revision: current.revision + 1,
        mappings: current.mappings.filter((row) => row.identity !== identity),
      };
      await this.persist(registry);
      this.state = registry;
      this.cachedAt = Date.now();
      return { registry: structuredClone(registry), previousRole };
    });
  }

  private async serialize<T>(fn: () => Promise<T>): Promise<T> {
    let result!: T;
    const run = this.chain.then(async () => {
      result = await fn();
    });
    this.chain = run.then(
      () => {},
      () => {},
    );
    await run;
    return result;
  }

  private async load(): Promise<IdentityRegistryFile> {
    let raw: string;
    try {
      raw = await readFile(this.file, "utf8");
    }
    catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return { version: 1, revision: 0, mappings: [] };
      throw error;
    }
    const parsed = JSON.parse(raw) as IdentityRegistryFile;
    if (parsed.version !== 1 || !Number.isSafeInteger(parsed.revision) || parsed.revision < 0 || !Array.isArray(parsed.mappings)) {
      throw new Error(`invalid identity registry file: ${this.file}`);
    }
    for (const row of parsed.mappings) {
      const valid = validateIdentityMapping(row);
      if (!valid.ok || typeof row.updatedAt !== "string" || typeof row.updatedBy !== "string") {
        throw new Error(`invalid identity mapping in ${this.file}`);
      }
    }
    return parsed;
  }

  private async persist(value: IdentityRegistryFile): Promise<void> {
    await mkdir(dirname(this.file), { recursive: true });
    const tmp = `${this.file}.${randomUUID()}.tmp`;
    await writeFile(tmp, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
    await rename(tmp, this.file);
  }
}
