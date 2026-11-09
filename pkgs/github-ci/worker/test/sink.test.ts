import { afterEach, describe, expect, it, vi } from "vitest";
import { advanceSnapshot, appendAgentEvent, type SinkEnv } from "../src/sink";

const brokerEnv: SinkEnv = {
  TURSO_QUEUE_URL: "https://fleet-queue.example.invalid",
  TURSO_QUEUE_TOKEN: "queue-token",
  EVENTS_BROKER_URL: "https://queue-broker.security.cab/",
  EVENTS_BROKER_TOKEN: "broker-token",
  EVENTS_BROKER_ACCESS_CLIENT_ID: "access-id",
  EVENTS_BROKER_ACCESS_CLIENT_SECRET: "access-secret",
};

afterEach(() => vi.unstubAllGlobals());

describe("appendAgentEvent broker route (#6925)", () => {
  it("POSTs /events-append with X-Agent-Token and CF-Access headers", async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    vi.stubGlobal("fetch", async (url: string, init: RequestInit) => {
      calls.push({ url, init });
      return new Response(JSON.stringify({ ok: true, row: { id: 7, kind: "push" } }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    });
    await appendAgentEvent(brokerEnv, "push", { repo: "developing-today/code" });
    expect(calls).toHaveLength(1);
    expect(calls[0].url).toBe("https://queue-broker.security.cab/events-append");
    const headers = calls[0].init.headers as Record<string, string>;
    expect(headers["X-Agent-Token"]).toBe("broker-token");
    expect(headers["CF-Access-Client-Id"]).toBe("access-id");
    expect(headers["CF-Access-Client-Secret"]).toBe("access-secret");
    expect(headers["Authorization"]).toBeUndefined();
    expect(JSON.parse(String(calls[0].init.body))).toEqual({
      worker_id: "github-webhook",
      kind: "push",
      detail: { repo: "developing-today/code" },
    });
  });

  it("omits auth headers when the broker secrets are unset", async () => {
    const calls: { init: RequestInit }[] = [];
    vi.stubGlobal("fetch", async (_url: string, init: RequestInit) => {
      calls.push({ init });
      return new Response(JSON.stringify({ ok: true }), { status: 200 });
    });
    const env: SinkEnv = { ...brokerEnv, EVENTS_BROKER_TOKEN: undefined, EVENTS_BROKER_ACCESS_CLIENT_ID: undefined, EVENTS_BROKER_ACCESS_CLIENT_SECRET: undefined };
    await appendAgentEvent(env, "issues", {});
    const headers = calls[0].init.headers as Record<string, string>;
    expect(headers["X-Agent-Token"]).toBeUndefined();
    expect(headers["CF-Access-Client-Id"]).toBeUndefined();
    expect(headers["CF-Access-Client-Secret"]).toBeUndefined();
  });

  it("throws when the broker answers not-ok", async () => {
    vi.stubGlobal("fetch", async () => new Response(JSON.stringify({ ok: false, error: "nope" }), { status: 200 }));
    await expect(appendAgentEvent(brokerEnv, "push", {})).rejects.toThrow("broker returned not-ok");
  });

  it("throws on a non-2xx broker response", async () => {
    vi.stubGlobal("fetch", async () => new Response("unauthorized", { status: 401 }));
    await expect(appendAgentEvent(brokerEnv, "push", {})).rejects.toThrow("HTTP 401");
  });
});

// #6945: advanceSnapshot must not move head_sha when default_branch is
// unknown (NULL), and a missing row must not seed head from a side branch.
const queueEnv: SinkEnv = {
  TURSO_QUEUE_URL: "https://fleet-queue.example.invalid",
  TURSO_QUEUE_TOKEN: "q-token",
};
const NOW = "2026-11-08T02:30:00Z";
const SHA = "bbb222bbb222bbb222bbb222bbb222bbb222bbbb";
const ZERO = "0000000000000000000000000000000000000000";

interface CapturedInsert { sql: string; args: { type: string; value?: string }[] }

function fakeTurso(state: { exists: boolean; defBranch: string | null }, captured: CapturedInsert[]) {
  return async (_url: string, init: RequestInit) => {
    const body = JSON.parse(String(init.body)) as { requests: { stmt: { sql: string; args: CapturedInsert["args"] } }[] };
    const stmt = body.requests[0].stmt;
    let rows: unknown[] = [];
    if (stmt.sql.includes("SELECT repo FROM")) {
      rows = state.exists ? [[{ type: "text", value: "developing-today/code" }]] : [];
    } else if (stmt.sql.includes("SELECT default_branch")) {
      rows = state.exists
        ? [[state.defBranch ? { type: "text", value: state.defBranch } : { type: "null" }]]
        : [];
    } else if (stmt.sql.startsWith("INSERT")) {
      captured.push({ sql: stmt.sql, args: stmt.args });
    }
    return new Response(
      JSON.stringify({ results: [{ type: "ok", response: { result: { rows } } }, { type: "ok" }] }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    );
  };
}

describe("advanceSnapshot default_branch guard (#6945)", () => {
  it("existing NULL default_branch + side-branch push writes nothing", async () => {
    const captured: CapturedInsert[] = [];
    vi.stubGlobal("fetch", fakeTurso({ exists: true, defBranch: null }, captured));
    await expect(
      advanceSnapshot(queueEnv, "developing-today/code", SHA, "refs/heads/feature-x", NOW, "main"),
    ).resolves.toBe(false);
    expect(captured).toHaveLength(0);
  });

  it("existing NULL default_branch + no payload default: even a main push does not advance", async () => {
    const captured: CapturedInsert[] = [];
    vi.stubGlobal("fetch", fakeTurso({ exists: true, defBranch: null }, captured));
    await expect(
      advanceSnapshot(queueEnv, "developing-today/code", SHA, "refs/heads/main", NOW),
    ).resolves.toBe(false);
    expect(captured).toHaveLength(0);
  });

  it("existing NULL default_branch + payload default main + main push advances head", async () => {
    const captured: CapturedInsert[] = [];
    vi.stubGlobal("fetch", fakeTurso({ exists: true, defBranch: null }, captured));
    await expect(
      advanceSnapshot(queueEnv, "developing-today/code", SHA, "refs/heads/main", NOW, "main"),
    ).resolves.toBe(true);
    expect(captured).toHaveLength(1);
    expect(captured[0].sql).toContain("head_sha");
    expect(captured[0].args.map((a) => a.value)).toContain(SHA);
  });

  it("missing row + side-branch push seeds a minimal row without head_sha", async () => {
    const captured: CapturedInsert[] = [];
    vi.stubGlobal("fetch", fakeTurso({ exists: false, defBranch: null }, captured));
    await expect(
      advanceSnapshot(queueEnv, "developing-today/code", SHA, "refs/heads/feature-x", NOW, "main"),
    ).resolves.toBe(true);
    expect(captured).toHaveLength(1);
    expect(captured[0].sql).not.toContain("head_sha");
    expect(captured[0].args.map((a) => a.value)).not.toContain(SHA);
  });

  it("missing row + default-branch push (payload default) seeds head_sha", async () => {
    const captured: CapturedInsert[] = [];
    vi.stubGlobal("fetch", fakeTurso({ exists: false, defBranch: null }, captured));
    await expect(
      advanceSnapshot(queueEnv, "developing-today/code", SHA, "refs/heads/main", NOW, "main"),
    ).resolves.toBe(true);
    expect(captured).toHaveLength(1);
    expect(captured[0].sql).toContain("head_sha");
    expect(captured[0].args.map((a) => a.value)).toContain(SHA);
  });

  it("controls: stored default main — side push ignored, default push advances, zero-after deletion unchanged", async () => {
    const side: CapturedInsert[] = [];
    vi.stubGlobal("fetch", fakeTurso({ exists: true, defBranch: "main" }, side));
    await expect(
      advanceSnapshot(queueEnv, "developing-today/code", SHA, "refs/heads/feature-x", NOW, "main"),
    ).resolves.toBe(false);
    expect(side).toHaveLength(0);

    const def: CapturedInsert[] = [];
    vi.stubGlobal("fetch", fakeTurso({ exists: true, defBranch: "main" }, def));
    await expect(
      advanceSnapshot(queueEnv, "developing-today/code", SHA, "refs/heads/main", NOW, "main"),
    ).resolves.toBe(true);
    expect(def).toHaveLength(1);
    expect(def[0].args.map((a) => a.value)).toContain(SHA);

    const del: CapturedInsert[] = [];
    vi.stubGlobal("fetch", fakeTurso({ exists: true, defBranch: "main" }, del));
    await expect(
      advanceSnapshot(queueEnv, "developing-today/code", ZERO, "refs/heads/main", NOW, "main"),
    ).resolves.toBe(false);
    expect(del).toHaveLength(0);
  });
});

describe("advanceSnapshot property: head_sha invariant (#6945, testing doctrine)", () => {
  // Oracle: the resolved default branch is storedDef (existing row only)
  // else the payload default else unknown. head_sha may be written ONLY
  // for a non-deletion push whose ref is exactly refs/heads/<resolved>,
  // and the written value must be the push's after SHA. The full factor
  // grid (row existence x stored default x payload default x push ref x
  // deletion SHA = 192 combinations) is enumerated exhaustively and
  // every cell is checked against this oracle via the captured Turso
  // pipeline INSERTs — no sampling, so no cell can be missed.
  it("property: head_sha is written only for a non-deletion push matching the resolved default", async () => {
    const existsOpts = [true, false] as const;
    const defOpts = [null, "main", "dev"] as const;
    const payloadOpts = [undefined, null, "main", "dev"] as const;
    const refOpts = ["refs/heads/main", "refs/heads/dev", "refs/heads/feature-x", "refs/tags/v1"] as const;
    const afterOpts = [SHA, ZERO] as const;
    let cells = 0;
    for (const exists of existsOpts)
    for (const storedDef of defOpts)
    for (const payloadDef of payloadOpts)
    for (const ref of refOpts)
    for (const after of afterOpts) {
      cells++;
      const resolved = (exists ? storedDef : null) || payloadDef || null;
      const expectHead = after !== ZERO && !!resolved && ref === `refs/heads/${resolved}`;
      const captured: CapturedInsert[] = [];
      vi.stubGlobal("fetch", fakeTurso({ exists, defBranch: storedDef }, captured));
      const wrote = await advanceSnapshot(queueEnv, "developing-today/code", after, ref, NOW, payloadDef);
      const ctx = JSON.stringify({ exists, storedDef, payloadDef, ref, after });
      if (exists && !expectHead) {
        expect(wrote, ctx).toBe(false);
        expect(captured, ctx).toHaveLength(0);
      } else {
        expect(wrote, ctx).toBe(true);
        expect(captured, ctx).toHaveLength(1);
        if (expectHead) {
          expect(captured[0].sql, ctx).toContain("head_sha");
          expect(captured[0].args.map((a) => a.value), ctx).toContain(after);
        } else {
          expect(captured[0].sql, ctx).not.toContain("head_sha");
          expect(captured[0].args.map((a) => a.value), ctx).not.toContain(after);
        }
      }
      vi.unstubAllGlobals();
    }
    expect(cells).toBe(
      existsOpts.length * defOpts.length * payloadOpts.length * refOpts.length * afterOpts.length,
    );
  });
});
