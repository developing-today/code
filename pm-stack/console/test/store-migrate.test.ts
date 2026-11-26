// Store migrations started from the app (#14256).
//
// Oracles: the issue's acceptance criteria, written out by hand — the
// app starts a migration run the way a deploy is started (a dispatch of
// the migration workflow on main, names only), shows each store's last
// run, and shows a failed run as failed with a link to the run. The
// workflow's own choices are the oracle for which stores can be named.
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { migrationSaid, MIGRATABLE_STORES } from "../public/app.js";
import { actions } from "../src/actions.ts";
import { readConfig, type Config } from "../src/config.ts";
import { HOLDABLE } from "../src/edge-proto.ts";
import { Ops, type Run } from "../src/ops.ts";
import {
	lastMigrationRuns,
	MIGRATE_RUN_PREFIX,
	MIGRATE_TARGETS,
	STORE_MIGRATE_WORKFLOW,
	storeMigrateInputs,
	storeMigrateRequest,
} from "../src/store-migrate.ts";

const ROOT = join(import.meta.dir, "..", "..", "..");
const WORKFLOW = readFileSync(
	join(ROOT, ".github", "workflows", STORE_MIGRATE_WORKFLOW),
	"utf8",
);
const STORES = JSON.parse(
	readFileSync(join(ROOT, "pm-stack", "platform", "stores.json"), "utf8"),
) as {
	stores: { name: string }[];
};

test("the console names exactly the workflow's choices, and every listed store is one", () => {
	const options = WORKFLOW.split("options:")[1]
		.split("\n")
		.map((line) => line.match(/^ {10}- ([a-z0-9-]+)$/)?.[1])
		.filter(Boolean);
	expect(options).toEqual([...MIGRATE_TARGETS]);
	const listed = STORES.stores.map((store) => store.name).sort();
	expect([...MIGRATABLE_STORES].sort()).toEqual(listed);
	const real: string[] = MIGRATE_TARGETS.filter(
		(target) => target !== "stand-in",
	).sort();
	expect(real).toEqual(listed);
});

test("each run is titled with its store, so the last run per store can be read", () => {
	expect(WORKFLOW).toContain(
		`run-name: ${MIGRATE_RUN_PREFIX}\${{ inputs.store }}`,
	);
});

test("a request: the stand-in needs nothing more, a real store needs its name repeated", () => {
	expect(storeMigrateRequest({ store: "stand-in" })).toEqual({
		store: "stand-in",
		confirm: "",
	});
	expect(
		storeMigrateRequest({ store: "fleet-queue", confirm_store: "fleet-queue" }),
	).toEqual({
		store: "fleet-queue",
		confirm: "fleet-queue",
	});
	for (const bad of [
		{},
		{ store: "fleet-nope" },
		{ store: "fleet-queue" },
		{ store: "fleet-queue", confirm_store: "fleet-mirror" },
		{ store: "fleet-queue", confirm_store: " fleet-queue" },
		{ store: ["fleet-queue"], confirm_store: "fleet-queue" },
	]) {
		expect(() => storeMigrateRequest(bad)).toThrow();
	}
	expect(
		storeMigrateInputs(storeMigrateRequest({ store: "stand-in" })),
	).toEqual({ store: "stand-in" });
	expect(
		storeMigrateInputs(
			storeMigrateRequest({
				store: "fleet-mirror",
				confirm_store: "fleet-mirror",
			}),
		),
	).toEqual({
		store: "fleet-mirror",
		confirm_store: "fleet-mirror",
	});
});

const run = (title: string, over: Record<string, unknown> = {}) => ({
	display_title: title,
	status: "completed",
	conclusion: "success",
	created_at: "2026-11-26T10:00:00Z",
	head_sha: "0123456789abcdef0123",
	html_url: `https://example.invalid/${title.replace(/ /g, "/")}`,
	...over,
});

