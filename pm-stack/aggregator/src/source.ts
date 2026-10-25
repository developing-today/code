// The one interface every data source implements. The views never know
// whether they are looking at fixtures or live readers — swapping one for
// the other is a config change (PM_AGG_SOURCE), not a code change.
import type {
  BoardItem,
  Comment,
  GhItem,
  OrchestrationSummary,
  Provenance,
  QuotaView,
  RefKey,
  RoadmapRow,
  SessionFeedEntry,
  SourceCoverage,
} from "./model.ts";

export interface Source {
  /** The curated in-flight set across all four repos. */
  board(): Promise<BoardItem[]>;
  /** The Grist roadmap rows with their registry links. */
  roadmap(): Promise<RoadmapRow[]>;
  /** Live state for each ref; refs the source cannot resolve are omitted. */
  items(refs: RefKey[]): Promise<Map<RefKey, GhItem>>;
  /** The comment projection for the refs, PR-interior comments included. */
  comments(refs: RefKey[]): Promise<Comment[]>;
  /** Quota + throughput view (#851). */
  quota(): Promise<QuotaView>;
  /** Raw session feed for the "needs you" view (#852), pre-classification. */
  attention(): Promise<SessionFeedEntry[]>;
  /** The #899 orchestration summary when one exists (slice 2). */
  orchestration(): Promise<OrchestrationSummary | null>;
  /** Which session sources are wired, for the coverage note (slice 2). */
  coverage(): Promise<SourceCoverage[]>;
  provenance(): Provenance;
}
