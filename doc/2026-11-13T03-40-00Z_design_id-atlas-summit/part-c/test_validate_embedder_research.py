#!/usr/bin/env python3
"""Oracle: research-matrix, #10157's closed candidate/provenance invariants.

Run: python3 -I <this-file>. Stdlib only; fixture scores are never real results.
"""
import copy
import hashlib
import importlib.util
import itertools
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('validator', HERE / 'validate_embedder_research.py')
VALIDATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATOR)
NAMES = ['Snowflake arctic-embed-m-v2.0', 'BGE-M3', 'Voyage',
         'OpenAI text-embedding-3', 'Workers AI']


def fixture():
    return {
        'schema': 'atlas-embedder-research/v1', 'issue': 10157, 'slice': 'C9138-01',
        'current_model': 'Snowflake arctic-embed-m-v2.0',
        'owner_decision': {'status': 'undecided', 'decision_ref': None},
        'independent_recall_run': False, 'second_embedder_enabled': False,
        'corpus_sha256': None, 'eval_set_sha256': None, 'quality_artifact': None,
        'sources': [{'id': 'bge', 'url': 'https://huggingface.co/BAAI/bge-m3',
                     'retrieved': '2026-11-27', 'fact': '1024 dimensions'}],
        'candidates': [{'identity': n, 'cost': 'unmeasured', 'self_hostability': 'see source',
                        'mixed_corpus': 'unmeasured', 'sources': ['bge'],
                        'quality_status': 'unmeasured'} for n in NAMES],
    }


class ResearchMatrixTests(unittest.TestCase):
    def check_report(self, report):
        VALIDATOR.validate(report, HERE)

    def test_valid_and_all_candidate_permutations(self):
        # Property oracle: coverage is an unordered set of exactly five identities.
        for names in itertools.permutations(NAMES):
            report = fixture()
            for row, name in zip(report['candidates'], names):
                row['identity'] = name
            self.check_report(report)

    def test_each_missing_duplicate_or_substituted_identity_is_rejected(self):
        for index in range(5):
            for replacement in (None, NAMES[(index + 1) % 5], 'Snowflake Cortex'):
                report = fixture()
                if replacement is None:
                    report['candidates'].pop(index)
                else:
                    report['candidates'][index]['identity'] = replacement
                with self.subTest(index=index, replacement=replacement), self.assertRaises(ValueError):
                    self.check_report(report)

    def test_safety_and_closed_schema(self):
        for key, value in [('current_model', 'Snowflake Cortex'),
                           ('independent_recall_run', True), ('second_embedder_enabled', True),
                           ('recall_at_10', 0.99), ('quality_artifact', {})]:
            report = fixture()
            report[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.check_report(report)
        for key, value in [('quality_status', 'best'), ('recall', 0.99), ('sources', ['missing'])]:
            report = fixture()
            report['candidates'][0][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.check_report(report)

    def test_decision_reference_required_and_bound_to_9092(self):
        report = fixture()
        report['owner_decision'] = {'status': 'decided', 'decision_ref': None}
        with self.assertRaises(ValueError):
            self.check_report(report)
        report['owner_decision']['decision_ref'] = 'https://github.com/developing-today/code/issues/9092#issuecomment-123'
        self.check_report(report)
        for ref in ('https://example.com/9092', 'https://github.com/developing-today/code/issues/90920'):
            report['owner_decision']['decision_ref'] = ref
            with self.assertRaises(ValueError):
                self.check_report(report)

    def test_optional_artifact_bytes_and_both_digests_match(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'owner.json'
            owner = {'owner_issue': 9092, 'corpus_sha256': 'a' * 64, 'eval_set_sha256': 'b' * 64}
            path.write_text(json.dumps(owner))
            report = fixture()
            report.update({k: owner[k] for k in ('corpus_sha256', 'eval_set_sha256')})
            report['quality_artifact'] = {'path': 'owner.json', 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
                                          'owner_ref': 'https://github.com/developing-today/code/issues/9092#issuecomment-123'}
            VALIDATOR.validate(report, Path(directory))
            for key in ('corpus_sha256', 'eval_set_sha256'):
                bad = copy.deepcopy(report)
                bad[key] = 'c' * 64
                with self.subTest(key=key), self.assertRaises(ValueError):
                    VALIDATOR.validate(bad, Path(directory))
            path.write_text('{}')
            with self.assertRaises(ValueError):
                VALIDATOR.validate(report, Path(directory))

    def test_malformed_shapes_are_controlled_errors(self):
        for report in (None, [], {}, {'candidates': None}):
            with self.subTest(report=report), self.assertRaises(ValueError):
                self.check_report(report)

    def test_cli_rejects_before_reading(self):
        for args in ([], ['--input'], ['--input', '/nonexistent', '--typo'],
                     ['--inp', '/nonexistent'], ['stray'],
                     ['--input', '/nonexistent', '--input', '/other']):
            result = subprocess.run([sys.executable, '-I', str(HERE / 'validate_embedder_research.py'), *args],
                                    capture_output=True, text=True)
            with self.subTest(args=args):
                self.assertEqual(result.returncode, 2)
                self.assertNotIn('No such file', result.stderr)


if __name__ == '__main__':
    unittest.main()
