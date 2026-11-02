// Per-pool extractors over the stewardship quota view's dated reads
// (#851, data path per #890). The quota view (visibility/quota-view.md)
// is a human document that preserves dated observations; each extractor
// below looks for its pool's freshest dated read with a targeted
// pattern. Fail-soft by design: a pool whose pattern stops matching
// renders with its identity and a "no current read" note instead of
// vanishing or inventing numbers.
//
// THE POOL MAP (corrected 2026-10-25, provider-quota-registry.md):
// exactly ONE Meta pool — the Muse API High Usage plan, reached only
// via the `meta` provider. There is no "muse plan" pool and the
// assistant's Power subscription is never a pool, so no extractor here
// emits either.
// exactly TWO ChatGPT pools — the main account (drewrypope) reached
// through the `codex` gatherer door (pool openai:codex) and the
// alternate account (popesdrew) through the `openai.alternate` door
// (pool openai.alternate). Separate accounts, separate allowances:
// never collapse them into one pool.
import { poolState } from "./derive.ts";
import type { QuotaPool, QuotaWindow } from "./model.ts";

const MONTHS: Record<string, number> = {
  Jan: 0,
  Feb: 1,
  Mar: 2,
  Apr: 3,
  May: 4,
  Jun: 5,
  Jul: 6,
  Aug: 7,
  Sep: 8,
  Oct: 9,
  Nov: 10,
  Dec: 11,
};

/** "Oct 30" + "18:01" UTC -> ISO. Year = now's; rolls forward if past. */
function monthDayTime(mon: string, day: string, hhmm: string, now: Date): string {
  const [h, m] = hhmm.split(":").map(Number);
  let d = new Date(Date.UTC(now.getUTCFullYear(), MONTHS[mon], Number(day), h, m));
  if (d.getTime() < now.getTime() - 30 * 86_400_000) {
    d = new Date(Date.UTC(now.getUTCFullYear() + 1, MONTHS[mon], Number(day), h, m));
  }
  return d.toISOString();
}

/** "2026-10-28" + "14:00" CDT (UTC-5) -> ISO. */
function dateTimeCDT(date: string, hhmm: string): string {
  return `${date}T${hhmm}:00-05:00`;
}

function pctWindow(name: string, pct: number, resetsAt: string | null): QuotaWindow {
  const rounded = Math.round(pct * 10) / 10;
  return { name, unit: "other", limit: 100, used: rounded, resetsAt, percent: rounded };
}

function usdWindow(name: string, used: number, limit: number, resetsAt: string | null): QuotaWindow {
  return {
    name,
    unit: "usd",
    limit,
    used,
    resetsAt,
    percent: limit > 0 ? (used / limit) * 100 : undefined,
  };
}

interface Raw {
  windows: QuotaWindow[];
  note?: string;
  wallLiftsAt?: string;
}

type Extractor = (section: string, now: Date) => Raw | null;

export function sectionOf(doc: string, headingPattern: RegExp): string {
  const lines = doc.split("\n");
  let start = -1;
  for (let i = 0; i < lines.length; i++) {
    if (/^#{2,3} /.test(lines[i]) && headingPattern.test(lines[i])) {
      start = i;
      break;
    }
  }
  if (start < 0) return "";
  let end = lines.length;
  for (let i = start + 1; i < lines.length; i++) {
    if (/^#{2,3} /.test(lines[i])) {
      end = i;
      break;
    }
  }
  return lines.slice(start, end).join("\n");
}

const first = (text: string, re: RegExp): RegExpExecArray | null => re.exec(text);

// --- extractors -----------------------------------------------------------

const extractClaude: Extractor = (s, now) => {
  const pct = first(s, /weekly\s+(\d+(?:\.\d+)?)%\s+used/i);
  if (!pct) return null;
  const reset = first(s, /weekly reset \w+ (\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}) CDT/i);
  return {
    windows: [pctWindow("weekly", Number(pct[1]), reset ? dateTimeCDT(reset[1], reset[2]) : null)],
    note: "Freshest dated read in the stewardship quota view; Claude exposes no host-side meter.",
  };
};

