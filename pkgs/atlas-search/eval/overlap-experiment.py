# lane-scratch: code-y /tmp/lane-y-20261108-0204 task 12883
#!/usr/bin/env python3
"""Atlas overlap budget experiment (issue #6039).

Chunks a representative corpus slice at 0% / 10% / 20% overlap per source
type (prose docs, logs), and measures golden-set recall vs index size.

Signal: BM25 (lexical), implemented here in stdlib Python so the script
runs anywhere with no model download and no network. The atlas eval
harness (eval/README.md, #5741) scores saved rankings offline; the runs
this script emits per configuration are plain ranked chunk-id lists, so
the same qrels can be re-scored under a dense signal when the #6027
golden set and an embedding route are available — that re-run is owed
and named in the report.

Golden set: PROVISIONAL, constructed by this script from the corpus
slice itself (see build_golden). It is not the #6027 golden set.
Queries are the top-5 highest-IDF content words of a sampled passage;
the gold relevant unit is the passage's word span in its document, and
a chunk is relevant iff its word span intersects the gold span in the
same document. Two query classes per type: "interior" (whole sampled
passages) and "boundary" (50-word spans centred on 0%-overlap window
boundaries — the case overlap exists for). Boundary scoring also
reports recall with a >=80% span-coverage requirement and the mean
best coverage in the top 10, because any-overlap relevance alone
cannot discriminate overlap settings. This construction is neutral across overlap settings
(the same queries/qrels are scored against every configuration), but
it favours lexical retrieval; treat absolute recall as an upper bound
for BM25-style signals, not as a production estimate.

Corpus slice (deterministic, from this repository at the pinned HEAD):
- prose: Markdown files under doc/ and pkgs/atlas-search/, >= 2000
  bytes each, sorted by path, capped at 240 KB total.
- logs: `git log` entries (subject + body) of this repository, newest
  1500 commits, grouped into documents of 60 consecutive entries.

Chunking: fixed 200-word windows per document, stride =
round(200 * (1 - overlap)). Overlap therefore duplicates boundary
content exactly as a production fixed-window chunker would.
"""
import json, math, re, subprocess, sys
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.cwd()
OUT = Path(sys.argv[2]) if len(sys.argv) > 2 else ROOT / "overlap-results.json"
WINDOW = 200
OVERLAPS = [0.0, 0.10, 0.20]
SEED_QUERIES_PER_TYPE = 16
WORD = re.compile(r"[a-z0-9][a-z0-9\-']+")
STOP = set("the a an and or of to in is are was were for on with as by at from that this it its be has have had not but we you they he she them then than so such can could should would may might do does did done".split())

def words(text):
    return [w for w in WORD.findall(text.lower())]

def load_prose(root):
    docs, total = [], 0
    cands = []
    for base in [root / "doc", root / "pkgs" / "atlas-search"]:
        for p in sorted(base.rglob("*.md")):
            if "target" in p.parts:
                continue
            cands.append(p)
    for p in cands:
        try:
            t = p.read_text(errors="ignore")
        except OSError:
            continue
        if len(t) < 2000:
            continue
        docs.append({"id": f"prose:{p.relative_to(root)}", "type": "prose", "text": t})
        total += len(t)
        if total >= 240_000:
            break
    return docs

def load_logs(root):
    out = subprocess.run(
        ["git", "-C", str(root), "log", "-n", "1500", "--format=%H%x1f%s%x1f%b%x1e"],
        capture_output=True, text=True, check=True).stdout
    entries = [e.strip() for e in out.split("\x1e") if e.strip()]
    docs = []
    for i in range(0, len(entries), 60):
        docs.append({"id": f"logs:gitlog:{i//60}", "type": "logs",
                     "text": "\n\x1e\n".join(entries[i:i+60])})
    return docs

