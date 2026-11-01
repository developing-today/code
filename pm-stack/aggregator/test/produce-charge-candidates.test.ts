import { describe, expect, test } from "bun:test";
import { amountFromBody } from "../scripts/produce-charge-candidates.ts";

describe("produce-charge-candidates amountFromBody", () => {
  test("requires a standalone total label so Subtotal does not match as total", () => {
    const text = `
Order Details:
Subtotal: $20.00
Tax: $1.60
Total: $21.60
`;
    expect(amountFromBody(text)).toBe("21.60");
  });

  test("test with Subtotal/Tax/Total (and a later Amount paid) records the paid total", () => {
    const text = `
Receipt summary:
Subtotal $20.00
Tax $1.60
Total $21.60
Amount paid $21.60
`;
    expect(amountFromBody(text)).toBe("21.60");
  });

  test("prefers explicit paid/charged amounts over bare total when amounts differ", () => {
    const text = `
Invoice #1234
Total: $100.00
Amount charged: $25.00
`;
    expect(amountFromBody(text)).toBe("25.00");
  });

  test("falls back to bare total if no explicit paid/charged label is present", () => {
    const text = `
Receipt
Item A $10.00
Subtotal $10.00
Total $10.00
`;
    expect(amountFromBody(text)).toBe("10.00");
  });

  test("falls back to generic dollar amount if neither explicit label nor total label is present", () => {
    const text = `
Payment confirmation
Here is your payment of $42.50.
`;
    expect(amountFromBody(text)).toBe("42.50");
  });
});
