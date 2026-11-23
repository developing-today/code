// The hosted capabilities document (#12583): the native
// `fleet capabilities` JSON shape (pkgs/fleet/crates/fleet/src/main.rs
// print_capabilities, registry pkgs/fleet/crates/fleet-registry) with
// the hosted door's values, so a caller can feature-detect the hosted
// door with the same code it uses on the native one.
//
// Mirrored fields keep native semantics:
//   build        — identity of THIS door: name fleet-hosted.
//   capabilities — capability ids usable at this door today.
//   compiled_capabilities — capability ids compiled into this build.
//   domains[]    — the native registry domains (name / summary /
//                  aliases / capabilities verbatim from fleet-registry),
//                  each with available + state in the native vocabulary
//                  (available | not-in-build | narrowed-by-name |
//                  disabled-by-config).
// Hosted additions (additive only):
//   domains[].hosted — the design §2 disposition for the domain.
//   domains[].filling_issue — the C issue that lands the door, when one
//                  owns it.
//   doors[]      — the component-C front doors of design §3.1 and their
//                  skeleton state.
//
// Truth, kept current per door: the route modules for the hosted
// doors are compiled in; a domain reports available only when its
// filling issue has landed the business logic. The stamping create
// path (#12585) made the hosted-transformed `gh` domain available;
// the remaining doors are still stubs, and the document says so.

import { SERVICE_NAME, type HostedConfig } from "./config.ts";

export interface HostedDoor {
  id: string;
  routes: string[];
  fillingIssue: string;
  /** "live" once the filling issue landed the door's logic. */
  state?: "live" | "stub";
}

export const HOSTED_DOORS: HostedDoor[] = [
  { id: "stamping", routes: ["POST /v1/stamps"], fillingIssue: "#12585", state: "live" },
  { id: "query", routes: ["GET /v1/query", "POST /v1/query"], fillingIssue: "#12587" },
  { id: "queue", routes: ["POST /v1/queue"], fillingIssue: "#12589" },
  { id: "ingestion", routes: ["POST /v1/ingestion/github"], fillingIssue: "#12590" },
  { id: "sync", routes: ["POST /v1/sync/project", "POST /v1/sync/queue-github"], fillingIssue: "#12591" },
];

interface NativeDomain {
  name: string;
  summary: string;
  aliases: string[];
  capabilities: string[];
  hosted: string;
  fillingIssue?: string;
  /** True once the filling issue landed this domain at the door. */
  available?: boolean;
}

// Name / summary / aliases / capabilities verbatim from
// fleet-registry's DOMAINS; `hosted` is the design §2 disposition.
const NATIVE_DOMAINS: NativeDomain[] = [
  {
    name: "queue",
    summary: "Repo work queue: enqueue, claim, and complete tasks through the queue broker.",
    aliases: ["repo-queue"],
    capabilities: ["queue"],
    hosted: "hosted-front-door",
    fillingIssue: "#12589",
  },
  {
    name: "serve",
    summary: "Run a fleet daemon role (queue-broker, secrets-broker, search, proxy) in the foreground.",
    aliases: ["queue-broker", "secrets-broker"],
    capabilities: ["queue-broker", "secrets-broker", "search-serve", "proxy-serve"],
    hosted: "transformed",
  },
  {
    name: "proxy",
    summary: "The session proxy: serve the broker socket, connect or init through it, run recovery SSH, splice raw TCP streams, or transfer files (get/put).",
    aliases: ["laptop-agent-broker"],
    capabilities: ["proxy-serve", "proxy-connect", "proxy-raw-splice"],
    hosted: "native-only",
  },
  {
    name: "dispatch",
    summary: "Thin client over pm-stack dispatch: status, launch, and enqueue verbs.",
    aliases: [],
    capabilities: ["dispatch"],
    hosted: "native-only",
  },
  {
    name: "dispatch-test",
    summary: "Explicit-dispatch CI test: scan fleet repos for free Actions capacity, submit a job, track its stored status.",
    aliases: [],
    capabilities: ["dispatch-test"],
    hosted: "native-only",
  },
  {
    name: "secrets",
    summary: "Resolve secrets by name: the laptop secrets-broker first, Bitwarden as fallback.",
    aliases: [],
    capabilities: ["secrets"],
    hosted: "native-only",
  },
  {
    name: "gh",
    summary: "The real `gh` CLI, passed through under a rotating fleet token (personal, then the developing.today App's installations); rate-limit and permission failures fail over to the next token.",
    aliases: [],
    capabilities: ["gh"],
    hosted: "hosted-transformed",
    fillingIssue: "#12585",
    available: true,
  },
  {
    name: "bridge",
    summary: "Fleet messaging through the message bridge: central sends messages or commands to an agent, a group or all; agents read their mailbox, answer and ack.",
    aliases: [],
    capabilities: ["bridge"],
    hosted: "already-hosted",
  },
  {
    name: "agent",
    summary: "The agent's front door: enrol an agent of any kind, check what this seat reaches, and take the next piece of work down the intake ladder (pushed command, queue, local shortlist, highest priority).",
    aliases: [],
    capabilities: ["agent"],
    hosted: "hosted",
  },
  {
    name: "shell",
    summary: "Run a command, or open a shell, where builds belong: the laptop, a codespace, or here.",
    aliases: [],
    capabilities: ["agent"],
    hosted: "native-only",
  },
  {
    name: "search",
    summary: "Atlas corpus search over the session history (Snowflake hybrid, via atlas-search).",
    aliases: ["atlas-search"],
    capabilities: ["search"],
    hosted: "already-hosted",
  },
  {
    name: "laptop-agent",
    summary: "Client for the laptop-agent daemon: run and manage jobs on the laptop.",
    aliases: ["laptop-agent"],
    capabilities: ["laptop-agent"],
    hosted: "native-only",
  },
];

export function buildCapabilitiesDocument(config: HostedConfig): Record<string, unknown> {
  const availableCapabilities = NATIVE_DOMAINS.filter((d) => d.available).flatMap(
    (d) => d.capabilities,
  );
  return {
    build: {
      name: SERVICE_NAME,
      version: config.version,
      invoked_as: SERVICE_NAME,
      profile: "hosted",
      edition: "hosted",
    },
    capabilities: availableCapabilities,
    compiled_capabilities: availableCapabilities,
    domains: NATIVE_DOMAINS.map((d) => ({
      name: d.name,
      summary: d.summary,
      aliases: d.aliases,
      capabilities: d.capabilities,
      available: d.available ?? false,
      state: d.available ? "available" : "not-in-build",
      hosted: d.hosted,
      ...(d.fillingIssue ? { filling_issue: d.fillingIssue } : {}),
    })),
    doors: HOSTED_DOORS.map((d) => ({
      id: d.id,
      routes: d.routes,
      state: d.state ?? "stub",
      filling_issue: d.fillingIssue,
    })),
  };
}
