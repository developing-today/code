#!/usr/bin/env bun
// produce-charge-candidates (#1057): the Muse-side producer for the
// usage-hub charges auto-matcher. Runs on the sandbox, where the
// Plaid and Gmail connectors live, and writes the period's charge
// candidates to the state file the daily collector
// (scripts/collect-fleet-feed.ts) consumes:
//
//   charges-candidates.json  { version, generatedAt, period, candidates }
//
// Sources:
//   - Plaid `transactions-get` for the period (all linked
//     institutions). Purchase-like transactions only: pending rows
//     and the money-movement categories (Transfer, Payment, Interest,
//     Bank Fees) are excluded — card payments, balance transfers,
//     interest and fees are not charges to a merchant. Plaid is the
//     source of record; the matcher prefers it on a cross-source tie.
//   - Gmail receipt / card-alert mail for the period, via two
//     newest-first searches (receipts/invoices, then charged/alert
//     mail) merged by message id. Amount and merchant are parsed
//     conservatively from the subject; receipts whose subject
//     carries no amount get one bounded body read (max 25) before
//     being skipped. Cancellations/refunds, card-payment
//     confirmations, promotions, and GitHub notification mail are
//     excluded by construction. Unparseable mail is counted in the
//     log line, never guessed at: a card alert that names no
//     merchant is skipped (the sender is the bank, not the
//     merchant), because Plaid already covers that charge.
//
// Charge evidence only: date, merchant, amount, payment-method
// label (card brand + mask). No balances, no debt, no account
// numbers, no account-owner names ever enter the file.
//
// Usage: bun run scripts/produce-charge-candidates.ts
//   [--start YYYY-MM-DD] [--end YYYY-MM-DD] [--out <path>] [--skip-gmail]
// Defaults: start = first of the current month, end = today,
// out = CHARGE_CANDIDATES_PATH or the collector's hidden-files path.
import { execFile } from "node:child_process";
import { writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import type { ChargeCandidate } from "../src/charges-match.ts";

const run = promisify(execFile);
const PLAID_CLI = process.env.PLAID_CLI ?? "/opt/hatch/bin/plaid";
const GMAIL_CLI = process.env.GMAIL_CLI ?? "/opt/hatch/bin/hatch_gws_cli";
const MAX_GMAIL_BODY_READS = 25;

/** Plaid categories that are money movement, not merchant charges. */
const EXCLUDED_CATEGORIES = new Set(["Transfer", "Payment", "Interest", "Bank Fees"]);

function argValue(flag: string): string | undefined {
  const i = process.argv.indexOf(flag);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

function isoDay(d: Date): string {
  return d.toISOString().slice(0, 10);
}

interface PlaidAccount {
  account_id: string;
  name?: string;
  mask?: string | null;
}

interface PlaidTransaction {
  account_id: string;
  amount: number;
  date: string;
  name?: string;
  merchant_name?: string | null;
  pending?: boolean;
  category?: string[] | null;
}

/** "Costco Anywhere Visa® Card by Citi" + 9545 -> "Visa •9545". */
function methodLabel(account: PlaidAccount | undefined): string | undefined {
  if (!account?.mask) return undefined;
  const name = account.name ?? "";
  if (/visa/i.test(name)) return `Visa •${account.mask}`;
  if (/mastercard/i.test(name)) return `Mastercard •${account.mask}`;
  if (/amex|american express/i.test(name)) return `Amex •${account.mask}`;
  if (/discover/i.test(name)) return `Discover •${account.mask}`;
  return `card •${account.mask}`;
}

async function plaidCandidates(start: string, end: string): Promise<ChargeCandidate[]> {
  const { stdout } = await run(PLAID_CLI, ["transactions-get", "--start-date", start, "--end-date", end], {
    maxBuffer: 64 * 1024 * 1024,
  });
  const wrapper = JSON.parse(stdout) as {
    ok?: boolean;
    body?: { accounts?: PlaidAccount[]; transactions?: PlaidTransaction[] };
  };
  if (!wrapper.ok || !wrapper.body) throw new Error("plaid transactions-get returned no body");
  const accounts = new Map((wrapper.body.accounts ?? []).map((a) => [a.account_id, a]));
  const out: ChargeCandidate[] = [];
  for (const t of wrapper.body.transactions ?? []) {
    if (t.pending || !(t.amount > 0)) continue;
    if (EXCLUDED_CATEGORIES.has(t.category?.[0] ?? "")) continue;
    const merchant = (t.merchant_name ?? t.name ?? "").trim();
    if (!merchant || !/^\d{4}-\d{2}-\d{2}$/.test(t.date)) continue;
    const method = methodLabel(accounts.get(t.account_id));
    out.push({ source: "plaid", at: t.date, merchant, amountUsd: t.amount, ...(method ? { method } : {}) });
  }
  return out;
}

interface TriageMessage {
  id: string;
  from?: string;
  subject?: string;
  message_sent_at?: { user_local?: string };
  date?: string;
}

const AMOUNT_RE = /\$([\d,]+\.\d{2})/;

export function amountFromBody(text: string): string | undefined {
  return (
    /\b(?:amount paid|amount charged|total charged|transaction amount)[:\s]*\$([\d,]+\.\d{2})/i.exec(text)?.[1] ??
    /\btotal[:\s]*\$([\d,]+\.\d{2})/i.exec(text)?.[1] ??
    AMOUNT_RE.exec(text)?.[1]
  );
}

/** "Oct 25, 2026" / "2026-10-25" as a merchant is a parse failure, not a merchant. */
function isDateLike(s: string): boolean {
  return /^\d{4}-\d{2}-\d{2}$/.test(s) || /^[A-Z][a-z]{2,8}\s+\d{1,2},\s*\d{4}$/.test(s);
}

function cleanMerchant(raw: string | undefined): string | undefined {
  const m = raw?.trim();
  return m && !isDateLike(m) ? m : undefined;
}

function merchantFromSubject(subject: string): string | undefined {
  // "Your Google Play Order Receipt from Oct 25, 2026": the merchant
  // leads, the date trails — capture the lead before "receipt from"
  // can grab the date.
  return (
    cleanMerchant(/^your (.+?)(?:\s+order)?\s+(?:receipt|invoice)\b/i.exec(subject)?.[1]) ??
    cleanMerchant(/(?:receipt|invoice|payment) from ([^#@<>]+?)(?:\s+#|\s*[-–|]\s*|\s*$)/i.exec(subject)?.[1]) ??
    cleanMerchant(/^your (.+?)(?:\s+api)?\s+(?:receipt|invoice)\b/i.exec(subject)?.[1]) ??
    cleanMerchant(/(?:payment to|paid to)\s+([^#@<>]+?)\s*$/i.exec(subject)?.[1]) ??
    cleanMerchant(
      /\b(?:at|to)\s+([A-Z0-9][A-Za-z0-9 .&'*+-]{1,40}?)(?:\s+on\b|\s+was\b|\s+for\b|\.$|\s*$)/.exec(
        subject.replace(AMOUNT_RE, " "),
      )?.[1],
    )
  );
}

/** Subjects that are not charges: money back, money movement, marketing. */
function isNotChargeSubject(subject: string): boolean {
  return (
    /cancellation|refund/i.test(subject) ||
    /last payment|payment received|payment was received|payment has posted|thank you for your payment|thanks for your payment|payment is due|payment reminder|payment coming due|autopay payment/i.test(
      subject,
    ) ||
    /% off|coupon|save \d|free .* with .* purchase/i.test(subject)
  );
}

async function gmailCandidates(
  start: string,
  end: string,
): Promise<{ candidates: ChargeCandidate[]; skipped: number }> {
  const gmailDate = (d: string) => d.replaceAll("-", "/");
  const after = gmailDate(start);
  // before: is exclusive in Gmail search; step one day past the end.
  const endNext = isoDay(new Date(Date.parse(end) + 86_400_000));
  // Two searches, merged: receipts/invoices are the SaaS evidence;
  // charged/alert mail mirrors card charges. "alert" alone is
  // unusable as a subject term — Google Alert digests match it
  // dozens of times a day and crowd a newest-first page — and
  // GitHub notification subjects quote issue titles full of
  // receipt/invoice words, so both are excluded by construction.
  const range = `after:${after} before:${gmailDate(endNext)}`;
  const queries = [
    `${range} (subject:(receipt OR invoice)) -from:notifications@github.com`,
    `${range} (subject:("card alert" OR charged OR purchase)) -from:googlealerts-noreply@google.com -from:notifications@github.com`,
  ];
  const byId = new Map<string, TriageMessage>();
  for (const query of queries) {
    const { stdout } = await run(GMAIL_CLI, ["gmail", "+triage", "--query", query, "--max", "50", "--format", "json"], {
      maxBuffer: 16 * 1024 * 1024,
    });
    for (const m of (JSON.parse(stdout) as { messages?: TriageMessage[] }).messages ?? []) byId.set(m.id, m);
  }
  const candidates: ChargeCandidate[] = [];
  let skipped = 0;
  let bodyReads = 0;
  for (const m of byId.values()) {
    const subject = m.subject ?? "";
    if (isNotChargeSubject(subject)) {
      skipped++;
      continue;
    }
    const at = (m.message_sent_at?.user_local ?? "").slice(0, 10) || isoDay(new Date(m.date ?? ""));
    if (!/^\d{4}-\d{2}-\d{2}$/.test(at)) {
      skipped++;
      continue;
    }
    let amount = AMOUNT_RE.exec(subject)?.[1];
    let merchant = merchantFromSubject(subject);
    if ((!amount || !merchant) && bodyReads < MAX_GMAIL_BODY_READS) {
      bodyReads++;
      try {
        const { stdout: body } = await run(GMAIL_CLI, ["gmail", "+read", "--id", m.id, "--format", "json"], {
          maxBuffer: 16 * 1024 * 1024,
        });
        const text = (JSON.parse(body) as { body_text?: string }).body_text ?? "";
        amount ??= amountFromBody(text);
        merchant ??= cleanMerchant(/\bmerchant[:\s]+([A-Za-z0-9][A-Za-z0-9 .&'*+-]{1,40})/i.exec(text)?.[1]);
      } catch {
        /* unreadable body: counted as skipped below */
      }
    }
    if (!amount || !merchant) {
      skipped++;
      continue;
    }
    candidates.push({ source: "gmail", at, merchant, amountUsd: Number(amount.replaceAll(",", "")) });
  }
  return { candidates, skipped };
}

async function main(): Promise<void> {
  const now = new Date();
  const start = argValue("--start") ?? isoDay(new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 1)));
  const end = argValue("--end") ?? isoDay(now);
  const outPath =
    argValue("--out") ??
    process.env.CHARGE_CANDIDATES_PATH ??
    join(homedir(), "workspace/goals/keep-the-four-repos-healthy/hidden_files/charges-candidates.json");

  const plaid = await plaidCandidates(start, end);
  let gmail: ChargeCandidate[] = [];
  let gmailSkipped = 0;
  if (!process.argv.includes("--skip-gmail")) {
    try {
      const g = await gmailCandidates(start, end);
      gmail = g.candidates;
      gmailSkipped = g.skipped;
    } catch (err) {
      console.error(
        `producer: gmail read failed (${err instanceof Error ? err.message : "error"}); plaid candidates only`,
      );
    }
  }
  const candidates = [...plaid, ...gmail].sort(
    (a, b) => a.at.localeCompare(b.at) || a.merchant.localeCompare(b.merchant),
  );
  await writeFile(
    outPath,
    JSON.stringify({ version: 1, generatedAt: now.toISOString(), period: { start, end }, candidates }, null, 2),
  );
  console.log(
    `producer: wrote ${outPath} — ${candidates.length} candidates for ${start}..${end} (plaid ${plaid.length}, gmail ${gmail.length}, gmail skipped ${gmailSkipped})`,
  );
}

if (import.meta.main) {
  await main();
}
