/**
 * BoardDO — rebuildable live view + hibernating dashboard subscriptions.
 * Durable state contains only the last derived tenant-DB view; presence
 * is intentionally memory-only and is discarded when the object restarts.
 */
import { loadPoolAsync, loadBoardSummariesAsync } from "./read-api/async-store.ts";
import type { AsyncSqlDb } from "./tenant-store/libsql-http.ts";

export interface BoardStorage {
  get<T>(key: string): Promise<T | undefined>;
  put<T>(key: string, value: T): Promise<void>;
}

export class MemoryBoardStorage implements BoardStorage {
  private readonly values = new Map<string, unknown>();
  async get<T>(key: string): Promise<T | undefined> {
    return this.values.get(key) as T | undefined;
  }
  async put<T>(key: string, value: T): Promise<void> {
    this.values.set(key, value);
  }
  snapshot(): Record<string, unknown> {
    return Object.fromEntries(this.values);
  }
}

export interface BoardView {
  boardId: string;
  snapshot: unknown;
  audit: unknown;
  items: unknown[];
}

export interface BoardSocket {
  send(message: string): void;
  close(code?: number, reason?: string): void;
}

const VIEW_KEY = "derived-board-view";

export async function loadBoardView(db: AsyncSqlDb, boardId: string): Promise<BoardView> {
  const summaries = await loadBoardSummariesAsync(db);
  const summary = summaries.find((entry) => entry.board.id === boardId);
  if (!summary) throw new Error(`unknown board: ${boardId}`);
  const pool = await loadPoolAsync(db);
  return {
    boardId,
    snapshot: summary.snapshot,
    audit: summary.audit,
    items: pool.filter((item) => item.boardId === boardId),
  };
}

export class BoardState {
  private readonly presence = new Map<BoardSocket, unknown>();
  constructor(
    private readonly boardId: string,
    private readonly storage: BoardStorage,
    private readonly reload: () => Promise<BoardView>,
    private readonly sockets: () => readonly BoardSocket[],
  ) {}

  async view(): Promise<BoardView> {
    const stored = await this.storage.get<BoardView>(VIEW_KEY);
    if (stored) return stored;
    return this.rebuild();
  }

  async rebuild(): Promise<BoardView> {
    const view = await this.reload();
    if (view.boardId !== this.boardId) throw new Error("BoardDO loader returned a different board");
    await this.storage.put(VIEW_KEY, view);
    return view;
  }

  async snapshotLanded(snapshotId: string): Promise<BoardView> {
    const view = await this.rebuild();
    const notice = JSON.stringify({ type: "snapshot", snapshotId, view });
    for (const socket of this.sockets()) {
      try {
        socket.send(notice);
      } catch {
        socket.close(1011, "notification failed");
      }
    }
    return view;
  }

  subscribe(socket: BoardSocket): void {
    // Sending the latest view makes a newly hibernated session current
    // immediately; subsequent updates arrive as push notices.
    void this.view().then((view) => socket.send(JSON.stringify({ type: "snapshot", view })));
  }

  setPresence(socket: BoardSocket, value: unknown): void {
    this.presence.set(socket, value);
    this.broadcastPresence();
  }

  removePresence(socket: BoardSocket): void {
    if (this.presence.delete(socket)) this.broadcastPresence();
  }

  private broadcastPresence(): void {
    const message = JSON.stringify({ type: "presence", viewers: [...this.presence.values()] });
    for (const socket of this.sockets()) {
      try {
        socket.send(message);
      } catch {
        socket.close(1011, "presence delivery failed");
      }
    }
  }
}

export interface BoardDOEnv {
  PM_TENANT_DB_URL?: string;
  PM_TENANT_DB_TOKEN?: string;
  BOARD_DO?: DurableObjectNamespace;
}

/** Adapter used by the collector after its tenant-DB transaction commits. */
export function boardDoNotifier(namespace: DurableObjectNamespace) {
  return (boardId: string) => ({
    async snapshotLanded(snapshotId: string): Promise<void> {
      const stub = namespace.get(namespace.idFromName(boardId));
      const response = await stub.fetch("https://board-do/internal/snapshot-landed", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ snapshotId }),
      });
      if (!response.ok) throw new Error(`BoardDO notification failed (${response.status}) for ${boardId}`);
    },
  });
}

/** Cloudflare Durable Object implementation; WebSocket attachments are
 * hibernation-safe and contain no presence state. */
export class BoardDO {
  private readonly state: DurableObjectState;
  private readonly env: BoardDOEnv;
  private readonly board: BoardState;

  constructor(state: DurableObjectState, env: BoardDOEnv) {
    this.state = state;
    this.env = env;
    const boardId = state.id.name ?? state.id.toString();
    const storage: BoardStorage = {
      get: <T>(key: string) => state.storage.get<T>(key),
      put: <T>(key: string, value: T) => state.storage.put(key, value),
    };
    this.board = new BoardState(
      boardId,
      storage,
      async () => {
        if (!env.PM_TENANT_DB_URL) throw new Error("tenant store is not configured");
        const { libsqlHttpDb } = await import("./tenant-store/libsql-http.ts");
        return loadBoardView(libsqlHttpDb({ url: env.PM_TENANT_DB_URL, token: env.PM_TENANT_DB_TOKEN }), boardId);
      },
      () => state.getWebSockets().map((socket) => socket as unknown as BoardSocket),
    );
  }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    if (request.method === "POST" && url.pathname === "/internal/snapshot-landed") {
      const body = (await request.json()) as { snapshotId?: string };
      if (!body.snapshotId) return new Response("snapshotId required", { status: 400 });
      await this.board.snapshotLanded(body.snapshotId);
      return new Response(null, { status: 204 });
    }
    if (request.method === "GET" && url.pathname === "/subscribe") {
      const pair = new WebSocketPair();
      this.state.acceptWebSocket(pair[1]);
      this.board.subscribe(pair[1] as unknown as BoardSocket);
      return new Response(null, { status: 101, webSocket: pair[0] });
    }
    return Response.json({ error: "not found" }, { status: 404 });
  }

  webSocketMessage(socket: WebSocket, message: string | ArrayBuffer): void {
    if (typeof message !== "string") return;
    let data: { type?: string; viewer?: unknown };
    try {
      data = JSON.parse(message) as typeof data;
    } catch {
      return;
    }
    if (data.type === "presence") this.board.setPresence(socket as unknown as BoardSocket, data.viewer ?? null);
  }

  webSocketClose(socket: WebSocket): void {
    this.board.removePresence(socket as unknown as BoardSocket);
  }
  webSocketError(socket: WebSocket): void {
    this.board.removePresence(socket as unknown as BoardSocket);
  }
}
