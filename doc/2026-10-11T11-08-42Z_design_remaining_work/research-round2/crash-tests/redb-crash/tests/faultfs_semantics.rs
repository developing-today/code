//! The fault model itself. If these fail, the sweep results mean nothing.

use redb::StorageBackend;
use redb_crash::faultfs::{FaultFs, Plan, Survive, Tear};

fn plan(event: u64, survive: Survive, tear: Tear, sync_completed: bool) -> Option<Plan> {
    Some(Plan {
        event,
        survive,
        tear,
        sync_completed,
        seed: 7,
    })
}

#[test]
fn drop_loses_writes_that_were_not_synced() {
    // 1: write A, 2: sync, 3: write B (unsynced), 4: write C cuts the power.
    let fs = FaultFs::new(Vec::new(), plan(4, Survive::Drop, Tear::Off, false), false);
    fs.write(0, b"A").unwrap();
    fs.sync_data().unwrap();
    fs.write(1, b"B").unwrap();
    fs.write(2, b"C").unwrap();
    assert!(fs.cut());
    assert_eq!(fs.disk_image(), b"A");
}

#[test]
fn keep_writes_back_unsynced_data() {
    let fs = FaultFs::new(Vec::new(), plan(4, Survive::Keep, Tear::Off, false), false);
    fs.write(0, b"A").unwrap();
    fs.sync_data().unwrap();
    fs.write(1, b"B").unwrap();
    fs.write(2, b"C").unwrap();
    assert_eq!(fs.disk_image(), b"AB");
}

#[test]
fn a_sync_cut_before_or_after_the_sync_takes_effect() {
    // 1: write A, 2: sync cuts the power.
    let before = FaultFs::new(Vec::new(), plan(2, Survive::Drop, Tear::Off, false), false);
    before.write(0, b"A").unwrap();
    before.sync_data().unwrap();
    assert_eq!(before.disk_image(), b"");

    let after = FaultFs::new(Vec::new(), plan(2, Survive::Drop, Tear::Off, true), false);
    after.write(0, b"A").unwrap();
    after.sync_data().unwrap();
    assert_eq!(after.disk_image(), b"A");
}

#[test]
fn a_torn_write_lands_a_prefix_and_sector_tears_are_whole_sectors() {
    for seed in 0..64 {
        for tear in [Tear::Byte, Tear::Sector] {
            let p = Some(Plan {
                event: 3,
                survive: Survive::Drop,
                tear,
                sync_completed: false,
                seed,
            });
            let fs = FaultFs::new(Vec::new(), p, false);
            fs.write(0, b"A").unwrap();
            fs.sync_data().unwrap();
            // 3: the in-flight write.
            fs.write(1, &[b'X'; 1024]).unwrap();
            let img = fs.disk_image();
            assert_eq!(img[0], b'A');
            assert!(img[1..].iter().all(|&b| b == b'X'), "seed {seed} {tear:?}");
            assert!(img.len() <= 1025);
            if tear == Tear::Sector {
                assert_eq!((img.len() - 1) % 512, 0, "seed {seed}");
            }
        }
    }
}

#[test]
fn a_lying_sync_leaves_nothing_durable() {
    // 1: write A, 2: sync (lies), 3: write B cuts the power.
    let fs = FaultFs::new(Vec::new(), plan(3, Survive::Drop, Tear::Off, false), true);
    fs.write(0, b"A").unwrap();
    fs.sync_data().unwrap();
    fs.write(1, b"B").unwrap();
    assert_eq!(fs.disk_image(), b"");
}

#[test]
fn calls_after_the_cut_do_not_reach_the_disk() {
    // 1: write A cuts the power, with the write itself lost.
    let fs = FaultFs::new(Vec::new(), plan(1, Survive::Keep, Tear::Off, false), false);
    fs.write(0, b"A").unwrap();
    fs.write(0, b"Z").unwrap();
    fs.sync_data().unwrap();
    fs.set_len(0).unwrap();
    assert!(fs.cut());
    assert_eq!(fs.disk_image(), b"");
    assert_eq!(fs.trace().len(), 1);
}
