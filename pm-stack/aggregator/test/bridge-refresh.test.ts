import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { startBridgeRefresh, BRIDGE_REFRESH_INTERVAL_MS, BRIDGE_REFRESH_LIFETIME_MS, BRIDGE_REFRESH_MAX_RUNS } from "../src/bridge-refresh.ts";

// Oracle: a deterministic scheduler advances exactly to due deadlines; no real
// timer, socket, credential read, filesystem write or collector is started.
class Clock {
  time = 0;
  created: { at: number; fn: () => void; active: boolean; unrefs: number; unref(): void }[] = [];
  now = () => this.time;
  schedule = (fn: () => void, ms: number) => {
    const timer = { at: this.time + ms, fn, active: true, unrefs: 0, unref() { this.unrefs++; } };
    this.created.push(timer); return timer;
  };
  cancel = (timer: { unref?(): unknown }) => { const entry = this.created.find(item => item === timer); if (entry) entry.active = false; };
  options = () => ({ snapshot: "1", refresh: "1", now: this.now, schedule: this.schedule, cancel: this.cancel });
  async advance(target: number) {
    for (;;) {
      await Promise.resolve(); await Promise.resolve();
      const next = this.created.filter(t => t.active && t.at <= target).sort((a,b) => a.at-b.at)[0];
      if (!next) break;
      this.time = next.at; next.active = false; next.fn();
    }
    this.time = target;
    await Promise.resolve(); await Promise.resolve();
  }
}

test("default, partial opt-in and unknown flag values start no work or timers", () => {
  const clock = new Clock(); let starts = 0;
  for (const flags of [{}, {snapshot:"1"}, {refresh:"1"}, {snapshot:"0",refresh:"1"}, {snapshot:"1",refresh:"0"}]) {
    const controller = startBridgeRefresh(async () => { starts++; }, {...clock.options(),snapshot:undefined,refresh:undefined,...flags});
    assert.equal(controller.enabled,false);assert.equal(controller.stopped,true);
  }
  for (const value of ["", "true", "false", "yes", "01", "1 ", "--enable"]) {
    assert.throws(() => startBridgeRefresh(async () => { starts++; }, {...clock.options(),refresh:value}), /invalid.*flag/);
    assert.throws(() => startBridgeRefresh(async () => { starts++; }, {...clock.options(),snapshot:value}), /invalid.*flag/);
  }
  assert.equal(starts,0);assert.equal(clock.created.length,0);
});

test("opt-in starts immediately, repeats after 65 seconds, and all timers are unref'd", async () => {
  const clock = new Clock(); const starts: number[] = [];
  const controller = startBridgeRefresh(async () => { starts.push(clock.now()); }, clock.options());
  assert.deepEqual(starts,[0]);
  await clock.advance(64999);assert.deepEqual(starts,[0]);
  await clock.advance(65000);assert.deepEqual(starts,[0,65000]);
  await clock.advance(130000);assert.deepEqual(starts,[0,65000,130000]);
  assert.ok(clock.created.every(timer => timer.unrefs === 1));
  controller.stop();assert.equal(controller.stopped,true);
  assert.ok(clock.created.every(timer => !timer.active));
});

test("finite pilot makes at most 57 starts and stops permanently at one hour", async () => {
  const clock = new Clock(); let starts=0;
  const controller = startBridgeRefresh(async () => { starts++; }, clock.options());
  await clock.advance(BRIDGE_REFRESH_LIFETIME_MS);
  assert.equal(controller.stopped,true);
  assert.equal(starts,Math.ceil(BRIDGE_REFRESH_LIFETIME_MS / BRIDGE_REFRESH_INTERVAL_MS));
  assert.ok(starts <= BRIDGE_REFRESH_MAX_RUNS);
  assert.ok(clock.created.every(timer => !timer.active));
  const ended=starts;await clock.advance(BRIDGE_REFRESH_LIFETIME_MS*10);assert.equal(starts,ended);
});

test("slow or hung collector never overlaps and independent deadline stops rearming", async () => {
  const clock = new Clock(); let starts=0;let finish!:()=>void;
  const controller = startBridgeRefresh(() => { starts++;return new Promise<void>(resolve => {finish=resolve;}); }, clock.options());
  assert.equal(controller.running,true);
  await clock.advance(900000);assert.equal(starts,1);
  await clock.advance(BRIDGE_REFRESH_LIFETIME_MS);assert.equal(controller.stopped,true);
  finish();await Promise.resolve();await Promise.resolve();
  assert.equal(controller.running,false);
  await clock.advance(BRIDGE_REFRESH_LIFETIME_MS*2);assert.equal(starts,1);
  assert.ok(clock.created.every(timer => !timer.active));
});

test("slow successful read waits 65 seconds after completion, exceeding source cache lifetime", async () => {
  const clock = new Clock();let starts=0;let finish!:()=>void;
  const controller=startBridgeRefresh(() => {starts++;return new Promise<void>(resolve=>{finish=resolve;});},clock.options());
  await clock.advance(40000);assert.equal(starts,1);
  finish();await Promise.resolve();await Promise.resolve();
  await clock.advance(104999);assert.equal(starts,1);
  await clock.advance(105000);assert.equal(starts,2);
  controller.stop();finish();
});

test("failure is retried without fabricating a newer successful source timestamp", async () => {
  const clock=new Clock();let starts=0;let sourceFetchedAt=0;
  const controller=startBridgeRefresh(async()=>{
    starts++;
    if(starts===2)throw new Error("synthetic failure; not logged");
    sourceFetchedAt=clock.now();
  },clock.options());
  await clock.advance(65000);assert.equal(starts,2);assert.equal(sourceFetchedAt,0);
  await clock.advance(130000);assert.equal(starts,3);assert.equal(sourceFetchedAt,130000);
  controller.stop();
});

test("abort and explicit cancellation prevent new starts, including late completion", async () => {
  const clock=new Clock();const abort=new AbortController();let starts=0;let finish!:()=>void;
  const controller=startBridgeRefresh(()=>{starts++;return new Promise<void>(resolve=>{finish=resolve;});},{...clock.options(),signal:abort.signal});
  abort.abort();assert.equal(controller.stopped,true);finish();
  await clock.advance(3600000);assert.equal(starts,1);
  const prior=clock.created.length;
  const disabled=startBridgeRefresh(async()=>{starts++;},{...clock.options(),signal:abort.signal});
  assert.equal(disabled.stopped,true);assert.equal(starts,1);assert.equal(clock.created.length,prior);
});

test("live hook calls existing single-flight collector; helper owns no credential or publication path", async () => {
  const source=await readFile(new URL("../src/sources/live.ts",import.meta.url),"utf8");
  const helper=await readFile(new URL("../src/bridge-refresh.ts",import.meta.url),"utf8");
  assert.ok(source.includes("startBridgeRefresh(queueSnapshot, {"));
  assert.ok(source.includes("refresh: process.env.PM_AGG_BRIDGE_REFRESH"));
  assert.ok(source.includes("snapshot: process.env.PM_AGG_BRIDGE_SNAPSHOT"));
  assert.ok(source.includes("signal: opts?.bridgeRefreshSignal"));
  for(const forbidden of ["readBrokerToken", "fetchQueueSnapshot", "publishBridgeQueueSnapshot", "writeFile", "fetch("]) assert.equal(helper.includes(forbidden),false);
});
