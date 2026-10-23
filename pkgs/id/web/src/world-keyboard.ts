/** Terminal-compatible input encoding for world applications. */

type KeyEventLike = Pick<KeyboardEvent, "key" | "ctrlKey" | "altKey" | "metaKey" | "shiftKey">;

const specialKeys: Record<string, string> = {
  Enter: "\r",
  Tab: "\t",
  Backspace: "\x7f",
  Escape: "\x1b",
  Delete: "\x1b[3~",
  Home: "\x1b[H",
  End: "\x1b[F",
  PageUp: "\x1b[5~",
  PageDown: "\x1b[6~",
  ArrowUp: "\x1b[A",
  ArrowDown: "\x1b[B",
  ArrowRight: "\x1b[C",
  ArrowLeft: "\x1b[D",
};

/**
 * Encode a browser key as the bytes a terminal sends for that key.
 * Returns null for browser/OS shortcuts and keys the Roc `Key` module does not
 * recognize yet. Printable input is UTF-8; control-letter input is C0.
 */
export function terminalBytesForKey(event: KeyEventLike): Uint8Array | null {
  if (event.metaKey || event.key === "Unidentified" || event.key === "Dead" || event.key === "Process") {
    return null;
  }

  if (event.key in specialKeys) {
    if ((event.ctrlKey || event.altKey || event.shiftKey) && event.key.startsWith("Arrow")) return null;
    if (event.shiftKey && event.key === "Tab") return null;
    return new TextEncoder().encode(specialKeys[event.key]);
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
