/**
 * Hosted read API acceptance suite (#12828).
 *
 * Oracle: issue #12828's acceptance criteria and #9060 §6/§8 —
 * all four endpoints serve the §6 shapes from tenant-DB data; the
 * genealogy sentence renders end-to-end on the §8 worked example
 * (the crate-split chain: steps → #3447 → #3195 → #2056); chain
 * breaks and missing data render honestly, never fabricated.
 *
 * The pool is seeded through slice 2's real landing path
 * (landSnapshot) so the suite also pins collector → API agreement;
 * roadmap rows and milestone snapshots are inserted as the rows
 * slice 1's schema defines (their writers are other slices).
 * The tenant DB is bun:sqlite in-memory with slice 1's real
 * migrations applied.
 */
import { describe, expect, test } from "bun:test";
import { createReadApiHandler, itemJson, projectsJson, releasesJson, roadmapJson } from "../src/read-api/api.ts";
import { deriveGenealogy } from "../src/read-api/genealogy.ts";
import { landSnapshot } from "../src/collector/store.ts";
import type { ProjectAudit, ProjectPoolItem } from "../src/collector/pool.ts";
import { tenantStoreMigrations } from "../src/tenant-store/load.ts";
import { applyMigrations, type SqlDb } from "../src/tenant-store/migrate.ts";
import type { BoardDef } from "../src/tenant-store/schema.ts";
import { openTenantDb } from "../src/tenant-store/sqlite.ts";

const T1 = "2026-11-23T06:00:00.000Z";
const T2 = "2026-11-23T07:00:00.000Z";
const REPO = "developing-today/code";
const ref = (number: number) => `${REPO}#${number}`;

const BOARDS: BoardDef[] = [
  {
    id: "devtoday-main",
    org: "developing-today",
    number: 4,
    nodeId: "PVT_kwDOAZNvF84Bouay",
    url: "https://github.com/orgs/developing-today/projects/4",
    role: "overall",
    siblingOrder: 0,
    title: "Developing Today",
  },
  {
    id: "forks-main",
    org: "developing-today-forks",
    number: 1,
    nodeId: "PVT_kwDOCEOo084BrYAh",
    url: "https://github.com/orgs/developing-today-forks/projects/1",
    role: "project",
    parentBoardId: "devtoday-main",
    siblingOrder: 1,
    title: "Developing Today Forks",
  },
];

// ---------------------------------------------------------------------------
// The #9060 §8 worked example: the crate-split chain, fully native —
// steps #4403–#4411 → #3447 (epic) → #3195 (design) → #2056 (roadmap
// item carrying the release milestone). Modeled with step #4403.
// ---------------------------------------------------------------------------

function poolItem(partial: Partial<ProjectPoolItem> & { number: number; title: string }): ProjectPoolItem {
  const url = `https://github.com/${REPO}/issues/${partial.number}`;
  return {
    id: `PVTI_${partial.number}`,
    databaseId: partial.number * 10,
    url,
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    repo: REPO,
    issueUrl: url,
    state: "open",
    contentKind: "issue",
    labels: [],
    status: "Todo",
    area: "ID",
    horizon: null,
    kind: null,
    ...partial,
    ref: ref(partial.number),
  };
}

const STEP = poolItem({
  number: 4403,
  title: "crate-split step: extract the id-events crate",
  kind: "Task",
  status: "In Progress",
  labels: ["primary-parent/id-crate-split"],
  parentRef: ref(3447),
  milestone: { title: "ID program", dueOn: null, closedAt: null },
});
const EPIC = poolItem({
  number: 3447,
  title: "epic: ID crate split",
  kind: "Epic",
  labels: ["epic", "epic/id-crate-split", "design/id-crate-split"],
  parentRef: ref(3195),
});
const DESIGN = poolItem({
  number: 3195,
  title: "design: ID crate split",
  labels: ["kind/design", "design/id-crate-split"],
  parentRef: ref(2056),
});
const ROADMAP = poolItem({
  number: 2056,
  title: "ID platform roadmap",
  labels: ["kind/roadmap-item"],
  milestone: { title: "ID platform — release 2026-12-01", dueOn: "2026-12-01", closedAt: null },
  blockedBy: [ref(1999)],
});
const ORPHAN = poolItem({ number: 7777, title: "orphan task with no chain", kind: "Task" });
const POOL = [STEP, EPIC, DESIGN, ROADMAP, ORPHAN];

