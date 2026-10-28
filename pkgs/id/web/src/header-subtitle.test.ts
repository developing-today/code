import { describe, expect, it } from "vitest";
import { updateHeaderSubtitle } from "./header-subtitle";

function renderSubtitle(filename: string, path: string): HTMLElement {
  document.body.innerHTML = "";
  const subtitle = document.createElement("div");
  subtitle.id = "header-subtitle";
  document.body.appendChild(subtitle);
  updateHeaderSubtitle(filename, path, true);
  return subtitle;
}

describe("header subtitle", () => {
  it("renders a markup-carrying filename as literal text, not elements", () => {
    const label = '<img src=x onerror="alert(1)">.md';
    const subtitle = renderSubtitle(label, "/edit/" + label);

    expect(subtitle.querySelector("img")).toBeNull();
    const link = subtitle.querySelector("a");
    expect(link).not.toBeNull();
    expect(link?.textContent).toBe(label);
    expect(link?.getAttribute("data-nav")).toBe("");
    expect(subtitle.querySelectorAll("a")).toHaveLength(1);
  });

  it("cannot be broken out of by an anchor-closing filename", () => {
    const label = "x</a><svg onload=alert(2)>";
    const subtitle = renderSubtitle(label, "/edit/x");

    expect(subtitle.querySelector("svg")).toBeNull();
    expect(subtitle.querySelectorAll("a")).toHaveLength(1);
    expect(subtitle.querySelector("a")?.textContent).toBe(label);
  });

  it("renders the benign control as one anchor with the file's text and href", () => {
    const subtitle = renderSubtitle("notes.md", "/edit/notes.md");

    const link = subtitle.querySelector("a");
    expect(link).not.toBeNull();
    expect(link?.textContent).toBe("notes.md");
    expect(link?.getAttribute("href")).toBe("/edit/notes.md");
    expect(subtitle.textContent).toBe("// notes.md");
  });

  it("keeps quotes in the path inside the href attribute", () => {
    const path = '/edit/a" onmouseover="alert(3)';
    const subtitle = renderSubtitle("notes.md", path);

    expect(subtitle.querySelectorAll("a")).toHaveLength(1);
    const link = subtitle.querySelector("a");
    expect(link?.getAttribute("href")).toBe(path);
    expect(link?.hasAttribute("onmouseover")).toBe(false);
  });

  it("falls back to the plain subtitle without history", () => {
    document.body.innerHTML = "";
    const subtitle = document.createElement("div");
    subtitle.id = "header-subtitle";
    document.body.appendChild(subtitle);

    updateHeaderSubtitle("notes.md", "/edit/notes.md", false);

    expect(subtitle.textContent).toBe("// p2p file sharing");
    expect(subtitle.querySelector("a")).toBeNull();
  });
});
