//! Power cuts on the event store: first creation (plan P1.3) and the store's directory.
//!
//! [`SimDir`] implements `id::event_store::Storage` over `faultfs` disks. Each file is a `FaultFs`,
//! which models the file's own durability. The directory keeps two views, the entries the process
//! sees and the entries a power cut would keep, and the directory operations since the last
//! `sync_dir` are pending. A cut lands in one of two places:
//!
//! - in a storage call of a file (a redb write or sync), where `FaultFs` decides what survives;
//! - before a directory operation (a create, a remove, a rename, or a `sync_dir`), where every
//!   subset of the pending operations is a possible disk.
//!
//! Each run cuts at one point and collects the disks the cut could leave. The store must reopen
//! every one of them. Creation builds the file under a temporary name and renames it into place, so
//! it passes. The control builds the file in place, and must fail at some cut. Without that failure
//! the sweep could not see the bug it guards against.

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

use id::event_store::{EventStore, FILE_NAME, Storage};
use redb::{Database, DatabaseError, StorageBackend, StorageError, TableDefinition};

use crate::faultfs::{FaultFs, Plan, Survive, Tear};

/// The bytes of every file in a directory, by name: one disk a cut could leave.
type Disk = BTreeMap<String, Vec<u8>>;

/// The storage-call policies each cut is run under. Random and reorder survival keep a subset of
/// the unsynced writes, and tearing lands a prefix of the write in flight.
const POLICIES: [(Survive, Tear); 5] = [
    (Survive::Drop, Tear::Off),
    (Survive::Keep, Tear::Off),
    (Survive::Random, Tear::Byte),
    (Survive::Random, Tear::Sector),
    (Survive::Reorder, Tear::Byte),
];

/// A change to the directory's entries. Applied in order when a crash state is built.
#[derive(Clone, Debug)]
enum DirOp {
    Link(String, FaultFs),
    Unlink(String),
    Rename(String, String),
}

#[derive(Debug, Default)]
struct State {
    /// The entries as the process sees them.
    live: BTreeMap<String, FaultFs>,
    /// The entries as of the last `sync_dir`.
    durable: BTreeMap<String, FaultFs>,
    /// Directory operations since the last `sync_dir`, in order.
    pending: Vec<DirOp>,
    /// Every file the directory has made, in order. The first one carries the redb plan.
    handles: Vec<FaultFs>,
    plan: Option<Plan>,
    /// Directory operations so far, and the operation before which power is cut.
    dir_ops: u64,
    cut_before_dir_op: Option<u64>,
    /// Set by a directory cut. Later operations change nothing.
    dead: bool,
    /// The crash states taken at a directory cut, before the process could change them.
    snapshot: Option<Vec<Disk>>,
}

/// A simulated directory, cheap to clone. One clone is the store's storage, and the test keeps
/// another to read the crash states.
#[derive(Clone, Debug)]
pub struct SimDir(Arc<Mutex<State>>);

impl SimDir {
    /// An empty directory. `plan` cuts the power at one storage call of the first file, and
    /// `cut_before_dir_op` cuts before the given directory operation (1-based).
    pub fn new(plan: Option<Plan>, cut_before_dir_op: Option<u64>) -> Self {
        Self(Arc::new(Mutex::new(State {
            plan,
            cut_before_dir_op,
            ..State::default()
        })))
    }

    /// A directory holding `disk`, all of it durable. This is the state a reopen starts from.
    pub fn from_disk(disk: &Disk) -> Self {
        let mut st = State::default();
        for (name, image) in disk {
            let handle = FaultFs::new(image.clone(), None, false);
            st.live.insert(name.clone(), handle.clone());
            st.durable.insert(name.clone(), handle.clone());
            st.handles.push(handle);
        }
        Self(Arc::new(Mutex::new(st)))
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().expect("sim dir lock")
    }

    /// The storage calls the first file has made so far, up to any cut.
    pub fn calls(&self) -> u64 {
        self.lock()
            .handles
            .first()
            .map_or(0, |handle| handle.trace().len() as u64)
    }

    /// The directory operations so far.
    pub fn dir_ops(&self) -> u64 {
        self.lock().dir_ops
    }

