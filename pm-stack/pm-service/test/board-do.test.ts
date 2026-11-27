/** Acceptance oracle: #12830 / component-A design §2. */
import { describe, expect, test } from "bun:test";
import { BoardState, MemoryBoardStorage, loadBoardView, type BoardSocket } from "../src/board-do.ts";
import type { AsyncSqlDb } from "../src/tenant-store/libsql-http.ts";

function tenantDb(snapshotId: string, title: string) {
  const statements: string[] = [];
  const db: AsyncSqlDb = {
    async all<T>(sql: string): Promise<T[]> {
      statements.push(sql);
      if (sql.includes("FROM pm_boards")) return [{
        board_id: "board-a", org: "acme", number: 1, node_id: "PVT_1", url: "https://github.com/orgs/acme/projects/1",
        role: "overall", parent_board_id: null, sibling_order: 0, title: "Board A",
      }] as T[];
      if (sql.includes("FROM pm_snapshots")) return [{
        snapshot_id: snapshotId, generated_at: "2026-11-25T00:00:00Z", reported_total: 1,
        emitted_entries: 1, writer: "hosted", epoch: 1, audit_json: "{}",
      }] as T[];
      if (sql.includes("FROM pm_items")) return [{ item_json: JSON.stringify({ boardId: "board-a", title }) }] as T[];
      throw new Error(`unexpected SQL: ${sql}`);
    },
  };
  return { db, statements };
}

class Socket implements BoardSocket {
  readonly messages: string[] = [];
  send(message: string) { this.messages.push(message); }
  close() {}
}

describe("BoardDO live state", () => {
  test("a landed snapshot pushes a notice to subscribers without polling", async () => {
    const { db } = tenantDb("snap-2", "new title");
    const socket = new Socket();
    const state = new BoardState("board-a", new MemoryBoardStorage(), () => loadBoardView(db, "board-a"), () => [socket]);
    await state.snapshotLanded("snap-2");
    expect(socket.messages).toHaveLength(1);
    expect(JSON.parse(socket.messages[0] as string)).toMatchObject({ type: "snapshot", snapshotId: "snap-2", view: { items: [{ title: "new title" }] } });
  });

  test("a new DO rebuilds derived state from the tenant DB after storage loss", async () => {
    const { db } = tenantDb("snap-3", "rebuilt title");
    const state = new BoardState("board-a", new MemoryBoardStorage(), () => loadBoardView(db, "board-a"), () => []);
    expect(await state.view()).toMatchObject({ snapshot: { snapshotId: "snap-3" }, items: [{ title: "rebuilt title" }] });
  });

  test("presence is ephemeral and no tenant DB write is issued", async () => {
    const { db, statements } = tenantDb("snap-1", "title");
    const storage = new MemoryBoardStorage();
    const socket = new Socket();
    const state = new BoardState("board-a", storage, () => loadBoardView(db, "board-a"), () => [socket]);
    state.setPresence(socket, { session: "viewer-1" });
    await state.view();
    expect(statements.every((sql) => /^\s*SELECT\b/i.test(sql))).toBe(true);
    expect(JSON.stringify(storage.snapshot())).not.toContain("viewer-1");
    expect(JSON.stringify(storage.snapshot())).not.toContain("presence");
    expect(JSON.parse(socket.messages[0] as string)).toEqual({ type: "presence", viewers: [{ session: "viewer-1" }] });
  });
});
