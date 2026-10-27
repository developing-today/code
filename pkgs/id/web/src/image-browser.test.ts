/**
 * Tests for the Image Browser dialog.
 * Tests ARIA attributes, keyboard navigation, selection, and close functionality.
 */

import { EditorState } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { richSchema } from "./editor";
import { openImageBrowser } from "./image-browser";

describe("openImageBrowser", () => {
  let container: HTMLElement;
  let view: EditorView;

  beforeEach(() => {
    container = document.createElement("div");
    document.body.appendChild(container);
    const state = EditorState.create({ schema: richSchema });
    view = new EditorView(container, { state });

    // Mock fetch for /api/images
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation(async (url: string) => {
        if (url === "/api/images") {
          return {
            ok: true,
            status: 200,
            json: async () => [
              {
                hash: "hash1",
                name: "test1.png",
                url: "/blob/hash1",
                size: 1024,
              },
              {
                hash: "hash2",
                name: "test2.jpg",
                url: "/blob/hash2",
                size: 2048,
              },
            ],
          };
        }
        return { ok: false, status: 404 };
      }),
    );
  });

  afterEach(() => {
    vi.restoreAllMocks();
    view.destroy();
    container.remove();
    // Clean up any remaining overlays
    const overlay = document.querySelector(".pm-image-browser-overlay");
    if (overlay) overlay.remove();
  });

  it("renders overlay and dialog with proper ARIA attributes", () => {
    openImageBrowser(view, richSchema);

    const dialog = document.querySelector(".pm-image-browser-dialog");
    expect(dialog).not.toBeNull();
    expect(dialog?.getAttribute("role")).toBe("dialog");
    expect(dialog?.getAttribute("aria-modal")).toBe("true");
    expect(dialog?.getAttribute("aria-label")).toBe("Image Browser");

    const closeBtn = document.querySelector(".pm-image-browser-close");
    expect(closeBtn).not.toBeNull();
    expect(closeBtn?.getAttribute("aria-label")).toBe("Close image browser");
  });

  it("does not create duplicate overlays when called twice", () => {
    openImageBrowser(view, richSchema);
    openImageBrowser(view, richSchema);

    const overlays = document.querySelectorAll(".pm-image-browser-overlay");
    expect(overlays.length).toBe(1);
  });

  it("renders image cards with accessibility attributes and handles selection and keyboard events", async () => {
    openImageBrowser(view, richSchema);

    // Allow promise tick to complete fetchImages .then()
    await new Promise((resolve) => setTimeout(resolve, 50));

    const cards = document.querySelectorAll<HTMLElement>(".pm-image-browser-card");
    expect(cards.length).toBe(2);
    const firstCard = cards[0];
    expect(firstCard.getAttribute("role")).toBe("button");
    expect(firstCard.getAttribute("tabindex")).toBe("0");
    expect(firstCard.getAttribute("aria-label")).toBe("Select image test1.png");

    const insertBtn = document.querySelector<HTMLButtonElement>(".pm-image-browser-insert");
    expect(insertBtn?.disabled).toBe(true);

    // Press Enter to select card via keyboard
    firstCard.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }) as unknown as Event);

    expect(firstCard.classList.contains("selected")).toBe(true);
    expect(insertBtn?.disabled).toBe(false);
  });

  it("closes dialog when Escape key is pressed", () => {
    openImageBrowser(view, richSchema);

    expect(document.querySelector(".pm-image-browser-overlay")).not.toBeNull();

    // Cast needed: Datastar overrides Document.dispatchEvent to only accept DatastarSignalEvent
    (document as unknown as EventTarget).dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));

    expect(document.querySelector(".pm-image-browser-overlay")).toBeNull();
  });

  it("closes dialog when close button is clicked", () => {
    openImageBrowser(view, richSchema);

    const closeBtn = document.querySelector<HTMLButtonElement>(".pm-image-browser-close");
    closeBtn?.click();

    expect(document.querySelector(".pm-image-browser-overlay")).toBeNull();
  });
});