    /// Every disk a power cut at this point could leave behind.
    pub fn crash_states(&self) -> Vec<Disk> {
        let st = self.lock();
        match &st.snapshot {
            Some(snapshot) => snapshot.clone(),
            None => snapshot_of(&st),
        }
    }
}

/// Whether power is already cut. A redb cut shows up as a cut file.
fn dead(st: &State) -> bool {
    st.dead || st.handles.iter().any(FaultFs::cut)
}

/// Counts one directory operation. Returns false if the operation must not happen, because power
/// is already cut or it is the one the cut lands before. A cut takes its snapshot here, before the
/// process can run any later operation.
fn dir_op(st: &mut State) -> bool {
    if dead(st) {
        return false;
    }
    st.dir_ops += 1;
    if st.cut_before_dir_op == Some(st.dir_ops) {
        st.dead = true;
        let snapshot = snapshot_of(st);
        st.snapshot = Some(snapshot);
        return false;
    }
    true
}

fn apply(names: &mut BTreeMap<String, FaultFs>, op: &DirOp) {
    match op {
        DirOp::Link(name, handle) => {
            names.insert(name.clone(), handle.clone());
        }
        DirOp::Unlink(name) => {
            names.remove(name);
        }
        DirOp::Rename(from, to) => {
            if let Some(handle) = names.remove(from) {
                names.insert(to.clone(), handle);
            }
        }
    }
}

/// What a file holds after a power cut: the image taken at the cut for the file that was cut,
/// and the durable bytes for every other file.
fn image_of(handle: &FaultFs) -> Vec<u8> {
    if handle.cut() {
        handle.disk_image()
    } else {
        handle.synced_image()
    }
}

/// Every disk a cut could leave now: the durable entries, with each subset of the pending
/// operations applied in order.
fn snapshot_of(st: &State) -> Vec<Disk> {
    (0..1u64 << st.pending.len())
        .map(|keep| {
            let mut names = st.durable.clone();
            for (index, op) in st.pending.iter().enumerate() {
                if keep & (1 << index) != 0 {
                    apply(&mut names, op);
                }
            }
            names
                .iter()
                .map(|(name, handle)| (name.clone(), image_of(handle)))
                .collect()
        })
        .collect()
}

fn not_found(name: &str) -> DatabaseError {
    DatabaseError::Storage(StorageError::Io(io::Error::new(
        io::ErrorKind::NotFound,
        format!("{name} is missing"),
    )))
}

impl Storage for SimDir {
    fn create_database(&self, name: &str) -> Result<Database, DatabaseError> {
        let handle = {
            let mut st = self.lock();
            if let Some(handle) = st.live.get(name).cloned() {
                handle
            } else {
                let plan = if st.handles.is_empty() { st.plan } else { None };
                let handle = FaultFs::new(Vec::new(), plan, false);
                st.handles.push(handle.clone());
                if dir_op(&mut st) {
                    st.live.insert(name.to_owned(), handle.clone());
                    st.pending
                        .push(DirOp::Link(name.to_owned(), handle.clone()));
                }
                handle
            }
        };
        Database::builder().create_with_backend(handle)
    }

    fn open_database(&self, name: &str) -> Result<Database, DatabaseError> {
        let handle = self
            .lock()
            .live
            .get(name)
            .cloned()
            .ok_or_else(|| not_found(name))?;
        Database::builder().create_with_backend(handle)
    }

    fn exists(&self, name: &str) -> io::Result<bool> {
        Ok(self.lock().live.contains_key(name))
    }

    fn remove(&self, name: &str) -> io::Result<()> {
        let mut st = self.lock();
        if st.live.contains_key(name) && dir_op(&mut st) {
            st.live.remove(name);
            st.pending.push(DirOp::Unlink(name.to_owned()));
        }
        Ok(())
    }

