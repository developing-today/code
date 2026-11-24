// The pieces of the HTTP surface that are the same wherever the app is
// served from: this host, or the edge in front of it.
import type { Config } from "./config.ts";
import { type Devices, type Limiter, summary } from "./devices.ts";
import type { Docs } from "./docs.ts";

export const BASE = "/console";

/** The install manifest of the app at this origin. The home-screen icon
 * opens the app, and a long press offers its four primary screens. */
export const MANIFEST = {
  name: "PM",
  short_name: "PM",
  description: "What the fleet is doing, what is next, what needs you, and a place to say what you want.",
  // What an installed copy is known by. It is the address the app opens
  // at, said outright, so that it stays one app if that address moves.
  id: "/console/",
  start_url: "/console/",
  scope: "/",
  display: "standalone",
  background_color: "#0b0f14",
  theme_color: "#11161d",
  icons: [
    { src: "/icons/icon-192.png", sizes: "192x192", type: "image/png", purpose: "any" },
    { src: "/icons/icon-512.png", sizes: "512x512", type: "image/png", purpose: "any" },
    { src: "/icons/icon-maskable-512.png", sizes: "512x512", type: "image/png", purpose: "maskable" },
  ],
  shortcuts: [
    { name: "Ask the fleet", short_name: "Ask", url: "/console/#ask" },
    { name: "Needs me", short_name: "Needs me", url: "/console/#needs" },
    { name: "In progress", short_name: "Now", url: "/console/#now" },
    { name: "Queued next", short_name: "Next", url: "/console/#next" },
  ],
};
export const SECURITY = {
  "Content-Security-Policy":
    "default-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
  "X-Content-Type-Options": "nosniff",
  "Referrer-Policy": "same-origin",
  // This site is only ever HTTPS: a browser that has been here does not
  // try plain HTTP again for a year. (Over plain HTTP, as on the host's
  // own loopback port, browsers ignore it.)
  "Strict-Transport-Security": "max-age=31536000",
};

/** Whether a browser's copy of a file is still the file: its
 * If-None-Match names this entity tag. The comparison is the weak one
 * of RFC 9110 (a W/ prefix on either side does not matter), which is
 * the one a conditional GET uses. */
export function sameEntity(ifNoneMatch: string | null, etag: string | null): boolean {
  if (!ifNoneMatch || !etag) return false;
  const bare = (tag: string) => tag.trim().replace(/^W\//, "");
  const mine = bare(etag);
  return ifNoneMatch.split(",").some((tag) => tag.trim() === "*" || bare(tag) === mine);
}

export function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json; charset=utf-8", "Cache-Control": "no-store", ...SECURITY },
  });
}

/** The install manifest, as a response. */
export function manifest(): Response {
  return new Response(JSON.stringify(MANIFEST), {
    headers: { "Content-Type": "application/manifest+json; charset=utf-8", "Cache-Control": "no-cache", ...SECURITY },
  });
}

/** The Digital Asset Links statement that lets the Android app open
 * this origin full screen. Public by design: Android fetches it. */
export function assetLinks(cfg: Pick<Config, "androidPackage" | "androidFingerprints">): unknown[] {
  if (cfg.androidFingerprints.length === 0) return [];
  return [
    {
      relation: ["delegate_permission/common.handle_all_urls"],
      target: {
        namespace: "android_app",
        package_name: cfg.androidPackage,
        sha256_cert_fingerprints: cfg.androidFingerprints,
      },
    },
  ];
}

export interface Phones {
  devices: Devices;
  /** Tries at linking, and reads refused for a wrong token. */
  link: Limiter;
  refused: Limiter;
}

/** Sections the summary is built from, and how long a phone waits for
 * them to be read again. */
const SUMMARY_SECTIONS = ["board", "sessions", "census", "asks", "questionCount", "pipeline"];
/** Under the five seconds the phone gives a read. */
const SUMMARY_WAIT_MS = 3_500;
const DEVICE_BODY_LIMIT = 1024;

