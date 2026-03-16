/**
 * Collaborative editing connection using WebSockets.
 * Implements the client side of prosemirror-collab's authority model.
 * 
 * Wire Protocol (MessagePack arrays):
 * - [0, version, doc] - Init: server sends initial state
 * - [1, version, steps, clientID] - Steps: client sends changes
 * - [2, steps, clientIDs] - Update: server broadcasts changes
 * - [3, version] - Ack: server confirms steps applied
 * - [4, clientID, head, anchor, name?] - Cursor position
 * - [5, error] - Error message
 */

import { Packr, Unpackr } from 'msgpackr';
import { receiveTransaction, getVersion } from 'prosemirror-collab';
import { Step } from 'prosemirror-transform';
import { schema, getSendableSteps, initEditor, type EditorInstance } from './editor';
import { updateCursor } from './cursors';

// Message type tags
const MSG = {
  INIT: 0,
  STEPS: 1,
  UPDATE: 2,
  ACK: 3,
  CURSOR: 4,
  ERROR: 5,
} as const;

// MessagePack encoder/decoder configured for array format
const packr = new Packr({ useRecords: false, structuredClone: true });
const unpackr = new Unpackr({ useRecords: false, structuredClone: true });

export interface CollabConnection {
  ws: WebSocket;
  docId: string;
  disconnect: () => void;
  editor: EditorInstance | null;
}

export type StatusCallback = (status: 'connecting' | 'connected' | 'disconnected' | 'error') => void;

/**
 * Initialize a collaborative editing connection.
 * This connects to the WebSocket first, receives the server version,
 * then initializes the ProseMirror editor with the correct version.
 * 
 * @param wsUrl - WebSocket URL for the collab server
 * @param container - The DOM container for the editor
 * @param initialContent - Initial HTML content for the editor
 * @param docId - Document identifier
 * @param onStatus - Callback for status changes
 * @param onEditorReady - Callback when editor is initialized
 * @returns The collab connection
 */
