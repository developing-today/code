/**
 * Image browser dialog for ProseMirror.
 *
 * Shows a gallery of previously uploaded images and allows
 * inserting one into the document at the current cursor position.
 */

import type { Schema } from "prosemirror-model";
import type { EditorView } from "prosemirror-view";

// ============================================================================
// Types
// ============================================================================

interface ImageEntry {
  hash: string;
  name: string;
  url: string;
  size: number;
}

interface ImageListPage {
  images: ImageEntry[];
  next_cursor: string | null;
}

// ============================================================================
// Image Browser
// ============================================================================

/**
 * The one open browser, if any. The overlay is appended to `document.body`,
 * so it outlives the editor view that opened it: `closeEditor()`, SPA
 * navigation, and collab re-init all destroy the view while a body-level
 * overlay would stay on screen, still holding the dead view in its Insert
 * closures (#1381). Module state lets those teardown paths close the
 * browser, mirroring `destroySearchPanel` / `destroyGotoLineDialog`.
 */
interface ActiveBrowser {
  view: EditorView;
  close: () => void;
}

let activeBrowser: ActiveBrowser | null = null;

/**
 * Close the image browser if it is open.
 * Call this when the editor is being destroyed. Safe to call when no
 * browser is open, and safe to call more than once.
 */
export function closeImageBrowser(): void {
  activeBrowser?.close();
}

/**
 * Open the image browser dialog.
 * Fetches available images from `/api/images` and displays a grid.
 * On selection, inserts the image at the current cursor position.
 */
export function openImageBrowser(view: EditorView, schema: Schema): void {
  if (activeBrowser) {
    // Don't open multiple for the same view.
    if (activeBrowser.view === view) return;
    // A browser left over from a previous (torn-down) view self-closes:
    // its Insert closures dispatch on a destroyed view (#1381).
    closeImageBrowser();
  }
  if (document.querySelector(".pm-image-browser-overlay")) return;

  // Overlay
  const overlay = document.createElement("div");
  overlay.className = "pm-image-browser-overlay";

  // Dialog
  const dialog = document.createElement("div");
  dialog.className = "pm-image-browser-dialog";
  dialog.setAttribute("role", "dialog");
  dialog.setAttribute("aria-modal", "true");
  dialog.setAttribute("aria-label", "Image Browser");

  // Header
  const header = document.createElement("div");
  header.className = "pm-image-browser-header";

  const title = document.createElement("h3");
  title.textContent = "Image Browser";
  title.className = "pm-image-browser-title";

  const closeBtn = document.createElement("button");
  closeBtn.className = "pm-image-browser-close";
  closeBtn.textContent = "✕";
  closeBtn.setAttribute("aria-label", "Close image browser");
  closeBtn.addEventListener("click", close);

  header.appendChild(title);
  header.appendChild(closeBtn);

  // Grid container
  const grid = document.createElement("div");
  grid.className = "pm-image-browser-grid";
  grid.textContent = "Loading…";

  // Footer with insert button
  const footer = document.createElement("div");
  footer.className = "pm-image-browser-footer";

  const insertBtn = document.createElement("button");
  insertBtn.className = "pm-image-browser-insert";
  insertBtn.textContent = "Insert";
  insertBtn.disabled = true;
  insertBtn.addEventListener("click", () => {
    if (selectedImage) {
      // Dispatching on a destroyed view throws: log it and dismiss in a
      // finally, so the dialog never stays stuck on screen (#1381).
      try {
        insertImage(selectedImage);
      } catch (err) {
        console.warn("[image-browser] Insert failed:", err);
      } finally {
        close();
      }
    }
  });

  footer.appendChild(insertBtn);

  const loadMoreBtn = document.createElement("button");
  loadMoreBtn.className = "pm-image-browser-load-more";
  loadMoreBtn.textContent = "Load more";
  loadMoreBtn.hidden = true;
  footer.appendChild(loadMoreBtn);

  dialog.appendChild(header);
  dialog.appendChild(grid);
  dialog.appendChild(footer);
  overlay.appendChild(dialog);

  // Close on overlay click
  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) close();
  });

  // Close on Escape
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") close();
  };
  document.addEventListener("keydown", onKeyDown);

  document.body.appendChild(overlay);

  // State
  let selectedImage: ImageEntry | null = null;

  let nextCursor: string | null = null;
  let hasCards = false;

  loadMoreBtn.addEventListener("click", () => {
    if (nextCursor === null || loadMoreBtn.disabled) return;
    loadMoreBtn.disabled = true;
    loadMoreBtn.textContent = "Loading…";
    fetchImagePage(nextCursor)
      .then((page) => renderPage(page, false))
      .catch(showLoadError)
      .finally(() => {
        loadMoreBtn.disabled = false;
        loadMoreBtn.textContent = "Load more";
      });
  });

  fetchImagePage()
    .then((page) => renderPage(page, true))
    .catch(showLoadError);

  function renderPage(page: ImageListPage, firstPage: boolean): void {
    if (firstPage) grid.textContent = "";
    if (page.images.length > 0 && !hasCards) grid.textContent = "";
    if (page.images.length === 0 && firstPage && page.next_cursor === null) {
      const empty = document.createElement("div");
      empty.className = "pm-image-browser-empty";
      empty.textContent = "No images uploaded yet. Paste or drag an image into the editor to upload.";
      grid.appendChild(empty);
    }
    if (page.images.length === 0 && firstPage && page.next_cursor !== null) {
      grid.textContent = "No images in this page. Load more to continue.";
    }

    for (const img of page.images) {
      const card = document.createElement("div");
      card.className = "pm-image-browser-card";
      card.dataset.hash = img.hash;
      card.setAttribute("role", "button");
      card.setAttribute("tabindex", "0");
      card.setAttribute("aria-label", `Select image ${img.name}`);

      const thumb = document.createElement("img");
      thumb.src = img.url;
      thumb.alt = img.name;
      thumb.className = "pm-image-browser-thumb";
      thumb.loading = "lazy";

      const name = document.createElement("span");
      name.className = "pm-image-browser-name";
      name.textContent = img.name;
      name.title = img.name;

      card.appendChild(thumb);
      card.appendChild(name);
      card.addEventListener("click", () => {
        for (const c of grid.querySelectorAll(".pm-image-browser-card.selected")) {
          c.classList.remove("selected");
        }
        card.classList.add("selected");
        selectedImage = img;
        insertBtn.disabled = false;
      });
      card.addEventListener("dblclick", () => {
        selectedImage = img;
        try {
          insertImage(img);
        } catch (err) {
          console.warn("[image-browser] Insert failed:", err);
        } finally {
          close();
        }
      });
      card.addEventListener("keydown", (e: KeyboardEvent) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          card.click();
        }
      });
      grid.appendChild(card);
      hasCards = true;
    }

    nextCursor = page.next_cursor;
    loadMoreBtn.hidden = nextCursor === null;
    if (!firstPage && page.images.length === 0 && !hasCards) {
      grid.textContent = "No images in this page. Load more to continue.";
    }
  }

  function showLoadError(err: unknown): void {
    // Keep already-loaded cards visible if a later page fails.
    if (!hasCards) {
      grid.textContent = `Failed to load images: ${err instanceof Error ? err.message : String(err)}`;
    }
  }

  function insertImage(img: ImageEntry): void {
    const imageNode = schema.nodes.image.create({
      src: img.url,
      alt: img.name,
    });
    const tr = view.state.tr.replaceSelectionWith(imageNode);
    view.dispatch(tr);
    view.focus();
  }

  function close(): void {
    if (activeBrowser?.close === close) {
      activeBrowser = null;
    }
    document.removeEventListener("keydown", onKeyDown);
    overlay.remove();
  }

  activeBrowser = { view, close };
}

