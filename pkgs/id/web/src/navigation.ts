/**
 * SPA navigation and partial swaps, with supersession (#1462).
 *
 * `navigateTo` and `fetchPartial` issue fetches that can overlap in
 * normal use: two rapid [data-nav] clicks, a click racing popstate, a
 * debounced search racing the tags-WS refresh on the same target.
 * Responses used to be applied in ARRIVAL order, so a slow earlier
 * request overwrote the page/list the user asked for last — with the
 * URL and the content agreeing with each other, so nothing looked
 * wrong.
 *
 * Supersession rule: issuing a request bumps a sequence token — one
 * slot for navigation, one slot per target selector for partials.
 * A response (or error) may write the DOM, push history, or run swap
 * side effects only while its token is still the slot's latest;
 * a superseded outcome is dropped silently.
 *
 * Extracted from main.ts so the race is testable without importing
 * the whole shell (importing main.ts runs init()).
 */

/** The slice of the app object navigation needs (main.ts's IdApp). */
type IdAppLike = {
  collab: unknown;
  closeEditor: () => void;
};

type NavigationDeps = {
  /** Run after a partial target's content is swapped. */
  onPartialSwapped: (targetSelector: string) => void;
  /** Run after #main is swapped by a navigation. */
  onMainSwapped: () => void;
};

export type Navigation = {
  fetchPartial: (url: string, targetSelector: string) => Promise<void>;
  navigateTo: (url: string, pushUrl?: boolean) => Promise<void>;
};

export function createNavigation(deps: NavigationDeps): Navigation {
  // One navigation slot: any newer navigation supersedes all earlier ones.
  let navigationToken = 0;
  // Page generation (#1578): bumped exactly once per LANDED navigation
  // swap. fetchPartial captures the generation at issue; a partial from
  // an older generation may still write to the old page before the swap
  // lands (the swap replaces it), but never after it.
  let pageGeneration = 0;
  // One slot per partial target: a newer fetch for the SAME target
  // supersedes earlier ones; other targets are independent.
  const partialTokens = new Map<string, number>();

  /**
   * Fetch a URL and replace the innerHTML of a target element.
   * Used for partial page updates (search, pagination, tags WS refresh, peers auto-refresh).
   */
  async function fetchPartial(url: string, targetSelector: string): Promise<void> {
    const token = (partialTokens.get(targetSelector) ?? 0) + 1;
    partialTokens.set(targetSelector, token);
    const generation = pageGeneration;
    const isCurrent = (): boolean => partialTokens.get(targetSelector) === token && generation === pageGeneration;
    try {
      const response = await fetch(url, {
        headers: { "X-Partial-Request": "true" },
      });
      if (!isCurrent()) return;
      if (!response.ok) {
        console.error("[id] fetchPartial failed:", response.status, response.statusText);
        return;
      }
      const html = await response.text();
      if (!isCurrent()) return;
      const target = document.querySelector(targetSelector);
      if (target) {
        target.innerHTML = html;
        deps.onPartialSwapped(targetSelector);
      }
    } catch (err) {
      if (!isCurrent()) return;
      console.error("[id] fetchPartial error:", err);
    }
  }

  /**
   * Navigate to a URL by fetching it as a partial and swapping #main content.
   * Used by [data-nav] link clicks, programmatic navigation, and popstate.
   */
  async function navigateTo(url: string, pushUrl: boolean = true): Promise<void> {
    const token = ++navigationToken;
    const isCurrent = (): boolean => token === navigationToken;
    try {
      const response = await fetch(url, {
        headers: { "X-Partial-Request": "true" },
      });
      if (!isCurrent()) return;
      if (!response.ok) {
        console.error("[id] navigateTo failed:", response.status, response.statusText);
        // Fallback to full page navigation
        window.location.href = url;
        return;
      }
      const html = await response.text();
      if (!isCurrent()) return;
      const main = document.getElementById("main");
      if (main) {
        // Close the editor only once this navigation is known to land:
        // a superseded navigation must not close/reopen the editor for
        // a page that never arrives (#1462). The old page's DOM is
        // still in place here, so the close sees the same state it did
        // when it ran before the fetch.
        const app = (window as unknown as Record<string, IdAppLike | undefined>).idApp;
        if (app?.collab) {
          app.closeEditor();
        }
        pageGeneration++;
        main.innerHTML = html;
        if (pushUrl) {
          window.history.pushState(null, "", url);
        }
        deps.onMainSwapped();
      } else {
        // No #main element, fallback
        window.location.href = url;
      }
    } catch (err) {
      if (!isCurrent()) return;
      console.error("[id] navigateTo error:", err);
      window.location.href = url;
    }
  }

  return { fetchPartial, navigateTo };
}
