import { afterEach, describe, expect, it, vi } from "vitest";
import { initHome, parseHomeFeed, renderTagList } from "./home";

const validFeed = {
  scope: "public local",
  popularity_metric: "tag count",
  recent: [{ name: "notes.md", hash: "abc123", updated_at: 1_800_000_000, tag_count: 1, tags: ["topic:rust"] }],
  popular: [],
  recent_tags: [{ label: "topic:rust", subject: "notes.md", timestamp: 1_800_000_000_000_000, occurrences: 1 }],
  popular_tags: [{ label: "topic:rust", timestamp: 1_800_000_000_000_000, occurrences: 4 }],
};

function malformedFeeds(): unknown[] {
  const arrayFields = ["recent", "popular", "recent_tags", "popular_tags"] as const;
  const malformed: unknown[] = [null, [], "abc", {}];

  for (const field of arrayFields) {
    const { [field]: _omitted, ...withoutArray } = validFeed;
    malformed.push(withoutArray);
    for (const invalid of [null, {}, "items"]) {
      malformed.push({ ...validFeed, [field]: invalid });
    }

    const isDocumentList = field === "recent" || field === "popular";
    const validItem = isDocumentList ? validFeed.recent[0] : validFeed.popular_tags[0];
    const wrongTypedItem = isDocumentList
      ? { ...validFeed.recent[0], name: null }
      : { ...validFeed.popular_tags[0], label: null };
    malformed.push(
      { ...validFeed, [field]: [null] },
      { ...validFeed, [field]: [wrongTypedItem] },
      { ...validFeed, [field]: [validItem, null] },
    );
  }

  malformed.push(
    { ...validFeed, recent: [{ ...validFeed.recent[0], updated_at: "now" }] },
    { ...validFeed, recent: [{ ...validFeed.recent[0], tag_count: "1" }] },
    { ...validFeed, recent: [{ ...validFeed.recent[0], tags: [1] }] },
    { ...validFeed, recent_tags: [{ ...validFeed.recent_tags[0], subject: 1 }] },
    { ...validFeed, recent_tags: [{ ...validFeed.recent_tags[0], timestamp: "now" }] },
    { ...validFeed, popular_tags: [{ ...validFeed.popular_tags[0], occurrences: null }] },
    { ...validFeed, scope: null },
    { ...validFeed, popularity_metric: 1 },
  );

  return malformed;
}

afterEach(() => {
  document.body.innerHTML = "";
  initHome();
  vi.unstubAllGlobals();
});

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

  it("does not replace a tag list when an item has the wrong shape", () => {
    const list = document.createElement("ol");
    renderTagList(list, [{ label: "keep", timestamp: 1_800_000_000_000_000, occurrences: 2 }]);
    const previousMarkup = list.innerHTML;
    const malformedInputs: unknown[] = [null, {}, "abc", [null], [{ label: "new" }, null]];

    for (const input of malformedInputs) {
      renderTagList(list, input as Parameters<typeof renderTagList>[1]);
      expect(list.innerHTML).toBe(previousMarkup);
    }
  });
});

describe("home feed validation (#7811)", () => {
  it("accepts a well-formed feed with an omitted optional subject", () => {
    expect(parseHomeFeed(validFeed)).toEqual(validFeed);
  });

  it("rejects malformed feed bodies, arrays, and item fields", () => {
    for (const feed of malformedFeeds()) {
      expect(parseHomeFeed(feed)).toBeNull();
    }
  });

  it("keeps the last rendered lists and shows a clean message for every malformed response", async () => {
    const visibility = Object.getOwnPropertyDescriptor(document, "visibilityState");
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });

    try {
      for (const malformedFeed of malformedFeeds()) {
        document.body.innerHTML = `
          <section data-home-app>
            <ol data-home-recent><li>last recent</li></ol>
            <ol data-home-popular><li>last popular</li></ol>
            <ol data-home-recent-tags><li>last recent tag</li></ol>
            <ol data-home-popular-tags><li>last popular tag</li></ol>
            <p data-home-updated>Previously updated.</p>
          </section>`;
        const lists = Array.from(document.querySelectorAll("[data-home-app] ol"));
        const previousMarkup = lists.map((list) => list.innerHTML);
        vi.stubGlobal(
          "fetch",
          vi.fn().mockResolvedValue({ ok: true, json: async () => malformedFeed }),
        );

        initHome();
        await new Promise((resolve) => setTimeout(resolve, 0));

        expect(lists.map((list) => list.innerHTML)).toEqual(previousMarkup);
        expect(document.querySelector("[data-home-updated]")?.textContent).toBe(
          "Feed is temporarily unavailable.",
        );
      }
    } finally {
      if (visibility) Object.defineProperty(document, "visibilityState", visibility);
      else Reflect.deleteProperty(document, "visibilityState");
    }
  });
});
