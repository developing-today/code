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

// ============================================================================
// Image Browser
// ============================================================================

/**
 * Open the image browser dialog.
 * Fetches available images from `/api/images` and displays a grid.
 * On selection, inserts the image at the current cursor position.
 */
export function openImageBrowser(view: EditorView, schema: Schema): void {
  // Don't open multiple
  if (document.querySelector(".pm-image-browser-overlay")) return;

  // Overlay
  const overlay = document.createElement("div");
  overlay.className = "pm-image-browser-overlay";

  // Dialog
  const dialog = document.createElement("div");
  dialog.className = "pm-image-browser-dialog";

  // Header
  const header = document.createElement("div");
  header.className = "pm-image-browser-header";

  const title = document.createElement("h3");
  title.textContent = "Image Browser";
  title.className = "pm-image-browser-title";

  const closeBtn = document.createElement("button");
  closeBtn.className = "pm-image-browser-close";
  closeBtn.textContent = "✕";
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
      insertImage(selectedImage);
      close();
    }
  });

  footer.appendChild(insertBtn);

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

  // Fetch images
  fetchImages().then((images) => {
    grid.textContent = "";

    if (images.length === 0) {
      const empty = document.createElement("div");
      empty.className = "pm-image-browser-empty";
      empty.textContent = "No images uploaded yet. Paste or drag an image into the editor to upload.";
      grid.appendChild(empty);
      return;
    }

    for (const img of images) {
      const card = document.createElement("div");
      card.className = "pm-image-browser-card";
      card.dataset.hash = img.hash;

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
        // Deselect previous
        for (const c of grid.querySelectorAll(".pm-image-browser-card.selected")) {
          c.classList.remove("selected");
        }
        card.classList.add("selected");
        selectedImage = img;
        insertBtn.disabled = false;
      });

      // Double-click to insert immediately
      card.addEventListener("dblclick", () => {
        selectedImage = img;
        insertImage(img);
        close();
      });

      grid.appendChild(card);
    }
  }).catch((err) => {
    grid.textContent = `Failed to load images: ${err}`;
  });

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
    document.removeEventListener("keydown", onKeyDown);
    overlay.remove();
  }
}

// ============================================================================
// API
// ============================================================================

async function fetchImages(): Promise<ImageEntry[]> {
  const response = await fetch("/api/images");
  if (!response.ok) {
    throw new Error(`Failed to fetch images: ${response.status}`);
  }
  return (await response.json()) as ImageEntry[];
}
