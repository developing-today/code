/**
 * Tests for image upload plugin helpers and constants.
 * Tests MIME validation, filename generation, extension mapping,
 * and ProseMirror plugin creation.
 */

import type { Slice } from "prosemirror-model";
import { EditorState, TextSelection } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { rawSchema, richSchema } from "./editor";
import {
  ALLOWED_IMAGE_TYPES,
  createImageUploadPlugin,
  generatePasteFilename,
  isImageFile,
  MAX_IMAGE_SIZE,
  mimeToExtension,
  uploadImageFile,
} from "./image-upload";

// ============================================================================
// Constants
// ============================================================================

describe("ALLOWED_IMAGE_TYPES", () => {
  it("is an array", () => {
    expect(Array.isArray(ALLOWED_IMAGE_TYPES)).toBe(true);
  });

  it("contains all 7 expected types", () => {
    expect(ALLOWED_IMAGE_TYPES).toContain("image/png");
    expect(ALLOWED_IMAGE_TYPES).toContain("image/jpeg");
    expect(ALLOWED_IMAGE_TYPES).toContain("image/gif");
    expect(ALLOWED_IMAGE_TYPES).toContain("image/webp");
    expect(ALLOWED_IMAGE_TYPES).toContain("image/svg+xml");
    expect(ALLOWED_IMAGE_TYPES).toContain("image/bmp");
    expect(ALLOWED_IMAGE_TYPES).toContain("image/x-icon");
  });

  it("has exactly 7 types", () => {
    expect(ALLOWED_IMAGE_TYPES).toHaveLength(7);
  });
});

describe("MAX_IMAGE_SIZE", () => {
  it("equals 10 MB", () => {
    expect(MAX_IMAGE_SIZE).toBe(10 * 1024 * 1024);
  });

  it("is 10485760 bytes", () => {
    expect(MAX_IMAGE_SIZE).toBe(10_485_760);
  });
});

// ============================================================================
// isImageFile
// ============================================================================

describe("isImageFile", () => {
  it("accepts image/png", () => {
    expect(isImageFile(new File([""], "test.png", { type: "image/png" }))).toBe(true);
  });

  it("accepts image/jpeg", () => {
    expect(isImageFile(new File([""], "test.jpg", { type: "image/jpeg" }))).toBe(true);
  });

  it("accepts image/gif", () => {
    expect(isImageFile(new File([""], "test.gif", { type: "image/gif" }))).toBe(true);
  });

  it("accepts image/webp", () => {
    expect(isImageFile(new File([""], "test.webp", { type: "image/webp" }))).toBe(true);
  });

  it("accepts image/svg+xml", () => {
    expect(isImageFile(new File([""], "test.svg", { type: "image/svg+xml" }))).toBe(true);
  });

  it("accepts image/bmp", () => {
    expect(isImageFile(new File([""], "test.bmp", { type: "image/bmp" }))).toBe(true);
  });

  it("accepts image/x-icon", () => {
    expect(isImageFile(new File([""], "test.ico", { type: "image/x-icon" }))).toBe(true);
  });

  it("rejects text/plain", () => {
    expect(isImageFile(new File([""], "test.txt", { type: "text/plain" }))).toBe(false);
  });

  it("rejects application/pdf", () => {
    expect(isImageFile(new File([""], "test.pdf", { type: "application/pdf" }))).toBe(false);
  });

  it("rejects empty type", () => {
    expect(isImageFile(new File([""], "test", { type: "" }))).toBe(false);
  });

  it("rejects application/json", () => {
    expect(isImageFile(new File([""], "test.json", { type: "application/json" }))).toBe(false);
  });
});

// ============================================================================
// mimeToExtension
// ============================================================================

describe("mimeToExtension", () => {
  it("maps image/png to png", () => {
    expect(mimeToExtension("image/png")).toBe("png");
  });

  it("maps image/jpeg to jpg", () => {
    expect(mimeToExtension("image/jpeg")).toBe("jpg");
  });

  it("maps image/gif to gif", () => {
    expect(mimeToExtension("image/gif")).toBe("gif");
  });

  it("maps image/webp to webp", () => {
    expect(mimeToExtension("image/webp")).toBe("webp");
  });

  it("maps image/svg+xml to svg", () => {
    expect(mimeToExtension("image/svg+xml")).toBe("svg");
  });

  it("maps image/bmp to bmp", () => {
    expect(mimeToExtension("image/bmp")).toBe("bmp");
  });

  it("maps image/x-icon to ico", () => {
    expect(mimeToExtension("image/x-icon")).toBe("ico");
  });

  it("returns bin for unknown MIME", () => {
    expect(mimeToExtension("application/octet-stream")).toBe("bin");
  });

  it("returns bin for empty string", () => {
    expect(mimeToExtension("")).toBe("bin");
  });
});

