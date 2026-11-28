#!/usr/bin/env python3
"""Deterministic feed-cursor model oracle; no runtime/system claims.

CLI contract: no positionals; optional --check. Parse the complete CLI before
constructing the fixture or running its model. The oracle performs no I/O.
"""

import argparse
import sys


class CursorInvalidated(Exception):
    pass


def require(condition, message):
    if not condition:
        raise AssertionError(message)


class FeedLedger:
    def __init__(
        self, server, tenant, query, generation, durable_high_water=0, entries=()
    ):
        self.server = server
        self.tenant = tenant
        self.query = query
        self.generation = generation
        self.high_water = durable_high_water
        self.entries = list(entries)
        self.seen_sources = {entry["source_key"] for entry in self.entries}

    def append(
        self, authority, source_attribute, epoch, source_sequence, source_object_id
    ):
        source_key = (
            authority,
            source_attribute,
            epoch,
            source_sequence,
            source_object_id,
        )
        if source_key in self.seen_sources:
            for entry in self.entries:
                if entry["source_key"] == source_key:
                    return entry
        self.high_water += 1
        entry = {
            "projection_seq": self.high_water,
            "entry_id": f"feed-entry-{self.high_water:08d}",
            "source_key": source_key,
            "source_authority": authority,
            "source_attribute": source_attribute,
            "source_epoch": epoch,
            "source_sequence": source_sequence,
            "source_object_id": source_object_id,
        }
        self.entries.append(entry)
        self.seen_sources.add(source_key)
        return entry

    def scope(self):
        return self.server, self.tenant, self.query, self.generation

    def complete_snapshot(self, cutoff_frontier):
        visible = []
        for entry in self.entries:
            cutoff = cutoff_frontier.get(
                (entry["source_authority"], entry["source_attribute"])
            )
            source_position = (entry["source_epoch"], entry["source_sequence"])
            if cutoff is not None and source_position <= cutoff:
                visible.append(entry)
        return sorted(visible, key=lambda e: (e["projection_seq"], e["entry_id"]))

    def cursor(self, snapshot, cutoff_frontier, after=None):
        last = after or (0, "")
        return {
            "scope": self.scope(),
            "last_projection_seq": last[0],
            "last_entry_id": last[1],
            "cutoff_frontier": dict(cutoff_frontier),
        }

    def _validate(self, cursor):
        if cursor["scope"] != self.scope():
            raise CursorInvalidated("scope or projection generation changed")

    def since(self, cursor):
        self._validate(cursor)
        last = (cursor["last_projection_seq"], cursor["last_entry_id"])
        snapshot = self.complete_snapshot(cursor["cutoff_frontier"])
        return [e for e in snapshot if (e["projection_seq"], e["entry_id"]) > last]

    def before(self, cursor):
        self._validate(cursor)
        last = (cursor["last_projection_seq"], cursor["last_entry_id"])
        snapshot = self.complete_snapshot(cursor["cutoff_frontier"])
        return [e for e in snapshot if (e["projection_seq"], e["entry_id"]) < last]


def run_cases():
    scope = ("server-a", "tenant-7", "query-comments-v1")
    ledger = FeedLedger(*scope, generation=4)

    # Distinct authorities may have both the same source sequence and differing
    # source epochs. The feed ledger, not source clocks, orders all three.
    first = ledger.append("authority-a", "attribute-a", 3, 1, "object-a-1")
    second = ledger.append("authority-b", "attribute-b", 7, 1, "object-b-1")
    third = ledger.append("authority-c", "attribute-c", 3, 1, "object-c-1")
    beyond_cutoff = ledger.append("authority-b", "attribute-b", 7, 2, "object-b-2")
    cutoffs = {
        ("authority-a", "attribute-a"): (3, 1),
        ("authority-b", "attribute-b"): (7, 1),
        ("authority-c", "attribute-c"): (3, 1),
    }
    complete = ledger.complete_snapshot(cutoffs)
    require(
        [e["entry_id"] for e in complete]
        == [first["entry_id"], second["entry_id"], third["entry_id"]],
        "complete feed dropped or reordered identical-source-sequence entries",
    )
    require(
        [e["projection_seq"] for e in complete] == [1, 2, 3],
        "projection ledger did not allocate a unique ordered sequence",
    )
    require(
        beyond_cutoff["entry_id"] not in {e["entry_id"] for e in complete},
        "entry beyond pinned source frontier leaked into the complete snapshot",
    )

    middle_cursor = ledger.cursor(
        complete, cutoffs, after=(first["projection_seq"], first["entry_id"])
    )
    require(
        [e["entry_id"] for e in ledger.since(middle_cursor)]
        == [second["entry_id"], third["entry_id"]],
        "since cursor omitted an entry from another source",
    )
    latest_cursor = ledger.cursor(
        complete, cutoffs, after=(third["projection_seq"], third["entry_id"])
    )
    require(
        [e["entry_id"] for e in ledger.before(latest_cursor)]
        == [first["entry_id"], second["entry_id"]],
        "before cursor order is not stable by projection sequence/entry ID",
    )

    # Restart restores the durable feed high-water mark; no projection sequence
    # can be reused even though source sequences are in a separate namespace.
    restarted = FeedLedger(
        *scope,
        generation=4,
        durable_high_water=ledger.high_water,
        entries=ledger.entries,
    )
    fourth = restarted.append("authority-a", "attribute-a", 3, 2, "object-a-2")
    require(
        fourth["projection_seq"] == 5,
        "restart reused or skipped durable projection sequence",
    )

    # Policy/access changes create a new projection generation and invalidate
    # old cursors. The feed-wide high-water mark remains monotonic.
    changed = FeedLedger(*scope, generation=5, durable_high_water=restarted.high_water)
    try:
        changed.since(middle_cursor)
    except CursorInvalidated:
        pass
    else:
        raise AssertionError(
            "policy generation change did not invalidate the old cursor"
        )
    fifth = changed.append("authority-b", "attribute-b", 7, 3, "object-b-3")
    require(
        fifth["projection_seq"] == 6,
        "new projection generation reused a prior sequence",
    )
    print("feed_cursor_cases=PASS")
    print(
        "cases=two_source_attributes_same_sequence_and_distinct_epochs,complete_cutoff,since,before,restart_high_water,policy_generation_invalidation"
    )
    print("runtime_claims=NONE; deterministic in-memory model oracle only")


def parse_args(argv):
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--check", action="store_true")
    return parser.parse_args(argv)


def main(argv):
    parse_args(argv)  # Complete CLI is parsed before fixture/model work.
    run_cases()
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
