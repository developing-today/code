// #3121: the Gmail body-amount fallback must not read the "total"
// inside "Subtotal" as the charged total, and an explicit paid amount
// wins over an earlier bare total.
import { describe, expect, test } from "bun:test";
import { amountFromBody, isNotChargeSubject } from "../scripts/produce-charge-candidates.ts";

describe("amountFromBody", () => {
  test("records the paid total, not the subtotal", () => {
    const body = "Subtotal $20.00\nTax $1.60\nTotal $21.60\nAmount paid $21.60";
    expect(amountFromBody(body)).toBe("21.60");
  });

  test("a later Amount paid beats an earlier bare Total", () => {
    const body = "Total $21.60\nSubtotal $20.00\nAmount paid $21.60";
    expect(amountFromBody(body)).toBe("21.60");
  });

  test("standalone Total is used when no paid label exists", () => {
    const body = "Subtotal $20.00\nTax $1.60\nTotal $21.60";
    expect(amountFromBody(body)).toBe("21.60");
  });

  test("total charged is an explicit paid/charged label", () => {
    expect(amountFromBody("Subtotal $20.00\nTotal charged $21.60")).toBe("21.60");
  });

  test("no amount in the body yields undefined", () => {
    expect(amountFromBody("Thank you for your order")).toBeUndefined();
  });
});

// #3120: declined/failed purchase alerts and amount-due-only invoices
// are not charges and must never become charge candidates.
describe("isNotChargeSubject (#3120)", () => {
  test("a declined purchase alert is not a charge", () => {
    expect(isNotChargeSubject("Your $20.00 purchase at OpenAI was declined")).toBe(true);
  });

  test("a failed payment alert is not a charge", () => {
    expect(isNotChargeSubject("Payment failed for your OpenAI subscription")).toBe(true);
  });

  test("an amount-due subject is not a charge", () => {
    expect(isNotChargeSubject("Amount due: $20.00 for your OpenAI invoice")).toBe(true);
  });

  test("a genuine purchase alert is still a charge", () => {
    expect(isNotChargeSubject("Your $20.00 purchase at OpenAI")).toBe(false);
  });

  test("a genuine receipt subject is still a charge", () => {
    expect(isNotChargeSubject("Your OpenAI receipt")).toBe(false);
  });
});

describe("amountFromBody (#3120)", () => {
  test("an amount-due-only invoice body yields no amount", () => {
    expect(amountFromBody("Your OpenAI invoice\nAmount due $20.00\nDue by Nov 15")).toBeUndefined();
  });

  test("a balance-due-only body yields no amount", () => {
    expect(amountFromBody("Balance due: $42.50")).toBeUndefined();
  });

  test("paid evidence still wins when due language is also present", () => {
    expect(amountFromBody("Amount paid $21.60\nAmount due $0.00")).toBe("21.60");
  });

  test("charged wording beats a trailing balance-due line (Codex P2 on PR #3193)", () => {
    expect(amountFromBody("Receipt\nYou were charged $20.00 for OpenAI\nBalance due $0.00")).toBe("20.00");
  });

  test("a genuine receipt body still yields its amount", () => {
    expect(amountFromBody("Receipt\nYou were charged $20.00 for OpenAI")).toBe("20.00");
  });
});
