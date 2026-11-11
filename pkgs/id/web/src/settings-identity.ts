/** Display text for the client id shown on the settings page (#1792). */

/**
 * The client id the settings page should show. The in-memory identity
 * wins; the localStorage-persisted identity is the fallback (it is what
 * makes the id stable across in-site navigation). A missing id — or the
 * all-zeros placeholder the original bug report showed ("client id
 * shows 0s") — renders as "Not registered yet", never as a fake id.
 */
export function clientIdForDisplay(
  currentClientId: string | null | undefined,
  storedClientId: string | null | undefined,
): string {
  const candidate = (currentClientId ?? storedClientId ?? "").trim();
  if (!candidate || /^0+$/.test(candidate)) return "Not registered yet";
  return candidate;
}
