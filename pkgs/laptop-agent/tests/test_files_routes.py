#!/usr/bin/env python3
"""Tests for the #5759 /files/* transfer routes on the laptop-agent.

Run with: python3 -m unittest discover -s pkgs/laptop-agent/tests

The oracle is the shipped fleet-proxy client
(pkgs/fleet/crates/fleet-proxy/src/files.rs): its in-memory fake
daemon defines the route semantics these tests assert against. The
invariants under test are the design's, not incidental behaviour:

  * deterministic transfer temp `.<name>.fleet-part-<sha256[..12]>`
  * offset-idempotent writes; a conflicting rewrite is refused
  * hash-checked commit, then an atomic rename, so partial content
    is never visible at the destination path
  * resume works purely from the temp's stat'd size
"""

import base64
import hashlib
import http.client
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

AGENT = Path(__file__).resolve().parents[1] / "src" / "laptop_agent.py"
TOKEN = "<MASKED_SECRET_002>"
CHUNK = 2 << 20  # the client's CHUNK_SIZE


def free_port():
    import socket

    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def sha_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def temp_for(path: str, sha: str) -> str:
    """Mirror of the client's temp_path_for (the protocol's naming)."""
    i = path.rfind("/")
    directory, name = (path[: i + 1], path[i + 1 :]) if i >= 0 else ("", path)
    return f"{directory}.{name}.fleet-part-{sha[:12]}"


