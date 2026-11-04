import { describe, expect, spyOn, test } from "bun:test";
import { parsePostAlertArgs, runPostAlert, type PostAlertDependencies } from "../scripts/post-alert.ts";

describe("post-alert argument parsing (#3723)", () => {
  test("rejects missing and flag-shaped values for every value-taking flag before side effects", async () => {
    const invalidArgs = [
      ["--source"], ["--source", "--key", "host1"],
      ["--key"], ["--key", "--severity", "info"],
      ["--severity"], ["--severity", "--title", "incident"],
      ["--title"], ["--title", "--body", "503"],
      ["--body"], ["--body", "--source", "watch"],
    ];
    let tokenCalls = 0;
    let fetchCalls = 0;
    const errors = spyOn(console, "error").mockImplementation(() => {});
    const deps: PostAlertDependencies = {
      resolveToken: async () => {
        tokenCalls++;
        return "test-token";
      },
      fetch: async () => {
        fetchCalls++;
        return new Response("unexpected", { status: 200 });
      },
    };

    try {
      for (const args of invalidArgs) {
        const result = parsePostAlertArgs(args);
        expect(result.ok).toBe(false);
        if (!result.ok) expect(result.error).toContain(args[0]);
        await expect(runPostAlert(args, deps)).resolves.toBe(2);
        expect(errors.mock.calls.at(-1)?.[0]).toContain(args[0]);
      }
    } finally {
      errors.mockRestore();
    }

    expect(tokenCalls).toBe(0);
    expect(fetchCalls).toBe(0);
  });

  test("preserves a valid control without resolving real secrets or sending a real request", async () => {
    let tokenCalls = 0;
    const requests: { url: RequestInfo | URL; init?: RequestInit }[] = [];
    const deps: PostAlertDependencies = {
      resolveToken: async () => {
        tokenCalls++;
        return "test-token";
      },
      fetch: async (url, init) => {
        requests.push({ url, init });
        return new Response("accepted", { status: 202 });
      },
    };
    const output = spyOn(console, "log").mockImplementation(() => {});
    const errors = spyOn(console, "error").mockImplementation(() => {});
    const args = [
      "--source", "globalping-watch",
      "--key", "zettel.drewrypope.com",
      "--severity", "warning",
      "--title", "host is slow",
      "--body", "probes saw elevated latency",
    ];

    try {
      expect(parsePostAlertArgs(args)).toEqual({
        ok: true,
        value: {
          source: "globalping-watch",
          key: "zettel.drewrypope.com",
          severity: "warning",
          title: "host is slow",
          body: "probes saw elevated latency",
        },
      });
      await expect(runPostAlert(args, deps)).resolves.toBe(0);
    } finally {
      output.mockRestore();
      errors.mockRestore();
    }

    expect(tokenCalls).toBe(1);
    expect(requests).toHaveLength(1);
    expect(requests[0].init?.headers).toEqual({
      Authorization: "Bearer test-token",
      "Content-Type": "application/json",
    });
    expect(JSON.parse(requests[0].init?.body as string)).toEqual({
      source: "globalping-watch",
      key: "zettel.drewrypope.com",
      severity: "warning",
      title: "host is slow",
      body: "probes saw elevated latency",
    });
  });
});
