import { describe, expect, test } from "bun:test";
import { needsUserStatePath } from "../src/needs-user-state-path.ts";

const dir = "/state/pm-aggregator";
const select = (env: Record<string, string | undefined>, files: string[] = []) => {
  const present = new Set(files.map((file) => `${dir}/${file}`));
  return needsUserStatePath(
    env,
    () => dir,
    (path) => present.has(path),
  );
};

describe("needs-user question state path migration", () => {
  test("the new explicit override wins over the legacy env and both files", () => {
    expect(
      select(
        {
          PM_AGG_NEEDS_USER_FILE: "/custom/needs-user.json",
          PM_AGG_NEEDS_DREW_FILE: "/custom/needs-drew.json",
        },
        ["needs-user.json", "needs-drew.json"],
      ),
    ).toBe("/custom/needs-user.json");
  });

  test("a blank new override falls back to the legacy env name", () => {
    expect(select({ PM_AGG_NEEDS_USER_FILE: "  ", PM_AGG_NEEDS_DREW_FILE: "/legacy/questions.json" })).toBe(
      "/legacy/questions.json",
    );
  });

  test("an existing new file wins, and the existing legacy file remains a fallback", () => {
    expect(select({}, ["needs-user.json", "needs-drew.json"])).toBe(`${dir}/needs-user.json`);
    expect(select({}, ["needs-drew.json"])).toBe(`${dir}/needs-drew.json`);
  });

  test("a fresh state directory defaults to the new write path", () => {
    expect(select({})).toBe(`${dir}/needs-user.json`);
  });
});
