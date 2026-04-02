/**
 * Tests for image upload plugin helpers and constants.
 * Tests MIME validation, filename generation, extension mapping,
 * and ProseMirror plugin creation.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";
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
