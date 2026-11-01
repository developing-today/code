// Environment configuration. Secrets are read by NAME from the environment
// only — never committed, never printed. Everything has a fixture-safe default.
import { homedir } from "node:os";
import { join } from "node:path";

const s = (v: string | undefined, d: string): string => (v && v.trim() !== "" ? v.trim() : d);

export interface Config {
  /** "fixtures" | "live" — the default Source. */
  source: "fixtures" | "live";
  port: number;
  /** Bind addresses (PM_AGG_HOSTS, comma-separated). Default loopback. */
  hosts: string[];
  /** Fleet feed file written by scripts/collect-fleet-feed.ts (#890). */
  feedFile: string;
  /** Dashboard comments store (#906 §2) — the aggregator's own writable data. */
  commentsFile: string;
  /** Dashboard actions store (#1152/#1154/#1156) — triage write path. */
  actionsFile: string;
  /** Saved-views store (#1276) — named per-person filter sets. */
  viewsFile: string;
  /** Durable identity-registry store (#906 §3), merged on every read. */
  registryFile: string;
  /** Usage snapshot store (hub §4): a directory of monthly JSONL files. */
  snapshotsDir: string;
  /** The stewardship quota view; the hub's live fallback source when the
   * fleet feed carries no saas/payg sections yet (hub §2.3). */
  quotaViewFile: string;
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

export function loadConfig(env: Record<string, string | undefined> = process.env): Config {
  return {
    source: env.PM_AGG_SOURCE === "live" ? "live" : "fixtures",
    port: Number(s(env.PM_AGG_PORT, "4870")),
    hosts: s(env.PM_AGG_HOSTS, "127.0.0.1")
      .split(",")
      .map((h) => h.trim())
      .filter(Boolean),
    feedFile: s(env.PM_AGG_FEED_FILE, join(process.cwd(), "fleet-feed.json")),
    commentsFile: s(env.PM_AGG_COMMENTS_FILE, join(process.cwd(), "comments.json")),
    actionsFile: s(env.PM_AGG_ACTIONS_FILE, join(process.cwd(), "actions.json")),
    viewsFile: s(env.PM_AGG_VIEWS_FILE, join(process.cwd(), "views.json")),
    registryFile: s(env.PM_AGG_REGISTRY_FILE, join(process.cwd(), "registry.json")),
    snapshotsDir: s(env.PM_AGG_SNAPSHOTS_DIR, join(process.cwd(), "snapshots")),
    quotaViewFile: s(
      env.PM_AGG_QUOTA_VIEW,
      join(
        homedir(),
        "workspace/goals/stewardship-commission-recurring-review-and-dispatch/hidden_files/visibility/quota-view.md",
      ),
    ),
    cloudflareExportFile: s(env.PM_AGG_CLOUDFLARE_EXPORT, join(homedir(), "atlas/cloudflare-export/latest.json")),
    supabaseExportFile: s(env.PM_AGG_SUPABASE_EXPORT, join(homedir(), "atlas/supabase-export/latest.json")),
    quotaRegistryFile: s(
      env.PM_AGG_QUOTA_REGISTRY,
      join(homedir(), "workspace/goals/keep-the-four-repos-healthy/hidden_files/provider-quota-registry.md"),
    ),
    quotaCurrentFile: s(env.PM_AGG_QUOTA_CURRENT_FILE, join(process.cwd(), "quota-current.json")),
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
