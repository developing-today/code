import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, lstat, mkdir, symlink, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { bridgeQueueSnapshot, publishBridgeQueueSnapshot, BRIDGE_STATUSES } from "../src/bridge-snapshot.ts";
import type { QueueSnapshot } from "../src/queue.ts";

const NOW = 1_800_000_000_000;
function source(): QueueSnapshot {
  return {
    bridgeVerified: true, available: true, stale: false, fetchedAt: NOW, errors: [],
    tasks: [{ id: 12, status: "queued", priority: 10, title: "SECRET", payload: "SECRET", progressNext: "SECRET" }],
    queues: [{ queued: 1, claimed: 2, parked: 3, handedOff: 4, blocked: 5, done: 6, cancelled: 7, failed: 8, queue: "SECRET" }],
  } as unknown as QueueSnapshot;
}
const path = (home: string) => join(home, ".local/state/pm-aggregator/bridge-queue.json");

// Oracle: output has only declared scalar fields and unknown completeness.
test("projection drops every free-form field and does not invent complete pagination", () => {
  const got = bridgeQueueSnapshot(source(), NOW);
  assert.deepEqual(got, {version:1,fetchedAt:NOW,available:true,stale:false,complete:false,total:null,
    tasks:[{id:12,status:"queued",priority:10}],
    counts:{queued:1,claimed:2,parked:3,handed_off:4,blocked:5,done:6,cancelled:7,failed:8}});
  assert.ok(!JSON.stringify(got).includes("SECRET"));
});

test("all eight statuses retained and output is capped without claiming completeness", () => {
  const input = source();
  input.tasks = Array.from({length: 1001}, (_, i) => ({...input.tasks[0],id:i+1,status:BRIDGE_STATUSES[i%8]}));
  const got = bridgeQueueSnapshot(input, NOW);
  assert.equal(got.tasks.length, 1000);
  assert.equal(got.complete, false);
  assert.equal(got.total, null);
  assert.deepEqual([...new Set(got.tasks.map(t => t.status))].sort(), [...BRIDGE_STATUSES].sort());
});

// Generated boundary corpus; oracle is fail closed with no stale row exposure.
test("invalid numbers, status, freshness, counts and unavailable source fail closed", () => {
  const mutations: ((s: QueueSnapshot) => void)[] = [
    s => {s.available=false;}, s => {s.stale=true;}, s => {s.fetchedAt=NOW-300001;},
    s => {s.fetchedAt=NOW+5001;}, s => {s.tasks[0].status="untrusted";},
    s => {s.tasks.push({...s.tasks[0]});},
  ];
  for (const value of [NaN, Infinity, -1, 0, 0.5, Number.MAX_SAFE_INTEGER+1]) mutations.push(s => {s.tasks[0].id=value;});
  for (const value of [NaN, Infinity, -1, 0.5, Number.MAX_SAFE_INTEGER+1]) mutations.push(s => {s.queues[0].queued=value;});
  for (const mutate of mutations) {
    const input=source(); mutate(input);
    const got=bridgeQueueSnapshot(input,NOW);
    assert.equal(got.available,false); assert.equal(got.stale,true); assert.deepEqual(got.tasks,[]);
  }
});

test("disabled publisher performs no filesystem effects", async () => {
  await publishBridgeQueueSnapshot(source(), undefined, "/path-that-does-not-exist", NOW);
  await publishBridgeQueueSnapshot(source(), "true", "/path-that-does-not-exist", NOW);
});

test("private atomic file, failure marker and symlink denial", async () => {
  const home=await mkdtemp(join(tmpdir(),"bridge-projection-"));
  const other=await mkdtemp(join(tmpdir(),"bridge-target-"));
  try {
    await publishBridgeQueueSnapshot(source(),"1",home,NOW);
    const first=JSON.parse(await readFile(path(home),"utf8"));
    assert.equal(first.available,true);
    assert.equal((await lstat(path(home))).mode & 0o777,0o600);
    const bad=source(); bad.available=false;
    await publishBridgeQueueSnapshot(bad,"1",home,NOW);
    assert.equal(JSON.parse(await readFile(path(home),"utf8")).available,false);
    await rm(path(home)); await symlink(join(other,"untouched"),path(home));
    await assert.rejects(publishBridgeQueueSnapshot(source(),"1",home,NOW), /target refused/);
    await assert.rejects(lstat(join(other,"untouched")), {code:"ENOENT"});
    await rm(join(home,".local"),{recursive:true});
    await symlink(other,join(home,".local"));
    await assert.rejects(publishBridgeQueueSnapshot(source(),"1",home,NOW), /directory refused/);
    await assert.rejects(lstat(join(other,"state")), {code:"ENOENT"});
  } finally { await rm(home,{recursive:true,force:true}); await rm(other,{recursive:true,force:true}); }
});

test("writable ancestor refused before a file is written", async () => {
  const home=await mkdtemp(join(tmpdir(),"bridge-mode-"));
  try {
    await mkdir(join(home,".local"),{mode:0o777});
    // mkdir is affected by umask, so explicitly set the unsafe fixture mode.
    const {chmod}=await import("node:fs/promises"); await chmod(join(home,".local"),0o777);
    await assert.rejects(publishBridgeQueueSnapshot(source(),"1",home,NOW), /directory refused/);
  } finally {await rm(home,{recursive:true,force:true});}
});

test("live source wires success and failure publication without another broker read", async () => {
  const live=await readFile(new URL("../src/sources/live.ts",import.meta.url),"utf8");
  const body=live.slice(live.indexOf("async function queueSnapshot()"),live.indexOf("\n  return {",live.indexOf("async function queueSnapshot()")));
  assert.equal((body.match(/fetchQueueSnapshot\(/g)||[]).length,1);
  assert.ok(body.includes("publishBridgeQueueSnapshot(snap)"));
  assert.ok(body.includes('publishBridgeQueueSnapshot(unavailableQueueSnapshot(Date.now(), "unavailable"))'));
});
