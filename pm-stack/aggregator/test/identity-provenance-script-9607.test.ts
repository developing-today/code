import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { describe, expect, test } from "bun:test";

const script = resolve(import.meta.dir, "../scripts/migrate-identity-provenance.ts");

describe("identity provenance migration script (#9607)", () => {
  test("dry-run is read-only and apply is idempotent", async () => {
    const directory = await mkdtemp(join(tmpdir(), "identity-provenance-"));
    const commentsPath = join(directory, "comments.json");
    const registryPath = join(directory, "registry.json");
    const comments = {
      comments: [{
        id: "comment-1",
        objectRef: "gh:developing-today/code#9607",
        kind: "note",
        author: { kind: "drew", id: "drew", label: "Drew  " },
        body: "Keep this text unchanged.",
        createdAt: "2026-11-01T00:00:00.000Z",
      }],
    };
    const registry = [{
      id: "session-1",
      source: "opencode-sandbox",
      label: "Exact title  ",
      agentId: "opencode:main",
      agentLabel: "claimed label",
      parentId: null,
      status: "idle",
      taskRefs: [],
    }];

    try {
      await writeFile(commentsPath, JSON.stringify(comments));
      await writeFile(registryPath, JSON.stringify(registry));
      const commentsBefore = await readFile(commentsPath, "utf8");
      const registryBefore = await readFile(registryPath, "utf8");
      const args = ["--comments", commentsPath, "--registry", registryPath];

      const dryRun = Bun.spawnSync([process.execPath, script, ...args], {
        stdout: "pipe",
        stderr: "pipe",
      });
      expect(dryRun.exitCode).toBe(0);
      expect(JSON.parse(dryRun.stdout.toString())).toMatchObject({
        mode: "dry-run",
        comments: { provenanceAdded: 1 },
        registry: { provenanceAdded: 1 },
      });
      expect(await readFile(commentsPath, "utf8")).toBe(commentsBefore);
      expect(await readFile(registryPath, "utf8")).toBe(registryBefore);

      const apply = Bun.spawnSync([process.execPath, script, ...args, "--apply"], {
        stdout: "pipe",
        stderr: "pipe",
      });
      expect(apply.exitCode).toBe(0);
      const migratedComments = JSON.parse(await readFile(commentsPath, "utf8"));
      const migratedRegistry = JSON.parse(await readFile(registryPath, "utf8"));
      expect(migratedComments.comments[0]).toMatchObject({
        provenance: "asserted-legacy",
        author: comments.comments[0]!.author,
        body: comments.comments[0]!.body,
      });
      expect(migratedRegistry[0]).toMatchObject({
        provenance: "asserted-legacy",
        label: registry[0]!.label,
        agentId: registry[0]!.agentId,
        agentLabel: registry[0]!.agentLabel,
      });

      const secondApply = Bun.spawnSync([process.execPath, script, ...args, "--apply"], {
        stdout: "pipe",
        stderr: "pipe",
      });
      expect(secondApply.exitCode).toBe(0);
      expect(JSON.parse(secondApply.stdout.toString())).toMatchObject({
        comments: { provenanceAdded: 0 },
        registry: { provenanceAdded: 0 },
      });
      expect(JSON.parse(await readFile(commentsPath, "utf8"))).toEqual(migratedComments);
      expect(JSON.parse(await readFile(registryPath, "utf8"))).toEqual(migratedRegistry);
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });
});
