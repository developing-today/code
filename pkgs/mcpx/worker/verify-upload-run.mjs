// Verify the GitHub Actions run that produced a promotion artifact.
// Usage: verify-upload-run.mjs <run-id> <attempt> <verified-run.json>
// Adapted from nixos/services/id-mail/worker/verify-upload-run.mjs.
import { fileURLToPath } from "node:url";
import { writeFileSync } from "node:fs";

export function verifyUploadRun(run, { runId, attempt, repository }) {
  if (run?.id !== Number(runId) || run?.run_attempt !== Number(attempt) ||
      run?.status !== "completed" || run?.conclusion !== "success" || run?.event !== "workflow_dispatch" ||
      run?.head_branch !== "main" || run?.path !== ".github/workflows/mcpx-worker-publish.yml" ||
      run?.head_sha?.length !== 40 || !/^[0-9a-f]{40}$/.test(run.head_sha) ||
      run?.repository?.full_name?.toLowerCase() !== repository.toLowerCase()) {
    throw new Error("selected upload run is not a successful manual upload on main in this repository");
  }
  return {
    id: run.id,
    attempt: run.run_attempt,
    sha: run.head_sha,
    repository: run.repository.full_name,
    workflow: run.path,
    event: run.event,
    ref: `refs/heads/${run.head_branch}`,
    conclusion: run.conclusion,
  };
}

async function main() {
  const args = process.argv.slice(2);
  if (args.length !== 3 || !/^\d+$/.test(args[0]) || !/^\d+$/.test(args[1]) || !args[2]) {
    console.error("usage: verify-upload-run.mjs <run-id> <attempt> <verified-run.json>");
    process.exit(2);
  }
  const repository = process.env.GITHUB_REPOSITORY;
  const token = process.env.GITHUB_TOKEN;
  if (!repository || !token) throw new Error("GITHUB_REPOSITORY and GITHUB_TOKEN are required");
  const response = await fetch(`https://api.github.com/repos/${repository}/actions/runs/${args[0]}/attempts/${args[1]}`, {
    headers: { Accept: "application/vnd.github+json", Authorization: `Bearer ${token}`, "X-GitHub-Api-Version": "2022-11-28" },
  });
  if (!response.ok) throw new Error(`GitHub Actions run lookup failed with HTTP ${response.status}`);
  const verified = verifyUploadRun(await response.json(), { runId: args[0], attempt: args[1], repository });
  writeFileSync(args[2], `${JSON.stringify(verified, null, 2)}\n`, { mode: 0o600 });
  console.log("selected upload attempt is successful and came from the main-branch publisher workflow");
}

if (process.argv[1] === fileURLToPath(import.meta.url)) await main();
