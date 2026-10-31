/** Terminal-compatible input encoding for world applications. */

type KeyEventLike = Pick<KeyboardEvent, "key" | "ctrlKey" | "altKey" | "metaKey" | "shiftKey">;

/**
 * A special (non-printable) key, described by the forms it can take.
 *
 * - `plain` is the byte sequence an unmodified press sends.
 * - `csiLetter` keys have a CSI form ending in a letter (arrows, Home, End,
 *   and Shift+Tab's `Z`): with a modifier they send `CSI 1 ; <mod> <letter>`.
 * - `csiTilde` keys have a CSI form ending in `~` (Delete, PageUp, PageDown):
 *   with a modifier they send `CSI <code> ; <mod> ~`.
 * - Keys with neither form (Enter, Tab without Shift, Backspace, Escape) have
 *   no CSI encoding for a modifier; see the policy in `terminalBytesForKey`.
 */
interface SpecialKey {
  plain: string;
  csiLetter?: string;
  csiTilde?: number;
}

const specialKeys: Record<string, SpecialKey> = {
  Enter: { plain: "\r" },
  Tab: { plain: "\t", csiLetter: "Z" },
  Backspace: { plain: "\x7f" },
  Escape: { plain: "\x1b" },
  Delete: { plain: "\x1b[3~", csiTilde: 3 },
  Home: { plain: "\x1b[H", csiLetter: "H" },
  End: { plain: "\x1b[F", csiLetter: "F" },
  PageUp: { plain: "\x1b[5~", csiTilde: 5 },
  PageDown: { plain: "\x1b[6~", csiTilde: 6 },
  ArrowUp: { plain: "\x1b[A", csiLetter: "A" },
  ArrowDown: { plain: "\x1b[B", csiLetter: "B" },
  ArrowRight: { plain: "\x1b[C", csiLetter: "C" },
  ArrowLeft: { plain: "\x1b[D", csiLetter: "D" },
};

/** xterm modifier parameter: 1 + (Shift ? 1) + (Alt ? 2) + (Ctrl ? 4). */
function modifierParameter(event: KeyEventLike): number {
  return 1 + (event.shiftKey ? 1 : 0) + (event.altKey ? 2 : 0) + (event.ctrlKey ? 4 : 0);
}

/**
 * Encode a browser key as the bytes a terminal sends for that key.
 * Returns null for browser/OS shortcuts and keys the Roc `Key` module does not
 * recognize yet. Printable input is UTF-8; control-letter input is C0.
 *
 * Modified special keys follow one policy (issue #732, Option 1 — encode the
 * modifier; no modified special key ever returns the plain key's bytes):
 *
 * - A key with a CSI form carries the standard xterm modifier parameter:
 *   Delete is `CSI 3 ; <mod> ~` (Ctrl+Delete = `ESC [ 3 ; 5 ~`), arrows,
 *   Home, and End are `CSI 1 ; <mod> <letter>` (Alt+Up = `ESC [ 1 ; 3 A`).
 *   Shift+Tab is the traditional backtab `ESC [ Z`; Shift+Tab with further
 *   modifiers is `CSI 1 ; <mod> Z`, like the other CSI letter keys.
 * - A key with no CSI form (Backspace, Enter, Escape, Tab without Shift) is
 *   ESC-prefixed when modified, the same convention the printable path below
 *   already uses for Alt+printable (Alt+Backspace = `ESC DEL`). The prefix
 *   signals "modified" even though these keys cannot distinguish which
 *   modifier was held.
 *
 * Unmodified special keys, control letters, and AltGr (Ctrl+Alt printable)
 * behaviour are unchanged.
 */
export function terminalBytesForKey(event: KeyEventLike): Uint8Array | null {
  if (event.metaKey || event.key === "Unidentified" || event.key === "Dead" || event.key === "Process") {
    return null;
  }

  if (event.key in specialKeys) {
    const special = specialKeys[event.key];
    const modified = event.ctrlKey || event.altKey || event.shiftKey;
    if (!modified) return new TextEncoder().encode(special.plain);
    const mod = modifierParameter(event);
    // Tab's CSI form exists only with Shift (backtab); a Tab modified only by
    // Ctrl and/or Alt has no CSI form and falls through to the ESC prefix.
    const hasCsiForm =
      special.csiTilde !== undefined || (special.csiLetter !== undefined && !(event.key === "Tab" && !event.shiftKey));
    if (hasCsiForm && special.csiTilde !== undefined) {
      return new TextEncoder().encode(`\x1b[${special.csiTilde};${mod}~`);
    }
    if (hasCsiForm && special.csiLetter !== undefined) {
      // Plain Shift+Tab keeps its traditional form, with no parameter.
      if (event.key === "Tab" && mod === 2) return new TextEncoder().encode("\x1b[Z");
      return new TextEncoder().encode(`\x1b[1;${mod}${special.csiLetter}`);
    }
    const plain = new TextEncoder().encode(special.plain);
    const withEscape = new Uint8Array(plain.length + 1);
    withEscape[0] = 0x1b;
    withEscape.set(plain, 1);
    return withEscape;
  }

  if (event.ctrlKey && !event.altKey) {
    if (event.key.length !== 1 || !/[a-z]/iu.test(event.key)) return null;
    return Uint8Array.of(event.key.toLowerCase().charCodeAt(0) - 96);
  }

  const key = event.key;
  const codePoints = Array.from(key);
  const printable =
    key.length === 1 || codePoints.length === 1 || (codePoints.length > 1 && /[^\p{L}\p{N}]/u.test(key));
  if (!printable || key.length === 0) return null;
  const text = new TextEncoder().encode(key);
  if (!event.altKey || event.ctrlKey) return text;
  const withEscape = new Uint8Array(text.length + 1);
  withEscape[0] = 0x1b;
  withEscape.set(text, 1);
  return withEscape;
}