const extractCodex: Extractor = (s, now) => {
  const pct = first(s, /7-day\s+(\d+(?:\.\d+)?)%\s+used/i);
  if (!pct) return null;
  const reset = first(s, /resets (\w{3}) (\d{1,2}), (\d{2}:\d{2}) UTC/i);
  return {
    windows: [pctWindow("7-day", Number(pct[1]), reset ? monthDayTime(reset[1], reset[2], reset[3], now) : null)],
    note: "Codex window read (the Plus/Pro chat allowances are a separate, unresolved record).",
  };
};

/**
 * The alternate ChatGPT account (popesdrew). It has no dated-read
 * section in the stewardship view yet — its live windows arrive
 * through the quota gatherer (the openai.alternate credential) and
 * are overlaid onto this pool by overlayQuotaCurrent. The extractor
 * exists so a future "### ChatGPT (Alternate)" section is picked up
 * like every other pool's dated read; until then it fail-softs to
 * the shell, like any extractor whose pattern stops matching.
 */
const extractChatGptAlternate: Extractor = (s) => {
  const fiveHour = first(s, /5-hour\s+(\d+(?:\.\d+)?)%\s+used/i);
  const weekly = first(s, /7-day\s+(\d+(?:\.\d+)?)%\s+used/i);
  if (!fiveHour && !weekly) return null;
  const windows: QuotaWindow[] = [];
  if (fiveHour) windows.push(pctWindow("5-hour", Number(fiveHour[1]), null));
  if (weekly) windows.push(pctWindow("7-day", Number(weekly[1]), null));
  return {
    windows,
    note: "Alternate ChatGPT account (popesdrew) — a separate pool from the main account; the two are never combined.",
  };
};

const extractMeta: Extractor = (s, now) => {
  // A dated billing-blocked read (402 billing_not_configured — the
  // billing-verification wall) outranks every window/reset read: the
  // pool is blocked at the account level, whatever the windows say.
  // The 100% weekly window is the fleet's blocked-status mapping (it
  // drives poolState → walled), NOT a measured consumption percentage;
  // the note says so, and no reset is invented for when it lifts.
  const blocked =
    /(\d{4}-\d{2}-\d{2})[^\n]{0,200}?billing_not_configured/i.exec(s) ??
    /billing_not_configured[^\n]{0,200}?(\d{4}-\d{2}-\d{2})/i.exec(s);
  if (blocked) {
    return {
      windows: [pctWindow("weekly", 100, null)],
      note: `BLOCKED ${blocked[1]} — API returns 402 billing_not_configured (billing-verification wall); laptop meta-provider probes all walled, stewardship measurement gate closed. The 100% weekly window is a blocked-status mapping, not measured consumption.`,
    };
  }
  const reset = first(s, /[Ww]eekly reset (?:today )?(\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}) CDT/);
  const resetIso = reset ? dateTimeCDT(reset[1], reset[2]) : null;
  const burned = /burned to empty/i.test(s);
  if (resetIso && now.getTime() >= Date.parse(resetIso)) {
    return {
      windows: [],
      note: `Weekly window reset ${resetIso}; no post-reset read yet.`,
    };
  }
  if (burned && resetIso) {
    return {
      windows: [pctWindow("weekly", 100, resetIso)],
      wallLiftsAt: resetIso,
      note: "Burned to empty 2026-10-25; charge-past-the-plan disabled on the account — the wall errors rather than bills.",
    };
  }
  const pct = first(s, /(\d+(?:\.\d+)?)% used at \d{2}:\d{2} CDT/i) ?? first(s, /(\d+(?:\.\d+)?)%\s+used/i);
  if (!pct) return null;
  return { windows: [pctWindow("weekly", Number(pct[1]), resetIso)] };
};

