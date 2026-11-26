import { join } from "node:path";
import { requireHome } from "./home.ts";

// C3 health hook (#9571): fire-and-forget opencode-working from a caller
// that found OpenCode unreachable. One hook process at a time per
// aggregator; the hook dedupes its own incidents, so a skipped firing
// loses nothing.
let running: { exitCode: number | null } | null = null;

export function fireOpenCodeWorkingHook(caller: string, detail: string): boolean {
  if (running && running.exitCode === null) return false;
  const hookPath = join(
    requireHome("the opencode-working health hook"),
    "git",
    "code",
    "pm-stack",
    "health-hooks",
    "opencode-working.sh",
  );
  try {
    running = Bun.spawn([hookPath, "--caller", caller, "--detail", detail], {
      stdout: "ignore",
      stderr: "ignore",
    });
    return true;
  } catch (err) {
    console.error(`health-hook: failed to spawn opencode-working: ${err instanceof Error ? err.message : err}`);
    return false;
  }
}
