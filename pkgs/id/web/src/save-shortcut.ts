/**
 * Save-shortcut predicate for the id web editor (#2951).
 *
 * Matches the physical S key via `event.code === "KeyS"` instead of
 * `event.key`, so Ctrl/Cmd+S fires regardless of the character the
 * layout produces: CapsLock yields key "S", and non-Latin layouts
 * yield e.g. "ы" on the same physical key. Ctrl+Shift+S is excluded
 * so the editor's strikethrough binding keeps working.
 */

export interface SaveShortcutEvent {
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  code: string;
}

export function isSaveShortcut(event: SaveShortcutEvent): boolean {
  return (
    (event.ctrlKey || event.metaKey) &&
    !event.shiftKey &&
    event.code === "KeyS"
  );
}