def chunk_doc(doc, overlap):
    ws = words(doc["text"])
    stride = max(1, round(WINDOW * (1 - overlap)))
    chunks = []
    for start in range(0, max(1, len(ws)), stride):
        end = min(len(ws), start + WINDOW)
        if end - start < 20 and chunks:
            break
        chunks.append({"id": f"{doc['id']}@{start}", "doc": doc["id"],
                       "start": start, "end": end, "tokens": ws[start:end]})
        if end >= len(ws):
            break
    return chunks

def bm25_rank(query_terms, chunks, df, n_docs, avgdl):
    k1, b = 1.5, 0.75
    scores = []
    for ch in chunks:
        tf = Counter(ch["tokens"])
        dl = len(ch["tokens"])
        s = 0.0
        for t in query_terms:
            if t not in tf:
                continue
            idf = math.log(1 + (n_docs - df[t] + 0.5) / (df[t] + 0.5))
            s += idf * (tf[t] * (k1 + 1)) / (tf[t] + k1 * (1 - b + b * dl / avgdl))
        if s > 0:
            scores.append((s, ch["id"]))
    scores.sort(key=lambda x: (-x[0], x[1]))
    return [cid for _, cid in scores]

def passages(doc):
    """Candidate gold passages: prose paragraphs / log entries, as word spans."""
    text = doc["text"]
    if doc["type"] == "prose":
        parts = re.split(r"\n\s*\n", text)
    else:
        parts = text.split("\x1e") if "\x1e" in text else text.split("\n\x1f")
        parts = [p for p in parts if p.strip()]
    spans, cursor = [], 0
    full = words(text)
    # locate each part's word span by sequential search on word stream
    pos = 0
    for part in parts:
        pw = words(part)
        if len(pw) < (30 if doc["type"] == "prose" else 12):
            continue
        # find pw[:6] starting at or after pos
        found = -1
        head = pw[:6]
        for i in range(pos, max(pos, len(full) - len(head) + 1)):
            if full[i:i+len(head)] == head:
                found = i
                break
        if found < 0:
            continue
        spans.append((found, found + len(pw), pw))
        pos = found + len(pw)
    return spans