// ============================================================================
// generatePasteFilename
// ============================================================================

describe("generatePasteFilename", () => {
  it("includes timestamp digits", () => {
    const name = generatePasteFilename("image/png");
    expect(name).toMatch(/^paste-\d+\.png$/);
  });

  it("uses correct extension for jpeg", () => {
    const name = generatePasteFilename("image/jpeg");
    expect(name).toMatch(/^paste-\d+\.jpg$/);
  });

  it("uses correct extension for gif", () => {
    const name = generatePasteFilename("image/gif");
    expect(name).toMatch(/^paste-\d+\.gif$/);
  });

  it("uses correct extension for svg", () => {
    const name = generatePasteFilename("image/svg+xml");
    expect(name).toMatch(/^paste-\d+\.svg$/);
  });

  it("uses bin for unknown MIME", () => {
    const name = generatePasteFilename("application/octet-stream");
    expect(name).toMatch(/^paste-\d+\.bin$/);
  });

  it("format matches paste-{digits}.{ext}", () => {
    const name = generatePasteFilename("image/webp");
    expect(name).toMatch(/^paste-\d{13,}\.webp$/);
  });
});

// ============================================================================
// createImageUploadPlugin
// ============================================================================

describe("createImageUploadPlugin", () => {
  it("returns Plugin when schema has image node (richSchema)", () => {
    const plugin = createImageUploadPlugin(richSchema);
    expect(plugin).not.toBeNull();
  });

  it("returns null when schema lacks image node (rawSchema)", () => {
    const plugin = createImageUploadPlugin(rawSchema);
    expect(plugin).toBeNull();
  });

  it("plugin has props with handlePaste", () => {
    const plugin = createImageUploadPlugin(richSchema);
    expect(plugin).not.toBeNull();
    expect(plugin!.props.handlePaste).toBeDefined();
    expect(typeof plugin!.props.handlePaste).toBe("function");
  });

  it("plugin has props with handleDrop", () => {
    const plugin = createImageUploadPlugin(richSchema);
    expect(plugin).not.toBeNull();
    expect(plugin!.props.handleDrop).toBeDefined();
    expect(typeof plugin!.props.handleDrop).toBe("function");
  });

  it("plugin has decorations prop", () => {
    const plugin = createImageUploadPlugin(richSchema);
    expect(plugin).not.toBeNull();
    expect(plugin!.props.decorations).toBeDefined();
    expect(typeof plugin!.props.decorations).toBe("function");
  });
});

// ============================================================================
// uploadImageFile
// ============================================================================

describe("uploadImageFile", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("sends FormData with file to /api/upload", async () => {
    const mockResponse = {
      ok: true,
      json: () => Promise.resolve({ hash: "abc123", name: "photo.png", url: "/blob/abc123?filename=photo.png" }),
    };
    const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse as Response);

    const file = new File(["fake image data"], "photo.png", { type: "image/png" });
    const result = await uploadImageFile(file);

    expect(fetchSpy).toHaveBeenCalledOnce();
    const [url, options] = fetchSpy.mock.calls[0];
    expect(url).toBe("/api/upload");
    expect(options?.method).toBe("POST");
    expect(options?.body).toBeInstanceOf(FormData);

    expect(result.hash).toBe("abc123");
    expect(result.name).toBe("photo.png");
    expect(result.url).toBe("/blob/abc123?filename=photo.png");
  });

  it("throws on non-ok response", async () => {
    const mockResponse = {
      ok: false,
      status: 400,
      text: () => Promise.resolve("Invalid file type"),
    };
    vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse as Response);

    const file = new File(["not an image"], "test.txt", { type: "text/plain" });
    await expect(uploadImageFile(file)).rejects.toThrow("Upload failed (400): Invalid file type");
  });

  it("throws on network error", async () => {
    vi.spyOn(globalThis, "fetch").mockRejectedValue(new Error("Network error"));

    const file = new File(["data"], "photo.png", { type: "image/png" });
    await expect(uploadImageFile(file)).rejects.toThrow("Network error");
  });
});

// ============================================================================
// handlePaste / handleDrop size filtering (#1321)
// ============================================================================

