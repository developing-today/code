/**
 * Collaborative editing connection using WebSockets.
 * Implements the client side of prosemirror-collab's authority model.
 *
 * Wire Protocol (MessagePack arrays):
 * - [0, version, doc, mode] - Init: server sends initial state and content mode
 * - [1, version, steps, clientID] - Steps: client sends changes
 * - [2, steps, clientIDs] - Update: server broadcasts changes
 * - [3, version] - Ack: server confirms steps applied
 * - [4, clientID, head, anchor, name?, idleSecs?] - Cursor position
 * - [5, error] - Error message
 * - [6, clientID] - Cursor removed (client disconnected)
 *
 * Content Modes:
 * - "rich" - ProseMirror JSON files, full editor
 * - "markdown" - Markdown files, full editor (server converts)
 * - "plain" - Plain text files, full editor
 * - "raw" - Code/config files, minimal editor (no toolbar)
 * - "media" - Media files, displayed natively
 * - "binary" - Binary files, not editable
 *
 * The idleSecs field is only sent when the server sends existing cursors
 * to a newly connected client, indicating how long the cursor has been idle.
 *
 * Additionally, the server sends empty text messages periodically
 * (instead of WebSocket Ping frames) to trigger cursor decoration refresh.
 * Client responds with empty text as pong.
 *
 * Connection State Management:
 * - On WebSocket open: calls setConnectionState('connected')
 * - On WebSocket close: calls setConnectionState('disconnected')
 * - On Init message: calls onInitReceived() to start stale cursor cleanup
 *
 * See cursors.ts for cursor state management and reconnect behavior.
 */

import { Packr, Unpackr } from "msgpackr";
import { receiveTransaction, getVersion } from "prosemirror-collab";
import { Step } from "prosemirror-transform";
import { getSendableSteps, initEditor, type EditorInstance, type ContentMode } from "./editor";
import { updateCursor, setConnectionState, onInitReceived, markCursorFresh, removeCursor } from "./cursors";

// Message type tags
const MSG = {
  INIT: 0,
  STEPS: 1,
  UPDATE: 2,
  ACK: 3,
  CURSOR: 4,
  ERROR: 5,
  CURSOR_REMOVE: 6,
} as const;

// MessagePack encoder/decoder configured for array format
const packr = new Packr({ useRecords: false, structuredClone: true });
const unpackr = new Unpackr({ useRecords: false, structuredClone: true });

export interface CollabConnection {
  ws: WebSocket;
  docId: string;
  disconnect: () => void;
  editor: EditorInstance | null;
  mode: ContentMode | null;
}

export type StatusCallback = (status: "connecting" | "connected" | "disconnected" | "error") => void;

/**
 * Initialize a collaborative editing connection.
 * This connects to the WebSocket first, receives the server version and document,
 * then initializes the ProseMirror editor with the server's state.
 *
 * @param wsUrl - WebSocket URL for the collab server
 * @param container - The DOM container for the editor
 * @param docId - Document identifier
 * @param filename - Optional filename for content mode detection
 * @param onStatus - Callback for status changes
 * @param onEditorReady - Callback when editor is initialized
 * @returns The collab connection
 */