// ============================================================================
// API
// ============================================================================

/**
 * Validate the /api/images payload against its contract before the
 * caller touches the grid (#7812). The server contract
 * (routes.rs `images_list_handler`) is a page with an `images` array,
 * nullable `next_cursor`, and entries with string `hash`/`name`/`url`
 * and numeric `size`. Validation is
 * all-or-nothing — one malformed element rejects the whole payload —
 * so a wrong-shape 200 fails closed with a clean error instead of
 * rendering a raw TypeError or bogus cards from unvalidated fields.
 * Oracle: the ImageEntry struct in pkgs/id/src/web/routes.rs.
 */
function parseImagePage(data: unknown): ImageListPage {
  if (
    data === null ||
    typeof data !== "object" ||
    !Array.isArray((data as Record<string, unknown>).images) ||
    !(
      (data as Record<string, unknown>).next_cursor === null ||
      typeof (data as Record<string, unknown>).next_cursor === "string"
    )
  ) {
    throw new Error("unexpected response from /api/images");
  }
  const images = (data as { images: unknown[] }).images;
  for (const entry of images as Array<Record<string, unknown> | null>) {
    if (
      entry === null ||
      typeof entry !== "object" ||
      typeof entry.hash !== "string" ||
      typeof entry.name !== "string" ||
      typeof entry.url !== "string" ||
      typeof entry.size !== "number"
    ) {
      throw new Error("unexpected response from /api/images");
    }
  }
  return data as ImageListPage;
}

async function fetchImagePage(after?: string): Promise<ImageListPage> {
  const query = after !== undefined ? `?after=${encodeURIComponent(after)}` : "";
  const response = await fetch(`/api/images${query}`);
  if (!response.ok) {
    throw new Error(`Failed to fetch images: ${response.status}`);
  }
  return parseImagePage(await response.json());
}