class FilesRoutesTest(unittest.TestCase):
    def setUp(self):
        self.work = Path(tempfile.mkdtemp())
        with tempfile.NamedTemporaryFile("w", delete=False) as f:
            f.write(TOKEN + "\n")
            self.token_path = f.name
        self.port = free_port()
        env = dict(
            os.environ,
            LAPTOP_AGENT_TOKEN_FILE=self.token_path,
            LAPTOP_AGENT_HOST="127.0.0.1",
            LAPTOP_AGENT_PORT=str(self.port),
            LAPTOP_AGENT_STATE=str(self.work / "state"),
        )
        self.proc = subprocess.Popen(
            [sys.executable, str(AGENT)],
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        deadline = time.time() + 10
        while time.time() < deadline:
            try:
                status, body = self.rpc_get("/health")
                if status == 200 and body.get("ok"):
                    return
            except Exception:
                time.sleep(0.1)
        self.tearDown()
        self.fail("agent did not start")

    def tearDown(self):
        self.proc.terminate()
        self.proc.wait(timeout=10)
        os.unlink(self.token_path)
        import shutil

        shutil.rmtree(self.work, ignore_errors=True)

    # -- transport ----------------------------------------------------

    def rpc_get(self, path):
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=30)
        c.request("GET", path, headers={"X-Agent-Token": TOKEN})
        r = c.getresponse()
        payload = r.read()
        c.close()
        return r.status, json.loads(payload) if payload else {}

    def rpc(self, route, body):
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=60)
        c.request(
            "POST",
            route,
            body=json.dumps(body).encode(),
            headers={"X-Agent-Token": TOKEN, "Content-Type": "application/json"},
        )
        r = c.getresponse()
        payload = r.read()
        c.close()
        return r.status, json.loads(payload) if payload else {}

    def ok(self, route, body):
        status, reply = self.rpc(route, body)
        self.assertEqual(status, 200, reply)
        self.assertNotIn("error", reply, reply)
        return reply

    def err(self, route, body):
        status, reply = self.rpc(route, body)
        self.assertEqual(status, 200, reply)
        self.assertIn("error", reply, reply)
        return reply

    # -- client-shaped helpers ----------------------------------------

    def put(self, dest, data, mode=None, resume=False):
        """Drive begin/write/commit exactly as the client does."""
        sha = sha_hex(data)
        begin = {"path": str(dest), "size": len(data), "sha256": sha}
        if mode is not None:
            begin["mode"] = mode
        self.ok("/files/begin", begin)
        start = 0
        if resume:
            st = self.ok(
                "/files/stat", {"path": temp_for(str(dest), sha), "sha256": False}
            )
            if (
                st.get("exists")
                and st.get("type") == "file"
                and st["size"] <= len(data)
            ):
                start = st["size"]
        for offset in range(start, len(data), CHUNK):
            chunk = data[offset : offset + CHUNK]
            reply = self.ok(
                "/files/write",
                {
                    "path": str(dest),
                    "sha256": sha,
                    "offset": offset,
                    "data_b64": base64.b64encode(chunk).decode(),
                },
            )
            self.assertEqual(reply["size"], offset + len(chunk))
        return self.ok("/files/commit", {"path": str(dest), "sha256": sha})

    def get(self, src):
        st = self.ok("/files/stat", {"path": str(src), "sha256": True})
        self.assertTrue(st["exists"])
        out = bytearray()
        offset = 0
        while offset < st["size"]:
            reply = self.ok(
                "/files/read",
                {
                    "path": str(src),
                    "offset": offset,
                    "length": min(CHUNK, st["size"] - offset),
                },
            )
            out.extend(base64.b64decode(reply["data_b64"]))
            offset = len(out)
        return bytes(out), st["sha256"]

    # -- /files/copy: delegated direct-path copy (#14219) --------------
    #
    # The oracle is the shipped client's `copy_file_direct` contract:
    # the client states {source, path, sha256, size, mode?} from its
    # stat of the source; the daemon must publish exactly those bytes
    # at `path`, hash-verified, by atomic rename, resuming from a
    # proven temp prefix and never exposing a partial destination.

    def copy_body(self, source, dest, data, **extra):
        body = {
            "source": str(source),
            "path": str(dest),
            "sha256": sha_hex(data),
            "size": len(data),
        }
        body.update(extra)
        return body

    def test_copy_publishes_exact_bytes_and_clears_temp(self):
        source = self.work / "copy-source.bin"
        dest = self.work / "copy-dest.bin"
        data = os.urandom(5 * 1024 * 1024)
        source.write_bytes(data)
        dest.write_bytes(b"old destination")
        report = self.ok("/files/copy", self.copy_body(source, dest, data))
        self.assertEqual(
            {key: report[key] for key in ("size", "sha256")},
            {"size": len(data), "sha256": sha_hex(data)},
        )
        self.assertEqual(dest.read_bytes(), data)
        self.assertEqual(list(self.work.glob(".copy-dest.bin.fleet-part-*")), [])

    def test_copy_missing_source_never_touches_destination(self):
        dest = self.work / "copy-failed.bin"
        dest.write_bytes(b"keep original")
        reply = self.err(
            "/files/copy",
            self.copy_body(self.work / "missing-source", dest, b"whatever"),
        )
        self.assertIn("no such file", reply["error"])
        self.assertEqual(dest.read_bytes(), b"keep original")
        self.assertEqual(list(self.work.glob(".copy-failed.bin.fleet-part-*")), [])

    def test_copy_resumes_from_proven_temp_prefix(self):
        source = self.work / "copy-resume-source.bin"
        dest = self.work / "copy-resume-dest.bin"
        data = os.urandom(4 * 1024 * 1024)
        source.write_bytes(data)
        # A killed earlier attempt left the first MiB in the temp.
        Path(temp_for(str(dest), sha_hex(data))).write_bytes(data[: 1 << 20])
        report = self.ok("/files/copy", self.copy_body(source, dest, data))
        self.assertEqual(report["sha256"], sha_hex(data))
        self.assertEqual(dest.read_bytes(), data)
        self.assertEqual(list(self.work.glob(".copy-resume-dest.bin.fleet-part-*")), [])

    def test_copy_wrong_stated_hash_refuses_and_keeps_destination(self):
        source = self.work / "copy-hash-source.bin"
        dest = self.work / "copy-hash-dest.bin"
        data = os.urandom(1 * 1024 * 1024)
        source.write_bytes(data)
        dest.write_bytes(b"keep original")
        body = self.copy_body(source, dest, data)
        body["sha256"] = sha_hex(b"not the source")
        reply = self.err("/files/copy", body)
        self.assertIn("sha256 mismatch", reply["error"])
        self.assertEqual(dest.read_bytes(), b"keep original")

    # -- AC1: whole-file round trips are sha256-identical ---------------
    #
    # The packet's acceptance is a 100 MiB transfer both ways. 100 MiB
    # is the live acceptance run (it needs the deployed daemon); the
    # multi-chunk path and the hash identity it exercises are the same
    # at 5 MiB, which keeps the hermetic suite fast.

    def test_put_get_round_trip_multi_chunk(self):
        data = bytes(range(256)) * (5 * 1024 * 1024 // 256 + 1)
        data = data[: 5 * 1024 * 1024]
        dest = self.work / "round-trip.bin"
        report = self.put(dest, data)
        self.assertEqual(report["size"], len(data))
        self.assertEqual(report["sha256"], sha_hex(data))
        self.assertEqual(dest.read_bytes(), data)
        got, stated = self.get(dest)
        self.assertEqual(got, data)
        self.assertEqual(stated, sha_hex(data))
        # The temp is consumed by commit: no .fleet-part-* left behind.
        self.assertEqual(list(self.work.glob("*.fleet-part-*")), [])

    def test_2mib_chunk_body_exceeds_exec_body_cap(self):
        """A literal 2 MiB chunk must be accepted (MAX_BODY would not)."""
        data = os.urandom(CHUNK)
        dest = self.work / "chunk-cap.bin"
        self.put(dest, data)
        self.assertEqual(dest.read_bytes(), data)

    # -- AC2: partial content is never visible at the destination -------

    def test_destination_untouched_until_commit(self):
        data = os.urandom(CHUNK + 1234)
        dest = self.work / "atomic.bin"
        dest.write_bytes(b"ORIGINAL-CONTENT")
        sha = sha_hex(data)
        self.ok("/files/begin", {"path": str(dest), "sha256": sha, "size": len(data)})
        # Write only the first chunk, then look at the destination.
        self.ok(
            "/files/write",
            {
                "path": str(dest),
                "sha256": sha,
                "offset": 0,
                "data_b64": base64.b64encode(data[:CHUNK]).decode(),
            },
        )
        self.assertEqual(dest.read_bytes(), b"ORIGINAL-CONTENT")
        # A sibling temp holds the partial bytes, by the protocol's name.
        temp = Path(temp_for(str(dest), sha))
        self.assertTrue(temp.exists())
        self.assertEqual(temp.stat().st_size, CHUNK)
        # Commit publishes atomically and clears the temp.
        self.ok(
            "/files/write",
            {
                "path": str(dest),
                "sha256": sha,
                "offset": CHUNK,
                "data_b64": base64.b64encode(data[CHUNK:]).decode(),
            },
        )
        self.ok("/files/commit", {"path": str(dest), "sha256": sha})
        self.assertEqual(dest.read_bytes(), data)
        self.assertFalse(temp.exists())

    def test_commit_hash_mismatch_keeps_temp_and_destination(self):
        dest = self.work / "mismatch.bin"
        dest.write_bytes(b"KEEP-ME")
        sha = sha_hex(b"the-true-content")
        self.ok("/files/begin", {"path": str(dest), "sha256": sha, "size": 0})
        self.ok(
            "/files/write",
            {
                "path": str(dest),
                "sha256": sha,
                "offset": 0,
                "data_b64": base64.b64encode(b"corrupted").decode(),
            },
        )
        reply = self.err("/files/commit", {"path": str(dest), "sha256": sha})
        self.assertIn("sha256 mismatch", reply["error"])
        self.assertIn("actual", reply)
        # Destination untouched; temp retained for resume or abort.
        self.assertEqual(dest.read_bytes(), b"KEEP-ME")
        self.assertTrue(Path(temp_for(str(dest), sha)).exists())

    # -- AC2: a killed transfer resumes and completes -------------------

    def test_resume_from_partial_temp(self):
        data = os.urandom(CHUNK * 2 + 999)
        dest = self.work / "resume.bin"
        sha = sha_hex(data)
        # Simulate a client killed after chunk 1: begin + one write.
        self.ok("/files/begin", {"path": str(dest), "sha256": sha, "size": len(data)})
        self.ok(
            "/files/write",
            {
                "path": str(dest),
                "sha256": sha,
                "offset": 0,
                "data_b64": base64.b64encode(data[:CHUNK]).decode(),
            },
        )
        self.assertFalse(dest.exists())
        # A fresh client re-derives the temp name and resumes from its size.
        st = self.ok("/files/stat", {"path": temp_for(str(dest), sha), "sha256": False})
        self.assertEqual(st["size"], CHUNK)
        report = self.put(dest, data, resume=True)
        self.assertEqual(report["size"], len(data))
        self.assertEqual(dest.read_bytes(), data)
        self.assertEqual(sha_hex(dest.read_bytes()), sha)

    def test_write_is_offset_idempotent(self):
        """Re-sending a boundary chunk after resume must be accepted."""
        data = os.urandom(4096)
        dest = self.work / "idem.bin"
        sha = sha_hex(data)
        self.ok("/files/begin", {"path": str(dest), "sha256": sha, "size": len(data)})
        self.ok(
            "/files/write",
            {
                "path": str(dest),
                "sha256": sha,
                "offset": 0,
                "data_b64": base64.b64encode(data[:2048]).decode(),
            },
        )
        # Replay the same bytes at the same offset: identical, so accepted.
        self.ok(
            "/files/write",
            {
                "path": str(dest),
                "sha256": sha,
                "offset": 0,
                "data_b64": base64.b64encode(data[:2048]).decode(),
            },
        )
        self.ok(
            "/files/write",
            {
                "path": str(dest),
                "sha256": sha,
                "offset": 2048,
                "data_b64": base64.b64encode(data[2048:]).decode(),
            },
        )
        self.ok("/files/commit", {"path": str(dest), "sha256": sha})
        self.assertEqual(dest.read_bytes(), data)

    def test_interior_identical_replay_reports_durable_size(self):
        """An identical replay fully interior to the temp is a no-op
        success reporting the durable length, not the replay's end
        (#14601): the tail past the replay must stay intact."""
        data = os.urandom(4096)
        dest = self.work / "interior.bin"
        sha = sha_hex(data)
        self.ok("/files/begin", {"path": str(dest), "sha256": sha, "size": len(data)})
        first = self.ok(
            "/files/write",
            {
                "path": str(dest),
                "sha256": sha,
                "offset": 0,
                "data_b64": base64.b64encode(data).decode(),
            },
        )
        self.assertEqual(first["size"], len(data))
        # Replay 10 identical bytes at offset 0 of a 4096-byte temp.
        replay = self.ok(
            "/files/write",
            {
                "path": str(dest),
                "sha256": sha,
                "offset": 0,
                "data_b64": base64.b64encode(data[:10]).decode(),
            },
        )
        self.assertEqual(replay["size"], len(data))
        # Mid-temp interior replay likewise reports the durable length.
        mid = self.ok(
            "/files/write",
            {
                "path": str(dest),
                "sha256": sha,
                "offset": 1000,
                "data_b64": base64.b64encode(data[1000:1010]).decode(),
            },
        )
        self.assertEqual(mid["size"], len(data))
        # The tail survived both replays: commit verifies the full hash.
        self.ok("/files/commit", {"path": str(dest), "sha256": sha})
        self.assertEqual(dest.read_bytes(), data)

    def test_interior_conflicting_rewrite_refused_and_preserves_temp(self):
        """A conflicting interior rewrite is refused and leaves every
        durable byte in place (#14601)."""
        data = os.urandom(4096)
        dest = self.work / "interior-conflict.bin"
        sha = sha_hex(data)
        self.ok("/files/begin", {"path": str(dest), "sha256": sha, "size": len(data)})
        self.ok(
            "/files/write",
            {
                "path": str(dest),
                "sha256": sha,
                "offset": 0,
                "data_b64": base64.b64encode(data).decode(),
            },
        )
        bad = bytearray(data[500:510])
        bad[0] ^= 0xFF
        reply = self.err(
            "/files/write",
            {
                "path": str(dest),
                "sha256": sha,
                "offset": 500,
                "data_b64": base64.b64encode(bytes(bad)).decode(),
            },
        )
        self.assertIn("conflicting rewrite", reply["error"])
        st = self.ok("/files/stat", {"path": temp_for(str(dest), sha), "sha256": False})
        self.assertEqual(st["size"], len(data))
        self.ok("/files/commit", {"path": str(dest), "sha256": sha})
        self.assertEqual(dest.read_bytes(), data)

    def test_conflicting_rewrite_refused(self):
        data = os.urandom(4096)
        dest = self.work / "conflict.bin"
        sha = sha_hex(data)
        self.ok("/files/begin", {"path": str(dest), "sha256": sha, "size": len(data)})
        self.ok(
            "/files/write",
            {
                "path": str(dest),
                "sha256": sha,
                "offset": 0,
                "data_b64": base64.b64encode(data[:2048]).decode(),
            },
        )
        reply = self.err(
            "/files/write",
            {
                "path": str(dest),
                "sha256": sha,
                "offset": 0,
                "data_b64": base64.b64encode(b"\x00" * 2048).decode(),
            },
        )
        self.assertIn("conflicting rewrite", reply["error"])

    def test_offset_beyond_end_refused(self):
        data = os.urandom(1024)
        dest = self.work / "gap.bin"
        sha = sha_hex(data)
        self.ok("/files/begin", {"path": str(dest), "sha256": sha, "size": len(data)})
        reply = self.err(
            "/files/write",
            {
                "path": str(dest),
                "sha256": sha,
                "offset": 8192,
                "data_b64": base64.b64encode(b"x").decode(),
            },
        )
        self.assertIn("offset beyond end", reply["error"])

    def test_abort_removes_temp(self):
        data = os.urandom(1024)
        dest = self.work / "abort.bin"
        sha = sha_hex(data)
        self.ok("/files/begin", {"path": str(dest), "sha256": sha, "size": len(data)})
        self.ok(
            "/files/write",
            {
                "path": str(dest),
                "sha256": sha,
                "offset": 0,
                "data_b64": base64.b64encode(data).decode(),
            },
        )
        self.ok("/files/abort", {"path": str(dest), "sha256": sha})
        self.assertFalse(Path(temp_for(str(dest), sha)).exists())
        self.assertFalse(dest.exists())

    def test_stat_missing_file(self):
        reply = self.ok(
            "/files/stat", {"path": str(self.work / "nope"), "sha256": True}
        )
        self.assertEqual(reply["exists"], False)
        self.assertEqual(reply["type"], "missing")

    def test_read_beyond_eof_refused(self):
        src = self.work / "short.bin"
        src.write_bytes(b"abc")
        reply = self.err("/files/read", {"path": str(src), "offset": 10, "length": 4})
        self.assertIn("offset beyond end", reply["error"])

    # -- path and mode validation ---------------------------------------

    def test_empty_path_refused(self):
        for route in ("/files/stat", "/files/begin", "/files/write", "/files/read"):
            reply = self.err(route, {"path": "", "sha256": "a" * 64})
            self.assertIn("path required", reply["error"], route)

    def test_nul_path_refused(self):
        reply = self.err("/files/stat", {"path": "a\0b"})
        self.assertIn("NUL", reply["error"])

    def test_mode_applied_at_commit(self):
        data = os.urandom(256)
        dest = self.work / "mode.bin"
        self.put(dest, data, mode=0o640)
        self.assertEqual(dest.stat().st_mode & 0o777, 0o640)

    def test_bad_mode_refused(self):
        reply = self.err(
            "/files/begin",
            {"path": str(self.work / "x"), "sha256": "a" * 64, "mode": "0755"},
        )
        self.assertIn("mode must be an integer", reply["error"])

    def test_unknown_files_route_404s(self):
        status, reply = self.rpc("/files/nonsense", {})
        self.assertEqual(status, 404)
        self.assertEqual(reply["error"], "no such route")

    def test_begin_requires_existing_parent(self):
        reply = self.err(
            "/files/begin",
            {"path": str(self.work / "no-such-dir" / "f"), "sha256": "a" * 64},
        )
        self.assertIn("no such directory", reply["error"])


if __name__ == "__main__":
    unittest.main()
