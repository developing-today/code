// When the platform's store tokens run out (#14255).
//
// Every store the platform names (pm-stack/platform/stores.json) says
// which secret holds its token. A Turso token is a JWT, so when it
// stops working is written in the token itself, in its `exp` claim:
// reading it needs no call to Turso and no other credential. A token
// is read here only long enough to find that date. Its value is never
// kept, logged, returned or drawn: only its name and when it runs out.
import type { StoreRow } from "./stores.ts";

/** A token this close to running out is worth telling the owner of. */
export const SOON_MS = 14 * 24 * 60 * 60_000;

/** How a token stands. "none": it carries no date, so it does not run
 * out by itself. "unread": it could not be fetched or is not a JWT. */
export type TokenState = "fine" | "soon" | "expired" | "none" | "unread";

export interface TokenExpiry {
  /** The name of the secret that holds it. */
  name: string;
  /** The stores that use it, by their database names. */
  stores: string[];
  /** When it runs out, or null when it does not or could not be read. */
  expiresAt: string | null;
  state: TokenState;
}

/** A name a secret can be fetched by. A store whose token is set some
 * other way (a Worker secret set by its deploy) says so in words, and
 * holds no token the platform can read by name. */
const SECRET_NAME = /^[a-z0-9][a-z0-9._-]{0,119}$/;

/** The tokens the stores name, each once, with the stores that use it,
 * in the order the list first names them. */
export function heldTokens(stores: StoreRow[]): { name: string; stores: string[] }[] {
  const byName = new Map<string, string[]>();
  for (const store of stores) {
    if (!SECRET_NAME.test(store.token)) continue;
    const using = byName.get(store.token) ?? [];
    if (!using.includes(store.name)) using.push(store.name);
    byName.set(store.token, using);
  }
  return [...byName].map(([name, using]) => ({ name, stores: using }));
}

/** When a JWT runs out, in milliseconds, read from its `exp` claim:
 * undefined when it is not a JWT (or its payload is not one), null when
 * it is one and carries no date. Never throws, and never says anything
 * of the token beyond that. */
export function jwtExpiry(token: string): number | null | undefined {
  const parts = token.trim().split(".");
  if (parts.length !== 3 || parts[1] === "" || !/^[A-Za-z0-9_-]+$/.test(parts[1])) return undefined;
  let claims: unknown;
  try {
    const b64 = parts[1].replace(/-/g, "+").replace(/_/g, "/");
    const padded = b64 + "=".repeat((4 - (b64.length % 4)) % 4);
    claims = JSON.parse(new TextDecoder().decode(Uint8Array.from(atob(padded), (c) => c.charCodeAt(0))));
  } catch {
    return undefined;
  }
  if (!claims || typeof claims !== "object" || Array.isArray(claims)) return undefined;
  const exp = (claims as { exp?: unknown }).exp;
  if (exp === undefined || exp === null) return null;
  if (typeof exp !== "number" || !Number.isFinite(exp) || exp <= 0) return undefined;
  return exp * 1000;
}

/** How a token that runs out at `expiresAt` stands at `now`. */
export function expiryState(expiresAt: number | null | undefined, now: number): TokenState {
  if (expiresAt === undefined) return "unread";
  if (expiresAt === null) return "none";
  if (expiresAt <= now) return "expired";
  if (expiresAt - now <= SOON_MS) return "soon";
  return "fine";
}

/** One token as the screen and the notifications see it. */
export function tokenExpiry(
  held: { name: string; stores: string[] },
  expiresAt: number | null | undefined,
  now: number,
): TokenExpiry {
  return {
    name: held.name,
    stores: held.stores,
    expiresAt: typeof expiresAt === "number" ? new Date(expiresAt).toISOString() : null,
    state: expiryState(expiresAt, now),
  };
}
