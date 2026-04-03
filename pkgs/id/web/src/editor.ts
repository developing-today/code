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
import {
  MenuItem,
  blockTypeItem,
  liftItem,
  redoItem,
  selectParentNodeItem,
  undoItem,
} from "prosemirror-menu";
import { type MarkType, Node, Schema } from "prosemirror-model";
import { schema as basicSchema } from "prosemirror-schema-basic";
import { addListNodes } from "prosemirror-schema-list";
import { EditorState, type Plugin, TextSelection, type Transaction } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { createActiveLinePlugin } from "./active-line";
import { createCursorPlugin, type SendCursorFn } from "./cursors";
import { createGotoLinePlugin } from "./goto-line";
import { createSyntaxHighlightPlugin } from "./highlight";
import { createImageUploadPlugin } from "./image-upload";
import { createIndentPlugin } from "./indent";
import { createSearchPlugins } from "./search-panel";
import { createWrapPlugins } from "./wrap";

/**
 * Content mode types matching server-side enum.
 * Determines how the editor is configured.
 */
export type ContentMode = "rich" | "markdown" | "plain" | "raw" | "media" | "binary";

/**
 * Full schema with list support and GFM extensions for rich/markdown/plain modes.
 * Extends prosemirror-schema-basic with strikethrough mark, task list nodes, and table nodes.
 */
export const richSchema = new Schema({
  nodes: addListNodes(basicSchema.spec.nodes, "paragraph block*", "block").append({
    task_list: {
      group: "block",
      content: "task_list_item+",
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
      ],
      toDOM() {
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
function markMenuItem(
  markType: MarkType,
  options: { title: string; label: string },
): InstanceType<typeof MenuItem> {
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
      if (empty) return !!(markType.isInSet(state.storedMarks || $from.marks()));
      return state.doc.rangeHasMark(from, to, markType);
    },
  });
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
    },
    dispatchTransaction(transaction: Transaction) {
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
