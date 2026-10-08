//! The native tier: a world program as a statically linked executable.
//!
//! A Roc app built for the platform's `x64musl` target is a worker binary
//! (`examples/roc-world/host/src/native_worker.zig`) that serves the same
//! program ABI over stdin/stdout frames. The host spawns it with OS
//! hardening and drives it exactly like a Wasm guest:
//!
//! - `no_new_privs`, rlimits (address space, CPU, file size, processes,
//!   core) and a **seccomp allowlist** — no `open*`, no sockets, no
//!   `clone`/`execve`/`ptrace`, and `mmap`/`mprotect` may not make memory
//!   executable. An unknown syscall kills the process.
//! - every call runs under a wall-clock deadline; a crash, timeout, protocol
//!   violation or oversize reply poisons the program, exactly like a Wasm
//!   trap.
//!
//! Unlike Wasm, the guarantee is the OS sandbox, not an import-free format:
//! native modules are admin-installed and opt-in (`--world-native`), and are
//! never offered for peer download.

use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};

use crate::world::WorldEvent;

/// Resource bounds for one native worker.
#[derive(Clone, Copy, Debug)]
pub struct NativeLimits {
    /// Address-space cap (`RLIMIT_AS`).
    pub memory_bytes: u64,
    /// CPU cap (`RLIMIT_CPU`).
    pub cpu_seconds: u64,
    /// Wall-clock budget per call.
    pub call_deadline: Duration,
}

impl Default for NativeLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 256 * 1024 * 1024,
            cpu_seconds: 10,
            // Generous: the host machine may be loaded while a world is
            // starting; correctness does not depend on snappy guests.
            call_deadline: Duration::from_secs(10),
        }
    }
}

const OP_INIT: u8 = 1;
const OP_UPDATE: u8 = 2;
const OP_VIEW: u8 = 3;
const OP_RECORDS: u8 = 4;
const OP_WANTS: u8 = 5;
const OP_SNAPSHOT: u8 = 6;
const OP_RESTORE: u8 = 7;

/// Whether `bytes` is a native module (a statically linked ELF) rather than
/// a Wasm module.
#[must_use]
pub fn is_native_module(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x7fELF")
}

enum Request {
    /// `op`, payload.
    Call(u8, Vec<u8>),
}

/// One running worker process plus its driver thread.
struct Worker {
    sender: mpsc::Sender<Request>,
    replies: mpsc::Receiver<Result<Vec<u8>, String>>,
    child: Child,
}

impl Worker {
    /// Spawn `path` hardened, then run its `init(seed)`.
    fn new(path: &Path, seed: u64, limits: &NativeLimits) -> Result<Self> {
        let mut command = Command::new(path);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let _sandbox_root = harden(&mut command, path, limits)?;
        let mut child = command
            .spawn()
            .with_context(|| format!("spawn native world worker {}", path.display()))?;
        let stdin = child.stdin.take().context("worker stdin")?;
        let stdout = child.stdout.take().context("worker stdout")?;
        let (sender, requests) = mpsc::channel::<Request>();
        let (reply_sender, replies) = mpsc::channel::<Result<Vec<u8>, String>>();
        std::thread::spawn(move || {
            drive_worker(stdin, stdout, &requests, &reply_sender);
        });
        let worker = Self {
            sender,
            replies,
            child,
        };
        worker
            .call(OP_INIT, seed.to_be_bytes().to_vec(), limits)
            .context("worker init failed")?;
        Ok(worker)
    }

