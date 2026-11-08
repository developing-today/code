// Provider display names for dashboard markup (#6125, Drew 2026-11-05).
//
// Rule 0 (#1823, Drew 2026-11-07): a leading underscore is a
// config/source-level ownership marker (fleet-built providers carry
// it: _claude, _antigravity, ...). Display surfaces strip it — the
// canonical id keeps it everywhere else.
//
// Rule (dashboard only, markup only): a provider whose name ends in
// "-direct" is displayed WITHOUT that suffix — but only when no
// non-direct provider with the same base name exists in the same
// provider set being rendered. When a plain base provider exists
// (a set containing both "openai" and "openai-direct"), the direct
// provider keeps its full name so the two stay distinguishable.
//
// The rule is a pure function over the actual rendered set — never a
// hardcoded provider list. It changes display strings only: provider
// ids, pool ids, hrefs, and data keys are never rewritten. A dotted
// pool/service id ("antigravity-direct.gemini") carries its provider
// as the head segment before the first ".", so the rule applies to
// that head and the remainder of the id-for-display is preserved.

const DIRECT_SUFFIX = "-direct";

/** The provider head of a display name or dotted id: the part before
 *  the first ".", or the whole string when there is no dot. */
function providerHead(name: string): string {
  const dot = name.indexOf(".");
  return dot === -1 ? name : name.slice(0, dot);
}

/**
 * Display names for a whole rendered provider set, keyed by the exact
 * input string. Compute the set ONCE per surface and look names up in
 * the returned map so every name on the surface is judged against the
 * same set.
 */
export function providerDisplayNames(names: Iterable<string>): Map<string, string> {
  const list = [...names].map((name) => {
    const head = providerHead(name);
    return head.startsWith("_") ? head.slice(1) + name.slice(head.length) : name;
  });
  const heads = new Set(list.map(providerHead));
  const out = new Map<string, string>();
  const originals = [...names];
  for (let i = 0; i < list.length; i++) {
    const name = list[i];
    const original = originals[i];
    if (out.has(original)) continue;
    const head = providerHead(name);
    if (head.endsWith(DIRECT_SUFFIX)) {
      const base = head.slice(0, -DIRECT_SUFFIX.length);
      if (base.length > 0 && !heads.has(base)) {
        out.set(original, base + name.slice(head.length));
        continue;
      }
    }
    out.set(original, name);
  }
  return out;
}

/** Display name for one provider within the rendered set `names`. */
export function providerDisplayName(name: string, names: Iterable<string>): string {
  return providerDisplayNames(names).get(name) ?? name;
}
