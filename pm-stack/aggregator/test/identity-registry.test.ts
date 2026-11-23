import { afterEach, describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { EventsStore } from "../src/events.ts";
import {
  IDENTITY_MAPPING_CACHE_TTL_MS,
  IdentityRegistryStore,
  identityMappingAuditEvent,
  validateIdentityMapping,
} from "../src/identity-registry.ts";

const dirs: string[] = [];
async function store(ttl = IDENTITY_MAPPING_CACHE_TTL_MS) {
  const dir = await mkdtemp(join(tmpdir(), "identity-registry-"));
  dirs.push(dir);
  return { dir, store: new IdentityRegistryStore(join(dir, "registry.json"), ttl) };
}

afterEach(async () => {
  await Promise.all(dirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
});

describe("aggregator identity registry (#9604)", () => {
  test("validates roles and supports sub and agent identities", () => {
    expect(validateIdentityMapping({ identity: "access|user-1", role: "orchestrator" })).toEqual({
      ok: true,
      identity: "access|user-1",
      role: "orchestrator",
    });
    expect(validateIdentityMapping({ identity: "agent:repo-worker-code", role: "agent" }).ok).toBe(true);
    expect(validateIdentityMapping({ identity: "missing-role", role: "root" }).ok).toBe(false);
  });

  test("persists upsert and removal with monotonic registry revisions", async () => {
    const { dir, store: registry } = await store();
    expect(await registry.roleFor("unknown")).toBe("read-only");
    const first = await registry.set("access|user-1", "owner", "drew");
    expect(first.registry).toMatchObject({ version: 1, revision: 1 });
    const second = await registry.set("access|user-1", "agent", "drew");
    expect(second.previousRole).toBe("owner");
    expect(second.registry.revision).toBe(2);
    expect(await registry.roleFor("access|user-1")).toBe("agent");
    const removed = await registry.remove("access|user-1");
    expect(removed.previousRole).toBe("agent");
    expect(removed.registry.revision).toBe(3);
    expect(JSON.parse(await readFile(join(dir, "registry.json"), "utf8"))).toEqual(removed.registry);
  });

  test("records mapping changes in the versioned audit stream", async () => {
    const { dir } = await store();
    const events = new EventsStore(join(dir, "events.json"));
    const event = await events.record(
      identityMappingAuditEvent({
        identity: "agent:worker",
        previousRole: null,
        role: "agent",
        revision: 1,
        actor: "access-owner-sub",
      }),
    );
    expect(event).toMatchObject({
      version: 2,
      actor: "access-owner-sub",
      kind: "identity-mapping-changed",
      ref: "identity:agent:worker",
      attribution: { confidence: "authenticated", asserted: true },
      summary: "Identity role unset -> agent; registry revision 1",
    });
  });

  test("caps cache TTL at five minutes and refreshes expired mappings", async () => {
    expect(() => new IdentityRegistryStore("unused", IDENTITY_MAPPING_CACHE_TTL_MS + 1)).toThrow(/5 minutes/);
    const dir = await mkdtemp(join(tmpdir(), "identity-registry-cache-"));
    dirs.push(dir);
    const path = join(dir, "registry.json");
    const reader = new IdentityRegistryStore(path, 1);
    const writer = new IdentityRegistryStore(path, 1);
    await writer.set("agent:worker", "agent", "drew");
    expect(await reader.roleFor("agent:worker")).toBe("agent");
    await writer.set("agent:worker", "read-only", "drew");
    await new Promise((resolve) => setTimeout(resolve, 5));
    expect(await reader.roleFor("agent:worker")).toBe("read-only");
  });
});
