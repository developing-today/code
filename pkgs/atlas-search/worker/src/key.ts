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
 * KEY_SCHEMA 2 (#12622): the material gains the resolved
 * disclosure context. Keys change, so entries cached under v1
 * names are orphaned — never served again, retained per the
 * no-eviction policy (cache_do.ts).
 */
import { CEILING, MODEL_REV, VEC_TABLE, type DisclosureContext } from "./space";

export const KEY_SCHEMA = 2;

/** Trim, NFC, and collapse every whitespace run to one space. */
export function normalizeQuery(q: string): string {
  return q.normalize("NFC").trim().replace(/\s+/g, " ");
}

export interface KeyParts {
  q: string;
  /** The resolved disclosure context the answer is bound for. */
  context: DisclosureContext;
  k: number;
}

/** The canonical key material (field order is part of the key). */
export function keyMaterial(parts: KeyParts): string {
  return JSON.stringify({
    v: KEY_SCHEMA,
    q: parts.q,
    ceiling: CEILING,
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
