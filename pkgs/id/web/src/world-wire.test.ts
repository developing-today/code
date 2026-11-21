import { Packr } from "msgpackr";
import { describe, expect, it } from "vitest";
import { decodeBinaryViewPatch } from "./world-wire";

const packr = new Packr({ useRecords: false });

describe("binary v2 world view frames", () => {
  it("decodes the shared Rust MessagePack golden vector", () => {
    const rustGolden = new Uint8Array([
      0x87, 0xa7, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6f, 0x6e, 0x01, 0xa4, 0x74, 0x79, 0x70, 0x65, 0xa4, 0x76, 0x69,
      0x65, 0x77, 0xa8, 0x73, 0x65, 0x71, 0x75, 0x65, 0x6e, 0x63, 0x65, 0x29, 0xad, 0x76, 0x69, 0x65, 0x77,
      0x5f, 0x72, 0x65, 0x76, 0x69, 0x73, 0x69, 0x6f, 0x6e, 0x09, 0xb2, 0x62, 0x61, 0x73, 0x65, 0x5f, 0x76,
      0x69, 0x65, 0x77, 0x5f, 0x72, 0x65, 0x76, 0x69, 0x73, 0x69, 0x6f, 0x6e, 0x08, 0xac, 0x72, 0x65, 0x73,
      0x75, 0x6d, 0x65, 0x5f, 0x74, 0x6f, 0x6b, 0x65, 0x6e, 0xa8, 0x72, 0x65, 0x73, 0x75, 0x6d, 0x65, 0x2d,
      0x39, 0xa5, 0x70, 0x61, 0x74, 0x63, 0x68, 0x84, 0xa7, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6f, 0x6e, 0x01,
      0xa5, 0x77, 0x69, 0x64, 0x74, 0x68, 0x02, 0xa6, 0x68, 0x65, 0x69, 0x67, 0x68, 0x74, 0x01, 0xa4, 0x72,
      0x75, 0x6e, 0x73, 0x90,
    ]);
    expect(decodeBinaryViewPatch(rustGolden)).toEqual({
      type: "view",
      sequence: 41,
      view_revision: 9,
      base_view_revision: 8,
      resume_token: "resume-9",
      patch: { version: 1, width: 2, height: 1, runs: [] },
    });
  });

  it("decodes the view revisions and patch from the MessagePack envelope", () => {
    const frame = {
      version: 1,
      type: "view",
      sequence: 41,
      view_revision: 9,
      base_view_revision: 8,
      resume_token: "resume-9",
      patch: { version: 1, width: 2, height: 1, runs: [] },
    };

    expect(decodeBinaryViewPatch(packr.pack(frame))).toEqual({
      type: "view",
      sequence: 41,
      view_revision: 9,
      base_view_revision: 8,
      resume_token: "resume-9",
      patch: frame.patch,
    });
  });

  it("fails closed for unsupported, malformed, or non-view frames", () => {
    expect(
      decodeBinaryViewPatch(
        packr.pack({
          version: 2,
          type: "view",
          sequence: 1,
          view_revision: 1,
          base_view_revision: 0,
          patch: {},
        }),
      ),
    ).toBeNull();
    expect(
      decodeBinaryViewPatch(
        packr.pack({
          version: 1,
          type: "event",
          sequence: 1,
          view_revision: 1,
          base_view_revision: 0,
          patch: {},
        }),
      ),
    ).toBeNull();
    expect(decodeBinaryViewPatch(new Uint8Array([0xc1]))).toBeNull();
    const validPrefix = packr.pack({ version: 1, type: "view" });
    expect(decodeBinaryViewPatch(new Uint8Array([...validPrefix, 0xc0]))).toBeNull();
  });

  it("rejects frames larger than the Rust codec limit", () => {
    expect(decodeBinaryViewPatch(new Uint8Array(1024 * 1024 + 1))).toBeNull();
  });
});
