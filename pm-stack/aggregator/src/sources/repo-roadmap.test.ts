import { describe, expect, test } from "bun:test";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { mergeRoadmapRows, parseRepoRoadmapFile, readRepoRoadmaps } from "./repo-roadmap.ts";
import type { RoadmapRow } from "../model.ts";

// An id.md-shaped code roadmap file: horizon lives in the ## heading,
// refs are bare #n against the file's own repo.
const CODE_FILE = `# Roadmap: id

The id program record.

## Now — the path to Phase 12

| Title              | Status      | Intent                 | Links |
| ------------------ | ----------- | ---------------------- | ----- |
| Many worlds per node | planned   | 1,000 idle / 100 active worlds. | F12.12 #557 |
| Trusted CI signal  | in-progress | A red check must mean something. | #241, [plan](https://github.com/developing-today/code/issues/999) |

## Ideas

| Title           | Status | Intent        | Links |
| --------------- | ------ | ------------- | ----- |
| Worlds on Workers | idea | A pkarr option. | #793  |
`;

const CODE_META = { repo: "developing-today/code", path: "planning/roadmaps/id.md" };

// A nix ROADMAP.md-shaped file: an "Areas" summary table (no Title
// column — must be skipped), then per-area sections whose tables carry
// the Horizon column; area comes from the section heading.
const NIX_FILE = `# Roadmap

Grouped by area.

## Areas

| Area       | Current focus |
| ---------- | ------------- |
| [Widgets](#widgets) | All the widgets |

## Widgets

| Title            | Horizon | Status   | Intent              | Links |
| ---------------- | ------- | -------- | ------------------- | ----- |
| Widget takeover  | Now     | planned  | Take over widgets.  | \`developing-today/code#237\` |
| Widget hygiene   | Next    | proposed | Sweep the widgets.  | #376 |
`;

const NIX_META = {
  repo: "dezren39/darwin-nix",
  path: "ROADMAP.md",
  program: "darwin-nix",
  areaFromSections: true,
};

describe("parseRepoRoadmapFile — code planning/roadmaps shape", () => {
  const rows = parseRepoRoadmapFile(CODE_FILE, CODE_META);

  test("the id program appears with zero Grist rows", () => {
    expect(rows.length).toBe(3);
    expect(rows.every((r) => r.area === "id")).toBe(true);
    expect(rows.every((r) => r.subject.startsWith("repo-file:developing-today/code/planning/roadmaps/id.md/"))).toBe(
      true,
    );
  });

  test("horizon comes from the section heading; Ideas is its own bucket, not an error", () => {
    expect(rows[0].horizon).toBe("Now");
    expect(rows[2].horizon).toBe("Ideas");
    expect(rows[2].horizonError).toBeUndefined();
  });

  test("bare and URL refs resolve against the file's repo; file link points at the heading", () => {
    expect(rows[0].links).toEqual([{ ref: "developing-today/code#557", relation: "tracks" }]);
    expect(rows[1].links.map((l) => l.ref).sort()).toEqual(["developing-today/code#241", "developing-today/code#999"]);
    expect(rows[0].fileUrl).toBe(
      "https://github.com/developing-today/code/blob/main/planning/roadmaps/id.md#now--the-path-to-phase-12",
    );
    expect(rows[0].source).toContain("repo file");
    expect(rows[0].notes).toContain("1,000 idle");
  });
});

describe("parseRepoRoadmapFile — nix ROADMAP.md shape", () => {
  const rows = parseRepoRoadmapFile(NIX_FILE, NIX_META);

  test("the Areas summary table is skipped; section tables parse with their Horizon column", () => {
    expect(rows.length).toBe(2);
    expect(rows.map((r) => r.title)).toEqual(["Widget takeover", "Widget hygiene"]);
    expect(rows[0].area).toBe("Widgets");
    expect(rows[0].horizon).toBe("Now");
    expect(rows[1].horizon).toBe("Next");
  });

  test("qualified refs keep their repo; bare refs take the file's repo", () => {
    expect(rows[0].links).toEqual([{ ref: "developing-today/code#237", relation: "tracks" }]);
    expect(rows[1].links).toEqual([{ ref: "dezren39/darwin-nix#376", relation: "tracks" }]);
  });

  test("a file with no Title table parses empty", () => {
    expect(parseRepoRoadmapFile("# Roadmap\n\nJust prose, no tables.\n", NIX_META)).toEqual([]);
  });
});

