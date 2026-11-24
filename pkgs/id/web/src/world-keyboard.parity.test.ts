/**
 * Web half of the kitty-disambiguate-v1 cross-language parity oracle
 * (#787 Slice D, design doc/2026-11-01_design_id787.md §6 row D).
 *
 * The shared corpus at `pkgs/id/doc/kitty-disambiguate-v1-fixtures.json`
 * is the oracle. Cases carrying a `browser` field name the browser key
 * event for the same keystroke as the fixture's terminal input; this
 * test pins the rich encoder (`richTerminalBytesForKey`) to emit
 * exactly the corpus's `browser.bytes` for that event. The Roc half
 * (`scripts/key-parity-test.sh`, wired into `scripts/roc-test.sh`)
 * decodes both the terminal input and those browser bytes with the
 * actual compiled Roc `Key.parse_events` and compares them against
 * the corpus's shared expected rich events — so the browser encoder
 * and a kitty terminal cannot drift apart silently.
 */
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { richTerminalBytesForKey } from "./world-keyboard";

type BrowserSpec = {
  key: string;
  ctrlKey?: boolean;
  altKey?: boolean;
  metaKey?: boolean;
  shiftKey?: boolean;
  bytes: string;
};

type FixtureCase = {
  name: string;
  input: string;
  expected: unknown[];
  browser?: BrowserSpec;
};

// The corpus lives at pkgs/id/doc/, one level above this package;
// vitest runs from the package root, so resolve from the working
// directory with an explicit search list (same pattern as the
// screen-parity twin).
const corpusCandidates = [
  path.resolve(process.cwd(), "../doc/kitty-disambiguate-v1-fixtures.json"),
  path.resolve(process.cwd(), "doc/kitty-disambiguate-v1-fixtures.json"),
];
const corpusPath = corpusCandidates.find((candidate) => existsSync(candidate));
if (!corpusPath) {
  throw new Error(
    `kitty-disambiguate-v1 corpus not found; tried: ${corpusCandidates.join(", ")} — the parity oracle must not skip silently (#787 Slice D)`,
  );
}
const corpus = JSON.parse(readFileSync(corpusPath, "utf8")) as {
  profile: string;
  cases: FixtureCase[];
};
const browserCases = corpus.cases.filter((fixture) => fixture.browser !== undefined);

describe("kitty-disambiguate-v1 parity corpus (#787 Slice D)", () => {
  it("corpus is complete enough to be an oracle", () => {
    expect(corpus.profile).toBe("kitty-disambiguate-v1");
    expect(corpus.cases.length).toBeGreaterThanOrEqual(40);
    for (const fixture of corpus.cases) {
      expect(fixture.input.length, fixture.name).toBeGreaterThan(0);
      expect(fixture.expected.length, fixture.name).toBeGreaterThan(0);
    }
    // The browser subset must stay large enough that the parity claim
    // is real; shrinking it weakens the oracle, so fail loudly here.
    expect(browserCases.length).toBeGreaterThanOrEqual(20);
  });

  for (const fixture of browserCases) {
    it(`browser encoder emits the corpus bytes: ${fixture.name}`, () => {
      const spec = fixture.browser!;
      const bytes = richTerminalBytesForKey({
        key: spec.key,
        ctrlKey: spec.ctrlKey ?? false,
        altKey: spec.altKey ?? false,
        metaKey: spec.metaKey ?? false,
        shiftKey: spec.shiftKey ?? false,
      });
      expect(bytes, fixture.name).not.toBeNull();
      expect(bytes, fixture.name).toEqual(new TextEncoder().encode(spec.bytes));
    });
  }
});
