import { describe, expect, test } from "bun:test";
import { APP_INSTALLED_REPOS, FLEET_REPOS, REPO_DIRS, REPO_QUEUES, loadConfig } from "../src/config.ts";

/** loadConfig requires SUPABASE_URL (#5120); tests use a fixture project. */
const ENV = { SUPABASE_URL: "https://fixture.supabase.co" };

describe("fleet config", () => {
  for (const value of ["abc", "4879.5", "0", "-1", "99999", "0x1234", "1e4"]) {
    test(`rejects invalid DISPATCH_PORT ${JSON.stringify(value)}`, () => {
      expect(() => loadConfig({ DISPATCH_PORT: value })).toThrow(`DISPATCH_PORT "${value}"`);
    });
  }

  test("keeps the default and accepts a decimal DISPATCH_PORT", () => {
    expect(loadConfig({ ...ENV }).port).toBe(4879);
    expect(loadConfig({ ...ENV, DISPATCH_PORT: "" }).port).toBe(4879);
    expect(loadConfig({ ...ENV, DISPATCH_PORT: "   " }).port).toBe(4879);
    expect(loadConfig({ ...ENV, DISPATCH_PORT: "4879" }).port).toBe(4879);
  });

  for (const value of [",", " , ,"]) {
    test(`rejects DISPATCH_HOSTS ${JSON.stringify(value)} when it has no hosts`, () => {
      expect(() => loadConfig({ DISPATCH_HOSTS: value })).toThrow(`DISPATCH_HOSTS "${value}"`);
    });
  }

  test("keeps a normal DISPATCH_HOSTS list unchanged", () => {
    expect(loadConfig({ ...ENV, DISPATCH_HOSTS: "0.0.0.0, 127.0.0.1" }).hosts).toEqual(["0.0.0.0", "127.0.0.1"]);
  });

  test("the allowlist default is exactly the six fleet repos", () => {
    expect([...FLEET_REPOS]).toEqual([
      "developing-today/code",
      "developing-today-forks/opencode",
      "developing-today-forks/openchamber",
      "dezren39/darwin-nix",
      "developing-today/zettel",
      "dezren39/dezren39",
    ]);
    expect(loadConfig({ ...ENV }).repoAllowlist).toEqual([...FLEET_REPOS]);
  });

  test("SUPABASE_URL is required and has no default (#5120)", () => {
    expect(() => loadConfig({})).toThrow("SUPABASE_URL is not set");
    expect(() => loadConfig({ SUPABASE_URL: "" })).toThrow("SUPABASE_URL is not set");
    expect(() => loadConfig({ SUPABASE_URL: "   " })).toThrow("SUPABASE_URL is not set");
  });

  test("SUPABASE_URL is used verbatim, minus a trailing slash", () => {
    expect(loadConfig({ SUPABASE_URL: "https://fixture.supabase.co" }).supabaseUrl).toBe("https://fixture.supabase.co");
    expect(loadConfig({ SUPABASE_URL: "https://fixture.supabase.co/" }).supabaseUrl).toBe(
      "https://fixture.supabase.co",
    );
  });

  test("every fleet repo has a queue and a session directory", () => {
    for (const repo of FLEET_REPOS) {
      expect(REPO_QUEUES[repo]).toBeTruthy();
      expect(REPO_DIRS[repo]).toMatch(/^\//);
    }
  });

  test("the new repos use their pilot queues and laptop checkouts", () => {
    expect(REPO_QUEUES["developing-today/zettel"]).toBe("repo-health-zettel-pilot");
    expect(REPO_QUEUES["dezren39/dezren39"]).toBe("repo-health-dezren39-pilot");
    expect(REPO_DIRS["developing-today/zettel"]).toBe("/home/user/git/zettel");
    expect(REPO_DIRS["dezren39/dezren39"]).toBe("/home/user/dezren39");
  });

  test("app installations: dezren39/darwin-nix is covered; zettel and dezren39/dezren39 are not", () => {
    expect(APP_INSTALLED_REPOS.has("dezren39/darwin-nix")).toBe(true);
    expect(APP_INSTALLED_REPOS.has("developing-today/zettel")).toBe(false);
    expect(APP_INSTALLED_REPOS.has("dezren39/dezren39")).toBe(false);
    for (const repo of APP_INSTALLED_REPOS) {
      expect(FLEET_REPOS).toContain(repo as (typeof FLEET_REPOS)[number]);
    }
  });
});