    fn rename(&self, from: &str, to: &str) -> io::Result<()> {
        let mut st = self.lock();
        if !st.live.contains_key(from) {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        if dir_op(&mut st) {
            if let Some(handle) = st.live.remove(from) {
                st.live.insert(to.to_owned(), handle);
            }
            st.pending
                .push(DirOp::Rename(from.to_owned(), to.to_owned()));
        }
        Ok(())
    }

    fn sync_file(&self, name: &str) -> io::Result<()> {
        let handle = {
            let st = self.lock();
            if dead(&st) {
                return Ok(());
            }
            st.live
                .get(name)
                .cloned()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?
        };
        handle.sync_data()
    }

    fn sync_dir(&self) -> io::Result<()> {
        let mut st = self.lock();
        if dir_op(&mut st) {
            st.durable = st.live.clone();
            st.pending.clear();
        }
        Ok(())
    }
}

/// The creation this sweep runs: the store's own open, which creates the file when it is absent.
fn creation(sim: &SimDir) {
    let _ = EventStore::open_with(sim);
}

/// The control: a file built in place, with one committed write. It has no temporary name and no
/// directory steps, so a cut during its first commit leaves the final name holding a partial file.
fn in_place(sim: &SimDir) {
    let Ok(db) = sim.create_database(FILE_NAME) else {
        return;
    };
    let Ok(txn) = db.begin_write() else {
        return;
    };
    let table: TableDefinition<'_, &[u8], &[u8]> = TableDefinition::new("meta");
    if let Ok(mut meta) = txn.open_table(table) {
        let _ = meta.insert(b"format".as_slice(), b"in place".as_slice());
    }
    let _ = txn.commit();
}

/// Reopens one crash state with the store and checks what it holds.
fn reopens(disk: &Disk) -> Result<(), String> {
    let store = EventStore::open_with(&SimDir::from_disk(disk))
        .map_err(|err| format!("does not open: {err}"))?;
    store
        .check_invariants()
        .map_err(|err| format!("opens, but is inconsistent: {err}"))
}

/// Runs `scenario` with a cut at every storage call of the first file, under every policy, and a
/// cut before every directory operation. Returns how many crash states were checked, and the
/// ones that `check` rejects.
fn sweep(scenario: fn(&SimDir), check: fn(&Disk) -> Result<(), String>) -> (usize, Vec<String>) {
    let clean = SimDir::new(None, None);
    scenario(&clean);
    let mut runs: Vec<(String, SimDir)> = Vec::new();
    for event in 1..=clean.calls() {
        for (index, (survive, tear)) in POLICIES.into_iter().enumerate() {
            for sync_completed in [false, true] {
                let plan = Plan {
                    event,
                    survive,
                    tear,
                    sync_completed,
                    seed: event * 7 + index as u64,
                };
                let label = format!(
                    "call {event} {} {} sync_completed={sync_completed}",
                    survive.name(),
                    tear.name()
                );
                runs.push((label, SimDir::new(Some(plan), None)));
            }
        }
    }
    for op in 1..=clean.dir_ops() {
        runs.push((
            format!("before directory operation {op}"),
            SimDir::new(None, Some(op)),
        ));
    }
    let mut states = 0;
    let mut failures = Vec::new();
    for (label, sim) in runs {
        scenario(&sim);
        for disk in sim.crash_states() {
            states += 1;
            if let Err(why) = check(&disk) {
                failures.push(format!("{label}: {why}"));
            }
        }
    }
    (states, failures)
}

fn listing(failures: &[String]) -> String {
    failures
        .iter()
        .take(20)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn clean_creation_leaves_a_store_that_reopens() {
    let sim = SimDir::new(None, None);
    creation(&sim);
    assert!(sim.calls() > 0, "creation made no storage calls");
    assert_eq!(sim.dir_ops(), 4, "creation is link, sync, rename, sync");
    for disk in sim.crash_states() {
        assert!(disk.contains_key(FILE_NAME), "the store file is missing");
        reopens(&disk).unwrap();
    }
}

#[test]
fn first_creation_reopens_after_every_crash_point() {
    let (states, failures) = sweep(creation, reopens);
    println!(
        "first creation: {states} crash states checked, {} failed",
        failures.len()
    );
    assert!(states > 0, "the sweep checked no crash states");
    assert!(
        failures.is_empty(),
        "{} crash states do not reopen:\n{}",
        failures.len(),
        listing(&failures)
    );
}

#[test]
fn creation_in_place_control_fails_at_some_crash_point() {
    let (states, failures) = sweep(in_place, reopens);
    println!(
        "control (in-place creation): {states} crash states checked, {} failed",
        failures.len()
    );
    assert!(
        !failures.is_empty(),
        "the control never failed: the sweep cannot see a torn first commit"
    );
}
