/**
 * Query normalization and Durable Object keying (#6788).
 *
 * One DO per distinct (normalized query, effective config). The
 * key material pins everything that changes what a correct answer
 * is — ceiling, disclosure context (#12622), embedding space
 * (model revision + vector table), result depth — so a cached
 * entry can never be served across a different ceiling, context,
 * or space (the same rule the search service applies to its job
 * dedupe).
 *
 * KEY_SCHEMA 3 (#8625 slice: ceilings): the material gains the
 * per-agent ceiling. Keys change, so entries cached under v2
 * names are orphaned — never served again, retained per the
 * no-eviction policy (cache_do.ts).
 */
import { isCeiling, MODEL_REV, VEC_TABLE, type Ceiling, type DisclosureContext } from "./space";

export const KEY_SCHEMA = 3;

/** Trim, NFC, and collapse every whitespace run to one space. */
export function normalizeQuery(q: string): string {
  return q.normalize("NFC").trim().replace(/\s+/g, " ");
}

export interface KeyParts {
  q: string;
  /** The ceiling this answer was computed under — part of the key so a
   * result can never be served to a caller with a lower ceiling. */
  ceiling: Ceiling;
  /** The resolved disclosure context the answer is bound for. */
  context: DisclosureContext;
  k: number;
}

/** The canonical key material (field order is part of the key). */
export function keyMaterial(parts: KeyParts): string {
  if (!isCeiling(parts.ceiling)) throw new Error(`unknown ceiling ${JSON.stringify(parts.ceiling)}`);
  return JSON.stringify({
    v: KEY_SCHEMA,
    q: parts.q,
    ceiling: parts.ceiling,
    context: parts.context,
    model_rev: MODEL_REV,
    vec_table: VEC_TABLE,
    k: parts.k,
  });
}

export async function queryKey(parts: KeyParts): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(keyMaterial(parts)));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}
