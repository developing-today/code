// #3121: the Gmail body-amount fallback must not read the "total"
// inside "Subtotal" as the charged total, and an explicit paid amount
// wins over an earlier bare total.
import { describe, expect, test } from "bun:test";
import { amountFromBody } from "../scripts/produce-charge-candidates.ts";

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
