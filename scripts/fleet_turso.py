"""Small Hrana client for fleet-platform operator scripts; no import-time I/O."""
import json
import os
import subprocess
import urllib.request
import urllib.error
from urllib.parse import urlsplit


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
        parsed = urlsplit(self.url)
        if (parsed.scheme != "https" or parsed.hostname != "fleet-platform-dezren39.aws-us-east-1.turso.io"
                or parsed.username or parsed.password or parsed.port not in (None, 443)
                or parsed.path or parsed.query or parsed.fragment):
            raise ValueError("FLEET_TURSO_URL must identify the fleet-platform HTTPS origin")
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
            steps.append({"condition": {"type": "not", "cond": {"type": "ok", "step": last + 1}}, "stmt": {"sql": "ROLLBACK"}})
        request = urllib.request.Request(self.url + "/v2/pipeline", data=json.dumps({"requests": [{"type": "batch", "batch": {"steps": steps}}, {"type": "close"}]}).encode(), headers={"Authorization": "Bearer " + self.token, "Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                result = json.load(response)["results"][0]
        except urllib.error.HTTPError as e:
            raise RuntimeError(f"Turso HTTP {e.code}; outcome unverified, read back before retrying") from None
        except (OSError, ValueError, KeyError, IndexError, TypeError) as e:
            raise RuntimeError("Turso transport/response failure; outcome unverified, read back before retrying") from None
        uncertain = "Turso response incomplete or rejected; outcome unverified, read back before retrying"
        if not isinstance(result, dict) or result.get("type") != "ok":
            raise RuntimeError(uncertain)
        response = result.get("response")
        if not isinstance(response, dict) or response.get("type") != "batch":
            raise RuntimeError(uncertain)
        batch = response.get("result")
        if not isinstance(batch, dict):
            raise RuntimeError(uncertain)
        results, errors = batch.get("step_results"), batch.get("step_errors")
        if (not isinstance(results, list) or not isinstance(errors, list)
                or len(results) != len(steps) or len(errors) != len(steps)):
            raise RuntimeError(uncertain)
        def valid(r):
            if not isinstance(r, dict) or not isinstance(r.get("cols"), list) or not isinstance(r.get("rows"), list):
                return False
            if not all(isinstance(c, dict) and isinstance(c.get("name"), str) for c in r["cols"]):
                return False
            return all(isinstance(row, list) and len(row) == len(r["cols"])
                       and all(isinstance(v, dict) and v.get("type") in ("null", "integer", "float", "text", "blob") for v in row)
                       for row in r["rows"])
        if any(e is not None for e in errors):
            rolled_back = transaction and errors[-1] is None and valid(results[-1])
            raise RuntimeError("Turso statement failed; rollback confirmed" if rolled_back else uncertain)
        # Every data step (and BEGIN/COMMIT for atomic batches) must be evidenced.
        expected = len(stmts) + (1 if transaction else 0)
        if not all(valid(r) for r in results[:expected]) or (transaction and results[-1] is not None):
            raise RuntimeError(uncertain)
        try:
            return [self.rows(r) for r in results[:len(stmts)]]
        except (KeyError, ValueError, TypeError):
            raise RuntimeError(uncertain) from None

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
