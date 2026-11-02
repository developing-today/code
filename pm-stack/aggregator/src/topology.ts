// The PM stack's services & ports topology (#1137): what runs where,
// bound how, consumed by whom. This is the registry the /health page
// renders (with live probe state from src/health.ts) — one home for
// the table, linked from /program, never duplicated.
//
// Identity facts (port, bind, runs-as, purpose) are static text: they
// change only when the stack is redeployed, and the implementing PR
// re-verified every row against laptop ground truth (ss -tln,
// systemctl --user, docker ps) on 2026-10-27. Anything that can rot —
// reachability, ages, section state — is NEVER encoded here; it comes
// from probes, the feed, and the section diagnosis at render time.
// Probe targets come from config where config has them (Kaneo, Grist,
// projector) so an env override moves the probe with the reader.
import type { Config } from "./config.ts";

/** How a service's current state is established (#1147). */
export type ProbeKind =
  /** HTTP probe of loopbackUrl + healthPath against expectStatuses. */
  | "http"
  /** This very process — serving the page is the proof. */
  | "self"
  /** No listener to probe: the fleet feed file's freshness is the pulse. */
  | "feed"
  /** State is the orchestration section diagnosis, verbatim. */
  | "section"
  /** Inferred from another service's probe (named in `inferredFrom`). */
  | "inferred"
  /** Off by config: never probed, never an error. */
  | "off";

export interface ServiceDef {
  id: string;
  name: string;
  /** Listener port, or null when the component has none. */
  port: number | null;
  /** The bind as actually deployed — stated plainly, never softened. */
  bind: string;
  /** systemd unit / container / scheduler that runs it. */
  runsAs: string;
  probe: ProbeKind;
  /** Probe target for probe:"http" (from config where config has it). */
  loopbackUrl?: string;
  healthPath?: string;
  /** HTTP statuses that prove life for this service. */
  expectStatuses?: number[];
  /** Human form of expectStatuses, rendered in the table. */
  upWhen?: string;
  /** For probe:"inferred" — the service id whose probe this follows. */
  inferredFrom?: string;
  /** Who reads it. */
  consumers: string;
  /** What degrades when it is down. */
  breaksIfDown: string;
  notes?: string;
}

/**
 * The projector's topology row follows its config: a configured
 * projector (live source + a base URL) is a real service with a real
 * probe; anything else keeps the off-by-config row. The projector was
 * deployed 2026-10-27 (#1199) — before that it had never run, and this
 * row was statically "off".
 */
function projectorDef(cfg: Config): ServiceDef {
  if (cfg.source === "live" && cfg.projector.baseUrl) {
    return {
      id: "projector",
      name: "Projector",
      port: 7801,
      bind: "127.0.0.1 only",
      runsAs: "systemd user unit pm-projector.service (NixOS) — runs from the ~/git/code checkout",
      probe: "http",
      loopbackUrl: cfg.projector.baseUrl,
      healthPath: "/health",
      expectStatuses: [200],
      upWhen: "HTTP 200 on /health",
      consumers: "the aggregator's item/comment/registry projection for refs outside the Project pool",
      breaksIfDown: "refs outside the Project pool fall back to fixtures; comment streams lose PR-interior comments",
    };
  }
  return {
    id: "projector",
    name: "Projector",
    port: 7801,
    bind: "— (not running)",
    runsAs: "off by config (PM_PROJECTOR_URL=off)",
    probe: "off",
    consumers: "the aggregator's comment/registry projection (disabled while off)",
    breaksIfDown: "nothing — while off, the comment projection runs on fixtures by design",
    notes: "Off is a configuration state, not a failure.",
  };
}

export function buildServices(cfg: Config): ServiceDef[] {
  return [
    {
      id: "aggregator",
      name: "Aggregator (this dashboard)",
      port: cfg.port,
      bind: "loopback + tailnet",
      runsAs: "pm-aggregator.service (bun)",
      probe: "self",
      consumers: "the user — the single URL for the whole stack",
      breaksIfDown: "the dashboard itself; every view is gone",
    },
    {
      id: "kaneo",
      name: "Kaneo",
      port: 5173,
      bind: "loopback only · public via Cloudflare tunnel (kaneo.security.cab, pm-tunnel.service)",
      runsAs: "container pm-stack-kaneo-1",
      probe: "http",
      loopbackUrl: cfg.kaneo.baseUrl,
      healthPath: "/api/health",
      expectStatuses: [200],
      upWhen: "HTTP 200 on /api/health",
      consumers: "the aggregator's board (its curated WIP set)",
      breaksIfDown: "the board loses its live source and falls back to fixtures",
    },
    {
      id: "kaneo-postgres",
      name: "Kaneo Postgres",
      port: 5432,
      bind: "container-internal only (no host listener)",
      runsAs: "container pm-stack-postgres-1",
      probe: "inferred",
      inferredFrom: "kaneo",
      consumers: "Kaneo only",
      breaksIfDown: "Kaneo is down with it — its state is inferred from Kaneo's probe",
    },
    {
      id: "ready-for-agent",
      name: "ready-for-agent",
      port: 6056,
      bind: "0.0.0.0 — every interface, incl. LAN (bind scope tracked in #1181)",
      runsAs: "ready-for-agent.service",
      probe: "http",
      loopbackUrl: cfg.readyForAgent.baseUrl,
      healthPath: "/",
      expectStatuses: [200],
      upWhen: "HTTP 200",
      consumers: "GitHub issues across the fleet (label-driven dispatch)",
      breaksIfDown: "queued issue dispatch stops; issues wait for the next runner",
    },
    {
      id: "pad",
      name: "Pad",
      port: 7777,
      bind: "loopback + tailnet",
      runsAs: "container pad-pad-1",
      probe: "http",
      loopbackUrl: cfg.pad.baseUrl,
      healthPath: "/",
      expectStatuses: [200],
      upWhen: "HTTP 200",
      consumers: "the user and agents — shared plans and docs",
      breaksIfDown: "shared planning docs are unreachable",
    },
    {
      id: "grist",
      name: "Grist",
      port: 8484,
      bind: "loopback + tailnet",
      runsAs: "container grist-grist-1",
      probe: "http",
      loopbackUrl: cfg.grist.baseUrl,
      healthPath: "/",
      expectStatuses: [200, 302],
      upWhen: "HTTP 200 or 302 (Grist answers 302 on /)",
      consumers: 'the aggregator\'s roadmap (its "Program roadmap" doc) and the collector',
      breaksIfDown: "the roadmap loses its live source and falls back to the feed/fixtures",
    },
    {
      id: "fleet-feed",
      name: "Fleet-feed collector",
      port: null,
      bind: "— (no listener)",
      runsAs: "laptop systemd user timer (fleet-feed-collector, ~5 min cadence; pm-stack NixOS module)",
      probe: "feed",
      consumers: "the aggregator — most live facets read the feed it writes",
      breaksIfDown: "feed-backed facets go stale, then fall back to fixtures",
    },
    {
      id: "orchestration",
      name: "Orchestration pass",
      port: null,
      bind: "— (no listener)",
      runsAs: "hourly cron (orchestration-pass)",
      probe: "section",
      consumers: "the aggregator's Today / Needs User / board in-flight lane",
      breaksIfDown: "tiers fall back to derived views; the honesty banners name it",
    },
    projectorDef(cfg),
  ];
}
