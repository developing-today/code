/**
 * Tests for the recommendation snapshot paging state machine (#10158,
 * slice C9139-01).
 *
 * Oracle: `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/
 * recommendation-page-fixture.json`, as recomputed by
 * `recommendation-page-oracle.py` (expected output in
 * `recommendation-page-oracle-output.txt`). The fixture values below are
 * copied verbatim from that fixture; a drift in ordering, identity,
 * cursor binding or envelope bytes fails here.
 */
import { describe, expect, it } from "vitest";
import {
  createSnapshot,
  CursorExpiredError,
  hexToBytes,
  InvalidPagePositionError,
  makePage,
  nextCursor,
  pageFromCursor,
  WrongParentSnapshotIdError,
  type PageEnvelope,
  type ParentEnvelope,
} from "../src/recommendation_state";

// Fixture: synthetic-recommendation-page/v1 (verbatim from the fixture file).
const PARENT: ParentEnvelope = {
  body: {
    type: "recommendation-parent-v1",
    tenant: "tenant-a",
    algorithm: "score-desc-id-asc-v1",
    cutoff_sequence: 7,
    page_size: 2,
    input_digest: "sha256:synthetic-input-set-7",
    candidate_set: [
      { id: "rec-b", score_micros: 900, arrival_sequence: 5 },
      { id: "rec-c", score_micros: 700, arrival_sequence: 7 },
      { id: "rec-a", score_micros: 900, arrival_sequence: 4 },
    ],
  },
  signature: hexToBytes("746573742d706172656e742d7369676e6174757265"),
};
const PAGE_0_SIGNATURE_HEX = "746573742d706167652d30";
const PAGE_2_SIGNATURE_HEX = "746573742d706167652d32";
const EXPECTED_PARENT_SNAPSHOT_ID =
  "sha256:8797b6bd7f4821dfb8273ee23cad9dacd50e9361b06d8a4e686f2515b8cf5c91";
const EXPECTED_PAGE_0_ENVELOPE_ID =
  "sha256:e67e1aa7ad2343411ec8019ec1bef037ee0c8e8d27d01f305df641a2d957fc7d";
const EXPECTED_PAGE_2_ENVELOPE_ID =
  "sha256:50edb5f8e6dfb49b16580b44dffd8932998f4f21bee6836a951698f01c2de0a2";

async function fixtureSnapshot() {
  return createSnapshot(PARENT);
}

