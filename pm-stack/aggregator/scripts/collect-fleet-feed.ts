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
//   - the AA cost explorer data — Supabase `ainventory` (the AA
//     workbook import: v_rankings, v_threshold_metrics,
//     benchmark_results — both the AA Intelligence Index rows and the
//     Coding Agent Index rows, kept as separate benchmark families —
//     the Test_Runs sheet, and the price event log's v_current_prices),
//     read through v_latest_workbook_batch via the Management API SQL endpoint.
//     Token plumbing mirrors the Projects read: the Bitwarden secret
//     named `supabase.access-token-muse-cli` (the sbp_ management
//     token), resolved by resolveBwsSecret and sent only to
//     api.supabase.com. Any failure embeds the section in its absent
//     form ({status:"absent", reason}) — the feed never fails over it.
//     Pool/catalog coverage is computed against the repo's own
//     .opencode/opencode.jsonc (provider catalog + model_routes
//     leaves); an unreadable config omits only the coverage.
//
// Usage: bun run scripts/collect-fleet-feed.ts [--out <path>]
// Env overrides: QUOTA_VIEW_PATH, JULES_STATE_PATH, SANDBOX_OPENCODE_URL,
// OPENCODE_SERVER_PASSWORD_FILE, ORCHESTRATION_PATH, DNS_REGISTER_PATH,
// CLAUDE_LAUNCH_RECORDS_PATH, QUOTA_REGISTRY_PATH (the hub's PAYG/charges
// sections read the provider registry's billing incidents, fail-soft),
// USAGE_SNAPSHOT_OUTBOX_PATH (the §4 writers' outbox JSONL — stewardship
// and check-usage append records there; each collection drains it into
// the feed's snapshots field for the laptop store to ingest),
// SUPABASE_PROJECT_REF (default: the muse-dev project),
// CHARGES_EXPORT_PATH (the #1057 charges producer export —
// ChargeCandidate[] JSON written by a Muse-side producer holding
// the Plaid/Gmail connectors; when present and parseable, the hub
// charges section comes from the auto-matcher in src/charges-match.ts
// instead of the prose extractor), CHARGES_SURFACED_STATE_PATH (the
// persisted keys of unmatched SaaS charges already surfaced in the
// feed, so each one surfaces exactly once),
// AA_OPENCODE_CONFIG_PATH (default: the repo's .opencode/opencode.jsonc).
import { execFile } from "node:child_process";
import { access, readFile, rename, stat, unlink, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { dnsSectionFromRegister } from "../src/dns.ts";
import { aaAbsent, aaSectionFromSupabase, type AaRawPayload } from "../src/aa.ts";
import { normalizeOrchestrationSection, orchestrationSectionProblem, type FleetFeed } from "../src/feed.ts";
import type {
  AaFeedSection,
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
import { matchCharges, type UnmatchedCharge } from "../src/charges-match.ts";
import {
  parseChargeCandidatesExport,
  parseSurfacedState,
  planUnmatchedSurfacing,
  serviceRefsFromSaas,
} from "../src/charges-collect.ts";
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
const snapshotOutboxPath = process.env.USAGE_SNAPSHOT_OUTBOX_PATH ?? join(HIDDEN, "usage-snapshot-outbox.jsonl");
const chargesExportPath = process.env.CHARGES_EXPORT_PATH ?? join(HOME, "atlas/charges-export/latest.json");
const chargesSurfacedStatePath = process.env.CHARGES_SURFACED_STATE_PATH ?? join(HIDDEN, "charges-surfaced-state.json");
const opencodeConfigPath = process.env.AA_OPENCODE_CONFIG_PATH ?? join(PKG, "..", "..", ".opencode", "opencode.jsonc");
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
 * Resolve one Bitwarden Secrets Manager secret by key, without ever
 * logging its value: the bws CLI + the machine-account access-token
 * file under $HOME/.config/bitwarden — both already present under the
 * collector systemd unit's pinned HOME. Returns null when the route
 * yields nothing (bws missing, file unreadable, key absent).
 */
async function resolveBwsSecret(key: string): Promise<string | null> {
  const bwsBin = process.env.BWS_BIN ?? join(HOME, "tools/bin/bws");
  const accessTokenFile = process.env.BWS_ACCESS_TOKEN_FILE ?? join(HOME, ".config/bitwarden/access-token");
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
    const entry = list.find((s) => s.key === key);
    if (!entry?.id) return null;
    const got = JSON.parse(await run(["secret", "get", entry.id])) as { value?: string };
    return got.value && got.value.trim() ? got.value.trim() : null;
  } catch {
    return null; // bws unreachable/unreadable — fail soft, no details logged
  }
}

/**
 * Resolve the Projects-scope GitHub token without ever logging it.
 * Order: (1) GITHUB_PROJECT_TOKEN / GH_TOKEN already in the process
 * env (agent runs, manual runs, a future systemd EnvironmentFile);
 * (2) Bitwarden Secrets Manager via resolveBwsSecret.
 * Returns null when neither route yields a token (section omitted).
 */
async function resolveProjectToken(): Promise<string | null> {
  const fromEnv = process.env.GITHUB_PROJECT_TOKEN ?? process.env.GH_TOKEN;
  if (fromEnv && fromEnv.trim()) return fromEnv.trim();
  return resolveBwsSecret("github.project-token");
}

// --- AA cost explorer (Supabase ainventory) ---------------------------------
// One SQL statement returns the whole section payload as a single
// jsonb object (keys pre-renamed to the section's camelCase). The
// Test_Runs sheet rides as raw rows; src/aa.ts maps them by the
// sheet's own header row and computes pool/catalog coverage.

const AA_SQL = `
select jsonb_build_object(
  'batch', (select import_batch::text from ainventory.v_latest_workbook_batch),
  'sources', coalesce((
    select jsonb_agg(jsonb_build_object(
      'name', name, 'kind', kind, 'importedAt', imported_at, 'driveModifiedAt', drive_modified_at
    ) order by id)
    from ainventory.sources
    where import_batch = (select import_batch from ainventory.v_latest_workbook_batch)
  ), '[]'::jsonb),
  'rankings', coalesce((
    select jsonb_agg(jsonb_build_object(
      'rowIndex', row_index, 'testRunKey', test_run_key, 'modelVariantKey', model_variant_key,
      'family', family, 'creator', creator, 'effort', effort,
      'benchmarkVersionKey', benchmark_version_key, 'rawScore', raw_score,
      'adjustedScore', adjusted_score, 'costTask', cost_task, 'timeTaskSec', time_task_sec,
      'speedTps', speed_tps, 'qualStatus', qual_status, 'availability', availability,
      'eligible', (eligible = 1), 'intelEffPerUsd', intel_eff_per_usd,
      'usdPerUsableUnit', usd_per_usable_unit, 'pointsPerDollar', points_per_dollar,
      'pointsPerSecond', points_per_second, 'balancedValue', balanced_value,
      'smartValue', smart_value, 'speedValue', speed_value, 'rankBalanced', rank_balanced,
      'rankSmart', rank_smart, 'rankSpeed', rank_speed, 'rankIntelligence', rank_intelligence,
      'rankLowCost', rank_low_cost
    ) order by rank_balanced nulls last, row_index)
    from ainventory.v_rankings
  ), '[]'::jsonb),
  'thresholds', coalesce((
    select jsonb_agg(jsonb_build_object(
      'testRunKey', test_run_key, 'target', target, 'usable', (usable = 1),
      'levelledScore', levelled_score, 'costTask', cost_task, 'timeMin', time_min,
      'usdPerUsablePoint', usd_per_usable_point
    ) order by test_run_key, target)
    from ainventory.v_threshold_metrics
  ), '[]'::jsonb),
  'configs', coalesce((
    select jsonb_agg(jsonb_build_object(
      'modelName', model_name, 'benchmark', benchmark, 'effort', effort,
      'intelligence', value_num, 'costPerTaskUsd', cost_per_task_usd, 'speedTps', speed_tps
    ) order by model_name, effort)
    from ainventory.benchmark_results br
    where br.import_batch = (select import_batch from ainventory.v_latest_workbook_batch)
      and br.benchmark like 'AA Intelligence Index%'
      and br.metric = 'intelligence_index'
  ), '[]'::jsonb),
  'testRunRows', coalesce((
    select jsonb_agg(jsonb_build_object('rowIndex', row_index, 'cells', data->'cells') order by row_index)
    from ainventory.sheet_rows sr
    where sr.import_batch = (select import_batch from ainventory.v_latest_workbook_batch)
      and sr.sheet_name = 'Test_Runs'
  ), '[]'::jsonb),
  'cai', coalesce((
    select jsonb_agg(jsonb_build_object(
      'modelName', model_name, 'effort', effort, 'version', raw->>'benchmark_version',
      'score', value_num, 'cost', cost_per_task_usd,
      'wallSec', nullif(raw->>'time_per_task_seconds', '')::numeric,
      'tokensPerTask', nullif(raw->>'token_usage_per_task', '')::numeric,
      'harness', raw->>'agent_harness',
      'creatorSlug', split_part(raw->>'observation_key', '|', 3),
      'deepSwe', nullif(raw->>'deep_swe_v1_1_pct', '')::numeric,
      'terminalBench', nullif(raw->>'terminal_bench_4_0_pct', '')::numeric,
      'sweAtlas', nullif(raw->>'swe_atlas_qna_pct', '')::numeric,
      'snapshotDate', nullif(raw->>'snapshot_date', ''), 'sourceUrl', nullif(raw->>'source_url', '')
    ) order by value_num desc nulls last, model_name)
    from ainventory.benchmark_results br
    where br.import_batch = (select import_batch from ainventory.v_latest_workbook_batch)
      and br.benchmark like 'Artificial Analysis Coding Agent Index%'
      and br.metric = 'cai_index_score'
  ), '[]'::jsonb),
  'prices', coalesce((
    select jsonb_agg(jsonb_build_object(
      'modelName', model_name, 'inputPer1M', input_usd_per_1m, 'outputPer1M', output_usd_per_1m,
      'cacheReadPer1M', cached_input_usd_per_1m, 'cacheWritePer1M', cache_write_usd_per_1m,
      'contextMinTokens', context_min_tokens, 'contextMaxTokens', context_max_tokens,
      'windowKind', window_kind
    ) order by model_name, context_min_tokens nulls first)
    from ainventory.v_current_prices
  ), '[]'::jsonb)
) as aa`;

/**
 * Build the AA feed section. Fail-soft like every other section, but
 * with the failure stated in the section itself (the absent form), so
 * the dashboard explains the gap instead of silently going stale.
 * The management token is only ever sent to api.supabase.com and is
 * never logged or written to the feed.
 */
async function collectAaSection(generatedAt: string): Promise<AaFeedSection> {
  const token = await resolveBwsSecret("supabase.access-token-muse-cli");
  if (!token) return aaAbsent("no Supabase management token available (bws)", generatedAt);
  const ref = process.env.SUPABASE_PROJECT_REF ?? "nltgxorqwxhgfzseiirt";
  let raw: AaRawPayload;
  try {
    const res = await fetch(`https://api.supabase.com/v1/projects/${ref}/database/query`, {
      method: "POST",
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      body: JSON.stringify({ query: AA_SQL }),
      signal: AbortSignal.timeout(60_000),
    });
    if (!res.ok) return aaAbsent(`Supabase query failed (HTTP ${res.status})`, generatedAt);
    const rows = (await res.json()) as { aa?: unknown }[];
    if (!Array.isArray(rows) || typeof rows[0]?.aa !== "object" || rows[0].aa === null) {
      return aaAbsent("Supabase returned no AA payload", generatedAt);
    }
    raw = rows[0].aa as AaRawPayload;
  } catch (err) {
    return aaAbsent(`Supabase unreachable (${err instanceof Error ? err.message : "error"})`, generatedAt);
  }
  let configText: string | null = null;
  try {
    configText = await readFile(opencodeConfigPath, "utf8");
  } catch {
    console.error("collector: opencode.jsonc unreadable; AA coverage omitted this run");
  }
  return (
    aaSectionFromSupabase(raw, configText, generatedAt) ??
    aaAbsent("Supabase payload failed its structural check", generatedAt)
  );
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
  let chargesUnmatched: UnmatchedCharge[] | undefined;
  let pendingSurfacedKeys: string[] | undefined;
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
    // --- charges auto-match (#1057): a producer export, when present
    // and parseable, replaces the prose evidence with matched rows ---
    try {
      const candidates = parseChargeCandidatesExport(await readFile(chargesExportPath, "utf8"));
      if (candidates && saas) {
        const matched = matchCharges(candidates, serviceRefsFromSaas(saas));
        charges = matched.rows;
        const stateText = await readFile(chargesSurfacedStatePath, "utf8").catch(() => "");
        const plan = planUnmatchedSurfacing(matched.unmatchedSaas, parseSurfacedState(stateText));
        chargesUnmatched = plan.fresh;
        pendingSurfacedKeys = plan.nextKeys;
      } else if (candidates === null) {
        console.error("collector: charges export unparseable; keeping prose-extracted charges");
      }
    } catch {
      // No export yet: prose extraction stays in charge (fail-soft).
    }
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
    // Embed the normalized form (#1110): the feed carries the section
    // in the one canonical shape the aggregator serves, whatever
    // provenance form the publisher emitted.
    const normalized = normalizeOrchestrationSection(parsed);
    if (normalized) {
      orchestration = normalized;
    } else {
      console.error(
        `collector: orchestration.json failed its structural check (${orchestrationSectionProblem(parsed)}); section omitted`,
      );
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
    console.error(
      `collector: GitHub Project read failed (${err instanceof Error ? err.message : "error"}); projects section omitted`,
    );
    projects = undefined;
  }

  // --- AA cost explorer (Supabase ainventory) ------------------------------
  // Always attempted; the section carries its own failure (absent
  // form) so the dashboard can say why instead of showing stale data.
  const aa = await collectAaSection(now.toISOString());
  if (aa.status === "absent") {
    console.error(`collector: AA section absent (${aa.reason})`);
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
    ...(chargesUnmatched?.length ? { chargesUnmatched } : {}),
    ...(subscriptions ? { subscriptions } : {}),
    ...(dedupedSnapshots.length ? { snapshots: dedupedSnapshots } : {}),
    ...(projects ? { projects } : {}),
    aa,
  };
  await writeFile(outPath, JSON.stringify(feed, null, 2));
  if (pendingSurfacedKeys) {
    await writeFile(chargesSurfacedStatePath, JSON.stringify({ surfacedKeys: pendingSurfacedKeys }, null, 2)).catch(
      (err) =>
        console.error(
          "collector: could not persist charges surfaced state (" + err + "); unmatched charges may re-surface",
        ),
    );
    console.log(
      "collector: charges auto-match - " +
        (charges?.length ?? 0) +
        " rows, " +
        (chargesUnmatched?.length ?? 0) +
        " newly surfaced unmatched SaaS",
    );
  }
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
      (aa.status === "ok"
        ? `, aa ${aa.rankings?.length ?? 0} rankings / ${aa.configs?.length ?? 0} configs / ${aa.testRuns?.length ?? 0} test runs`
        : ", aa absent") +
      `, registry ${registry.length} records`,
  );
}

await main();
