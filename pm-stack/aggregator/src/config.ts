// Environment configuration. Secrets are read by NAME from the environment
// only — never committed, never printed. Everything has a fixture-safe
// default, except the default Source: implicit fixtures require the
// explicit PM_AGG_DEV=1 switch (#1195).
import { homedir } from "node:os";
import { join } from "node:path";

const s = (v: string | undefined, d: string): string => (v && v.trim() !== "" ? v.trim() : d);

function parseAggregatorPort(value: string | undefined, name = "PM_AGG_PORT", defaultValue = "4870"): number {
  const normalized = s(value, defaultValue);
  const port = Number(normalized);
  if (!/^\d+$/.test(normalized) || !Number.isSafeInteger(port) || port < 1 || port > 65535) {
    throw new Error(`Invalid ${name}=${JSON.stringify(value)}; expected a decimal integer in range 1-65535`);
  }
  return port;
}

function parseAggregatorHosts(value: string | undefined): string[] {
  const hosts = s(value, "127.0.0.1")
    .split(",")
    .map((host) => host.trim())
    .filter(Boolean);
  if (hosts.length === 0) {
    throw new Error(`Invalid PM_AGG_HOSTS=${JSON.stringify(value)}; expected at least one non-empty host`);
  }
  return hosts;
}

export interface Config {
  /** Default Source: "live" when PM_AGG_SOURCE=live; "fixtures" when
   * explicitly requested, or when PM_AGG_DEV=1 allows the implicit
   * (unset/invalid PM_AGG_SOURCE) fixture default. Anything else
   * throws at startup rather than silently serving canned data (#1195). */
  source: "fixtures" | "live";
  port: number;
  /** Bind addresses (PM_AGG_HOSTS, comma-separated). Default loopback. */
  hosts: string[];
  /** Fleet feed file written by scripts/collect-fleet-feed.ts (#890). */
  feedFile: string;
  /** Dashboard comments store (#906 §2) — the aggregator's own writable data. */
  commentsFile: string;
  /** Append-only roadmap annotation store (#1284), separate from comments. */
  roadmapAnnotationsFile: string;
  /** Dashboard actions store (#1152/#1154/#1156) — triage write path. */
  actionsFile: string;
  /** Saved-views store (#1276) — named per-person filter sets. */
  viewsFile: string;
  /** Cross-stack event log store (#1739) — one chronological stream. */
  eventsFile: string;
  /** Source freshness invalidation journal, separate from #1739/#3321. */
  freshnessFile: string;
  /** Loopback-only publisher listener; never included in PM_AGG_HOSTS. */
  freshnessPort: number;
  activityFile: string;
  /** Durable identity-registry store (#906 §3), merged on every read. */
  registryFile: string;
  /** Usage snapshot store (hub §4): a directory of monthly JSONL files. */
  snapshotsDir: string;
  /** Scheduled-digest archive (#1278): one JSON file per digest. */
  digestsDir: string;
  /** Append-only notification state/send/suppression log (#3317). */
  notificationsFile: string;
  /** Versioned item-history JSONL (#3981) the forecast contract
   * (#3984) derives from: one cutoff per line, read via
   * ItemHistoryStore. */
  itemHistoryFile: string;
  /** The stewardship quota view; the hub's live fallback source when the
   * fleet feed carries no saas/payg sections yet (hub §2.3). */
  quotaViewFile: string;
  /** Tracker of evaluated free services without automated quota cards (#1172). */
  freeServicesTrackerFile: string;
  cloudflareExportFile: string;
  /** The Supabase usage exporter's document (#1050): the gathered live read
   * preferred over the quota view's Supabase prose when fresh. */
  supabaseExportFile: string;
  /** The provider quota registry; the hub's second dated-read source
   * (billing incidents) for the same fallback path. */
  quotaRegistryFile: string;
  /** The quota gatherer's latest-state file (#1151), written into the
   * aggregator's own data dir by scripts/gather-quota.ts. */
  quotaCurrentFile: string;
  coverageSnapshotFile: string;
  /** OpenChamber managed-opencode state file (laptop attention source). */
  openchamberStateFile: string;
  kaneo: {
    baseUrl: string;
    /** Kaneo's client URL — the Origin its auth accepts (better-auth). */
    origin: string;
    email: string | null;
    password: string | null;
    /** Authoritative project slug -> repo registry (#1201):
     * pm-stack/kaneo-projects.json. Unknown slugs stay unmapped. */
    projectsFile: string;
  };
  grist: {
    baseUrl: string;
    apiKey: string | null;
    docId: string;
  };
  projector: {
    baseUrl: string | null;
  };
  /** Probe targets for the /health page (#1147): the stack services the
   * aggregator does not otherwise read. Kaneo/Grist/projector targets
   * already live in their own sections above. */
  pad: {
    baseUrl: string;
  };
  readyForAgent: {
    baseUrl: string;
  };
  /** Canonical HTTPS origin for the PWA + push deep links (#3316 §4):
   * Tailscale Serve fronts the aggregator at this origin; the
   * plain-HTTP tailnet listener is not a secure context and can
   * never hold a service worker or a push subscription. */
  publicOrigin: string;
  /** Alert throat (#3676): escalation ack window + engine cadence. */
  alerts: {
    ackWindowMin: number;
    tickMs: number;
  };
}

