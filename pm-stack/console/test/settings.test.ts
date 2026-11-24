// The platform's own settings: one definition, carried by the console
// on a machine and by the Worker, and how a machine that says other
// settings is told apart.
import { describe, expect, test } from "bun:test";
import { readConfig } from "../src/config.ts";
import {
  PLATFORM_SETTING_NAMES,
  type PlatformSettings,
  platformSettings,
  settingsThatDiffer,
} from "../src/settings.ts";

describe("the platform's settings", () => {
  // The oracle is the one definition: a console whose environment says
  // nothing runs with exactly it, so a machine and Cloudflare that both
  // follow main agree. A default written out a second time in the
  // console's configuration, and changed in one place only, fails here.
  test("a console whose environment says nothing runs with them", () => {
    const cfg = readConfig({ HOME: "/h" });
    const platform = platformSettings();
    expect(Object.fromEntries(PLATFORM_SETTING_NAMES.map((name) => [name, cfg[name]]))).toEqual({ ...platform });
    expect(settingsThatDiffer(cfg)).toEqual([]);
  });

  test("every setting has a name it is said by, and no name is of anything else", () => {
    expect([...PLATFORM_SETTING_NAMES].map(String).sort()).toEqual(Object.keys(platformSettings()).sort());
  });

  test("whoever is handed them may change theirs without changing anyone else's", () => {
    const mine = platformSettings();
    mine.needsRepos.push("someone/else");
    mine.repo = "someone/else";
    const fresh = platformSettings();
    expect(fresh.needsRepos).not.toContain("someone/else");
    expect(fresh.repo).not.toBe("someone/else");
  });

  // Where the queue's token is sent is one of them: it must be a place
  // reached over TLS, or the Worker would refuse the configuration.
  test("where the queue's store answers is an https address", () => {
    expect(new URL(platformSettings().queueDbUrl).protocol).toBe("https:");
  });
});

describe("telling a machine on other settings apart", () => {
  const other: PlatformSettings = {
    repo: "o/other",
    queue: "another-queue",
    queueDbUrl: "https://elsewhere.example",
    queueTokenName: "another.name",
    androidPackage: "another.package",
    promptLabel: "another/label",
    needsRepos: ["o/other"],
  };

  test("the same settings differ in nothing", () => {
    expect(settingsThatDiffer(platformSettings())).toEqual([]);
    // What else a machine says (who may sign in, its own files) is not compared.
    expect(settingsThatDiffer({ ...readConfig({ HOME: "/h", PM_CONSOLE_OWNER_EMAILS: "a@b.example" }) })).toEqual([]);
  });

  test("each setting that is another is named, and only it", () => {
    for (const name of PLATFORM_SETTING_NAMES) {
      expect(settingsThatDiffer({ ...platformSettings(), [name]: other[name] })).toEqual([name]);
    }
  });

  test("several are named in the order they are said in, whatever order they were told in", () => {
    // Told last-first.
    const told = Object.fromEntries(
      [...PLATFORM_SETTING_NAMES]
        .reverse()
        .map((name) => [
          name,
          name === "repo" || name === "queue" || name === "needsRepos" ? other[name] : platformSettings()[name],
        ]),
    );
    expect(Object.keys(told)[0]).toBe("needsRepos");
    expect(settingsThatDiffer(told)).toEqual(["repo", "queue", "needsRepos"]);
    expect(settingsThatDiffer(other)).toEqual([...PLATFORM_SETTING_NAMES]);
  });

  test("a list of the same repositories in another order is another list", () => {
    const platform = platformSettings();
    expect(settingsThatDiffer({ ...platform, needsRepos: [...platform.needsRepos].reverse() })).toEqual(["needsRepos"]);
    // One fewer, one more, and a list that only begins the same.
    expect(settingsThatDiffer({ ...platform, needsRepos: platform.needsRepos.slice(0, -1) })).toEqual(["needsRepos"]);
    expect(settingsThatDiffer({ ...platform, needsRepos: [...platform.needsRepos, "o/more"] })).toEqual(["needsRepos"]);
    expect(settingsThatDiffer({ ...platform, needsRepos: [] })).toEqual(["needsRepos"]);
    // A copy of the same list is the same list.
    expect(settingsThatDiffer({ ...platform, needsRepos: [...platform.needsRepos] })).toEqual([]);
  });

  test("a setting a machine does not say at all differs, and so does one of another kind", () => {
    const { promptLabel: _unsaid, ...rest } = platformSettings();
    expect(settingsThatDiffer(rest)).toEqual(["promptLabel"]);
    expect(settingsThatDiffer({})).toEqual([...PLATFORM_SETTING_NAMES]);
    // A console from before it said where the queue is says "" and no repositories.
    expect(
      settingsThatDiffer({
        ...platformSettings(),
        queueDbUrl: "",
        queueTokenName: "",
        promptLabel: "",
        needsRepos: [],
      }),
    ).toEqual(["queueDbUrl", "queueTokenName", "promptLabel", "needsRepos"]);
    expect(settingsThatDiffer({ ...platformSettings(), needsRepos: "developing-today/code" })).toEqual(["needsRepos"]);
    expect(settingsThatDiffer({ ...platformSettings(), repo: ["developing-today/code"] })).toEqual(["repo"]);
  });

  test("it is told apart from the settings given, when others than the repository's are", () => {
    expect(settingsThatDiffer(other, other)).toEqual([]);
    expect(settingsThatDiffer(platformSettings(), other)).toEqual([...PLATFORM_SETTING_NAMES]);
  });
});
