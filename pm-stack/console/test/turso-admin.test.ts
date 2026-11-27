// Turso management operations from the app (#14257).
//
// Oracles: the issue's acceptance criteria, written out by hand —
// the operations run through a workflow with the organisation token
// never in the Worker/console, the workflow's outcome is visible, and
// a failed workflow is a failure, not a silent no-op. The token-name
// scan's oracle is the repository itself: the organisation token's
// secret name may appear only in the one workflow file.
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { readConfig, type Config } from "../src/config.ts";
import { Ops, type Run } from "../src/ops.ts";
import {
  TURSO_ADMIN_WORKFLOW,
  TURSO_GROUP,
  TURSO_ORG_TOKEN_SECRET,
  tursoAdminInputs,
  tursoAdminRequest,
  tursoAdminRun,
} from "../src/turso-admin.ts";

const ROOT = join(import.meta.dir, "..", "..", "..");
const WORKFLOW = readFileSync(join(ROOT, ".github", "workflows", TURSO_ADMIN_WORKFLOW), "utf8");

test("a request: what each operation must name, and what is refused", () => {
  expect(tursoAdminRequest({ operation: "create-db", database: "fleet-new" })).toEqual({
    operation: "create-db",
    database: "fleet-new",
    group: TURSO_GROUP,
    expiry: null,
    secretName: null,
  });
  expect(
    tursoAdminRequest({ operation: "mint-token", database: "fleet-new", secret_name: "turso_fleet_new_rw_token" }),
  ).toEqual({
    operation: "mint-token",
    database: "fleet-new",
    group: null,
    expiry: "4w",
    secretName: "turso_fleet_new_rw_token",
  });
  for (const bad of [
    { operation: "drop-db", database: "fleet-new" },
    { operation: "create-db", database: "Fleet New" },
    { operation: "create-db", database: "../x" },
    { operation: "create-db", database: "fleet-new", group: "other-group" },
    { operation: "mint-token", database: "fleet-new" }, // no secret name to hold the token
    { operation: "mint-token", database: "fleet-new", secret_name: "has space" },
    { operation: "mint-token", database: "fleet-new", secret_name: "bad-name" },
    { operation: "mint-token", database: "fleet-new", secret_name: "GITHUB_TOKEN" },
    { operation: "mint-token", database: "fleet-new", secret_name: "github_token" },
    { operation: "mint-token", database: "fleet-new", secret_name: "TURSO_PLATFORM_API_TOKEN" },
    { operation: "mint-token", database: "fleet-new", secret_name: "turso_platform_api_token" },
    { operation: "mint-token", database: "fleet-new", secret_name: "REPO_AUTOMATION_TOKEN" },
    { operation: "mint-token", database: "fleet-new", secret_name: "ok_name", expiry: "a while" },
  ]) {
    expect(() => tursoAdminRequest(bad)).toThrow();
  }
});

test("dispatch inputs carry names and choices only, never a token value", () => {
  expect(tursoAdminInputs(tursoAdminRequest({ operation: "create-db", database: "fleet-new" }))).toEqual({
    operation: "create-db",
    database: "fleet-new",
    group: TURSO_GROUP,
  });
  const mint = tursoAdminInputs(
    tursoAdminRequest({
      operation: "mint-token",
      database: "fleet-new",
      secret_name: "turso_fleet_new_rw_token",
      expiry: "30d",
    }),
  );
  expect(mint).toEqual({
    operation: "mint-token",
    database: "fleet-new",
    expiry: "30d",
    secret_name: "turso_fleet_new_rw_token",
  });
  expect(JSON.stringify(mint)).not.toContain("Bearer");
});

test("a run's outcome: a failure stays a failure", () => {
  const base = { created_at: "2026-11-25T00:00:00Z", html_url: "https://example.invalid/run/1" };
  expect(tursoAdminRun({ ...base, status: "completed", conclusion: "success" })).toEqual({
    state: "success",
    failed: false,
    at: base.created_at,
    url: base.html_url,
  });
  expect(tursoAdminRun({ ...base, status: "completed", conclusion: "failure" }).failed).toBe(true);
  expect(tursoAdminRun({ ...base, status: "completed", conclusion: null }).state).toBe("unknown");
  expect(tursoAdminRun({ ...base, status: "in_progress", conclusion: null })).toEqual({
    state: "in_progress",
    failed: false,
    at: base.created_at,
    url: base.html_url,
  });
});