function goWorkspace(name: "Personal" | "Lab"): Extractor {
  return (s, now) => {
    const m = first(
      s,
      new RegExp(`\\*\\*${name} — monthly (\\d+(?:\\.\\d+)?)% used(?:, weekly (\\d+(?:\\.\\d+)?)%)?`, "i"),
    );
    if (!m) return null;
    // Paragraph anchoring (#1626): a workspace's reset is the first
    // 'monthly reset' in its own paragraph — the text from its percentage
    // match to the next blank line (or the end of the section). A paragraph
    // with no reset yields null; a reset is never borrowed from another
    // workspace's paragraph or from an earlier mention elsewhere in the
    // section (the old section-wide index pairing shifted both workspaces'
    // resets whenever one reset line was missing or an extra one appeared).
    const paraEnd = s.indexOf("\n\n", m.index);
    const paragraph = s.slice(m.index, paraEnd === -1 ? undefined : paraEnd);
    const r = first(paragraph, /monthly reset (\w{3}) (\d{1,2}), (\d{2}:\d{2}) UTC/i);
    const monthlyReset = r ? monthDayTime(r[1], r[2], r[3], now) : null;
    const windows: QuotaWindow[] = [usdWindow("monthly", (Number(m[1]) / 100) * 60, 60, monthlyReset)];
    if (m[2] !== undefined) windows.push(usdWindow("weekly", (Number(m[2]) / 100) * 30, 30, null));
    return {
      windows,
      note:
        name === "Personal"
          ? "Personal workspace (monthly read). Cancelled, paid through Nov 2 — burn before Lab per rotation order."
          : "Lab workspace (monthly read); the active workspace on the laptop.",
    };
  };
}

const extractClinePass: Extractor = (s) => {
  const measured = first(s, /Measured (\d{4}-\d{2}-\d{2}) ~(\d{2}:\d{2}) CDT/i);
  const m = first(
    s,
    /5-hour \*\*(\d+(?:\.\d+)?)%\*\* used; weekly \*\*(\d+(?:\.\d+)?)%\*\* used \(resets in (\d+)d (\d+)h\); monthly \*\*(\d+(?:\.\d+)?)%\*\* used \(resets in (\d+)d (\d+)h\)/i,
  );
  if (!measured || !m) return null;
  const base = Date.parse(dateTimeCDT(measured[1], measured[2]));
  const rel = (d: string, h: string) => new Date(base + (Number(d) * 24 + Number(h)) * 3_600_000).toISOString();
  return {
    windows: [
      pctWindow("5-hour", Number(m[1]), null),
      pctWindow("weekly", Number(m[2]), rel(m[3], m[4])),
      pctWindow("monthly", Number(m[5]), rel(m[6], m[7])),
    ],
    note: "Dashboard-only source (no usage API); resets computed from the measured read's relative offsets.",
  };
};

