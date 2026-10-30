import { afterEach, beforeEach, describe, expect, it, type Mock, vi } from "vitest";
import { createNavigation, type Navigation } from "./navigation";

/**
 * #1462 regression tests: SPA navigation and partial swaps must apply
 * only the most recently issued request. Fetch is fully deferred —
 * each in-flight request resolves exactly when the test says so, in
 * the spirit of the laptop probe that reproduced the race.
 */

type DeferredResponse = {
  resolve: (html: string) => void;
  resolveStatus: (status: number) => void;
  reject: (err: unknown) => void;
};

function okResponse(html: string): Response {
  return {
    ok: true,
    status: 200,
    statusText: "OK",
    text: () => Promise.resolve(html),
  } as Response;
}

function errorResponse(status: number): Response {
  return {
    ok: false,
    status,
    statusText: "Error",
    text: () => Promise.resolve(""),
  } as Response;
}

/** A fetch stub whose responses the test settles by hand, keyed by URL. */
function deferredFetch(): {
  fetchFn: typeof fetch;
  pending: Map<string, DeferredResponse>;
} {
  const pending = new Map<string, DeferredResponse>();
  const fetchFn = ((url: string) =>
    new Promise<Response>((resolvePromise, rejectPromise) => {
      pending.set(url, {
        resolve: (html: string) => resolvePromise(okResponse(html)),
        resolveStatus: (status: number) => resolvePromise(errorResponse(status)),
        reject: (err: unknown) => rejectPromise(err),
      });
    })) as typeof fetch;
  return { fetchFn, pending };
}

let nav: Navigation;
let pending: Map<string, DeferredResponse>;
let onMainSwapped: Mock<() => void>;
let onPartialSwapped: Mock<(targetSelector: string) => void>;
let closeEditor: Mock<() => void>;

beforeEach(() => {
  document.body.innerHTML = `<main id="main"></main><div id="file-list-content"></div><div id="peers-content"></div>`;
  window.history.pushState(null, "", "/");
  const stub = deferredFetch();
  pending = stub.pending;
  vi.stubGlobal("fetch", stub.fetchFn);
  onMainSwapped = vi.fn();
  onPartialSwapped = vi.fn();
  closeEditor = vi.fn();
  (window as unknown as Record<string, unknown>).idApp = { collab: {}, closeEditor };
  nav = createNavigation({ onMainSwapped, onPartialSwapped });
  vi.spyOn(console, "error").mockImplementation(() => {});
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  delete (window as unknown as Record<string, unknown>).idApp;
});

describe("navigateTo supersession", () => {
  it("drops a slow earlier navigation that resolves after the newer one", async () => {
    const first = nav.navigateTo("/files");
    const second = nav.navigateTo("/edit/b.md");

    // Second click's response lands first: it wins.
    pending.get("/edit/b.md")!.resolve("<p>page B (second click)</p>");
    await second;
    expect(document.getElementById("main")!.innerHTML).toBe("<p>page B (second click)</p>");
    expect(window.location.pathname).toBe("/edit/b.md");
    expect(onMainSwapped).toHaveBeenCalledTimes(1);
    expect(closeEditor).toHaveBeenCalledTimes(1);

    // The first click's response lands last: it must change nothing.
    pending.get("/files")!.resolve("<p>files page (first click)</p>");
    await first;
    expect(document.getElementById("main")!.innerHTML).toBe("<p>page B (second click)</p>");
    expect(window.location.pathname).toBe("/edit/b.md");
    expect(onMainSwapped).toHaveBeenCalledTimes(1);
    expect(closeEditor).toHaveBeenCalledTimes(1);
  });

  it("drops an earlier navigation even when it resolves first, once superseded", async () => {
    const first = nav.navigateTo("/files");
    const second = nav.navigateTo("/edit/b.md");

    pending.get("/files")!.resolve("<p>files page (first click)</p>");
    await first;
    expect(document.getElementById("main")!.innerHTML).toBe("");
    expect(onMainSwapped).not.toHaveBeenCalled();
    expect(closeEditor).not.toHaveBeenCalled();

    pending.get("/edit/b.md")!.resolve("<p>page B (second click)</p>");
    await second;
    expect(document.getElementById("main")!.innerHTML).toBe("<p>page B (second click)</p>");
    expect(window.location.pathname).toBe("/edit/b.md");
    expect(onMainSwapped).toHaveBeenCalledTimes(1);
    expect(closeEditor).toHaveBeenCalledTimes(1);
  });

  it("a superseded navigation's failure does not trigger the full-page fallback", async () => {
    const first = nav.navigateTo("/files");
    const second = nav.navigateTo("/edit/b.md");

    pending.get("/files")!.reject(new Error("network down"));
    await first;
    expect(window.location.pathname).toBe("/");

    pending.get("/edit/b.md")!.resolve("<p>page B</p>");
    await second;
    expect(document.getElementById("main")!.innerHTML).toBe("<p>page B</p>");
    expect(window.location.pathname).toBe("/edit/b.md");
  });

  it("a failed CURRENT navigation still falls back to a full page load", async () => {
    const hrefBefore = window.location.href;
    const only = nav.navigateTo("/files");
    pending.get("/files")!.resolveStatus(500);
    await only;
    expect(window.location.href).not.toBe(hrefBefore);
    expect(window.location.href).toContain("/files");
    expect(document.getElementById("main")!.innerHTML).toBe("");
    expect(onMainSwapped).not.toHaveBeenCalled();
  });
});

