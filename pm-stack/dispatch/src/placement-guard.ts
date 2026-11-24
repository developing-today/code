import { spawnSync } from "node:child_process";
import type { ModelRef } from "./validate.ts";

export interface PlacementGuardConfig {
  enabled: boolean;
  executable: string;
  policyFile: string;
  quotaCurrentFile: string;
  logError?: (line: string) => void;
}

export interface PlacementCommand {
  args: string[];
  input: string;
}

export interface PlacementCommandResult {
  status: number | null;
  stdout: string;
  stderr: string;
  error?: Error;
}

export interface PlacementRequest {
  taskId: number;
  repo: string;
  model: ModelRef | null;
  activeGoAccountLabel?: string;
}

export interface PlacementGuardResult {
  poolId: string;
  sessionModel: ModelRef;
}

const GO_ACCOUNT_SUFFIXES = new Set(["drewrypope", "popesdrew", "developing-today-0", "0x111dev"]);
const POOL_BY_PROVIDER: Readonly<Record<string, string>> = {
  "opencode-go.drewrypope": "opencode-go.drewrypope",
  "opencode-go.popesdrew": "opencode-go.popesdrew",
  "opencode-go.developing-today-0": "opencode-go.developing-today-0",
  "opencode-go.0x111dev": "opencode-go.0x111dev",
  "opencode.drewrypope": "opencode-go.drewrypope",
  "opencode.popesdrew": "opencode-go.popesdrew",
  "opencode.developing-today-0": "opencode-go.developing-today-0",
  "opencode.0x111dev": "opencode-go.0x111dev",
  "cline-pass": "cline-pass",
};

/** Resolve an explicit model to its quota serviceId and session provider. */
export function placementForModel(model: ModelRef | null, activeGoAccountLabel?: string): PlacementGuardResult | null {
  if (!model) return null;
  if (model.providerID === "opencode-go" || model.providerID === "opencode") {
    if (!activeGoAccountLabel || !GO_ACCOUNT_SUFFIXES.has(activeGoAccountLabel)) {
      throw new Error("placement-record: cannot identify the active OpenCode Go account");
    }
    const poolId = `opencode-go.${activeGoAccountLabel}`;
    const providerPrefix = model.providerID === "opencode" ? "opencode" : "opencode-go";
    return {
      poolId,
      // Bind the session to the account just checked so a concurrent
      // Console-account switch cannot change which quota the session uses.
      sessionModel: { ...model, providerID: `${providerPrefix}.${activeGoAccountLabel}` },
    };
  }
  if (Object.hasOwn(POOL_BY_PROVIDER, model.providerID)) {
    return { poolId: POOL_BY_PROVIDER[model.providerID]!, sessionModel: model };
  }
  return null;
}

/**
 * Run placement-record before this dispatch creates an OpenCode session.
 * This first wiring slice covers explicit Go/Zen account routes, a bare Go
 * route resolved to its active account, and ClinePass hints;
 * unknown/default routes remain for the follow-up mapping slice.
 */
export function enforcePlacementGuard(
  config: PlacementGuardConfig,
  request: PlacementRequest,
  run: (command: PlacementCommand) => PlacementCommandResult = (command) =>
    runPlacementCommand(config.executable, command),
): PlacementGuardResult | null {
  if (!config.enabled) return null;
  const placement = placementForModel(request.model, request.activeGoAccountLabel);
  if (!placement) return null;
  if (!config.policyFile.trim()) throw new Error("placement-record: policy file is not configured");
  if (!config.quotaCurrentFile.trim()) throw new Error("placement-record: quota-current file is not configured");

  const command: PlacementCommand = {
    args: [
      "--policy",
      config.policyFile,
      "--answers",
      "/dev/stdin",
      "--quota-current",
      config.quotaCurrentFile,
      "--pool",
      placement.poolId,
    ],
    input: JSON.stringify({
      task: `queue task ${request.taskId} / ${request.repo}`,
      // pm-dispatch runs on the laptop and launches the default R4
      // OpenCode-session option. The local-state guard excludes only the
      // options documented by R2; it does not bypass the monthly guard.
      q2_needs_local_or_secrets: true,
    }),
  };
  const result = run(command);
  const stderr = result.stderr.trim();
  if (result.error || result.status !== 0) {
    const detail = stderr || result.error?.message || `exited with status ${String(result.status)}`;
    const line = detail.startsWith("placement-record:") ? detail : `placement-record: ${detail}`;
    (config.logError ?? console.error)(line);
    throw new Error(line);
  }

  let record: unknown;
  try {
    record = JSON.parse(result.stdout);
  } catch {
    throw new Error("placement-record: successful result was not valid JSON");
  }
  if (
    record === null ||
    typeof record !== "object" ||
    (record as { option?: unknown }).option !== "opencode-session" ||
    (record as { rule?: unknown }).rule !== "R4"
  ) {
    throw new Error("placement-record: expected the default R4 opencode-session record");
  }
  if ((record as { b4_unknown_read_relied_on?: unknown }).b4_unknown_read_relied_on === true) {
    const b4Line = stderr.startsWith("placement-record: B4")
      ? stderr
      : `placement-record: B4 last-known quota relied on for pool ${JSON.stringify(placement.poolId)}`;
    (config.logError ?? console.error)(b4Line);
  }
  return placement;
}

function runPlacementCommand(executable: string, command: PlacementCommand): PlacementCommandResult {
  const result = spawnSync(executable, command.args, {
    encoding: "utf8",
    input: command.input,
    maxBuffer: 256 * 1024,
  });
  return {
    status: result.status,
    stdout: result.stdout ?? "",
    stderr: result.stderr ?? "",
    ...(result.error ? { error: result.error } : {}),
  };
}
