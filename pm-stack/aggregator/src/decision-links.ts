// Canonical, credential-free links into the hosted read-only decision pages.
// Decision activity is owner-gated at the Worker; the set ID is not a credential.
const DECISION_PAGES_ORIGIN = "https://pm.security.cab";

export function hostedDecisionPageUrl(setId: string): string {
  return `${DECISION_PAGES_ORIGIN}/decisions?set=${encodeURIComponent(setId)}`;
}