describe("handlePaste/handleDrop oversized filtering", () => {
  const plugin = createImageUploadPlugin(richSchema);
  if (!plugin) throw new Error("rich schema must have an image node");
  const handlePaste = plugin.props.handlePaste;
  const handleDrop = plugin.props.handleDrop;
  if (!handlePaste || !handleDrop) throw new Error("plugin must define paste/drop handlers");

  function oversizedFile(name = "big.png"): File {
    const file = new File(["x"], name, { type: "image/png" });
    Object.defineProperty(file, "size", { value: MAX_IMAGE_SIZE + 1 });
    return file;
  }

  function stubView() {
    const tr = { setMeta: vi.fn().mockReturnThis() };
    return {
      state: { tr, selection: { from: 0 } },
      dispatch: vi.fn(),
      posAtCoords: vi.fn().mockReturnValue(null),
    };
  }

  function pasteEvent(files: File[]) {
    return { clipboardData: { files }, preventDefault: vi.fn() };
  }

  function dropEvent(files: File[]) {
    return { dataTransfer: { files }, preventDefault: vi.fn(), clientX: 10, clientY: 20 };
  }

  beforeEach(() => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    // Uploads never settle, so dispatch counts stay at the synchronous
    // placeholder insert for accepted files.
    vi.spyOn(globalThis, "fetch").mockImplementation(() => new Promise(() => {}));
  });

  it("oversized-only paste is not claimed: returns false, no preventDefault, no dispatch", () => {
    const view = stubView();
    const event = pasteEvent([oversizedFile()]);
    const result = handlePaste.call(
      plugin,
      view as unknown as EditorView,
      event as unknown as ClipboardEvent,
      undefined as unknown as Slice,
    );
    expect(result).toBe(false);
    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(view.dispatch).not.toHaveBeenCalled();
  });

  it("oversized-only drop is not claimed: returns false, no preventDefault, no dispatch", () => {
    const view = stubView();
    const event = dropEvent([oversizedFile()]);
    const result = handleDrop.call(
      plugin,
      view as unknown as EditorView,
      event as unknown as DragEvent,
      undefined as unknown as Slice,
      false,
    );
    expect(result).toBe(false);
    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(view.dispatch).not.toHaveBeenCalled();
    expect(view.posAtCoords).not.toHaveBeenCalled();
  });

  it("mixed paste uploads only the valid file and still claims the event", () => {
    const view = stubView();
    const event = pasteEvent([oversizedFile(), new File(["ok"], "ok.png", { type: "image/png" })]);
    const result = handlePaste.call(
      plugin,
      view as unknown as EditorView,
      event as unknown as ClipboardEvent,
      undefined as unknown as Slice,
    );
    expect(result).toBe(true);
    expect(event.preventDefault).toHaveBeenCalledOnce();
    // One synchronous placeholder dispatch, for the one accepted file.
    expect(view.dispatch).toHaveBeenCalledTimes(1);
  });

  it("mixed drop uploads only the valid file and still claims the event", () => {
    const view = stubView();
    const event = dropEvent([new File(["ok"], "ok.png", { type: "image/png" }), oversizedFile()]);
    const result = handleDrop.call(
      plugin,
      view as unknown as EditorView,
      event as unknown as DragEvent,
      undefined as unknown as Slice,
      false,
    );
    expect(result).toBe(true);
    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(view.dispatch).toHaveBeenCalledTimes(1);
  });
});

// ============================================================================
// handlePaste batch ordering (#1323)
// ============================================================================

