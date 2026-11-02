import { expect, mock, test } from "bun:test";

type WebPushArgs = [
  subscription: { endpoint: string; keys: { p256dh: string; auth: string } },
  payload: string,
  options: { TTL: number; urgency: string },
];

const lookups: string[] = [];
const forgotten: string[] = [];
const configurations: Array<{ subject: string; publicKey: string; privateKey: string }> = [];
let resolveSecret: (name: string) => Promise<string | null> = async () => null;
let sendNotification: (...args: WebPushArgs) => Promise<{ statusCode: number }> = async () => ({ statusCode: 201 });

mock.module("web-push", () => ({
  default: {
    setVapidDetails: (subject: string, publicKey: string, privateKey: string) => {
      configurations.push({ subject, publicKey, privateKey });
    },
    sendNotification: (...args: WebPushArgs) => sendNotification(...args),
  },
}));

mock.module("../src/secrets.ts", () => ({
  resolveBwsSecret: (name: string) => {
    lookups.push(name);
    return resolveSecret(name);
  },
  forgetBwsSecret: (name: string) => forgotten.push(name),
}));

const pushModule = import("../src/push.ts");

test("VAPID setup retries failures, shares in-flight work, and resets on key rotation", async () => {
  const push = await pushModule;
  const target = { endpoint: "https://push.example/sub", p256dh: "public", auth: "auth" };
  const payload = { title: "test" };
  const missing = { ok: false, gone: false, error: "VAPID private key unavailable (bws)" } as const;

  const initialSecrets: Array<string | null> = [null, null, "initial-key"];
  resolveSecret = async () => initialSecrets.shift() ?? null;

  expect(await push.sendPush(target, payload)).toEqual(missing);
  expect(await push.sendPush(target, payload)).toEqual(missing);
  expect(lookups).toHaveLength(2);
  expect(await push.ensurePushConfigured()).toBe(true);
  expect(lookups).toHaveLength(3);
  expect(configurations.map(({ privateKey }) => privateKey)).toEqual(["initial-key"]);

  expect(await push.ensurePushConfigured()).toBe(true);
  expect(lookups).toHaveLength(3);
  expect(await push.sendPush(target, payload)).toEqual({ ok: true, status: 201 });

  sendNotification = async () => {
    throw Object.assign(new Error("unauthorized"), { statusCode: 401 });
  };
  expect(await push.sendPush(target, payload)).toMatchObject({ ok: false, status: 401 });
  expect(forgotten).toEqual([push.VAPID_PRIVATE_KEY_SECRET]);

  let resolveDeferred!: (secret: string | null) => void;
  resolveSecret = async () => new Promise((resolve) => {
    resolveDeferred = resolve;
  });
  const first = push.ensurePushConfigured();
  const second = push.ensurePushConfigured();
  expect(lookups).toHaveLength(4);
  resolveDeferred("rotated-key");
  await expect(Promise.all([first, second])).resolves.toEqual([true, true]);
  expect(configurations.map(({ privateKey }) => privateKey)).toEqual(["initial-key", "rotated-key"]);

  sendNotification = async () => {
    throw Object.assign(new Error("forbidden"), { statusCode: 403 });
  };
  expect(await push.sendPush(target, payload)).toMatchObject({ ok: false, status: 403 });
  expect(forgotten).toEqual([push.VAPID_PRIVATE_KEY_SECRET, push.VAPID_PRIVATE_KEY_SECRET]);

  resolveSecret = async () => "rotated-again-key";
  expect(await push.ensurePushConfigured()).toBe(true);
  expect(configurations.map(({ privateKey }) => privateKey)).toEqual([
    "initial-key",
    "rotated-key",
    "rotated-again-key",
  ]);
});
