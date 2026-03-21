/**
 * ProseMirror editor setup for collaborative document editing.
 * Uses prosemirror-example-setup for baseline functionality.
 * 
 * Supports multiple content modes:
 * - "rich" / "markdown" / "plain" - Full editor with toolbar
 * - "raw" - Minimal editor for code/config (no toolbar, no formatting shortcuts)
 * - "media" / "binary" - Not editable (handled elsewhere)
 */

import { EditorState, type Transaction, type Plugin } from 'prosemirror-state';
import { EditorView } from 'prosemirror-view';
import { Schema, DOMParser, Node } from 'prosemirror-model';
import { schema as basicSchema } from 'prosemirror-schema-basic';
import { addListNodes } from 'prosemirror-schema-list';
import { exampleSetup } from 'prosemirror-example-setup';
import { collab, sendableSteps, getVersion } from 'prosemirror-collab';
import { keymap } from 'prosemirror-keymap';
import { baseKeymap } from 'prosemirror-commands';
import { history } from 'prosemirror-history';
import { dropCursor } from 'prosemirror-dropcursor';
import { gapCursor } from 'prosemirror-gapcursor';
import { createCursorPlugin, type SendCursorFn } from './cursors';

/**
 * Content mode types matching server-side enum.
 * Determines how the editor is configured.
 */
export type ContentMode = 'rich' | 'markdown' | 'plain' | 'raw' | 'media' | 'binary';

/**
 * Full schema with list support for rich/markdown/plain modes.
 */
export const richSchema = new Schema({
  nodes: addListNodes(basicSchema.spec.nodes, 'paragraph block*', 'block'),
  marks: basicSchema.spec.marks,
});

/**
 * Minimal schema for raw mode (code/config files).
 * Only allows doc containing code_block nodes with text.
 * No marks (formatting) allowed.
 */
export const rawSchema = new Schema({
  nodes: {
    doc: { content: 'code_block+' },
    text: { group: 'inline' },
    code_block: {
      content: 'text*',
      marks: '',
      group: 'block',
      code: true,
      defining: true,
      parseDOM: [{ tag: 'pre', preserveWhitespace: 'full' }],
      toDOM() { return ['pre', ['code', 0]]; },
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
  return mode === 'raw' ? rawSchema : richSchema;
}

/**
 * Check if a mode uses the full editor with toolbar.
 */
export function hasToolbar(mode: ContentMode): boolean {
  return mode === 'rich' || mode === 'markdown' || mode === 'plain';
}

/**
 * Check if a mode is editable.
 */
export function isEditable(mode: ContentMode): boolean {
  return mode !== 'media' && mode !== 'binary';
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
 * Initialize a ProseMirror editor in the given container.
 * 
 * @param container - The DOM element to mount the editor in
 * @param initialDoc - Optional initial document as ProseMirror JSON
 * @param collabVersion - Starting version for collaboration (default 0)
 * @param mode - Content mode determining schema and plugins
 * @param sendCursor - Optional callback to send cursor updates
 * @returns The editor instance
 */
export function initEditor(
  container: HTMLElement,
  initialDoc?: unknown,
  collabVersion: number = 0,
  mode: ContentMode = 'raw',
  sendCursor?: SendCursorFn
): EditorInstance {
  // Generate a random client ID for this session
  const clientID = Math.floor(Math.random() * 0xFFFFFFFF);
  
  // Select schema based on mode
  const editorSchema = getSchema(mode);
  
  // Parse initial document from JSON if provided, otherwise create empty
  let doc: Node;
  if (initialDoc && typeof initialDoc === 'object') {
    try {
      doc = Node.fromJSON(editorSchema, initialDoc);
    } catch (err) {
      console.error('[editor] Failed to parse initial doc JSON, using empty doc:', err);
      doc = editorSchema.topNodeType.createAndFill() ?? editorSchema.node('doc');
    }
  } else {
    doc = editorSchema.topNodeType.createAndFill() ?? editorSchema.node('doc');
  }

  // Build plugins list based on mode
  const plugins: Plugin[] = [];
  
  if (hasToolbar(mode)) {
    // Full editor with toolbar, menu, and all formatting features
    plugins.push(...exampleSetup({ schema: editorSchema }));
  } else {
    // Minimal setup for raw mode - just basic editing, no menu/toolbar
    plugins.push(
      history(),
      dropCursor(),
      gapCursor(),
      keymap(baseKeymap),
    );
  }
  
  // Always add collab plugin
  plugins.push(collab({ version: collabVersion, clientID }));
  
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
  const editorClass = hasToolbar(mode) ? 'id-editor id-editor-rich' : 'id-editor id-editor-raw';

  // Create editor view
  const view = new EditorView(container, {
    state,
    dispatchTransaction(transaction: Transaction) {
      const newState = view.state.apply(transaction);
      view.updateState(newState);
      
      // Dispatch custom event for collab sync
      // Only for LOCAL changes - remote changes have 'addToHistory' set to false by receiveTransaction
      // We also check that the transaction wasn't created by the collab plugin itself
      const isLocalChange = transaction.docChanged && 
        transaction.getMeta('addToHistory') !== false;
      
      if (isLocalChange) {
        console.log('[editor] Local document change, dispatching editor:change event');
        const event = new CustomEvent('editor:change', {
          detail: { transaction, state: newState },
          bubbles: true,
        });
        container.dispatchEvent(event);
      } else if (transaction.docChanged) {
        console.log('[editor] Remote document change (from collab), not dispatching event');
      }
    },
    attributes: {
      class: editorClass,
      spellcheck: 'false',
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
    steps: sendable ? sendable.steps.map(s => s.toJSON()) : null,
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
    steps: sendable.steps.map(s => s.toJSON()),
    clientID: sendable.clientID,
  };
}

export { getVersion };
