import json
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from pathlib import Path
from server import ThreadingHTTPServer, handler, setup


class MailboxTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        path = Path(self.tmp.name) / "test.sqlite3"
        setup(path)
        for label, routes in [
            ("p", {"/mcp"}),
            ("c", {"/consumer/pull", "/consumer/reply"}),
        ]:
            srv = ThreadingHTTPServer(("127.0.0.1", 0), handler(path, label, routes))
            setattr(self, label, "http://127.0.0.1:" + str(srv.server_port))
            threading.Thread(target=srv.serve_forever, daemon=True).start()
            self.addCleanup(srv.server_close)
            self.addCleanup(srv.shutdown)

    def req(self, root, route, body, token, **headers):
        rq = urllib.request.Request(
            root + route,
            data=json.dumps(body).encode(),
            headers={"Authorization": "Bearer " + token, **headers},
        )
        try:
            with urllib.request.urlopen(rq, timeout=3) as r:
                return r.status, json.loads(r.read())
        except urllib.error.HTTPError as e:
            return e.code, None

    def rpc(self, method, params=None):
        return self.req(
            self.p,
            "/mcp",
            {"jsonrpc": "2.0", "id": 1, "method": method, "params": params},
            "p",
        )[1]

    def tool(self, name, **args):
        return json.loads(
            self.rpc("tools/call", {"name": name, "arguments": args})["result"][
                "content"
            ][0]["text"]
        )

    def test_round_trip(self):
        self.assertEqual(
            self.rpc("initialize")["result"]["protocolVersion"], "2025-03-26"
        )
        self.assertEqual(len(self.rpc("tools/list")["result"]["tools"]), 2)
        i = self.tool("send_message", text="hello", request_id="client-request-0001")[
            "message_id"
        ]
        self.assertEqual(self.tool("get_reply", message_id=i)["status"], "pending")
        self.assertEqual(
            self.req(self.c, "/consumer/pull", {}, "c")[1]["messages"],
            [{"message_id": i, "text": "hello"}],
        )
        for value, expected in [
            ("hello back", 200),
            ("hello back", 200),
            ("overwrite", 409),
        ]:
            self.assertEqual(
                self.req(
                    self.c, "/consumer/reply", {"message_id": i, "text": value}, "c"
                )[0],
                expected,
            )
        self.assertEqual(
            self.tool("get_reply", message_id=i),
            {"status": "replied", "text": "hello back"},
        )
        self.assertEqual(self.req(self.c, "/consumer/pull", {}, "c")[1]["messages"], [])

    def test_send_retry(self):
        args = {"text": "hello", "request_id": "client-request-0002"}
        self.assertEqual(
            self.tool("send_message", **args), self.tool("send_message", **args)
        )
        args["text"] = "different"
        result = self.rpc("tools/call", {"name": "send_message", "arguments": args})
        self.assertEqual(result["error"]["code"], -32602)
        self.assertEqual(
            len(self.req(self.c, "/consumer/pull", {}, "c")[1]["messages"]), 1
        )
        annotations = {
            t["name"]: t["annotations"]
            for t in self.rpc("tools/list")["result"]["tools"]
        }
        self.assertFalse(annotations["send_message"]["readOnlyHint"])
        self.assertTrue(annotations["get_reply"]["readOnlyHint"])

    def test_boundaries(self):
        self.assertEqual(self.req(self.p, "/consumer/pull", {}, "c")[0], 404)
        self.assertEqual(self.req(self.c, "/mcp", {}, "p")[0], 404)
        self.assertEqual(self.req(self.c, "/consumer/pull", {}, "p")[0], 401)
        self.assertEqual(self.req(self.p, "/mcp", {}, "c")[0], 401)
        self.assertEqual(
            self.req(self.p, "/mcp", {}, "p", Origin="https://wrong.example")[0], 403
        )
        result = self.rpc(
            "tools/call",
            {"name": "send_message", "arguments": {"text": "hi", "owner": "forged"}},
        )
        self.assertEqual(result["error"]["code"], -32602)


if __name__ == "__main__":
    unittest.main()