const gristRow = (title: string, subject: string): RoadmapRow => ({
  id: 7,
  subject,
  title,
  area: "id",
  status: "planned",
  horizon: "Now",
  source: "",
  notes: "",
  links: [{ ref: "developing-today/code#557", relation: "tracks" }],
});

describe("mergeRoadmapRows — dedupe prefers the repo row", () => {
  test("a Grist duplicate collapses into the repo row with provenance noted", () => {
    const repo = parseRepoRoadmapFile(CODE_FILE, CODE_META).map((r, i) => ({ ...r, id: -1 - i }));
    const grist = [
      gristRow("Many Worlds Per Node!", "grist:doc/Roadmap/7"),
      gristRow("Only in Grist", "grist:doc/Roadmap/8"),
    ];
    const { rows, collapsedCount } = mergeRoadmapRows(grist, repo);
    expect(collapsedCount).toBe(1);
    expect(rows.some((r) => r.subject === "grist:doc/Roadmap/7")).toBe(false);
    expect(rows.some((r) => r.subject === "grist:doc/Roadmap/8")).toBe(true);
    const twin = rows.find((r) => r.title === "Many worlds per node");
    expect(twin?.notes).toContain("grist:doc/Roadmap/7");
    expect(twin?.links).toEqual([{ ref: "developing-today/code#557", relation: "tracks" }]);
  });
});

describe("readRepoRoadmaps — section states say why (#1500)", () => {
  test("ok: both sources read, ids are unique negatives", async () => {
    const root = await mkdtemp(join(tmpdir(), "repo-roadmap-"));
    await mkdir(join(root, "code", "planning", "roadmaps"), { recursive: true });
    await writeFile(join(root, "code", "planning", "roadmaps", "id.md"), CODE_FILE);
    await mkdir(join(root, "nix"), { recursive: true });
    await writeFile(join(root, "nix", "ROADMAP.md"), NIX_FILE);
    const { rows, sections } = await readRepoRoadmaps({
      codeCheckout: join(root, "code"),
      nixCheckout: join(root, "nix"),
    });
    expect(rows.length).toBe(5);
    expect(new Set(rows.map((r) => r.id)).size).toBe(5);
    expect(rows.every((r) => r.id < 0)).toBe(true);
    expect(sections.map((s) => s.state)).toEqual(["ok", "ok"]);
  });

  test("missing: neither checkout carries the files", async () => {
    const root = await mkdtemp(join(tmpdir(), "repo-roadmap-"));
    const { rows, sections } = await readRepoRoadmaps({
      codeCheckout: join(root, "code"),
      nixCheckout: join(root, "nix"),
    });
    expect(rows).toEqual([]);
    expect(sections.map((s) => s.state)).toEqual(["missing", "missing"]);
    expect(sections[0].reason).toContain("PM_AGG_CODE_CHECKOUT");
    expect(sections[1].reason).toContain("PM_AGG_NIX_CHECKOUT");
  });

  test("parse-empty: files exist but carry no Title-table rows", async () => {
    const root = await mkdtemp(join(tmpdir(), "repo-roadmap-"));
    await mkdir(join(root, "code", "planning", "roadmaps"), { recursive: true });
    await writeFile(join(root, "code", "planning", "roadmaps", "empty.md"), "# Roadmap: empty\n\nNo tables yet.\n");
    await mkdir(join(root, "nix"), { recursive: true });
    await writeFile(join(root, "nix", "ROADMAP.md"), "# Roadmap\n\nNothing parsed.\n");
    const { sections } = await readRepoRoadmaps({ codeCheckout: join(root, "code"), nixCheckout: join(root, "nix") });
    expect(sections.map((s) => s.state)).toEqual(["parse-empty", "parse-empty"]);
  });

  test("unreadable: a file where a directory is expected (and vice versa)", async () => {
    const root = await mkdtemp(join(tmpdir(), "repo-roadmap-"));
    await mkdir(join(root, "code", "planning"), { recursive: true });
    await writeFile(join(root, "code", "planning", "roadmaps"), "not a directory");
    await mkdir(join(root, "nix", "ROADMAP.md"), { recursive: true });
    const { sections } = await readRepoRoadmaps({ codeCheckout: join(root, "code"), nixCheckout: join(root, "nix") });
    expect(sections.map((s) => s.state)).toEqual(["unreadable", "unreadable"]);
  });
});
