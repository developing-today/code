#!/usr/bin/env bun
// One-shot backfill for the usage snapshot store (hub design §4): seed
// history from the dated observations already recorded in
// quota-view.md and provider-quota-registry.md (2026-10-20 → today).
// Each record keeps its ORIGINAL date and source class; the prose
// stays the citation, snapshots are derived data. Fail-soft per fact:
// a pattern that no longer matches is skipped with a warning, never
// guessed. Re-running is safe — the store dedupes on (service, metric,
// ts, value).
//
// The DeepSeek balance (¥44.63) is deliberately NOT backfilled: the
// §4 unit enum (pct|usd|count|gb|tokens) has no CNY, and recording it
// as USD would corrupt the record.
//
// Usage: bun run scripts/backfill-usage-snapshots.ts [--dir <storeDir>]
// Env: PM_AGG_SNAPSHOTS_DIR, QUOTA_VIEW_PATH, QUOTA_REGISTRY_PATH.
import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { sectionOf } from "../src/quota-extract.ts";
import { SnapshotStore } from "../src/snapshots.ts";
import type { SnapshotRecord } from "../src/model.ts";

const PKG = join(dirname(fileURLToPath(import.meta.url)), "..");
const HOME = homedir();
const dirArg = process.argv.indexOf("--dir");
const storeDir = dirArg >= 0 ? process.argv[dirArg + 1] : (process.env.PM_AGG_SNAPSHOTS_DIR ?? join(PKG, "snapshots"));
const quotaViewPath =
  process.env.QUOTA_VIEW_PATH ??
  join(
    HOME,
    "workspace/goals/stewardship-commission-recurring-review-and-dispatch/hidden_files/visibility/quota-view.md",
  );
const registryPath =
  process.env.QUOTA_REGISTRY_PATH ??
  join(HOME, "workspace/goals/keep-the-four-repos-healthy/hidden_files/provider-quota-registry.md");

const BY_VIEW = "backfill:quota-view.md";
const BY_REGISTRY = "backfill:provider-quota-registry.md";
const warnings: string[] = [];
const records: SnapshotRecord[] = [];

function push(r: SnapshotRecord | null, what: string): void {
  if (r) records.push(r);
  else warnings.push(`skipped (pattern unmatched): ${what}`);
}

const pct = (
  serviceId: string,
  metricId: string,
  value: number,
  ts: string,
  source: SnapshotRecord["source"],
  observedBy: string,
  resetsAt?: string,
): SnapshotRecord => ({
  ts,
  serviceId,
  metricId,
  value,
  unit: "pct",
  limit: 100,
  ...(resetsAt ? { resetsAt } : {}),
  source,
  observedBy,
});

/** "2026-10-25" + "09:38" CDT -> ISO instant. */
const cdt = (date: string, hhmm: string): string => `${date}T${hhmm}:00-05:00`;
/** A date-only read, stamped midday UTC (≈ morning CDT) — no invented time. */
const dayTs = (date: string): string => `${date}T12:00:00Z`;

function first(text: string, re: RegExp): RegExpExecArray | null {
  return re.exec(text);
}

