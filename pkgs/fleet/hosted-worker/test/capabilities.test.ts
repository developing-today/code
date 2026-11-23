// Health + capabilities tests (#12583). Oracle: the native
// `fleet capabilities` document shape (fleet-registry / print_capabilities)
// for the mirrored fields, and the design §2 dispositions for the
// hosted values. The document must be truthful: a domain reports
// available only when its filling issue landed — so far only the
// hosted-transformed `gh` domain, via the stamping door (#12585).

import assert from "node:assert/strict";
import { test } from "node:test";
import { dispatch } from "../src/router.ts";
import { makeHarness } from "./helpers.ts";

const h = await makeHarness();

test("health answers publicly with the hosted door's identity", async () => {
  const res = await dispatch(new Request("https://fleet-hosted.test/health"), h.deps);
  assert.equal(res.status, 200);
  const body = (await res.json()) as {
    status: string;
    service: string;
    doors: string[];
    auth: { rp_configured: boolean; tenancy_configured: boolean };
  };
  assert.equal(body.status, "ok");
  assert.equal(body.service, "fleet-hosted");
  assert.deepEqual(body.doors, ["stamping", "query", "queue", "ingestion", "sync"]);
  assert.deepEqual(body.auth, { rp_configured: true, tenancy_configured: true });
});

test("capabilities mirrors the native shape with hosted values", async () => {
  const res = await dispatch(new Request("https://fleet-hosted.test/capabilities"), h.deps);
  assert.equal(res.status, 200);
  const doc = (await res.json()) as {
    build: { name: string; version: string; invoked_as: string; profile: string };
    capabilities: string[];
    compiled_capabilities: string[];
    domains: {
      name: string;
      summary: string;
      aliases: string[];
      capabilities: string[];
      available: boolean;
      state: string;
      hosted: string;
      filling_issue?: string;
    }[];
    doors: { id: string; state: string; filling_issue: string }[];
  };

  // Native shape: build identity + both capability lists + domains.
  assert.equal(doc.build.name, "fleet-hosted");
  assert.equal(doc.build.invoked_as, "fleet-hosted");
  assert.equal(doc.build.profile, "hosted");
  assert.ok(Array.isArray(doc.capabilities));
  assert.ok(Array.isArray(doc.compiled_capabilities));

  // The native registry's twelve domains, each fully shaped.
  assert.equal(doc.domains.length, 12);
  for (const d of doc.domains) {
    assert.equal(typeof d.name, "string");
    assert.equal(typeof d.summary, "string");
    assert.ok(Array.isArray(d.aliases));
    assert.ok(Array.isArray(d.capabilities));
    if (d.name === "gh") continue; // asserted below: landed by #12585
    // Truth: no other business domain is available at this door yet.
    assert.equal(d.available, false, `${d.name} must not report available yet`);
    assert.equal(d.state, "not-in-build");
  }

  const byName = new Map(doc.domains.map((d) => [d.name, d]));
  assert.equal(byName.get("gh")?.hosted, "hosted-transformed");
  assert.equal(byName.get("gh")?.filling_issue, "#12585");
  // #12585 landed the hosted gh create path (through the stamping
  // door): the domain reports available and its capability is listed.
  assert.equal(byName.get("gh")?.available, true);
  assert.equal(byName.get("gh")?.state, "available");
  assert.deepEqual(doc.capabilities, ["gh"]);
  assert.deepEqual(doc.compiled_capabilities, ["gh"]);
  assert.equal(byName.get("queue")?.hosted, "hosted-front-door");
  assert.equal(byName.get("queue")?.filling_issue, "#12589");
  assert.equal(byName.get("secrets")?.hosted, "native-only");
  assert.equal(byName.get("bridge")?.hosted, "already-hosted");

  // The component-C doors and the issues that fill them.
  assert.deepEqual(
    doc.doors.map((d) => [d.id, d.filling_issue]),
    [
      ["stamping", "#12585"],
      ["query", "#12587"],
      ["queue", "#12589"],
      ["ingestion", "#12590"],
      ["sync", "#12591"],
    ],
  );
  const doorById = new Map(doc.doors.map((d) => [d.id, d]));
  assert.equal(doorById.get("stamping")?.state, "live");
  assert.equal(doorById.get("query")?.state, "live");
});