describe("fetchPartial supersession (per target)", () => {
  it("keeps the newer list content when the stale search response resolves last", async () => {
    const stale = nav.fetchPartial("/api/files?search=ab", "#file-list-content");
    const fresh = nav.fetchPartial("/api/files?search=abc", "#file-list-content");

    pending.get("/api/files?search=abc")!.resolve("<li>results for abc</li>");
    await fresh;
    expect(document.querySelector("#file-list-content")!.innerHTML).toBe("<li>results for abc</li>");
    expect(onPartialSwapped).toHaveBeenCalledTimes(1);

    pending.get("/api/files?search=ab")!.resolve("<li>results for ab (stale)</li>");
    await stale;
    expect(document.querySelector("#file-list-content")!.innerHTML).toBe("<li>results for abc</li>");
    expect(onPartialSwapped).toHaveBeenCalledTimes(1);
  });

  it("does not supersede a different target", async () => {
    const files = nav.fetchPartial("/api/files?search=ab", "#file-list-content");
    const peers = nav.fetchPartial("/api/peers", "#peers-content");

    pending.get("/api/peers")!.resolve("<li>peer list</li>");
    await peers;
    pending.get("/api/files?search=ab")!.resolve("<li>results for ab</li>");
    await files;

    expect(document.querySelector("#peers-content")!.innerHTML).toBe("<li>peer list</li>");
    expect(document.querySelector("#file-list-content")!.innerHTML).toBe("<li>results for ab</li>");
    expect(onPartialSwapped).toHaveBeenCalledTimes(2);
  });

  it("a failed current partial only logs and writes nothing", async () => {
    const only = nav.fetchPartial("/api/files?search=ab", "#file-list-content");
    pending.get("/api/files?search=ab")!.resolveStatus(503);
    await only;
    expect(document.querySelector("#file-list-content")!.innerHTML).toBe("");
    expect(onPartialSwapped).not.toHaveBeenCalled();
    expect(console.error).toHaveBeenCalled();
  });
});

describe("fetchPartial x navigateTo (page generation, #1578)", () => {
  // The partial target lives INSIDE #main, as in the real pages: a
  // navigation swap replaces it with the new page's own element.
  function seedPage(listHtml: string): void {
    document.getElementById("main")!.innerHTML = `<div id="file-list-content">${listHtml}</div>`;
  }
  function listContent(): string {
    return document.querySelector("#main #file-list-content")!.innerHTML;
  }

  it("partial-then-navigate: a stale partial resolving after the swap writes nothing", async () => {
    seedPage("old page list");
    const stale = nav.fetchPartial("/api/files?search=ab", "#file-list-content");
    const navigation = nav.navigateTo("/files");

    pending.get("/files")!.resolve(`<div id="file-list-content">new page list</div>`);
    await navigation;
    expect(listContent()).toBe("new page list");

    pending.get("/api/files?search=ab")!.resolve("<li>stale results</li>");
    await stale;
    expect(listContent()).toBe("new page list");
    expect(onPartialSwapped).not.toHaveBeenCalled();
  });

  it("navigate-then-partial: a partial issued after the swap writes normally", async () => {
    seedPage("old page list");
    const navigation = nav.navigateTo("/files");
    pending.get("/files")!.resolve(`<div id="file-list-content">new page list</div>`);
    await navigation;

    const fresh = nav.fetchPartial("/api/files?search=abc", "#file-list-content");
    pending.get("/api/files?search=abc")!.resolve("<li>fresh results</li>");
    await fresh;
    expect(listContent()).toBe("<li>fresh results</li>");
    expect(onPartialSwapped).toHaveBeenCalledTimes(1);
  });

  it("a partial issued during an in-flight navigation is dropped when it resolves after the swap", async () => {
    seedPage("old page list");
    const navigation = nav.navigateTo("/files");
    const during = nav.fetchPartial("/api/files?search=ab", "#file-list-content");

    pending.get("/files")!.resolve(`<div id="file-list-content">new page list</div>`);
    await navigation;

    pending.get("/api/files?search=ab")!.resolve("<li>issued mid-navigation</li>");
    await during;
    expect(listContent()).toBe("new page list");
    expect(onPartialSwapped).not.toHaveBeenCalled();
  });

  it("reverse-order control: a partial resolving before the swap writes the old page, which the swap then replaces", async () => {
    seedPage("old page list");
    const partial = nav.fetchPartial("/api/files?search=ab", "#file-list-content");
    const navigation = nav.navigateTo("/files");

    pending.get("/api/files?search=ab")!.resolve("<li>old page refresh</li>");
    await partial;
    expect(listContent()).toBe("<li>old page refresh</li>");
    expect(onPartialSwapped).toHaveBeenCalledTimes(1);

    pending.get("/files")!.resolve(`<div id="file-list-content">new page list</div>`);
    await navigation;
    expect(listContent()).toBe("new page list");
  });
});