function parseAggregatorSource(env: Record<string, string | undefined>): "fixtures" | "live" {
  const raw = env.PM_AGG_SOURCE;
  const value = raw?.trim() ?? "";
  // #1195: an explicit choice is always honored — "live" for the
  // deployment, "fixtures" for deliberate fixture work.
  if (value === "live" || value === "fixtures") return value;
  // Anything else (unset, empty, or a typo) silently serving fixtures
  // would let a non-development deployment look healthy while showing
  // canned data, so fixture mode here requires the explicit dev switch.
  if (env.PM_AGG_DEV === "1") return "fixtures";
  throw new Error(
    `Refusing to serve fixture data outside explicit dev mode: PM_AGG_SOURCE=${JSON.stringify(raw ?? null)}; ` +
      `set PM_AGG_SOURCE=live, or PM_AGG_SOURCE=fixtures with PM_AGG_DEV=1`,
  );
}

export function loadConfig(env: Record<string, string | undefined> = process.env): Config {
  return {
    source: parseAggregatorSource(env),
    port: parseAggregatorPort(env.PM_AGG_PORT),
    hosts: parseAggregatorHosts(env.PM_AGG_HOSTS),
    feedFile: s(env.PM_AGG_FEED_FILE, join(process.cwd(), "fleet-feed.json")),
    commentsFile: s(env.PM_AGG_COMMENTS_FILE, join(process.cwd(), "comments.json")),
    roadmapAnnotationsFile: s(env.PM_AGG_ROADMAP_ANNOTATIONS_FILE, join(process.cwd(), "roadmap-annotations.json")),
    actionsFile: s(env.PM_AGG_ACTIONS_FILE, join(process.cwd(), "actions.json")),
    viewsFile: s(env.PM_AGG_VIEWS_FILE, join(process.cwd(), "views.json")),
    eventsFile: s(env.PM_AGG_EVENTS_FILE, join(process.cwd(), "events.json")),
    freshnessFile: s(
      env.PM_AGG_FRESHNESS_FILE,
      join(homedir(), ".local", "state", "pm-aggregator", "freshness.json"),
    ),
    freshnessPort: parseAggregatorPort(env.PM_AGG_FRESHNESS_PORT, "PM_AGG_FRESHNESS_PORT", "4871"),
    // Default is the state dir, NOT cwd (#5389): aggregator runtime state
    // lives in ~/.local/state/pm-aggregator (2026-10-27 precedent), and the
    // cwd default let an env-less run write this store into whatever
    // checkout it started from -- in the primary clone that dirtied the
    // tree and blocked pm-git-sync. install.sh renders PM_AGG_ACTIVITY_FILE
    // to this same path for the unit.
    activityFile: s(
      env.PM_AGG_ACTIVITY_FILE,
      join(homedir(), ".local", "state", "pm-aggregator", "activity-ingestion.json"),
    ),
    registryFile: s(env.PM_AGG_REGISTRY_FILE, join(process.cwd(), "registry.json")),
    snapshotsDir: s(env.PM_AGG_SNAPSHOTS_DIR, join(process.cwd(), "snapshots")),
    digestsDir: s(env.PM_AGG_DIGESTS_DIR, join(process.cwd(), "digests")),
    notificationsFile: s(
      env.PM_AGG_NOTIFICATIONS_FILE,
      join(homedir(), ".local", "state", "pm-aggregator", "notifications.jsonl"),
    ),
    quotaViewFile: s(
      env.PM_AGG_QUOTA_VIEW,
      join(
        homedir(),
        "workspace/goals/stewardship-commission-recurring-review-and-dispatch/hidden_files/visibility/quota-view.md",
      ),
    ),
    freeServicesTrackerFile: s(
      env.PM_AGG_FREE_SERVICES_TRACKER,
      join(homedir(), "hatch-personal/workspace/ai-subscriptions/free-services-to-check.md"),
    ),
    cloudflareExportFile: s(env.PM_AGG_CLOUDFLARE_EXPORT, join(homedir(), "atlas/cloudflare-export/latest.json")),
    supabaseExportFile: s(env.PM_AGG_SUPABASE_EXPORT, join(homedir(), "atlas/supabase-export/latest.json")),
    quotaRegistryFile: s(
      env.PM_AGG_QUOTA_REGISTRY,
      join(homedir(), "workspace/goals/keep-the-four-repos-healthy/hidden_files/provider-quota-registry.md"),
    ),
    quotaCurrentFile: s(env.PM_AGG_QUOTA_CURRENT_FILE, join(process.cwd(), "quota-current.json")),
    itemHistoryFile: s(
      env.PM_AGG_ITEM_HISTORY_FILE,
      join(homedir(), ".local", "state", "pm-aggregator", "item-history.jsonl"),
    ),
    coverageSnapshotFile: s(env.PM_AGG_COVERAGE_SNAPSHOT_FILE, join(process.cwd(), "coverage-snapshot.json")),
    openchamberStateFile: s(
      env.PM_AGG_OPENCHAMBER_STATE,
      join(homedir(), ".config", "openchamber", "managed-opencode", "state.json"),
    ),
    kaneo: {
      baseUrl: s(env.KANEO_URL, "http://127.0.0.1:5173"),
      // Kaneo's auth accepts only its configured client URL as Origin.
      origin: s(env.KANEO_ORIGIN, s(env.KANEO_CLIENT_URL, "https://kaneo.security.cab")),
      email: env.KANEO_ADMIN_EMAIL ?? null,
      password: env.KANEO_ADMIN_PASSWORD ?? null,
      // The registry is authoritative for slug -> repo (#1201); the reader
      // runs with cwd=pm-stack/aggregator, hence the sibling path.
      projectsFile: s(env.PM_KANEO_PROJECTS_FILE, join(process.cwd(), "..", "kaneo-projects.json")),
    },
    grist: {
      baseUrl: s(env.GRIST_URL, "http://127.0.0.1:8484"),
      apiKey: env.GRIST_API_KEY ?? null,
      // The "Program roadmap" document seeded in Phase 2.
      docId: s(env.GRIST_ROADMAP_DOC, "czYHYKgG9SHcm9CPwiKEq1"),
    },
    projector: {
      // Session J's projector serves on 127.0.0.1:7801 by default; set
      // PM_PROJECTOR_URL=off to disable the live registry/item/comment reads.
      baseUrl:
        env.PM_PROJECTOR_URL === "off" || env.PM_PROJECTOR_URL === "none"
          ? null
          : s(env.PM_PROJECTOR_URL, "http://127.0.0.1:7801"),
    },
    pad: {
      baseUrl: s(env.PM_AGG_PAD_URL, "http://127.0.0.1:7777"),
    },
    readyForAgent: {
      baseUrl: s(env.PM_AGG_RFA_URL, "http://127.0.0.1:6056"),
    },
    publicOrigin: s(env.PM_AGG_PUBLIC_ORIGIN, "https://amd-6.raptor-pumpkinseed.ts.net").replace(/\/+$/, ""),
    alerts: {
      ackWindowMin: Math.max(1, Math.floor(Number(s(env.ALERT_ACK_WINDOW_MIN, "15"))) || 15),
      tickMs: 60_000,
    },
  };
}
