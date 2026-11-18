"""Small Hrana client for fleet-platform operator scripts; no import-time I/O."""
import json
import os
import subprocess
import urllib.request
import urllib.error


def argument(value):
    if value is None:
        return {"type": "null"}
    if isinstance(value, bool):
        return {"type": "integer", "value": str(int(value))}
    if isinstance(value, int):
        return {"type": "integer", "value": str(value)}
    if isinstance(value, float):
        return {"type": "float", "value": value}
    if isinstance(value, (dict, list)):
        value = json.dumps(value)
    return {"type": "text", "value": str(value)}


class Turso:
    def __init__(self):
        self.url = os.environ.get("FLEET_TURSO_URL", "https://fleet-platform-dezren39.aws-us-east-1.turso.io").replace("libsql://", "https://").rstrip("/")
        if not self.url.startswith("https://"):
            raise ValueError("FLEET_TURSO_URL must use HTTPS")
        self.token = os.environ.get("FLEET_TURSO_TOKEN")
        if not self.token:
            result = subprocess.run(["fleet", "secrets", "get", "turso.fleet-platform-rw-token"], capture_output=True, text=True, timeout=60)
            if result.returncode:
                raise RuntimeError("could not resolve fleet-platform credential")
            self.token = result.stdout.strip()
        if not self.token:
            raise RuntimeError("empty fleet-platform credential")

    def batch(self, statements, transaction=False):
        # Single Hrana batch, conditional COMMIT and rollback on any error.
        stmts = list(statements)
        if transaction:
            stmts = [("BEGIN IMMEDIATE", [])] + stmts
        steps = []
        for i, (sql, args) in enumerate(stmts):
            step = {"stmt": {"sql": sql, "args": [argument(x) for x in args], "want_rows": True}}
            if i:
                step["condition"] = {"type": "ok", "step": i - 1}
            steps.append(step)
        if transaction:
            last = len(steps)-1
            steps.append({"condition": {"type": "ok", "step": last}, "stmt": {"sql": "COMMIT"}})
            steps.append({"condition": {"type": "not", "cond": {"type": "ok", "step": last}}, "stmt": {"sql": "ROLLBACK"}})
        request = urllib.request.Request(self.url + "/v2/pipeline", data=json.dumps({"requests": [{"type": "batch", "batch": {"steps": steps}}, {"type": "close"}]}).encode(), headers={"Authorization": "Bearer " + self.token, "Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                result = json.load(response)["results"][0]
        except urllib.error.HTTPError as e:
            raise RuntimeError(f"Turso HTTP {e.code}") from None
        if result["type"] == "error":
            raise RuntimeError("Turso batch rejected")
        batch = result["response"]["result"]
        if any(batch.get("step_errors", [])):
            raise RuntimeError("Turso statement failed; transaction rolled back" if transaction else "Turso statement failed")
        return [self.rows(r) for r in batch["step_results"][:len(stmts)]]

    @staticmethod
    def rows(result):
        if result is None:
            return []
        def value(v):
            if v["type"] == "null": return None
            if v["type"] == "integer": return int(v["value"])
            if v["type"] == "float": return float(v["value"])
            return v.get("value")
        return [{c["name"]: value(v) for c, v in zip(result["cols"], row)} for row in result["rows"]]

    def query(self, sql, args=()):
        return self.batch([(sql, args)])[0]
