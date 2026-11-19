"""Private, single-owner mailbox. Bind behind an HTTPS reverse proxy."""
import hmac
import json
import os
import secrets
import sqlite3
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PROTOCOL = "2025-03-26"
CAP = 65536
TOOLS = {
    "send_message": ("Queue a message for the existing Dot's next poll.", {"text": {"type": "string", "minLength": 1, "maxLength": 8000}, "request_id": {"type": "string", "minLength": 16, "maxLength": 128}}),
    "get_reply": ("Check the reply for a message ID.", {"message_id": {"type": "string"}}),
}

def setup(path):
    with sqlite3.connect(path) as db:
        db.execute("CREATE TABLE IF NOT EXISTS messages(id TEXT PRIMARY KEY, text TEXT NOT NULL, reply TEXT, request_id TEXT)")
        if "request_id" not in {r[1] for r in db.execute("PRAGMA table_info(messages)")}:
            db.execute("ALTER TABLE messages ADD COLUMN request_id TEXT")
        db.execute("CREATE UNIQUE INDEX IF NOT EXISTS messages_request_id ON messages(request_id)")

def handler(path, token, routes):
    class Handler(BaseHTTPRequestHandler):
        timeout = 10

        def do_GET(self):
            self.respond(405 if self.path in routes else 404)
        def log_message(self, *_args):
            pass

        def respond(self, status, body=None):
            self.send_response(status)
            self.send_header("Cache-Control", "no-store")
            if body is not None:
                data = json.dumps(body).encode()
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            if body is not None:
                self.wfile.write(data)

        def permitted(self, token):
            raw = self.headers.get("Authorization", "")
            return hmac.compare_digest(raw.encode(), ("Bearer " + token).encode())

        def do_POST(self):
            if self.path not in routes:
                return self.respond(404)
            if self.headers.get("Origin") is not None:
                return self.respond(403)
            if not self.permitted(token):
                return self.respond(401)
            try:
                size = int(self.headers.get("Content-Length", "0"))
                if not 0 < size <= CAP:
                    return self.respond(413)
                data = json.loads(self.rfile.read(size))
                if not isinstance(data, dict):
                    return self.respond(400)
            except (ValueError, UnicodeError):
                return self.respond(400)
            try:
                if self.path == "/mcp":
                    return self.rpc(data)
                with sqlite3.connect(path) as db:
                    if self.path == "/consumer/pull":
                        rows = db.execute("SELECT id,text FROM messages WHERE reply IS NULL ORDER BY rowid LIMIT 20").fetchall()
                        return self.respond(200, {"messages": [{"message_id": i, "text": t} for i, t in rows]})
                    i, t = data.get("message_id"), data.get("text")
                    if not isinstance(i, str) or not isinstance(t, str) or not 0 < len(t) <= 8000:
                        return self.respond(400)
                    changed = db.execute("UPDATE messages SET reply=? WHERE id=? AND reply IS NULL", (t, i)).rowcount
                    if not changed:
                        old = db.execute("SELECT reply FROM messages WHERE id=?", (i,)).fetchone()
                        if not old or old[0] != t:
                            return self.respond(409)
                return self.respond(200, {"saved": True})
            except sqlite3.Error:
                return self.respond(503)

        def rpc(self, data):
            ident, method = data.get("id"), data.get("method")
            def result(value):
                return self.respond(200, {"jsonrpc": "2.0", "id": ident, "result": value})
            def error(code, message):
                return self.respond(200, {"jsonrpc": "2.0", "id": ident, "error": {"code": code, "message": message}})
            if data.get("jsonrpc") != "2.0" or not isinstance(method, str):
                return error(-32600, "Invalid request")
            if "id" not in data:
                return self.respond(202)
            if method == "initialize":
                params = data.get("params") or {}
                if not isinstance(params, dict):
                    return error(-32602, "Invalid initialize parameters")
                requested = params.get("protocolVersion")
                return result({"protocolVersion": requested if requested in (PROTOCOL, "2025-06-18", "2025-11-25") else PROTOCOL,
                               "capabilities": {"tools": {}}, "serverInfo": {"name": "dot-mailbox", "version": "1.0"}})
            if method == "ping":
                return result({})
            if method == "tools/list":
                return result({"tools": [{"name": n, "description": desc, "inputSchema": {"type": "object", "properties": props,
                              "required": list(props), "additionalProperties": False}, "annotations": {"readOnlyHint": n == "get_reply", "destructiveHint": False, "idempotentHint": True, "openWorldHint": False}} for n, (desc, props) in TOOLS.items()]})
            if method != "tools/call":
                return error(-32601, "Method not found")
            params = data.get("params")
            if not isinstance(params, dict) or not isinstance(params.get("arguments"), dict):
                return error(-32602, "Invalid parameters")
            name, args = params.get("name"), params["arguments"]
            with sqlite3.connect(path) as db:
                if name == "send_message" and set(args) == {"text", "request_id"} and isinstance(args.get("text"), str) and 0 < len(args["text"]) <= 8000 and isinstance(args["request_id"], str) and 16 <= len(args["request_id"]) <= 128:
                    i = secrets.token_urlsafe(24)
                    db.execute("INSERT INTO messages (id,text,request_id) VALUES (?,?,?) ON CONFLICT(request_id) DO NOTHING",
                               (i, args["text"], args["request_id"]))
                    i, original = db.execute("SELECT id,text FROM messages WHERE request_id=?",
                                             (args["request_id"],)).fetchone()
                    if original != args["text"]:
                        return error(-32602, "request_id was previously used with different text")
                    value = {"message_id": i, "status": "queued"}
                elif name == "get_reply" and set(args) == {"message_id"} and isinstance(args.get("message_id"), str):
                    row = db.execute("SELECT reply FROM messages WHERE id=?", (args["message_id"],)).fetchone()
                    value = {"status": "not_found"} if row is None else ({"status": "pending"} if row[0] is None else {"status": "replied", "text": row[0]})
                else:
                    return error(-32602, "Unknown tool or invalid arguments")
            return result({"content": [{"type": "text", "text": json.dumps(value)}]})

    return Handler

if __name__ == "__main__":
    os.umask(0o077)
    p, c = os.environ["MAILBOX_PRODUCER_TOKEN"], os.environ["MAILBOX_CONSUMER_TOKEN"]
    if min(len(p), len(c)) < 32 or hmac.compare_digest(p.encode(), c.encode()):
        raise SystemExit("Use distinct high-entropy tokens of at least 32 characters")
    path = os.environ.get("MAILBOX_DB", "mailbox.sqlite3")
    setup(path)
    public_backend = ThreadingHTTPServer(("127.0.0.1", int(os.environ.get("PORT", "8000"))), handler(path, p, {"/mcp"}))
    private_backend = ThreadingHTTPServer(("127.0.0.1", int(os.environ.get("DOT_PORT", "8001"))),
                                         handler(path, c, {"/consumer/pull", "/consumer/reply"}))
    threading.Thread(target=private_backend.serve_forever, daemon=True).start()
    public_backend.serve_forever()
