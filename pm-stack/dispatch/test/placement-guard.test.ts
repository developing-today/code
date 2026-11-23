import { describe, expect, test } from "bun:test";
import { enforcePlacementGuard, type PlacementCommand } from "../src/placement-guard.ts";

describe("pm-dispatch placement guard (issue #2732; oracle = placement-record argv and JSON contract)", () => {
  test("OpenCode Go runs placement-record before seating", () => {
    const calls: PlacementCommand[] = [];
    const result = enforcePlacementGuard(
      {
        enabled: true,
        executable: "placement-record",
        policyFile: "/policy/placement-policy.yaml",
        quotaCurrentFile: "/state/quota-current.json",
      },
      { taskId: 14492, repo: "developing-today/code", model: { providerID: "opencode-go", id: "gpt-5.6-luna" } },
      (command) => {
        calls.push(command);
        return { status: 0, stdout: "{\"option\":\"opencode-session\",\"rule\":\"R4\"}\n", stderr: "" };
      },
    );

    expect(result).toBe("opencode-go");
    expect(calls).toHaveLength(1);
    expect(calls[0]).toEqual({
      args: [
        "--policy",
        "/policy/placement-policy.yaml",
        "--answers",
        "/dev/stdin",
        "--quota-current",
        "/state/quota-current.json",
        "--pool",
        "opencode-go",
      ],
      input: JSON.stringify({
        task: "queue task 14492 / developing-today/code",
        q2_needs_local_or_secrets: true,
      }),
    });
  });

  test("monthly guard refusal is logged verbatim and prevents seating (oracle = placement-record stderr)", () => {
    const refusal =
      'placement-record: monthly-guard refusal: option "opencode-session" on pool "opencode-go" refused (monthly 98.4% at/above 97% threshold [source: monthly-guard])';
    const errors: string[] = [];
    expect(() =>
      enforcePlacementGuard(
        {
          enabled: true,
          executable: "placement-record",
          policyFile: "/policy/placement-policy.yaml",
          quotaCurrentFile: "/state/quota-current.json",
          logError: (line) => errors.push(line),
        },
        { taskId: 8, repo: "developing-today/code", model: { providerID: "opencode-go", id: "gpt-5.6-luna" } },
        () => ({ status: 1, stdout: "", stderr: `${refusal}\n` }),
      ),
    ).toThrow(refusal);
    expect(errors).toEqual([refusal]);
  });

  test("ClinePass uses its quota-current service id (oracle = serviceId in quota-current.json)", () => {
    const calls: PlacementCommand[] = [];
    const result = enforcePlacementGuard(
      {
        enabled: true,
        executable: "placement-record",
        policyFile: "/policy/placement-policy.yaml",
        quotaCurrentFile: "/state/quota-current.json",
      },
      { taskId: 2, repo: "developing-today/code", model: { providerID: "cline-pass", id: "claude-sonnet" } },
      (command) => {
        calls.push(command);
        return { status: 0, stdout: "{\"option\":\"opencode-session\",\"rule\":\"R4\"}", stderr: "" };
      },
    );
    expect(result).toBe("cline-pass");
    expect(calls[0]?.args.at(-1)).toBe("cline-pass");
  });

  test("disabled guard and non-covered providers do not invoke the helper", () => {
    let calls = 0;
    const command = () => {
      calls += 1;
      return { status: 0, stdout: "", stderr: "" };
    };
    const config = {
      enabled: true,
      executable: "placement-record",
      policyFile: "/policy/placement-policy.yaml",
      quotaCurrentFile: "/state/quota-current.json",
    };

    expect(
      enforcePlacementGuard({ ...config, enabled: false }, { taskId: 1, repo: "r", model: null }, command),
    ).toBeNull();
    expect(
      enforcePlacementGuard(
        config,
        { taskId: 2, repo: "r", model: { providerID: "anthropic", id: "claude-sonnet" } },
        command,
      ),
    ).toBeNull();
    expect(calls).toBe(0);
  });
});
