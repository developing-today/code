/**
 * Image upload plugin for ProseMirror.
 *
 * Handles paste and drag-drop of images, uploading them to the server
 * via `/api/upload` and inserting `<img>` nodes into the document.
 *
 * Only active when the editor schema includes an `image` node
 * (rich/markdown/plain modes, not raw mode).
 */

import type { Schema } from "prosemirror-model";
import { Plugin, PluginKey } from "prosemirror-state";
import { Decoration, DecorationSet, type EditorView } from "prosemirror-view";

// ============================================================================
// Constants
// ============================================================================

/** MIME types accepted for image upload. */
export const ALLOWED_IMAGE_TYPES: readonly string[] = [
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
  "image/svg+xml",
  "image/bmp",
  "image/x-icon",
] as const;

/** Maximum image file size in bytes (10 MB). */
export const MAX_IMAGE_SIZE = 10 * 1024 * 1024;

// ============================================================================
// Helpers
// ============================================================================

/** Check if a File is an allowed image type. */
export function isImageFile(file: File): boolean {
  return ALLOWED_IMAGE_TYPES.includes(file.type);
}

/** Map a MIME type to a file extension. */
export function mimeToExtension(mime: string): string {
  const map: Record<string, string> = {
    "image/png": "png",
    "image/jpeg": "jpg",
    "image/gif": "gif",
    "image/webp": "webp",
    "image/svg+xml": "svg",
    "image/bmp": "bmp",
    "image/x-icon": "ico",
  };
  return map[mime] ?? "bin";
}

/** Generate a filename for a pasted image (no original name). */
export function generatePasteFilename(mimeType: string): string {
  const ext = mimeToExtension(mimeType);
  return `paste-${Date.now()}.${ext}`;
}

// ============================================================================
// Upload API
// ============================================================================

/** Response from the `/api/upload` endpoint. */
export interface UploadResponse {
  hash: string;
  name: string;
  url: string;
}

/** Upload an image file to the server. */
export async function uploadImageFile(file: File): Promise<UploadResponse> {
  const formData = new FormData();
  formData.append("file", file);

  const response = await fetch("/api/upload", {
    method: "POST",
    body: formData,
  });

  if (!response.ok) {
    const text = await response.text();
    throw new Error(`Upload failed (${response.status}): ${text}`);
  }

  return (await response.json()) as UploadResponse;
}

// ============================================================================
// Placeholder Decorations
// ============================================================================

const imageUploadKey = new PluginKey<DecorationSet>("imageUpload");

interface PlaceholderAction {
  type: "add" | "remove";
  id: string;
  pos?: number;
}

function createPlaceholderWidget(): HTMLElement {
  const el = document.createElement("span");
  el.className = "image-upload-placeholder";
  return el;
}

// ============================================================================
// ProseMirror Plugin
// ============================================================================

/**
 * Create the image upload plugin for a schema.
 * Returns `null` if the schema has no `image` node (e.g., raw mode).
 */
export function createImageUploadPlugin(schema: Schema): Plugin | null {
  if (!schema.nodes.image) {
    return null;
  }

  return new Plugin<DecorationSet>({
    key: imageUploadKey,

    state: {
      init(): DecorationSet {
        return DecorationSet.empty;
      },
      apply(tr, set): DecorationSet {
        // Map decorations through document changes
        let mapped = set.map(tr.mapping, tr.doc);

        const action = tr.getMeta(imageUploadKey) as PlaceholderAction | undefined;
        if (action) {
          if (action.type === "add" && action.pos != null) {
            const widget = Decoration.widget(action.pos, createPlaceholderWidget, {
              id: action.id,
            });
            mapped = mapped.add(tr.doc, [widget]);
          } else if (action.type === "remove") {
            const found = mapped.find(undefined, undefined, (spec) => spec.id === action.id);
            if (found.length > 0) {
              mapped = mapped.remove(found);
            }
          }
        }

        return mapped;
      },
    },

    props: {
      decorations(state) {
        return imageUploadKey.getState(state);
      },

      handlePaste(view: EditorView, event: ClipboardEvent): boolean {
        const files = event.clipboardData?.files;
        if (!files || files.length === 0) return false;

        const imageFiles = Array.from(files).filter(isImageFile);
        if (imageFiles.length === 0) return false;

        event.preventDefault();

        for (const file of imageFiles) {
          if (file.size > MAX_IMAGE_SIZE) {
            console.warn(`[image-upload] File too large: ${file.name} (${file.size} bytes, max ${MAX_IMAGE_SIZE})`);
            continue;
          }
          handleImageUpload(view, file, view.state.selection.from);
        }

        return true;
      },

      handleDrop(view: EditorView, event: DragEvent): boolean {
        const files = event.dataTransfer?.files;
        if (!files || files.length === 0) return false;

        const imageFiles = Array.from(files).filter(isImageFile);
        if (imageFiles.length === 0) return false;

        event.preventDefault();

        // Get drop position from coordinates
        const coords = { left: event.clientX, top: event.clientY };
        const pos = view.posAtCoords(coords);
        const insertPos = pos ? pos.pos : view.state.selection.from;

        for (const file of imageFiles) {
          if (file.size > MAX_IMAGE_SIZE) {
            console.warn(`[image-upload] File too large: ${file.name} (${file.size} bytes, max ${MAX_IMAGE_SIZE})`);
            continue;
          }
          handleImageUpload(view, file, insertPos);
        }

        return true;
      },
    },
  });
}

/**
 * Handle uploading a single image file and inserting it into the document.
 */
function handleImageUpload(view: EditorView, file: File, insertPos: number): void {
  const id = `upload-${Math.random().toString(36).slice(2, 10)}`;

  // Add placeholder decoration
  const tr = view.state.tr;
  tr.setMeta(imageUploadKey, { type: "add", id, pos: insertPos } satisfies PlaceholderAction);
  view.dispatch(tr);

  const fileName = file.name || generatePasteFilename(file.type);

  uploadImageFile(file)
    .then((result) => {
      // Remove placeholder
      const removeTr = view.state.tr;
      removeTr.setMeta(imageUploadKey, { type: "remove", id } satisfies PlaceholderAction);

      // Find where to insert (placeholder position may have shifted)
      const decos = imageUploadKey.getState(view.state);
      const found = decos?.find(undefined, undefined, (spec) => spec.id === id);
      const pos = found && found.length > 0 ? found[0].from : insertPos;

      // Insert image node
      const imageNode = view.state.schema.nodes.image.create({
        src: result.url,
        alt: result.name || fileName,
      });
      removeTr.insert(pos, imageNode);
      view.dispatch(removeTr);
    })
    .catch((err) => {
      console.warn(`[image-upload] Upload failed for ${fileName}:`, err);
      // Remove placeholder on failure
      const removeTr = view.state.tr;
      removeTr.setMeta(imageUploadKey, { type: "remove", id } satisfies PlaceholderAction);
      view.dispatch(removeTr);
    });
}
