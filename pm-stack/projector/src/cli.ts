// CLI: one-off reconciliation against the live GitHub API.
//   bun run src/cli.ts reconcile
import { openDb, migrate } from "./store";
import { reconcileRepos } from "./reconcile";

const cmd = process.argv[2] ?? "reconcile";
const env = process.env as Record<string, string | undefined>;

const db = openDb(env);
await migrate(db);

if (cmd === "reconcile") {
  console.log("[projector] reconcile start:", env.PM_REPOS);
  const result = await reconcileRepos(db, env);
  console.log(JSON.stringify(result, null, 2));
} else {
  console.error(`unknown command: ${cmd}`);
  process.exit(2);
}
