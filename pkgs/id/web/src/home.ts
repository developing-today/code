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

let homeRefreshTimer: number | undefined;

function renderList(list: HTMLOListElement, documents: FeedDocument[]): void {
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
      const feed = (await response.json()) as HomeFeed;
      if (!root.isConnected) return;
      renderList(recent, feed.recent);
      renderList(popular, feed.popular);
      renderTagList(recentTags, feed.recent_tags);
      renderTagList(popularTags, feed.popular_tags);
      updated.textContent = `Updated live from ${feed.scope}. ${feed.popularity_metric}.`;
    } catch (error) {
      if (root.isConnected) {
        updated.textContent = error instanceof Error ? error.message : "Feed is temporarily unavailable.";
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
