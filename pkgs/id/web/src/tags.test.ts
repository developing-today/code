import { describe, expect, it, vi } from "vitest";
import { init } from "./main";

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

describe("loadFileTags response validation (#7841)", () => {
  const setup = async () => {
    document.body.innerHTML = `
      <div id="editor-tag-panel" data-filename="test.md">
        <div id="editor-tag-list"></div>
      </div>
    `;
    localStorage.setItem("id_identity_token", "dummy-token");
    localStorage.setItem("id_identity_client_id", "dummy-client");
    await init();
  };

  it("replaces stale pills with the load-failure state on a wrong-shape 200", async () => {
    await setup();
    window.idApp.renderEditorTags([{ key: "stale", value: "old" }]);
    expect(document.querySelectorAll(".tag-pill-removable").length).toBe(1);

    vi.stubGlobal(
      "fetch",
      async () => ({ ok: true, status: 200, json: async () => ({}) }),
    );
    try {
      await window.idApp.loadFileTags("other.md");
    } finally {
      vi.unstubAllGlobals();
    }

    expect(document.querySelectorAll(".tag-pill-removable").length).toBe(0);
    expect(document.querySelector(".tag-load-error")?.textContent).toBe("failed to load tags");
  });

  it("renders the object control shape and filters system tags", async () => {
    await setup();
    vi.stubGlobal(
      "fetch",
      async () => ({
        ok: true,
        status: 200,
        json: async () => ({
          tags: [
            { key: "project", value: "alpha" },
            { key: "created", value: "2026-01-01" },
            { key: "plain", value: null },
          ],
        }),
      }),
    );
    try {
      await window.idApp.loadFileTags("test.md");
    } finally {
      vi.unstubAllGlobals();
    }

    const pills = [...document.querySelectorAll(".tag-pill-removable")].map((p) =>
      p.textContent?.replace("×", ""),
    );
    expect(pills).toEqual(["project: alpha", "plain"]);
  });
});
