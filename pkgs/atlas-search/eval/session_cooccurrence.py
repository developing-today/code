"""Offline single-subject item candidates and temporal held-out evaluation."""

import argparse
from collections import Counter, defaultdict
import json
from math import log2


def reference(value):
    if not isinstance(value, str) or not value.strip():
        raise ValueError("expected a nonempty opaque reference")
    return value


def reference_set(values):
    if not isinstance(values, list):
        raise ValueError("expected a reference list")
    return {reference(value) for value in values}


def partitions(data):
    if data["schema"] != "atlas-recommendation-eval/v1":
        raise ValueError("unsupported evaluation schema")
    subject = reference(data["subject"])
    allowed_sessions = reference_set(data["eligible_sessions"])
    allowed_items = reference_set(data["eligible_items"])
    cutoff = data["cutoff"]
    if type(cutoff) is not int or not isinstance(data["sessions"], list):
        raise ValueError("expected integer cutoff and sessions array")

    train, held_out = [], []
    seen = set()
    for session in data["sessions"]:
        if session["subject"] != subject or session["id"] not in allowed_sessions:
            continue
        if session["id"] in seen:
            raise ValueError("duplicate scoped session")
        seen.add(session["id"])
        start, end = session["start"], session["end"]
        if type(start) is not int or type(end) is not int or start > end:
            raise ValueError("invalid session interval")
        if start <= cutoff < end:
            continue
        if not isinstance(session["opens"], list):
            raise ValueError("expected ordered opens")
        opens = dict.fromkeys(
            item for item in session["opens"] if reference(item) in allowed_items
        )
        if end <= cutoff:
            train.append(tuple(opens))
        else:
            held_out.append(tuple(opens))
    return train, held_out


def index_sessions(sessions):
    index = defaultdict(list)
    for session in sessions:
        items = frozenset(session)
        for item in items:
            index[item].append(items)
    return index


def candidates(index, seeds, k):
    """One vote per training session and distinct seed/candidate pair."""
    scores = Counter()
    seen = set(seeds)
    for seed in seen:
        for session in index.get(seed, ()):
            scores.update(session - seen)
    return sorted(scores, key=lambda item: (-scores[item], item))[:k]


def evaluate(data, k):
    if type(k) is not int or k <= 0:
        raise ValueError("k must be a positive integer")
    train, held_out = partitions(data)
    index = index_sessions(train)
    scored = hits = relevant = 0
    reciprocal_ranks = ndcg = 0.0
    for session in held_out:
        if len(session) < 2:
            continue
        half = len(session) // 2
        targets = set(session[half:])
        ranking = candidates(index, session[:half], k)
        ranks = [i for i, item in enumerate(ranking, 1) if item in targets]
        scored += 1
        hits += len(ranks)
        relevant += len(targets)
        if ranks:
            reciprocal_ranks += 1 / ranks[0]
            dcg = sum(1 / log2(rank + 1) for rank in ranks)
            ideal = sum(
                1 / log2(rank + 1) for rank in range(1, min(k, len(targets)) + 1)
            )
            ndcg += dcg / ideal
    return {
        "k": k,
        "training_sessions": len(train),
        "held_out_sessions": len(held_out),
        "scored_sessions": scored,
        "hits": hits,
        "held_out_items": relevant,
        "precision_at_k": hits / (scored * k) if scored else None,
        "recall_at_k": hits / relevant if relevant else None,
        "mrr_at_k": reciprocal_ranks / scored if scored else None,
        "ndcg_at_k": ndcg / scored if scored else None,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture", type=argparse.FileType("r", encoding="utf-8"))
    parser.add_argument("-k", type=int, default=10)
    args = parser.parse_args()
    try:
        with args.fixture as stream:
            result = evaluate(json.load(stream), args.k)
    except (KeyError, ValueError, TypeError) as error:
        parser.exit(2, f"invalid evaluation input ({type(error).__name__})\n")
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
