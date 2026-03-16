/**
 * ProseMirror editor setup for collaborative document editing.
 * Uses prosemirror-example-setup for baseline functionality.
 */

import { EditorState, type Transaction, type Plugin } from 'prosemirror-state';
import { EditorView } from 'prosemirror-view';
import { Schema, DOMParser, type Node } from 'prosemirror-model';
import { schema as basicSchema } from 'prosemirror-schema-basic';
import { addListNodes } from 'prosemirror-schema-list';
import { exampleSetup } from 'prosemirror-example-setup';
import { collab, sendableSteps, getVersion } from 'prosemirror-collab';
import { createCursorPlugin, type SendCursorFn } from './cursors';

// Extended schema with list support
export const schema = new Schema({
  nodes: addListNodes(basicSchema.spec.nodes, 'paragraph block*', 'block'),
  marks: basicSchema.spec.marks,
});

export interface EditorInstance {
  view: EditorView;
  schema: typeof schema;
  clientID: number;
}

export interface CollabState {
  version: number;
  unconfirmed: Transaction[];
}

/**
 * Initialize a ProseMirror editor in the given container.
 * 
 * @param container - The DOM element to mount the editor in
 * @param initialContent - Optional initial HTML content
 * @param collabVersion - Starting version for collaboration (default 0)
 * @param sendCursor - Optional callback to send cursor updates
 * @returns The editor instance
 */
export function initEditor(
  container: HTMLElement,
  initialContent?: string,
  collabVersion: number = 0,
  sendCursor?: SendCursorFn
): EditorInstance {
  // Generate a random client ID for this session
  const clientID = Math.floor(Math.random() * 0xFFFFFFFF);
  
  // Parse initial content if provided
  let doc: Node | undefined = schema.topNodeType.createAndFill() ?? undefined;
  if (initialContent) {
    const element = document.createElement('div');
    element.innerHTML = initialContent;
    doc = DOMParser.fromSchema(schema).parse(element);
  }

  // Build plugins list
  const plugins: Plugin[] = [
    ...exampleSetup({ schema }),
    collab({ version: collabVersion, clientID }),
  ];
  
  // Add cursor plugin if sendCursor callback provided
  if (sendCursor) {
    plugins.push(createCursorPlugin(clientID, sendCursor));
  }

  // Create editor state with example setup plugins and collab
  const state = EditorState.create({
    doc,
    plugins,
  });

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
      class: 'id-editor',
      spellcheck: 'false',
    },
  });

  return { view, schema, clientID };
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
