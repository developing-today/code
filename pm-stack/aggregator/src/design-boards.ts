// #8864: read-only board-family projection over collector snapshots. No I/O.
import { isIsoDate, isRecord, isTimestampNotFuture } from "./validate.ts";

export interface BoardEntry {
  id: string;
  title: string;
  url: string;
  status: string | null;
  area: string | null;
  theme: string | null;
  repo: string | null;
  milestone: string | null;
  labels: string[];
  start: string | null;
  target: string | null;
}
export interface BoardSnapshot {
  number: number;
  url: string;
  generatedAt: string;
  totalItems: number;
  views: { id: string; name: string; layout: string; filter: string | null }[];
  items: BoardEntry[];
}
export const DESIGN_VIEWS = [
  "Design Register",
  "Designs — Atlas",
  "Designs — ID & fleet",
  "Designs — PM stack",
  "Designs — Cloudflare",
  "Designs — Turso",
  "Designs — Adjacent infra",
];
export const ROADMAP_VIEWS = [
  "Roadmap — code",
  "Roadmap — opencode",
  "Roadmap — openchamber",
  "Roadmap — darwin-nix",
  "Roadmap — hatch-personal (Atlas)",
];
const safeUrl = (v: unknown): v is string => typeof v === "string" && /^https:\/\/github\.com\//.test(v);
const nullableString = (v: unknown) => v === null || typeof v === "string";
const dateValue = (v: unknown) =>
  v === null || (typeof v === "string" && /^\d{4}-\d{2}-\d{2}$/.test(v) && isIsoDate(v));
export function isBoardSnapshot(v: unknown): v is BoardSnapshot {
  return (
    isRecord(v) &&
    (v.number === 4 || v.number === 5) &&
    safeUrl(v.url) &&
    isTimestampNotFuture(v.generatedAt) &&
    Number.isInteger(v.totalItems) &&
    (v.totalItems as number) >= 0 &&
    Array.isArray(v.views) &&
    v.views.every(
      (w) =>
        isRecord(w) &&
        typeof w.id === "string" &&
        typeof w.name === "string" &&
        typeof w.layout === "string" &&
        nullableString(w.filter),
    ) &&
    Array.isArray(v.items) &&
    new Set(v.items.map((i) => (isRecord(i) ? i.id : null))).size === v.items.length &&
    v.items.length <= (v.totalItems as number) &&
    v.items.every(
      (i) =>
        isRecord(i) &&
        typeof i.id === "string" &&
        typeof i.title === "string" &&
        safeUrl(i.url) &&
        [i.status, i.area, i.theme, i.repo, i.milestone].every(nullableString) &&
        Array.isArray(i.labels) &&
        i.labels.every((l) => typeof l === "string") &&
        dateValue(i.start) &&
        dateValue(i.target),
    )
  );
}

/** The subset used by these named views. Unknown syntax is unavailable, never
 * an unfiltered/full-board result pretending to match the GitHub view. */
