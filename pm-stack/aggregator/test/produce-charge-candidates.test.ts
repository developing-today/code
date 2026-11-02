// #3121: the Gmail body-amount fallback must not read the "total"
// inside "Subtotal" as the charged total, and an explicit paid amount
// wins over an earlier bare total.
import { describe, expect, test } from "bun:test";
import { amountFromBody, gmailMessageDay, isNotChargeSubject, stagedManifestPeriod } from "../scripts/produce-charge-candidates.ts";

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

  test("future and negated verb evidence is not a payment", () => {
    for (const body of [
      "You will be charged $20.00",
      "Amount to be paid $20.00",
      "You were not charged $20.00",
      "You were never charged $20.00",
      "You weren't charged $20.00",
    ]) {
      expect(amountFromBody(body)).toBeUndefined();
    }
  });

  test("a later valid past-tense charge wins over earlier future wording", () => {
    expect(amountFromBody("You will be charged $20.00 next month. You were charged $15.00 today")).toBe("15.00");
  });

  test("an unrelated negation earlier in the sentence does not suppress a charge", () => {
    expect(amountFromBody("The refund was not processed, but you were charged $20.00")).toBe("20.00");
  });

  test("standalone total and explicit paid labels retain precedence over invalid verbs", () => {
    expect(amountFromBody("You will be charged $20.00\nTotal $21.60")).toBe("21.60");
    expect(amountFromBody("Amount paid $21.60\nYou will be charged $20.00")).toBe("21.60");
  });

  test("a genuine receipt body still yields its amount", () => {
    expect(amountFromBody("Receipt\nYou were charged $20.00 for OpenAI")).toBe("20.00");
  });
});

// #2448: a Gmail message with a missing or garbled date must be
// skippable by the caller (empty day -> format check -> skipped),
// never throw RangeError and discard the whole candidate batch.
describe("gmailMessageDay (#2448)", () => {
  test("the triage local day wins when present", () => {
    expect(gmailMessageDay({ message_sent_at: { user_local: "2026-10-25T09:30:00" }, date: "garbage" })).toBe(
      "2026-10-25",
    );
  });

  test("a parseable message date is used when no local day exists", () => {
    expect(gmailMessageDay({ date: "Sat, 25 Oct 2026 09:30:00 -0500" })).toBe("2026-10-25");
  });

  test("a missing date yields an empty day instead of throwing", () => {
    expect(gmailMessageDay({})).toBe("");
  });

  test("a garbled date yields an empty day instead of throwing", () => {
    expect(gmailMessageDay({ date: "not a date" })).toBe("");
    expect(gmailMessageDay({ date: "" })).toBe("");
  });

  test("one bad message does not stop the batch: later messages still resolve", () => {
    const days = [{ date: "garbage" }, { date: "Sat, 25 Oct 2026 09:30:00 -0500" }, {}].map(gmailMessageDay);
    expect(days).toEqual(["", "2026-10-25", ""]);
  });
});

// #4663: the staged manifest period is validated to the file's
// isRealCalendarDate standard before any candidate work; a manifest
// that does not describe a real period fails the run instead of
// writing a candidates file with period {} under a success exit.
describe("stagedManifestPeriod (#4663)", () => {
  test("a field-less manifest throws", () => {
    expect(() => stagedManifestPeriod({})).toThrow("not a real calendar window");
  });

  test("a manifest missing one field throws", () => {
    expect(() => stagedManifestPeriod({ start: "2026-11-01" })).toThrow("not a real calendar window");
    expect(() => stagedManifestPeriod({ end: "2026-11-02" })).toThrow("not a real calendar window");
  });

  test("malformed and non-calendar dates throw", () => {
    expect(() => stagedManifestPeriod({ start: "2026-13-40", end: "2026-11-02" })).toThrow(
      "not a real calendar window",
    );
    expect(() => stagedManifestPeriod({ start: "2026-11-01", end: "2026-02-30" })).toThrow(
      "not a real calendar window",
    );
    expect(() => stagedManifestPeriod({ start: "11/01/2026", end: "2026-11-02" })).toThrow(
      "not a real calendar window",
    );
  });

  test("an inverted window throws", () => {
    expect(() => stagedManifestPeriod({ start: "2026-11-02", end: "2026-11-01" })).toThrow(
      "not a real calendar window",
    );
  });

  test("a valid window passes through, including a single day", () => {
    expect(stagedManifestPeriod({ start: "2026-11-01", end: "2026-11-02" })).toEqual({
      start: "2026-11-01",
      end: "2026-11-02",
    });
    expect(stagedManifestPeriod({ start: "2026-11-01", end: "2026-11-01" })).toEqual({
      start: "2026-11-01",
      end: "2026-11-01",
    });
  });
});