    fn call(&self, op: u8, payload: Vec<u8>, limits: &NativeLimits) -> Result<Vec<u8>> {
        let payload_len = payload.len();
        self.sender
            .send(Request::Call(op, payload))
            .map_err(|_| anyhow::anyhow!("native worker is gone"))?;
        match self
            .replies
            .recv_timeout(limits.call_deadline.max(Duration::from_millis(1)))
        {
            Ok(Ok(bytes)) => {
                ensure!(
                    bytes.len() <= crate::world_session::MAX_FRAME_BYTES,
                    "worker reply exceeds the frame limit"
                );
                Ok(bytes)
            }
            Ok(Err(message)) => bail!("{message}"),
            Err(_) => bail!("native worker did not answer a {payload_len}-byte call in time"),
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Owns the child's pipes: one request in, one reply out. A worker that
/// dies or misbehaves reports an error once and then exits.
fn drive_worker(
    mut stdin: ChildStdin,
    mut stdout: ChildStdout,
    requests: &mpsc::Receiver<Request>,
    replies: &mpsc::Sender<Result<Vec<u8>, String>>,
) {
    let fail = |message: String| {
        let _ = replies.send(Err(message));
    };
    loop {
        let Ok(Request::Call(op, payload)) = requests.recv() else {
            return;
        };
        let mut frame = Vec::with_capacity(5 + payload.len());
        let total: u32 = 1 + u32::try_from(payload.len()).unwrap_or(u32::MAX);
        frame.extend_from_slice(&total.to_be_bytes());
        frame.push(op);
        frame.extend_from_slice(&payload);
        if frame[4..].len() != payload.len() + 1 || write_all(&mut stdin, &frame).is_err() {
            fail("native worker stopped accepting calls".to_owned());
            return;
        }
        let mut length = [0_u8; 4];
        if read_exact(&mut stdout, &mut length).is_err() {
            fail("native worker exited".to_owned());
            return;
        }
        let total = u32::from_be_bytes(length) as usize;
        if total < 1 || total - 1 > crate::world_session::MAX_FRAME_BYTES {
            fail("native worker sent an invalid reply".to_owned());
            return;
        }
        let mut body = vec![0_u8; total];
        if read_exact(&mut stdout, &mut body).is_err() {
            fail("native worker exited mid-reply".to_owned());
            return;
        }
        let (ok, payload) = (body[0], &body[1..]);
        let _ = replies.send(if ok == 1 {
            Ok(payload.to_vec())
        } else {
            Err("native worker refused the call".to_owned())
        });
    }
}

fn read_exact(stdout: &mut ChildStdout, buffer: &mut [u8]) -> Result<()> {
    stdout.read_exact(buffer).context("read from native worker")
}

fn write_all(stdin: &mut ChildStdin, buffer: &[u8]) -> Result<()> {
    stdin.write_all(buffer).context("write to native worker")
}

/// A native world program: the server-side half of the worker protocol,
/// implementing the same [`crate::world::WorldProgram`] trait as a Wasm
/// guest.
pub struct NativeProgram {
    worker: Option<Worker>,
    limits: NativeLimits,
    /// The worker's exit state, reported once and then sticky.
    dead: Option<String>,
}

impl std::fmt::Debug for NativeProgram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeProgram")
            .field("alive", &self.worker.is_some())
            .finish_non_exhaustive()
    }
}

impl NativeProgram {
    /// Spawn the worker for `path` and initialize it with `seed`; with
    /// `snapshot`, the worker restores that state instead of starting fresh.
    ///
    /// # Errors
    ///
    /// Spawn, sandbox installation, init or restore failures.
    pub fn new(
        path: &Path,
        seed: u64,
        limits: NativeLimits,
        snapshot: Option<&str>,
    ) -> Result<Self> {
        let worker = Worker::new(path, seed, &limits)?;
        let mut program = Self {
            worker: Some(worker),
            limits,
            dead: None,
        };
        if let Some(snapshot) = snapshot {
            program.call(OP_RESTORE, snapshot.as_bytes().to_vec())?;
        }
        Ok(program)
    }

    /// Deliver an event as `participant`; host events arrive as 0.
    fn deliver(&mut self, participant: u64, bytes: &[u8]) -> Result<()> {
        let _ = std::str::from_utf8(bytes).context("native input must be UTF-8")?;
        let participant = participant.min(u32::MAX as u64) as u32;
        let mut payload = participant.to_be_bytes().to_vec();
        payload.extend_from_slice(bytes);
        self.call(OP_UPDATE, payload)?;
        Ok(())
    }

    /// One round trip; a sticky failure poisons the program like a Wasm
    /// trap.
    fn call(&mut self, op: u8, payload: Vec<u8>) -> Result<Vec<u8>> {
        if let Some(dead) = &self.dead {
            bail!("native worker is dead: {dead}");
        }
        let Some(worker) = self.worker.as_mut() else {
            self.dead = Some("worker is gone".to_owned());
            bail!("native worker is gone");
        };
        match worker.call(op, payload, &self.limits) {
            Ok(reply) => Ok(reply),
            Err(error) => {
                self.dead = Some(error.to_string());
                self.worker = None;
                Err(error)
            }
        }
    }

