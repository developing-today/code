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
    // Resets appear in the same paragraph order as the workspaces.
    const resets = [...s.matchAll(/monthly reset (\w{3}) (\d{1,2}), (\d{2}:\d{2}) UTC/gi)];
    const r = name === "Personal" ? resets[0] : resets[1];
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
  const bal = first(s, /\$(\d+(?:\.\d+)?) of \$(\d+(?:\.\d+)?) used/i);
  if (!bal) return null;
  const end = first(s, /period ends (\w{3}) (\d{1,2}), (\d{2}:\d{2}) UTC/i);
  const windows: QuotaWindow[] = [
    usdWindow(
      "monthly credits",
      Number(bal[1]),
      Number(bal[2]),
      end ? monthDayTime(end[1], end[2], end[3], now) : null,
    ),
  ];
  const short = first(s, /5h (\d+(?:\.\d+)?)%, weekly (\d+(?:\.\d+)?)%/i);
  if (short) {
    windows.push(pctWindow("5-hour", Number(short[1]), null));
    windows.push(pctWindow("weekly", Number(short[2]), null));
  }
  return {
    windows,
    note: "Registry: the Go plan does not include API access — wiring activates if the plan tier is upgraded.",
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

/** Extract every pool's freshest dated read from the quota view text. */
export function extractQuotaPools(quotaViewText: string, now = new Date()): QuotaPool[] {
  const pools: QuotaPool[] = [];
  for (const spec of POOL_SPECS) {
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
  return [...pools, ...SHELL_POOLS.map((p) => ({ ...p }))];
}
