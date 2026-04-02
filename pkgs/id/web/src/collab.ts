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
import { getVersion, receiveTransaction } from "prosemirror-collab";
import { Step } from "prosemirror-transform";
import { markCursorFresh, onInitReceived, removeCursor, setConnectionState, updateCursor } from "./cursors";
import { type ContentMode, type EditorInstance, getSendableSteps, initEditor } from "./editor";

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

  let reconnectAttempts = 0;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  const sendQueue: Uint8Array[] = [];
  let connected = false;
  let editorInstance: EditorInstance | null = null;
  let documentMode: ContentMode | null = null;
  // Mutable reference to the current WebSocket — updated on reconnect
  let currentWs: WebSocket | null = null;
  // Flag to track client-initiated disconnects. Calling ws.close(1000) doesn't
  // guarantee onclose fires with code 1000 — the close handshake can fail/timeout,
  // causing the browser to fire onclose with code 1006 instead. This flag ensures
  // we don't spuriously reconnect after an intentional disconnect.
  let intentionalClose = false;
  // App-level connection timeout — if the WS doesn't reach OPEN within this
  // many ms, we close it and schedule reconnect directly.
  // Browsers default to ~20s TCP timeout which is far too slow for UX.
  let connectTimer: ReturnType<typeof setTimeout> | null = null;
  const CONNECT_TIMEOUT_MS = 2000;

  // Track our clientID (set when editor initializes)
  let myClientID: number | null = null;

  const updateStatus = (status: "connecting" | "connected" | "disconnected" | "error"): void => {
    if (onStatus) onStatus(status);
  };

  // Encode and send a message (always uses currentWs)
  const send = (msgType: number, ...fields: unknown[]): void => {
    const data = packr.pack([msgType, ...fields]);
    if (connected && currentWs && currentWs.readyState === WebSocket.OPEN) {
      currentWs.send(data);
    } else {
      sendQueue.push(data);
    }
  };

  const flushQueue = (): void => {
    while (sendQueue.length > 0 && currentWs && currentWs.readyState === WebSocket.OPEN) {
      const data = sendQueue.shift();
      if (data && currentWs) {
        currentWs.send(data);
      }
    }
  };

  const scheduleReconnect = (): void => {
    if (reconnectAttempts >= 10) {
      console.error("[collab] Max reconnection attempts reached");
      updateStatus("error");
      return;
    }

    // Exponential backoff with jitter: base * 2^attempt + random jitter
    // Fast initial retries (250ms) for localhost/LAN; caps at 5s for WAN.
    // Combined with 2s connect timeout: worst-case cycle ≈ 2.25–7s per attempt.
    const baseDelay = Math.min(250 * 2 ** reconnectAttempts, 5000);
    const jitter = Math.random() * Math.min(250, baseDelay * 0.2);
    const delay = baseDelay + jitter;
    reconnectAttempts++;

    console.log(`[collab] Reconnecting in ${Math.round(delay)}ms (attempt ${reconnectAttempts}/10)`);
    updateStatus("connecting");
    reconnectTimer = setTimeout(() => {
      const newWs = new WebSocket(finalWsUrl);
      newWs.binaryType = "arraybuffer";
      // Update the mutable reference so send()/disconnect() use the new socket
      currentWs = newWs;
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
    let msg: unknown[];
    try {
      msg = unpackr.unpack(new Uint8Array(data)) as unknown[];
    } catch (err) {
      console.error("[collab] Failed to decode MessagePack message:", err);
      return;
    }
    const msgType = msg[0] as number;
    console.log("[collab] handleMessage msgType:", msgType);

    switch (msgType) {
      case MSG.INIT: {
        // [0, version, doc, mode]
        const version = msg[1] as number;
        const doc = msg[2] as { type: string; content?: unknown[] };
        const mode = ((msg[3] as string) || "raw") as ContentMode;
        documentMode = mode;
        console.log("[collab] Received Init, version:", version, "mode:", mode);

        if (!editorInstance) {
          // First connection — initialize the editor from scratch
          console.log("[collab] Initializing editor with server version:", version, "mode:", mode);
          editorInstance = initEditor(container, doc, version, mode, sendCursor);
          myClientID = editorInstance.clientID;
          console.log("[collab] Our clientID:", myClientID);

          // Set up event listener for editor changes
          container.addEventListener("editor:change", handleEditorChange);

          // Notify that editor is ready
          if (onEditorReady) {
            onEditorReady(editorInstance);
          }
        } else {
          // Reconnect — destroy old editor and re-initialize with fresh server state.
          // The server sends a full Init with potentially different version/doc after
          // reconnect, so the ProseMirror collab plugin must be re-created to avoid
          // version mismatch errors.
          console.log("[collab] Reconnect: re-initializing editor, server version:", version);
          container.removeEventListener("editor:change", handleEditorChange);
          editorInstance.view.destroy();

          editorInstance = initEditor(container, doc, version, mode, sendCursor);
          myClientID = editorInstance.clientID;
          console.log("[collab] Reconnect: new clientID:", myClientID);

          container.addEventListener("editor:change", handleEditorChange);

          // Re-notify that editor is ready (re-enables save button etc.)
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
            // Step application failed — editor state is desynchronized.
            // Trigger reconnect to get fresh Init + catch-up from server.
            console.error("[collab] Failed to apply steps, reconnecting:", err);
            connected = false;
            intentionalClose = true;
            if (currentWs) {
              currentWs.close(4001, "Step apply failure");
            }
            scheduleReconnect();
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

        // Version mismatch and desync errors are recoverable via reconnect —
        // the server will send a fresh Init with the correct state
        if (typeof error === "string" && (error.includes("Version mismatch") || error.includes("desynchronized"))) {
          console.log("[collab] Recoverable error — scheduling reconnect to resync:", error);
          connected = false;
          intentionalClose = true;
          if (currentWs) {
            currentWs.close(4000, "Resync");
          }
          scheduleReconnect();
        } else {
          updateStatus("error");
        }
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
    // Start a connection timeout — if the WS doesn't reach OPEN within
    // CONNECT_TIMEOUT_MS, abandon it and schedule reconnect directly.
    // Without this, browsers can hang in CONNECTING state for ~20s (Firefox)
    // waiting for their internal TCP timeout, which is terrible for UX.
    //
    // IMPORTANT: We detach all event handlers and schedule reconnect directly
    // instead of relying on onclose, because:
    // 1. socket.close() with no args uses code 1000, and our onclose handler
    //    skips reconnect for code 1000 (treats it as intentional/clean close)
    // 2. Firefox may not fire onclose at all when closing a CONNECTING socket
    if (connectTimer) clearTimeout(connectTimer);
    connectTimer = setTimeout(() => {
      connectTimer = null;
      if (socket.readyState === WebSocket.CONNECTING) {
        console.warn(`[collab] Connection timeout after ${CONNECT_TIMEOUT_MS}ms, aborting`);
        // Detach all handlers so if onopen/onclose eventually fire on the
        // dead socket, they don't interfere with the new connection
        socket.onopen = null;
        socket.onclose = null;
        socket.onerror = null;
        socket.onmessage = null;
        try {
          socket.close();
        } catch (_) {
          /* ignore — may throw if already garbage collected */
        }
        // Update state directly — don't rely on onclose
        connected = false;
        currentWs = null;
        if (editorInstance) {
          setConnectionState(editorInstance.view, "disconnected");
        }
        updateStatus("disconnected");
        scheduleReconnect();
      }
    }, CONNECT_TIMEOUT_MS);

    socket.onopen = (): void => {
      // Connection succeeded — cancel the timeout
      if (connectTimer) {
        clearTimeout(connectTimer);
        connectTimer = null;
      }
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
      // Clean up any pending connection timeout
      if (connectTimer) {
        clearTimeout(connectTimer);
        connectTimer = null;
      }
      console.log("[collab] Disconnected:", event.code, event.reason);
      const wasConnected = connected;
      connected = false;
      // Only reconnect if this was NOT an intentional disconnect.
      // We check both the intentionalClose flag AND event.code because:
      // - intentionalClose: covers client-initiated disconnect() calls where the
      //   close handshake may fail/timeout, causing the browser to fire onclose
      //   with code 1006 instead of the requested 1000
      // - event.code === 1000 AND wasConnected: covers server-initiated clean
      //   closes when we had a working session. We MUST still reconnect if the
      //   connection dropped before we were fully connected (e.g., immediately
      //   after WS handshake but before Init message was processed), because
      //   that indicates a transient failure, not an intentional close.
      const wasIntentional = intentionalClose;
      intentionalClose = false;
      if (!wasIntentional && !(event.code === 1000 && wasConnected)) {
        // Update cursor state if editor was already initialized
        if (editorInstance) {
          setConnectionState(editorInstance.view, "disconnected");
        }
        updateStatus("disconnected");
        // Always schedule reconnect — even if editorInstance is null (WS closed
        // before Init message arrived). Without this, the client would be stuck
        // forever with no editor and no reconnect if the first connection drops.
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

  // Set up the initial WebSocket
  const initialWs = new WebSocket(finalWsUrl);
  initialWs.binaryType = "arraybuffer";
  currentWs = initialWs;
  setupWebSocket(initialWs);

  const disconnect = (): void => {
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
    }
    if (connectTimer) {
      clearTimeout(connectTimer);
      connectTimer = null;
    }
    container.removeEventListener("editor:change", handleEditorChange);
    // Note: We intentionally don't call setConnectionState here because
    // the view will be destroyed immediately after this function returns.
    // Set intentionalClose BEFORE close() so the onclose handler knows not to reconnect
    // (the browser may fire onclose with code 1006 if the close handshake fails/times out)
    intentionalClose = true;
    if (currentWs) {
      currentWs.close(1000, "Client disconnected");
    }
    currentWs = null;
    updateStatus("disconnected");
  };

  return {
    get ws() {
      return currentWs as WebSocket;
    },
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