const extractCommandCode: Extractor = (s, now) => {
  // Drew's ruling (2026-11-01, #3410): Command Code quota reads report
  // the PLAN windows only (monthly limit, weekly, 5-hour) — never the
  // extra/top-up credit balance. The section preserves several dated
  // reads by design, so select the LATEST dated read instead of the
  // first pattern that matches anywhere: the old first-match on
  // "$X of $Y used" rendered the 2026-10-25 read as current because
  // the freshest (2026-10-27) console read is percentage-only.
  //
  // Split the section into segments at each ISO date stamp (the text
  // before the first stamp is one undated segment). A segment is a
  // parseable read when one of the recorded formats matches:
  //   - percentage-only console format: "monthly limit N% used",
  //     "weekly limit N%", "5-hour limit N%"
  //   - its compact restatement: "monthly N%, weekly N%, 5h N%"
  //   - the dollar format: "$X of $Y used" (the $ used of the $10
  //     included credits is a monthly-limit figure per the ruling)
  //     plus the "5h A%, weekly B%" short windows.
  const stamps = [...s.matchAll(/\d{4}-\d{2}-\d{2}/g)];
  const segments: { date: string; index: number; text: string }[] = [];
  if (stamps.length === 0) {
    segments.push({ date: "", index: 0, text: s });
  } else {
    if (stamps[0].index > 0) segments.push({ date: "", index: 0, text: s.slice(0, stamps[0].index) });
    for (let i = 0; i < stamps.length; i++) {
      const start = stamps[i].index;
      const end = i + 1 < stamps.length ? stamps[i + 1].index : s.length;
      segments.push({ date: stamps[i][0], index: start, text: s.slice(start, end) });
    }
  }
  const parseRead = (text: string): QuotaWindow[] | null => {
    const monthly = first(text, /monthly limit (\d+(?:\.\d+)?)% used/i);
    if (monthly) {
      const windows = [pctWindow("monthly", Number(monthly[1]), null)];
      const weekly = first(text, /weekly limit (\d+(?:\.\d+)?)%/i);
      if (weekly) windows.push(pctWindow("weekly", Number(weekly[1]), null));
      const fiveHour = first(text, /5-hour limit (\d+(?:\.\d+)?)%/i);
      if (fiveHour) windows.push(pctWindow("5-hour", Number(fiveHour[1]), null));
      return windows;
    }
    const compact = first(text, /monthly (\d+(?:\.\d+)?)%, weekly (\d+(?:\.\d+)?)%, 5h (\d+(?:\.\d+)?)%/i);
    if (compact) {
      return [
        pctWindow("monthly", Number(compact[1]), null),
        pctWindow("weekly", Number(compact[2]), null),
        pctWindow("5-hour", Number(compact[3]), null),
      ];
    }
    const bal = first(text, /\$(\d+(?:\.\d+)?) of \$(\d+(?:\.\d+)?) used/i);
    if (bal) {
      const end = first(text, /period ends (\w{3}) (\d{1,2}), (\d{2}:\d{2}) UTC/i);
      const windows: QuotaWindow[] = [
        usdWindow(
          "monthly credits",
          Number(bal[1]),
          Number(bal[2]),
          end ? monthDayTime(end[1], end[2], end[3], now) : null,
        ),
      ];
      const short = first(text, /5h (\d+(?:\.\d+)?)%, weekly (\d+(?:\.\d+)?)%/i);
      if (short) {
        windows.push(pctWindow("5-hour", Number(short[1]), null));
        windows.push(pctWindow("weekly", Number(short[2]), null));
      }
      return windows;
    }
    return null;
  };
  const parsed = segments
    .map((seg) => ({ ...seg, windows: parseRead(seg.text) }))
    .filter((seg) => seg.windows !== null);
  if (parsed.length === 0) return null;
  parsed.sort((a, b) => (a.date < b.date ? -1 : a.date > b.date ? 1 : a.index - b.index));
  const latest = parsed[parsed.length - 1];
  // A newer dated segment that carries a monthly figure none of the
  // formats could parse is a fresher read we cannot read: render no
  // windows rather than presenting the older read as current (#3410).
  const stale = segments.some(
    (seg) =>
      (seg.date > latest.date || (seg.date === latest.date && seg.index > latest.index)) &&
      parseRead(seg.text) === null &&
      /monthly/i.test(seg.text) &&
      /\d+(?:\.\d+)?\s*(%|percent)/i.test(seg.text),
  );
  if (stale) return null;
  return {
    windows: latest.windows!,
    note: "Plan windows only (Drew's 2026-11-01 ruling, #3410): freshest dated read in the quota view; the top-up credit balance is never reported as quota. Registry: the Go plan does not include API access — wiring activates if the plan tier is upgraded.",
  };
};

const extractAntigravityGemini: Extractor = (s) => {
  const m = first(s, /Gemini weekly (\d+(?:\.\d+)?)% remaining/i);
  if (!m) return null;
  return {
    windows: [pctWindow("weekly", 100 - Number(m[1]), null)],
    note: "Antigravity has TWO independent pools; this is the Gemini pool. Argon is the expensive tier — prefer Flash.",
  };
};

const extractAntigravityClaudeGpt: Extractor = (s) => {
  const m = first(s, /Claude\+GPT pools (\d+(?:\.\d+)?)% remaining/i);
  if (!m) return null;
  return {
    windows: [pctWindow("weekly", 100 - Number(m[1]), null)],
    note: "Antigravity's supported-Claude/GPT pool — independent of its Gemini pool.",
  };
};

// --- pool table -----------------------------------------------------------

interface PoolSpec {
  id: string;
  provider: string;
  plan: string;
  heading: RegExp;
  extract: Extractor;
  shellNote: string;
}