const AUDIT: ProjectAudit = {
  title: "Developing Today",
  number: 4,
  url: "https://github.com/orgs/developing-today/projects/4",
  generatedAt: "2026-11-23T05:59:00.000Z",
  totalItems: POOL.length,
  fields: [{ id: "PVTF_status", name: "Status", dataType: "SINGLE_SELECT" }],
  views: [{ id: "PVTV_1", name: "ID Program", layout: "TABLE_LAYOUT", filter: null }],
  counts: { status: { Todo: 4, "In Progress": 1 }, area: { ID: 5 }, repo: { [REPO]: 5 } },
};

async function seededDb(): Promise<SqlDb> {
  const { sql } = openTenantDb();
  applyMigrations(sql, await tenantStoreMigrations(), () => T1);
  for (const board of BOARDS) {
    sql.run(
      `INSERT INTO pm_boards (board_id, org, number, node_id, url, role, parent_board_id, sibling_order, title, recorded_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      [
        board.id,
        board.org,
        board.number,
        board.nodeId,
        board.url,
        board.role,
        board.parentBoardId ?? null,
        board.siblingOrder,
        board.title,
        T1,
      ],
    );
  }
  landSnapshot(sql, {
    snapshotId: "snap-main-1",
    boardId: "devtoday-main",
    generatedAt: AUDIT.generatedAt,
    audit: AUDIT,
    counts: { reportedTotal: POOL.length, emittedEntries: POOL.length },
    items: POOL,
    now: T1,
  });
  // Roadmap rows: one derived from its tracks link, one with the
  // stored overlay set (the overlay wins where set, #9060 §6).
  sql.run(
    `INSERT INTO pm_roadmap_rows (row_id, subject, title, area, status, horizon, horizon_error, source, notes,
       links_json, priority_ref, after_refs_json, release_ref, grist_url, file_url, recorded_at)
     VALUES (1, 'grist:docA/tblA/1', 'ID crate split', 'ID', 'Active', '', NULL, 'grist', '',
       ?, NULL, '[]', NULL, NULL, NULL, ?)`,
    [JSON.stringify([{ ref: ref(3447), relation: "tracks" }]), T1],
  );
  sql.run(
    `INSERT INTO pm_roadmap_rows (row_id, subject, title, area, status, horizon, horizon_error, source, notes,
       links_json, priority_ref, after_refs_json, release_ref, grist_url, file_url, recorded_at)
     VALUES (2, 'grist:docA/tblA/2', 'Stored overlay row', 'ID', 'Active', '', NULL, 'grist', '',
       '[]', ?, ?, ?, NULL, NULL, ?)`,
    [ref(2000), JSON.stringify([ref(1999)]), ref(2056), T1],
  );
  // A closed release milestone with its stored close-out snapshot.
  sql.run(
    `INSERT INTO pm_milestones (repo, number, title, kind, due_on, closed_at, state, recorded_at)
     VALUES (?, 3, 'ID platform — release 2026-11-01', 'release', '2026-11-01', '2026-11-01T17:00:00.000Z', 'closed', ?)`,
    [REPO, T1],
  );
  sql.run(
    `INSERT INTO pm_release_snapshots (repo, milestone_number, release_date,
       issues_covered, issues_closed, issues_carried_over,
       designs_covered, designs_closed, designs_carried_over,
       epics_covered, epics_closed, epics_carried_over,
       carried_over_epics_json, computed_at, snapshot_json)
     VALUES (?, 3, '2026-11-01', 10, 8, 2, 3, 3, 0, 2, 1, 1, ?, ?, '{}')`,
    [REPO, JSON.stringify([ref(2056)]), T1],
  );
  return sql;
}

const deps = async () => ({ db: await seededDb(), now: () => T2 });

describe("/api/projects.json", () => {
  test("projects[] serves every registry board with its per-board audit", async () => {
    const body = projectsJson(await deps());
    expect(body.generatedAt).toBe(T2);
    expect(body.projects).toHaveLength(2);
    const [main, forks] = body.projects;
    expect(main?.board.id).toBe("devtoday-main");
    expect(main?.audit).toEqual(AUDIT);
    expect(main?.snapshot?.writer).toBe("hosted");
    expect(main?.snapshot?.epoch).toBe(0);
    // A board with no landed snapshot: the absence is served
    // honestly as nulls, never a fabricated audit.
    expect(forks?.board.id).toBe("forks-main");
    expect(forks?.snapshot).toBeNull();
    expect(forks?.audit).toBeNull();
    expect(body.items.map((item) => item.ref)).toEqual([ref(2056), ref(3195), ref(3447), ref(4403), ref(7777)]);
  });
});

describe("/api/item/<id>.json — the #9060 §8 worked example", () => {
  test("the crate-split chain renders end-to-end: ancestors, release, sentence", async () => {
    const { status, body } = itemJson(await deps(), ref(4403));
    expect(status).toBe(200);
    const served = body as {
      ancestors: { ref: string; relation: string }[];
      release: { title: string; dueOn: string | null; ref: string } | null;
      genealogySentence: string;
    };
    expect(served.ancestors).toEqual([
      { ref: ref(3447), title: EPIC.title, kind: "Epic", relation: "parent" },
      { ref: ref(3195), title: DESIGN.title, kind: null, relation: "parent" },
      { ref: ref(2056), title: ROADMAP.title, kind: null, relation: "parent" },
      { ref: ref(3447), title: EPIC.title, kind: "Epic", relation: "primary-parent" },
      { ref: ref(3195), title: DESIGN.title, kind: null, relation: "spawned-by" },
      { ref: ref(2056), title: ROADMAP.title, kind: null, relation: "roadmap" },
    ]);
    expect(served.release).toEqual({
      title: "ID platform — release 2026-12-01",
      dueOn: "2026-12-01",
      ref: ref(2056),
    });
    expect(served.genealogySentence).toBe(
      `Working on ${STEP.title} as part of epic ${EPIC.title} (${ref(3447)}), ` +
        `spawned by design ${DESIGN.title} (${ref(3195)}); on roadmap item ${ROADMAP.title} (${ref(2056)}); ` +
        `release ID platform — release 2026-12-01 due 2026-12-01.`,
    );
  });

  test("a chain break renders honestly: no epic, no design, no roadmap item, no release", async () => {
    const { status, body } = itemJson(await deps(), ref(7777));
    expect(status).toBe(200);
    const served = body as { ancestors: unknown[]; release: unknown; genealogySentence: string };
    expect(served.ancestors).toEqual([]);
    expect(served.release).toBeNull();
    expect(served.genealogySentence).toBe(
      `Working on ${ORPHAN.title} with no epic yet, with no design yet; with no roadmap item yet.`,
    );
  });

  test("an unknown ref is a structured 404, never a fabricated item", async () => {
    const { status, body } = itemJson(await deps(), ref(999999));
    expect(status).toBe(404);
    expect(body).toEqual({ error: `no pool item with ref ${ref(999999)}` });
  });

  test("the priority rung: the release rides the priority item's milestone", () => {
    const priority = poolItem({
      number: 2000,
      title: "overall priority: ID platform",
      labels: ["kind/roadmap-item"],
      milestone: { title: "ID platform — release 2027-01-15", dueOn: "2027-01-15", closedAt: null },
    });
    const item = poolItem({ number: 2002, title: "leaf task", parentRef: ref(2001) });
    const roadmap = poolItem({
      number: 2001,
      title: "ID roadmap item",
      labels: ["kind/roadmap-item"],
      parentRef: ref(2000),
    });
    const genealogy = deriveGenealogy(item, [item, roadmap, priority]);
    expect(genealogy.ancestors).toContainEqual({
      ref: ref(2000),
      title: priority.title,
      kind: null,
      relation: "priority",
    });
    expect(genealogy.release).toEqual({
      title: "ID platform — release 2027-01-15",
      dueOn: "2027-01-15",
      ref: ref(2000),
    });
    expect(genealogy.sentence).toContain(`overall priority ${priority.title} (${ref(2000)})`);
  });
});

describe("/api/roadmap.json", () => {
  test("rows serve priority/afterRefs/releaseRef — derived from tracks links, stored overlay winning where set", async () => {
    const body = roadmapJson(await deps());
    expect(body.rows).toHaveLength(2);
    const [derived, stored] = body.rows;
    // Row 1 has no stored overlay: the overlay is derived over the
    // pool from its tracks link to the crate-split epic.
    expect(derived?.priority).toBe(ref(2056));
    expect(derived?.afterRefs).toEqual([ref(1999)]);
    expect(derived?.releaseRef).toBe(ref(2056));
    // Row 2's stored overlay is served exactly as recorded.
    expect(stored?.priority).toBe(ref(2000));
    expect(stored?.afterRefs).toEqual([ref(1999)]);
    expect(stored?.releaseRef).toBe(ref(2056));
  });
});

describe("/api/releases.json", () => {
  test("stored close-out snapshots are the record; unstored milestones get retroactive accounting", async () => {
    const body = releasesJson(await deps());
    expect(body.releases.map((release) => release.milestone.title)).toEqual([
      "ID platform — release 2026-11-01",
      "ID platform — release 2026-12-01",
      "ID program",
    ]);
    const [stored, open, program] = body.releases;
    // The stored snapshot, served as recorded (§4 close-out shape).
    expect(stored?.kind).toBe("release");
    expect(stored?.releaseDate).toBe("2026-11-01");
    expect(stored?.counts).toEqual({
      issues: { closed: 8, carriedOver: 2, total: 10 },
      designs: { closed: 3, carriedOver: 0, total: 3 },
      epics: { closed: 1, carriedOver: 1, total: 2 },
      total: { closed: 12, carriedOver: 3, total: 15 },
    });
    expect(stored?.carriedOverEpics).toEqual([ref(2056)]);
    expect(stored?.descriptionLine).toBe(
      "Closed with the release on 2026-11-01 covering 15 items: 10 issues, 3 designs, 2 epics (12 closed, 3 carried over).",
    );
    // The open release milestone on the roadmap item: computed.
    expect(open?.kind).toBe("release");
    expect(open?.releaseDate).toBe("2026-12-01");
    expect(open?.counts.total).toEqual({ closed: 0, carriedOver: 1, total: 1 });
    // The ported line builder keys on releaseDate presence, not on
    // closure: an open release milestone with a dueOn renders the
    // "closed with the release on <dueOn>" line. That is the laptop
    // #9067 behavior verbatim — parity ports it, never redesigns it.
    expect(open?.descriptionLine).toBe(
      "Closed with the release on 2026-12-01 covering 1 items: 1 issues, 0 designs, 0 epics (0 closed, 1 carried over).",
    );
    // The rolling program milestone: retroactive counts, no release date.
    expect(program?.kind).toBe("program");
    expect(program?.releaseDate).toBeNull();
    expect(program?.descriptionLine).toBe(
      "Program milestone (rolling, never closes) — retroactive accounting: 1 items: 1 issues, 0 designs, 0 epics (0 closed, 1 carried over).",
    );
  });
});

describe("the fetch handler", () => {
  test("routes the four GET endpoints and 404s everything else", async () => {
    const handler = createReadApiHandler(await deps());
    const get = async (path: string) => {
      const res = await handler(new Request(`http://pm.test${path}`));
      return { status: res.status, body: (await res.json()) as Record<string, unknown> };
    };
    expect((await get("/api/projects.json")).status).toBe(200);
    expect((await get("/api/roadmap.json")).status).toBe(200);
    expect((await get("/api/releases.json")).status).toBe(200);
    const item = await get(`/api/item/${encodeURIComponent(ref(4403))}.json`);
    expect(item.status).toBe(200);
    expect(item.body.genealogySentence).toContain("crate-split step");
    expect((await get(`/api/item/${encodeURIComponent(ref(424242))}.json`)).status).toBe(404);
    expect((await get("/api/nope.json")).status).toBe(404);
    const post = await handler(new Request("http://pm.test/api/projects.json", { method: "POST" }));
    expect(post.status).toBe(404);
  });
});
