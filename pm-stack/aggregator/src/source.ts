// The one interface every data source implements. The views never know
// whether they are looking at fixtures or live readers — swapping one for
// the other is a config change (PM_AGG_SOURCE), not a code change.
import type {
  BoardItem,
  Comment,
  DnsSection,
  GhItem,
  OrchestrationSection,
  Provenance,
  QuotaView,
  RefKey,
  UsageView,
  RoadmapRow,
  SessionFeedEntry,
  SessionRecord,
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
  /** Usage & billing hub view: family-2 SaaS services, PAYG lines,
   * charges evidence, bubble flags, and snapshot history (hub §3). */
  usage(): Promise<UsageView>;
  /** Raw session feed for the "needs you" view (#852), pre-classification. */
  attention(): Promise<SessionFeedEntry[]>;
  /** The orchestration job's latest section (#899); null = not measured yet. */
  orchestration(): Promise<OrchestrationSection | null>;
  /** The DNS/domain register (#902) for the register page; null = not collected yet. */
  dns(): Promise<DnsSection | null>;
  /** The identity registry (#906 §3): durable session records, oldest sources merged. */
  registry(): Promise<SessionRecord[]>;
  /** Which session sources are wired, for the coverage note (slice 2). */
  coverage(): Promise<SourceCoverage[]>;
  provenance(): Provenance;
}
