# RESULTS: power-loss tests for the event store (N13)

Date: 2026-10-11. Folder: `/home/user/work/research-round2/crash-tests/`. Host: NixOS, kernel 6.18.44, uid 1337 (`user`), shared machine (other agents running).
Labels: **[verified]** = seen in command output or source in this run. **[inferred]** = my reasoning from verified facts. **[not checked]** = not tested here.

## Questions (verbatim)

Owner: "n13 plan for some sort of portable test, is lazyfs user-space usable?"
Context: power-loss testing for the event store. Round 1 (S6) used SIGKILL only, which does not test unsynced writes.

1. LazyFS. Check whether it is usable here: `/dev/fuse` and `fusermount3` for uid 1337; whether a FUSE mount works inside `unshare -Urm`, with a trivial passthrough first; whether LazyFS is in `/nix/store`; build it only if cheap; run its basic scenario (write, fsync, write without fsync, simulated crash, unsynced data dropped).
2. Portable alternative, in-process. redb 4.3.0 `StorageBackend`: name the trait and its methods. Build a fault-injecting backend that buffers writes, drops writes not followed by fsync, crashes at write number k with an optional torn last write. Check that redb reopens with the last committed transaction intact and no partial batch visible. At least 200 crash points across 50 transactions, with counts.
3. Recommend a test strategy: which tests are portable, which need Linux with FUSE, and how the crash corpus is a static replay list so that every failure becomes a permanent test.

## Short answers