/** A phone presents the verifier for a link the owner approved. */
export async function deviceLink(
  req: Request,
  phones: Phones,
  audit: (entry: Record<string, unknown>) => Promise<void>,
): Promise<Response> {
  if (!phones.link.allow()) return json({ error: "too many tries: wait a minute" }, 429);
  if (Number(req.headers.get("content-length") ?? "0") > DEVICE_BODY_LIMIT) return json({ error: "too large" }, 413);
  let verifier: unknown;
  try {
    const text = await req.text();
    if (text.length > DEVICE_BODY_LIMIT) return json({ error: "too large" }, 413);
    verifier = (JSON.parse(text) as { verifier?: unknown } | null)?.verifier;
  } catch {
    return json({ error: "body: expected a JSON object" }, 400);
  }
  const linked = await phones.devices.link(verifier);
  if (!linked) return json({ error: "not approved" }, 404);
  await audit({
    at: new Date().toISOString(),
    who: `device:${linked.id}`,
    via: "device",
    action: "device.link",
    phase: "done",
    ok: true,
  }).catch((error) => console.error(`pm-console: audit: ${(error as Error).message}`));
  return json(linked);
}

/** A linked phone reads the counts and the top line of each list. */
export async function deviceSummary(req: Request, docs: Docs, phones: Phones): Promise<Response> {
  const header = req.headers.get("authorization") ?? "";
  const device = header.startsWith("Bearer ") ? await phones.devices.check(header.slice(7)) : null;
  if (!device) {
    // Wrong tokens are counted; a linked phone is never slowed by them.
    if (!phones.refused.allow()) return json({ error: "too many tries: wait a minute" }, 429);
    // `unlinked` tells the app to forget its token; a refusal from
    // anything else in front of this service does not carry it.
    return json({ error: "this phone is not linked", unlinked: true }, 401);
  }
  const { data, at, missing } = await docs.current(SUMMARY_SECTIONS, SUMMARY_WAIT_MS);
  // Before the queue has been read once there is nothing true to show:
  // the phone keeps what it has and asks again.
  if (missing.includes("board")) return json({ error: "not ready yet" }, 503);
  return json(summary(data, at));
}

/** A write must come from this site. A browser sends Origin on every
 * cross-site write and Sec-Fetch-Site on every request, and the sign-in
 * cookie alone must not be enough for another site to act as the owner. */
export function crossSiteWrite(req: Request): boolean {
  if (req.method === "GET" || req.method === "HEAD") return false;
  if (req.headers.get("sec-fetch-site") === "cross-site") return true;
  const origin = req.headers.get("origin");
  if (origin === null) return false;
  try {
    return new URL(origin).host !== (req.headers.get("host") ?? "");
  } catch {
    return true;
  }
}

/** A state-changing request must be one this page made: a custom header
 * a cross-site form cannot set, and an Origin that is this host. */
export function sameSiteWrite(req: Request): boolean {
  if (req.headers.get("x-pm-console") !== "1") return false;
  const origin = req.headers.get("origin");
  if (origin === null) return true;
  try {
    return new URL(origin).host === (req.headers.get("host") ?? "");
  } catch {
    return false;
  }
}

/** One of the three state documents, as a response: the tag for "you
 * already have this", and beside it the time the state was read. */
export function docResponse(
  req: Request,
  doc: { etag: string; body: string; asOf: number },
  shrink?: (req: Request, bytes: Uint8Array, headers: Headers) => Response | null,
): Response {
  const headers = new Headers({
    "Content-Type": "application/json; charset=utf-8",
    "Cache-Control": "no-cache",
    ETag: doc.etag,
    "X-As-Of": String(doc.asOf),
    ...SECURITY,
  });
  if (req.headers.get("if-none-match") === doc.etag) return new Response(null, { status: 304, headers });
  return shrink?.(req, new TextEncoder().encode(doc.body), headers) ?? new Response(doc.body, { headers });
}
