//! Power-loss model behind redb's `StorageBackend`.
//!
//! `write` and `set_len` calls wait in `pending` until a `sync_data` call completes. A power cut
//! keeps the durable bytes plus the unsynced operations that `Survive` selects, applied in issue
//! order (`Keep`, `Random`) or shuffled (`Reorder`). The write in flight at the cut can be torn:
//! only a prefix of its bytes lands. After the cut every call returns `Ok` and does nothing, as
//! redb's own `CrashBackend` does (`src/db.rs` in redb 4.3.0). The caller must stop counting work
//! once `cut()` is true, because the process it models is dead.

use redb::StorageBackend;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

/// Which unsynced operations reach the disk at a power cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Survive {
    /// None. This is what `LazyFS` `clear-cache` does.
    Drop,
    /// All, in issue order. This is what the kernel does when it writes back before the cut.
    Keep,
    /// Each one independently, in issue order.
    Random,
    /// A random subset, in random order. Stronger than real disks need to be for redb.
    Reorder,
}

impl Survive {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Drop => "Drop",
            Self::Keep => "Keep",
            Self::Random => "Random",
            Self::Reorder => "Reorder",
        }
    }
}

/// What lands of the write in flight at the cut. Only applies when the cut lands on a write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tear {
    /// Nothing lands.
    Off,
    /// A random byte prefix, from 0 to the full length.
    Byte,
    /// A random prefix of whole 512-byte sectors.
    Sector,
}

impl Tear {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Byte => "Byte",
            Self::Sector => "Sector",
        }
    }
}

/// Where power is lost, and what survives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// 1-based index of the storage call on which power is lost.
    pub event: u64,
    pub survive: Survive,
    pub tear: Tear,
    /// Only for a sync cut: true means the sync finished before power was lost.
    pub sync_completed: bool,
    /// Picks which unsynced operations survive, and how far a torn write lands.
    pub seed: u64,
}

/// The kind of a storage call, as recorded in the trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Write,
    SetLen,
    Sync,
}

impl Kind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Write => "write",
            Self::SetLen => "set_len",
            Self::Sync => "sync",
        }
    }
}

/// Every storage call up to and including the cut: kind, offset (the length for `set_len`), and
/// data length.
pub type Trace = Vec<(Kind, u64, usize)>;

#[derive(Clone, Debug)]
enum Op {
    Write { offset: u64, data: Vec<u8> },
    SetLen(u64),
}

#[derive(Debug, Default)]
struct State {
    /// What the process sees: durable bytes with every pending operation applied.
    live: Vec<u8>,
    /// What a completed sync has made durable.
    durable: Vec<u8>,
    /// Operations issued since the last completed sync.
    pending: Vec<Op>,
    trace: Trace,
    plan: Option<Plan>,
    /// When true, `sync_data` returns `Ok` without making anything durable (the control).
    lying_sync: bool,
    cut: bool,
    /// The disk after the cut.
    image: Vec<u8>,
}

/// Cloneable handle. redb owns one clone; the harness keeps another to read the outcome.
#[derive(Clone, Debug)]
pub struct FaultFs(Arc<Mutex<State>>);

impl FaultFs {
    /// A disk whose contents are `image`, and which starts out durable. `plan` cuts the power at
    /// one storage call. `lying_sync` makes `sync_data` report success without making anything
    /// durable.
    pub fn new(image: Vec<u8>, plan: Option<Plan>, lying_sync: bool) -> Self {
        let state = State {
            live: image.clone(),
            durable: image,
            plan,
            lying_sync,
            ..State::default()
        };
        Self(Arc::new(Mutex::new(state)))
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().expect("faultfs state lock")
    }

    /// True once the power cut has landed.
    pub fn cut(&self) -> bool {
        self.lock().cut
    }

    /// Every storage call so far, up to the cut.
    pub fn trace(&self) -> Trace {
        self.lock().trace.clone()
    }

    /// The disk to reopen: the image at the cut, or the live bytes if no cut happened.
    pub fn disk_image(&self) -> Vec<u8> {
        let st = self.lock();
        if st.cut {
            st.image.clone()
        } else {
            st.live.clone()
        }
    }

