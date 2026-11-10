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
// through the `codex` gatherer door (pool openai.main) and the
// alternate account (popesdrew) through the `openai.alternate` door
// (pool openai.alternate). Separate accounts, separate allowances:
// never collapse them into one pool.
import { poolState } from "./derive.ts";
import type { QuotaPool, QuotaWindow } from "./model.ts";
import { parseIsoDate } from "./validate.ts";

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
function monthDayTime(mon: string, day: string, hhmm: string, now: Date): string | null {
  const normalizedMonth = `${mon.slice(0, 1).toUpperCase()}${mon.slice(1).toLowerCase()}`;
  const month = MONTHS[normalizedMonth];
  if (month === undefined) return null;

  const [h, m] = hhmm.split(":").map(Number);
  const dayNumber = Number(day);
  const dateInYear = (year: number) => new Date(Date.UTC(year, month, dayNumber, h, m));
  const isRequestedDate = (date: Date) => date.getUTCMonth() === month && date.getUTCDate() === dayNumber;

  let d = dateInYear(now.getUTCFullYear());
  if (!isRequestedDate(d)) return null;
  if (d.getTime() < now.getTime() - 30 * 86_400_000) {
    d = dateInYear(now.getUTCFullYear() + 1);
    if (!isRequestedDate(d)) return null;
  }
  return d.toISOString();
}

const CHICAGO_WALL_FORMATTER = new Intl.DateTimeFormat("en-US", {
  timeZone: "America/Chicago",
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
});

interface WallTimeParts {
  year: number;
  month: number;
  day: number;
  hour: number;
  minute: number;
}

function wallPartsAt(instant: number): WallTimeParts {
  const parts = Object.fromEntries(
    CHICAGO_WALL_FORMATTER.formatToParts(new Date(instant)).map(({ type, value }) => [type, value]),
  );
  return {
    year: Number(parts.year),
    month: Number(parts.month),
    day: Number(parts.day),
    hour: Number(parts.hour),
    minute: Number(parts.minute),
  };
}

function wallPartsEpoch(parts: WallTimeParts): number {
  const date = new Date(0);
  date.setUTCFullYear(parts.year, parts.month - 1, parts.day);
  date.setUTCHours(parts.hour, parts.minute, 0, 0);
  return date.getTime();
}

/** Interpret a dated Central wall time using its America/Chicago offset. */
function dateTimeCentral(date: string, hhmm: string): string {
  const dateMatch = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  const timeMatch = /^(\d{2}):(\d{2})$/.exec(hhmm);
  if (!dateMatch || !timeMatch) throw new RangeError(`Invalid Central wall time: ${date} ${hhmm}`);

  const requested: WallTimeParts = {
    year: Number(dateMatch[1]),
    month: Number(dateMatch[2]),
    day: Number(dateMatch[3]),
    hour: Number(timeMatch[1]),
    minute: Number(timeMatch[2]),
  };
  const wallEpoch = wallPartsEpoch(requested);
  const normalizedInput = new Date(wallEpoch);
  if (
    requested.month < 1 ||
    requested.month > 12 ||
    requested.day < 1 ||
    requested.hour > 23 ||
    requested.minute > 59 ||
    normalizedInput.getUTCFullYear() !== requested.year ||
    normalizedInput.getUTCMonth() + 1 !== requested.month ||
    normalizedInput.getUTCDate() !== requested.day ||
    normalizedInput.getUTCHours() !== requested.hour ||
    normalizedInput.getUTCMinutes() !== requested.minute
  ) {
    throw new RangeError(`Invalid Central wall time: ${date} ${hhmm}`);
  }

  // Sampling either side of the wall date finds both offsets on transition
  // days. Round-trip candidates to reject skipped times; for a repeated time,
  // choose the earlier instant deterministically.
  const offsets = new Set(
    [-36, 0, 36].map((hours) => {
      const instant = wallEpoch + hours * 3_600_000;
      return wallPartsEpoch(wallPartsAt(instant)) - instant;
    }),
  );
  const candidates = [...offsets]
    .map((offset) => wallEpoch - offset)
    .filter((instant) => {
      const actual = wallPartsAt(instant);
      return (
        actual.year === requested.year &&
        actual.month === requested.month &&
        actual.day === requested.day &&
        actual.hour === requested.hour &&
        actual.minute === requested.minute
      );
    })
    .sort((a, b) => a - b);
  if (candidates.length === 0) throw new RangeError(`Nonexistent Central wall time: ${date} ${hhmm}`);

  const offsetMinutes = Math.round((wallEpoch - candidates[0]) / 60_000);
  const sign = offsetMinutes < 0 ? "-" : "+";
  const absoluteMinutes = Math.abs(offsetMinutes);
  const offset = `${sign}${String(Math.floor(absoluteMinutes / 60)).padStart(2, "0")}:${String(absoluteMinutes % 60).padStart(2, "0")}`;
  return `${date}T${hhmm}:00${offset}`;
}

