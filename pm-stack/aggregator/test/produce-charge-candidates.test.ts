import { describe, expect, test } from "bun:test";
import {
  extractAmountFromBody,
  isDeclinedOrFailed,
  isNotChargeSubject,
  merchantFromSubject,
  parseGmailCandidate,
  type TriageMessage,
} from "../scripts/produce-charge-candidates.ts";

describe("produce-charge-candidates: declined / failed purchase alerts (#3120)", () => {
  test("isNotChargeSubject rejects declined, failed, and unsuccessful subjects", () => {
    expect(isNotChargeSubject("Your $20.00 purchase at OpenAI was declined")).toBe(true);
    expect(isNotChargeSubject("Payment failed: your OpenAI subscription")).toBe(true);
    expect(isNotChargeSubject("Card alert: declined purchase at OpenAI")).toBe(true);
    expect(isNotChargeSubject("Your transaction was unsuccessful")).toBe(true);
    expect(isNotChargeSubject("Payment failed")).toBe(true);
  });

  test("isDeclinedOrFailed identifies declined or failed purchases in email body text", () => {
    expect(isDeclinedOrFailed("Your $20.00 purchase at OpenAI was declined.")).toBe(true);
    expect(isDeclinedOrFailed("Payment of $20.00 has failed.")).toBe(true);
    expect(isDeclinedOrFailed("Your transaction was unsuccessful.")).toBe(true);
    expect(isDeclinedOrFailed("Your card was declined for a charge of $20.00.")).toBe(true);
    expect(isDeclinedOrFailed("We were unable to process your payment.")).toBe(true);
    expect(isDeclinedOrFailed("Thank you for your payment of $20.00.")).toBe(false);
    expect(isDeclinedOrFailed("Your receipt for $20.00.")).toBe(false);
  });

  test("declined card alert subject produces no candidate (counted as skipped)", () => {
    const msg: TriageMessage = {
      id: "declined-1",
      subject: "Your $20.00 purchase at OpenAI was declined",
      date: "2026-10-31",
    };
    const res = parseGmailCandidate(msg);
    expect(res.skipped).toBe(true);
    expect(res.candidate).toBeUndefined();
    expect(res.needsBody).toBe(false);
  });

  test("declined card alert in body produces no candidate (counted as skipped)", () => {
    const msg: TriageMessage = {
      id: "declined-2",
      subject: "Card alert: purchase at OpenAI",
      date: "2026-10-31",
    };
    const initial = parseGmailCandidate(msg);
    expect(initial.needsBody).toBe(true);

    const body = "Your purchase of $20.00 at OpenAI was declined by your bank.";
    const res = parseGmailCandidate(msg, body);
    expect(res.skipped).toBe(true);
    expect(res.candidate).toBeUndefined();
  });
});

describe("produce-charge-candidates: outstanding invoices and amount-due language (#3120)", () => {
  test("isNotChargeSubject rejects amount-due and payment-due subjects", () => {
    expect(isNotChargeSubject("Payment is due: invoice #1234")).toBe(true);
    expect(isNotChargeSubject("Amount due: $20.00 for OpenAI")).toBe(true);
    expect(isNotChargeSubject("Invoice #1234 - $20.00 due")).toBe(true);
    expect(isNotChargeSubject("Payment reminder: your bill is due")).toBe(true);
  });

  test("outstanding invoice with amount due only in body produces no candidate", () => {
    const msg: TriageMessage = {
      id: "inv-due-1",
      subject: "Your OpenAI invoice",
      date: "2026-10-31",
    };
    const body = "OpenAI Invoice #1234\nAmount due $20.00\nDue Nov 1, 2026";
    const res = parseGmailCandidate(msg, body);
    expect(res.skipped).toBe(true);
    expect(res.candidate).toBeUndefined();
  });

  test("outstanding invoice with total and amount due in body produces no candidate", () => {
    const msg: TriageMessage = {
      id: "inv-due-2",
      subject: "Your OpenAI invoice",
      date: "2026-10-31",
    };
    const body = "OpenAI Invoice #1234\nTotal: $20.00\nAmount Due: $20.00\nDue Date: Nov 15, 2026";
    const res = parseGmailCandidate(msg, body);
    expect(res.skipped).toBe(true);
    expect(res.candidate).toBeUndefined();
  });

  test("invoice with no payment evidence produces no candidate", () => {
    const msg: TriageMessage = {
      id: "inv-due-3",
      subject: "Your OpenAI invoice",
      date: "2026-10-31",
    };
    const body = "OpenAI Invoice #1234\nTotal: $20.00\nDue Date: Nov 15, 2026";
    const res = parseGmailCandidate(msg, body);
    expect(res.skipped).toBe(true);
    expect(res.candidate).toBeUndefined();
  });

  test("invoice whose subject has amount but body has no payment evidence produces no candidate", () => {
    const msg: TriageMessage = {
      id: "inv-due-4",
      subject: "Your $20.00 invoice from OpenAI",
      date: "2026-10-31",
    };
    const initial = parseGmailCandidate(msg);
    // Even though amount is in subject, invoice requires payment evidence
    expect(initial.needsBody).toBe(true);

    const body = "OpenAI Invoice #1234\nAmount due: $20.00\nDue Nov 1, 2026";
    const res = parseGmailCandidate(msg, body);
    expect(res.skipped).toBe(true);
    expect(res.candidate).toBeUndefined();
  });
});