test("the newest run per store, a failure kept a failure, untitled runs passed over", () => {
	const got = lastMigrationRuns([
		run("pm-store-migrate fleet-queue", {
			conclusion: "failure",
			created_at: "2026-11-26T12:00:00Z",
		}),
		run("pm-store-migrate stand-in", {
			status: "in_progress",
			conclusion: null,
		}),
		run("pm-store-migrate fleet-queue"),
		run("pm-store-migrate"),
		run("pm-store-migrate fleet-nope"),
	]);
	expect(Object.keys(got).sort()).toEqual(["fleet-queue", "stand-in"]);
	expect(got["fleet-queue"]).toEqual({
		state: "failure",
		failed: true,
		at: "2026-11-26T12:00:00Z",
		commit: "0123456789ab",
		url: "https://example.invalid/pm-store-migrate/fleet-queue",
	});
	expect(got["stand-in"]).toMatchObject({
		state: "in_progress",
		failed: false,
	});
	expect(lastMigrationRuns([run("x", { conclusion: null })])).toEqual({});
	expect(
		lastMigrationRuns([
			run("pm-store-migrate fleet-mirror", { conclusion: null }),
		])["fleet-mirror"],
	).toMatchObject({
		state: "unknown",
		failed: true,
	});
});

test("the Stores section says a failed run as failed, and no run as none", () => {
	const NOW = Date.parse("2026-11-26T10:08:00Z");
	const done = {
		state: "success",
		failed: false,
		at: "2026-11-26T10:00:00Z",
		commit: "0123456789ab",
		url: "u",
	};
	expect(migrationSaid(undefined, NOW)).toEqual({
		text: "No migration has been run from here yet.",
		tone: "",
	});
	expect(migrationSaid(done, NOW)).toEqual({
		text: "Last migration: Done · 8 min ago · 0123456",
		tone: "",
	});
	expect(
		migrationSaid({ ...done, state: "failure", failed: true }, NOW),
	).toEqual({
		text: "Last migration failed: Failed · 8 min ago · 0123456",
		tone: "warn",
	});
});

const CFG: Config = readConfig({
	HOME: "/h",
	PM_CONSOLE_OWNER_EMAILS: "owner@example.com",
	PM_CONSOLE_ANDROID_SHA256: Array(32).fill("AB").join(":"),
});

test("Ops dispatches the migration workflow on main and reads the last run per store", async () => {
	const calls: { argv: string[]; stdin?: string }[] = [];
	const fake: Run = async (argv, opts) => {
		calls.push({ argv, stdin: opts?.stdin });
		const path = argv.find((a) => a.startsWith("repos/")) ?? "";
		if (path.includes("/runs")) {
			return {
				code: 0,
				stdout: JSON.stringify({
					workflow_runs: [
						run("pm-store-migrate fleet-platform", { conclusion: "failure" }),
					],
				}),
				stderr: "",
			};
		}
		return { code: 0, stdout: "", stderr: "" };
	};
	const ops = new Ops(CFG, fake);
	expect(
		await ops.storeMigrate({
			store: "fleet-platform",
			confirm_store: "fleet-platform",
		}),
	).toEqual({
		started: "fleet-platform",
	});
	const dispatch = calls.find((c) =>
		c.argv.some((a) => a.includes("/dispatches")),
	);
	expect(dispatch?.argv).toContain(
		`repos/developing-today/code/actions/workflows/${STORE_MIGRATE_WORKFLOW}/dispatches`,
	);
	expect(JSON.parse(dispatch?.stdin ?? "null")).toEqual({
		ref: "main",
		inputs: { store: "fleet-platform", confirm_store: "fleet-platform" },
	});
	const read = await ops.storeMigrations();
	expect(read.runs["fleet-platform"]).toMatchObject({
		state: "failure",
		failed: true,
	});
	expect(read.runs["fleet-platform"].url).toBe(
		"https://example.invalid/pm-store-migrate/fleet-platform",
	);
	// A refused request never reaches GitHub.
	const before = calls.length;
	await expect(ops.storeMigrate({ store: "fleet-platform" })).rejects.toThrow(
		/repeat the store name/,
	);
	expect(calls.length).toBe(before);
});

test("the action keeps the store's name only, and is never done late", async () => {
	const table = actions(
		new Ops(CFG, async () => ({ code: 0, stdout: "", stderr: "" })),
		{} as never,
	);
	expect(
		table["store.migrate"].audit({
			store: "fleet-queue",
			confirm_store: "fleet-queue",
			extra: "x",
		}),
	).toEqual({
		store: "fleet-queue",
	});
	expect(await table["store.migrate"].run({ store: "stand-in" })).toEqual({
		started: "stand-in",
	});
	expect(HOLDABLE.has("store.migrate")).toBe(false);
});
