import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AUTOSAVE_DEBOUNCE_MS, AutoSaveManager, type SaveResult } from "./auto-save";

/** A saveFn whose promises resolve only when the test says so. */
function deferredSaveFn() {
  const resolvers: Array<(result: SaveResult) => void> = [];
  let calls = 0;
  let inFlight = 0;
  let maxInFlight = 0;
  const fn = () => {
    calls++;
    inFlight++;
    maxInFlight = Math.max(maxInFlight, inFlight);
    return new Promise<SaveResult>((resolve) => {
      resolvers.push((result) => {
        inFlight--;
        resolve(result);
      });
    });
  };
  return {
    fn,
    get calls() {
      return calls;
    },
    get maxInFlight() {
      return maxInFlight;
    },
    /** Resolve the oldest outstanding save with the given result. */
    resolveNext(result: SaveResult) {
      const resolve = resolvers.shift();
      if (!resolve) throw new Error("no save in flight to resolve");
      resolve(result);
    },
  };
}

/** Flush pending microtasks (save completions, follow-up scheduling). */
async function flush() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

describe("AutoSaveManager serialization (#1440)", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("runs exactly one coalesced follow-up when edits arrive during a save", async () => {
    const save = deferredSaveFn();
    const mgr = new AutoSaveManager(save.fn);

    mgr.onContentChange();
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DEBOUNCE_MS);
    expect(save.calls).toBe(1);
    expect(mgr.state).toBe("saving");

    // Two edits during the flight; their debounce fires mid-flight too.
    mgr.onContentChange();
    mgr.onContentChange();
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DEBOUNCE_MS);
    expect(save.calls).toBe(1); // still serialized — no concurrent save

    save.resolveNext({ ok: true });
    await flush();
    expect(save.calls).toBe(2); // exactly one follow-up
    expect(save.maxInFlight).toBe(1);

    save.resolveNext({ ok: true });
    await flush();
    expect(mgr.state).toBe("saved");
    await vi.advanceTimersByTimeAsync(10_000);
    expect(save.calls).toBe(2); // no further saves — the follow-up coalesced both edits
  });

  it("never marks newer edits saved when a stale save completes", async () => {
    const save = deferredSaveFn();
    const mgr = new AutoSaveManager(save.fn);

    mgr.onContentChange();
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DEBOUNCE_MS);
    expect(save.calls).toBe(1);

    // Edit arrives while save #1 is in flight; its debounce is still pending
    // when the stale save resolves (the #1440 scenario B shape).
    mgr.onContentChange();
    save.resolveNext({ ok: true });
    await flush();
    expect(mgr.state).not.toBe("saved");

    // The follow-up save covers the newer edit; only its completion may
    // move the indicator to "saved".
    expect(save.calls).toBe(2);
    save.resolveNext({ ok: true });
    await flush();
    expect(mgr.state).toBe("saved");
  });

  it("serializes the 429 retry: edits during backoff coalesce into the retry", async () => {
    const save = deferredSaveFn();
    const mgr = new AutoSaveManager(save.fn);

    mgr.onContentChange();
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DEBOUNCE_MS);
    expect(save.calls).toBe(1);

    save.resolveNext({ ok: false, retryAfterMs: 5000 });
    await flush();
    expect(mgr.state).toBe("rate-limited");

    // Edit during the backoff: its debounce must not start an early save,
    // and must not cancel the pending retry either.
    mgr.onContentChange();
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DEBOUNCE_MS);
    expect(save.calls).toBe(1);

    // The retry fires on schedule and captures the latest document.
    await vi.advanceTimersByTimeAsync(3000);
    expect(save.calls).toBe(2);
    expect(save.maxInFlight).toBe(1);

    save.resolveNext({ ok: true });
    await flush();
    expect(mgr.state).toBe("saved");
    await vi.advanceTimersByTimeAsync(10_000);
    expect(save.calls).toBe(2);
  });

  it("runs exactly one follow-up when an edit lands during a generically failing save (#1616)", async () => {
    const save = deferredSaveFn();
    const mgr = new AutoSaveManager(save.fn);

    mgr.onContentChange();
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DEBOUNCE_MS);
    expect(save.calls).toBe(1);
    expect(mgr.state).toBe("saving");

    // Edit during the flight; its debounce fires mid-flight too, so the
    // follow-up is both requested and visible in the edit generation.
    mgr.onContentChange();
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DEBOUNCE_MS);
    expect(save.calls).toBe(1); // still serialized

    // The save fails generically: the coalesced edit must not be stranded —
    // exactly one follow-up runs.
    save.resolveNext({ ok: false });
    await flush();
    expect(save.calls).toBe(2);
    expect(save.maxInFlight).toBe(1);

    // The follow-up also fails generically, with no new edits: it must stop
    // in a terminal unsaved/error state — no third call, no retry loop.
    save.resolveNext({ ok: false });
    await flush();
    expect(mgr.state).toBe("error");
    await vi.advanceTimersByTimeAsync(10_000);
    expect(save.calls).toBe(2);
    expect(mgr.state).toBe("unsaved");
  });

  it("saveNow saves immediately when idle and is a no-op while saving", async () => {
    const save = deferredSaveFn();
    const mgr = new AutoSaveManager(save.fn);

    mgr.onContentChange();
    mgr.saveNow(); // immediate — no debounce wait
    await flush();
    expect(save.calls).toBe(1);
    expect(mgr.state).toBe("saving");

    mgr.saveNow(); // already saving — unchanged no-op behaviour
    await flush();
    expect(save.calls).toBe(1);

    save.resolveNext({ ok: true });
    await flush();
    expect(mgr.state).toBe("saved");
    await vi.advanceTimersByTimeAsync(10_000);
    expect(save.calls).toBe(1);
  });
});
