/**
 * The pinned embedding space (developing-today/code#6788).
 *
 * The Atlas corpus lives in exactly one vector space: the pinned
 * Snowflake model at full width. These constants mirror the fleet
 * search service's own pins:
 *   - model: atlas-core `DEFAULT_MODEL`
 *     (pkgs/atlas-search/crates/atlas-core/src/types.rs)
 *   - revision: `MODEL_REV` in fleet-search `serve.rs`
 *   - table/dims: the live service config's `vec_table`
 *     (`vec_snowflake_m_v2_768_f32` — 768 dimensions, full width)
 *
 * A vector from any other space (e.g. Workers AI's 384-dim
 * bge-small) is not comparable with the corpus: cosine similarity
 * across embedding spaces is meaningless. `validateEmbed` is the
 * runtime enforcement: a miss-path embedding that does not name
 * this exact space is refused, never cached, never used.
 */
export const EMBED_MODEL = "Snowflake/snowflake-arctic-embed-m-v2.0";
export const EMBED_DIMS = 768;
export const MODEL_REV = "95c2741480856aa9666782eb4afe11959938017f";
export const VEC_TABLE = "vec_snowflake_m_v2_768_f32";

/** The one ceiling this surface may ever search under. */
export const CEILING = "personal";

export interface EmbedResponse {
  model: string;
  dims: number;
  vector: number[];
}

export class SpaceError extends Error {
  /** Surfaced to callers as a bad-upstream (502) failure. */
  status = 502;
}

/**
 * Validate an untrusted `/v1/embed` response against the pinned
 * space. Oracle: the constants above; anything else is a bug or a
 * misconfigured service and must fail the request loudly.
 */
export function validateEmbed(raw: unknown): EmbedResponse {
  if (typeof raw !== "object" || raw === null) {
    throw new SpaceError("embed response is not an object");
  }
  const r = raw as Record<string, unknown>;
  if (r.model !== EMBED_MODEL) {
    throw new SpaceError(`embed model ${JSON.stringify(r.model)} is not the pinned space ${EMBED_MODEL}`);
  }
  if (r.dims !== EMBED_DIMS) {
    throw new SpaceError(`embed dims ${JSON.stringify(r.dims)} is not the pinned width ${EMBED_DIMS}`);
  }
  if (!Array.isArray(r.vector) || r.vector.length !== EMBED_DIMS) {
    throw new SpaceError("embed vector length does not match the pinned width");
  }
  for (const x of r.vector) {
    if (typeof x !== "number" || !Number.isFinite(x)) {
      throw new SpaceError("embed vector contains a non-finite value");
    }
  }
  return { model: r.model, dims: r.dims, vector: r.vector as number[] };
}