    fn text_call(&mut self, op: u8) -> Result<String> {
        let bytes = self.call(op, Vec::new())?;
        String::from_utf8(bytes).context("native worker sent non-UTF-8 text")
    }
}

impl crate::world::WorldProgram for NativeProgram {
    fn update(&mut self, event: &WorldEvent) -> Result<()> {
        match &event.kind {
            crate::world::WorldEventKind::Input(bytes) => {
                self.deliver(event.participant_id, bytes)?;
                Ok(())
            }
            // Host events are JSON text delivered as participant 0.
            crate::world::WorldEventKind::System { event } => {
                self.deliver(0, event.as_bytes())?;
                Ok(())
            }
            crate::world::WorldEventKind::Chat(_)
            | crate::world::WorldEventKind::ProgramInstalled { .. } => Ok(()),
        }
    }

    fn view(&mut self, _viewer: &crate::world::Participant) -> Result<Option<String>> {
        let bytes = self.call(OP_VIEW, Vec::new())?;
        Ok(Some(
            String::from_utf8(bytes).context("native view must be UTF-8")?,
        ))
    }

    fn records(&mut self) -> Result<Option<String>> {
        Ok(Some(self.text_call(OP_RECORDS)?).filter(|text| !text.is_empty()))
    }

    fn wants(&mut self) -> Result<Option<String>> {
        Ok(Some(self.text_call(OP_WANTS)?).filter(|text| !text.is_empty()))
    }

    fn snapshot(&mut self) -> Result<Option<String>> {
        Ok(Some(self.text_call(OP_SNAPSHOT)?).filter(|text| !text.is_empty()))
    }
}

/// Opens the worker binary, the only path the sandbox may execute, and
/// installs the hardening hook. The returned descriptor must outlive the
/// spawn: the hook refers to it by number.
#[cfg(target_os = "linux")]
fn harden(command: &mut Command, path: &Path, limits: &NativeLimits) -> Result<OwnedFd> {
    use std::os::fd::{AsRawFd, FromRawFd};

    // A C string: raw syscalls need NUL-terminated paths, and
    // `as_encoded_bytes` is not NUL-terminated (this exact bug made openat
    // fail with ENOENT on an existing file).
    let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
        .context("worker path contains a NUL")?;
    // Safety: `c_path` is NUL-terminated and outlives the call.
    #[allow(unsafe_code)]
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat,
            libc::AT_FDCWD,
            c_path.as_ptr(),
            libc::O_PATH | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).context("open native worker for sandboxing");
    }
    // Safety: `openat` just returned this descriptor and nothing else owns it.
    #[allow(unsafe_code)]
    let root = unsafe { OwnedFd::from_raw_fd(fd as i32) };
    let (memory_bytes, cpu_seconds, worker_fd) =
        (limits.memory_bytes, limits.cpu_seconds, root.as_raw_fd());
    // Safety: `pre_exec` runs between fork and exec; the hook calls only
    // async-signal-safe functions, see `apply_native_sandbox`.
    #[allow(unsafe_code)]
    unsafe {
        command.pre_exec(move || apply_native_sandbox(memory_bytes, cpu_seconds, worker_fd));
    }
    Ok(root)
}

/// Native workers run only where the sandbox exists; elsewhere they are refused.
#[cfg(not(target_os = "linux"))]
fn harden(_command: &mut Command, _path: &Path, _limits: &NativeLimits) -> Result<OwnedFd> {
    bail!("the native tier requires Linux")
}

