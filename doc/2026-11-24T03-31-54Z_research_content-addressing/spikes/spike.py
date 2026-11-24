#!/usr/bin/env python3
"""
Content-addressing strategy spike (CONTENT-ADDRESSING-RESEARCH lane, 2026-11-23).

Models four addressing strategies over one representative corpus and measures
byte accounting + hash-operation counts. The hash used for labels is SHA-256
(stdlib); every reported quantity (bytes on the wire, dedup counts, proof
sizes, op counts) is independent of which strong hash is used, because the
strategies differ in *what is hashed and how identity is assigned*, not in
digest size (all contenders use 32-byte digests/labels).

Strategies
----------
bao_whole   : one blob, fixed 1024-byte chunks, BLAKE3/Bao semantics.
              Chunk identity is POSITIONAL: chunk CV depends on its index,
              so identical bytes at different positions are different pieces.
              Sync model = digest-set resync (request pieces you lack, by
              identity). Across versions nothing is reusable, because the
              piece identity includes the version's root + index.
bab_whole   : one blob, fixed 1024-byte chunks, Bab/WILLIAM3 semantics.
              Chunk label = H(chunk bytes) only (no index); inner label =
              H(left, right, subtree_length). Identical chunks/subtrees are
              the same piece anywhere, in any blob.
per_item    : the blob is a sequence of items; each item is its own
              addressed unit (digest + length); a manifest (ordered list of
              40-byte entries: 32-byte digest + 8-byte length) names them.
cdc_whole   : one blob, content-defined chunking (FastCDC-style gear hash,
              simplified single-mask variant), chunk identity = H(chunk).

Scenarios
---------
A  fetch one ~3 KiB item from the middle of the collection, verified.
B  edit-and-republish: insert 200 B early, modify one item, delete one
   item, append new items. A replica holding v1 resyncs to v2 by
   requesting exactly the pieces it lacks (digest-set resync).
C  transclusion fetch: a fresh client fetches a small page plus the 12
   items it embeds, verified, from the collection.
D  length proof: bytes needed to prove the blob's total length.

Tree slice accounting for bao/bab implements the actual Bao traversal:
a slice includes the 8-byte length header, every inner node on the
traversal to the requested chunks (emitted as a 64-byte pair of labels),
and the requested chunks themselves. For the bao-tree (iroh) variant the
leaves are 16 KiB chunk groups and whole groups are transferred.
"""

import hashlib
import json
import random
import sys

CHUNK = 1024
GROUP = 16 * CHUNK  # bao-tree / iroh-blobs chunk group
LABEL = 32

rng = random.Random(20261123)

# ---------------------------------------------------------------- corpus --

WORDS = (
    "graph object item payload digest author subspace entry path value "
    "sync replica capability grant revoke embed transclude snapshot live "
    "reference collection forum note thread media photo render host "
    "contract verb edge node link root state version replace pointer "
    "signed name manifest chunk slice proof verify stream store"
).split()

BOILER = (
    b"%% shared boilerplate block: this header text is identical in every "
    b"item that carries it, modelling a common schema preamble. fields: "
    b"id, author, created, parents, type. %%\n"
)


def make_item(i: int) -> bytes:
    lines = [f"item {i}: title {' '.join(rng.choices(WORDS, k=6))}"]
    if i % 3 == 0:
        lines.append(BOILER.decode())
    n = rng.randint(18, 46)
    for _ in range(n):
        lines.append(" ".join(rng.choices(WORDS, k=rng.randint(6, 14))))
    return ("\n".join(lines) + "\n").encode()


ATTACH_AT = {120, 260, 388}


def build_blob(items, instances, attach, pad_to_chunk=False):
    parts, spans, pos = [], [], 0
    for k, idx in enumerate(instances):
        if k in ATTACH_AT:
            parts.append(attach)
            spans.append((pos, pos + len(attach), "attachment"))
            pos += len(attach)
            if pad_to_chunk:
                pad = (-pos) % CHUNK
                parts.append(b"\x00" * pad)
                pos += pad
        data = b"%%ITEM%%\n" + items[idx]
        parts.append(data)
        spans.append((pos, pos + len(data), idx))
        pos += len(data)
        if pad_to_chunk:
            pad = (-pos) % CHUNK
            parts.append(b"\x00" * pad)
            pos += pad
    return b"".join(parts), spans


