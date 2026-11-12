import { describe, expect, it } from "vitest";
import { EMBED_DIMS, EMBED_MODEL, SpaceError, validateEmbed } from "../src/space";

function good(): { model: string; dims: number; vector: number[] } {
  const vector = new Array(EMBED_DIMS).fill(0);
  vector[0] = 1.0;
  return { model: EMBED_MODEL, dims: EMBED_DIMS, vector };
}

describe("validateEmbed (embedding-space consistency)", () => {
  it("accepts the pinned space at full width", () => {
    const out = validateEmbed(good());
    expect(out.dims).toBe(768);
    expect(out.vector).toHaveLength(768);
  });

  it("rejects a foreign model (e.g. Workers AI bge-small)", () => {
    const bad = { ...good(), model: "@cf/baai/bge-small-en-v1.5", dims: 384, vector: new Array(384).fill(0) };
    expect(() => validateEmbed(bad)).toThrow(SpaceError);
  });

  it("rejects the right model at a truncated width", () => {
    const bad = { model: EMBED_MODEL, dims: 384, vector: new Array(384).fill(0) };
    expect(() => validateEmbed(bad)).toThrow(SpaceError);
  });

  it("rejects dims/vector length disagreement and non-finite values", () => {
    const short = good();
    short.vector = short.vector.slice(0, 100);
    expect(() => validateEmbed(short)).toThrow(SpaceError);
    const nan = good();
    nan.vector[7] = Number.NaN;
    expect(() => validateEmbed(nan)).toThrow(SpaceError);
  });
});
