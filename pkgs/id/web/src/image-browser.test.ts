/**
 * Tests for the image browser dialog (#1381).
 *
 * The overlay is appended to document.body and captures the EditorView in
 * its Insert closures, so it must not survive the view: teardown closes it
 * via closeImageBrowser(), opening for a different view self-closes the
 * stale one, and Insert/double-click dismiss in a finally even when the
 * insert dispatch throws.
 */

import { EditorState } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { richSchema } from "./editor";
import { closeImageBrowser, openImageBrowser } from "./image-browser";

const IMAGES = [{ hash: "abc123", name: "cat.png", url: "/api/images/abc123", size: 42 }];

function stubFetchImages(): void {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => ({
      ok: true,
      status: 200,
      json: async () => IMAGES,
    })),
  );
}

/** Create an EditorView with a one-paragraph doc in a container div. */
function createView(): { view: EditorView; container: HTMLElement } {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const doc = richSchema.node("doc", null, [richSchema.node("paragraph", null, [richSchema.text("hello")])]);
  const state = EditorState.create({ schema: richSchema, doc });
  const view = new EditorView(container, { state });
  return { view, container };
}

/** Open the browser and wait for the image cards to render. */
async function openAndLoad(view: EditorView): Promise<void> {
  openImageBrowser(view, richSchema);
  await vi.waitFor(() => {
    expect(document.querySelector(".pm-image-browser-card")).not.toBeNull();
  });
}

function overlay(): Element | null {
  return document.querySelector(".pm-image-browser-overlay");
}

function selectFirstCard(): void {
  const card = document.querySelector<HTMLElement>(".pm-image-browser-card");
  expect(card).not.toBeNull();
  card?.click();
}

const views: EditorView[] = [];
const containers: HTMLElement[] = [];

function trackedView(): { view: EditorView; container: HTMLElement } {
  const made = createView();
  views.push(made.view);
  containers.push(made.container);
  return made;
}

beforeEach(() => {
  stubFetchImages();
});

afterEach(() => {
  closeImageBrowser();
  for (const view of views) {
    view.destroy();
  }
  views.length = 0;
  for (const container of containers) {
    container.remove();
  }
  containers.length = 0;
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

// ── closeImageBrowser ──────────────────────────────────────────────

describe("closeImageBrowser", () => {
  it("can be called without error when no browser is open", () => {
    expect(() => closeImageBrowser()).not.toThrow();
  });

  it("can be called multiple times safely", () => {
    expect(() => {
      closeImageBrowser();
      closeImageBrowser();
      closeImageBrowser();
    }).not.toThrow();
  });

  it("removes the overlay after the browser has opened (editor teardown)", async () => {
    const { view } = trackedView();
    await openAndLoad(view);
    expect(overlay()).not.toBeNull();

    // Simulates closeEditor(): the teardown hook closes the browser for
    // the view that is about to be destroyed.
    closeImageBrowser();
    expect(overlay()).toBeNull();
  });
});

// ── stale browser from a previous view ─────────────────────────────

describe("openImageBrowser for a different view", () => {
  it("self-closes the previous view's browser instead of leaving it stuck", async () => {
    const first = trackedView();
    await openAndLoad(first.view);
    expect(overlay()).not.toBeNull();

    // The first editor is torn down (navigation / collab re-init) and a
    // new editor opens the browser — without any explicit close call.
    const second = trackedView();
    openImageBrowser(second.view, richSchema);

    // Exactly one overlay, owned by the second view.
    expect(document.querySelectorAll(".pm-image-browser-overlay")).toHaveLength(1);
    await vi.waitFor(() => {
      expect(document.querySelector(".pm-image-browser-card")).not.toBeNull();
    });

    // Insert now dispatches on the live second view, not the stale one.
    selectFirstCard();
    document.querySelector<HTMLButtonElement>(".pm-image-browser-insert")?.click();
    expect(overlay()).toBeNull();
    let imageInSecond = false;
    second.view.state.doc.descendants((node) => {
      if (node.type.name === "image") imageInSecond = true;
    });
    expect(imageInSecond).toBe(true);
  });

  it("does not open a second browser for the same view", async () => {
    const { view } = trackedView();
    await openAndLoad(view);
    openImageBrowser(view, richSchema);
    expect(document.querySelectorAll(".pm-image-browser-overlay")).toHaveLength(1);
  });
});

// ── Insert dismissal ───────────────────────────────────────────────

describe("Insert dismissal", () => {
  it("inserts the image at the selection and closes on the normal path", async () => {
    const { view } = trackedView();
    await openAndLoad(view);

    selectFirstCard();
    const insertBtn = document.querySelector<HTMLButtonElement>(".pm-image-browser-insert");
    expect(insertBtn?.disabled).toBe(false);
    insertBtn?.click();

    expect(overlay()).toBeNull();
    let found = false;
    view.state.doc.descendants((node) => {
      if (node.type.name === "image" && node.attrs.src === "/api/images/abc123") found = true;
    });
    expect(found).toBe(true);
  });

  it("closes even when the insert dispatch throws (Insert button)", async () => {
    const { view } = trackedView();
    await openAndLoad(view);
    // Simulate dispatching on a destroyed view.
    view.dispatch = () => {
      throw new Error("dispatch on destroyed view");
    };

    selectFirstCard();
    document.querySelector<HTMLButtonElement>(".pm-image-browser-insert")?.click();
    expect(overlay()).toBeNull();
  });

  it("closes even when the insert dispatch throws (double-click)", async () => {
    const { view } = trackedView();
    await openAndLoad(view);
    view.dispatch = () => {
      throw new Error("dispatch on destroyed view");
    };

    const card = document.querySelector<HTMLElement>(".pm-image-browser-card");
    card?.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    expect(overlay()).toBeNull();
  });
});
