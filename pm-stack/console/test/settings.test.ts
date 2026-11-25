// The platform's own settings: one definition, carried by the console
// on a machine and by the Worker, and how a machine that says other
// settings is told apart.
import { describe, expect, test } from "bun:test";
import { readConfig } from "../src/config.ts";
import {
  effectiveSettings,
  goesBy,
  MACHINES_FOR_NOW,
  OWNER_CHANGEABLE,
  PLATFORM_OWN,
  PLATFORM_SETTING_NAMES,
  type PlatformSettings,
  platformSettings,
  settingsDifferences,
  settingsInForce,
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

describe("what the platform goes by", () => {
  const other: PlatformSettings = {
    repo: "o/other",
    queue: "another-queue",
    queueDbUrl: "https://elsewhere.example",
    queueTokenName: "another.name",
    androidPackage: "another.package",
    promptLabel: "another/label",
    needsRepos: ["o/other"],
  };

  // Every setting is one or the other: a new one that is neither is
  // gone by nobody's, and one that is both would be the machine's after all.
  test("each setting is either the platform's own or, for now, a machine's: never both, never neither", () => {
    expect([...PLATFORM_OWN, ...MACHINES_FOR_NOW].map(String).sort()).toEqual(
      [...PLATFORM_SETTING_NAMES].map(String).sort(),
    );
    expect(new Set<string>([...PLATFORM_OWN, ...MACHINES_FOR_NOW]).size).toBe(PLATFORM_SETTING_NAMES.length);
  });

  // The one that decides where a secret is sent must be the platform's own.
  test("where the queue's token is sent is the platform's own", () => {
    expect(PLATFORM_OWN).toContain("queueDbUrl");
    expect(goesBy(other).queueDbUrl).toBe(platformSettings().queueDbUrl);
  });

  test("its own for what is its own, the machine's for the list of repositories, and everything else as it was told", () => {
    const told = { ...other, owners: ["a@b.example"], androidApp: true };
    const by = goesBy(told);
    const platform = platformSettings();
    for (const name of PLATFORM_OWN) expect([name, by[name]]).toEqual([name, platform[name]]);
    expect(by.needsRepos).toEqual(other.needsRepos);
    expect([by.owners, by.androidApp]).toEqual([["a@b.example"], true]);
    // What it was handed is not changed.
    expect(told.queue).toBe(other.queue);
    expect(by).not.toBe(told);
  });

  test("a machine on the platform's settings is gone by as it is", () => {
    const told = { ...platformSettings(), owners: ["a@b.example"] };
    expect(goesBy(told)).toEqual(told);
  });

  test("gone by the settings given, when others than the repository's are", () => {
    expect(goesBy(platformSettings(), other)).toEqual({ ...other, needsRepos: platformSettings().needsRepos });
  });
});

describe("saying a machine's other settings to the owner", () => {
  test("each is said with what the machine says, what the platform has, and which is gone by", () => {
    const platform = platformSettings();
    expect(settingsDifferences(platform)).toEqual([]);
    expect(settingsDifferences({ ...platform, queue: "another-queue", needsRepos: ["o/other"] })).toEqual([
      { name: "queue", machine: "another-queue", platform: platform.queue, goesBy: "platform" },
      { name: "needsRepos", machine: ["o/other"], platform: platform.needsRepos, goesBy: "machine" },
    ]);
  });

  test("which is gone by is what goesBy does", () => {
    const other = { ...platformSettings(), queue: "q", repo: "o/r", needsRepos: ["o/r"], promptLabel: "l" };
    const by = goesBy(other) as unknown as Record<string, unknown>;
    for (const d of settingsDifferences(other)) {
      expect([d.name, by[d.name]]).toEqual([d.name, d.goesBy === "platform" ? d.platform : d.machine]);
    }
  });

  test("a setting the machine does not say is said as nothing, and what it says is not shown without end", () => {
    const { promptLabel: _unsaid, ...rest } = platformSettings();
    expect(settingsDifferences(rest)).toEqual([
      { name: "promptLabel", machine: null, platform: platformSettings().promptLabel, goesBy: "platform" },
    ]);
    const long = settingsDifferences({
      ...platformSettings(),
      queue: "q".repeat(1_000),
      needsRepos: Array.from({ length: 300 }, (_, i) => `${"o".repeat(500)}/r${i}`),
    });
    expect((long[0].machine as string).length).toBe(200);
    expect((long[1].machine as string[]).length).toBe(50);
    expect((long[1].machine as string[]).every((item) => item.length === 200)).toBe(true);
    // What the platform has is its own, and is said whole.
    expect(long[1].platform).toEqual(platformSettings().needsRepos);
    // What is not a list or a string is said as it is.
    expect(settingsDifferences({ ...platformSettings(), queue: 7 })[0].machine).toBe(7);
  });
});

describe("owner-changeable settings", () => {
  const platform = {
    ...platformSettings(),
    promptLabel: "repo/prompt",
    needsRepos: ["repo/project"],
  };
  const told: PlatformSettings = {
    ...platform,
    repo: "machine/repo",
    queue: "machine-queue",
    promptLabel: "machine/prompt",
    needsRepos: ["machine/project"],
  };

  test("S1 oracle: repository → machine → owner precedence follows design §3", () => {
    const owner = {
      rev: 4,
      at: 1_700_000_000_000,
      by: "owner@example.test",
      values: { promptLabel: "owner/prompt" },
    };
    const effective = effectiveSettings(told, owner, platform);

    expect(OWNER_CHANGEABLE).toEqual(["promptLabel", "needsRepos"]);
    expect(effective.repo).toBe(platform.repo);
    expect(effective.queue).toBe(platform.queue);
    expect(effective.promptLabel).toBe(owner.values.promptLabel);
    // No owner override exists yet, so needsRepos still comes from the machine.
    expect(effective.needsRepos).toEqual(told.needsRepos);
    expect(told.promptLabel).toBe("machine/prompt");
    expect(told.needsRepos).toEqual(["machine/project"]);
  });

  test("owner needsRepos overrides the machine, and clearing reveals the machine again", () => {
    const owner = {
      rev: 5,
      at: 1_700_000_001_000,
      by: "owner@example.test",
      values: { needsRepos: ["owner/project"] },
    };

    expect(effectiveSettings(told, owner, platform).needsRepos).toEqual(["owner/project"]);
    expect(effectiveSettings(told, { ...owner, values: {} }, platform).needsRepos).toEqual(told.needsRepos);
    expect(effectiveSettings(told, { ...owner, values: {} }, platform).promptLabel).toBe(platform.promptLabel);
  });

  test("in-force projection names source and timestamps while keeping repository contrast", () => {
    const owner = {
      rev: 6,
      at: 1_700_000_002_000,
      by: "owner@example.test",
      values: { promptLabel: "owner/prompt", needsRepos: ["owner/project"] },
    };
    const rows = settingsInForce(told, owner, platform);

    expect(rows).toHaveLength(PLATFORM_SETTING_NAMES.length);
    expect(rows.find((row) => row.name === "promptLabel")).toEqual({
      name: "promptLabel",
      source: "owner",
      since: owner.at,
      value: owner.values.promptLabel,
      repository: platform.promptLabel,
      by: owner.by,
      rev: owner.rev,
    });
    expect(rows.find((row) => row.name === "needsRepos")).toEqual({
      name: "needsRepos",
      source: "owner",
      since: owner.at,
      value: owner.values.needsRepos,
      repository: platform.needsRepos,
      by: owner.by,
      rev: owner.rev,
    });
    expect(rows.find((row) => row.name === "queue")).toEqual({
      name: "queue",
      source: "repository",
      since: null,
      value: platform.queue,
      repository: platform.queue,
    });
    expect(settingsInForce(null, null, platform).find((row) => row.name === "needsRepos")).toEqual({
      name: "needsRepos",
      source: "repository",
      since: null,
      value: platform.needsRepos,
      repository: platform.needsRepos,
    });
  });
});
