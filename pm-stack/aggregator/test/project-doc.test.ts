// #1108 remainder: tests for the derived project doc — the
// /program/<id> page's description (from planning/roadmaps/<slug>.md)
// and latest status (from the newest doc/<UTC>_status_<slug>/
// record), read from the code checkout instead of the registry's
// hand-maintained copies.
//
// Oracle: the seed is lab-cluster. On the real repo record, the
// description is the basement-cluster roadmap's own prose and the
// latest status is doc/2026-10-27T03-14-08Z_status_basement-cluster/
// — the fixtures below mirror that record's shapes, and the
// loadProjectDoc test rebuilds it in a temp checkout.
import { describe, expect, test } from "bun:test";
import { mkdtemp, mkdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { GhItem, Provenance, RefKey } from "../src/model.ts";
import {
  buildProjectDocSnapshot,
  loadProjectDoc,
  markdownPlainText,
  parseStatusRecordName,
  roadmapDescription,
  selectLatestStatusRecord,
  statusRecordTitle,
  type ProjectDocTarget,
} from "../src/project-doc.ts";
import { buildProjectDetail, projectConfigById, projectDocTarget } from "../src/project-config.ts";
import { renderProgramProject } from "../src/render.ts";

const TARGET: ProjectDocTarget = {
  repo: "developing-today/code",
  roadmapPath: "planning/roadmaps/basement-cluster.md",
  slug: "basement-cluster",
};

const ROADMAP = `# Roadmap: basement cluster

The basement cluster is a home lab built from the Threadripper machines in the basement: the first 3–5 machines become a Talos Linux cluster running Cozystack, bootstrapped declaratively with TALM.

Hard rules: **no HashiCorp anything**, and **everything bootstrappable from config**. The earlier test cluster has been torn down; those files are historical reference only. Latest status: [2026-10-27 status record](../../doc/2026-10-27T03-14-08Z_status_basement-cluster/2026-10-27T03-14-08Z_status_basement-cluster.md).

## Now

| Title | Status | Intent | Links |
| --- | --- | --- | --- |
| Hardware inventory | planned | Gather stats. | #1080 |
`;

const STATUS = `# Basement cluster status, 2026-10-26

- Status: status record
- Date: 2026-10-27T03-14-08Z
`;

const PROV: Provenance = {
  board: "x",
  roadmap: "x",
  items: "x",
  comments: "x",
  quota: "x",
  usage: "x",
  attention: "x",
  orchestration: "x",
  dns: "x",
  registry: "x",
  aa: "x",
};

describe("roadmapDescription (#1108)", () => {
  test("takes the prose before the first section, links and emphasis as plain text", () => {
    const description = roadmapDescription(ROADMAP);
    expect(description).toContain("The basement cluster is a home lab built from the Threadripper machines");
    expect(description).toContain("no HashiCorp anything");
    expect(description).not.toContain("**");
    expect(description).not.toContain("Hardware inventory"); // table rows are not prose
  });

  test("cuts the Latest status pointer — it is metadata, not description", () => {
    const description = roadmapDescription(ROADMAP);
    expect(description).toContain("historical reference only.");
    expect(description).not.toContain("Latest status:");
    expect(description).not.toContain("2026-10-27 status record");
  });

  test("a roadmap with no prose before its first section has no description", () => {
    expect(roadmapDescription("# Roadmap: x\n\n## Now\n\n| Title |\n")).toBeNull();
    expect(roadmapDescription("")).toBeNull();
  });
});

describe("status record names (#1108)", () => {
  test("parseStatusRecordName accepts only the _status_ shape with a real timestamp", () => {
    expect(parseStatusRecordName("2026-10-27T03-14-08Z_status_basement-cluster")).toEqual({
      timestamp: "2026-10-27T03-14-08Z",
      slug: "basement-cluster",
    });
    // doc/ carries other record kinds under the same timestamp prefix.
    expect(parseStatusRecordName("2026-10-09T09-59-07Z_design_attenuated_capabilities")).toBeNull();
    expect(parseStatusRecordName("2026-13-99T99-99-99Z_status_basement-cluster")).toBeNull();
    expect(parseStatusRecordName("status_basement-cluster")).toBeNull();
  });

  test("selectLatestStatusRecord picks the newest record for the slug only", () => {
    const names = [
      "2026-10-20T00-00-00Z_status_basement-cluster",
      "2026-10-27T03-14-08Z_status_basement-cluster",
      "2026-11-01T00-00-00Z_status_id",
      "2026-10-09T09-59-07Z_design_attenuated_capabilities",
    ];
    expect(selectLatestStatusRecord(names, "basement-cluster")).toEqual({
      timestamp: "2026-10-27T03-14-08Z",
      dir: "2026-10-27T03-14-08Z_status_basement-cluster",
    });
    expect(selectLatestStatusRecord(names, "mcpx")).toBeNull();
  });

  test("statusRecordTitle reads the record's own heading", () => {
    expect(statusRecordTitle(STATUS)).toBe("Basement cluster status, 2026-10-26");
    expect(statusRecordTitle("")).toBeNull();
  });
});

describe("buildProjectDocSnapshot (#1108)", () => {
  test("blob URL + title when the record file was read; tree URL when it was not", () => {
    const names = ["2026-10-27T03-14-08Z_status_basement-cluster"];
    const read = buildProjectDocSnapshot(TARGET, ROADMAP, names, STATUS, "2026-11-17T00:00:00Z");
    expect(read.description).toContain("home lab");
    expect(read.latestStatus?.timestamp).toBe("2026-10-27T03-14-08Z");
    expect(read.latestStatus?.title).toBe("Basement cluster status, 2026-10-26");
    expect(read.latestStatus?.url).toBe(
      "https://github.com/developing-today/code/blob/main/doc/2026-10-27T03-14-08Z_status_basement-cluster/2026-10-27T03-14-08Z_status_basement-cluster.md",
    );
    const unread = buildProjectDocSnapshot(TARGET, ROADMAP, names, null, "2026-11-17T00:00:00Z");
    expect(unread.latestStatus?.title).toBeNull();
    expect(unread.latestStatus?.url).toBe(
      "https://github.com/developing-today/code/tree/main/doc/2026-10-27T03-14-08Z_status_basement-cluster",
    );
  });

  test("absence is per-half: no status record and no prose stay null, never invented", () => {
    const snap = buildProjectDocSnapshot(TARGET, "# Roadmap: x\n\n## Now\n", [], null, "2026-11-17T00:00:00Z");
    expect(snap.description).toBeNull();
    expect(snap.latestStatus).toBeNull();
  });
});

describe("loadProjectDoc (#1108)", () => {
  test("reads the seed record from a checkout: description + newest status", async () => {
    const root = await mkdtemp(join(tmpdir(), "project-doc-"));
    await mkdir(join(root, "planning", "roadmaps"), { recursive: true });
    await writeFile(join(root, "planning", "roadmaps", "basement-cluster.md"), ROADMAP);
    const older = join(root, "doc", "2026-10-20T00-00-00Z_status_basement-cluster");
    const newest = join(root, "doc", "2026-10-27T03-14-08Z_status_basement-cluster");
    await mkdir(older, { recursive: true });
    await mkdir(newest, { recursive: true });
    await writeFile(join(newest, "2026-10-27T03-14-08Z_status_basement-cluster.md"), STATUS);
    const snap = await loadProjectDoc(root, TARGET);
    expect(snap?.description).toContain("Threadripper machines");
    expect(snap?.latestStatus?.timestamp).toBe("2026-10-27T03-14-08Z");
    expect(snap?.latestStatus?.title).toBe("Basement cluster status, 2026-10-26");
  });

  test("an unreadable checkout yields null — the page falls back, never a fake absence", async () => {
    const root = await mkdtemp(join(tmpdir(), "project-doc-empty-"));
    expect(await loadProjectDoc(root, TARGET)).toBeNull();
  });

  test("a missing roadmap with a readable doc/ is a verified absence, not null", async () => {
    const root = await mkdtemp(join(tmpdir(), "project-doc-noroadmap-"));
    await mkdir(join(root, "doc"), { recursive: true });
    const snap = await loadProjectDoc(root, TARGET);
    expect(snap).not.toBeNull();
    expect(snap?.description).toBeNull();
    expect(snap?.latestStatus).toBeNull();
  });
});

describe("projectDocTarget + render (#1108)", () => {
  test("only roadmap home docs in the code repo qualify", () => {
    expect(projectDocTarget(projectConfigById("lab-cluster")!)).toEqual(TARGET);
    expect(projectDocTarget(projectConfigById("id")!)?.slug).toBe("id");
    // pm-dashboard's home doc is a design record, not a roadmap.
    expect(projectDocTarget(projectConfigById("pm-dashboard")!)).toBeNull();
    expect(projectDocTarget(projectConfigById("cloud-program")!)).toBeNull();
  });

  test("the lab-cluster page renders the derived description + latest status instead of the registry copies", () => {
    const lab = projectConfigById("lab-cluster")!;
    const snap = buildProjectDocSnapshot(
      TARGET,
      ROADMAP,
      ["2026-10-27T03-14-08Z_status_basement-cluster"],
      STATUS,
      "2026-11-17T00:00:00Z",
    );
    const items = new Map<RefKey, GhItem>();
    const html = renderProgramProject(buildProjectDetail(lab, items, [], [], snap), items, PROV);
    expect(html).toContain("The basement cluster is a home lab built from the Threadripper machines");
    expect(html).not.toContain("A home lab built from the ~12 Threadripper machines"); // the hand-maintained intro
    expect(html).toContain("Latest status:");
    expect(html).toContain("Basement cluster status, 2026-10-26");
    expect(html).toContain(
      "doc/2026-10-27T03-14-08Z_status_basement-cluster/2026-10-27T03-14-08Z_status_basement-cluster.md",
    );
    expect(html).not.toContain("Status report:"); // the hand-pinned copy does not render alongside
  });

  test("a snapshot with neither half renders explicit absences, not the hand-maintained copies", () => {
    const lab = projectConfigById("lab-cluster")!;
    const snap = buildProjectDocSnapshot(TARGET, null, [], null, "2026-11-17T00:00:00Z");
    const items = new Map<RefKey, GhItem>();
    const html = renderProgramProject(buildProjectDetail(lab, items, [], [], snap), items, PROV);
    expect(html).toContain("No status record under");
    expect(html).toContain("No prose description found in planning/roadmaps/basement-cluster.md");
    expect(html).not.toContain("Status report:");
  });

  test("markdownPlainText collapses links and strips emphasis", () => {
    expect(markdownPlainText("a **bold** [link](https://example.com) and `code`")).toBe("a bold link and code");
  });
});