async function main(): Promise<void> {
  const doc = await readFile(quotaViewPath, "utf8");
  const registry = await readFile(registryPath, "utf8").catch(() => "");

  // --- Claude Pro weekly (console read, 2026-10-25 morning) ---------------
  {
    const s = sectionOf(doc, /^### Claude$/);
    const m = first(s, /weekly\s+(\d+(?:\.\d+)?)%\s+used/i);
    const reset = first(s, /weekly reset \w+ (\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}) CDT/i);
    push(
      m
        ? pct(
            "anthropic:claude-pro",
            "weekly",
            Number(m[1]),
            dayTs("2026-10-25"),
            "console",
            BY_VIEW,
            reset ? cdt(reset[1], reset[2]) : undefined,
          )
        : null,
      "claude weekly %",
    );
  }

  // --- Codex 7-day (model-efficiency plan read, 2026-10-25) ---------------
  {
    const s = sectionOf(doc, /^### ChatGPT/);
    const m = first(s, /7-day\s+(\d+(?:\.\d+)?)%\s+used/i);
    push(
      m ? pct("openai:codex", "7-day", Number(m[1]), dayTs("2026-10-25"), "console", BY_VIEW) : null,
      "codex 7-day %",
    );
  }

  // --- OpenCode Go workspaces (console reads, 2026-10-25) ------------------
  {
    const s = sectionOf(doc, /^### OpenCode Go/);
    const fresh = first(
      s,
      /Personal — monthly (\d+(?:\.\d+)?)% used, weekly (\d+(?:\.\d+)?)%, rolling (\d+(?:\.\d+)?)%/i,
    );
    push(
      fresh
        ? pct("opencode-go:personal", "monthly", Number(fresh[1]), cdt("2026-10-25", "09:17"), "console", BY_VIEW)
        : null,
      "go personal monthly (09:17 read)",
    );
    push(
      fresh
        ? pct("opencode-go:personal", "weekly", Number(fresh[2]), cdt("2026-10-25", "09:17"), "console", BY_VIEW)
        : null,
      "go personal weekly (09:17 read)",
    );
    const freshLab = first(
      s,
      /Lab — monthly (\d+(?:\.\d+)?)% used, weekly (\d+(?:\.\d+)?)%, rolling (\d+(?:\.\d+)?)%/i,
    );
    push(
      freshLab
        ? pct("opencode-go:lab", "monthly", Number(freshLab[1]), cdt("2026-10-25", "09:18"), "console", BY_VIEW)
        : null,
      "go lab monthly (09:18 read)",
    );
    push(
      freshLab
        ? pct("opencode-go:lab", "weekly", Number(freshLab[2]), cdt("2026-10-25", "09:18"), "console", BY_VIEW)
        : null,
      "go lab weekly (09:18 read)",
    );
    const morning = first(s, /\*\*Personal — monthly (\d+(?:\.\d+)?)% used\*\*/i);
    push(
      morning
        ? pct("opencode-go:personal", "monthly", Number(morning[1]), dayTs("2026-10-25"), "console", BY_VIEW)
        : null,
      "go personal monthly (morning read)",
    );
    const morningLab = first(s, /\*\*Lab — monthly (\d+(?:\.\d+)?)% used, weekly (\d+(?:\.\d+)?)%\*\*/i);
    push(
      morningLab
        ? pct("opencode-go:lab", "monthly", Number(morningLab[1]), dayTs("2026-10-25"), "console", BY_VIEW)
        : null,
      "go lab monthly (morning read)",
    );
  }

  // --- ClinePass (dashboard read, 2026-10-25 ~04:48 CDT) -------------------
  {
    const s = sectionOf(doc, /^### ClinePass/);
    const m = first(
      s,
      /5-hour \*\*(\d+(?:\.\d+)?)%\*\* used; weekly \*\*(\d+(?:\.\d+)?)%\*\* used \(resets in (\d+)d (\d+)h\); monthly \*\*(\d+(?:\.\d+)?)%\*\* used/i,
    );
    const ts = cdt("2026-10-25", "04:48");
    push(m ? pct("cline-pass:cline-pass", "5-hour", Number(m[1]), ts, "console", BY_VIEW) : null, "clinepass 5h");
    push(m ? pct("cline-pass:cline-pass", "weekly", Number(m[2]), ts, "console", BY_VIEW) : null, "clinepass weekly");
    push(m ? pct("cline-pass:cline-pass", "monthly", Number(m[5]), ts, "console", BY_VIEW) : null, "clinepass monthly");
  }

  // --- Command Code ---------------------------------------------------------
  {
    const s = sectionOf(doc, /^### Command Code/);
    const bal = first(s, /\$(\d+(?:\.\d+)?) of \$(\d+(?:\.\d+)?) used/i);
    push(
      bal
        ? {
            ts: dayTs("2026-10-25"),
            serviceId: "commandcode:go",
            metricId: "monthly-credits",
            value: Number(bal[1]),
            unit: "usd",
            limit: Number(bal[2]),
            source: "console",
            observedBy: BY_VIEW,
          }
        : null,
      "command code $ used (2026-10-25 read)",
    );
    const short = first(s, /5h (\d+(?:\.\d+)?)%, weekly (\d+(?:\.\d+)?)%/i);
    push(
      short ? pct("commandcode:go", "5-hour", Number(short[1]), dayTs("2026-10-25"), "console", BY_VIEW) : null,
      "command code 5h",
    );
    push(
      short ? pct("commandcode:go", "weekly", Number(short[2]), dayTs("2026-10-25"), "console", BY_VIEW) : null,
      "command code weekly",
    );
    push(
      /68% monthly limit used/.test(s)
        ? pct("commandcode:go", "monthly", 68, dayTs("2026-10-20"), "manual", BY_VIEW)
        : null,
      "command code 68% (2026-10-20)",
    );
    const left = first(s, /\$(\d+(?:\.\d+)?) left/i);
    push(
      left
        ? {
            ts: dayTs("2026-10-24"),
            serviceId: "commandcode:go",
            metricId: "monthly-credits",
            value: Math.round((10 - Number(left[1])) * 100) / 100,
            unit: "usd",
            limit: 10,
            source: "manual",
            observedBy: BY_VIEW,
          }
        : null,
      "command code $ left (2026-10-24)",
    );
  }

  // --- Muse consumer weekly (init-log panel captures) ----------------------
  {
    const s = sectionOf(doc, /^### Muse — consumer/);
    push(
      /36% observed 2026-10-20/.test(s)
        ? pct("muse:consumer", "weekly", 36, dayTs("2026-10-20"), "manual", BY_VIEW)
        : null,
      "muse consumer 36% (2026-10-20)",
    );
    push(
      /\*\*34% of weekly limit used/.test(s)
        ? pct("muse:consumer", "weekly", 34, cdt("2026-10-25", "07:24"), "manual", BY_VIEW)
        : null,
      "muse consumer 34% (07:24 panel)",
    );
    push(
      /37% of weekly limit \(2026-10-25 08:49 CDT/.test(doc)
        ? pct("muse:consumer", "weekly", 37, cdt("2026-10-25", "08:49"), "manual", BY_VIEW)
        : null,
      "muse consumer 37% (08:49 read)",
    );
  }

  // --- Meta High Usage + PAYG (console reads, 2026-10-25) -------------------
  {
    const s = sectionOf(doc, /^### Meta Model API/);
    push(
      /weekly quota \*\*46% at 03:20 CDT\*\*/.test(s) || /46% used at 03:20 CDT/.test(registry)
        ? pct("meta:high-usage", "weekly", 46, cdt("2026-10-25", "03:20"), "console", BY_VIEW)
        : null,
      "meta weekly 46% (03:20)",
    );
    push(
      /weekly limit \*\*85% used at 09:38 CDT\*\*/.test(s)
        ? pct("meta:high-usage", "weekly", 85, cdt("2026-10-25", "09:38"), "console", BY_VIEW)
        : null,
      "meta weekly 85% (09:38 console)",
    );
    push(
      /short window 1%/.test(registry)
        ? pct("meta:high-usage", "short-window", 1, cdt("2026-10-25", "09:38"), "console", BY_REGISTRY)
        : null,
      "meta short window 1% (registry console note)",
    );
    const accrued = first(s, /PAYG Oct 19–25: \$(\d+(?:\.\d+)?) accrued/);
    push(
      accrued
        ? {
            ts: cdt("2026-10-25", "09:38"),
            serviceId: "meta:high-usage",
            metricId: "payg-accrued",
            value: Number(accrued[1]),
            unit: "usd",
            source: "console",
            observedBy: BY_VIEW,
          }
        : null,
      "meta PAYG accrued $ (Oct 19-25)",
    );
  }

  // --- Antigravity pools (panel read, 2026-10-25 03:52) ---------------------
  {
    const s = sectionOf(doc, /^### Gemini/);
    const g = first(s, /Gemini weekly (\d+(?:\.\d+)?)% remaining/i);
    push(
      g
        ? pct(
            "antigravity:gemini",
            "weekly",
            Math.round((100 - Number(g[1])) * 10) / 10,
            cdt("2026-10-25", "03:52"),
            "console",
            BY_VIEW,
          )
        : null,
      "antigravity gemini weekly",
    );
    const cg = first(s, /Claude\+GPT pools (\d+(?:\.\d+)?)% remaining/i);
    push(
      cg
        ? pct(
            "antigravity:claude-gpt",
            "weekly",
            Math.round((100 - Number(cg[1])) * 10) / 10,
            cdt("2026-10-25", "03:52"),
            "console",
            BY_VIEW,
          )
        : null,
      "antigravity claude+gpt weekly",
    );
  }

  // --- Turso (dashboard read, 2026-10-20) ------------------------------------
  {
    const s = sectionOf(doc, /^### Turso$/m);
    push(
      /~0%, far behind: 1 DB/.test(s) || /1 DB `/.test(s)
        ? {
            ts: dayTs("2026-10-20"),
            serviceId: "turso",
            metricId: "databases",
            value: 1,
            unit: "count",
            limit: 100,
            source: "manual",
            observedBy: BY_VIEW,
          }
        : null,
      "turso databases (2026-10-20)",
    );
    push(
      /~0%/.test(sectionOf(doc, /^## Summary \(b\)/))
        ? pct("turso", "usage", 0, dayTs("2026-10-20"), "manual", BY_VIEW)
        : null,
      "turso ~0% (2026-10-20 summary)",
    );
  }

  // --- Cloudflare (Tracker footprint 2026-10-20 + Decision-004 estimates) ----
  {
    const s = sectionOf(doc, /^### Cloudflare — Workers platform/);
    const r2 = /~(\d+(?:\.\d+)?) GB/.exec(s);
    push(
      r2
        ? {
            ts: dayTs("2026-10-20"),
            serviceId: "cloudflare",
            metricId: "r2-storage",
            value: Number(r2[1]),
            unit: "gb",
            limit: 10,
            source: "manual",
            observedBy: BY_VIEW,
          }
        : null,
      "cloudflare r2 storage footprint",
    );
    const est = /low \$(\d+(?:\.\d+)?) \/ high \$(\d+(?:\.\d+)?) per month/i.exec(s);
    push(
      est
        ? {
            ts: dayTs("2026-10-25"),
            serviceId: "cloudflare",
            metricId: "platform-excess-est",
            value: { low: Number(est[1]), high: Number(est[2]) },
            unit: "usd-est",
            source: "estimate",
            observedBy: BY_VIEW,
          }
        : null,
      "cloudflare platform excess estimate",
    );
    const gw = /\$(\d+(?:\.\d+)?) per budget alert vs \$(\d+(?:\.\d+)?) per gateway cost attribution/.exec(registry);
    push(
      gw
        ? {
            ts: dayTs("2026-10-25"),
            serviceId: "cloudflare",
            metricId: "gateway-accrual-est",
            value: { low: Number(gw[1]), high: Number(gw[2]) },
            unit: "usd-est",
            source: "estimate",
            observedBy: BY_REGISTRY,
          }
        : null,
      "cloudflare gateway accrual estimate (registry)",
    );
  }

  // --- GitHub comped minutes (Memory note recorded 2026-10-25) ---------------
  {
    const s = sectionOf(doc, /^### GitHub$/m);
    const comped = /(\d[\d,]*) Linux \+ (\d+) macOS minutes/.exec(s);
    push(
      comped
        ? {
            ts: dayTs("2026-10-25"),
            serviceId: "github",
            metricId: "actions-comped-minutes",
            value: Number(comped[1].replace(/,/g, "")) + Number(comped[2]),
            unit: "count",
            source: "manual",
            observedBy: BY_VIEW,
          }
        : null,
      "github comped minutes (October)",
    );
  }

  const store = new SnapshotStore(storeDir);
  const appended = await store.append(records);
  for (const w of warnings) console.error(`backfill: ${w}`);
  console.log(
    `backfill: ${appended} record(s) appended to ${storeDir} (${records.length - appended} already present, ${warnings.length} skipped)`,
  );
}

await main();