/// Hardening applied to the worker between fork and exec. Syscall arguments
/// are variadic, so literal widths are spelled out rather than inferred.
#[cfg(target_os = "linux")]
#[allow(trivial_casts, trivial_numeric_casts, clippy::unnecessary_cast)]
fn apply_native_sandbox(
    memory_bytes: u64,
    cpu_seconds: u64,
    worker_fd: i32,
) -> std::io::Result<()> {
    use libc::{PR_SET_DUMPABLE, PR_SET_NO_NEW_PRIVS, rlimit};

    // No gaining privileges via setuid binaries, and no core dumps.
    // Safety: between fork and exec; `prctl` is async-signal-safe.
    #[allow(unsafe_code)]
    unsafe {
        if libc::syscall(libc::SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::syscall(libc::SYS_prctl, PR_SET_DUMPABLE, 0, 0, 0, 0) != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    // The worker gets no filesystem access at all: Landlock denies every
    // path before the exec, so an allowed `execve` can never load another
    // binary. Kernels without Landlock refuse to spawn a native worker.
    // Safety: between fork and exec; the Landlock calls are
    // async-signal-safe.
    #[allow(unsafe_code)]
    unsafe {
        const LANDLOCK_CREATE_RULESET_VERSION: u64 = 1;
        let abi = libc::syscall(
            libc::SYS_landlock_create_ruleset,
            0 as *const libc::c_void,
            0 as libc::c_ulong,
            LANDLOCK_CREATE_RULESET_VERSION,
        );
        if abi < 1 {
            return Err(std::io::Error::from_raw_os_error(2001));
        }
        // ABI 1 knows bits 0..=10; ABI 2 adds TRUNCATE; ABI 3 adds IOCTL_DEV.
        let max_bit = if abi >= 3 {
            12
        } else if abi >= 2 {
            11
        } else {
            10
        };
        let handled: u64 = (1u64 << (max_bit + 1)) - 1;
        // struct landlock_ruleset_attr { handled_access_fs, handled_access_net,
        // scoped } — defined locally: libc does not ship Landlock types.
        #[repr(C)]
        struct LandlockRulesetAttr {
            handled_access_fs: u64,
            handled_access_net: u64,
            scoped: u64,
        }
        let ruleset_attr = LandlockRulesetAttr {
            handled_access_fs: handled,
            handled_access_net: 0,
            scoped: 0,
        };
        let ruleset = libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &ruleset_attr as *const LandlockRulesetAttr as *const libc::c_void,
            size_of::<LandlockRulesetAttr>() as libc::c_ulong,
            0 as libc::c_uint,
        );
        if ruleset < 0 {
            return Err(std::io::Error::from_raw_os_error(2002));
        }
        // struct landlock_path_beneath_attr { allowed_access, parent_fd }.
        #[repr(C)]
        struct LandlockPathBeneathAttr {
            allowed_access: u64,
            parent_fd: i32,
        }
        // The kernel refuses `execve` under EXECUTE alone here; READ_FILE is
        // granted with it (the binary is readable to the uid regardless).
        let rule = LandlockPathBeneathAttr {
            allowed_access: 1 | 4, // LANDLOCK_ACCESS_FS_EXECUTE | READ_FILE
            parent_fd: worker_fd,
        };
        if libc::syscall(
            libc::SYS_landlock_add_rule,
            ruleset as libc::c_ulong,
            1 as libc::c_uint, // LANDLOCK_RULE_PATH_BENEATH
            &rule as *const LandlockPathBeneathAttr as *const libc::c_void,
            0 as libc::c_uint,
        ) != 0
        {
            return Err(std::io::Error::from_raw_os_error(2004));
        }
        // No other rules: everything handled is denied, including `.`.
        if libc::syscall(
            libc::SYS_landlock_restrict_self,
            ruleset as libc::c_ulong,
            0 as libc::c_uint,
        ) != 0
        {
            return Err(std::io::Error::from_raw_os_error(2005));
        }
    }
    let cap = |resource: u32, limit: libc::rlim_t| {
        let limits = rlimit {
            rlim_cur: limit,
            rlim_max: limit,
        };
        // Safety: between fork and exec; `setrlimit` is async-signal-safe.
        #[allow(unsafe_code)]
        unsafe {
            if libc::setrlimit(resource, &limits) != 0 {
                return Err(std::io::Error::from_raw_os_error(1003 + resource as i32));
            }
        }
        Ok(())
    };
    cap(libc::RLIMIT_AS, memory_bytes)?;
    cap(libc::RLIMIT_CPU, cpu_seconds)?;
    cap(libc::RLIMIT_FSIZE, 8 * 1024 * 1024)?;
    cap(libc::RLIMIT_NPROC, 8)?;
    cap(libc::RLIMIT_CORE, 0)?;

    // The syscall allowlist. seccomp applies only after NO_NEW_PRIVS, and an
    // unlisted syscall kills the process rather than returning an error, so
    // a probe of the sandbox is as fatal as an attack on it.
    // The filter is prebuilt on a stack array: the post-fork hook must not
    // touch the heap (the parent's other threads may hold its locks).
    let (program_len, program) = native_seccomp_filter();
    let filter = libc::sock_fprog {
        len: u16::try_from(program_len).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "filter too long")
        })?,
        filter: program.as_ptr().cast_mut(),
    };
    // Safety: between fork and exec; `prctl(PR_SET_SECCOMP)` with a valid
    // filter is async-signal-safe.
    #[allow(unsafe_code)]
    unsafe {
        let rc = libc::syscall(
            libc::SYS_prctl,
            libc::PR_SET_SECCOMP as libc::c_ulong,
            libc::SECCOMP_MODE_FILTER as libc::c_ulong,
            &filter as *const libc::sock_fprog as *const libc::c_void,
            0 as libc::c_ulong,
            0 as libc::c_ulong,
        );
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
mod filter {
    /// BPF helper constants (linux/filter.h and linux/seccomp.h).
    pub const BPF_LD: u16 = 0x00;
    pub const BPF_W: u16 = 0x00;
    pub const BPF_ABS: u16 = 0x20;
    pub const BPF_JMP: u16 = 0x05;
    pub const BPF_JEQ: u16 = 0x10;
    pub const BPF_RET: u16 = 0x06;
    pub const BPF_ALU: u16 = 0x04;
    pub const BPF_AND: u16 = 0x50;
    pub const BPF_K: u16 = 0x00;

    pub const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
    /// `SECCOMP_RET_ALLOW`.
    pub const ALLOW: u32 = 0x7fff_0000;
    /// `SECCOMP_RET_KILL_PROCESS`.
    pub const KILL: u32 = 0x8000_0000;

    /// Syscall numbers on x86_64.
    pub mod nr {
        pub const READ: u32 = 0;
        pub const WRITE: u32 = 1;
        pub const CLOSE: u32 = 3;
        pub const FSTAT: u32 = 5;
        pub const LSEEK: u32 = 8;
        pub const MMAP: u32 = 9;
        pub const MPROTECT: u32 = 10;
        pub const MUNMAP: u32 = 11;
        pub const BRK: u32 = 12;
        pub const RT_SIGACTION: u32 = 13;
        pub const RT_SIGPROCMASK: u32 = 14;
        pub const IOCTL: u32 = 16;
        pub const PREAD64: u32 = 17;
        pub const PWRITE64: u32 = 18;
        pub const SCHED_YIELD: u32 = 24;
        pub const MREMAP: u32 = 25;
        pub const MADVISE: u32 = 28;
        pub const DUP: u32 = 32;
        pub const DUP2: u32 = 33;
        pub const NANOSLEEP: u32 = 35;
        pub const GETPID: u32 = 39;
        pub const EXIT: u32 = 60;
        pub const FCNTL: u32 = 72;
        pub const GETRLIMIT: u32 = 97;
        pub const GETRUSAGE: u32 = 98;
        pub const SYSINFO: u32 = 99;
        pub const ARCH_PRCTL: u32 = 158;
        pub const PRCTL: u32 = 157;
        pub const GETTID: u32 = 186;
        pub const FUTEX: u32 = 202;
        pub const SET_TID_ADDRESS: u32 = 218;
        pub const CLOCK_GETTIME: u32 = 228;
        pub const EXIT_GROUP: u32 = 231;
        pub const TGKILL: u32 = 234;
        pub const SET_ROBUST_LIST: u32 = 273;
        pub const SIGALTSTACK: u32 = 131;
        pub const SCHED_GETAFFINITY: u32 = 204;
        pub const GETRANDOM: u32 = 318;
        pub const EXECVE: u32 = 59;
        pub const EXECVEAT: u32 = 322;
        pub const MEMBARRIER: u32 = 324;
        pub const RSEQ: u32 = 386;
    }
}

#[cfg(target_os = "linux")]
/// Fixed-capacity: this runs inside the post-fork hook, where the heap
/// (shared with the parent's other threads) must not be touched.
fn native_seccomp_filter() -> (usize, [libc::sock_filter; 64]) {
    #[allow(clippy::wildcard_imports)]
    use filter::*;

    #[derive(Clone, Copy)]
    struct Ins {
        code: u16,
        jt: u8,
        jf: u8,
        k: u32,
    }

    const LD_ABS_ARCH: Ins = Ins {
        code: BPF_LD | BPF_W | BPF_ABS,
        jt: 0,
        jf: 0,
        k: 4,
    };
    const LD_ABS_NR: Ins = Ins {
        code: BPF_LD | BPF_W | BPF_ABS,
        jt: 0,
        jf: 0,
        k: 0,
    };
    const RET_KILL: Ins = Ins {
        code: BPF_RET | BPF_K,
        jt: 0,
        jf: 0,
        k: KILL,
    };
    const RET_ALLOW: Ins = Ins {
        code: BPF_RET | BPF_K,
        jt: 0,
        jf: 0,
        k: ALLOW,
    };

    const PLAIN: &[u32] = &[
        nr::READ,
        nr::WRITE,
        nr::CLOSE,
        nr::FSTAT,
        nr::LSEEK,
        nr::MUNMAP,
        nr::BRK,
        nr::RT_SIGACTION,
        nr::RT_SIGPROCMASK,
        nr::IOCTL,
        nr::PREAD64,
        nr::PWRITE64,
        nr::SCHED_YIELD,
        nr::MREMAP,
        nr::MADVISE,
        nr::DUP,
        nr::DUP2,
        nr::NANOSLEEP,
        nr::GETPID,
        nr::EXIT,
        nr::FCNTL,
        nr::GETRLIMIT,
        nr::GETRUSAGE,
        nr::SYSINFO,
        nr::ARCH_PRCTL,
        nr::PRCTL,
        nr::GETTID,
        nr::FUTEX,
        nr::SET_TID_ADDRESS,
        nr::CLOCK_GETTIME,
        nr::EXIT_GROUP,
        nr::TGKILL,
        nr::SET_ROBUST_LIST,
        nr::EXECVE,
        nr::EXECVEAT,
        nr::SIGALTSTACK,
        nr::SCHED_GETAFFINITY,
        nr::GETRANDOM,
        nr::MEMBARRIER,
        nr::RSEQ,
    ];
    // Checked for the executable bit in their protection argument.
    const EXEC_CHECKED: &[u32] = &[nr::MMAP, nr::MPROTECT];

    // Layout: [0] ld arch, [1] jeq arch, [2] ld nr,
    //         [3..3+PLAIN] jeq checks,
    //         [..+EXEC_CHECKED] jeq checks (jump into their blocks),
    //         [ret kill],
    //         [EXEC_CHECKED blocks: ld args2, and PROT_EXEC, jeq0->allow,
    //          ret kill],
    //         [ret allow]
    let plain = PLAIN.len();
    let checked = EXEC_CHECKED.len();
    let ret_kill_index = 3 + plain + checked;
    let ret_allow_index = ret_kill_index + 1 + checked * 4;

    let mut program = [Ins {
        code: 0,
        jt: 0,
        jf: 0,
        k: 0,
    }; 64];
    let mut len = 0usize;
    program[len] = LD_ABS_ARCH;
    len += 1;
    // Wrong architecture: kill.
    program[len] = Ins {
        code: BPF_JMP | BPF_JEQ | BPF_K,
        jt: 0,
        jf: (ret_kill_index - 2) as u8,
        k: AUDIT_ARCH_X86_64,
    };
    len += 1;
    program[len] = LD_ABS_NR;
    len += 1;
    for (index, syscall) in PLAIN.iter().enumerate() {
        program[len] = Ins {
            code: BPF_JMP | BPF_JEQ | BPF_K,
            // From this check, the ALLOW ret is past the remaining checks,
            // the ret-kill, and every checked block.
            jt: ((ret_allow_index - (3 + index)) - 1) as u8,
            jf: 0,
            k: *syscall,
        };
        len += 1;
    }
    for (index, syscall) in EXEC_CHECKED.iter().enumerate() {
        program[len] = Ins {
            code: BPF_JMP | BPF_JEQ | BPF_K,
            jt: ((ret_kill_index + 1 + index * 4) - (3 + plain + index) - 1) as u8,
            jf: 0,
            k: *syscall,
        };
        len += 1;
    }
    program[len] = RET_KILL;
    len += 1;
    for block in 0..EXEC_CHECKED.len() {
        // args[2] is the protection argument of both mmap and mprotect; only
        // non-executable mappings may be made. The block is [ld, and, jeq,
        // ret kill]; its zero-jump must reach the final ALLOW.
        let ld_index = ret_kill_index + 1 + block * 4;
        let jeq_index = ld_index + 2;
        program[len] = Ins {
            code: BPF_LD | BPF_W | BPF_ABS,
            jt: 0,
            jf: 0,
            k: 16 + 16,
        };
        len += 1;
        program[len] = Ins {
            code: BPF_ALU | BPF_AND | BPF_K,
            jt: 0,
            jf: 0,
            k: 0x1000, // PROT_EXEC
        };
        len += 1;
        program[len] = Ins {
            code: BPF_JMP | BPF_JEQ | BPF_K,
            jt: (ret_allow_index - jeq_index - 1) as u8,
            jf: 0,
            k: 0,
        };
        len += 1;
        program[len] = RET_KILL;
        len += 1;
    }
    program[len] = RET_ALLOW;
    len += 1;

    let mut out = [libc::sock_filter {
        code: 0,
        jt: 0,
        jf: 0,
        k: 0,
    }; 64];
    for (out, ins) in out.iter_mut().zip(program.iter()) {
        *out = libc::sock_filter {
            code: ins.code,
            jt: ins.jt,
            jf: ins.jf,
            k: ins.k,
        };
    }
    (len, out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::world::{WorldEvent, WorldEventKind};
    use std::path::PathBuf;

    fn counter_path() -> Option<PathBuf> {
        let path = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/roc-counter/counter-native"
        ));
        path.is_file().then_some(path)
    }

    fn input_event(sequence: u64, participant: u64, text: &str) -> WorldEvent {
        WorldEvent {
            sequence,
            participant_id: participant,
            kind: WorldEventKind::Input(text.as_bytes().to_vec()),
        }
    }

    #[test]
    fn the_native_counter_runs_as_a_world_program() {
        let Some(path) = counter_path() else {
            eprintln!("skipping: no native counter built");
            return;
        };
        let mut program = NativeProgram::new(&path, 7, NativeLimits::default(), None).unwrap();
        assert_eq!(
            crate::world::WorldProgram::view(
                &mut program,
                &crate::world::Participant {
                    id: 1,
                    display_name: "ann".to_owned()
                }
            )
            .unwrap()
            .unwrap(),
            "count=0"
        );
        crate::world::WorldProgram::update(&mut program, &input_event(1, 1, "inc")).unwrap();
        assert_eq!(
            crate::world::WorldProgram::view(
                &mut program,
                &crate::world::Participant {
                    id: 1,
                    display_name: "ann".to_owned()
                }
            )
            .unwrap()
            .unwrap(),
            "count=1"
        );
        assert_eq!(
            crate::world::WorldProgram::records(&mut program)
                .unwrap()
                .unwrap(),
            r#"{"count":1}"#
        );
        // Wants (the counter uses none) and a snapshot round trip.
        assert_eq!(
            crate::world::WorldProgram::wants(&mut program)
                .unwrap()
                .unwrap(),
            "{}"
        );
        let snapshot = crate::world::WorldProgram::snapshot(&mut program)
            .unwrap()
            .unwrap();
        assert_eq!(snapshot, "1");
        let mut restored =
            NativeProgram::new(&path, 7, NativeLimits::default(), Some(&snapshot)).unwrap();
        crate::world::WorldProgram::update(&mut restored, &input_event(2, 1, "inc")).unwrap();
        assert_eq!(
            crate::world::WorldProgram::view(
                &mut restored,
                &crate::world::Participant {
                    id: 1,
                    display_name: "ann".to_owned()
                }
            )
            .unwrap()
            .unwrap(),
            "count=2"
        );
    }

    #[test]
    fn actor_call_order_is_safe() {
        let Some(path) = counter_path() else {
            eprintln!("skipping: no native counter built");
            return;
        };
        let mut program = NativeProgram::new(&path, 7, NativeLimits::default(), None).unwrap();
        // The actor's install sequence: records, then wants.
        let _ = crate::world::WorldProgram::records(&mut program).unwrap();
        let _ = crate::world::WorldProgram::wants(&mut program).unwrap();
        // Then the input, the post-update records, and the view.
        crate::world::WorldProgram::update(&mut program, &input_event(1, 1, "inc")).unwrap();
        let _ = crate::world::WorldProgram::records(&mut program).unwrap();
        assert_eq!(
            crate::world::WorldProgram::view(
                &mut program,
                &crate::world::Participant {
                    id: 1,
                    display_name: "ann".to_owned()
                }
            )
            .unwrap()
            .unwrap(),
            "count=1"
        );
    }

    #[test]
    fn a_dead_native_worker_poisons_the_program() {
        let Some(path) = counter_path() else {
            eprintln!("skipping: no native counter built");
            return;
        };
        let mut program = NativeProgram::new(&path, 7, NativeLimits::default(), None).unwrap();
        // The counter ignores unknown input, but a poisoned program (killed
        // worker) refuses everything afterwards. Kill the worker directly.
        program.worker.take();
        let error = crate::world::WorldProgram::update(&mut program, &input_event(1, 1, "inc"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("gone"), "{error}");
    }

    /// A binary that pokes at exactly what the sandbox forbids. Built with
    /// zig when available; the assertions hold only on Linux.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_seccomp_allowlist_kills_escaping_workers() {
        let Some(zig) = which_zig() else {
            eprintln!("skipping: no zig toolchain");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("outlaw.c");
        std::fs::write(
            &source,
            r#"
#include <fcntl.h>
#include <sys/socket.h>
#include <unistd.h>
int main() {
    int fd = open("/etc/passwd", O_RDONLY);
    (void)fd;
    int s = socket(AF_INET, SOCK_STREAM, 0);
    (void)s;
    return fd >= 0 ? 1 : 0;
}
"#,
        )
        .unwrap();
        let outlaw = dir.path().join("outlaw");
        let status = Command::new(&zig)
            .args([
                "cc",
                "-target",
                "x86_64-linux-musl",
                "-static",
                source.to_str().unwrap(),
                "-o",
                outlaw.to_str().unwrap(),
            ])
            .status()
            .unwrap();
        assert!(status.success(), "zig cc failed");

        // The unhardened binary runs; under the sandbox it is killed for
        // touching `open` (SIGSYS, or death of any kind — never success).
        let plain = Command::new(&outlaw).status().unwrap();
        assert!(plain.success() || plain.code() == Some(1));
        let limits = NativeLimits::default();
        let worker = Worker::spawn_for_test(&outlaw, &limits);
        let reply = worker.call_for_test();
        assert!(reply.is_err(), "an outlaw survived the sandbox: {reply:?}");

        // A benign worker still runs under the same filter.
        let Some(path) = counter_path() else {
            eprintln!("skipping: no native counter built");
            return;
        };
        let program = NativeProgram::new(&path, 7, NativeLimits::default(), None).unwrap();
        drop(program);
    }

    #[cfg(target_os = "linux")]
    fn which_zig() -> Option<String> {
        Command::new("zig")
            .arg("version")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|_| "zig".to_owned())
    }

    #[cfg(target_os = "linux")]
    impl Worker {
        fn spawn_for_test(path: &Path, limits: &NativeLimits) -> Self {
            // The test path spawns without an INIT handshake.
            let mut command = Command::new(path);
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            let _sandbox_root = harden(&mut command, path, limits).unwrap();
            let mut child = command.spawn().unwrap();
            let stdin = child.stdin.take().unwrap();
            let stdout = child.stdout.take().unwrap();
            let (sender, requests) = mpsc::channel::<Request>();
            let (reply_sender, replies) = mpsc::channel::<Result<Vec<u8>, String>>();
            std::thread::spawn(move || {
                drive_worker(stdin, stdout, &requests, &reply_sender);
            });
            Self {
                sender,
                replies,
                child,
            }
        }

        fn call_for_test(&self) -> Result<Vec<u8>, String> {
            self.sender
                .send(Request::Call(2, Vec::new()))
                .map_err(|_| "worker gone".to_owned())?;
            self.replies
                .recv_timeout(Duration::from_secs(2))
                .map_err(|error| format!("timeout: {error}"))?
        }
    }
}
