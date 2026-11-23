#!/usr/bin/env python3
"""Disposable stand-in server for the C3 Slice D drill (issue #9571).

Modes:
  aggregator  -- GET /board returns the status in the board_status control
                 file (200 or 503); POST /api/alerts is captured to
                 captures.jsonl (Authorization value is NEVER recorded,
                 only its presence) and answered 200.
  opencode    -- GET /global/health returns 200 {"healthy": true}.

Usage: capture_server.py <mode> <port> <workdir>
"""

import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

mode, port, workdir = sys.argv[1], int(sys.argv[2]), Path(sys.argv[3])


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):  # silence
        pass

    def _send(self, code, body=b"{}"):
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if mode == "aggregator" and self.path == "/board":
            try:
                status = int((workdir / "board_status").read_text().strip())
            except Exception:
                status = 200
            self._send(
                status,
                b'{"board":"drill"}' if status == 200 else b'{"error":"drill down"}',
            )
        elif mode == "opencode" and self.path == "/global/health":
            self._send(200, b'{"healthy": true, "version": "slice-d-drill"}')
        else:
            self._send(404, b'{"error":"not found"}')

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b""
        try:
            body = json.loads(raw or b"{}")
        except json.JSONDecodeError:
            body = {"_unparsed": raw.decode("utf-8", "replace")}
        capture = {
            "method": "POST",
            "path": self.path,
            "authorizationPresent": bool(self.headers.get("Authorization")),
            "contentType": self.headers.get("Content-Type"),
            "body": body,
        }
        with (workdir / "captures.jsonl").open("a") as f:
            f.write(json.dumps(capture) + "\n")
        self._send(200, b'{"ok": true}')


HTTPServer(("127.0.0.1", port), Handler).serve_forever()
