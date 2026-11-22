import { describe, expect, it } from "vitest";
import { decodeViewPatch } from "./screen-codec";

/**
 * Golden-vector parity with the Rust codec (id-core screen_codec,
 * #12242/#12655): GOLDEN_VIEW_PATCH_B64 is byte-for-byte the vector
 * pinned by the Rust test
 * `view_patch_matches_versioned_messagepack_golden_vector` — the
 * two suites must agree on exactly what one frame is. The oracle is
 * the MessagePack layout the Rust encoder produces. Base64 keeps
 * the literals stable under the formatter.
 */
const GOLDEN_VIEW_PATCH_B64 =
  "gqd2ZXJzaW9uAaVmcmFtZYWoc2VxdWVuY2UHrXZpZXdfcmV2aXNpb24DsmJhc2Vfdmlld19yZXZpc2lvbgKscmVzdW1lX3Rva2VuwKVwYXRjaISndmVyc2lvbgGld2lkdGgCpmhlaWdodAGkcnVuc5A=";
const GOLDEN_VIEW_PATCH = Uint8Array.from(atob(GOLDEN_VIEW_PATCH_B64), (c) => c.charCodeAt(0));

/**
 * The same envelope with a populated patch: the golden vector's
 * patch with its empty `runs` replaced by the populated runs of the
 * #12241 patch golden (cells "A" with the warning token and "水"
 * double-width). The byte splice is exact — both patches share
 * version, width, and height — and the decode assertion below pins
 * the result.
 */
const POPULATED_VIEW_PATCH_B64 =
  "gqd2ZXJzaW9uAaVmcmFtZYWoc2VxdWVuY2UHrXZpZXdfcmV2aXNpb24DsmJhc2Vfdmlld19yZXZpc2lvbgKscmVzdW1lX3Rva2VuwKVwYXRjaISndmVyc2lvbgGld2lkdGgCpmhlaWdodAGkcnVuc5GCpXN0YXJ0AKVjZWxsc5KGpnN5bWJvbKFBomZnAaJiZwKkYm9sZMOnY29sdW1ucwGldG9rZW6nd2FybmluZ4Wmc3ltYm9so+awtKJmZwCiYmcApGJvbGTCp2NvbHVtbnMC";
const POPULATED_VIEW_PATCH = Uint8Array.from(atob(POPULATED_VIEW_PATCH_B64), (c) =>
  c.charCodeAt(0),
);

describe("decodeViewPatch", () => {
  it("decodes the Rust golden view-patch frame", () => {
    expect(decodeViewPatch(GOLDEN_VIEW_PATCH)).toEqual({
      sequence: 7,
      view_revision: 3,
      base_view_revision: 2,
      resume_token: null,
      patch: { version: 1, width: 2, height: 1, runs: [] },
    });
  });

  it("decodes a populated patch: runs, cells, style tokens, wide glyphs", () => {
    expect(decodeViewPatch(POPULATED_VIEW_PATCH)).toEqual({
      sequence: 7,
      view_revision: 3,
      base_view_revision: 2,
      resume_token: null,
      patch: {
        version: 1,
        width: 2,
        height: 1,
        runs: [
          {
            start: 0,
            cells: [
              { symbol: "A", fg: 1, bg: 2, bold: true, columns: 1, token: "warning" },
              { symbol: "水", fg: 0, bg: 0, bold: false, columns: 2, token: null },
            ],
          },
        ],
      },
    });
  });

  it("rejects an unsupported codec version", () => {
    const bytes = new Uint8Array(GOLDEN_VIEW_PATCH);
    bytes[9] = 2; // the version value, as in the Rust rejection test
    expect(decodeViewPatch(bytes)).toBeNull();
  });

  it("rejects trailing bytes after the one frame value", () => {
    const bytes = new Uint8Array(GOLDEN_VIEW_PATCH.length + 1);
    bytes.set(GOLDEN_VIEW_PATCH);
    bytes[GOLDEN_VIEW_PATCH.length] = 0xc0;
    expect(decodeViewPatch(bytes)).toBeNull();
  });

  it("rejects truncated and non-MessagePack input", () => {
    expect(decodeViewPatch(GOLDEN_VIEW_PATCH.slice(0, 40))).toBeNull();
    expect(decodeViewPatch(new Uint8Array([0xc1]))).toBeNull();
    expect(decodeViewPatch(new Uint8Array(0))).toBeNull();
  });

  it("rejects the patch-only envelope (no frame fields)", () => {
    // The #12241 {version, patch} value is a different frame kind;
    // offered as a view patch it must fail safely, not decode.
    const patchOnly = Uint8Array.from(
      atob(
        "gqd2ZXJzaW9uAaVwYXRjaISmdmVyc2lvbgFld2lkdGgCpmhlaWdodAGkcnVuc5GCpXN0YXJ0AKVjZWxsc5KGpnN5bWJvbKFBomZnAaJiZwKkYm9sZMOnY29sdW1ucwGldG9rZW6nd2FybmluZ4Wmc3ltYm9so+awtKJmZwCiYmcApGJvbGTCp2NvbHVtbnMC",
      ),
      (c) => c.charCodeAt(0),
    );
    expect(decodeViewPatch(patchOnly)).toBeNull();
  });
});