**1. LazyFS: usable, but only inside a user namespace, and not portable.** [verified] `/dev/fuse` is `crw-rw-rw- root root 10,229`, and uid 1337 can open it. `fusermount3` is the setuid wrapper `/run/wrappers/bin/fusermount3` (`-r-s--x--x root root`). A passthrough FUSE filesystem I wrote mounts and works both as uid 1337 through the setuid helper, and inside `unshare -Urm`, where the mount shows `user_id=0`. LazyFS is not in `/nix/store` [verified: no `lazyfs` entry], so I built it from the GitHub archive with g++ 15.3.0 and CMake 4.3.4. Clang (`c++` here is clang 21.1.8) fails on three narrowing errors; g++ compiles with three narrowing warnings. Its basic scenario behaves as documented: unsynced data is dropped by `lazyfs::clear-cache` and by killing the daemon, and kept by `lazyfs::cache-checkpoint` [verified, `logs/lazyfs-basic.log`]. Two limits: `-o allow_other` (which LazyFS's own mount script passes) is refused outside a user namespace because `/etc/fuse.conf` lacks `user_allow_other` [verified], so runs must go inside `unshare -Urm`; and the build downloads toml11 and spdlog at configure time [verified]. It is a good Linux validation tool. It is not a portable test.

**2. Portable in-process alternative: built, and it works.** [verified] The trait is `redb::StorageBackend` (`redb-4.3.0/src/db.rs:74`) with required methods `len`, `read`, `set_len`, `sync_data`, `write`, plus `close` and byte-range lock methods that default to `Unsupported`. My backend (`redb-crash/src/faultfs.rs`) holds writes until `sync_data`, applies one of four survival policies at a power cut, and can tear the in-flight write. The 50-transaction workload has 687 storage calls, and every one is a crash point: 625 writes, 2 `set_len`, 60 syncs. That gives 7,808 runs (12 variants per write, 4 per `set_len`, 5 per sync). Faithful sweep, release build: 7,621 reopen to the acknowledged state, 167 to acknowledged+1 (the in-flight commit became durable), 20 fail to open before the first commit, and **0 failures**. A control with a lying `sync_data` produces 5,731 failures, so the oracle does catch lost durability. The 20 pre-commit open errors are a usability finding: a crash during first creation leaves a file that redb rejects with "Not a redb database: magic number mismatch". No acknowledged data is lost there.

**3. Recommended strategy.** [inferred from the verified results above] Gate CI with the in-process fault model: portable, deterministic, about 40 s in release on this machine. Keep LazyFS as an optional Linux job that runs a few corpus points through the real store binary. Store the corpus as a checked-in TSV, one crash point per line, with the expected class. Every sweep failure is appended with its fix, and a replay test fails if a recorded verdict changes. The "either valid prefix" class (`ok`) is required, because debug and release builds can legitimately recover different prefixes at a few boundary points (see E9).

## Evidence

### E1. Environment [verified]

```
$ id
uid=1337(user) gid=100(users) groups=...,1(wheel),...
$ uname -r
6.18.44
$ ls -la /dev/fuse
crw-rw-rw- 1 root root 10, 229 Oct 11 02:06 /dev/fuse
$ command -v fusermount3 && ls -la /run/wrappers/bin/fusermount3
/run/wrappers/bin/fusermount3
-r-s--x--x 1 root root 70712 Oct 11 02:06 /run/wrappers/bin/fusermount3   (setuid; execs fuse-3.18.2-bin)
$ grep -i fuse /proc/filesystems
nodev	fuse
nodev	fusectl
$ unshare -Urm sh -c 'id; cat /proc/self/uid_map'
uid=0(root) gid=0(root) ...
         0       1337          1
$ cat /etc/fuse.conf | grep -v '^#'
mount_max = 1000                              # no user_allow_other
$ pkg-config --modversion fuse3               -> not found (no headers in /usr/include)
$ ls /nix/store/74l9w9lsqhy6c0li69yw78c2ppmla7zk-fuse-3.16.2-dev/include/fuse3/
fuse.h fuse_lowlevel.h fuse_common.h ...      # usable headers, libfuse3.so from 36a91rn5...-fuse-3.16.2
$ gcc --version -> gcc (GCC) 15.3.0 ; cmake --version -> 4.3.4 ; c++ --version -> clang version 21.1.8
$ ls /nix/store | grep -i lazy                 -> only lazygit/lazy_static/... (no lazyfs)
```

### E2. Trivial FUSE passthrough [verified]

`crash-tests/fuse-probe/pt.c` maps a mount to a root directory (134 lines, libfuse 3 high-level API). Built against the store's fuse 3.16.2 headers:

```
gcc -O2 -Wall -Wextra -std=gnu11 -o pt pt.c \
  -I/nix/store/74l9w9lsqhy6c0li69yw78c2ppmla7zk-fuse-3.16.2-dev/include \
  -L/nix/store/36a91rn5w65prs6whwqjffk9s495wjym-fuse-3.16.2/lib -lfuse3 \
  -Wl,-rpath,/nix/store/36a91rn5w65prs6whwqjffk9s495wjym-fuse-3.16.2/lib -lpthread
```

`fuse-probe/probe-passthrough.sh` output (`logs/fuse-passthrough-probe.log`):

```
== A: plain process, uid 1337, fusermount3 = /run/wrappers/bin/fusermount3
  mount: pt .../work/mnt-a fuse.pt rw,nosuid,nodev,relatime,user_id=1337,group_id=100
  write via mount: ok
  read via mount: hello-a
  underlying root-a: hello-a
  fusermount3 -u: ok
  daemon exit: 0
== B: inside unshare -Urm (user+mount namespace)
  mount: pt .../work/mnt-b fuse.pt rw,nosuid,nodev,relatime,user_id=0,group_id=0
  write via mount: ok
  read via mount: hello-b
  underlying root-b: hello-b
```

The "uid inside" line in the script prints the same value for both labels. That is a labelling mistake in my script, not a result. Inside the namespace, uid 0 maps to 1337 (`uid_map`). The `unshare` exit status (8) comes from the daemon's exit after SIGTERM; I did not investigate it.

### E3. LazyFS build [verified unless marked]

- Source: `curl -sSL -o lazyfs-main.tar.gz https://codeload.github.com/dsrhaslab/lazyfs/tar.gz/refs/heads/main` on 2026-10-11 11:08 UTC. sha256 `b44234ee4bdf50c3d3f0d6625e7eab74dcdaca3bae5dfb954f7276b82232a551`. The archive has no git metadata, so the commit is not recorded. Extracted to `crash-tests/untrusted/lazyfs/lazyfs-main/`.
- README requirements (`github.com/dsrhaslab/lazyfs` README, fetched 2026-10-11): C++17, CMake, g++ (tested with 9.4.0 and cmake 3.16.3), FUSE 3 development headers, and `user_allow_other` in `/etc/fuse.conf` for `allow_other`.
- `libs/libpcache` configure (FetchContent downloads toml11 v3.7.1 by git and spdlog 1.10.0 as an archive). With CMake 4.3.4 the first configure failed: `CMake Error at build/_deps/toml-src/CMakeLists.txt:1 (cmake_minimum_required): Compatibility with CMake < 3.5 has been removed`. The fix is `-DCMAKE_POLICY_VERSION_MINIMUM=3.5`, which is printed in the error text. No source was changed for this.
- Clang build (`c++` = clang 21.1.8), not kept in a log file: `lazyfs/src/lazyfs.cpp:1164:35: error: non-constant-expression cannot be narrowed from type 'off_t' ... to 'const int'`, the same at lines 1210 and 1449 ("3 warnings and 3 errors generated"). These are the only errors.
- g++ build (kept): `cmake -S libs/libpcache -B libs/libpcache/build -DCMAKE_CXX_COMPILER=g++ -DCMAKE_C_COMPILER=gcc -DCMAKE_BUILD_TYPE=Release -DCMAKE_POLICY_VERSION_MINIMUM=3.5` then `cmake --build ... -j4` (real 3m32s, includes the two FetchContent deps). `cmake -S lazyfs -B lazyfs/build -DCMAKE_CXX_COMPILER=g++ -DCMAKE_C_COMPILER=gcc -DLAZYFS_BUILD_TESTS=OFF -DCMAKE_BUILD_TYPE=Release -DCMAKE_EXE_LINKER_FLAGS="-Wl,-rpath,<fuse lib>:<pcache build>"`, then build (real 1m51s). Result: `lazyfs/build/lazyfs` built; `ldd` shows `libfuse3.so.3` (3.16.2) and `libpcache.so.0`. The g++ build has three `-Wnarrowing` warnings at the same three lines.
- Environment note: `LIBRARY_PATH` and `CPLUS_INCLUDE_PATH` pointed at the store's fuse headers and library during configure and build (`logs/lazyfs-configure.log`, `logs/lazyfs-build.log`).

### E4. LazyFS basic scenario [verified]

`lazyfs-probe/lazyfs-basic.sh` runs `lazyfs-inner.sh` inside `unshare -Urm`. The command is `-o allow_other -o modules=subdir -o subdir=<root> -s -f`, the same options as LazyFS's `scripts/mount-lazyfs.sh`. Output (`logs/lazyfs-basic.log`):

```
uid inside namespace: 0
lazyfs ... mounted: lazyfs .../mnt fuse.lazyfs rw,...,user_id=0,group_id=0,allow_other
== S1: fsync(A), write(B) without fsync, then lazyfs::clear-cache
  before clear-cache: via mount='AB' on disk='A'
  after  clear-cache: via mount='A' on disk='A'
== S2: fsync(A), write(C) without fsync, lazyfs::cache-checkpoint, then lazyfs::clear-cache
  before checkpoint:  via mount='AC' on disk='A'
  after  checkpoint:  via mount='AC' on disk='AC'
  after  clear-cache: via mount='AC' on disk='AC'
== S3: fsync(A), write(B) without fsync, kill -9 the LazyFS daemon (no FIFO command)
  before kill: via mount='AB' on disk='A'
  daemon killed (status 137)
  after  kill: on disk='A'
```

So unsynced data is dropped by `clear-cache` (S1) and by a daemon kill (S3), and `cache-checkpoint` writes it to the real file system without fsync, where it survives a later `clear-cache` (S2). This matches the README's description.

Outside the namespace, the same daemon refuses to start [verified, `logs/lazyfs-nons-allow_other.log`]:

```
/nix/store/jpkz...-fuse-3.18.2-bin/bin/fusermount3: option allow_other only allowed if 'user_allow_other' is set in /etc/fuse.conf
```

Not tested: `torn-op` / `torn-seq` injection, the FIFO `crash` command, and running redb or the event store through a LazyFS mount.

### E5. redb 4.3.0 backend interface [verified, source]

`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/redb-4.3.0/`:

- `src/db.rs:74`: `pub trait StorageBackend: 'static + Debug + Send + Sync`
- Required: `fn len(&self) -> Result<u64, io::Error>` (76), `fn read(&self, offset: u64, out: &mut [u8])` (81), `fn set_len(&self, len: u64)` (86), `fn sync_data(&self)` (89), `fn write(&self, offset: u64, data: &[u8])` (92).
- Optional: `fn close(&self)` (101, default `Ok`); `try_lock_range`, `try_lock_shared_range`, `lock_range`, `lock_shared_range`, `unlock_range`, `query_lock_range` (113 onward, default `BackendError::Unsupported`).
- Construction: `Database::builder().create_with_backend(backend)` (`db.rs:2255`); `Builder::set_repair_callback(&mut self, impl Fn(&mut RepairSession))` (`db.rs:2130`); `Database::check_integrity(&mut self) -> Result<bool, DatabaseError>` (`db.rs:874`); `Database::builder()` (`db.rs:1751`).
- Durability: `Durability::{None, Immediate}` (`transactions.rs:378`). The default is `Immediate`, so `commit` returns after `sync_data`.
- redb's own crash test backend (`db.rs:3381`, `CrashBackend`) makes calls after a "dead" flag silently succeed without effect. My post-cut behaviour follows it.
- redb has no required dependencies. On this platform cargo compiled `libc 0.2.190` as well.

### E6. Prototype build and tests [verified]

Crate: `crash-tests/redb-crash/` (`redb = "=4.3.0"`, own `[workspace]`). Toolchain: `RUSTUP_TOOLCHAIN=1.97.0`, `rustc 1.97.1`, `cargo 1.97.0`, path from `rustup which --toolchain 1.97.0 cargo`. Target dir: `CARGO_TARGET_DIR=/home/user/work/research-round2/crash-tests/target`. Cargo resolved offline from the local registry cache.

```
cargo build --offline --all-targets         # lib, tests, sweep: builds, 1 unused-import warning, fixed
cargo test --offline                        # debug
  faultfs_semantics: 6 passed                # drop, keep, sync before/after, torn prefix, lying sync, inert after cut
  replay: clean_run_reopens_with_every_transaction ok; corpus_replays_with_recorded_verdicts ok (20 s)
  determinism_probe: prints the trace and verdict for one plan (no assertions)
cargo test --offline --release              # same tests: all pass
cargo build --offline --release --bin sweep
```

Model (`src/workload.rs`): 50 transactions. Transaction b inserts `claims(b)` event rows and their author-index rows (`claims` is 8–32, and 120 for every tenth transaction), deletes the batch from five transactions earlier (so pages are freed and reused), and updates two meta rows. The model for n transactions is the map after the first n. The verifier compares a full dump of all three tables against models `acked` and `acked+1`.

Fault model (`src/faultfs.rs`):
- `write`/`set_len` wait in `pending` until `sync_data` completes.
- A cut at storage call k keeps `durable` plus the pending operations chosen by `Survive`: `Drop` (none, like LazyFS `clear-cache`), `Keep` (all, in order, like a kernel write-back), `Random` (each independently), `Reorder` (a random subset in random order).
- `Tear`: `Off` (the in-flight write leaves nothing), `Byte` (a random byte prefix, 0 to full), `Sector` (a prefix of whole 512-byte sectors). Only applies when the cut lands on a write.
- A sync cut is either "sync lost" (with any survival policy) or "sync completed, then power lost".
- After the cut, calls succeed without effect (as in redb's own CrashBackend). The harness counts a commit as acknowledged only if it returned before the cut.

Oracle (`src/harness.rs`): reopen the image; exactly one of `ok_acked` (state = model[acked]), `ok_acked_plus_one` (state = model[acked+1]) must hold. Other classes: `lost_committed`, `partial`, `future`, `open_error` (after acknowledged commits), `integrity` (`check_integrity` false or failing), `post_recovery` (a further commit and clean reopen must work), `no_crash_error`, `panic`. `open_error_pre_commit` (nothing acknowledged yet) is reported, not counted as failure.

### E7. Faithful sweep [verified]

```
$ target/release/sweep --threads 4 --out results
clean run (no cut): verdict=ok_acked acked=50 storage_calls=687
crash points (one per storage call): 687 {"set_len": 2, "sync": 60, "write": 625}
runs: 7808 (control: false)
verdicts: {"ok_acked": 7621, "ok_acked_plus_one": 167, "open_error_pre_commit": 20}
  set_len  ok_acked 4 / ok_acked_plus_one 4
  sync     ok_acked 196 / ok_acked_plus_one 100 / open_error_pre_commit 4
  write    ok_acked 7421 / ok_acked_plus_one 63 / open_error_pre_commit 16
reopens that ran redb repair: 7759 runs, 26723 callbacks
failing runs: 0 at 0 distinct crash points
real 0m42.661s   (1 thread: 40.0 s, same machine)
```

Counts: 687 crash points (≥200 required). The `ok_acked_plus_one` rows show the in-flight commit became durable before the cut, which is the boundary case the oracle must allow. There were no `partial` verdicts.

Pre-commit open errors: 20 runs, all with `acked=0`, at storage calls 2 (8 variants), 3 (2), 4 (8), 5 (2). All 20 report `open: I/O error: Not a redb database: magic number mismatch` (`results/sweep-faithful.tsv`, column 10). Torn or zero-filled first writes leave bytes that redb will not open. Nothing acknowledged is lost, but the store cannot start without handling.

### E8. Control: lying `sync_data` [verified]

`sweep --lying-sync` makes `sync_data` return `Ok` without making data durable. A correct oracle must fail these runs.

```
verdicts: {"lost_committed": 1204, "ok_acked": 1894, "ok_acked_plus_one": 113,
           "open_error": 4522, "open_error_pre_commit": 70, "panic": 5}
failing runs: 5731 (1204 lost_committed + 4522 open_error + 5 panic; counted from the TSV)
```

The control is detected, so the faithful zero is not an artefact of a blind oracle. The 5 `panic` runs are redb panicking while reopening images it cannot make sense of, for example "entered unreachable code" and `Utf8Error`. Those images come from the lying control, not from a faithful fsync. They are still a robustness signal for redb. I have not checked whether these panics are already reported upstream.

The `partial` class was never produced, by either the faithful or the control sweep. The `partial` branch of the oracle is therefore not demonstrated by a control. [verified: `grep -c partial` gives 0 in both TSVs]

### E9. Build-profile difference [verified behaviour; cause not found]

The same plan gives different verdicts in debug and release. Event 149, `Keep`, `Byte`, seed `12777398887419732391`:

```
debug:   ok_acked         acked=11 repairs=4   (200 of 200 runs, same process)
release: ok_acked_plus_one acked=11 repairs=3   (200 of 200 runs)
```

The storage-call traces are identical in both profiles: 687 calls, same counts, same offsets and lengths around event 149 (`tests/determinism_probe.rs` output). Four rows differ across the two full faithful sweeps (events 149 and 506, `Keep` with `Byte` or `Sector`), and all four are `ok_acked_plus_one` in release and `ok_acked` in debug. Both are valid prefixes, so the oracle passes both. Not investigated further: whether the image bytes differ between profiles, or redb's repair choice differs. This is worth a separate look before relying on `ok_acked_plus_one` boundary verdicts as exact.

### E10. Debug faithful sweep [verified]

```
$ target/debug/sweep --threads 4 --out results-debug
verdicts: {"ok_acked": 7625, "ok_acked_plus_one": 163, "open_error_pre_commit": 20}
failing runs: 0 (real 13m47s, debug build, shared machine)
```

### E11. Static corpus and replay [verified]

`corpus/redb-crash-corpus.tsv` has 195 lines, one crash point each: `name lying_sync event survive tear sync_completed seed expect`. Contents:
- 167 faithful `ok_acked_plus_one` rows, expect `ok` (either valid prefix). The in-flight-commit boundary points.
- 20 faithful `open_error_pre_commit` rows, expect exact.
- 8 control rows (`lying_sync=1`): 2 each of `lost_committed`, `open_error`, `panic`, and `ok_acked_plus_one`, expect exact.

`tests/replay.rs` reruns each row through `run_one` and checks the class. Passing in debug and release. The `ok` class accepts either valid prefix. That is the change E9 made necessary: the first version pinned the exact prefix, and the release-built corpus then failed in debug.

### E12. Harness bugs found and fixed during the run [verified]

The first sweep reported `post_recovery` for every run, including the clean run. Cause: the post-recovery check reopened the workload's original image, not the reopened one. Fixed by passing the reopened backend. Second, a panic inside reopen (in the control) killed the sweep (exit 101). Fixed by wrapping reopen in `catch_unwind` and reporting it as `panic`. The numbers above come from the fixed harness only.

## Options

| Option | Portable | What it exercises | Cost / blockers here | Verdict |
| --- | --- | --- | --- | --- |
| In-process `FaultFs` on redb (this work) | Yes: any OS with redb | Every storage call; four survival policies; torn writes; no kernel or FUSE | 40 s release sweep; 14 min debug sweep | **Use as CI gate** (release build) |
| LazyFS over FUSE | Linux only; needs FUSE, a user namespace (or `user_allow_other`), g++, network for FetchContent | Real file-system page cache: clear-cache, checkpoint, process kill | Build about 5 min; `allow_other` needs `unshare -Ur` | **Optional Linux job** for a few corpus points through the real binary |
| SIGKILL test (S6) | Yes | Process crash only; OS page cache survives | Cheap | Keep, but it does not test unsynced writes |
| VM power cut (qemu, kill the VM) | Needs a VM host | Real device write-back, torn sectors | Not set up | Final sign-off for the engine, not a unit test |
| dm-flakey / dm-log-writes | Needs root | Block-level loss and reordering | Needs root | Not tested |

## Recommendation

1. **Gate: the in-process fault model.** Add a `faultfs` module and a `workload` for the event store under `pkgs/id` as a test-only dependency, plus `cargo test` for the portable pieces (fault semantics, clean run, replay). Run the full sweep in release mode on a nightly or pre-release job (about 40 s here), and the control sweep alongside it. Reasons: deterministic, no privileges, runs on every platform redb runs on, and it found no failures for redb 4.3.0 on this workload.
2. **Optional Linux job: LazyFS.** Build with g++, not clang, and pin the CMake policy flag. Run a small set of corpus points through the real store binary under `unshare -Ur`. Reasons: real file-system page-cache behaviour that the in-process model does not have. Not a gate: it needs FUSE, namespaces, and network for FetchContent.
3. **Corpus as a static replay list.** One line per crash point, checked in beside the test. Every sweep failure gets appended with the fix that resolves it. Expected classes: an exact verdict for failures and controls, `ok` for faithful points. Keep the lying-sync control rows, so the oracle is proven to still detect lost durability. Add a header that records the workload's storage-call count and a hash of the call trace (kind, offset, length). If the workload changes, the replay fails loudly instead of testing the wrong crash points.
4. **Design items for task 02 (not product changes yet).**
   - Pre-commit open failure: create the store file under a temporary name, make the first commit, then rename it into place (and fsync the directory). Alternatively, detect "file has no committed transaction" and recreate it. E7 shows a crash there is otherwise a startup failure.
   - Open path: a test-only constructor that takes a `StorageBackend` (redb exposes `create_with_backend`), so the real store can be run under the fault model.
5. **What would change this.** If the event store adds multiple files, renames, or directory operations beyond one redb file, the redb-level model is not enough, and LazyFS (or a VM cut) becomes necessary for those paths. If the E9 profile difference turns out to be a redb recovery bug, the engine decision needs a second look before the boundary verdicts are trusted. If CI runners lack unprivileged user namespaces, the LazyFS job cannot run there.

## Not verified

- Real power cuts (VM or hardware). Torn writes here are a model, not device behaviour. [not checked]
- LazyFS with redb or the event store under the mount. Only the basic scenario ran. [not checked]
- LazyFS `torn-op`, `torn-seq`, and the FIFO `crash` command. [not checked]
- Clang versus g++ behaviour beyond the build errors. I only saw the build failure, not a runtime difference. [not checked]
- Cause of the debug/release difference (E9). Image bytes were not compared between profiles. [not checked]
- Whether the 5 redb reopen panics in the control are known upstream. [not checked]
- The `partial` class: never observed, and no control produces it. The oracle's partial branch is untested. [not checked]
- Directory fsync and rename semantics (relevant to the temp-file-then-rename pattern). [not checked]
- Whether unprivileged user namespaces are available on the CI runners. Checked only on this host. [not checked]
- The event store itself: only a synthetic redb workload was run, not the task-02 schema or its code. [not checked]
- Single-thread versus four-thread equality beyond one row: row 149 gave the same verdict in a single-thread release run. [verified for that row only]
- Turso, libsql, and other engines: not measured. [not checked]
- Time budget: the run exceeded the 60–90 minute brief. The first work in this folder is at 11:02 local, the tarball at 11:08 local, and the last log at 12:51 local, with the debug sweep (14 min) and this report after that. [verified from file timestamps]

## Files

- `fuse-probe/`: `pt.c`, `probe-passthrough.sh`. Logs: `logs/fuse-passthrough-probe.log`.
- `lazyfs-probe/`: `lazyfs-basic.sh`, `lazyfs-inner.sh`. Logs: `logs/lazyfs-basic.log`, `logs/lazyfs-nons-allow_other.log`, `logs/lazyfs-*build*.log`, `logs/lazyfs-pcache-configure.log`, `logs/lazyfs-configure.log`.
- `downloads/lazyfs/lazyfs-main.tar.gz` (archive, sha256 above). `untrusted/lazyfs/lazyfs-main/`: extracted source and build trees (`libs/libpcache/build`, `lazyfs/build`).
- `redb-crash/`: `src/{lib,faultfs,workload,harness}.rs`, `src/bin/sweep.rs`, `tests/{faultfs_semantics,replay,determinism_probe}.rs`, `corpus/redb-crash-corpus.tsv`, `Cargo.toml`, `Cargo.lock`.
- `redb-crash/results/`: `sweep-faithful.{tsv,stdout}`, `sweep-lying-sync-control.{tsv,stdout}`, `failures-*.tsv`. `redb-crash/results-debug/` and `results-debug.stdout`: debug faithful sweep. `redb-crash/results-1thread/`: single-thread release sweep (row 149).
- `target/`: shared build directory for this work (`CARGO_TARGET_DIR`).

Commands used for the sweeps:
```
cd crash-tests/redb-crash
/home/user/work/research-round2/crash-tests/target/release/sweep --threads 4 --out results
/home/user/work/research-round2/crash-tests/target/release/sweep --lying-sync --threads 4 --out results
/home/user/work/research-round2/crash-tests/target/debug/sweep --threads 4 --out results-debug
```