test("the workflow: born --tursodb in the named group, minted token straight into the secret store", () => {
  expect(WORKFLOW).toContain("workflow_dispatch");
  expect(WORKFLOW).toContain("--tursodb");
  expect(WORKFLOW).toContain("--group");
  expect(WORKFLOW).toContain("gh secret set");
  // The minted token is never printed: no step echoes the token file.
  expect(WORKFLOW).not.toMatch(/echo[^|\n]*token_file/);
  expect(WORKFLOW).not.toMatch(/cat[^|\n]*token_file/);
  // GitHub expressions never enter shell program text; inputs arrive through env.
  const runScripts: string[] = [];
  let inRun = false;
  for (const line of WORKFLOW.split("\n")) {
    if (/^        run: \|$/.test(line)) {
      inRun = true;
    } else if (inRun && line.trim() && !line.startsWith("          ")) {
      inRun = false;
    } else if (inRun) {
      runScripts.push(line.slice(10));
    }
  }
  expect(runScripts.join("\n")).not.toContain("${{");
  expect(WORKFLOW).toContain("INPUT_DATABASE: ${{ inputs.database }}");
  expect(WORKFLOW).toContain("INPUT_SECRET_NAME: ${{ inputs.secret_name }}");
  expect(WORKFLOW).toContain('== GITHUB_* || "$secret_name_upper" == TURSO_PLATFORM_API_TOKEN');
  expect(WORKFLOW).toContain("REPO_AUTOMATION_TOKEN");
});

test("the organisation token is referenced by name only, in that one workflow", () => {
  expect(WORKFLOW).toContain(`secrets.${TURSO_ORG_TOKEN_SECRET}`);
  const tracked = Bun.spawnSync(["git", "ls-files", "--", ".github/workflows", "pm-stack/console"], { cwd: ROOT })
    .stdout.toString()
    .split("\n")
    .filter(Boolean);
  const holders = tracked.filter((f) => readFileSync(join(ROOT, f), "utf8").includes(TURSO_ORG_TOKEN_SECRET));
  expect(holders.sort()).toEqual(
    [
      `.github/workflows/${TURSO_ADMIN_WORKFLOW}`,
      "pm-stack/console/src/turso-admin.ts",
      "pm-stack/console/test/turso-admin.test.ts",
    ].sort(),
  );
  // The console names it (documentation of the split) but never reads a secret by it.
  const consoleSrc = tracked
    .filter((f) => f.startsWith("pm-stack/console/src/"))
    .map((f) => readFileSync(join(ROOT, f), "utf8"))
    .join("\n");
  expect(consoleSrc).not.toMatch(/secrets.*TURSO|TURSO.*secret store value/);
});

const CFG: Config = readConfig({
  HOME: "/h",
  PM_CONSOLE_OWNER_EMAILS: "owner@example.com",
  PM_CONSOLE_ANDROID_SHA256: Array(32).fill("AB").join(":"),
});

test("Ops dispatches the admin workflow on main and reads its last run", async () => {
  const calls: { argv: string[]; stdin?: string }[] = [];
  const run: Run = async (argv, opts) => {
    calls.push({ argv, stdin: opts?.stdin });
    const path = argv.find((a) => a.startsWith("repos/")) ?? "";
    if (path.includes("/runs")) {
      return {
        code: 0,
        stdout: JSON.stringify({
          workflow_runs: [
            {
              status: "completed",
              conclusion: "failure",
              created_at: "2026-11-25T00:00:00Z",
              html_url: "https://example.invalid/run/9",
            },
          ],
        }),
        stderr: "",
      };
    }
    return { code: 0, stdout: "", stderr: "" };
  };
  const ops = new Ops(CFG, run);
  expect(await ops.tursoAdmin({ operation: "create-db", database: "fleet-new" })).toEqual({
    started: "create-db",
    database: "fleet-new",
  });
  const dispatch = calls.find((c) => c.argv.some((a) => a.includes("/dispatches")));
  expect(dispatch?.argv).toContain(`repos/developing-today/code/actions/workflows/${TURSO_ADMIN_WORKFLOW}/dispatches`);
  expect(JSON.parse(dispatch?.stdin ?? "null")).toEqual({
    ref: "main",
    inputs: { operation: "create-db", database: "fleet-new", group: TURSO_GROUP },
  });
  // A failed last run is reported as failed, not hidden.
  expect(await ops.tursoAdminLastRun()).toEqual({
    state: "failure",
    failed: true,
    at: "2026-11-25T00:00:00Z",
    url: "https://example.invalid/run/9",
  });
  // A refused request never reaches GitHub.
  const before = calls.length;
  await expect(ops.tursoAdmin({ operation: "drop-db", database: "fleet-new" })).rejects.toThrow();
  expect(calls.length).toBe(before);
});
