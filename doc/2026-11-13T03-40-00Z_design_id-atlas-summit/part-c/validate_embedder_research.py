#!/usr/bin/env python3
"""research-matrix oracle for C9138-01.

CLI contract: only --input PATH (or --input=PATH), required exactly once;
-h/--help prints usage. Parse strictly before file I/O. Never run inference.
"""
import argparse
from datetime import date
import hashlib
import json
import re
from urllib.parse import urlparse
from pathlib import Path
import sys


IDENTITIES = {'Snowflake arctic-embed-m-v2.0', 'BGE-M3', 'Voyage',
              'OpenAI text-embedding-3', 'Workers AI'}
OWNER_REF = r'https://github\.com/developing-today/code/issues/9092(?:#issuecomment-[0-9]+)?'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def shape(value, keys, label):
    require(isinstance(value, dict) and set(value) == set(keys.split()),
            f'{label}: expected fields {keys}')


def text(value):
    return isinstance(value, str) and bool(value.strip())


def digest(value):
    return isinstance(value, str) and re.fullmatch(r'[0-9a-f]{64}', value) is not None


def owner_ref(value):
    return isinstance(value, str) and re.fullmatch(OWNER_REF, value) is not None


def validate(report, directory):
    """research-matrix structural oracle, not authentication or quality evaluation.

    Optional owner artifact: local JSON, owner_issue=9092, corpus_sha256 and
    eval_set_sha256. Other artifact fields are opaque; never recompute metrics.
    The owner_ref must be reviewed by a person/agent before trusting provenance.
    """
    shape(report, 'schema issue slice current_model owner_decision independent_recall_run '
          'second_embedder_enabled corpus_sha256 eval_set_sha256 quality_artifact sources candidates', 'report')
    require(report['schema'] == 'atlas-embedder-research/v1'
            and type(report['issue']) is int and report['issue'] == 10157
            and report['slice'] == 'C9138-01', 'wrong research identity')
    require(report['current_model'] == 'Snowflake arctic-embed-m-v2.0', 'current model substitution')
    require(report['independent_recall_run'] is False and report['second_embedder_enabled'] is False,
            'research must not run recall or enable a second embedder')
    decision = report['owner_decision']
    shape(decision, 'status decision_ref', 'owner decision')
    require((decision['status'] == 'undecided' and decision['decision_ref'] is None)
            or (decision['status'] == 'decided' and owner_ref(decision['decision_ref'])),
            'decision must be undecided or cite #9092')
    sources = report['sources']
    require(isinstance(sources, list) and bool(sources), 'sources must be nonempty')
    source_ids = set()
    for source in sources:
        shape(source, 'id url retrieved fact', 'source')
        require(all(text(source[k]) for k in source), 'source fields must be nonempty strings')
        require(source['id'] not in source_ids, 'duplicate source id')
        source_ids.add(source['id'])
        url = urlparse(source['url'])
        require(url.scheme == 'https' and url.netloc in {
            'huggingface.co', 'docs.voyageai.com', 'developers.openai.com',
            'developers.cloudflare.com'}, 'source must cite a primary vendor URL')
        date.fromisoformat(source['retrieved'])
    candidates = report['candidates']
    require(isinstance(candidates, list) and len(candidates) == 5, 'need exactly five candidates')
    identities = []
    for candidate in candidates:
        shape(candidate, 'identity cost self_hostability mixed_corpus sources quality_status', 'candidate')
        require(text(candidate['identity']), 'candidate identity must be text')
        identities.append(candidate['identity'])
        require(all(text(candidate[k]) for k in ('cost', 'self_hostability', 'mixed_corpus')),
                'cost, self-hostability and mixed-corpus discussion required')
        require(candidate['quality_status'] == 'unmeasured', 'no independent quality results')
        refs = candidate['sources']
        require(isinstance(refs, list) and bool(refs) and all(
            isinstance(ref, str) and ref in source_ids for ref in refs), 'candidate source missing')
    require(set(identities) == IDENTITIES, 'missing, duplicate or substituted candidate')
    artifact = report['quality_artifact']
    if artifact is None:
        require(report['corpus_sha256'] is None and report['eval_set_sha256'] is None,
                'absent artifact must not invent corpus/eval digests')
        return
    shape(artifact, 'path sha256 owner_ref', 'quality artifact')
    require(text(artifact['path']) and digest(artifact['sha256']) and owner_ref(artifact['owner_ref']),
            'artifact requires local path, byte digest and #9092 provenance reference')
    for key in ('corpus_sha256', 'eval_set_sha256'):
        require(digest(report[key]), f'{key} must be lowercase SHA-256')
    relative = Path(artifact['path'])
    require(not relative.is_absolute() and '..' not in relative.parts, 'artifact must be relative')
    path = (directory / relative).resolve()
    require(path.is_relative_to(directory.resolve()), 'artifact must stay within report directory')
    data = path.read_bytes()
    require(hashlib.sha256(data).hexdigest() == artifact['sha256'], 'artifact byte digest mismatch')
    metadata = json.loads(data)
    require(isinstance(metadata, dict) and type(metadata.get('owner_issue')) is int
            and metadata.get('owner_issue') == 9092, 'wrong artifact owner')
    for key in ('corpus_sha256', 'eval_set_sha256'):
        require(metadata.get(key) == report[key], f'artifact {key} mismatch')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument('--input', required=True, action='append')
    args = parser.parse_args(argv)
    if len(args.input) != 1:
        parser.error('--input must occur exactly once')
    try:
        path = Path(args.input[0])
        validate(json.loads(path.read_text()), path.resolve().parent)
    except (OSError, ValueError, TypeError, KeyError) as error:
        print(f'research-matrix: FAIL: {error}', file=sys.stderr)
        return 1
    print('research-matrix: PASS (structural evidence only; no quality winner)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