/**
 * Reset captures (#2470): the hand-maintained quota view can carry an
 * impossible calendar date or clock time (month 13, Feb 30, day 39,
 * 99:99). The shared dateTimeCentral conversion validates Gregorian
 * dates and 24-hour times and throws on those; for a reset the right
 * outcome is resetsAt: null with the usage window preserved — never
 * the verbatim invalid string, never a discarded read, and never a
 * wallLiftsAt derived from a reset that does not exist. Valid captures
 * flow through the shared conversion unchanged.
 */
function resetDateTimeCentral(date: string, hhmm: string): string | null {
  // Gate the calendar date and clock time with the shared primitive FIRST:
  // dateTimeCentral supplies the Central-time offset conversion, but the
  // ingest contract's real-date check belongs to validate.ts (#2470).
  if (parseIsoDate(`${date}T${hhmm}`) === null) return null;
  try {
    return dateTimeCentral(date, hhmm);
  } catch {
    return null;
  }
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
    windows: [pctWindow("weekly", Number(pct[1]), reset ? resetDateTimeCentral(reset[1], reset[2]) : null)],
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
  // Dated-read fallback only (Drew, 2026-11-03 — the dashboard is a
  // straight display of the OpenCode quota read and must not interpose
  // interpretations of its own): this extractor reports the newest
  // DATED percentage/reset read in the steward's view and nothing
  // more. The former billing_not_configured branch PROMOTED a dated
  // prose mention into a fabricated walled 100% window + BLOCKED note
  // that outranked the live read for days after the account
  // recovered; a billing wall now arrives as the quota read's own
  // error state (overlayQuotaCurrent), never as promoted prose.
  const reset = first(s, /[Ww]eekly reset (?:today )?(\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}) CDT/);
  const resetIso = reset ? resetDateTimeCentral(reset[1], reset[2]) : null;
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

function goWorkspace(name: "drewrypope" | "popesdrew" | "developing-today-0"): Extractor {
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
        name === "drewrypope"
          ? "drewrypope workspace (monthly read). Renamed from Personal 2026-11-03."
          : "popesdrew workspace (monthly read). New Go account registered 2026-11-03, replacing Lab.",
    };
  };
}

/** ClinePass has 5-hour, weekly, and monthly windows; an offset beyond a
 * year is not a plausible reset for any of them (#2383). */
const MAX_CLINEPASS_RESET_OFFSET_DAYS = 366;

