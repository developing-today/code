/**
 * ProseMirror editor setup for collaborative document editing.
 * Uses prosemirror-example-setup for baseline functionality.
 *
 * Supports multiple content modes:
 * - "rich" / "markdown" / "plain" - Full editor with toolbar
 * - "raw" - Minimal editor for code/config (no toolbar, no formatting shortcuts)
 * - "media" / "binary" - Not editable (handled elsewhere)
 */

import { collab, getVersion, sendableSteps } from "prosemirror-collab";
import { baseKeymap, toggleMark } from "prosemirror-commands";
import { dropCursor } from "prosemirror-dropcursor";
import { buildMenuItems, exampleSetup } from "prosemirror-example-setup";
import { gapCursor } from "prosemirror-gapcursor";
import { history } from "prosemirror-history";
import { keymap } from "prosemirror-keymap";
import { MenuItem, blockTypeItem, liftItem, redoItem, selectParentNodeItem, undoItem } from "prosemirror-menu";
import { Fragment, type Mark, type MarkType, Node, Schema, Slice } from "prosemirror-model";
import { schema as basicSchema } from "prosemirror-schema-basic";
import { addListNodes } from "prosemirror-schema-list";
import { EditorState, type Plugin, Selection, TextSelection, type Transaction } from "prosemirror-state";
import { ReplaceStep } from "prosemirror-transform";
import { EditorView } from "prosemirror-view";
import { createActiveLinePlugin } from "./active-line";
import { createCursorPlugin, type SendCursorFn } from "./cursors";
import { createGotoLinePlugin } from "./goto-line";
import { createSyntaxHighlightPlugin } from "./highlight";
import { openImageBrowser } from "./image-browser";
import { ImageNodeView } from "./image-node-view";
import { createImageUploadPlugin } from "./image-upload";
import { createIndentPlugin } from "./indent";
import { createSearchPlugins } from "./search-panel";
import { createWrapPlugins } from "./wrap";
import { MAX_STEPS_MESSAGE_BYTES, stepWireSize } from "./collab-step-budget";

/**
 * Content mode types matching server-side enum.
 * Determines how the editor is configured.
 */
export type ContentMode = "rich" | "markdown" | "plain" | "raw" | "media" | "binary";

/**
 * Parse an <ol> `start` attribute into the ordered_list `order` attr
 * (#2005). Only a plain non-negative decimal integer that fits in a
 * safe integer is preserved (0 included); anything else — malformed
 * text, fractions, negatives, hex/scientific notation, empty, or an
 * unrepresentable magnitude — falls back to 1. The bare Number() this
 * replaces let NaN/fractional/reinterpreted starts through, and the
 * markdown codec (which reads order as u64) then silently renumbered
 * pasted lists from 1 on save.
 */
export function parseOrderedListStart(raw: string | null): number {
  if (raw === null) return 1;
  const trimmed = raw.trim();
  if (!/^\d+$/.test(trimmed)) return 1;
  const n = Number(trimmed);
  return Number.isSafeInteger(n) ? n : 1;
}

/**
 * Parse an <img> `width`/`height` attribute into the image node's
 * width/height attr (#2010). Only a plain non-negative decimal integer
 * that fits in a safe integer is preserved (0 included); anything else
 * — malformed text, fractions, negatives, units/percentages, hex or
 * scientific notation, empty, an absent attribute, or an
 * unrepresentable magnitude — is dropped to null at parse time. The
 * bare Number() this replaces let NaN through and reinterpreted values
 * like "0x10" as 16, while the markdown codec reads dimensions as u64,
 * so invalid dimensions were silently dropped (or invented) on save
 * instead of being rejected where the paste happened.
 */
export function parseImageDimension(raw: string | null): number | null {
  if (raw === null) return null;
  const trimmed = raw.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const n = Number(trimmed);
  return Number.isSafeInteger(n) ? n : null;
}

/**
 * Full schema with list support and GFM extensions for rich/markdown/plain modes.
 * Extends prosemirror-schema-basic with strikethrough mark, task list nodes, and table nodes.
 */
