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
}

const POOL_BY_PROVIDER: Readonly<Record<string, string>> = {
  "opencode-go": "opencode-go",
  "cline-pass": "cline-pass",
};

/** Resolve the serviceId used by quota-current for this explicit model. */
export function poolForModel(model: ModelRef | null): string | null {
  if (!model) return null;
  if (Object.hasOwn(POOL_BY_PROVIDER, model.providerID)) return POOL_BY_PROVIDER[model.providerID]!;
  return null;
}

/**
 * Run placement-record before this dispatch creates an OpenCode session.
 * This first wiring slice covers explicit OpenCode Go and ClinePass hints;
 * unknown/default routes remain for the follow-up mapping slice.
 */
export function enforcePlacementGuard(
  config: PlacementGuardConfig,
  request: PlacementRequest,
  run: (command: PlacementCommand) => PlacementCommandResult = (command) =>
    runPlacementCommand(config.executable, command),
): string | null {
  if (!config.enabled) return null;
  const pool = poolForModel(request.model);
  if (!pool) return null;
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
      pool,
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
  return pool;
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
