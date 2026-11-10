import { describe, expect, it } from "vitest";
import { renderTagList } from "./home";

describe("home tag feeds", () => {
  it("renders untrusted tag labels as text rather than HTML", () => {
    const list = document.createElement("ol");
    const label = '<img src=x onerror="alert(1)">';

    renderTagList(list, [{ label, subject: "notes.md", timestamp: 1_800_000_000_000_000, occurrences: 1 }]);

    expect(list.querySelector("img")).toBeNull();
    expect(list.querySelector("a")?.textContent).toBe(label);
    expect(list.textContent).toContain("notes.md");
  });

  it("shows occurrence counts for popular global tags", () => {
    const list = document.createElement("ol");
    renderTagList(list, [{ label: "topic:rust", timestamp: 1_800_000_000_000_000, occurrences: 4 }]);

    expect(list.textContent).toContain("topic:rust");
    expect(list.textContent).toContain("4 documents");
  });
});
