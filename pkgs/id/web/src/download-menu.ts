/**
 * Disclosure behaviour for the editor's download menu.
 *
 * The menu is a list of actions behind the "dl" toggle. Left to the old
 * `classList.toggle("show")` handler, the toggle never told assistive
 * technology whether the menu was open, Escape did nothing (so a keyboard
 * user had to click elsewhere to dismiss it), and picking a format left the
 * menu hanging open. This module is the single owner of that open/closed
 * state so all three paths agree.
 */

export interface DownloadMenuRefs {
  toggle: HTMLElement | null;
  menu: HTMLElement | null;
}

export function downloadMenuRefs(doc: Document = document): DownloadMenuRefs {
  return {
    toggle: doc.getElementById("download-btn"),
    menu: doc.getElementById("download-menu"),
  };
}

/** True when the menu is currently rendered open. */
export function isDownloadMenuOpen(doc: Document = document): boolean {
  const { menu } = downloadMenuRefs(doc);
  return menu?.classList.contains("show") ?? false;
}

/** Mirror the rendered state onto the toggle (disclosure pattern). */
function syncExpanded(toggle: HTMLElement | null, open: boolean): void {
  toggle?.setAttribute("aria-expanded", open ? "true" : "false");
}

/** Open the menu if closed, close it if open. */
export function toggleDownloadMenu(doc: Document = document): void {
  const { toggle, menu } = downloadMenuRefs(doc);
  if (!menu) return;
  const open = menu.classList.toggle("show");
  syncExpanded(toggle, open);
}

/**
 * Close the menu. Returns whether it had been open, so callers can avoid
 * stealing focus when there was nothing to dismiss. `restoreFocus` returns
 * keyboard focus to the toggle instead of letting it fall to the body when
 * the focused menu item disappears.
 */
export function closeDownloadMenu(doc: Document = document, options: { restoreFocus?: boolean } = {}): boolean {
  const { toggle, menu } = downloadMenuRefs(doc);
  if (!menu?.classList.contains("show")) return false;

  menu.classList.remove("show");
  syncExpanded(toggle, false);
  if (options.restoreFocus) toggle?.focus();
  return true;
}
