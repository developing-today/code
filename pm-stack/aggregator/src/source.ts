// The one interface every data source implements. The views never know
// whether they are looking at fixtures or live readers — swapping one for
// the other is a config change (PM_AGG_SOURCE), not a code change.
import type { SectionDiagnosis } from "./feed.ts";
import type { QueueSnapshot } from "./queue.ts";
import type { Phase12Snapshot } from "./phase12.ts";
import type { PhaseMappingSnapshot } from "./phase-mapping.ts";
import type { BuildHealthSnapshot } from "./build-health.ts";
import type { F12PrRef } from "./f12-readiness.ts";
import type { NeedsDrewCensus } from "./needs-drew.ts";
import type { RepoCensus } from "./coverage.ts";
import type {
  AaFeedSection,
  BoardItem,
  Comment,
  DnsSection,
  GhItem,
  OrchestrationSection,
  ProjectPoolItem,
  ProjectAudit,
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
  /** The latest comment per ref, guaranteed per ref (#1279): the bulk
   * comments() read applies one row limit across all requested refs, so
   * a busy batch can omit an item's latest comment entirely. Briefs only
   * need the latest, and need it for every item. Optional — callers fall
   * back to comments() when a source does not provide it. */
  latestComments?(refs: RefKey[]): Promise<Comment[]>;
  /** Quota + throughput view (#851). */
  quota(): Promise<QuotaView>;
  /** Usage & billing hub view: family-2 SaaS services, PAYG lines,
   * charges evidence, bubble flags, and snapshot history (hub §3). Read-only. */
  usage(): Promise<UsageView>;
  /** Land the latest feed-carried usage snapshots into the local store.
   * Optional: live mode runs this from its background writer tick, never
   * as a side effect of a page/read request. */
  ingestSnapshots?(): Promise<number>;
  /** Raw session feed for the "needs you" view (#852), pre-classification. */
  attention(): Promise<SessionFeedEntry[]>;
  /** The orchestration job's latest section (#899); null = not measured yet. */
  orchestration(): Promise<OrchestrationSection | null>;
  /** The DNS/domain register (#902) for the register page; null = not collected yet. */
  dns(): Promise<DnsSection | null>;
  /** The AA cost explorer section (ainventory import data via the
   * fleet feed); null = not collected yet. The absent form
   * ({status:"absent"}) is a real section: it renders its reason.
   * `batch` selects a cached historical workbook version (#3302);
   * omitted keeps the latest/default behaviour. */
  aa(batch?: string | null): Promise<AaFeedSection | null>;
  /** The identity registry (#906 §3): durable session records, oldest sources merged. */
  registry(): Promise<SessionRecord[]>;
  /** The GitHub Project pool (#910 stage 5) from the fleet feed; empty when not collected. */
  projects(): Promise<ProjectPoolItem[]>;
  /** Scheduled Project #4 audit snapshot; null when unavailable or malformed. */
  projectAudit?(): Promise<ProjectAudit | null>;
  /** The collector's per-repo backlog census (#1220), when the source
   * carries one; null = not collected. Optional so fixtures and test
   * doubles keep working — the coverage panel degrades honestly. */
  coverageCensus?(): Promise<RepoCensus[] | null>;
  /** Whether the Project pool was actually read (#1220): false when the
   * feed carried no projects section, so coverage renders the pool as
   * unavailable instead of zero. Optional, like coverageCensus. */
  projectsAvailable?(): Promise<boolean>;
  /** The Needs-Drew label census (#1226): open user/review +
   *  user/escalated items across the four repos, labels as a read-only
   *  source. Optional like coverageCensus: sources that cannot census
   *  omit it and the attention view renders the census as unavailable
   *  instead of as an empty queue. */
  needsDrew?(): Promise<NeedsDrewCensus | null>;
  /** The #1235 issue-body snapshot for the Path to Phase 12 panel
   * (#9922); null = not collected / the body feed could not be read.
   * Optional like needsDrew: sources that cannot read issue bodies
   * omit it and the panel renders unavailable instead of inventing
   * a step list. */
  phase12?(): Promise<Phase12Snapshot | null>;
  /** The Phase tracker F-item mapping snapshot (#9923 step 3),
   * parsed from planning/roadmaps/id.md in the code checkout;
   * null = unavailable. Optional like phase12: sources that cannot
   * read the checkout omit it and the panel withholds mapped
   * progress instead of inventing it. */
  phaseMapping?(): Promise<PhaseMappingSnapshot | null>;
  /** The Actions-API build-health snapshot for the id section's
   * Build health panel (#9924); null = not collected / unavailable.
   * Optional like phase12: sources that cannot read the Actions API
   * omit it and the panel renders unavailable. */
  buildHealth?(): Promise<BuildHealthSnapshot | null>;
  /** The #534 issue-body snapshot for the Phase 12 readiness panel's
   * design column (#9926); null = unavailable. Optional like
   * phase12: sources that cannot read issue bodies omit it. */
  phase12Design?(): Promise<Phase12Snapshot | null>;
  /** PR cross-references per issue number, from issue timelines, for
   * the readiness panel's PR column (#9926); null = the timeline
   * read is unavailable. Optional like phase12Design. */
  f12PrRefs?(): Promise<Map<number, F12PrRef[]> | null>;
  /** The read-only fleet-queue collector snapshot (#9920): per-queue
   * counts + task rows from the queue-broker's /health and /list
   * read verbs, with the source timestamp and stale/unavailable
   * state on the payload. Optional like phase12: sources that
   * cannot read the broker omit it and the panels render the queue
   * source as unavailable instead of inventing counts. */
  queue?(): Promise<QueueSnapshot | null>;
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
