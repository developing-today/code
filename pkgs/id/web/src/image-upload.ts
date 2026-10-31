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
import { insertPoint } from "prosemirror-transform";

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

/**
 * The image files from a paste/drop that will actually be uploaded:
 * allowed type and within MAX_IMAGE_SIZE. Oversized files are skipped
 * with a warning. Handlers must run this BEFORE claiming the event:
 * when it returns empty, the paste/drop is not ours to claim, and
 * default handling (including any other clipboard content) must
 * still run (#1321).
 */
function uploadableImages(files: FileList): File[] {
  const accepted: File[] = [];
  for (const file of Array.from(files).filter(isImageFile)) {
    if (file.size > MAX_IMAGE_SIZE) {
      console.warn(`[image-upload] File too large: ${file.name} (${file.size} bytes, max ${MAX_IMAGE_SIZE})`);
      continue;
    }
    accepted.push(file);
  }
  return accepted;
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
              key: `placeholder-${action.id}`,
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

        const accepted = uploadableImages(files);
        if (accepted.length === 0) return false;

        event.preventDefault();

        startImageUploadBatch(view, accepted, view.state.selection.from);

        return true;
      },

      handleDrop(view: EditorView, event: DragEvent): boolean {
        const files = event.dataTransfer?.files;
        if (!files || files.length === 0) return false;

        const accepted = uploadableImages(files);
        if (accepted.length === 0) return false;

        event.preventDefault();

        // Get drop position from coordinates
        const coords = { left: event.clientX, top: event.clientY };
        const pos = view.posAtCoords(coords);
        const insertPos = pos ? pos.pos : view.state.selection.from;

        startImageUploadBatch(view, accepted, insertPos);

        return true;
      },
    },
  });
}

/**
 * Start one paste/drop batch: every upload starts immediately (they run
 * concurrently), but the document insertions are chained in file order
 * (#1323). Each upload's insertion step waits for every earlier file in
 * the batch to reach its own insertion step, so images land in
 * clipboard/file-list order no matter which upload finishes first. A
 * failed upload resolves its step without inserting, so the survivors
 * keep their relative order.
 */
function startImageUploadBatch(view: EditorView, files: File[], insertPos: number): void {
  let insertionGate: Promise<void> = Promise.resolve();
  for (const file of files) {
    insertionGate = handleImageUpload(view, file, insertPos, insertionGate);
  }
}

/**
 * Handle uploading a single image file and inserting it into the
 * document. The placeholder is added and the upload is started
 * synchronously; the returned promise resolves once this file's
 * insertion step has run. The step waits for `insertionGate` (the
 * previous batch member's step) before touching the document, which is
 * what pins the batch's document order to the file order (#1323).
 */
function handleImageUpload(
  view: EditorView,
  file: File,
  insertPos: number,
  insertionGate: Promise<void>,
): Promise<void> {
  const id = `upload-${Math.random().toString(36).slice(2, 10)}`;

  // Add placeholder decoration
  const tr = view.state.tr;
  tr.setMeta(imageUploadKey, { type: "add", id, pos: insertPos } satisfies PlaceholderAction);
  view.dispatch(tr);

  const fileName = file.name || generatePasteFilename(file.type);

  const uploaded = uploadImageFile(file).then(
    (result) => ({ ok: true as const, result }),
    (err: unknown) => {
      console.warn(`[image-upload] Upload failed for ${fileName}:`, err);
      return { ok: false as const, result: null };
    },
  );

  return (async () => {
    const outcome = await uploaded;
    // Insert strictly after every earlier batch member's step (#1323).
    await insertionGate;

    if (outcome.ok) {
      try {
        // Find where to insert (placeholder position may have shifted).
        // The placeholder may be gone (its range was deleted while the
        // upload was in flight); then fall back to the captured insert
        // position, clamped into the current document range — the doc
        // may have shrunk past it, and inserting there would throw and
        // silently lose a successfully uploaded image (#1379).
        const decos = imageUploadKey.getState(view.state);
        const found = decos?.find(undefined, undefined, (spec) => spec.id === id);
        const wanted = found && found.length > 0 ? found[0].from : insertPos;
        const clamped = Math.max(0, Math.min(wanted, view.state.doc.content.size));
        const pos = insertPoint(view.state.doc, clamped, view.state.schema.nodes.image) ?? clamped;

        // Insert the image and remove the placeholder in one step.
        const insertTr = view.state.tr;
        insertTr.setMeta(imageUploadKey, { type: "remove", id } satisfies PlaceholderAction);
        const imageNode = view.state.schema.nodes.image.create({
          src: outcome.result.url,
          alt: outcome.result.name || fileName,
        });
        insertTr.insert(pos, imageNode);
        view.dispatch(insertTr);
        return;
      } catch (err) {
        // Distinct from an upload failure: the upload succeeded, the
        // document insert failed. Never surface this as "Upload
        // failed" and never let it escape as an unhandled rejection
        // (#1379).
        console.warn(`[image-upload] Insertion failed for ${fileName}:`, err);
      }
    }

    // Upload failed or insertion failed: remove only the placeholder.
    const removeTr = view.state.tr;
    removeTr.setMeta(imageUploadKey, { type: "remove", id } satisfies PlaceholderAction);
    view.dispatch(removeTr);
  })();
}