export const POOL_SPECS: PoolSpec[] = [
  {
    id: "meta:high-usage",
    provider: "Meta Model API",
    plan: "Muse API High Usage",
    heading: /^### Meta Model API/,
    extract: extractMeta,
    shellNote: "The ONE Meta pool (High Usage plan; `meta` provider is the only door). No current read.",
  },
  {
    id: "anthropic:claude-pro",
    provider: "Anthropic",
    plan: "Claude Pro",
    heading: /^### Claude$/,
    extract: extractClaude,
    shellNote: "No current read (console-only usage).",
  },
  {
    id: "openai:codex",
    provider: "OpenAI",
    plan: "ChatGPT (Codex pool)",
    heading: /^### ChatGPT/,
    extract: extractCodex,
    shellNote: "No current read. OpenAI is the fleet's final reserve pool.",
  },
  {
    id: "openai.alternate",
    provider: "OpenAI",
    plan: "ChatGPT (Alternate — popesdrew)",
    heading: /^### ChatGPT \(Alternate\)/,
    extract: extractChatGptAlternate,
    shellNote:
      "No dated read. The fleet's second ChatGPT account (popesdrew), measured live by the quota gatherer through the openai.alternate credential — a separate pool from the main account (openai:codex); the two are never combined.",
  },
  {
    id: "opencode-go:personal",
    provider: "OpenCode Go",
    plan: "Personal workspace",
    heading: /^### OpenCode Go/,
    extract: goWorkspace("Personal"),
    shellNote: "No current read.",
  },
  {
    id: "opencode-go:lab",
    provider: "OpenCode Go",
    plan: "Lab workspace",
    heading: /^### OpenCode Go/,
    extract: goWorkspace("Lab"),
    shellNote: "No current read.",
  },
  {
    id: "cline-pass:cline-pass",
    provider: "ClinePass",
    plan: "Cline Pass (Annual)",
    heading: /^### ClinePass/,
    extract: extractClinePass,
    shellNote: "No current read (dashboard-only source).",
  },
  {
    id: "commandcode:go",
    provider: "Command Code",
    plan: "GO",
    heading: /^### Command Code/,
    extract: extractCommandCode,
    shellNote: "No current read.",
  },
  {
    id: "antigravity:gemini",
    provider: "Antigravity",
    plan: "Gemini pool",
    heading: /^### Gemini/,
    extract: extractAntigravityGemini,
    shellNote: "No current read.",
  },
  {
    id: "antigravity:claude-gpt",
    provider: "Antigravity",
    plan: "Claude+GPT pool",
    heading: /^### Gemini/,
    extract: extractAntigravityClaudeGpt,
    shellNote: "No current read.",
  },
];

/** Pools that render with identity only (no dated-read extractor). */
const SHELL_POOLS: QuotaPool[] = [
  {
    id: "opendesign:go",
    provider: "Open Design",
    plan: "Go",
    state: "healthy",
    windows: [],
    note: "Usage uninstrumented — no current read. API route verified; a manual dashboard read is required for a percentage.",
  },
  {
    id: "deepseek:api",
    provider: "DeepSeek",
    plan: "API (PAYG)",
    state: "healthy",
    windows: [],
    note: "PAYG balance ¥44.63 (2026-10-25 read), parked. Not a plan pool — never a fallback for supervised sessions; plan-covered Flash routes are preferred.",
  },
];

/**
 * Pools whose consuming opencode provider ("door") is NOT the pool-id
 * prefix. Source of truth: `poolFor` in scripts/collect-fleet-feed.ts,
 * which attributes sessions to pools by door provider — the Claude Pro
 * pool is burned exclusively through the `claude-code` provider (there
 * is no `anthropic` provider in the deployed config), so disabling
 * `claude-code` must filter `anthropic:claude-pro`. Every other pool's
 * door is its prefix.
 */
const POOL_DOOR_PROVIDER_ID: Record<string, string> = {
  "anthropic:claude-pro": "claude-code",
};

/**
 * The opencode provider id whose disablement filters a pool: the pool's
 * door provider ("openai" in "openai:codex"; "claude-code" for
 * "anthropic:claude-pro").
 */
