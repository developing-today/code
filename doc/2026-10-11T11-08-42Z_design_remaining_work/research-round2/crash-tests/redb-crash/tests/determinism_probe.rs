//! Probe: does the storage-call trace and verdict for one plan depend on the build profile?
use redb_crash::faultfs::{Plan, Survive, Tear};
use redb_crash::harness::run_one;
use redb_crash::workload;

#[test]
fn trace_and_verdict_for_one_plan() {
    let models = workload::models();
    let (clean, trace) = run_one(None, false, &models);
    let counts = trace.iter().fold([0u64; 3], |mut acc, (k, _, _)| {
        acc[match k { redb_crash::faultfs::Kind::Write => 0, redb_crash::faultfs::Kind::SetLen => 1, redb_crash::faultfs::Kind::Sync => 2 }] += 1;
        acc
    });
    println!("PROBE clean verdict={} calls={} writes/setlens/syncs={:?}", clean.verdict.name(), trace.len(), counts);
    println!("PROBE calls 140..=150: {:?}", &trace[139..150]);
    let plan = Plan { event: 149, survive: Survive::Keep, tear: Tear::Byte, sync_completed: false, seed: 12777398887419732391 };
    let (o, _) = run_one(Some(plan), false, &models);
    println!("PROBE plan149 verdict={} acked={} repairs={}", o.verdict.name(), o.acked, o.repairs);
}
