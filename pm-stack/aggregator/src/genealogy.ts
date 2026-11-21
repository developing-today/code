import type { ProjectPoolItem, RefKey, RoadmapRow } from "./model.ts";

export type GenealogyRelation = "parent" | "historical-parent" | "primary-parent" | "spawned-by" | "roadmap" | "priority";

export interface GenealogyAncestor {
  ref: RefKey;
  title: string;
  kind: string | null;
  relation: GenealogyRelation;
}

export interface GenealogyRelease {
  title: string;
  dueOn: string | null;
  ref: RefKey;
}

export interface ItemGenealogy {
  ancestors: GenealogyAncestor[];
  release: GenealogyRelease | null;
  afterRefs: RefKey[];
  sentence: string;
}

const labelTarget = (labels: string[], prefix: string): string | null =>
  labels.find((label) => label.startsWith(prefix))?.slice(prefix.length) ?? null;

/** Derive the item's known genealogy using only verified pool edges and labels. */
export function deriveGenealogy(item: ProjectPoolItem, pool: readonly ProjectPoolItem[]): ItemGenealogy {
  // Board memberships are separate pool rows, but Kind is canonical issue
  // metadata. Preserve a populated Kind when a later satellite copy omits it.
  const byRef = new Map<RefKey, ProjectPoolItem>();
  for (const entry of pool) {
    const existing = byRef.get(entry.ref);
    byRef.set(entry.ref, existing ? { ...existing, kind: existing.kind ?? entry.kind } : entry);
  }
  const chain: ProjectPoolItem[] = [];
  const seen = new Set<RefKey>([item.ref]);
  let cursor = item.parentRef ? byRef.get(item.parentRef) : undefined;
  while (cursor && !seen.has(cursor.ref)) {
    chain.push(cursor);
    seen.add(cursor.ref);
    cursor = cursor.parentRef ? byRef.get(cursor.parentRef) : undefined;
  }

  const ancestors: GenealogyAncestor[] = [];
  const add = (entry: ProjectPoolItem | undefined, relation: GenealogyRelation) => {
    if (entry && !ancestors.some((ancestor) => ancestor.ref === entry.ref && ancestor.relation === relation)) {
      ancestors.push({ ref: entry.ref, title: entry.title, kind: entry.kind, relation });
    }
  };
  for (const entry of chain) add(entry, "parent");
  const historicalParents = (item.historicalParentRefs ?? []).flatMap((ref) => {
    const former = byRef.get(ref);
    if (!former) return [];
    add(former, "historical-parent");
    return [former];
  });

  const nearestEpic = chain.find((entry) => entry.kind === "Epic");
  const primarySlug = labelTarget(item.labels ?? [], "primary-parent/");
  const primaryEpic = primarySlug
    ? pool.find((entry) => (entry.labels ?? []).includes(`epic/${primarySlug}`) && entry.kind === "Epic")
    : undefined;
  add(primaryEpic ?? nearestEpic, "primary-parent");

  const design =
    chain.find((entry) => (entry.labels ?? []).includes("kind/design") || entry.kind === "Design") ??
    (() => {
      const designLabel = labelTarget(item.labels ?? [], "design/");
      return designLabel
        ? pool.find(
            (entry) =>
              (entry.labels ?? []).includes(`design/${designLabel}`) &&
              ((entry.labels ?? []).includes("kind/design") || entry.kind === "Design"),
          )
        : undefined;
    })();
  add(design, "spawned-by");

  const roadmapItem = chain.find(
    (entry) => (entry.labels ?? []).includes("kind/roadmap-item") || entry.kind === "Roadmap item",
  );
  add(roadmapItem, "roadmap");
  const priority = roadmapItem
    ? chain.find(
        (entry) =>
          entry.ref !== roadmapItem.ref &&
          ((entry.labels ?? []).includes("kind/roadmap-item") || entry.kind === "Roadmap item"),
      )
    : undefined;
  add(priority, "priority");

  const releaseItem = priority ?? roadmapItem;
  const release = releaseItem?.milestone
    ? { title: releaseItem.milestone.title, dueOn: releaseItem.milestone.dueOn, ref: releaseItem.ref }
    : null;
  const afterRefs = [...new Set((roadmapItem ?? priority ?? item).blockedBy ?? [])];
  const epicPhrase = nearestEpic ? `as part of epic ${nearestEpic.title} (${nearestEpic.ref})` : "with no epic yet";
  const designPhrase = design ? `, spawned by design ${design.title} (${design.ref})` : ", with no design yet";
  const roadmapPhrase = roadmapItem
    ? `; on roadmap item ${roadmapItem.title} (${roadmapItem.ref})`
    : "; with no roadmap item yet";
  const priorityPhrase = priority ? `, overall priority ${priority.title} (${priority.ref})` : "";
  const releasePhrase = release ? `; release ${release.title}${release.dueOn ? ` due ${release.dueOn}` : ""}` : "";
  const historicalPhrase = historicalParents.length
    ? ` ${historicalParents.map((former) => `Formerly part of ${former.title} (${former.ref})`).join(". ")}.`
    : "";
  return {
    ancestors,
    release,
    afterRefs,
    sentence: `Working on ${item.title} ${epicPhrase}${designPhrase}${roadmapPhrase}${priorityPhrase}${releasePhrase}.${historicalPhrase}`,
  };
}

/** Attach genealogy overlay fields to legacy Grist/repo roadmap rows when their links resolve. */
export function enrichRoadmapRows(rows: readonly RoadmapRow[], pool: readonly ProjectPoolItem[]) {
  const byRef = new Map(pool.map((item) => [item.ref, item]));
  return rows.map((row) => {
    const linked = [...row.links]
      .sort((a, b) => Number(b.relation === "tracks") - Number(a.relation === "tracks"))
      .map((link) => byRef.get(link.ref))
      .find((item): item is ProjectPoolItem => Boolean(item));
    const genealogy = linked ? deriveGenealogy(linked, pool) : null;
    const roadmapRef = genealogy?.ancestors.find((ancestor) => ancestor.relation === "roadmap")?.ref;
    const priorityRef = genealogy?.ancestors.find((ancestor) => ancestor.relation === "priority")?.ref ?? roadmapRef;
    return {
      ...row,
      priority: priorityRef ?? null,
      afterRefs: genealogy?.afterRefs ?? [],
      releaseRef: genealogy?.release?.ref ?? null,
    };
  });
}
