type FeedDocument = {
  name: string;
  hash: string;
  updated_at: number;
  tag_count: number;
  tags: string[];
};

type FeedTag = {
  label: string;
  subject?: string;
  timestamp: number;
  occurrences: number;
};

type HomeFeed = {
  scope: string;
  popularity_metric: string;
  recent: FeedDocument[];
  popular: FeedDocument[];
  recent_tags: FeedTag[];
  popular_tags: FeedTag[];
};

const FEED_UNAVAILABLE = "Feed is temporarily unavailable.";

let homeRefreshTimer: number | undefined;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}

function isFeedDocument(value: unknown): value is FeedDocument {
  return (
    isRecord(value) &&
    typeof value.name === "string" &&
    typeof value.hash === "string" &&
    isFiniteNumber(value.updated_at) &&
    isFiniteNumber(value.tag_count) &&
    isStringArray(value.tags)
  );
}

function isFeedTag(value: unknown): value is FeedTag {
  return (
    isRecord(value) &&
    typeof value.label === "string" &&
    (value.subject === undefined || typeof value.subject === "string") &&
    isFiniteNumber(value.timestamp) &&
    isFiniteNumber(value.occurrences)
  );
}

function isFeedDocumentArray(value: unknown): value is FeedDocument[] {
  return Array.isArray(value) && value.every(isFeedDocument);
}

function isFeedTagArray(value: unknown): value is FeedTag[] {
  return Array.isArray(value) && value.every(isFeedTag);
}

/**
 * Validate an untrusted `/api/home/feed` body. Returns `null` for any payload that
 * does not match the server contract, so callers never render a partial feed.
 */
export function parseHomeFeed(value: unknown): HomeFeed | null {
  if (!isRecord(value)) return null;
  if (typeof value.scope !== "string" || typeof value.popularity_metric !== "string") return null;
  if (!isFeedDocumentArray(value.recent) || !isFeedDocumentArray(value.popular)) return null;
  if (!isFeedTagArray(value.recent_tags) || !isFeedTagArray(value.popular_tags)) return null;
  return {
    scope: value.scope,
    popularity_metric: value.popularity_metric,
    recent: value.recent,
    popular: value.popular,
    recent_tags: value.recent_tags,
    popular_tags: value.popular_tags,
  };
}

function renderList(list: HTMLOListElement, documents: FeedDocument[]): void {
  if (!isFeedDocumentArray(documents)) return;
  list.replaceChildren();
  if (documents.length === 0) {
    const empty = document.createElement("li");
    empty.textContent = "No publicly tagged local documents yet.";
    list.append(empty);
    return;
  }
  for (const documentInfo of documents) {
    const row = document.createElement("li");
    const link = document.createElement("a");
    link.href = `/edit/${encodeURIComponent(documentInfo.name)}`;
    link.dataset.nav = "";
    link.textContent = documentInfo.name;
    const detail = document.createElement("span");
    detail.textContent = ` — ${documentInfo.tag_count} global tags`;
    if (documentInfo.tags.length > 0) {
      const tags = document.createElement("span");
      tags.textContent = ` (${documentInfo.tags.join(", ")})`;
      detail.append(tags);
    }
    row.append(link, detail);
    list.append(row);
  }
}

/** Populate a tag list using text nodes; tag values are untrusted user metadata. */
export function renderTagList(list: HTMLOListElement, tags: FeedTag[]): void {
  if (!isFeedTagArray(tags)) return;
  list.replaceChildren();
  if (tags.length === 0) {
    const empty = document.createElement("li");
    empty.textContent = "No public local tags yet.";
    list.append(empty);
    return;
  }
  for (const tag of tags) {
    const row = document.createElement("li");
    if (tag.subject) {
      const link = document.createElement("a");
      link.href = `/edit/${encodeURIComponent(tag.subject)}`;
      link.dataset.nav = "";
      link.textContent = tag.label;
      row.append(link);
      const detail = document.createElement("span");
      const time = new Date(tag.timestamp / 1000).toLocaleString();
      detail.textContent = ` — ${tag.subject} · ${time}`;
      row.append(detail);
    } else {
      const label = document.createElement("span");
      label.textContent = tag.label;
      const count = document.createElement("span");
      count.textContent = ` — ${tag.occurrences} documents`;
      row.append(label, count);
    }
    list.append(row);
  }
}

export function initHome(): void {
  if (homeRefreshTimer !== undefined) window.clearTimeout(homeRefreshTimer);
  homeRefreshTimer = undefined;
  const root = document.querySelector<HTMLElement>("[data-home-app]");
  if (!root || root.dataset.initialized === "true") return;
  root.dataset.initialized = "true";
  const recent = root.querySelector<HTMLOListElement>("[data-home-recent]");
  const popular = root.querySelector<HTMLOListElement>("[data-home-popular]");
  const recentTags = root.querySelector<HTMLOListElement>("[data-home-recent-tags]");
  const popularTags = root.querySelector<HTMLOListElement>("[data-home-popular-tags]");
  const updated = root.querySelector<HTMLElement>("[data-home-updated]");
  if (!recent || !popular || !recentTags || !popularTags || !updated) return;

  let pending = false;
  const refresh = async () => {
    if (pending || document.visibilityState === "hidden") return;
    pending = true;
    try {
      const response = await fetch("/api/home/feed", { headers: { accept: "application/json" } });
      if (!response.ok) throw new Error(`feed request failed (${response.status})`);
      const feed = parseHomeFeed(await response.json());
      if (!root.isConnected) return;
      if (feed === null) {
        updated.textContent = FEED_UNAVAILABLE;
        return;
      }
      renderList(recent, feed.recent);
      renderList(popular, feed.popular);
      renderTagList(recentTags, feed.recent_tags);
      renderTagList(popularTags, feed.popular_tags);
      updated.textContent = `Updated live from ${feed.scope}. ${feed.popularity_metric}.`;
    } catch (error) {
      if (root.isConnected) {
        updated.textContent =
          error instanceof Error && error.message.startsWith("feed request failed (")
            ? error.message
            : FEED_UNAVAILABLE;
      }
    } finally {
      pending = false;
    }
  };
  const poll = async () => {
    await refresh();
    if (root.isConnected) homeRefreshTimer = window.setTimeout(() => void poll(), 15_000);
  };
  void poll();
}
