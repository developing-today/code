import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { init } from "./main";

class FakeWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 3;
  static instances: FakeWebSocket[] = [];

  readyState = FakeWebSocket.CONNECTING;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;

  constructor(_url: string) {
    FakeWebSocket.instances.push(this);
  }

  open(): void {
    this.readyState = FakeWebSocket.OPEN;
    this.onopen?.();
  }

  drop(): void {
    this.readyState = FakeWebSocket.CLOSED;
    this.onclose?.();
  }

  close(): void {
    this.readyState = FakeWebSocket.CLOSED;
  }
}

describe("editor tag removal accessibility", () => {
  it("renders tag remove buttons with descriptive aria-label", async () => {
    // Setup minimal DOM required by init and renderEditorTags
    document.body.innerHTML = `
      <div id="editor-tag-panel" data-filename="test.md">
        <div id="editor-tag-list"></div>
      </div>
    `;

    // Store a dummy token in localStorage so ensureIdentity doesn't throw NetworkError in Happy DOM
    localStorage.setItem("id_identity_token", "dummy-token");
    localStorage.setItem("id_identity_client_id", "dummy-client");

    await init();

    window.idApp.renderEditorTags([
      { key: "project", value: "alpha" },
      { key: "pinned", value: null },
    ]);

    const removeButtons = document.querySelectorAll<HTMLButtonElement>(".tag-remove-btn");
    expect(removeButtons.length).toBe(2);

    expect(removeButtons[0].getAttribute("aria-label")).toBe("Remove tag project: alpha");
    expect(removeButtons[0].getAttribute("title")).toBe("Remove tag");

    expect(removeButtons[1].getAttribute("aria-label")).toBe("Remove tag pinned");
    expect(removeButtons[1].getAttribute("title")).toBe("Remove tag");
  });
});

describe("tag WebSocket resync", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket);
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new Error("offline test fetch");
      }),
    );
    localStorage.setItem("id_identity_token", "dummy-token");
    localStorage.setItem("id_identity_client_id", "dummy-client");
    document.body.innerHTML = '<div id="main"><div id="file-list-content"></div></div>';
  });

  afterEach(() => {
    window.idApp?.disconnectTagsWs();
    vi.useRealTimers();
    vi.unstubAllGlobals();
    localStorage.clear();
    document.body.innerHTML = "";
  });

  it("refreshes the visible file list after the server closes a lagged socket", async () => {
    const fetchMock = vi.mocked(fetch);
    await init();

    const first = FakeWebSocket.instances[0];
    expect(first).toBeDefined();
    first.open();
    expect(fetchMock.mock.calls.filter(([url]) => url === "/api/files")).toHaveLength(1);

    first.drop();
    vi.advanceTimersByTime(5_000);
    const reconnected = FakeWebSocket.instances[1];
    expect(reconnected).toBeDefined();
    reconnected.open();
    expect(fetchMock.mock.calls.filter(([url]) => url === "/api/files")).toHaveLength(2);
  });
});
