// Command palette (Cmd+K) + keyboard-first triage on /attention (#1275).
//
// The palette searches ONLY data the aggregator already serves: the page
// manifest (src/pages.ts) and the /api/*.json twins of the attention,
// board, overview, and roadmap pages. No new endpoints, no second data
// path. Item jumps go to the item's own URL/ref; page jumps go to the
// manifest route.
//
// The /attention keyboard layer never builds an action body itself: a
// key press submits the focused card's EXISTING .aform for that verb
// (form.requestSubmit()), so the delegated ACTION_SCRIPT handler in
// render.ts builds the POST /api/actions envelope — same clientToken,
// same author picker, same audit entry as the click path.

import { PAGES } from "./pages.ts";

export interface PaletteEntry {
  title: string;
  hint: string;
  href: string;
  kind: "page" | "item";
}

/** The manifest pages as palette entries (always available, no fetch). */
export function pageEntries(): PaletteEntry[] {
  return PAGES.map((p) => ({ title: p.title, hint: p.blurb, href: p.route, kind: "page" as const }));
}

/** "owner/repo#123" -> the GitHub issue URL; anything else -> null. */
export function refToUrl(ref: unknown): string | null {
  if (typeof ref !== "string") return null;
  const m = ref.match(/^([\w.-]+\/[\w.-]+)#(\d+)$/);
  return m ? `https://github.com/${m[1]}/issues/${m[2]}` : null;
}

function str(v: unknown): string | null {
  return typeof v === "string" && v.trim() ? v : null;
}

/**
 * Flatten one /api/*.json twin into palette entries. Tolerant walker:
 * any object with a title/name and a derivable href (url, href, route,
 * or a GitHub ref) becomes an entry. Attention items carry no URL of
 * their own, so they fall back to the /attention page itself.
 */
export function flattenItems(json: unknown, source: string, limit = 200): PaletteEntry[] {
  const out: PaletteEntry[] = [];
  const seen = new Set<string>();
  const walk = (node: unknown, depth: number): void => {
    if (out.length >= limit || node === null || typeof node !== "object" || depth > 5) return;
    if (Array.isArray(node)) {
      for (const child of node) walk(child, depth + 1);
      return;
    }
    const rec = node as Record<string, unknown>;
    const title = str(rec.title) ?? str(rec.name) ?? str(rec.label);
    if (title) {
      const href =
        str(rec.url) ??
        str(rec.href) ??
        str(rec.route) ??
        refToUrl(rec.ref) ??
        (source === "attention" ? "/attention" : null);
      if (href) {
        const key = `${href}\n${title}`;
        if (!seen.has(key)) {
          seen.add(key);
          out.push({ title, hint: source, href, kind: "item" });
        }
      }
    }
    for (const child of Object.values(rec)) walk(child, depth + 1);
  };
  walk(json, 0);
  return out;
}

/** Case-insensitive substring filter over title + hint; stable order. */
export function filterEntries(entries: PaletteEntry[], query: string, limit = 12): PaletteEntry[] {
  const q = query.trim().toLowerCase();
  if (!q) return entries.slice(0, limit);
  return entries.filter((e) => `${e.title} ${e.hint}`.toLowerCase().includes(q)).slice(0, limit);
}

/**
 * The /attention one-key map: key -> candidate form verbs, in priority
 * order. Escalate/de-escalate are tier-dependent spellings of the same
 * move; the script picks whichever verb the focused card renders.
 * "x" (dismiss) only auto-submits a form whose reason is already
 * filled (the quick-action chips); otherwise it focuses the dismiss
 * form's reason input — a required reason is never invented.
 */
export const KEY_VERBS: Record<string, string[]> = {
  s: ["snooze"],
  e: ["escalate", "de-escalate"],
  g: ["delegate"],
  x: ["dismiss"],
  m: ["mark-done"],
};

/** The footer/help one-liner listing the keys (discoverability). */
export const KEY_HINT =
  "⌘K palette · on Needs you: j/k move · s snooze · e escalate · g delegate · x dismiss · m mark done · ? all keys";

const KEY_HELP: [string, string][] = [
  ["⌘K / Ctrl+K", "Open the command palette (search pages + items, Enter jumps)"],
  ["j / k", "On Needs you: move between queue items"],
  ["s", "Snooze the focused item (default preset, same as clicking Snooze)"],
  ["e", "Escalate / de-escalate the focused item"],
  ["g", "Delegate the focused item (First Pass)"],
  ["x", "Dismiss the focused item (quick reason chip, or focus the reason box)"],
  ["m", "Mark the focused item done"],
  ["?", "Show / hide this shortcut list"],
  ["Esc", "Close the palette or this list"],
];

/** Palette + help overlay markup for the shared shell (every page). */
export function paletteHtml(): string {
  const pages = JSON.stringify(pageEntries()).replace(/</g, "\\u003c");
  const apis = JSON.stringify(
    PAGES.filter((p) => p.api && ["attention", "board", "overview", "roadmap"].includes(p.id)).map((p) => ({
      id: p.id,
      api: p.api,
    })),
  ).replace(/</g, "\\u003c");
  const help = KEY_HELP.map(([k, d]) => `<div class="cmdk-help-row"><kbd>${k}</kbd><span>${d}</span></div>`).join("");
  return `<style>
.cmdk-overlay { position: fixed; inset: 0; background: rgba(1,4,9,.6); display: none; z-index: 60; }
.cmdk-overlay.open { display: block; }
.cmdk-box { max-width: 560px; margin: 10vh auto 0; background: #161b22; border: 1px solid #30363d; border-radius: 10px; padding: 10px; }
.cmdk-box input { width: 100%; box-sizing: border-box; background: #0d1117; color: #e6edf3; border: 1px solid #30363d; border-radius: 6px; padding: 10px; font: inherit; }
.cmdk-list { margin-top: 8px; max-height: 50vh; overflow: auto; }
.cmdk-item { display: block; width: 100%; text-align: left; background: none; border: 0; color: #e6edf3; padding: 8px; border-radius: 6px; cursor: pointer; font: inherit; }
.cmdk-item.sel { background: #1f6feb; color: #fff; }
.cmdk-item .hint { color: #8b949e; font-size: 12px; margin-left: 8px; }
.cmdk-item.sel .hint { color: #dbeafe; }
.cmdk-help-row { display: flex; gap: 10px; padding: 4px 8px; color: #e6edf3; }
.cmdk-help-row kbd { min-width: 92px; color: #8b949e; }
.card.kbd-focus { outline: 2px solid #1f6feb; outline-offset: 2px; }
</style>
<div class="cmdk-overlay" id="cmdk" role="dialog" aria-modal="true" aria-label="Command palette">
  <div class="cmdk-box">
    <input id="cmdk-input" type="text" placeholder="Jump to a page or item…" aria-label="Command palette search">
    <div class="cmdk-list" id="cmdk-list"></div>
  </div>
</div>
<div class="cmdk-overlay" id="kbd-help" role="dialog" aria-modal="true" aria-label="Keyboard shortcuts">
  <div class="cmdk-box"><strong>Keyboard shortcuts</strong>${help}</div>
</div>
<script type="application/json" id="cmdk-pages">${pages}</script>
<script type="application/json" id="cmdk-apis">${apis}</script>`;
}

/**
 * The keyboard script, injected once by the shared layout. Plain JS
 * (no build step), delegated listeners only — /attention cards arrive
 * via Datastar fragments, so everything is resolved at keypress time.
 */
export const KEYBOARD_SCRIPT = `<script>
(function () {
  var pages = JSON.parse(document.getElementById("cmdk-pages").textContent);
  var apis = JSON.parse(document.getElementById("cmdk-apis").textContent);
  var items = null; // lazily flattened twins, fetched on first palette open
  var overlay = document.getElementById("cmdk");
  var input = document.getElementById("cmdk-input");
  var list = document.getElementById("cmdk-list");
  var help = document.getElementById("kbd-help");
  var shown = [];
  var sel = 0;

  function refToUrl(ref) {
    var m = typeof ref === "string" && ref.match(/^([\\w.-]+\\/[\\w.-]+)#(\\d+)$/);
    return m ? "https://github.com/" + m[1] + "/issues/" + m[2] : null;
  }
  function flatten(json, source, out) {
    (function walk(node, depth) {
      if (!node || typeof node !== "object" || depth > 5 || out.length >= 200) return;
      if (Array.isArray(node)) { node.forEach(function (c) { walk(c, depth + 1); }); return; }
      var title = node.title || node.name || node.label;
      if (typeof title === "string" && title) {
        var href = node.url || node.href || node.route || refToUrl(node.ref) || (source === "attention" ? "/attention" : null);
        if (href) out.push({ title: title, hint: source, href: href, kind: "item" });
      }
      Object.keys(node).forEach(function (k) { walk(node[k], depth + 1); });
    })(json, 0);
    return out;
  }
  function loadItems() {
    if (items) return;
    items = [];
    apis.forEach(function (a) {
      fetch(a.api).then(function (r) { return r.json(); }).then(function (j) {
        flatten(j, a.id, items);
        if (overlay.classList.contains("open")) renderList();
      }).catch(function () {});
    });
  }
  function entries() { return pages.concat(items || []); }
  function renderList() {
    var q = input.value.trim().toLowerCase();
    shown = entries().filter(function (e) {
      return !q || (e.title + " " + e.hint).toLowerCase().indexOf(q) !== -1;
    }).slice(0, 12);
    if (sel >= shown.length) sel = Math.max(0, shown.length - 1);
    list.innerHTML = "";
    shown.forEach(function (e, i) {
      var b = document.createElement("button");
      b.type = "button";
      b.className = "cmdk-item" + (i === sel ? " sel" : "");
      b.textContent = e.title;
      var h = document.createElement("span");
      h.className = "hint";
      h.textContent = e.kind === "page" ? "page · " + e.href : e.hint;
      b.appendChild(h);
      b.addEventListener("click", function () { go(e); });
      list.appendChild(b);
    });
  }
  function go(e) { if (e) location.href = e.href; }
  function openPalette() {
    loadItems();
    overlay.classList.add("open");
    input.value = "";
    sel = 0;
    renderList();
    input.focus();
  }
  function closeAll() { overlay.classList.remove("open"); help.classList.remove("open"); }
  function typingTarget(el) {
    return el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT" || el.isContentEditable);
  }

  // --- /attention item navigation + one-key actions -------------------
  var focusIdx = -1;
  function cards() {
    return Array.prototype.filter.call(document.querySelectorAll(".card"), function (c) {
      return c.querySelector(".actbox form.aform");
    });
  }
  function setFocus(i) {
    var cs = cards();
    if (!cs.length) return;
    focusIdx = Math.max(0, Math.min(i, cs.length - 1));
    cs.forEach(function (c, j) { c.classList.toggle("kbd-focus", j === focusIdx); });
    cs[focusIdx].scrollIntoView({ block: "nearest" });
  }
  function submitVerb(card, verbs, opts) {
    var forms = card.querySelectorAll("form.aform");
    for (var v = 0; v < verbs.length; v++) {
      for (var i = 0; i < forms.length; i++) {
        var f = forms[i];
        if (f.dataset.verb !== verbs[v]) continue;
        if (opts && opts.mode) {
          var mode = f.querySelector('input[name="mode"]');
          if (!mode || mode.value !== opts.mode) continue;
        }
        if (opts && opts.needHiddenReason) {
          var r = f.querySelector('input[name="reason"][type="hidden"]');
          if (!r || !r.value) continue;
        }
        var btn = f.querySelector("button[type=submit]");
        if (btn && btn.disabled) return true; // pending — a click could not send it either
        f.requestSubmit();
        return true;
      }
    }
    return false;
  }
  function actOnFocused(key) {
    var cs = cards();
    if (!cs.length) return;
    if (focusIdx < 0) setFocus(0);
    var card = cs[focusIdx];
    if (!card) return;
    if (key === "s") submitVerb(card, ["snooze"]);
    else if (key === "e") submitVerb(card, ["escalate", "de-escalate"]);
    else if (key === "g") submitVerb(card, ["delegate"], { mode: "first-pass" });
    else if (key === "m") submitVerb(card, ["mark-done"]);
    else if (key === "x") {
      if (!submitVerb(card, ["dismiss"], { needHiddenReason: true })) {
        var forms = card.querySelectorAll('form.aform[data-verb="dismiss"]');
        for (var i = 0; i < forms.length; i++) {
          var inp = forms[i].querySelector('input[name="reason"][type="text"]');
          if (inp) { inp.focus(); break; }
        }
      }
    }
  }

  document.addEventListener("keydown", function (ev) {
    var key = ev.key;
    if ((ev.metaKey || ev.ctrlKey) && (key === "k" || key === "K")) {
      ev.preventDefault();
      overlay.classList.contains("open") ? closeAll() : openPalette();
      return;
    }
    if (overlay.classList.contains("open")) {
      if (key === "Escape") closeAll();
      else if (key === "ArrowDown") { ev.preventDefault(); sel = Math.min(sel + 1, shown.length - 1); renderList(); }
      else if (key === "ArrowUp") { ev.preventDefault(); sel = Math.max(sel - 1, 0); renderList(); }
      else if (key === "Enter") { ev.preventDefault(); go(shown[sel]); }
      return;
    }
    if (help.classList.contains("open")) {
      if (key === "Escape" || key === "?") help.classList.remove("open");
      return;
    }
    if (typingTarget(ev.target)) return;
    if (key === "?") { help.classList.add("open"); return; }
    if (location.pathname !== "/attention") return;
    if (key === "j") { ev.preventDefault(); setFocus(focusIdx + 1); }
    else if (key === "k") { ev.preventDefault(); setFocus(focusIdx - 1); }
    else if (["s", "e", "g", "x", "m"].indexOf(key) !== -1) { ev.preventDefault(); actOnFocused(key); }
  });
  input.addEventListener("input", function () { sel = 0; renderList(); });
  overlay.addEventListener("click", function (ev) { if (ev.target === overlay) closeAll(); });
  help.addEventListener("click", function (ev) { if (ev.target === help) help.classList.remove("open"); });
})();
</script>`;