export const richSchema = new Schema({
  nodes: addListNodes(basicSchema.spec.nodes, "paragraph block*", "block")
    .update("code_block", {
      // Inherited prosemirror-schema-basic fields, preserved unchanged:
      content: "text*",
      marks: "",
      group: "block",
      code: true,
      defining: true,
      parseDOM: [{ tag: "pre", preserveWhitespace: "full" }],
      toDOM() {
        return ["pre", ["code", 0]];
      },
      // #1436: mirror the markdown codec's code_block attrs into the
      // editor schema, following the #880 image linkHref / #881 table
      // alignments mirror pattern. The Rust codec (markdown.rs) sets
      // attrs {params: html, htmlBlock: true} for HTML blocks and
      // {params: <info>} for authored fences, and on save reads
      // attrs.params and requires attrs.htmlBlock == true to emit an
      // HtmlBlock. Without these declared here, Node.fromJSON drops
      // both attrs on editor load: HTML blocks re-fence as an html
      // fence (the #982 regression path) and authored fences lose
      // their language/info string.
      attrs: {
        params: { default: null },
        htmlBlock: { default: null },
      },
    })
    .update("bullet_list", {
      // Inherited prosemirror-schema-list fields, preserved unchanged:
      content: "list_item+",
      group: "block",
      parseDOM: [{ tag: "ul" }],
      toDOM() {
        return ["ul", 0];
      },
      // #1469: mirror the markdown codec's bullet_list tight attr into
      // the editor schema, following the #1036 task_list mirror. The
      // Rust codec (markdown.rs) emits attrs {tight} for bullet lists
      // and on save reads attrs.tight (missing -> false/loose). Without
      // it declared here, Node.fromJSON drops tight on editor load and
      // every tight bullet list saves as a loose list.
      attrs: {
        tight: { default: true },
      },
    })
    .update("ordered_list", {
      // Inherited prosemirror-schema-list fields, preserved unchanged:
      content: "list_item+",
      group: "block",
      parseDOM: [
        {
          tag: "ol",
          getAttrs(dom) {
            const el = dom as HTMLElement;
            return { order: parseOrderedListStart(el.getAttribute("start")) };
          },
        },
      ],
      toDOM(node) {
        return node.attrs.order === 1 ? ["ol", 0] : ["ol", { start: node.attrs.order }, 0];
      },
      // #1469: mirror the markdown codec's ordered_list tight attr,
      // keeping the inherited order attr (default and validation)
      // unchanged. The codec emits attrs {order, tight} and on save
      // reads attrs.tight (missing -> false/loose); undeclared, tight
      // was dropped on editor load and tight ordered lists saved loose.
      attrs: {
        order: { default: 1, validate: "number" },
        tight: { default: true },
        // #1380: mirror the codec's delimiter attr ("period"/"paren",
        // default period) so paren-style lists are not rewritten on
        // save; undeclared, it was dropped on editor load.
        delimiter: { default: "period" },
      },
    })
    .update("image", {
      inline: true,
      group: "inline",
      draggable: true,
      attrs: {
        src: {},
        alt: { default: null },
        title: { default: null },
        width: { default: null },
        height: { default: null },
        // #880: a linked image ([![alt](src)](href)) carries its link on
        // the image node; the markdown codec reads/writes these attrs.
        linkHref: { default: null },
        linkTitle: { default: null },
      },
      parseDOM: [
        {
          tag: "img[src]",
          getAttrs(dom) {
            const el = dom as HTMLImageElement;
            // #893: a linked image arrives wrapped in an anchor
            // (<a href="..."><img ...></a>) — read the link from the
            // wrapping <a>, or it is dropped on parse/paste.
            const anchor = el.closest("a");
            return {
              src: el.getAttribute("src"),
              alt: el.getAttribute("alt"),
              title: el.getAttribute("title"),
              width: parseImageDimension(el.getAttribute("width")),
              height: parseImageDimension(el.getAttribute("height")),
              linkHref: anchor?.getAttribute("href") ?? null,
              linkTitle: anchor?.getAttribute("title") ?? null,
            };
          },
        },
      ],
      toDOM(node) {
        const attrs: Record<string, string> = { src: node.attrs.src };
        if (node.attrs.alt) attrs.alt = node.attrs.alt;
        if (node.attrs.title) attrs.title = node.attrs.title;
        if (node.attrs.width) attrs.width = String(node.attrs.width);
        if (node.attrs.height) attrs.height = String(node.attrs.height);
        if (node.attrs.linkHref) {
          const linkAttrs: Record<string, string> = { href: node.attrs.linkHref };
          if (node.attrs.linkTitle) linkAttrs.title = node.attrs.linkTitle;
          return ["a", linkAttrs, ["img", attrs]];
        }
        return ["img", attrs];
      },
    })
    .append({
      task_list: {
        group: "block",
        content: "task_list_item+",
        // #1036: tight/loose carried for the markdown codec, mirroring
        // the #881 table-alignments pattern; absent/loose rendering
        // depends on it so multi-block task items do not fuse on save.
        // #997: ordered task lists carry their start in `order` (null for
        // bullet task lists), mirroring the markdown codec's attrs.order.
        // #1380: ordered task lists also carry `delimiter`, mirroring
        // the codec (default period, as in ordered_list above).
        attrs: {
          tight: { default: true },
          order: { default: null },
          delimiter: { default: "period" },
        },
        parseDOM: [
          {
            tag: "ul",
            getAttrs(dom) {
              // Match <ul> that contains task list items (with checkboxes)
              const el = dom as HTMLElement;
              if (el.classList.contains("contains-task-list")) return {};
              // Also match if first child li has a checkbox
              const firstLi = el.querySelector("li");
              if (firstLi?.querySelector('input[type="checkbox"]')) return {};
              return false;
            },
          },
          {
            tag: "ol",
            getAttrs(dom) {
              // Match <ol class=contains-task-list> (ordered task list);
              // its start is the list's order, read through the same
              // validated parse as ordered_list (#4100; the raw el.start
              // reflection admits fractional/negative/hex starts the u64
              // markdown codec cannot represent).
              const el = dom as HTMLOListElement;
              if (el.classList.contains("contains-task-list"))
                return { order: parseOrderedListStart(el.getAttribute("start")) };
              const firstLi = el.querySelector("li");
              if (firstLi?.querySelector('input[type="checkbox"]'))
                return { order: parseOrderedListStart(el.getAttribute("start")) };
              return false;
            },
          },
        ],
        toDOM(node) {
          if (node.attrs.order != null) {
            const attrs: Record<string, string> = { class: "contains-task-list" };
            if (node.attrs.order !== 1) attrs.start = String(node.attrs.order);
            return ["ol", attrs, 0];
          }
          return ["ul", { class: "contains-task-list" }, 0];
        },
      },
      task_list_item: {
        content: "paragraph block*",
        defining: true,
        attrs: { checked: { default: false } },
        parseDOM: [
          {
            tag: "li",
            getAttrs(dom) {
              const el = dom as HTMLElement;
              const checkbox = el.querySelector('input[type="checkbox"]');
              if (!checkbox) return false;
              return { checked: (checkbox as HTMLInputElement).checked };
            },
          },
        ],
        toDOM(node) {
          return [
            "li",
            { class: `task-list-item${node.attrs.checked ? " task-list-item-checked" : ""}` },
            [
              "input",
              {
                type: "checkbox",
                ...(node.attrs.checked ? { checked: "" } : {}),
                // Note: actual toggle is handled by nodeView
              },
            ],
            ["div", { class: "task-list-item-content" }, 0],
          ];
        },
      },
      table: {
        content: "table_row+",
        tableRole: "table",
        group: "block",
        isolating: true,
        // #881: column alignments (["left"|"center"|"right"|"none", ...])
        // carried for the markdown codec; absent = unaligned.
        attrs: { alignments: { default: null } },
        parseDOM: [{ tag: "table" }],
        toDOM() {
          return ["table", { class: "pm-table" }, ["tbody", 0]];
        },
      },
      table_row: {
        content: "(table_cell | table_header)+",
        tableRole: "row",
        parseDOM: [{ tag: "tr" }],
        toDOM() {
          return ["tr", 0];
        },
      },
      table_cell: {
        content: "paragraph+",
        tableRole: "cell",
        isolating: true,
        parseDOM: [{ tag: "td" }],
        toDOM() {
          return ["td", 0];
        },
      },
      table_header: {
        content: "paragraph+",
        tableRole: "header_cell",
        isolating: true,
        parseDOM: [{ tag: "th" }],
        toDOM() {
          return ["th", 0];
        },
      },
      // #943: raw inline HTML (e.g. <sub>, <sup>, <u>, <kbd>) as an atom
      // node carrying the tag verbatim in `html`; the markdown codec
      // reads/writes it. Display shows the tag as inert text — the attr
      // is never rendered via innerHTML or as live markup.
      // #1004: unused link reference definitions, carried for the
      // markdown codec as an atom block with label/url/title attrs
      // (mirrors the codec's link_reference_definition node, following
      // the #943 html_inline mirror pattern). Undeclared, Node.fromJSON
      // would drop the node on editor load and the definition would
      // be silently deleted on save. Rendered as inert text — the
      // attrs are never rendered as live markup.
      link_reference_definition: {
        group: "block",
        atom: true,
        selectable: true,
        attrs: {
          label: { default: "" },
          url: { default: "" },
          title: { default: null },
        },
        parseDOM: [{ tag: "div[data-link-ref-def]" }],
        toDOM(node) {
          const title = node.attrs.title ? ` "${node.attrs.title as string}"` : "";
          return [
            "div",
            { "data-link-ref-def": "" },
            `[${node.attrs.label as string}]: ${node.attrs.url as string}${title}`,
          ];
        },
      },
      html_inline: {
        inline: true,
        group: "inline",
        atom: true,
        selectable: true,
        attrs: { html: { default: "" } },
        parseDOM: [
          {
            tag: "span[data-html-inline]",
            getAttrs(dom) {
              return { html: (dom as HTMLElement).dataset.htmlInline ?? "" };
            },
          },
        ],
        toDOM(node) {
          return ["span", { "data-html-inline": node.attrs.html as string }, node.attrs.html as string];
        },
      },
      // #1369: YAML front matter block (`---` delimiter only; TOML
      // `+++` is out of scope) carried verbatim in `text`; the markdown
      // codec reads/writes it. Atom leaf, never a heading/paragraph.
      front_matter: {
        group: "block",
        atom: true,
        selectable: false,
        attrs: { text: { default: "" } },
        parseDOM: [
          {
            tag: "pre[data-front-matter]",
            getAttrs(dom) {
              return { text: (dom as HTMLElement).dataset.frontMatter ?? "" };
            },
          },
        ],
        toDOM(node) {
          return ["pre", { "data-front-matter": node.attrs.text as string }];
        },
      },
      // #1347: footnote reference atom carrying `label`; the markdown
      // codec reads/writes it (markdown.rs). Display shows the
      // reference as inert text — never rendered as live markup.
      footnote_reference: {
        inline: true,
        group: "inline",
        atom: true,
        selectable: true,
        attrs: { label: { default: "" } },
        parseDOM: [
          {
            tag: "span[data-footnote-reference]",
            getAttrs(dom) {
              return { label: (dom as HTMLElement).dataset.footnoteReference ?? "" };
            },
          },
        ],
        toDOM(node) {
          const label = node.attrs.label as string;
          return ["span", { "data-footnote-reference": label }, `[^${label}]`];
        },
      },
      // #1347: footnote definition block carrying `label` plus block
      // children (paragraphs, lists, code blocks per #1031); the
      // markdown codec reads/writes it. The label travels in the data
      // attribute only — children render normally.
      footnote_definition: {
        group: "block",
        content: "block+",
        defining: true,
        attrs: { label: { default: "" } },
        parseDOM: [
          {
            tag: "div[data-footnote-definition]",
            getAttrs(dom) {
              return { label: (dom as HTMLElement).dataset.footnoteDefinition ?? "" };
            },
          },
        ],
        toDOM(node) {
          return ["div", { "data-footnote-definition": node.attrs.label as string }, 0];
        },
      },
    }),
  marks: basicSchema.spec.marks.append({
    strikethrough: {
      parseDOM: [
        { tag: "s" },
        { tag: "del" },
        { tag: "strike" },
        {
          style: "text-decoration",
          getAttrs: (value) => (value === "line-through" ? null : false),
        },
      ],
      toDOM() {
        return ["s", 0];
      },
    },
  }),
});

