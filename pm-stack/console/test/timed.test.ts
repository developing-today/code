// A deadline that leaves nothing running behind it.
//
// Oracles: the function's own contract (the work's answer or failure,
// a TimeoutError from the signal at the deadline), the platform's
// `setTimeout`/`clearTimeout` as counted by wrapping them, and, for
// the rule that nothing the Worker runs uses `AbortSignal.timeout`, the
// sources themselves, followed from the Worker's entry by their imports.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { timed, unread } from "../src/timed.ts";

/** Counts the timers started and cleared while `work` runs. */
async function timers<T>(work: () => Promise<T>): Promise<{ result: T; started: number; cleared: number }> {
  const real = { set: globalThis.setTimeout, clear: globalThis.clearTimeout };
  const live = new Set<unknown>();
  let started = 0;
  let cleared = 0;
  globalThis.setTimeout = ((fn: (...args: unknown[]) => void, ms?: number, ...args: unknown[]) => {
    started++;
    const id = real.set(() => {
      live.delete(id);
      fn(...args);
    }, ms);
    live.add(id);
    return id;
  }) as typeof setTimeout;
  globalThis.clearTimeout = ((id?: Parameters<typeof clearTimeout>[0]) => {
    if (live.delete(id)) cleared++;
    real.clear(id);
  }) as typeof clearTimeout;
  try {
    return { result: await work(), started, cleared };
  } finally {
    globalThis.setTimeout = real.set;
    globalThis.clearTimeout = real.clear;
    for (const id of live) real.clear(id as Parameters<typeof clearTimeout>[0]);
  }
}

describe("a deadline that leaves nothing running", () => {
  test("work that finishes in time gives its answer, and the timer is cleared at once", async () => {
    const got = await timers(() => timed(60_000, async (signal) => [signal.aborted, "done"]));
    expect(got).toEqual({ result: [false, "done"], started: 1, cleared: 1 });
  });

  test("work that fails in time fails as itself, and the timer is cleared all the same", async () => {
    const got = await timers(() =>
      timed(60_000, async () => {
        throw new Error("refused");
      }).catch((error: Error) => error.message),
    );
    expect(got).toEqual({ result: "refused", started: 1, cleared: 1 });
  });

  test("work that takes too long is stopped by its signal, with the reason a timeout has", async () => {
    const got = await timers(() =>
      timed(
        20,
        (signal) =>
          new Promise<string>((_resolve, reject) => {
            signal.addEventListener("abort", () => reject(signal.reason));
          }),
      ).catch((error: DOMException) => `${error.name}: ${error.message}`),
    );
    // The timer ran out: there was nothing left to clear.
    expect(got).toEqual({ result: "TimeoutError: The operation timed out.", started: 1, cleared: 0 });
  });

  test("a real fetch that never answers is stopped at the deadline", async () => {
    const server = Bun.serve({ port: 0, hostname: "127.0.0.1", fetch: () => new Promise<Response>(() => {}) });
    try {
      const began = Date.now();
      const said = await timed(150, (signal) => fetch(`http://127.0.0.1:${server.port}/`, { signal })).then(
        () => "answered",
        (error: Error) => error.name,
      );
      expect([said, Date.now() - began < 5_000]).toEqual(["TimeoutError", true]);
    } finally {
      await server.stop(true);
    }
  });

  test("a body that never ends is stopped too, when it is read inside the work", async () => {
    const server = Bun.serve({
      port: 0,
      hostname: "127.0.0.1",
      fetch: () =>
        new Response(
          new ReadableStream({
            start(controller) {
              controller.enqueue(new TextEncoder().encode('{"begun":'));
            },
          }),
        ),
    });
    try {
      const said = await timed(150, async (signal) => {
        const res = await fetch(`http://127.0.0.1:${server.port}/`, { signal });
        return res.json();
      }).then(
        () => "read",
        (error: Error) => error.name,
      );
      expect(said).toBe("TimeoutError");
    } finally {
      await server.stop(true);
    }
  });
});

describe("a body nobody will read", () => {
  test("is let go of, so that nothing is kept open for it", async () => {
    let cancelled = 0;
    const res = new Response(
      new ReadableStream({
        cancel() {
          cancelled++;
        },
      }),
    );
    unread(res);
    await Promise.resolve();
    expect(cancelled).toBe(1);
  });

  test("and letting go asks nothing of what has no body, has had it read, or cannot let go", async () => {
    // None of these throws, now or later (a rejection nobody handles fails the run).
    unread(undefined);
    unread(null);
    unread({ status: 201 });
    unread({ status: 304, body: null });
    const read = new Response("read already");
    expect(await read.text()).toBe("read already");
    unread(read);
    unread({
      body: {
        cancel: () => {
          throw new Error("cannot");
        },
      },
    });
    unread({ body: { cancel: async () => Promise.reject(new Error("would not")) } });
    await new Promise((resolve) => setTimeout(resolve, 5));
  });
});

describe("what the Worker runs", () => {
  /** Every source the Worker's entry reaches by its imports. */
  function reached(entry: string): string[] {
    const seen = new Set<string>();
    const walk = (file: string) => {
      if (seen.has(file)) return;
      seen.add(file);
      const text = readFileSync(file, "utf8");
      const imports = /(?:^|\n)\s*(?:import|export)\b[^;]*?\bfrom\s+"(\.[^"]+)"|(?:^|\n)\s*import\s+"(\.[^"]+)"/g;
      for (const found of text.matchAll(imports)) walk(resolve(dirname(file), found[1] ?? found[2]));
    };
    walk(entry);
    return [...seen].sort();
  }

  test("nothing it runs leaves a timeout's timer running: a timer left running keeps its object in memory", () => {
    const root = join(import.meta.dir, "..");
    const files = reached(join(root, "edge", "src", "worker.ts"));
    const names = files.map((file) => file.slice(root.length + 1));
    // The walk did reach what the object is made of, and what it shares with the laptop.
    for (const name of [
      "edge/src/edge.ts",
      "edge/src/direct.ts",
      "edge/src/entry.ts",
      "src/auth.ts",
      "src/ops.ts",
      "src/push.ts",
      "src/work.ts",
      "src/timed.ts",
    ]) {
      expect(names).toContain(name);
    }
    const using = files
      .filter((file) => file !== join(root, "src", "timed.ts"))
      .filter((file) => /AbortSignal\s*\.\s*timeout\s*\(/.test(readFileSync(file, "utf8")))
      .map((file) => file.slice(root.length + 1));
    expect(using).toEqual([]);
  });
});