export function initCollab(
  wsUrl: string,
  container: HTMLElement,
  initialContent: string,
  docId: string,
  onStatus?: StatusCallback,
  onEditorReady?: (editor: EditorInstance) => void
): CollabConnection {
  const ws = new WebSocket(wsUrl);
  ws.binaryType = 'arraybuffer'; // Receive binary data as ArrayBuffer
  
  let reconnectAttempts = 0;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let sendQueue: Uint8Array[] = [];
  let connected = false;
  let editorInstance: EditorInstance | null = null;
  
  // Track our clientID (set when editor initializes)
  let myClientID: number | null = null;

  const updateStatus = (status: 'connecting' | 'connected' | 'disconnected' | 'error'): void => {
    if (onStatus) onStatus(status);
  };

  // Encode and send a message
  const send = (msgType: number, ...fields: unknown[]): void => {
    const data = packr.pack([msgType, ...fields]);
    if (connected && ws.readyState === WebSocket.OPEN) {
      ws.send(data);
    } else {
      sendQueue.push(data);
    }
  };

  const flushQueue = (): void => {
    while (sendQueue.length > 0 && ws.readyState === WebSocket.OPEN) {
      const data = sendQueue.shift();
      if (data) {
        ws.send(data);
      }
    }
  };

  const scheduleReconnect = (): void => {
    if (reconnectAttempts >= 5) {
      console.error('[collab] Max reconnection attempts reached');
      updateStatus('error');
      return;
    }
    
    const delay = Math.min(1000 * Math.pow(2, reconnectAttempts), 30000);
    reconnectAttempts++;
    
    console.log(`[collab] Reconnecting in ${delay}ms (attempt ${reconnectAttempts})`);
    updateStatus('connecting');
    reconnectTimer = setTimeout(() => {
      const newWs = new WebSocket(wsUrl);
      newWs.binaryType = 'arraybuffer';
      setupWebSocket(newWs);
    }, delay);
  };

  // Send cursor position to server: [4, clientID, head, anchor, name?]
  const sendCursor = (head: number, anchor: number): void => {
    if (myClientID === null) return;
    send(MSG.CURSOR, myClientID, head, anchor, null);
  };

  // Listen for editor changes and send steps: [1, version, steps, clientID]
  const handleEditorChange = (): void => {
    if (!editorInstance) return;
    
    console.log('[collab] handleEditorChange triggered');
    const sendable = getSendableSteps(editorInstance.view);
    console.log('[collab] sendable:', sendable);
    if (sendable) {
      console.log('[collab] Sending', sendable.steps.length, 'steps, version:', sendable.version);
      send(MSG.STEPS, sendable.version, sendable.steps, sendable.clientID);
    }
  };

  const handleMessage = (data: ArrayBuffer): void => {
    const msg = unpackr.unpack(new Uint8Array(data)) as unknown[];
    const msgType = msg[0] as number;

    switch (msgType) {
      case MSG.INIT: {
        // [0, version, doc]
        const version = msg[1] as number;
        const doc = msg[2];
        console.log('[collab] Received initial state, version:', version);
        
        // Initialize the editor with the server's version
        if (!editorInstance) {
          console.log('[collab] Initializing editor with server version:', version);
          editorInstance = initEditor(container, initialContent, version, sendCursor);
          myClientID = editorInstance.clientID;
          console.log('[collab] Our clientID:', myClientID);
          
          // Set up event listener for editor changes
          container.addEventListener('editor:change', handleEditorChange);
          
          // Notify that editor is ready
          if (onEditorReady) {
            onEditorReady(editorInstance);
          }
        }
        break;
      }

      case MSG.UPDATE: {
        // [2, steps, clientIDs]
        if (!editorInstance) {
          console.warn('[collab] Received update before editor initialized');
          break;
        }
        
        const steps = msg[1] as unknown[];
        const clientIDs = msg[2] as number[];
        
        if (steps && clientIDs && steps.length > 0) {
          console.log('[collab] Received update with', steps.length, 'steps, clientIDs:', clientIDs);
          
          try {
            // Pass ALL steps to receiveTransaction - it will:
            // 1. Recognize and confirm our own steps (matching our clientID)
            // 2. Apply remote steps from other clients
            // 3. Rebase any unconfirmed local steps over remote steps
            const parsedSteps = steps.map(s => Step.fromJSON(schema, s));
            const tr = receiveTransaction(editorInstance.view.state, parsedSteps, clientIDs);
            editorInstance.view.dispatch(tr);
            console.log('[collab] Applied transaction, new version:', getVersion(editorInstance.view.state));
          } catch (err) {
            console.error('[collab] Failed to apply steps:', err);
          }
        }
        break;
      }

      case MSG.CURSOR: {
        // [4, clientID, head, anchor, name?]
        if (!editorInstance) break;
        
        const clientID = msg[1] as number;
        const head = msg[2] as number;
        const anchor = msg[3] as number;
        const name = msg[4] as string | null;
        
        if (clientID === myClientID) break; // Ignore our own cursor
        
        console.log('[collab] Cursor update from', clientID, 'at', head);
        updateCursor(editorInstance.view, clientID, head, anchor, name ?? undefined);
        break;
      }

      case MSG.ACK: {
        // [3, version]
        const version = msg[1] as number;
        console.log('[collab] Steps acknowledged, new version:', version);
        break;
      }

      case MSG.ERROR: {
        // [5, error]
        const error = msg[1] as string;
        console.error('[collab] Server error:', error);
        updateStatus('error');
        break;
      }

      default:
        console.warn('[collab] Unknown message type:', msgType);
    }
  };

  const setupWebSocket = (socket: WebSocket): void => {
    socket.onopen = (): void => {
      console.log('[collab] Connected to', wsUrl);
      connected = true;
      reconnectAttempts = 0;
      updateStatus('connected');
      flushQueue();
    };

    socket.onclose = (event): void => {
      console.log('[collab] Disconnected:', event.code, event.reason);
      connected = false;
      
      if (event.code !== 1000) { // Not a normal closure
        updateStatus('disconnected');
        scheduleReconnect();
      }
    };

    socket.onerror = (error): void => {
      console.error('[collab] WebSocket error:', error);
      updateStatus('error');
    };

    socket.onmessage = (event): void => {
      if (event.data instanceof ArrayBuffer) {
        handleMessage(event.data);
      } else {
        console.error('[collab] Expected binary message, got:', typeof event.data);
      }
    };
  };

  // Set up the WebSocket
  setupWebSocket(ws);

  const disconnect = (): void => {
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
    }
    container.removeEventListener('editor:change', handleEditorChange);
    ws.close(1000, 'Client disconnected');
    updateStatus('disconnected');
  };

  return {
    ws,
    docId,
    disconnect,
    get editor() { return editorInstance; },
  };
}