/**
 * Minimal schema for raw mode (code/config files).
 * Only allows doc containing code_block nodes with text.
 * No marks (formatting) allowed.
 */
export const rawSchema = new Schema({
  nodes: {
    doc: { content: "code_block+" },
    text: { group: "inline" },
    code_block: {
      content: "text*",
      marks: "",
      group: "block",
      code: true,
      defining: true,
      attrs: {
        language: { default: null },
      },
      parseDOM: [{ tag: "pre", preserveWhitespace: "full" }],
      toDOM(node) {
        const attrs: Record<string, string> = {};
        if (node.attrs.language) attrs["data-language"] = node.attrs.language as string;
        return ["pre", attrs, ["code", 0]];
      },
    },
  },
  marks: {},
});

// Export both schemas and a helper to get the right one
export { richSchema as schema };

/**
 * Get the schema for a content mode.
 */
export function getSchema(mode: ContentMode): Schema {
  return mode === "raw" ? rawSchema : richSchema;
}

/**
 * Check if a mode uses the full editor with toolbar.
 */
export function hasToolbar(mode: ContentMode): boolean {
  return mode === "rich" || mode === "markdown" || mode === "plain";
}

/**
 * Check if a mode is editable.
 */
export function isEditable(mode: ContentMode): boolean {
  return mode !== "media" && mode !== "binary";
}