const extractClinePass: Extractor = (s) => {
  const measured = first(s, /Measured (\d{4}-\d{2}-\d{2}) ~(\d{2}:\d{2}) CDT/i);
  const m = first(
    s,
    /5-hour \*\*(\d+(?:\.\d+)?)%\*\* used; weekly \*\*(\d+(?:\.\d+)?)%\*\* used \(resets in (\d+)d (\d+)h\); monthly \*\*(\d+(?:\.\d+)?)%\*\* used \(resets in (\d+)d (\d+)h\)/i,
  );
  if (!measured || !m) return null;
  // The measured capture is the anchor for both relative resets (#2383).
  // The measured capture anchors reset dates, not the percentages. If it
  // is an impossible calendar/wall time, retain the matched windows and
  // mark only their resets unknown (#2383). The shared primitive owns
  // date validity; dateTimeCentral owns the Central-time offset conversion.
  let base: number | null = null;
  if (parseIsoDate(`${measured[1]}T${measured[2]}`) !== null) {
    try {
      const anchor = dateTimeCentral(measured[1], measured[2]);
      base = parseIsoDate(anchor)?.getTime() ?? null;
    } catch {
      // Nonexistent Central wall time: keep the read, with no reset anchor.
    }
  }
  const rel = (d: string, h: string): string | null => {
    const days = Number(d);
    const hours = Number(h);
    if (
      base === null ||
      !Number.isInteger(days) ||
      !Number.isInteger(hours) ||
      days < 0 ||
      hours < 0 ||
      hours > 23 ||
      days > MAX_CLINEPASS_RESET_OFFSET_DAYS
    ) {
      return null;
    }
    const resetMs = base + (days * 24 + hours) * 3_600_000;
    if (!Number.isFinite(resetMs)) return null;
    const reset = new Date(resetMs);
    if (!Number.isFinite(reset.getTime())) return null;
    const iso = reset.toISOString();
    return parseIsoDate(iso) === null ? null : iso;
  };
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

// POOL NAMING (2026-11-03, Drew): a pool's id is its OpenCode config
// provider id whenever the config defines one — meta, openai.main,
// openai.alternate, opencode-go.drewrypope, opencode-go.popesdrew,
// opencode-go.developing-today-0, cline-pass,
// opendesign, deepseek, commandcode. The colon-suffixed spellings these
// replaced (meta:high-usage, openai:codex, opencode-go:drewrypope, ...)
// survive only in the historical snapshot series and the one-shot
// backfill seed (scripts/backfill-usage-snapshots.ts), which keep the
// ids in use on their observation dates. The two Antigravity pools
// are the quota module's model-family splits of the one Antigravity
// account; they ride under the config's surviving antigravity
// provider, antigravity-direct, with the family as a dot suffix —
// antigravity-direct.gemini / antigravity-direct.claude-gpt — the
// opencode-go.<workspace> pattern (Drew, 2026-11-03: "why are you not
// including antigravity in the board?"). Ground truth for the
// RENAMED 2026-11-07 (#1823, Drew): fleet-built providers carry a
// leading underscore — the commandcode pool is now _commandcode and
// the Antigravity pools ride under _antigravity.gemini /
// _antigravity.claude-gpt. Per the rule above, each starts a NEW
// snapshot series; the pre-rename series stay in the store under
// their old ids, unrewritten.
// choice: the deployed config defines antigravity-agy,
// antigravity-direct and antigravity-ls (ls/agy are the disabled
// legacy doors; no antigravity-local provider exists), and
// antigravity-direct is the in-fork door the account's traffic was
// consolidated onto. One pool keeps a colon id because the config
// defines no provider with its name: anthropic:claude-pro (burned
// through the claude-code provider; the pool is the Claude Pro
// subscription that door spends, and the colon id keeps that
// attribution honest).
export const POOL_SPECS: PoolSpec[] = [
  {
    id: "meta",
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
    id: "openai.main",
    provider: "OpenAI",
    plan: "ChatGPT (Main — drewrypope)",
    heading: /^### ChatGPT(?! \(Alternate\))/,
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
      "No dated read. The fleet's second ChatGPT account (popesdrew), measured live by the quota gatherer through the openai.alternate credential — a separate pool from the main account (openai.main); the two are never combined.",
  },
  {
    id: "opencode-go.drewrypope",
    provider: "OpenCode Go",
    plan: "drewrypope account",
    heading: /^### OpenCode Go/,
    extract: goWorkspace("drewrypope"),
    shellNote: "No current read.",
  },
  {
    id: "opencode-go.popesdrew",
    provider: "OpenCode Go",
    plan: "popesdrew account",
    heading: /^### OpenCode Go/,
    extract: goWorkspace("popesdrew"),
    shellNote: "No current read.",
  },
  {
    id: "opencode-go.developing-today-0",
    provider: "OpenCode Go",
    plan: "developing-today-0 account",
    heading: /^### OpenCode Go/,
    extract: goWorkspace("developing-today-0"),
    shellNote: "No current read.",
  },
  {
    id: "cline-pass",
    provider: "ClinePass",
    plan: "Cline Pass (Annual)",
    heading: /^### ClinePass/,
    extract: extractClinePass,
    shellNote: "No current read (dashboard-only source).",
  },
  {
    id: "_commandcode",
    provider: "Command Code",
    plan: "GO",
    heading: /^### Command Code/,
    extract: extractCommandCode,
    shellNote: "No current read.",
  },
  {
    id: "_antigravity.gemini",
    provider: "Antigravity",
    plan: "Gemini pool",
    heading: /^### Gemini/,
    extract: extractAntigravityGemini,
    shellNote: "No current read.",
  },
  {
    id: "_antigravity.claude-gpt",
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
    id: "opendesign",
    provider: "Open Design",
    plan: "Go",
    state: "healthy",
    windows: [],
    note: "No dated read. Live windows arrive through the quota gatherer (the OpenCode opendesign credential, read by the OpenChamber quota module) and are overlaid onto this pool by overlayQuotaCurrent.",
  },
  {
    id: "deepseek",
    provider: "DeepSeek",
    plan: "API (PAYG)",
    state: "healthy",
    windows: [],
    note: "PAYG balance ¥44.63 (2026-10-25 read), parked. Not a plan pool — never a fallback for supervised sessions; plan-covered Flash routes are preferred.",
  },
  {
    id: "mistral",
    provider: "Mistral AI",
    plan: "API + Vibe (monthly credits)",
    state: "healthy",
    windows: [],
    note: "No dated read. Live windows arrive through the quota gatherer (the Mistral credential, read by the OpenChamber quota module) and are overlaid onto this pool by overlayQuotaCurrent.",
  },
];

/**
 * REMOVED 2026-11-03 (Drew: "the dashboard should not be doing any of
 * that"): the #1114 disabled-door filter family — poolProviderId,
 * poolProviderIds, poolFilteredByDisabled and the FILTER_EXEMPT_POOL_IDS
 * exemption hack that had to be bolted on when the filter ate the
 * meta pool — used to drop pools from the board whenever their
 * OpenCode config door sat in disabled_providers. That hid live,
 * measured pools whose traffic runs through model routes rather than
 * the disabled direct door: at removal time the deployed board was
 * missing anthropic:claude-pro, _commandcode, deepseek and BOTH
 * Antigravity pools even though the OpenCode quota read reported
 * them. Pool assembly no longer consults disabled_providers at all;
 * extractQuotaPools returns every pool it knows, and the serve-time
 * overlay renders every pool the read reports (see
 * overlayQuotaCurrent's unmapped-pool synthesis in quota-current.ts).
 */

/**
 * Extract every pool's freshest dated read from the quota view text.
 *
 * PASS-THROUGH (Drew, 2026-11-03): every spec pool and shell pool is
 * returned, unconditionally. The dated reads here are only the
 * FAIL-SOFT FALLBACK for pools the live OpenCode quota read cannot
 * currently measure (console-only pools, a read in error); the live
 * read overlays them at serve time (overlayQuotaCurrent) and wins
 * wherever it reports. No disabled_providers filter applies — a
 * disabled direct door says nothing about whether the pool is burned
 * through model routes or measured by OpenCode, and filtering on it
 * is what made live pools vanish from the board (#1114, reversed).
 */
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
