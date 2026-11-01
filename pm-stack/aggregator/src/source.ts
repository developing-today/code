// The one interface every data source implements. The views never know
// whether they are looking at fixtures or live readers — swapping one for
// the other is a config change (PM_AGG_SOURCE), not a code change.
import type { SectionDiagnosis } from "./feed.ts";
import type { RepoCensus } from "./coverage.ts";
import type {
  AaFeedSection,
  BoardItem,
  Comment,
  DnsSection,
  GhItem,
  OrchestrationSection,
  ProjectPoolItem,
  Provenance,
  ProvenanceDetail,
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
  /** The AA cost explorer section (ainventory import data via the
   * fleet feed); null = not collected yet. The absent form
   * ({status:"absent"}) is a real section: it renders its reason. */
  aa(): Promise<AaFeedSection | null>;
  /** The identity registry (#906 §3): durable session records, oldest sources merged. */
  registry(): Promise<SessionRecord[]>;
  /** The GitHub Project pool (#910 stage 5) from the fleet feed; empty when not collected. */
  projects(): Promise<ProjectPoolItem[]>;
  /** The collector's per-repo backlog census (#1220), when the source
   * carries one; null = not collected. Optional so fixtures and test
   * doubles keep working — the coverage panel degrades honestly. */
  coverageCensus?(): Promise<RepoCensus[] | null>;
  /** Whether the Project pool was actually read (#1220): false when the
   * feed carried no projects section, so coverage renders the pool as
   * unavailable instead of zero. Optional, like coverageCensus. */
  projectsAvailable?(): Promise<boolean>;
  /** Which session sources are wired, for the coverage note (slice 2). */
  coverage(): Promise<SourceCoverage[]>;
  provenance(): Provenance;
  /**
   * Structured per-section diagnosis (#1139/#1140): the honesty
   * surfaces read the orchestration section's state from here instead
   * of inferring it from a null section — absent, invalid, and stale
   * are different failures with different remedies. Optional so test
   * doubles and older sources keep working; absent = unmeasured.
   */
  diagnostics?(): { orchestration: SectionDiagnosis };
  /**
   * Structured per-facet freshness for the chips (#1138). Optional for
   * the same reason as diagnostics(); when a source does not provide
   * it, pages render without chips rather than with invented states.
   */
  provenanceDetail?(): ProvenanceDetail;
}
