import { describe, expect, it } from "vitest";
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
