import { describe, expect, test } from "bun:test";
import {
  APP_INSTALLED_REPOS,
  FLEET_REPOS,
  REPO_DIRS,
  REPO_QUEUES,
  loadConfig,
} from "../src/config.ts";

describe("fleet config", () => {
  test("the allowlist default is exactly the six fleet repos", () => {
    expect([...FLEET_REPOS]).toEqual([
      "developing-today/code",
      "developing-today/opencode",
      "developing-today/openchamber",
      "dezren39/nix",
      "developing-today/zettel",
      "dezren39/dezren39",
    ]);
    expect(loadConfig({}).repoAllowlist).toEqual([...FLEET_REPOS]);
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

  test("app installations: dezren39/nix is covered; zettel and dezren39/dezren39 are not", () => {
    expect(APP_INSTALLED_REPOS.has("dezren39/nix")).toBe(true);
    expect(APP_INSTALLED_REPOS.has("developing-today/zettel")).toBe(false);
    expect(APP_INSTALLED_REPOS.has("dezren39/dezren39")).toBe(false);
    for (const repo of APP_INSTALLED_REPOS) {
      expect(FLEET_REPOS).toContain(repo as (typeof FLEET_REPOS)[number]);
    }
  });
});
