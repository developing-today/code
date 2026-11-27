"""Cloudflare account model for the repository applier tests."""

import copy

import cloudflare


class Account(cloudflare.Cloudflare):
    def __init__(self, check=False):
        super().__init__("token", check)
        self.idps = [{"id": "idp-cf", "type": "cloudflare", "name": ""}]
        self.apps = []
        self.policies = {}
        self.ingress = [
            {"hostname": "kaneo.security.cab", "service": "http://localhost:5173"},
            {"service": "http_status:404"},
        ]
        self.dns = []
        self.zones = {"security.cab": "zone"}
        self.order = []
        self.routes = []
        self.worker_up = True
        self.worker_host = "pm.security.cab"
        self.health_misses = 0
        self.worker_secrets = {}
        self.uploads = 0
        self.fail_upload = 0
        self.delete_failures = 0

    def call(self, method, path, body=None):
        path, _, query = path.partition("?")
        self.asked = getattr(self, "asked", 0) + 1
        if method != "GET":
            self.order.append(f"{method} {path}")
        if path == "/accounts":
            result = [{"id": "acc"}]
        elif path.endswith("/access/organizations"):
            result = {"auth_domain": "team.cloudflareaccess.com"}
        elif path.endswith("/access/identity_providers"):
            if method == "POST":
                self.idps.append({"id": f"idp-{body['type']}", **body})
            result = self.idps[-1] if method == "POST" else self.idps
        elif "/access/identity_providers/" in path and method == "PUT":
            idp = next(i for i in self.idps if i["id"] == path.split("/")[-1])
            idp.update(body)
            result = idp
        elif "/access/apps/" in path and method == "PUT":
            app = next(a for a in self.apps if a["id"] == path.split("/")[-1])
            app.update(body)
            result = app
        elif path.endswith("/access/apps"):
            if method == "POST":
                self.apps.append({"id": f"app-{len(self.apps)}", "aud": f"aud-{len(self.apps)}", **body})
                self.policies[self.apps[-1]["id"]] = []
            result = self.apps[-1] if method == "POST" else self.apps
        elif path.endswith("/policies"):
            app = path.split("/")[-2]
            if method == "POST":
                self.policies[app].append(body)
            result = self.policies[app]
        elif path.endswith("/cfd_tunnel"):
            result = [{"id": "tun", "name": cloudflare.TUNNEL}]
        elif path.endswith("/configurations"):
            if method == "PUT":
                self.ingress = body["config"]["ingress"]
            result = {"config": {"ingress": copy.deepcopy(self.ingress), "warp-routing": {"enabled": False}}}
        elif path == "/zones":
            name = query.partition("name=")[2]
            result = [{"id": self.zones[name]}] if name in self.zones else []
        elif path.endswith(f"/workers/scripts/{cloudflare.WORKER}/secrets"):
            if method == "PUT":
                self.uploads += 1
                if self.uploads == self.fail_upload:
                    raise SystemExit("cloudflare PUT secrets: refused")
                self.worker_secrets[body["name"]] = body["text"]
            result = [{"name": name, "type": "secret_text"} for name in self.worker_secrets]
        elif f"/workers/scripts/{cloudflare.WORKER}/secrets/" in path and method == "DELETE":
            if self.delete_failures > 0:
                self.delete_failures -= 1
                raise SystemExit("cloudflare DELETE secret: refused")
            self.worker_secrets.pop(path.split("/")[-1], None)
            result = None
        elif path.endswith("/workers/subdomain"):
            result = {"subdomain": "acct"}
        elif "/workers/routes" in path:
            if method == "POST":
                self.routes.append({"id": f"route-{len(self.routes)}", **body})
            elif method == "DELETE":
                self.routes = [r for r in self.routes if r["id"] != path.split("/")[-1]]
            result = self.routes
        elif path.endswith("/dns_records"):
            if method == "POST":
                self.dns.append({**body, "zone": path.split("/")[2]})
            name = query.partition("name=")[2]
            result = [r for r in self.dns if r["name"] == name]
        else:
            raise AssertionError(f"unexpected call {method} {path}")
        return {"success": True, "result": result}