export interface EditorInstance {
  view: EditorView;
  schema: Schema;
  clientID: number;
  mode: ContentMode;
}

export interface CollabState {
  version: number;
  unconfirmed: Transaction[];
}

/**
 * Create a menu item that toggles a mark on the current selection.
 * Used for inline formatting buttons (strikethrough, etc.).
 */
function markMenuItem(markType: MarkType, options: { title: string; label: string }): InstanceType<typeof MenuItem> {
  const cmd = toggleMark(markType);
  return new MenuItem({
    title: options.title,
    label: options.label,
    run(state, dispatch) {
      cmd(state, dispatch);
    },
    select(state) {
      return cmd(state);
    },
    active(state) {
      const { from, $from, to, empty } = state.selection;
      if (empty) return !!markType.isInSet(state.storedMarks || $from.marks());
      return state.doc.rangeHasMark(from, to, markType);
    },
  });
}

type TextRun = { text: string; marks: readonly Mark[] };

/**
 * Turn a single oversized, closed text replacement into smaller replace
 * steps before the collab plugin sees it. Keeping these steps in one
 * transaction preserves normal history grouping and selection mapping.
 */
export function splitOversizedTextInsertion(state: EditorState, transaction: Transaction): Transaction {
  if (transaction.steps.length !== 1 || transaction.getMeta("addToHistory") === false) return transaction;

  const step = transaction.steps[0];
  if (!(step instanceof ReplaceStep) || step.slice.openStart !== 0 || step.slice.openEnd !== 0) return transaction;
  if (!(transaction.selection instanceof TextSelection)) return transaction;
  if (stepWireSize(step.toJSON()) <= MAX_STEPS_MESSAGE_BYTES) return transaction;

  const runs: TextRun[] = [];
  let textLength = 0;
  let textOnly = step.slice.content.childCount > 0;
  step.slice.content.forEach((node) => {
    if (!node.isText || typeof node.text !== "string") {
      textOnly = false;
      return;
    }
    runs.push({ text: node.text, marks: node.marks });
    textLength += node.text.length;
  });
  if (!textOnly || textLength === 0) return transaction;

  const text = runs.map((run) => run.text).join("");
  const sliceBetween = (from: number, to: number): Slice => {
    const nodes: Node[] = [];
    let offset = 0;
    for (const run of runs) {
      const runEnd = offset + run.text.length;
      const start = Math.max(from, offset);
      const end = Math.min(to, runEnd);
      if (start < end) nodes.push(state.schema.text(run.text.slice(start - offset, end - offset), run.marks));
      offset = runEnd;
      if (offset >= to) break;
    }
    return new Slice(Fragment.fromArray(nodes), 0, 0);
  };
  const safeBoundary = (position: number): number => {
    if (position > 0 && position < text.length) {
      const before = text.charCodeAt(position - 1);
      const after = text.charCodeAt(position);
      if (before >= 0xd800 && before <= 0xdbff && after >= 0xdc00 && after <= 0xdfff) return position - 1;
    }
    return position;
  };

  const replacement = state.tr;
  let start = 0;
  while (start < text.length) {
    let low = start + 1;
    let high = text.length;
    let best = start;
    while (low <= high) {
      const end = safeBoundary(Math.floor((low + high) / 2));
      if (end <= start) {
        low = Math.floor((low + high) / 2) + 1;
        continue;
      }
      const from = step.from + start;
      const to = start === 0 ? step.to : from;
      const candidate = new ReplaceStep(from, to, sliceBetween(start, end));
      if (stepWireSize(candidate.toJSON()) <= MAX_STEPS_MESSAGE_BYTES) {
        best = end;
        low = Math.floor((low + high) / 2) + 1;
      } else {
        high = Math.floor((low + high) / 2) - 1;
      }
    }
    if (best === start) return transaction;

    const from = step.from + start;
    const to = start === 0 ? step.to : from;
    replacement.step(new ReplaceStep(from, to, sliceBetween(start, best)));
    start = best;
  }

  // Fail closed if the narrow rewrite did not produce precisely the same
  // document. All generated steps must be within the shared outbound budget.
  if (!replacement.doc.eq(transaction.doc)) return transaction;
  if (replacement.steps.some((candidate) => stepWireSize(candidate.toJSON()) > MAX_STEPS_MESSAGE_BYTES)) {
    return transaction;
  }

  replacement.setSelection(Selection.fromJSON(replacement.doc, transaction.selection.toJSON()));
  const transactionMeta = (transaction as unknown as { meta: Record<string, unknown> }).meta;
  for (const [key, value] of Object.entries(transactionMeta)) replacement.setMeta(key, value);
  if (transaction.storedMarksSet) replacement.setStoredMarks(transaction.storedMarks);
  replacement.setTime(transaction.time);
  if (transaction.scrolledIntoView) replacement.scrollIntoView();
  return replacement;
}

