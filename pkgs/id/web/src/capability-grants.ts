import type { GrantPageWorld } from "./world-http";

const HISTORY_LABELS: Record<GrantPageWorld["history"][number]["kind"], string> = {
  accept: "Remembered after connecting",
  narrow: "Narrowed after a permission was removed",
  pin: "Pinned by an invite",
  "grant-page-edit": "Edited from this page",
  "yolo-auto-accept": "Updated while automatic approval was enabled",
};

/** Render only the Rust listing's client-facing messages and transition history. */
export function renderCapabilityGrants(root: HTMLElement, worlds: GrantPageWorld[]): void {
  root.replaceChildren();
  if (worlds.length === 0) {
    const empty = document.createElement("p");
    empty.className = "text-sm opacity-70";
    empty.textContent = "No remembered permission decisions yet.";
    root.append(empty);
    return;
  }

  for (const world of worlds) {
    const article = document.createElement("article");
    article.className = "rounded border border-base-300 p-3 space-y-2";
    const heading = document.createElement("h3");
    heading.className = "font-semibold";
    heading.textContent = world.world_id;
    article.append(heading);

    const permissions = document.createElement("ul");
    permissions.className = "list-disc pl-5";
    if (world.current.length === 0) {
      const none = document.createElement("li");
      none.textContent = "No remembered permissions";
      permissions.append(none);
    } else {
      for (const line of world.current) {
        const item = document.createElement("li");
        item.textContent = line.message;
        permissions.append(item);
      }
    }
    article.append(permissions);

    const history = document.createElement("details");
    const summary = document.createElement("summary");
    summary.textContent = `Decision history (${world.history.length})`;
    history.append(summary);
    const entries = document.createElement("ol");
    entries.className = "list-decimal pl-5";
    for (const entry of world.history) {
      const item = document.createElement("li");
      const count = entry.message_set.length;
      item.textContent = `${entry.at}: ${HISTORY_LABELS[entry.kind]} (${count} permission${count === 1 ? "" : "s"})`;
      entries.append(item);
    }
    history.append(entries);
    article.append(history);
    root.append(article);
  }
}
