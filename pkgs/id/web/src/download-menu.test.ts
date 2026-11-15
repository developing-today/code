/**
 * Tests for the download menu disclosure (#10449).
 */

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { closeDownloadMenu, isDownloadMenuOpen, toggleDownloadMenu } from "./download-menu";

function renderMenu(): { toggle: HTMLButtonElement; menu: HTMLElement } {
  document.body.innerHTML = `
    <span class="relative" id="download-dropdown">
      <button id="download-btn" aria-haspopup="true" aria-expanded="false" aria-controls="download-menu">dl</button>
      <span class="dropdown-menu" id="download-menu">
        <button data-dl-format="raw">raw</button>
        <button data-dl-format="json">pm json</button>
      </span>
    </span>
  `;
  return {
    toggle: document.getElementById("download-btn") as HTMLButtonElement,
    menu: document.getElementById("download-menu") as HTMLElement,
  };
}

beforeEach(() => {
  renderMenu();
});

afterEach(() => {
  document.body.innerHTML = "";
});

describe("download menu disclosure", () => {
  it("starts closed with aria-expanded=false", () => {
    expect(isDownloadMenuOpen()).toBe(false);
    expect(document.getElementById("download-btn")?.getAttribute("aria-expanded")).toBe("false");
  });

  it("toggle opens the menu and reports aria-expanded=true", () => {
    toggleDownloadMenu();
    expect(isDownloadMenuOpen()).toBe(true);
    expect(document.getElementById("download-btn")?.getAttribute("aria-expanded")).toBe("true");
  });

  it("toggle closes the menu again and reports aria-expanded=false", () => {
    toggleDownloadMenu();
    toggleDownloadMenu();
    expect(isDownloadMenuOpen()).toBe(false);
    expect(document.getElementById("download-btn")?.getAttribute("aria-expanded")).toBe("false");
  });

  it("close dismisses an open menu and reports that it was open", () => {
    toggleDownloadMenu();
    expect(closeDownloadMenu()).toBe(true);
    expect(isDownloadMenuOpen()).toBe(false);
    expect(document.getElementById("download-btn")?.getAttribute("aria-expanded")).toBe("false");
  });

  it("closing a closed menu is a no-op and does not steal focus", () => {
    const outside = document.createElement("button");
    document.body.appendChild(outside);
    outside.focus();

    expect(closeDownloadMenu(document, { restoreFocus: true })).toBe(false);
    expect(document.activeElement).toBe(outside);
  });

  it("restoreFocus returns focus to the toggle", () => {
    toggleDownloadMenu();
    (document.querySelector("[data-dl-format]") as HTMLElement).focus();

    closeDownloadMenu(document, { restoreFocus: true });
    expect(document.activeElement).toBe(document.getElementById("download-btn"));
  });

  it("does nothing when the page has no download menu", () => {
    document.body.innerHTML = "";
    expect(() => toggleDownloadMenu()).not.toThrow();
    expect(closeDownloadMenu(document, { restoreFocus: true })).toBe(false);
  });
});