    /// What a power cut now would leave if it dropped every unsynced operation: the durable bytes.
    pub fn synced_image(&self) -> Vec<u8> {
        self.lock().durable.clone()
    }
}

/// `SplitMix64`. Every random choice in the model is derived from a seed through this function.
pub const fn splitmix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Whether the unsynced operation at `index` survives `Survive::Random` for this seed.
const fn keeps(seed: u64, index: usize) -> bool {
    splitmix(seed ^ splitmix(index as u64 + 1)) & 1 == 1
}

fn apply(image: &mut Vec<u8>, op: &Op) {
    match op {
        Op::Write { offset, data } => {
            let start = *offset as usize;
            let end = start + data.len();
            if image.len() < end {
                image.resize(end, 0);
            }
            image[start..end].copy_from_slice(data);
        }
        Op::SetLen(len) => image.resize(*len as usize, 0),
    }
}

/// The disk after a cut that keeps only the unsynced operations `Survive` selects.
fn surviving_image(st: &State) -> Vec<u8> {
    let plan = st.plan.expect("a cut needs a plan");
    let n = st.pending.len();
    let order: Vec<usize> = match plan.survive {
        Survive::Drop => Vec::new(),
        Survive::Keep => (0..n).collect(),
        Survive::Random => (0..n).filter(|&i| keeps(plan.seed, i)).collect(),
        Survive::Reorder => {
            let mut v: Vec<usize> = (0..n).filter(|&i| keeps(plan.seed, i)).collect();
            let mut s = plan.seed ^ 0xA5A5_A5A5;
            for i in (1..v.len()).rev() {
                s = splitmix(s);
                v.swap(i, (s % (i as u64 + 1)) as usize);
            }
            v
        }
    };
    let mut image = st.durable.clone();
    for i in order {
        apply(&mut image, &st.pending[i]);
    }
    image
}

/// Makes every pending operation durable, in issue order.
fn flush(st: &mut State) {
    let State {
        durable, pending, ..
    } = st;
    for op in pending.drain(..) {
        apply(durable, &op);
    }
}

/// Counts a storage call and reports whether the power cut lands on it.
fn tick(st: &mut State, kind: Kind, offset: u64, len: usize) -> bool {
    st.trace.push((kind, offset, len));
    st.plan.is_some_and(|p| p.event == st.trace.len() as u64)
}

impl StorageBackend for FaultFs {
    fn len(&self) -> Result<u64, io::Error> {
        Ok(self.lock().live.len() as u64)
    }

    fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), io::Error> {
        let st = self.lock();
        let start = offset as usize;
        let end = start + out.len();
        if end > st.live.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "read past end",
            ));
        }
        out.copy_from_slice(&st.live[start..end]);
        Ok(())
    }

    fn set_len(&self, len: u64) -> Result<(), io::Error> {
        let mut guard = self.lock();
        let st = &mut *guard;
        if st.cut {
            return Ok(());
        }
        if tick(st, Kind::SetLen, len, 0) {
            st.image = surviving_image(st);
            st.cut = true;
            return Ok(());
        }
        st.live.resize(len as usize, 0);
        st.pending.push(Op::SetLen(len));
        Ok(())
    }

    fn sync_data(&self) -> Result<(), io::Error> {
        let mut guard = self.lock();
        let st = &mut *guard;
        if st.cut {
            return Ok(());
        }
        if tick(st, Kind::Sync, 0, 0) {
            let plan = st.plan.expect("a cut needs a plan");
            if plan.sync_completed && !st.lying_sync {
                flush(st);
                st.image = st.durable.clone();
            } else {
                st.image = surviving_image(st);
            }
            st.cut = true;
            return Ok(());
        }
        if !st.lying_sync {
            flush(st);
        }
        Ok(())
    }

    fn write(&self, offset: u64, data: &[u8]) -> Result<(), io::Error> {
        let mut guard = self.lock();
        let st = &mut *guard;
        if st.cut {
            return Ok(());
        }
        if tick(st, Kind::Write, offset, data.len()) {
            let plan = st.plan.expect("a cut needs a plan");
            let mut image = surviving_image(st);
            let landed = match plan.tear {
                Tear::Off => 0,
                Tear::Byte => (splitmix(plan.seed ^ 0x51) % (data.len() as u64 + 1)) as usize,
                Tear::Sector => {
                    let sectors = (data.len() / 512) as u64;
                    (splitmix(plan.seed ^ 0x52) % (sectors + 1)) as usize * 512
                }
            };
            if landed > 0 {
                apply(
                    &mut image,
                    &Op::Write {
                        offset,
                        data: data[..landed].to_vec(),
                    },
                );
            }
            st.image = image;
            st.cut = true;
            return Ok(());
        }
        let end = offset as usize + data.len();
        if st.live.len() < end {
            st.live.resize(end, 0);
        }
        st.live[offset as usize..end].copy_from_slice(data);
        st.pending.push(Op::Write {
            offset,
            data: data.to_vec(),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{FaultFs, Plan, Survive, Tear};
    use redb::StorageBackend;
    use std::collections::BTreeSet;

    fn plan(event: u64, survive: Survive, tear: Tear, sync_completed: bool) -> Plan {
        Plan {
            event,
            survive,
            tear,
            sync_completed,
            seed: 7,
        }
    }

    #[test]
    fn unsynced_writes_are_dropped_at_a_power_cut() {
        // 1: write A, 2: sync, 3: write B (unsynced), 4: write C cuts the power.
        let fs = FaultFs::new(
            Vec::new(),
            Some(plan(4, Survive::Drop, Tear::Off, false)),
            false,
        );
        fs.write(0, b"A").unwrap();
        fs.sync_data().unwrap();
        fs.write(1, b"B").unwrap();
        fs.write(2, b"C").unwrap();
        assert!(fs.cut());
        assert_eq!(fs.disk_image(), b"A");
    }

    #[test]
    fn synced_writes_are_kept_and_unsynced_writes_survive_under_keep() {
        // The same calls as above. Keep writes back B; C is the in-flight write and lands nothing.
        let fs = FaultFs::new(
            Vec::new(),
            Some(plan(4, Survive::Keep, Tear::Off, false)),
            false,
        );
        fs.write(0, b"A").unwrap();
        fs.sync_data().unwrap();
        fs.write(1, b"B").unwrap();
        fs.write(2, b"C").unwrap();
        assert_eq!(fs.disk_image(), b"AB");
    }

    #[test]
    fn a_completed_sync_makes_the_earlier_writes_durable() {
        // 1: write A, 2: write B, 3: the sync completes, then power is lost.
        let fs = FaultFs::new(
            Vec::new(),
            Some(plan(3, Survive::Drop, Tear::Off, true)),
            false,
        );
        fs.write(0, b"A").unwrap();
        fs.write(1, b"B").unwrap();
        fs.sync_data().unwrap();
        assert!(fs.cut());
        assert_eq!(fs.disk_image(), b"AB");
    }

    #[test]
    fn a_sync_cut_before_the_sync_takes_effect_loses_the_writes() {
        // 1: write A, 2: power is lost during the sync.
        let fs = FaultFs::new(
            Vec::new(),
            Some(plan(2, Survive::Drop, Tear::Off, false)),
            false,
        );
        fs.write(0, b"A").unwrap();
        fs.sync_data().unwrap();
        assert_eq!(fs.disk_image(), b"");
    }

    #[test]
    fn a_torn_write_lands_a_prefix_of_its_bytes() {
        let data: Vec<u8> = (0..1024u32).map(|i| (i % 251) as u8 + 1).collect();
        let mut byte_lengths = BTreeSet::new();
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
                // 3: the in-flight write, torn.
                fs.write(1, &data).unwrap();
                let img = fs.disk_image();
                assert_eq!(img[0], b'A');
                let landed = img.len() - 1;
                assert!(landed <= data.len(), "seed {seed} {tear:?}");
                assert_eq!(
                    &img[1..],
                    &data[..landed],
                    "seed {seed} {tear:?}: not a prefix"
                );
                if tear == Tear::Sector {
                    assert_eq!(
                        landed % 512,
                        0,
                        "seed {seed}: a sector tear lands whole sectors"
                    );
                } else {
                    byte_lengths.insert(landed);
                }
            }
        }
        assert!(
            byte_lengths.len() > 2,
            "byte tears should land different lengths: {byte_lengths:?}"
        );
    }

    #[test]
    fn a_lying_sync_leaves_nothing_durable() {
        // 1: write A, 2: sync (lies), 3: write B cuts the power.
        let fs = FaultFs::new(
            Vec::new(),
            Some(plan(3, Survive::Drop, Tear::Off, false)),
            true,
        );
        fs.write(0, b"A").unwrap();
        fs.sync_data().unwrap();
        fs.write(1, b"B").unwrap();
        assert!(fs.cut());
        assert_eq!(fs.disk_image(), b"");
    }

    #[test]
    fn set_len_is_unsynced_until_a_sync() {
        // 1: write AB, 2: sync, 3: set_len(1) is unsynced, 4: a write cuts the power.
        let dropped = FaultFs::new(
            Vec::new(),
            Some(plan(4, Survive::Drop, Tear::Off, false)),
            false,
        );
        dropped.write(0, b"AB").unwrap();
        dropped.sync_data().unwrap();
        dropped.set_len(1).unwrap();
        dropped.write(2, b"Z").unwrap();
        assert_eq!(dropped.disk_image(), b"AB");

        let kept = FaultFs::new(
            Vec::new(),
            Some(plan(4, Survive::Keep, Tear::Off, false)),
            false,
        );
        kept.write(0, b"AB").unwrap();
        kept.sync_data().unwrap();
        kept.set_len(1).unwrap();
        kept.write(2, b"Z").unwrap();
        assert_eq!(kept.disk_image(), b"A");
    }

    #[test]
    fn random_and_reorder_keep_a_subset_of_the_unsynced_writes() {
        for survive in [Survive::Random, Survive::Reorder] {
            let (mut some_lost, mut some_kept) = (false, false);
            for seed in 0..64 {
                let p = Some(Plan {
                    event: 4,
                    survive,
                    tear: Tear::Off,
                    sync_completed: false,
                    seed,
                });
                let fs = FaultFs::new(Vec::new(), p, false);
                // 1-3: three unsynced writes at distinct offsets. 4: a write cuts the power.
                fs.write(0, b"a").unwrap();
                fs.write(1, b"b").unwrap();
                fs.write(2, b"c").unwrap();
                fs.write(3, b"d").unwrap();
                let img = fs.disk_image();
                assert!(
                    img.len() <= 3,
                    "seed {seed} {survive:?}: the in-flight write landed"
                );
                for (i, &byte) in img.iter().enumerate() {
                    assert!(
                        byte == 0 || byte == b"abc"[i],
                        "seed {seed} {survive:?}: byte {i} is {byte}"
                    );
                }
                let kept = (0..3)
                    .filter(|&i| img.get(i).is_some_and(|&b| b != 0))
                    .count();
                some_lost |= kept < 3;
                some_kept |= kept > 0;
            }
            assert!(
                some_lost && some_kept,
                "{survive:?} should keep some writes and drop others"
            );
        }
    }

    #[test]
    fn calls_after_the_cut_do_not_reach_the_disk() {
        // 1: write A cuts the power, and the write itself is lost.
        let fs = FaultFs::new(
            Vec::new(),
            Some(plan(1, Survive::Keep, Tear::Off, false)),
            false,
        );
        fs.write(0, b"A").unwrap();
        fs.write(0, b"Z").unwrap();
        fs.sync_data().unwrap();
        fs.set_len(0).unwrap();
        assert!(fs.cut());
        assert_eq!(fs.disk_image(), b"");
        assert_eq!(fs.trace().len(), 1);
    }
}