/**
 * Initialize a ProseMirror editor in the given container.
 *
 * @param container - The DOM element to mount the editor in
 * @param initialDoc - Optional initial document as ProseMirror JSON
 * @param collabVersion - Starting version for collaboration (default 0)
 * @param mode - Content mode determining schema and plugins
 * @param sendCursor - Optional callback to send cursor updates
 * @param filename - Optional filename for syntax highlighting language detection
 * @returns The editor instance
 */
export function initEditor(
  container: HTMLElement,
  initialDoc?: unknown,
  collabVersion: number = 0,
  mode: ContentMode = "raw",
  sendCursor?: SendCursorFn,
  filename?: string,
): EditorInstance {
  console.log("[editor] initEditor called with mode:", mode, "collabVersion:", collabVersion);
  console.log("[editor] initialDoc:", initialDoc ? JSON.stringify(initialDoc).slice(0, 300) : "undefined");

  // Generate a random client ID for this session
  const clientID = Math.floor(Math.random() * 0xffffffff);

  // Select schema based on mode
  const editorSchema = getSchema(mode);
  console.log("[editor] Using schema for mode:", mode, "hasToolbar:", hasToolbar(mode));

  // Parse initial document from JSON if provided, otherwise create empty
  let doc: Node;
  if (initialDoc && typeof initialDoc === "object") {
    try {
      console.log("[editor] Parsing initialDoc with Node.fromJSON");
      doc = Node.fromJSON(editorSchema, initialDoc);
      console.log("[editor] Parsed doc successfully, content:", doc.toString().slice(0, 200));
    } catch (err) {
      console.error("[editor] Failed to parse initial doc JSON, using empty doc:", err);
      doc = editorSchema.topNodeType.createAndFill() ?? editorSchema.node("doc");
    }
  } else {
    console.log("[editor] No initialDoc, creating empty doc");
    doc = editorSchema.topNodeType.createAndFill() ?? editorSchema.node("doc");
  }

  // Build plugins list based on mode
  const plugins: Plugin[] = [];

  if (hasToolbar(mode)) {
    // Build custom menu that flattens the Type dropdown into inline buttons
    const menuItems = buildMenuItems(editorSchema);

    // Filter out null/undefined items
    const cut = <T>(arr: (T | null | undefined)[]): T[] => arr.filter((x): x is T => x != null);

    // Create compact block type items with short labels
    const paragraph = editorSchema.nodes.paragraph;
    const codeBlock = editorSchema.nodes.code_block;
    const heading = editorSchema.nodes.heading;

    const makeParagraph =
      paragraph &&
      blockTypeItem(paragraph, {
        title: "Change to paragraph",
        label: "¶",
      });
    const makeCodeBlock =
      codeBlock &&
      blockTypeItem(codeBlock, {
        title: "Change to code block",
        label: "</>",
      });

    // Create heading items H1-H6 with compact labels
    const makeHeadings = heading
      ? [1, 2, 3, 4, 5, 6].map((level) =>
          blockTypeItem(heading, {
            title: `Change to heading ${level}`,
            label: `H${level}`,
            attrs: { level },
          }),
        )
      : [];

    // Create strikethrough menu item
    const strikethrough = editorSchema.marks.strikethrough;
    const toggleStrikethrough = strikethrough
      ? markMenuItem(strikethrough, {
          title: "Toggle strikethrough (Mod-Shift-s)",
          label: "~~S~~",
        })
      : null;

    // Create "Browse images" menu item
    const browseImages = new MenuItem({
      title: "Browse uploaded images",
      label: "🖼",
      run(state, _dispatch, edView) {
        openImageBrowser(edView, editorSchema);
      },
    });

    // Build flattened menu structure:
    // Row 1: inline formatting (bold, italic, code, strikethrough, link)
    // Row 2: block types (paragraph, code, H1-H6) + undo/redo
    // Row 3: lists, blockquote, structure tools
    const customMenu = [
      // Inline formatting
      cut([
        menuItems.toggleStrong,
        menuItems.toggleEm,
        menuItems.toggleCode,
        toggleStrikethrough,
        menuItems.toggleLink,
        menuItems.insertImage,
        browseImages,
      ]),
      // Block types flattened + undo/redo
      cut([makeParagraph, makeCodeBlock, ...makeHeadings, undoItem, redoItem]),
      // Block structure tools
      cut([
        menuItems.wrapBulletList,
        menuItems.wrapOrderedList,
        menuItems.wrapBlockQuote,
        liftItem,
        selectParentNodeItem,
      ]),
    ];

    plugins.push(
      ...exampleSetup({
        schema: editorSchema,
        menuContent: customMenu,
      }),
    );

    // Strikethrough keyboard shortcut (Mod-Shift-s)
    if (strikethrough) {
      plugins.push(keymap({ "Mod-Shift-s": toggleMark(strikethrough) }));
    }
  } else {
    // Minimal setup for raw mode - just basic editing, no menu/toolbar
    plugins.push(history(), dropCursor(), gapCursor(), keymap(baseKeymap));
  }

  // Always add collab plugin
  plugins.push(collab({ version: collabVersion, clientID }));

  // Add syntax highlighting + line numbers for code_block nodes
  plugins.push(createSyntaxHighlightPlugin({ filename, lineNumbers: true }));

  // Add word wrap toggle (default: ON, toggle with Alt+Z)
  plugins.push(...createWrapPlugins({ defaultEnabled: true }));

  // Add line number toggle (Alt+L) — shown by default, CSS class hides them
  plugins.push(
    keymap({
      "Alt-l": (_state, _dispatch, view) => {
        if (view) {
          view.dom.classList.toggle("id-editor-no-line-numbers");
        }
        return true;
      },
    }),
  );

  // Add active line highlight
  plugins.push(createActiveLinePlugin());

  // Add find/replace (Ctrl+F / Ctrl+H)
  plugins.push(...createSearchPlugins());

  // Add Go to Line (Ctrl+G)
  plugins.push(createGotoLinePlugin());

  // Add Tab/Shift+Tab indentation for code blocks
  plugins.push(createIndentPlugin());

  // Add image paste/drop upload (only for schemas with image node)
  const imageUploadPlugin = createImageUploadPlugin(editorSchema);
  if (imageUploadPlugin) {
    plugins.push(imageUploadPlugin);
  }

  // Add cursor plugin if sendCursor callback provided
  if (sendCursor) {
    plugins.push(createCursorPlugin(clientID, sendCursor));
  }

  // Create editor state
  const state = EditorState.create({
    doc,
    plugins,
  });

  // Add mode-specific CSS class
  const editorClass = hasToolbar(mode) ? "id-editor id-editor-rich" : "id-editor id-editor-raw";

  // Create editor view
  const view = new EditorView(container, {
    state,
    nodeViews: {
      // Custom nodeView for task_list_item: renders a clickable checkbox
      task_list_item(node, outerView, getPos) {
        const li = document.createElement("li");
        li.classList.add("task-list-item");
        if (node.attrs.checked) li.classList.add("task-list-item-checked");

        const checkbox = document.createElement("input");
        checkbox.type = "checkbox";
        checkbox.checked = !!node.attrs.checked;
        checkbox.contentEditable = "false";
        checkbox.addEventListener("change", () => {
          const pos = typeof getPos === "function" ? getPos() : null;
          if (pos != null) {
            outerView.dispatch(
              outerView.state.tr.setNodeMarkup(pos, undefined, {
                ...node.attrs,
                checked: checkbox.checked,
              }),
            );
          }
        });

        const content = document.createElement("div");
        content.classList.add("task-list-item-content");

        li.appendChild(checkbox);
        li.appendChild(content);

        return {
          dom: li,
          contentDOM: content,
          update(updatedNode) {
            if (updatedNode.type !== node.type) return false;
            checkbox.checked = !!updatedNode.attrs.checked;
            li.classList.toggle("task-list-item-checked", !!updatedNode.attrs.checked);
            node = updatedNode;
            return true;
          },
        };
      },
      // Custom nodeView for images: alt-text popover + resize handles
      image(node: Node, imgView: EditorView, getPos: () => number | undefined) {
        return new ImageNodeView(node, imgView, getPos);
      },
    },
    dispatchTransaction(transaction: Transaction) {
      transaction = splitOversizedTextInsertion(view.state, transaction);
      const newState = view.state.apply(transaction);
      view.updateState(newState);

      // Dispatch custom event for collab sync
      // Only for LOCAL changes - remote changes have 'addToHistory' set to false by receiveTransaction
      // We also check that the transaction wasn't created by the collab plugin itself
      const isLocalChange = transaction.docChanged && transaction.getMeta("addToHistory") !== false;

      if (isLocalChange) {
        console.log("[editor] Local document change, dispatching editor:change event");
        const event = new CustomEvent("editor:change", {
          detail: { transaction, state: newState },
          bubbles: true,
        });
        container.dispatchEvent(event);
      } else if (transaction.docChanged) {
        console.log("[editor] Remote document change (from collab), not dispatching event");
      }
    },
    handleKeyDown(view, event) {
      // Fix for cursor jumping 2 lines when pressing Up at start of visual line.
      // The browser's native caret can be positioned at end of previous line
      // (visually same as start of current line), causing Up to move from there.
      // We detect this case and manually compute the correct target position.
      if (event.key === "ArrowUp") {
        const { $head } = view.state.selection;

        // Check if we're at the start of a visual line:
        // - parentOffset is 0 (start of block), OR
        // - character before cursor is a newline
        const textBefore = $head.parent.textContent.slice(0, $head.parentOffset);
        const isAtVisualLineStart = $head.parentOffset === 0 || textBefore.endsWith("\n");

        if (isAtVisualLineStart) {
          // Get current visual coordinates
          const coords = view.coordsAtPos($head.pos);
          const lineHeight = coords.bottom - coords.top;

          // Move up by half a line height to ensure we're in the previous line
          const targetY = coords.top - lineHeight / 2;
          const targetPos = view.posAtCoords({ left: coords.left, top: targetY });

          if (targetPos && targetPos.pos < $head.pos) {
            const tr = view.state.tr.setSelection(TextSelection.create(view.state.doc, targetPos.pos));
            view.dispatch(tr);
            return true; // Handled
          }
        }
      }
      return false; // Let ProseMirror/browser handle it
    },
    attributes: {
      class: editorClass,
      spellcheck: "false",
    },
  });

  return { view, schema: editorSchema, clientID, mode };
}

/**
 * Get the current state suitable for serialization.
 */
export function getEditorState(view: EditorView): {
  version: number;
  doc: unknown;
  steps: unknown[] | null;
} {
  const state = view.state;
  const sendable = sendableSteps(state);

  return {
    version: getVersion(state),
    doc: state.doc.toJSON(),
    steps: sendable ? sendable.steps.map((s) => s.toJSON()) : null,
  };
}

/**
 * Get sendable steps from the editor for transmission to server.
 */
export function getSendableSteps(view: EditorView): {
  version: number;
  steps: unknown[];
  clientID: string | number;
} | null {
  const sendable = sendableSteps(view.state);
  if (!sendable) return null;

  return {
    version: getVersion(view.state),
    steps: sendable.steps.map((s) => s.toJSON()),
    clientID: sendable.clientID,
  };
}

export { getVersion };