describe("produce-charge-candidates: genuine receipts and alerts produce candidates (#3120)", () => {
  test("genuine purchase alert from subject produces candidate", () => {
    const msg: TriageMessage = {
      id: "genuine-1",
      subject: "Your $20.00 purchase at OpenAI on Oct 25",
      date: "2026-10-25",
    };
    const res = parseGmailCandidate(msg);
    expect(res.skipped).toBe(false);
    expect(res.candidate).toEqual({
      source: "gmail",
      at: "2026-10-25",
      merchant: "OpenAI",
      amountUsd: 20,
    });
  });

  test("paid invoice with explicit amount paid produces candidate", () => {
    const msg: TriageMessage = {
      id: "genuine-2",
      subject: "Your OpenAI invoice",
      date: "2026-10-25",
    };
    const body = "OpenAI Invoice #1234\nTotal: $20.00\nAmount paid: $20.00\nPayment method: Visa •1234";
    const res = parseGmailCandidate(msg, body);
    expect(res.skipped).toBe(false);
    expect(res.candidate).toEqual({
      source: "gmail",
      at: "2026-10-25",
      merchant: "OpenAI",
      amountUsd: 20,
    });
  });

  test("paid invoice with status: paid produces candidate", () => {
    const msg: TriageMessage = {
      id: "genuine-3",
      subject: "Invoice from Anthropic",
      date: "2026-10-25",
    };
    const body = "Anthropic Invoice\nStatus: Paid\nTotal: $20.00";
    const res = parseGmailCandidate(msg, body);
    expect(res.skipped).toBe(false);
    expect(res.candidate).toEqual({
      source: "gmail",
      at: "2026-10-25",
      merchant: "Anthropic",
      amountUsd: 20,
    });
  });

  test("receipt with subtotal and total matches total, not subtotal", () => {
    const msg: TriageMessage = {
      id: "genuine-4",
      subject: "Receipt from Acme Corp",
      date: "2026-10-25",
    };
    const body = "Acme Corp Receipt\nSubtotal: $20.00\nTax: $1.60\nTotal: $21.60\nPaid with Visa •9999";
    const res = parseGmailCandidate(msg, body);
    expect(res.skipped).toBe(false);
    expect(res.candidate).toEqual({
      source: "gmail",
      at: "2026-10-25",
      merchant: "Acme Corp",
      amountUsd: 21.60,
    });
  });

  test("receipt with subtotal, tax, and explicit amount charged matches amount charged", () => {
    const msg: TriageMessage = {
      id: "genuine-5",
      subject: "Your OpenAI receipt",
      date: "2026-10-25",
    };
    const body = "Receipt #8888\nSubtotal: $20.00\nTax: $1.60\nTotal: $21.60\nAmount charged: $21.60";
    const res = parseGmailCandidate(msg, body);
    expect(res.skipped).toBe(false);
    expect(res.candidate).toEqual({
      source: "gmail",
      at: "2026-10-25",
      merchant: "OpenAI",
      amountUsd: 21.60,
    });
  });

  test("order receipt with lead merchant parses correctly", () => {
    const msg: TriageMessage = {
      id: "genuine-6",
      subject: "Your Google Play Order Receipt from Oct 25, 2026",
      date: "2026-10-25",
    };
    const body = "Order receipt: Total: $9.99\nPaid with Visa";
    const res = parseGmailCandidate(msg, body);
    expect(res.skipped).toBe(false);
    expect(res.candidate).toEqual({
      source: "gmail",
      at: "2026-10-25",
      merchant: "Google Play",
      amountUsd: 9.99,
    });
  });
});
