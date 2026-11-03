// path-segment.ts — the guarded path-segment decode (#2747/#2748).
//
// Routes that take a value from the URL path (/discuss/<ref>,
// /item/<id>) must decode it exactly once, and a malformed
// percent-encoding is client input gone wrong — a JSON 400 in the
// aggregator's established shape, never a raw URIError escaping the
// handler as an unhandled 500. This helper is the one place that
// decode happens: it wraps decodeURIComponent only (decode-once
// semantics are unchanged — %2F still decodes inside a value, and
// 100%25 still yields the literal "100%") and returns null on
// URIError so the route can answer 400 naming the bad encoding.

export function decodePathSegment(raw: string): string | null {
  try {
    return decodeURIComponent(raw);
  } catch {
    return null;
  }
}
