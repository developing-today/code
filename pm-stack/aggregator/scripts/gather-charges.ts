#!/usr/bin/env bun
// Charges producer (#1057): the Muse-side half of the charges
// auto-matcher. The aggregator's collector cannot reach the Plaid or
// Gmail connectors in-process, so an agent holding those connectors
// saves their raw reads to files and this script normalizes them into
// the charges export the collector ingests
// (~/atlas/charges-export/latest.json — the file-export convention,
// gather-quota.ts family 2).
//
// Inputs (both fail-soft; either may be absent):
//   PLAID_TRANSACTIONS_PATH — a Plaid transactions-get response saved
//     verbatim: { accounts?: [{account_id, name, mask}], transactions:
//     [{date, name, merchant_name?, amount, pending?}] }. Plaid's
//     amount sign: positive = money out. Pending rows are skipped.
//     Balances are never read or written — charge evidence only.
//   GMAIL_ALERTS_PATH — receipt/card alert extractions as an array of
//     {at, merchant, amountUsd, method?} (the reading agent extracts
//     these fields from the mail; this script does not parse mail).
//
// Output: { generatedAt, candidates: ChargeCandidate[] } written
// atomically to CHARGES_EXPORT_PATH (default
// ~/atlas/charges-export/latest.json). The collector's matcher
// dedupes across sources, so overlapping reads are safe; re-running
// for the same period republishes the same candidates.
//
// Usage: bun run scripts/gather-charges.ts
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import type { ChargeCandidate } from "../src/charges-match.ts";
import { parseChargeCandidatesExport } from "../src/charges-collect.ts";

const HOME = homedir();
const plaidPath = process.env.PLAID_TRANSACTIONS_PATH ?? join(HOME, "atlas/charges-export/plaid-transactions.json");
const gmailPath = process.env.GMAIL_ALERTS_PATH ?? join(HOME, "atlas/charges-export/gmail-alerts.json");
const outPath = process.env.CHARGES_EXPORT_PATH ?? join(HOME, "atlas/charges-export/latest.json");

interface PlaidAccount {
  account_id?: string;
  name?: string;
  mask?: string;
}
interface PlaidTransaction {
  date?: string;
  name?: string;
  merchant_name?: string;
  amount?: number;
  pending?: boolean;
  account_id?: string;
}

function plaidCandidates(doc: unknown): ChargeCandidate[] {
  const d = doc as { accounts?: PlaidAccount[]; transactions?: PlaidTransaction[] };
  if (!Array.isArray(d?.transactions)) return [];
  const accounts = new Map((d.accounts ?? []).map((a) => [a.account_id, a]));
  const out: ChargeCandidate[] = [];
  for (const tx of d.transactions) {
    if (tx.pending) continue;
    if (typeof tx.amount !== "number" || !(tx.amount > 0)) continue;
    if (typeof tx.date !== "string") continue;
    const merchant = (tx.merchant_name || tx.name || "").trim();
    if (!merchant) continue;
    const account = tx.account_id ? accounts.get(tx.account_id) : undefined;
    const method =
      account?.name && account?.mask
        ? account.name + " •" + account.mask
        : account?.mask
          ? "•" + account.mask
          : undefined;
    out.push({ source: "plaid", at: tx.date, merchant, amountUsd: tx.amount, ...(method ? { method } : {}) });
  }
  return out;
}

async function readJson(path: string): Promise<unknown | null> {
  try {
    return JSON.parse(await readFile(path, "utf8"));
  } catch {
    return null;
  }
}

const candidates: ChargeCandidate[] = [];
const plaidDoc = await readJson(plaidPath);
if (plaidDoc) candidates.push(...plaidCandidates(plaidDoc));
else console.error("gather-charges: no readable Plaid transactions at " + plaidPath);

const gmailDoc = await readJson(gmailPath);
if (gmailDoc) {
  const parsed = parseChargeCandidatesExport(JSON.stringify(gmailDoc));
  candidates.push(...(parsed ?? []).filter((c) => c.source === "gmail"));
} else {
  console.error("gather-charges: no readable Gmail alerts at " + gmailPath);
}

await mkdir(dirname(outPath), { recursive: true });
const tmp = outPath + ".tmp";
await writeFile(tmp, JSON.stringify({ generatedAt: new Date().toISOString(), candidates }, null, 2));
await rename(tmp, outPath);
console.log("gather-charges: wrote " + candidates.length + " candidates to " + outPath);