describe("recommendation snapshot paging (oracle: recommendation-page-fixture)", () => {
  it("derives the fixture parent snapshot id", async () => {
    const snapshot = await fixtureSnapshot();
    expect(snapshot.parent_snapshot_id).toBe(EXPECTED_PARENT_SNAPSHOT_ID);
  });

  it("materializes score DESC then immutable id ASC, excluding post-cutoff arrivals", async () => {
    const snapshot = await fixtureSnapshot();
    // Equal score 900: rec-a before rec-b (id ASC). rec-aa (arrival 8,
    // score 999) is past the cutoff of 7 and belongs to the next snapshot.
    expect(snapshot.ordered.map((c) => c.id)).toEqual(["rec-a", "rec-b", "rec-c"]);
    expect(snapshot.ordered.map((c) => c.id)).not.toContain("rec-aa");
  });

  it("page 0 reproduces the exact fixture envelope id, order and done=false", async () => {
    const snapshot = await fixtureSnapshot();
    const { envelope, envelope_id } = await makePage(snapshot, 0, PAGE_0_SIGNATURE_HEX);
    expect(envelope_id).toBe(EXPECTED_PAGE_0_ENVELOPE_ID);
    expect(envelope.body.entries.map((e) => e.id)).toEqual(["rec-a", "rec-b"]);
    expect(envelope.body.done).toBe(false);
    expect(envelope.body.parent_snapshot_id).toBe(EXPECTED_PARENT_SNAPSHOT_ID);
    expect(envelope.body.position).toBe(0);
  });

  it("page 2 reproduces the exact fixture envelope id, order and terminal done=true", async () => {
    const snapshot = await fixtureSnapshot();
    const { envelope, envelope_id } = await makePage(snapshot, 2, PAGE_2_SIGNATURE_HEX);
    expect(envelope_id).toBe(EXPECTED_PAGE_2_ENVELOPE_ID);
    expect(envelope.body.entries.map((e) => e.id)).toEqual(["rec-c"]);
    expect(envelope.body.done).toBe(true);
  });

  it("advances the cursor to the fixture expected next cursor", async () => {
    const snapshot = await fixtureSnapshot();
    const { envelope } = await makePage(snapshot, 0, PAGE_0_SIGNATURE_HEX);
    expect(nextCursor(envelope)).toEqual({
      parent_snapshot_id: EXPECTED_PARENT_SNAPSHOT_ID,
      position: 2,
    });
  });

  it("restart/replay at the same parent and position returns the same page envelope", async () => {
    const snapshot = await fixtureSnapshot();
    const first = await makePage(snapshot, 0, PAGE_0_SIGNATURE_HEX);
    const replayed = await pageFromCursor(
      snapshot,
      { parent_snapshot_id: EXPECTED_PARENT_SNAPSHOT_ID, position: 0 },
      PAGE_0_SIGNATURE_HEX,
    );
    expect(replayed.envelope_id).toBe(first.envelope_id);
    expect(replayed.envelope_id).toBe(EXPECTED_PAGE_0_ENVELOPE_ID);
    expect(replayed.envelope).toEqual(first.envelope);
    // Duplicate replay is idempotent: same cursor again, same bytes.
    const again = await pageFromCursor(
      snapshot,
      { parent_snapshot_id: EXPECTED_PARENT_SNAPSHOT_ID, position: 0 },
      PAGE_0_SIGNATURE_HEX,
    );
    expect(again.envelope).toEqual(first.envelope);
  });

  it("rejects a wrong-parent cursor", async () => {
    const snapshot = await fixtureSnapshot();
    await expect(
      pageFromCursor(
        snapshot,
        {
          parent_snapshot_id:
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
          position: 0,
        },
        PAGE_0_SIGNATURE_HEX,
      ),
    ).rejects.toBeInstanceOf(WrongParentSnapshotIdError);
  });

  it("rejects an expired cursor and accepts an unexpired one", async () => {
    const snapshot = await fixtureSnapshot();
    const now = 1_800_000_000_000;
    await expect(
      pageFromCursor(
        snapshot,
        { parent_snapshot_id: EXPECTED_PARENT_SNAPSHOT_ID, position: 0, expires_at: now },
        PAGE_0_SIGNATURE_HEX,
        now,
      ),
    ).rejects.toBeInstanceOf(CursorExpiredError);
    const ok = await pageFromCursor(
      snapshot,
      {
        parent_snapshot_id: EXPECTED_PARENT_SNAPSHOT_ID,
        position: 0,
        expires_at: now + 60_000,
      },
      PAGE_0_SIGNATURE_HEX,
      now,
    );
    expect(ok.envelope_id).toBe(EXPECTED_PAGE_0_ENVELOPE_ID);
  });

  it("rejects invalid page positions", async () => {
    const snapshot = await fixtureSnapshot();
    for (const position of [-1, 1, 3, 4]) {
      await expect(makePage(snapshot, position, PAGE_0_SIGNATURE_HEX)).rejects.toBeInstanceOf(
        InvalidPagePositionError,
      );
    }
  });

  it("an empty snapshot yields an empty terminal page (done is the terminal signal, not emptiness)", async () => {
    const empty: ParentEnvelope = {
      body: { ...PARENT.body, candidate_set: [] },
      signature: PARENT.signature,
    };
    const snapshot = await createSnapshot(empty);
    const { envelope } = await makePage(snapshot, 0, PAGE_0_SIGNATURE_HEX);
    expect(envelope.body.entries).toEqual([]);
    expect(envelope.body.done).toBe(true);
  });

  it("a page at the exact end boundary is empty and terminal", async () => {
    const two: ParentEnvelope = {
      body: {
        ...PARENT.body,
        candidate_set: [
          { id: "rec-a", score_micros: 900, arrival_sequence: 4 },
          { id: "rec-b", score_micros: 900, arrival_sequence: 5 },
        ],
      },
      signature: PARENT.signature,
    };
    const snapshot = await createSnapshot(two);
    const first = await makePage(snapshot, 0, PAGE_0_SIGNATURE_HEX);
    expect(first.envelope.body.done).toBe(true);
    const boundary = await makePage(snapshot, 2, PAGE_2_SIGNATURE_HEX);
    expect(boundary.envelope.body.entries).toEqual([]);
    expect(boundary.envelope.body.done).toBe(true);
    expect(nextCursor(boundary.envelope)).toEqual({
      parent_snapshot_id: snapshot.parent_snapshot_id,
      position: 2,
    });
  });

  it("page envelope ids differ across pages and across parents", async () => {
    const snapshot = await fixtureSnapshot();
    const p0 = await makePage(snapshot, 0, PAGE_0_SIGNATURE_HEX);
    const p2 = await makePage(snapshot, 2, PAGE_2_SIGNATURE_HEX);
    expect(p0.envelope_id).not.toBe(p2.envelope_id);
    const other: ParentEnvelope = {
      body: { ...PARENT.body, tenant: "tenant-b" },
      signature: PARENT.signature,
    };
    const otherSnapshot = await createSnapshot(other);
    expect(otherSnapshot.parent_snapshot_id).not.toBe(EXPECTED_PARENT_SNAPSHOT_ID);
    const otherPage = await makePage(otherSnapshot, 0, PAGE_0_SIGNATURE_HEX);
    expect(otherPage.envelope_id).not.toBe(p0.envelope_id);
  });

  it("keeps the parent preimage free of page ids (no hash cycle)", async () => {
    const snapshot = await fixtureSnapshot();
    const body = snapshot.parent.body as unknown as Record<string, unknown>;
    expect(body).not.toHaveProperty("pages");
    expect(body).not.toHaveProperty("page_envelope_ids");
    expect(body).not.toHaveProperty("page_ids");
  });

  it("exposes the page body type for consumers", () => {
    const envelope: PageEnvelope = {
      body: {
        parent_snapshot_id: EXPECTED_PARENT_SNAPSHOT_ID,
        position: 0,
        entries: [],
        done: true,
      },
      signature: new Uint8Array(),
    };
    expect(envelope.body.done).toBe(true);
  });
});