def build_golden(docs):
    # corpus IDF over documents
    df = Counter()
    for d in docs:
        for w in set(words(d["text"])):
            df[w] += 1
    n = len(docs)
    def query_for(pw):
        cand = [w for w in set(pw) if w not in STOP and len(w) >= 5]
        cand.sort(key=lambda w: (-(math.log(1 + (n - df[w] + 0.5) / (df[w] + 0.5))), w))
        return cand[:5]
    golden = []
    # interior class: sampled whole passages (paragraphs / log entries)
    all_spans = []
    for d in docs:
        for (s, e, pw) in passages(d):
            all_spans.append((d, s, e, pw))
    all_spans.sort(key=lambda x: (x[0]["type"], x[0]["id"], x[1]))
    per_type = defaultdict(list)
    for item in all_spans:
        per_type[item[0]["type"]].append(item)
    for typ, items in sorted(per_type.items()):
        if not items:
            continue
        step = max(1, len(items) // SEED_QUERIES_PER_TYPE)
        for j, (d, s, e, pw) in enumerate(items[::step][:SEED_QUERIES_PER_TYPE]):
            q = query_for(pw)
            if len(q) >= 3:
                golden.append({"id": f"{typ}-interior-q{j}", "type": typ,
                               "class": "interior", "doc": d["id"],
                               "start": s, "end": e, "query_terms": q})
    # boundary class: 50-word spans centred on 0%-overlap window boundaries
    # (multiples of WINDOW within each document). At 0% overlap no chunk
    # contains the whole span; higher overlap shifts windows so some
    # chunk does. This is the case overlap exists for.
    for typ in ["prose", "logs"]:
        tdocs = [d for d in docs if d["type"] == typ]
        spans = []
        for d in tdocs:
            ws = words(d["text"])
            for b in range(WINDOW, len(ws) - 25, WINDOW):
                spans.append((d, b - 25, b + 25, ws[b - 25:b + 25]))
        if not spans:
            continue
        step = max(1, len(spans) // SEED_QUERIES_PER_TYPE)
        for j, (d, s, e, pw) in enumerate(spans[::step][:SEED_QUERIES_PER_TYPE]):
            q = query_for(pw)
            if len(q) >= 3:
                golden.append({"id": f"{typ}-boundary-q{j}", "type": typ,
                               "class": "boundary", "doc": d["id"],
                               "start": s, "end": e, "query_terms": q})
    return golden

def main():
    docs = load_prose(ROOT) + load_logs(ROOT)
    golden = build_golden(docs)
    results = {"window_words": WINDOW, "overlaps": OVERLAPS,
               "corpus": {}, "golden_queries": len(golden), "configs": []}
    for typ in ["prose", "logs"]:
        tdocs = [d for d in docs if d["type"] == typ]
        results["corpus"][typ] = {"documents": len(tdocs),
                                  "words": sum(len(words(d["text"])) for d in tdocs),
                                  "bytes": sum(len(d["text"]) for d in tdocs)}
    for overlap in OVERLAPS:
        cfg = {"overlap": overlap, "per_type": {}}
        for typ in ["prose", "logs"]:
            tdocs = [d for d in docs if d["type"] == typ]
            chunks = []
            for d in tdocs:
                chunks.extend(chunk_doc(d, overlap))
            df = Counter()
            for ch in chunks:
                for w in set(ch["tokens"]):
                    df[w] += 1
            avgdl = sum(len(c["tokens"]) for c in chunks) / max(1, len(chunks))
            by_id = {c["id"]: c for c in chunks}
            qs = [g for g in golden if g["type"] == typ]
            per_class = {}
            for cls in ["interior", "boundary"]:
                cqs = [g for g in qs if g.get("class") == cls]
                hit10 = strict10 = 0
                mrr = 0.0
                covs = []
                misses = 0
                for g in cqs:
                    ranking = bm25_rank(g["query_terms"], chunks, df, len(chunks), avgdl)
                    span_len = g["end"] - g["start"]
                    best_cov, best_rank, strict_rank = 0.0, None, None
                    for i, cid in enumerate(ranking[:10]):
                        ch = by_id[cid]
                        if ch["doc"] != g["doc"]:
                            continue
                        inter = max(0, min(ch["end"], g["end"]) - max(ch["start"], g["start"]))
                        cov = inter / span_len if span_len else 0.0
                        if cov > 0 and best_rank is None:
                            best_rank = i + 1
                        if cov > best_cov:
                            best_cov = cov
                        if cov >= 0.8 and strict_rank is None:
                            strict_rank = i + 1
                    covs.append(best_cov)
                    if best_rank is not None:
                        hit10 += 1
                        mrr += 1.0 / best_rank
                    else:
                        misses += 1
                    if strict_rank is not None:
                        strict10 += 1
                per_class[cls] = {
                    "queries": len(cqs),
                    "recall_at_10_any_overlap": round(hit10 / len(cqs), 4) if cqs else None,
                    "recall_at_10_coverage80": round(strict10 / len(cqs), 4) if cqs else None,
                    "mrr_at_10": round(mrr / len(cqs), 4) if cqs else None,
                    "mean_best_coverage_top10": round(sum(covs) / len(covs), 4) if covs else None,
                    "misses": misses,
                }
            total_words = sum(len(c["tokens"]) for c in chunks)
            cfg["per_type"][typ] = {
                "chunks": len(chunks),
                "stored_bytes": sum(len(" ".join(c["tokens"])) for c in chunks),
                "embed_words": total_words,
                "embed_tokens_est": round(total_words * 1.33),
                "queries": len(qs),
                "per_class": per_class,
            }
        results["configs"].append(cfg)
    results["golden"] = golden
    OUT.write_text(json.dumps(results, indent=1, sort_keys=True))
    print(json.dumps({k: v for k, v in results.items() if k != "golden"}, indent=1))

if __name__ == "__main__":
    main()