def build_collection():
    items = [make_item(i) for i in range(340)]
    instances = []  # item indices in order; ~15% are repeats (transclusions)
    for k in range(400):
        if k % 7 == 3 and instances:
            instances.append(rng.choice(instances[: len(instances)]))
        else:
            instances.append(k % 340)
    attach = bytes(rng.randbytes(40960))  # opaque 40 KiB attachment
    blob, spans = build_blob(items, instances, attach)
    return blob, spans, items, instances, attach


BLOB, SPANS, ITEMS, INSTANCES, ATTACH = build_collection()

# ---------------------------------------------------------------- helpers --


def H(b: bytes) -> bytes:
    return hashlib.sha256(b).digest()


def fixed_chunks(data: bytes, size: int):
    return [data[i : i + size] for i in range(0, len(data), size)] or [b""]


class Tree:
    """Binary (left-complete, BLAKE3-shaped) tree over leaf labels."""

    def __init__(self, leaf_labels):
        self.levels = [leaf_labels]
        while len(self.levels[-1]) > 1:
            cur = self.levels[-1]
            nxt = []
            for i in range(0, len(cur) - 1, 2):
                nxt.append(H(cur[i] + cur[i + 1]))
            if len(cur) % 2 == 1:
                nxt.append(cur[-1])
            self.levels.append(nxt)
        self.n_leaves = len(leaf_labels)

    def slice_cost(self, first_leaf: int, last_leaf: int):
        """(proof_bytes, parent_hash_ops) for a Bao-style slice covering
        leaves [first_leaf, last_leaf]: 8-byte header + 64 B per inner node
        the traversal must reveal + we count one hash op per revealed node
        plus one per leaf on the path recomputation (approximated by the
        number of revealed nodes + depth)."""
        # Standard result: the set of maximal subtrees covering the range;
        # proof = sibling subtrees adjacent to that cover. Compute exactly
        # by walking levels.
        proof_nodes = 0
        cover = {(0, i) for i in range(first_leaf, last_leaf + 1)}
        # lift cover to maximal subtrees
        changed = True
        nodes = set(cover)
        while changed:
            changed = False
            for lvl, i in list(nodes):
                if i % 2 == 0 and (lvl, i + 1) in nodes:
                    nodes.discard((lvl, i))
                    nodes.discard((lvl, i + 1))
                    nodes.add((lvl + 1, i // 2))
                    changed = True
        # proof = siblings of cover nodes up to the root
        proof = set()
        for lvl, i in nodes:
            l, j = lvl, i
            while l < len(self.levels) - 1:
                sib = j ^ 1
                if sib < len(self.levels[l]):
                    proof.add((l, sib))
                l += 1
                j //= 2
        proof_nodes = len(proof)
        return 8 + 64 * proof_nodes, proof_nodes


def bao_slice(data_len, leaf_size, start, end):
    """Wire bytes (data + proof) to fetch byte range [start, end) verified,
    from a blob of data_len addressed with leaves of leaf_size."""
    n = max(1, -(-data_len // leaf_size))
    first = start // leaf_size
    last = min(n - 1, (end - 1) // leaf_size)
    leaves = [b"\x00" * LABEL] * n  # labels are placeholders; shape only
    t = Tree(leaves)
    proof, ops = t.slice_cost(first, last)
    data_bytes = min(data_len, (last + 1) * leaf_size) - first * leaf_size
    return data_bytes + proof, proof, ops


GEAR = [rng.randrange(1 << 64) for _ in range(256)]


def cdc_chunks(data: bytes, min_s=4096, avg=16384, max_s=65536):
    mask = avg - 1  # avg is a power of two in this spike
    out, start, fp = [], 0, 0
    for i, byte in enumerate(data):
        fp = ((fp << 1) + GEAR[byte]) & 0xFFFFFFFFFFFFFFFF
        ln = i - start + 1
        if ln >= max_s or (ln >= min_s and (fp & mask) == 0):
            out.append(data[start : i + 1])
            start = i + 1
            fp = 0
    if start < len(data):
        out.append(data[start:])
    return out


# -------------------------------------------------------------- strategies --

results = {
    "corpus": {
        "blob_bytes": len(BLOB),
        "instances": len(SPANS),
        "distinct_items": len(ITEMS),
    }
}

bao_chunks = fixed_chunks(BLOB, CHUNK)
bab_labels = [H(c) for c in bao_chunks]
results["within_blob_dedup"] = {
    "chunks_total": len(bao_chunks),
    "bab_distinct_chunk_labels": len(set(bab_labels)),
    "bab_duplicate_chunks": len(bao_chunks) - len(set(bab_labels)),
    "bao_duplicate_chunks": 0,  # positional CVs: distinct by construction
}

# storage overhead of the tree itself (outboard / label store), 1 blob
n = len(bao_chunks)
results["tree_storage_bytes"] = {
    "bao_vanilla_outboard": 64 * (n - 1) + 8,
    "bao_tree_16k_groups_outboard": 64 * (max(1, -(-len(BLOB) // GROUP)) - 1) + 8,
    "bab_label_store_all_nodes": LABEL * (2 * n - 1),
}

# ------------------------------------------------------------- scenario A --
target = next(s for s in SPANS if isinstance(s[2], int) and 300_000 < s[0] < 600_000)
ts, te = target[0], target[1]
resA = {}
resA["bao_whole"], _, _ = bao_slice(len(BLOB), CHUNK, ts, te)
resA["bao_tree_groups"], _, _ = bao_slice(len(BLOB), GROUP, ts, te)
resA["bab_whole"], _, _ = bao_slice(len(BLOB), CHUNK, ts, te)
# per_item: item bytes + manifest slice (manifest = 40 B * instances)
man_len = 40 * len(SPANS)
entry_off = 40 * SPANS.index(target)
m_slice, _, _ = bao_slice(man_len, CHUNK, entry_off, entry_off + 40)
resA["per_item"] = (te - ts) + m_slice
cdc = cdc_chunks(BLOB)
off, csum = 0, 0
for c in cdc:
    if off < te and off + len(c) > ts:
        csum += len(c)
    off += len(c)
resA["cdc_whole"] = csum + 40 * 8  # + a few manifest entries, stated approx
resA["item_bytes"] = te - ts
results["scenario_A_fetch_one_item"] = resA

# aligned-repeat dedup: same collection, but every unit padded to a chunk
# multiple before concatenation (a real store design choice). Bab chunk
# labels are position-free, so aligned repeats dedup; unaligned ones (the
# main corpus) almost never share a chunk.
PADBLOB, _ = build_blob(ITEMS, INSTANCES, ATTACH, pad_to_chunk=True)
pad_labels = [H(c) for c in fixed_chunks(PADBLOB, CHUNK)]
inst_bytes = [len(b"%%ITEM%%\n" + ITEMS[i]) for i in INSTANCES]
results["aligned_repeat_dedup"] = {
    "padded_blob_bytes": len(PADBLOB),
    "chunks_total": len(pad_labels),
    "bab_duplicate_chunks_when_aligned": len(pad_labels) - len(set(pad_labels)),
    "duplicate_item_instances": len(INSTANCES) - len(set(INSTANCES)),
    "per_item_duplicate_bytes": sum(inst_bytes)
    - sum(len(b"%%ITEM%%\n" + ITEMS[i]) for i in set(INSTANCES))
    + 3 * len(ATTACH)
    - len(ATTACH),  # attachment stored once, not 3x
}

# ------------------------------------------------------------- scenario B --
# v2, built with the SAME builder (same attachments, same framing):
# insert ~200 B into instance 50, modify the item instanced at 200 (all
# its instances change), delete instance 300, append 5 new items.
inst2 = list(INSTANCES)
items2 = list(ITEMS)
items2[inst2[200]] = items2[inst2[200]].replace(b"graph", b"chart", 40)
del inst2[300]
items2 = items2 + [make_item(10_000 + i) for i in range(5)]
inst2 = inst2 + [340 + i for i in range(5)]
base2, spans2 = build_blob(items2, inst2, ATTACH)
a50 = spans2[50][0]  # no attachment precedes instance 50 in the span list
ins = b"INSERTED " * 22  # 198 bytes inserted 300 B into instance 50
BLOB2 = base2[: a50 + 300] + ins + base2[a50 + 300 :]
resB = {"v2_bytes": len(BLOB2)}
# bao: positional identity -> nothing reusable across versions
resB["bao_whole"] = len(BLOB2)
# bab: chunks of v2 whose label not in v1 label set
v1_labels = set(bab_labels)
b2 = fixed_chunks(BLOB2, CHUNK)
resB["bab_whole"] = sum(len(c) for c in b2 if H(c) not in v1_labels)
# per_item: changed/new items (by digest, each sent once), + manifest diff
v1_item_digests = {H(b"%%ITEM%%\n" + ITEMS[i]) for i in INSTANCES}
send, seen = 0, set()
for idx in inst2:
    d = H(b"%%ITEM%%\n" + items2[idx])
    if d not in v1_item_digests and d not in seen:
        send += len(b"%%ITEM%%\n" + items2[idx])
        seen.add(d)
resB["per_item"] = send + 1024  # + ~one manifest chunk, tree-diffed
# cdc: chunks of v2 whose digest not in v1 chunk set
v1_cdc = {H(c) for c in cdc}
resB["cdc_whole"] = sum(len(c) for c in cdc_chunks(BLOB2) if H(c) not in v1_cdc)
results["scenario_B_edit_republish"] = resB

# Scenario B2: same-length in-place edit only (one word swapped inside one
# item, no insertion/deletion): alignment survives everywhere else.
items3 = list(ITEMS)
items3[inst2[200]] = items3[inst2[200]].replace(b"graph", b"chart", 40)
BLOB3, _ = build_blob(items3, INSTANCES, ATTACH)
b3 = fixed_chunks(BLOB3, CHUNK)
results["scenario_B2_inplace_edit"] = {
    "v_bytes": len(BLOB3),
    "bao_whole": len(BLOB3),  # positional identity: new root, no reuse
    "bab_whole": sum(len(c) for c in b3 if H(c) not in v1_labels),
    "cdc_whole": sum(len(c) for c in cdc_chunks(BLOB3) if H(c) not in v1_cdc),
}

# ------------------------------------------------------------- scenario C --
embedded = []
seen_idx = set()
for s in SPANS:
    if isinstance(s[2], int) and s[2] not in seen_idx:
        embedded.append(s)
        seen_idx.add(s[2])
    if len(embedded) == 12:
        break
resC = {"items": len(embedded), "item_bytes_total": sum(e - s for s, e, _ in embedded)}
tot = 2048  # the page itself
tb = 0
for s, e, _ in embedded:
    w, _, _ = bao_slice(len(BLOB), CHUNK, s, e)
    tb += w
resC["bao_whole"] = tot + tb
tg = 0
for s, e, _ in embedded:
    w, _, _ = bao_slice(len(BLOB), GROUP, s, e)
    tg += w
resC["bao_tree_groups"] = tot + tg
resC["bab_whole"] = tot + tb  # same traversal shape as bao at 1 KiB leaves
pi = tot
for j, (s, e, _) in enumerate(embedded):
    eo = 40 * SPANS.index((s, e, _))
    ms, _, _ = bao_slice(man_len, CHUNK, eo, eo + 40)
    pi += (e - s) + ms
resC["per_item"] = pi
results["scenario_C_transclusion_fetch"] = resC

# ------------------------------------------------------------- scenario D --
depth = (n - 1).bit_length()
results["scenario_D_length_proof_bytes"] = {
    "bao": 8 + 64 * depth + CHUNK,  # header + right-edge parents + final chunk
    "bab": 72,  # two child labels + length; constant
    "note": "bao length is only validated once the final chunk is "
    "validated (Bao spec, final chunk requirement); bab root "
    "labels commit to total length via hash_inner length input",
}

# ------------------------------------------- analytic proof-size table ----
table = {}
for size in (1 << 20, 100 << 20, 1 << 30):
    ch = -(-size // CHUNK)
    gr = -(-size // GROUP)
    table[f"{size}"] = {
        "bao_outboard_bytes": 64 * (ch - 1) + 8,
        "bao_tree_outboard_bytes": 64 * (gr - 1) + 8,
        "bao_slice_proof_1chunk_bytes": 8 + 64 * (ch - 1).bit_length(),
        "bao_tree_min_transfer_bytes": GROUP,
        "bab_slice_proof_1chunk_bytes": 8 + 64 * (ch - 1).bit_length(),
        "bab_length_proof_bytes": 72,
        "bao_length_proof_bytes": 8 + 64 * (ch - 1).bit_length() + CHUNK,
    }
results["analytic_table"] = table

print(json.dumps(results, indent=1))