export function initCollab(
  wsUrl: string,
  container: HTMLElement,
  docId: string,
  filename?: string,
  onStatus?: StatusCallback,
  onEditorReady?: (editor: EditorInstance) => void,
): CollabConnection {
  // Append filename as query parameter if provided
  const finalWsUrl = filename ? `${wsUrl}?filename=${encodeURIComponent(filename)}` : wsUrl;
  const ws = new WebSocket(finalWsUrl);
  ws.binaryType = "arraybuffer"; // Receive binary data as ArrayBuffer

  let reconnectAttempts = 0;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  const sendQueue: Uint8Array[] = [];
  let connected = false;
  let editorInstance: EditorInstance | null = null;
  let documentMode: ContentMode | null = null;

  // Track our clientID (set when editor initializes)
  let myClientID: number | null = null;

  const updateStatus = (status: "connecting" | "connected" | "disconnected" | "error"): void => {
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
      console.error("[collab] Max reconnection attempts reached");
      updateStatus("error");
      return;
    }

    const delay = Math.min(1000 * 2 ** reconnectAttempts, 30000);
    reconnectAttempts++;

    console.log(`[collab] Reconnecting in ${delay}ms (attempt ${reconnectAttempts})`);
    updateStatus("connecting");
    reconnectTimer = setTimeout(() => {
      const newWs = new WebSocket(finalWsUrl);
      newWs.binaryType = "arraybuffer";
      setupWebSocket(newWs);
    }, delay);
  };

  // Send cursor position to server: [4, clientID, head, anchor, name?, idleSecs?]
  // Note: client never sends idleSecs, only server sends it on initial load
  const sendCursor = (head: number, anchor: number): void => {
    if (myClientID === null) return;
    send(MSG.CURSOR, myClientID, head, anchor, null);
  };

  // Listen for editor changes and send steps: [1, version, steps, clientID]
  const handleEditorChange = (): void => {
    if (!editorInstance) return;

    console.log("[collab] handleEditorChange triggered");
    const sendable = getSendableSteps(editorInstance.view);
    console.log("[collab] sendable:", sendable);
    if (sendable) {
      console.log("[collab] Sending", sendable.steps.length, "steps, version:", sendable.version);
      send(MSG.STEPS, sendable.version, sendable.steps, sendable.clientID);
    }
  };

  const handleMessage = (data: ArrayBuffer): void => {
    const msg = unpackr.unpack(new Uint8Array(data)) as unknown[];
    const msgType = msg[0] as number;
    console.log("[collab] handleMessage msgType:", msgType, "full msg:", msg);

    switch (msgType) {
      case MSG.INIT: {
        // [0, version, doc, mode]
        const version = msg[1] as number;
        const doc = msg[2] as { type: string; content?: unknown[] };
        const mode = ((msg[3] as string) || "raw") as ContentMode;
        documentMode = mode;
        console.log("[collab] Received initial state, version:", version, "mode:", mode);
        console.log("[collab] Doc type:", doc?.type);
        console.log("[collab] Doc content length:", doc?.content?.length);
        if (doc?.content?.[0]) {
          const firstNode = doc.content[0] as { type?: string; attrs?: unknown };
          console.log("[collab] First node type:", firstNode?.type, "attrs:", firstNode?.attrs);
        }
        console.log("[collab] Full doc:", JSON.stringify(doc).slice(0, 500));

        // Initialize the editor with the server's document and mode
        if (!editorInstance) {
          console.log("[collab] Initializing editor with server version:", version, "mode:", mode);
          console.log("[collab] Container element:", container, "innerHTML before:", container.innerHTML.slice(0, 100));
          // Pass the server's ProseMirror JSON doc, not the HTML initialContent
          editorInstance = initEditor(container, doc, version, mode, sendCursor);
          console.log("[collab] Container innerHTML after initEditor:", container.innerHTML.slice(0, 200));
          myClientID = editorInstance.clientID;
          console.log("[collab] Our clientID:", myClientID);

          // Set up event listener for editor changes
          container.addEventListener("editor:change", handleEditorChange);

          // Notify that editor is ready
          if (onEditorReady) {
            onEditorReady(editorInstance);
          }
        }

        // Start reconnect cleanup timer (will remove stale cursors after 1s)
        // Must be called after editor is initialized
        onInitReceived(editorInstance.view);
        break;
      }

      case MSG.UPDATE: {
        // [2, steps, clientIDs]
        if (!editorInstance) {
          console.warn("[collab] Received update before editor initialized");
          break;
        }

        const steps = msg[1] as unknown[];
        const clientIDs = msg[2] as number[];

        if (steps && clientIDs && steps.length > 0) {
          console.log("[collab] Received update with", steps.length, "steps, clientIDs:", clientIDs);

          try {
            // Pass ALL steps to receiveTransaction - it will:
            // 1. Recognize and confirm our own steps (matching our clientID)
            // 2. Apply remote steps from other clients
            // 3. Rebase any unconfirmed local steps over remote steps
            // Use the schema from the editor instance (mode-aware)
            const editorSchema = editorInstance.view.state.schema;
            const parsedSteps = steps.map((s) => Step.fromJSON(editorSchema, s));
            const tr = receiveTransaction(editorInstance.view.state, parsedSteps, clientIDs);
            editorInstance.view.dispatch(tr);
            console.log("[collab] Applied transaction, new version:", getVersion(editorInstance.view.state));
          } catch (err) {
            console.error("[collab] Failed to apply steps:", err);
          }
        }
        break;
      }

      case MSG.CURSOR: {
        // [4, clientID, head, anchor, name?, idleSecs?]
        if (!editorInstance) break;

        const clientID = msg[1] as number;
        const head = msg[2] as number;
        const anchor = msg[3] as number;
        const name = msg[4] as string | null;
        const idleSecs = msg[5] as number | null | undefined;

        // Mark cursor as fresh for reconnect cleanup (including own cursor)
        markCursorFresh(editorInstance.view, clientID);

        // Note: We no longer filter out own cursor here - the cursor plugin
        // handles displaying own cursor with a distinct style (no tooltip)

        console.log(
          "[collab] Cursor update from",
          clientID,
          "at",
          head,
          idleSecs ? `(idle ${idleSecs}s)` : "",
          clientID === myClientID ? "(own)" : "",
        );
        updateCursor(editorInstance.view, clientID, head, anchor, name ?? undefined, idleSecs ?? undefined);
        break;
      }

      case MSG.ACK: {
        // [3, version]
        const version = msg[1] as number;
        console.log("[collab] Steps acknowledged, new version:", version);
        break;
      }

      case MSG.ERROR: {
        // [5, error]
        const error = msg[1] as string;
        console.error("[collab] Server error:", error);
        updateStatus("error");
        break;
      }

      case MSG.CURSOR_REMOVE: {
        // [6, clientID]
        if (!editorInstance) break;

        const clientID = msg[1] as number;

        // Don't remove our own cursor on disconnect notification
        if (clientID === myClientID) break;

        console.log("[collab] Cursor removed for client", clientID);
        removeCursor(editorInstance.view, clientID);
        break;
      }

      default:
        console.warn("[collab] Unknown message type:", msgType);
    }
  };

  const setupWebSocket = (socket: WebSocket): void => {
    socket.onopen = (): void => {
      console.log("[collab] Connected to", wsUrl);
      connected = true;
      reconnectAttempts = 0;
      updateStatus("connected");
      if (editorInstance) {
        setConnectionState(editorInstance.view, "connected");
      }
      flushQueue();
    };

    socket.onclose = (event): void => {
      console.log("[collab] Disconnected:", event.code, event.reason);
      connected = false;
      // Only update connection state if we're not intentionally closing
      // (the view may be destroyed if this is an intentional disconnect)
      if (event.code !== 1000 && editorInstance) {
        setConnectionState(editorInstance.view, "disconnected");
        updateStatus("disconnected");
        scheduleReconnect();
      }
    };

    socket.onerror = (error): void => {
      console.error("[collab] WebSocket error:", error);
      updateStatus("error");
    };

    socket.onmessage = (event): void => {
      // Ignore messages if we're not connected (e.g., during close)
      if (!connected) return;

      if (event.data instanceof ArrayBuffer) {
        handleMessage(event.data);
      } else if (typeof event.data === "string") {
        // Empty text message from server = ping that triggers JS
        // (WebSocket Ping frames don't trigger onmessage)
        // Refresh cursor decorations and respond with pong
        if (event.data === "" && editorInstance) {
          // Dispatch empty transaction to refresh cursor decorations
          editorInstance.view.dispatch(editorInstance.view.state.tr);
          // Send pong (empty text)
          socket.send("");
        }
      }
    };
  };

  // Set up the WebSocket
  setupWebSocket(ws);

  const disconnect = (): void => {
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
    }
    container.removeEventListener("editor:change", handleEditorChange);
    // Note: We intentionally don't call setConnectionState here because
    // the view will be destroyed immediately after this function returns.
    // The close code 1000 tells the onclose handler not to try using the view.
    ws.close(1000, "Client disconnected");
    updateStatus("disconnected");
  };

  return {
    ws,
    docId,
    disconnect,
    get editor() {
      return editorInstance;
    },
    get mode() {
      return documentMode;
    },
  };
}