export function poolProviderId(poolId: string): string {
  return POOL_DOOR_PROVIDER_ID[poolId] ?? poolId.split(":")[0];
}

/**
 * Pools consumed through SEVERAL door providers, none of which is the
 * pool-id prefix: there is no `antigravity` provider in the deployed
 * config — the two Antigravity pools are burned through the
 * `antigravity-ls`, `antigravity-agy` and `google` providers (the same
 * three doors `poolFor` in scripts/collect-fleet-feed.ts attributes to
 * them). Such a pool is filtered only when EVERY door is disabled; one
 * usable door remaining keeps it, because sessions can still burn it.
 */
const POOL_DOOR_PROVIDER_IDS: Record<string, string[]> = {
  "antigravity:gemini": ["antigravity-ls", "antigravity-agy", "google"],
  "antigravity:claude-gpt": ["antigravity-ls", "antigravity-agy", "google"],
};

/** Every opencode provider id that can burn a pool (its doors). */
export function poolProviderIds(poolId: string): string[] {
  return POOL_DOOR_PROVIDER_IDS[poolId] ?? [poolProviderId(poolId)];
}

/**
 * True when a pool must be filtered for the given disabled set: single-
 * door pools when their door is disabled, multi-door pools only when
 * every door is disabled.
 */
export function poolFilteredByDisabled(poolId: string, disabledProviderIds: ReadonlySet<string>): boolean {
  return poolProviderIds(poolId).every((id) => disabledProviderIds.has(id));
}

/**
 * Extract every pool's freshest dated read from the quota view text.
 *
 * #1114: a pool whose provider id is in the deployed opencode config's
 * disabled_providers set is filtered out HERE, at pool assembly — never
 * at render time — so quota.json and the guidance derived from these
 * pools can never name a disabled provider. Matching is exact provider-id
 * equality against the pool's DOOR provider(s) (poolProviderIds): "openai" is
 * not "openai-codex", "opendesign" is not "open-design", "opencode-go" is
 * not "opencode" — "anthropic:claude-pro" answers to "claude-code",
 * its only door, and the antigravity pools answer to antigravity-ls /
 * antigravity-agy / google jointly (filtered only when all three are
 * disabled). A filtered pool is
 * reported through onFiltered (visibly noted, never silently dropped).
 */
export function extractQuotaPools(
  quotaViewText: string,
  now = new Date(),
  disabledProviderIds: ReadonlySet<string> = new Set(),
  onFiltered?: (poolId: string, providerId: string) => void,
): QuotaPool[] {
  const pools: QuotaPool[] = [];
  for (const spec of POOL_SPECS) {
    if (poolFilteredByDisabled(spec.id, disabledProviderIds)) {
      onFiltered?.(spec.id, poolProviderIds(spec.id).join(","));
      continue;
    }
    const section = sectionOf(quotaViewText, spec.heading);
    let raw: Raw | null = null;
    try {
      raw = section ? spec.extract(section, now) : null;
    } catch {
      raw = null;
    }
    if (!raw) {
      pools.push({
        id: spec.id,
        provider: spec.provider,
        plan: spec.plan,
        state: "healthy",
        windows: [],
        note: spec.shellNote,
      });
      continue;
    }
    const pool: QuotaPool = {
      id: spec.id,
      provider: spec.provider,
      plan: spec.plan,
      state: "healthy",
      windows: raw.windows,
      note: raw.note,
      wallLiftsAt: raw.wallLiftsAt,
    };
    pool.state = raw.wallLiftsAt ? "walled" : poolState(pool);
    if (pool.state === "walled" && !pool.wallLiftsAt) {
      const w = pool.windows.find((x) => x.resetsAt);
      if (w?.resetsAt) pool.wallLiftsAt = w.resetsAt;
    }
    pools.push(pool);
  }
  const shells = SHELL_POOLS.filter((p) => {
    if (!poolFilteredByDisabled(p.id, disabledProviderIds)) return true;
    onFiltered?.(p.id, poolProviderIds(p.id).join(","));
    return false;
  });
  return [...pools, ...shells.map((p) => ({ ...p }))];
}
