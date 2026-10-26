#!/usr/bin/env bun
// Fleet feed collector (#890): runs on the sandbox host, where the
// quota sources of record live, and emits fleet-feed.json — the file
// the laptop aggregator renders for its quota (#851) and sandbox
// attention (#852) facets.
//
// Sources (all read-only, all fail-soft):
//   - the stewardship quota view (dated per-pool reads) — extracted by
//     src/quota-extract.ts; a pool whose read pattern stops matching
//     renders with identity + a "no current read" note.
//   - the sandbox OpenCode server API — sandbox session feed entries
//     (shared mapper, src/sources/opencode.ts) + live per-pool burn
//     attribution for the quota view.
//   - jules-review-state.json — keeper supervision entries annotate
//     matching sessions as evidence.
//   - orchestration.json — the orchestration job's latest section
//     (#899: tiers, dependencies, goal measurements), embedded as-is
//     after a structural check; absent/malformed = section omitted.
//   - the org GitHub Project "Developing Today" (#4, #910 stage 5) —
//     read via GraphQL Projects v2 into the feed's `projects` pool.
//     Token plumbing: GITHUB_PROJECT_TOKEN / GH_TOKEN from the env
//     when present; otherwise the collector resolves the Bitwarden
//     secret named `github.project-token` itself (see
//     resolveProjectToken) using ~/tools/bin/bws + the machine-account
//     access-token file under $HOME/.config/bitwarden — both already
//     present under the collector systemd unit's pinned HOME, so no
//     unit change is needed. The token value is never logged, never
//     written to the feed, and only ever sent to api.github.com.
//     Any failure (no token, bws missing, GraphQL error) omits the
//     section and logs only that it was omitted.
//
// Usage: bun run scripts/collect-fleet-feed.ts [--out <path>]
// Env overrides: QUOTA_VIEW_PATH, JULES_STATE_PATH, SANDBOX_OPENCODE_URL,
// OPENCODE_SERVER_PASSWORD_FILE, ORCHESTRATION_PATH, DNS_REGISTER_PATH,
// CLAUDE_LAUNCH_RECORDS_PATH, QUOTA_REGISTRY_PATH (the hub's PAYG/charges
// sections read the provider registry's billing incidents, fail-soft),
// USAGE_SNAPSHOT_OUTBOX_PATH (the §4 writers' outbox JSONL — stewardship
// and check-usage append records there; each collection drains it into
// the feed's snapshots field for the laptop store to ingest).
import { execFile } from "node:child_process";
import { access, readFile, rename, stat, unlink, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { dnsSectionFromRegister } from "../src/dns.ts";
import { isOrchestrationSection, type FleetFeed } from "../src/feed.ts";
import type {
  DnsSection,
  JulesSummary,
  OrchestrationSection,
  QuotaPool,
  SessionFeedEntry,
  SessionRecord,
} from "../src/model.ts";
import { extractQuotaPools } from "../src/quota-extract.ts";
import { computeChoices } from "../src/quota-guidance.ts";
import { extractCharges, extractPaygLines, extractSaasServices, extractSubscriptions } from "../src/saas-extract.ts";
import type { ChargeRow, PaygLine, SaasService, SnapshotRecord } from "../src/model.ts";
import { recordsFromDns, recordsFromPaygLines, recordsFromSaasServices } from "../src/snapshot-records.ts";
import { isSnapshotRecord, snapshotKey } from "../src/snapshots.ts";
import { fetchProjectPool } from "../src/projects.ts";
import type { ProjectPoolItem } from "../src/model.ts";
import { parseLaunchRecords, recordFromSessionEntry } from "../src/registry.ts";
import { julesSummary, mapJulesItems, type JulesReviewState } from "../src/sources/jules.ts";
import { mapServerSessions, poolBurn } from "../src/sources/opencode.ts";

const PKG = join(dirname(fileURLToPath(import.meta.url)), "..");
const HOME = homedir();
const HIDDEN = join(HOME, "workspace/goals/keep-the-four-repos-healthy/hidden_files");

const outArg = process.argv.indexOf("--out");
const outPath = outArg >= 0 ? process.argv[outArg + 1] : (process.env.FLEET_FEED_OUT ?? join(PKG, "fleet-feed.json"));
const quotaViewPath =
  process.env.QUOTA_VIEW_PATH ??
  join(
    HOME,
    "workspace/goals/stewardship-commission-recurring-review-and-dispatch/hidden_files/visibility/quota-view.md",
  );
const julesStatePath = process.env.JULES_STATE_PATH ?? join(HIDDEN, "jules-review-state.json");
const orchestrationPath = process.env.ORCHESTRATION_PATH ?? join(HIDDEN, "orchestration.json");
const dnsRegisterPath =
  process.env.DNS_REGISTER_PATH ??
  join(HOME, "workspace/goals/stand-up-lab-cluster-talm-talos-cozystack-postgres-rustfs/files/dns-domain-register.md");
const launchRecordsPath = process.env.CLAUDE_LAUNCH_RECORDS_PATH ?? join(HIDDEN, "claude-launch-records.jsonl");
const quotaRegistryPath = process.env.QUOTA_REGISTRY_PATH ?? join(HIDDEN, "provider-quota-registry.md");
const snapshotOutboxPath =
  process.env.USAGE_SNAPSHOT_OUTBOX_PATH ?? join(HIDDEN, "usage-snapshot-outbox.jsonl");
const serverUrl = process.env.SANDBOX_OPENCODE_URL ?? "http://127.0.0.1:4096";
const passwordFile =
  process.env.OPENCODE_SERVER_PASSWORD_FILE ?? join(HOME, "tools/secrets.d/opencode-server-password");

/** Provider id (OpenCode config) -> quota pool id, for burn attribution. */
function poolFor(provider: string, model: string): string | null {
  switch (provider) {
    case "meta":
      return "meta:high-usage";
    case "claude-code":
      return "anthropic:claude-pro";
    case "openai":
    case "openai-codex":
    case "chatgpt":
      return "openai:codex";
    case "cline-pass":
      return "cline-pass:cline-pass";
    case "opendesign":
      return "opendesign:go";
    case "deepseek":
      return "deepseek:api";
    case "antigravity-ls":
    case "antigravity-agy":
    case "google":
      return model.startsWith("gemini") ? "antigravity:gemini" : "antigravity:claude-gpt";
    default:
      // opencode-go is deliberately unattributed in slice 1: which
      // workspace (Personal vs Lab) a sandbox session burns is not
      // visible from the session list alone.
      return null;
  }
}

/** sessionId -> "keeper <action> <at>" from jules-review-state.json. */
async function supervisionNotes(): Promise<Map<string, string>> {
  const notes = new Map<string, string>();
  try {
    const state = JSON.parse(await readFile(julesStatePath, "utf8")) as {
      opencode_supervision?: { at?: string; action?: string; error?: string; outcome?: string }[];
    };
    for (const entry of state.opencode_supervision ?? []) {
      const text = `${entry.error ?? ""}\n${entry.outcome ?? ""}`;
      for (const m of text.matchAll(/ses_[A-Za-z0-9]+/g)) {
        notes.set(m[0], `keeper ${entry.action ?? "assessed"} ${entry.at ?? ""}`.trim());
      }
    }
  } catch {
    /* annotations are best-effort */
  }
  return notes;
}

/**
 * Drain the §4 writers' outbox: records appended by stewardship-cycle
 * (scripts/append-usage-snapshots.ts) and the keeper's check-usage.
 * Rename-then-read so a writer appending mid-collection lands in the
 * fresh file for the next run — nothing is lost, and the laptop store
 * dedupes anything forwarded twice.
 */
async function drainSnapshotOutbox(): Promise<SnapshotRecord[]> {
  const drainPath = `${snapshotOutboxPath}.drain-${process.pid}`;
  try {
    await rename(snapshotOutboxPath, drainPath);
  } catch {
    return []; // no outbox yet — the normal state until a writer runs
  }
  try {
    const text = await readFile(drainPath, "utf8");
    const out: SnapshotRecord[] = [];
    for (const line of text.split("\n")) {
      if (!line.trim()) continue;
      try {
        const parsed: unknown = JSON.parse(line);
        if (isSnapshotRecord(parsed)) out.push(parsed);
      } catch {
        // one bad line never blocks the drain
      }
    }
    return out;
  } catch (err) {
    console.error(`collector: snapshot outbox drain failed (${err}); records stay for the next run`);
    return [];
  } finally {
    await unlink(drainPath).catch(() => {});
  }
}

/**
 * Resolve the Projects-scope GitHub token without ever logging it.
 * Order: (1) GITHUB_PROJECT_TOKEN / GH_TOKEN already in the process
 * env (agent runs, manual runs, a future systemd EnvironmentFile);
 * (2) Bitwarden Secrets Manager via the bws CLI — the same access-
 * token file ~/tools/bws-env.sh sources for interactive shells.
 * The fleet-feed-collector.service pins HOME=/home/hatch and runs as
 * root, so ~/.config/bitwarden/access-token and ~/tools/bin/bws are
 * reachable under the unit as-is; no unit change is required.
 * Returns null when neither route yields a token (section omitted).
 */
async function resolveProjectToken(): Promise<string | null> {
  const fromEnv = process.env.GITHUB_PROJECT_TOKEN ?? process.env.GH_TOKEN;
  if (fromEnv && fromEnv.trim()) return fromEnv.trim();
  const bwsBin = process.env.BWS_BIN ?? join(HOME, "tools/bin/bws");
  const accessTokenFile =
    process.env.BWS_ACCESS_TOKEN_FILE ?? join(HOME, ".config/bitwarden/access-token");
  try {
    await access(bwsBin);
    const accessToken = (await readFile(accessTokenFile, "utf8")).trim();
    if (!accessToken) return null;
    const run = (args: string[]): Promise<string> =>
      new Promise((resolve, reject) => {
        execFile(
          bwsBin,
          args,
          { env: { ...process.env, BWS_ACCESS_TOKEN: accessToken }, maxBuffer: 16 * 1024 * 1024, timeout: 30_000 },
          (err, stdout) => (err ? reject(err) : resolve(stdout)),
        );
      });
    const list = JSON.parse(await run(["secret", "list"])) as { id?: string; key?: string }[];
    const entry = list.find((s) => s.key === "github.project-token");
    if (!entry?.id) return null;
    const got = JSON.parse(await run(["secret", "get", entry.id])) as { value?: string };
    return got.value && got.value.trim() ? got.value.trim() : null;
  } catch {
    return null; // bws unreachable/unreadable — fail soft, no details logged
  }
}

async function main(): Promise<void> {
  const now = new Date();
  // Identity-registry records (#906 §3), accumulated across sources below.
  const registry: SessionRecord[] = [];

  // --- quota -------------------------------------------------------------
  let pools: QuotaPool[] = [];
  let quotaViewText = "";
  try {
    quotaViewText = await readFile(quotaViewPath, "utf8");
    pools = extractQuotaPools(quotaViewText, now);
  } catch (err) {
    console.error(`collector: quota view unreadable (${err}); emitting no pools`);
  }

  // --- usage & billing hub sections (design §2.3: ride the same feed) ---
  let saas: SaasService[] | undefined;
  let payg: PaygLine[] | undefined;
  let charges: ChargeRow[] | undefined;
  let subscriptions: FleetFeed["subscriptions"];
  if (quotaViewText) {
    let registryText = "";
    try {
      registryText = await readFile(quotaRegistryPath, "utf8");
    } catch {
      console.error("collector: provider quota registry unreadable; hub PAYG/charges lose the registry reads");
    }
    saas = extractSaasServices(quotaViewText);
    payg = extractPaygLines(quotaViewText, registryText);
    charges = extractCharges(quotaViewText, registryText);
    subscriptions = extractSubscriptions(quotaViewText);
  }

  // --- sandbox attention + burn -------------------------------------------
  let attention: SessionFeedEntry[] = [];
  try {
    const password = (await readFile(passwordFile, "utf8")).trim();
    const mapped = await mapServerSessions({ baseUrl: serverUrl, password }, now.getTime(), {
      source: "opencode-sandbox",
    });
    if (mapped) {
      attention = mapped.entries;
      for (const e of mapped.entries) {
        registry.push(recordFromSessionEntry(e, mapped.tokensBySession.get(e.sessionId)));
      }
      const notes = await supervisionNotes();
      for (const e of attention) {
        const note = notes.get(e.sessionId);
        if (note) e.supervisionNote = note;
      }
      // Burn attribution: sessions the server holds active right now.
      const simple = mapped.entries.map((e) => ({
        id: e.sessionId,
        provider: e.provider,
        model: e.model,
      }));
      const burn = poolBurn(simple, mapped.tokensBySession, mapped.activeIds);
      for (const pool of pools) {
        const sessions: { session: string; model: string; tokens: number }[] = [];
        for (const [provider, list] of burn) {
          for (const s of list) {
            if (poolFor(provider, s.model) === pool.id) sessions.push(s);
          }
        }
        if (sessions.length) pool.activeSessions = sessions;
      }
    } else {
      console.error("collector: sandbox OpenCode server unreachable; attention empty");
    }
  } catch (err) {
    console.error(`collector: sandbox server read failed (${err}); attention empty`);
  }

  // --- Jules sessions + keeper totals (slice 2, #898 §5) ------------------
  let jules: JulesSummary | undefined;
  try {
    const state = JSON.parse(await readFile(julesStatePath, "utf8")) as JulesReviewState;
    const entries = mapJulesItems(state);
    if (entries.length) attention = [...attention, ...entries];
    for (const e of entries) registry.push(recordFromSessionEntry(e));
    jules = julesSummary(state);
  } catch {
    console.error("collector: jules-review state unreadable; no Jules sessions this run");
  }

  // --- Muse launch records (#906 §3) --------------------------------
  // Muse sessions have no server to scrape: they register by
  // appending one JSON line at launch (scripts/record-claude-launch.ts).
  // Absent file = no Muse sessions recorded yet — the normal state
  // until the helper is used.
  try {
    const jsonl = await readFile(launchRecordsPath, "utf8");
    registry.push(...parseLaunchRecords(jsonl));
  } catch {
    /* no launch records yet */
  }

  // --- DNS register (#902's file, parsed; #906 §1 owns the view) ---------
  let dns: DnsSection | undefined;
  try {
    const md = await readFile(dnsRegisterPath, "utf8");
    dns =
      dnsSectionFromRegister(md, now.toISOString(), "register file (#902 pipeline) via fleet feed collector") ??
      undefined;
    if (!dns) console.error("collector: DNS register parsed to zero entries; section omitted");
  } catch {
    console.error("collector: DNS register unreadable; section omitted");
  }

  // --- orchestration section (#899) --------------------------------------
  let orchestration: OrchestrationSection | undefined;
  try {
    const parsed: unknown = JSON.parse(await readFile(orchestrationPath, "utf8"));
    if (isOrchestrationSection(parsed)) {
      orchestration = parsed;
    } else {
      console.error("collector: orchestration.json failed its structural check; section omitted");
    }
  } catch {
    // No orchestration file yet (the job has not run) is the normal
    // pre-#899 state — omit the section silently.
  }

  // --- GitHub Project pool (#910 stage 5) --------------------------------
  // Read-only GraphQL read of org Project #4. Fail-soft like every
  // other section: no token / read failure = section omitted, and the
  // aggregator's items() falls back to its previous chain.
  let projects: ProjectPoolItem[] | undefined;
  try {
    const token = await resolveProjectToken();
    if (!token) {
      console.error("collector: no GitHub Project token available (env or bws); projects section omitted");
    } else {
      projects = await fetchProjectPool(token);
      if (!projects.length) {
        console.error("collector: GitHub Project read returned zero items; projects section omitted");
        projects = undefined;
      }
    }
  } catch (err) {
    console.error(`collector: GitHub Project read failed (${err instanceof Error ? err.message : "error"}); projects section omitted`);
    projects = undefined;
  }

  // --- usage snapshot records (hub §4 writers) ---------------------------
  // Writer (a): records derived from this run's own hub sections. Each
  // carries its read's own date, so an unchanged read re-derives the
  // identical record and the laptop store dedupes it — only genuinely
  // new observations grow the history. Writers (b)/(c) arrive via the
  // outbox drain and ride the same field.
  const snapshotRecords: SnapshotRecord[] = [];
  if (saas) snapshotRecords.push(...recordsFromSaasServices(saas, "fleet-feed-collector"));
  if (payg) snapshotRecords.push(...recordsFromPaygLines(payg, "fleet-feed-collector"));
  if (dns) {
    try {
      const registerStat = await stat(dnsRegisterPath);
      snapshotRecords.push(...recordsFromDns(dns, registerStat.mtime.toISOString(), "fleet-feed-collector"));
    } catch {
      console.error("collector: DNS register stat failed; dns snapshot records omitted this run");
    }
  }
  snapshotRecords.push(...(await drainSnapshotOutbox()));
  const dedupedSnapshots: SnapshotRecord[] = [];
  {
    const seen = new Set<string>();
    for (const r of snapshotRecords) {
      const k = snapshotKey(r);
      if (!seen.has(k)) {
        seen.add(k);
        dedupedSnapshots.push(r);
      }
    }
  }

  const feed: FleetFeed = {
    version: 1,
    generatedAt: now.toISOString(),
    quota: { pools, burn: [], choices: computeChoices(pools, now.getTime()) },
    attention,
    julesSummary: jules,
    ...(orchestration ? { orchestration } : {}),
    ...(dns ? { dns } : {}),
    ...(registry.length ? { registry } : {}),
    ...(saas ? { saas } : {}),
    ...(payg ? { payg } : {}),
    ...(charges ? { charges } : {}),
    ...(subscriptions ? { subscriptions } : {}),
    ...(dedupedSnapshots.length ? { snapshots: dedupedSnapshots } : {}),
    ...(projects ? { projects } : {}),
  };
  await writeFile(outPath, JSON.stringify(feed, null, 2));
  const withReads = pools.filter((p) => p.windows.length > 0).length;
  console.log(
    `collector: wrote ${outPath} — ${pools.length} pools (${withReads} with reads), ${attention.length} sessions (sandbox + jules${jules ? `, ${jules.tracked} jules tracked` : ""})` +
      (orchestration ? `, orchestration ${orchestration.runId}` : ", no orchestration section") +
      (dns ? `, dns ${dns.entries.length} domains` : ", no dns section") +
      (saas
        ? `, hub ${saas.length} services / ${payg?.length ?? 0} payg / ${charges?.length ?? 0} charges`
        : ", no hub sections") +
      (dedupedSnapshots.length ? `, snapshots ${dedupedSnapshots.length} records` : "") +
      (projects ? `, projects ${projects.length} items` : ", no projects section") +
      `, registry ${registry.length} records`,
  );
}

await main();
