// #9607 identity migration operator tool.
// Command contract: --comments PATH --registry PATH [--name-links PATH] [--apply].
// Dry-run is the default and performs reads only. --name-links must be an
// owner-authored JSON file in the {version:1,links:[...]} shape documented
// by OwnerConfirmedNameLink in src/identity-migration.ts. No mapping is
// inferred from labels or ids.
import { readFile, rename, writeFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { resolve } from "node:path";
import { applyOwnerConfirmedNameLinks, type OwnerConfirmedNameLink } from "../src/identity-migration.ts";
import { migrateRegistryProvenance } from "../src/registry.ts";
import type { DashComment, SessionRecord } from "../src/model.ts";

interface Args {
  comments: string;
  registry: string;
  nameLinks?: string;
  apply: boolean;
}

function parseArgs(argv: string[]): Args {
  const values = new Map<string, string>();
  let apply = false;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]!;
    if (arg === "--apply") {
      if (apply) throw new Error("--apply may be supplied only once");
      apply = true;
      continue;
    }
    if (arg !== "--comments" && arg !== "--registry" && arg !== "--name-links") {
      throw new Error(`unknown argument: ${arg}`);
    }
    if (values.has(arg)) throw new Error(`${arg} may be supplied only once`);
    const value = argv[++i];
    if (!value || value.startsWith("--")) throw new Error(`${arg} requires a path`);
    values.set(arg, value);
  }
  const comments = values.get("--comments");
  const registry = values.get("--registry");
  if (!comments || !registry)
    throw new Error(
      "usage: migrate-identity-provenance.ts --comments PATH --registry PATH [--name-links PATH] [--apply]",
    );
  const nameLinks = values.get("--name-links");
  const outputPaths = [comments, registry].map((path) => resolve(path));
  if (outputPaths[0] === outputPaths[1]) throw new Error("comments and registry paths must be different");
  if (nameLinks && outputPaths.includes(resolve(nameLinks)))
    throw new Error("name-links input must not be an output path");
  return { comments, registry, nameLinks, apply };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseLinks(value: unknown): OwnerConfirmedNameLink[] {
  if (!isRecord(value) || value.version !== 1 || !Array.isArray(value.links)) {
    throw new Error("name-links file must have {version:1,links:[...]}");
  }
  const allowedKinds = new Set(["drew", "muse", "agent"]);
  const allowedSources = new Set(["opencode-sandbox", "opencode-laptop", "jules", "claude-code", "cron"]);
  return value.links.map((item, index) => {
    if (!isRecord(item) || !isRecord(item.author) || !isRecord(item.identity)) {
      throw new Error(`name-links entry ${index} must have author and identity objects`);
    }
    const { kind, id, label } = item.author;
    const { source, id: identityId } = item.identity;
    if (
      typeof kind !== "string" ||
      !allowedKinds.has(kind) ||
      typeof id !== "string" ||
      id.length === 0 ||
      typeof label !== "string" ||
      label.length === 0 ||
      typeof source !== "string" ||
      !allowedSources.has(source) ||
      typeof identityId !== "string" ||
      identityId.length === 0
    ) {
      throw new Error(`name-links entry ${index} has unsupported or empty fields`);
    }
    return {
      author: { kind: kind as "drew" | "muse" | "agent", id, label },
      identity: { source: source as SessionRecord["source"], id: identityId },
    };
  });
}

async function readJson(path: string): Promise<unknown> {
  return JSON.parse(await readFile(path, "utf8"));
}

async function atomicWrite(path: string, value: unknown): Promise<void> {
  const tmp = `${path}.tmp-${process.pid}-${randomUUID()}`;
  await writeFile(tmp, JSON.stringify(value, null, 2));
  await rename(tmp, path);
}

async function main(): Promise<void> {
  // Parse the complete command line before any file operation.
  const args = parseArgs(process.argv.slice(2));
  const commentsEnvelope = await readJson(args.comments);
  const registryRaw = await readJson(args.registry);
  const links = args.nameLinks ? parseLinks(await readJson(args.nameLinks)) : [];

  if (!isRecord(commentsEnvelope) || !Array.isArray(commentsEnvelope.comments)) {
    throw new Error("comments file must contain an object with a comments array");
  }
  if (!Array.isArray(registryRaw)) throw new Error("registry file must contain a JSON array");
  const comments = commentsEnvelope.comments.map((value, index) => {
    if (!isRecord(value)) throw new Error(`comment row ${index} is not an object`);
    if (
      !isRecord(value.author) ||
      typeof value.author.kind !== "string" ||
      !new Set(["drew", "muse", "agent"]).has(value.author.kind) ||
      typeof value.author.id !== "string" ||
      typeof value.author.label !== "string"
    ) {
      throw new Error(`comment row ${index} has no supported author identity`);
    }
    if (value.provenance !== undefined && value.provenance !== "verified" && value.provenance !== "asserted-legacy") {
      throw new Error(`comment row ${index} has unsupported provenance`);
    }
    return {
      ...value,
      ...(value.provenance === undefined ? { provenance: "asserted-legacy" } : {}),
    } as unknown as DashComment;
  });
  const records = registryRaw.map((value, index) => {
    if (!isRecord(value)) throw new Error(`registry row ${index} is not an object`);
    if (value.provenance !== undefined && value.provenance !== "verified" && value.provenance !== "asserted-legacy") {
      throw new Error(`registry row ${index} has unsupported provenance`);
    }
    return value as unknown as SessionRecord;
  });
  const registryMigration = migrateRegistryProvenance(records);
  const nameLinkMigration = applyOwnerConfirmedNameLinks(comments, registryMigration.records, links);
  const migratedComments = comments.filter(
    (comment, index) => commentsEnvelope.comments[index]?.provenance === undefined,
  ).length;

  console.log(
    JSON.stringify(
      {
        mode: args.apply ? "apply" : "dry-run",
        comments: { total: comments.length, provenanceAdded: migratedComments },
        registry: {
          total: records.length,
          provenanceAdded: registryMigration.changed ? records.filter((r) => r.provenance === undefined).length : 0,
        },
        ownerConfirmedLinks: { supplied: links.length, applied: nameLinkMigration.linked },
      },
      null,
      2,
    ),
  );
  if (!args.apply) return;

  await atomicWrite(args.comments, { ...commentsEnvelope, comments: nameLinkMigration.comments });
  await atomicWrite(args.registry, registryMigration.records);
}

main().catch((error: unknown) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
});
