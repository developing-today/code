//! Power-loss model behind redb's `StorageBackend`.
//!
//! `write` and `set_len` calls wait in `pending` until a `sync_data` call completes. A power cut
//! keeps the durable bytes plus the unsynced operations that `Survive` selects, applied in issue
//! order (`Keep`, `Random`) or shuffled (`Reorder`). The write in flight at the cut can be torn:
//! only a prefix of its bytes lands. After the cut every call returns `Ok` and does nothing, as
//! redb's own `CrashBackend` does (src/db.rs). The caller must stop counting work once `cut()`
//! is true, because the process it models is dead.

use redb::StorageBackend;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

/// Which unsynced operations reach the disk at a power cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Survive {
    /// None. This is what LazyFS `clear-cache` does.
    Drop,
    /// All, in issue order. This is what the kernel does when it writes back before the cut.
    Keep,
    /// Each one independently, in issue order.
    Random,
    /// A random subset, in random order. Stronger than real disks need to be for redb.
    Reorder,
}

/// What lands of the write in flight at the cut (only when the cut lands on a write).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tear {
    Off,
    /// A random byte prefix, from 0 to the full length.
    Byte,
    /// A random prefix of whole 512-byte sectors.
    Sector,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// 1-based index of the storage call on which power is lost.
    pub event: u64,
    pub survive: Survive,
    pub tear: Tear,
    /// Only for a sync cut: true means the sync finished before power was lost.
    pub sync_completed: bool,
    pub seed: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Write,
    SetLen,
    Sync,
}

#[derive(Clone, Debug)]
enum Op {
    Write { offset: u64, data: Vec<u8> },
    SetLen(u64),
}

#[derive(Debug, Default)]
struct State {
    /// What the process sees: durable bytes with every pending operation applied.
    live: Vec<u8>,
    /// What a sync has made durable.
    durable: Vec<u8>,
    /// Operations issued since the last completed sync.
    pending: Vec<Op>,
    trace: Vec<(Kind, u64, usize)>,
    plan: Option<Plan>,
    /// When true, `sync_data` returns `Ok` without making anything durable (a control).
    lying_sync: bool,
    cut: bool,
    /// The disk after the cut.
    image: Vec<u8>,
}

/// Cloneable handle. redb owns one clone; the harness keeps another to read the outcome.
#[derive(Clone, Debug)]
pub struct FaultFs(Arc<Mutex<State>>);

impl FaultFs {
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

    /// Every storage call so far, in order: kind, offset (or length for set_len), data length.
    pub fn trace(&self) -> Vec<(Kind, u64, usize)> {
        self.lock().trace.clone()
    }

    /// The disk to reopen: the image at the cut, or the live bytes if no cut happened.
    pub fn disk_image(&self) -> Vec<u8> {
        let st = self.lock();
        if st.cut { st.image.clone() } else { st.live.clone() }
    }
}

pub fn splitmix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn keeps(seed: u64, index: usize) -> bool {
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

/// The disk after a cut that keeps only some unsynced operations.
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

fn flush(st: &mut State) {
    let State { durable, pending, .. } = st;
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
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "read past end"));
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
                apply(&mut image, &Op::Write { offset, data: data[..landed].to_vec() });
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
        st.pending.push(Op::Write { offset, data: data.to_vec() });
        Ok(())
    }
}
