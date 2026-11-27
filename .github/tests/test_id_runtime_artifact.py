#!/usr/bin/env python3
"""Oracle: packaging protocol model, fake Nix tools, and actual checksum bytes.

No compiler or network is used. Each fake subprocess records its invocation;
invalid CLI inputs must have no subprocess or output-directory side effects.
"""
import gzip
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / '.github/scripts/id-runtime-artifact.py'
SHA = 'a' * 40


class RuntimeArtifactTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.tools = self.root / 'tools'
        self.tools.mkdir()
        self.package = self.root / 'package'
        (self.package / 'bin').mkdir(parents=True)
        self.log = self.root / 'calls'
        fake = '''#!/usr/bin/env python3
import os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
with open(os.environ['CALLS'], 'a') as log:
    log.write(name + ' ' + ' '.join(sys.argv[1:]) + '\\n')
if os.environ.get('FAIL_TOOL') == name:
    sys.exit(7)
if name == 'nix':
    print(os.environ['PACKAGE'])
elif name == 'nix-store':
    if sys.argv[1] == '--query':
        print(os.environ['PACKAGE'])
    elif not os.environ.get('EMPTY_EXPORT'):
        sys.stdout.buffer.write(b'closure-fixture')
'''
        for path in (self.tools / 'nix', self.tools / 'nix-store', self.package / 'bin/id'):
            path.write_text(fake)
            path.chmod(0o755)
        self.env = dict(os.environ, PATH=f'{self.tools}:{os.environ["PATH"]}',
                        CALLS=str(self.log), PACKAGE=str(self.package))
        self.out = self.root / 'artifact'

    def run_script(self, *args, **env):
        return subprocess.run([sys.executable, str(SCRIPT), *args],
                              env=dict(self.env, **env), capture_output=True, text=True)

    def args(self):
        return ['--output-dir', str(self.out), '--source-sha', SHA]

    def test_invalid_arguments_have_no_side_effects(self):
        # Model: exactly one output directory and a 40-character hex revision.
        cases = [[], ['extra'], ['--unknown'], ['--output-dir'],
                 ['--source-sha'], [*self.args(), 'extra'],
                 [*self.args(), '--typo'], [*self.args(), '--source-sha', SHA]]
        cases += [['--output-dir', str(self.out), '--source-sha', sha]
                  for sha in ('', 'main', 'a' * 39, 'a' * 41, 'z' * 40)]
        for args in cases:
            with self.subTest(args=args):
                result = self.run_script(*args)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertFalse(self.log.exists())
                self.assertFalse(self.out.exists())

    def test_complete_artifact_probes_cli_and_has_verifiable_bytes(self):
        result = self.run_script(*self.args())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.out.is_dir(), 'successful packaging must publish an artifact')
        manifest = json.loads((self.out / 'manifest.json').read_text())
        self.assertEqual(manifest['source_sha'], SHA)
        self.assertEqual(manifest['store_path'], str(self.package))
        self.assertEqual(gzip.decompress((self.out / 'runtime.closure.gz').read_bytes()),
                         b'closure-fixture')
        for line in (self.out / 'SHA256SUMS').read_text().splitlines():
            digest, name = line.split('  ')
            self.assertEqual(digest, hashlib.sha256((self.out / name).read_bytes()).hexdigest())
        calls = self.log.read_text()
        self.assertIn('id serve --help', calls)
        self.assertIn('id world join --help', calls)
        self.assertEqual(calls.count('nix build '), 1)
        self.assertIn('nix-store --export ', calls)
        self.assertIn('nix-store --import', (self.out / 'INSTALL.txt').read_text())
        self.assertNotIn('profile install', (self.out / 'INSTALL.txt').read_text())

    def test_failed_tools_or_empty_export_publish_nothing(self):
        for env in ({'FAIL_TOOL': 'nix'}, {'FAIL_TOOL': 'nix-store'},
                    {'FAIL_TOOL': 'id'}, {'EMPTY_EXPORT': '1'}):
            with self.subTest(env=env):
                result = self.run_script(*self.args(), **env)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(self.out.exists())

    def test_existing_output_is_never_overwritten(self):
        self.out.mkdir()
        marker = self.out / 'keep'
        marker.write_text('existing work')
        result = self.run_script(*self.args())
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(marker.read_text(), 'existing work')
        self.assertFalse(self.log.exists())


if __name__ == '__main__':
    unittest.main()
