// The page's own small functions, run as the browser runs them.
//
// Oracles: the elapsed-time phrases and the device labels are written
// out here by hand for real inputs (two real browser names).
import { expect, test } from "bun:test";
import { ago, band, deviceLabel, endpointMark, subscriptionState } from "../public/app.js";
import { contact, endpointMark as serverMark } from "../src/push.ts";

test("elapsed time reads as a phrase on its own, never 'just now ago'", () => {
  const now = Date.parse("2026-11-22T12:00:00Z");
  const at = (seconds: number) => new Date(now - seconds * 1000).toISOString();
  expect(ago(at(0), now)).toBe("just now");
  expect(ago(at(89), now)).toBe("just now");
  expect(ago(at(90), now)).toBe("2 min ago");
  expect(ago(at(45 * 60), now)).toBe("45 min ago");
  expect(ago(at(5400), now)).toBe("2 h ago");
  expect(ago(at(26 * 3600), now)).toBe("26 h ago");
  expect(ago(at(3 * 86400), now)).toBe("3 d ago");
  // The queue store writes times without a zone: they are UTC.
  expect(ago("2026-11-22T11:00:00", now)).toBe("60 min ago");
  // A time in the future (clock skew) is now, and no time is nothing.
  expect(ago(at(-500), now)).toBe("just now");
  expect([ago("", now), ago(null, now), ago("not a time", now)]).toEqual(["", "", ""]);
});

test("a device is labelled by what it runs on, not by the engine's name", () => {
  expect(
    deviceLabel(
      "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Mobile Safari/537.36",
    ),
  ).toBe("Linux; Android 10; K");
  expect(deviceLabel("Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0")).toBe(
    "X11; Linux x86_64; rv:140.0",
  );
  expect(deviceLabel("curl/8.5.0")).toBe("curl/8.5.0");
  expect(deviceLabel(`(${"x".repeat(100)})`)).toHaveLength(60);
});

test("priority bands in words", () => {
  expect([10, 20, 30, 40, 50, 70, 90].map(band)).toEqual(["Top", "High", "Normal", "Low", "Low", "Later", "Later"]);
  expect([band(null), band(undefined), band("")]).toEqual(["Normal", "Normal", "Normal"]);
});

test("a device with notifications on tells whether the sender still has it, and for which key", async () => {
  // The app and the sender must compute the same mark, or every device
  // would look lost on every load.
  const endpoint = "https://fcm.googleapis.com/fcm/send/abc:DEF-123";
  expect(await endpointMark(endpoint)).toBe(await serverMark(endpoint));
  // Worked by hand: SHA-256 of the text, base64url, first 16 characters.
  expect(await endpointMark("a")).toBe("ypeBEsobvcr6wjGz");
  const status = { key: "KEY-NOW", known: ["mine"] };
  expect(subscriptionState("KEY-NOW", "mine", status)).toBe("keep");
  // The sender has lost it (a new store, a restore from an old copy).
  expect(subscriptionState("KEY-NOW", "other", status)).toBe("again");
  expect(subscriptionState("KEY-NOW", "mine", { key: "KEY-NOW" })).toBe("again");
  // It was made for another sender's key: the app moved from the laptop to the edge.
  expect(subscriptionState("KEY-OLD", "mine", status)).toBe("renew");
  // A browser that does not say which key it was made for is judged by the mark alone.
  expect(subscriptionState("", "mine", status)).toBe("keep");
  expect(subscriptionState("", "other", status)).toBe("again");
  // No answer worth acting on: leave it.
  expect(subscriptionState("KEY-OLD", "mine", { key: "" })).toBe("keep");
  expect(subscriptionState("KEY-OLD", "mine", null)).toBe("keep");
});

test("the contact given to push services is the project's page, never a person's address", () => {
  expect(contact("developing-today/code")).toBe("https://github.com/developing-today/code");
});