describe("handlePaste batch ordering (#1323)", () => {
  const plugin = createImageUploadPlugin(richSchema);
  if (!plugin) throw new Error("rich schema must have an image node");
  const handlePaste = plugin.props.handlePaste;
  if (!handlePaste) throw new Error("plugin must define a paste handler");

  interface Deferred<T> {
    promise: Promise<T>;
    resolve: (value: T) => void;
    reject: (err: unknown) => void;
  }
  function deferred<T>(): Deferred<T> {
    let resolve!: (value: T) => void;
    let reject!: (err: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
      resolve = res;
      reject = rej;
    });
    return { promise, resolve, reject };
  }

  let view: EditorView | null = null;
  let container: HTMLElement | null = null;
  const uploads = new Map<string, Deferred<Response>>();

  function createView(): EditorView {
    container = document.createElement("div");
    document.body.appendChild(container);
    const doc = richSchema.node("doc", null, [richSchema.node("paragraph")]);
    const state = EditorState.create({ schema: richSchema, doc, plugins: [plugin!] });
    view = new EditorView(container, { state });
    return view;
  }

  function imageAlts(v: EditorView): string[] {
    const alts: string[] = [];
    v.state.doc.descendants((node) => {
      if (node.type.name === "image") alts.push(String(node.attrs.alt));
      return true;
    });
    return alts;
  }

  function paste(v: EditorView, files: File[]): void {
    const event = { clipboardData: { files }, preventDefault: vi.fn() };
    const result = handlePaste!.call(
      plugin,
      v,
      event as unknown as ClipboardEvent,
      undefined as unknown as Slice,
    );
    expect(result).toBe(true);
  }

  function okResponse(name: string): Response {
    return {
      ok: true,
      json: () => Promise.resolve({ hash: name, name, url: `/blob/${name}` }),
    } as Response;
  }

  beforeEach(() => {
    uploads.clear();
    vi.spyOn(console, "warn").mockImplementation(() => {});
    vi.spyOn(globalThis, "fetch").mockImplementation((_url, init) => {
      const file = (init?.body as FormData).get("file") as File;
      const d = deferred<Response>();
      uploads.set(file.name, d);
      return d.promise;
    });
  });

  afterEach(() => {
    view?.destroy();
    view = null;
    container?.remove();
    container = null;
    vi.restoreAllMocks();
  });

  it("inserts a pasted batch in clipboard order when uploads finish out of order", async () => {
    const v = createView();
    paste(v, [
      new File(["a"], "first.png", { type: "image/png" }),
      new File(["b"], "second.png", { type: "image/png" }),
    ]);
    expect(uploads.size).toBe(2);

    // The second upload completes first; nothing may be inserted yet,
    // because first.png's slot comes first in the batch.
    uploads.get("second.png")!.resolve(okResponse("second.png"));
    await vi.waitFor(() => {
      expect(uploads.get("second.png")).toBeDefined();
    });
    await new Promise((r) => setTimeout(r, 20));
    expect(imageAlts(v)).toEqual([]);

    uploads.get("first.png")!.resolve(okResponse("first.png"));
    await vi.waitFor(() => {
      expect(imageAlts(v)).toEqual(["first.png", "second.png"]);
    });
  });

  it("a failed upload mid-batch removes only its placeholder; survivors keep order", async () => {
    const v = createView();
    paste(v, [
      new File(["a"], "one.png", { type: "image/png" }),
      new File(["b"], "bad.png", { type: "image/png" }),
      new File(["c"], "three.png", { type: "image/png" }),
    ]);
    expect(uploads.size).toBe(3);

    // Complete in reverse order, with the middle upload failing.
    uploads.get("three.png")!.resolve(okResponse("three.png"));
    uploads.get("bad.png")!.reject(new Error("boom"));
    await new Promise((r) => setTimeout(r, 20));
    expect(imageAlts(v)).toEqual([]);

    uploads.get("one.png")!.resolve(okResponse("one.png"));
    await vi.waitFor(() => {
      expect(imageAlts(v)).toEqual(["one.png", "three.png"]);
    });
  });
});

// ============================================================================
// Upload completing after the document shrank (#1379)
// ============================================================================

describe("upload completing after the document shrank (#1379)", () => {
  const plugin = createImageUploadPlugin(richSchema);
  if (!plugin) throw new Error("rich schema must have an image node");
  const handlePaste = plugin.props.handlePaste;
  if (!handlePaste) throw new Error("plugin must define a paste handler");

  it("inserts at a clamped in-range position instead of throwing and losing the image", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    // A paragraph long enough to paste at position 25.
    const longDoc = richSchema.node("doc", null, [
      richSchema.node("paragraph", null, [richSchema.text("abcdefghijklmnopqrstuvwxyz0123456789")]),
    ]);
    const state = EditorState.create({
      schema: richSchema,
      doc: longDoc,
      plugins: [plugin!],
      selection: TextSelection.create(longDoc, 25),
    });
    const view = new EditorView(container, { state });

    let resolveFetch!: (value: Response) => void;
    const fetchPromise = new Promise<Response>((res) => {
      resolveFetch = res;
    });
    vi.spyOn(globalThis, "fetch").mockImplementation(() => fetchPromise);
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

    try {
      const event = {
        clipboardData: { files: [new File(["x"], "shrunk.png", { type: "image/png" })] },
        preventDefault: vi.fn(),
      };
      const claimed = handlePaste!.call(
        plugin,
        view,
        event as unknown as ClipboardEvent,
        undefined as unknown as Slice,
      );
      expect(claimed).toBe(true);

      // The document is replaced with a single character while the
      // upload is in flight: the captured insert position (25) is now
      // out of range and the placeholder decoration is deleted with
      // the replaced range.
      const smallDoc = richSchema.node("doc", null, [
        richSchema.node("paragraph", null, [richSchema.text("x")]),
      ]);
      view.dispatch(view.state.tr.replaceWith(0, view.state.doc.content.size, smallDoc.content));

      resolveFetch({
        ok: true,
        json: () => Promise.resolve({ hash: "shrunk", name: "shrunk.png", url: "/blob/shrunk.png" }),
      } as Response);

      const alts: string[] = [];
      await vi.waitFor(() => {
        alts.length = 0;
        view.state.doc.descendants((node) => {
          if (node.type.name === "image") alts.push(String(node.attrs.alt));
          return true;
        });
        expect(alts).toEqual(["shrunk.png"]);
      });
      // Neither an upload failure nor an insertion failure was logged:
      // the upload succeeded and the image landed.
      expect(warnSpy).not.toHaveBeenCalled();
    } finally {
      view.destroy();
      container.remove();
      vi.restoreAllMocks();
    }
  });
});