export function boardFilter(filter: string | null): ((i: BoardEntry) => boolean) | null {
  const terms: { key: string; values: string[]; negate: boolean }[] = [];
  let rest = (filter ?? "").trim();
  while (rest) {
    const m = /^(-?)(label|repo|area|theme|status):("[^"]*"|[^\s]+)(?:\s+|$)/i.exec(rest);
    if (!m || /[*!@<>]/.test(m[3]!)) return null;
    terms.push({
      key: m[2]!.toLowerCase(),
      negate: m[1] === "-",
      values: m[3]!
        .replace(/^"|"$/g, "")
        .split(",")
        .map((s) => s.toLowerCase()),
    });
    rest = rest.slice(m[0].length).trim();
  }
  return (i) =>
    terms.every((t) => {
      const values = t.key === "label" ? i.labels : [i[t.key as "repo" | "area" | "theme" | "status"] ?? ""];
      const match = values.some((v) => t.values.includes(v.toLowerCase()));
      return t.negate ? !match : match;
    });
}
export interface BoardPanel {
  name: string;
  section: "design" | "roadmap" | "ideas";
  snapshot?: BoardSnapshot;
  items: BoardEntry[];
  undatedItems: number;
  filter: string | null;
  problem: string | null;
}
export function boardFamilyPanels(snapshots: BoardSnapshot[]): BoardPanel[] {
  return [
    ...DESIGN_VIEWS.map((name) => ({ name, section: "design" as const, number: 4 })),
    ...ROADMAP_VIEWS.map((name) => ({ name, section: "roadmap" as const, number: 4 })),
    { name: "Ideas board", section: "ideas" as const, number: 5 },
  ].map((spec) => {
    const snapshot = snapshots.find((s) => s.number === spec.number);
    const view = snapshot?.views.find((v) => v.name === spec.name);
    const matches = view ? boardFilter(view.filter) : null;
    const problem = !snapshot
      ? "Board data unavailable"
      : !view
        ? "Named view unavailable in the cached board"
        : !matches
          ? "View filter unsupported; open the GitHub view"
          : null;
    const matched = matches && snapshot ? snapshot.items.filter(matches) : [];
    const items = matched.filter((i) => spec.section !== "roadmap" || i.start || i.target);
    if (spec.section === "roadmap")
      items.sort((a, b) => (a.start ?? a.target ?? "").localeCompare(b.start ?? b.target ?? ""));
    const undatedItems = matched.length - items.length;
    return {
      name: spec.name,
      section: spec.section,
      snapshot,
      items,
      undatedItems,
      filter: view?.filter ?? null,
      problem,
    };
  });
}
const esc = (v: unknown) =>
  String(v ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
export function renderDesignBoards(snapshots: BoardSnapshot[]): string {
  const panels = boardFamilyPanels(snapshots.filter(isBoardSnapshot));
  const card = (i: BoardEntry) =>
    `<li><a href="${esc(i.url)}">${esc(i.title)}</a> <span class="muted">${esc([i.status, i.area, i.repo, i.milestone].filter(Boolean).join(" · "))}</span>${i.start || i.target ? `<div>Start ${esc(i.start ?? "not set")} → Target ${esc(i.target ?? "not set")}</div>` : ""}</li>`;
  const panel = (p: BoardPanel) => {
    const groups = new Map<string, BoardEntry[]>();
    for (const i of p.items) {
      const key = (p.section === "ideas" ? i.theme : i.status) ?? "(unset)";
      groups.set(key, [...(groups.get(key) ?? []), i]);
    }
    const stamp = p.snapshot
      ? `<p class="muted">Board data as of <time datetime="${esc(p.snapshot.generatedAt)}">${esc(p.snapshot.generatedAt)}</time>${Date.now() - Date.parse(p.snapshot.generatedAt) > 15 * 60_000 ? " · stale (over 15 minutes)" : ""} · ${p.items.length} matching cards${p.section === "roadmap" ? ` with Start/Target dates · ${p.undatedItems} matching cards without dates` : ""} · ${p.snapshot.totalItems} board items${p.snapshot.totalItems > p.snapshot.items.length ? ` · ${p.snapshot.totalItems - p.snapshot.items.length} unavailable/unlinked titles` : ""}</p>`
      : "";
    return `<details class="card"><summary><strong>${esc(p.name)}</strong> · ${p.problem ? "unavailable" : `${p.items.length} cards`}${p.snapshot ? ` · <a href="${esc(p.snapshot.url)}">GitHub Project #${p.snapshot.number}</a>` : ""}</summary>${stamp}${p.problem ? `<p class="banner">${esc(p.problem)}</p>` : [...groups].map(([name, items]) => `<h4>${esc(name)} (${items.length})</h4><ul>${items.map(card).join("")}</ul>`).join("") || `<p class="muted">${p.section === "roadmap" ? "No cards with Start/Target dates in this view; undated work is not shown here." : "No matching cards in this snapshot."}</p>`}</details>`;
  };
  return `<section id="designs"><h2>Designs</h2><p class="muted">Read-only cached GitHub views. Parked and Abandoned are displayed exactly as set on the board. Some personal-repository items are outside the token’s coverage; absent cards are not proof of no work.</p>${panels
    .filter((p) => p.section !== "roadmap")
    .map(panel)
    .join("")}</section><section id="design-roadmaps"><h2>Roadmaps</h2>${panels
    .filter((p) => p.section === "roadmap")
    .map(panel)
    .join("")}</section>`;
}
