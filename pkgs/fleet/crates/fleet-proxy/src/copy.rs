//! `fleet proxy copy` — any-to-any file copy with SCP-style addressing
//! on both sides (developing-today/code#10355, requirements 1 and 2).
//!
//! Each side is either a local path or `name:path` on a named remote.
//! The parse follows scp: a colon whose prefix is non-empty and holds
//! no `/` makes the argument remote, so `./odd:name` and `/abs:x` stay
//! local. A name then resolves through a small table:
//!
//! - `local`, `localhost`, and this machine's own fleet location (the
//!   `fleet init` location, design #5687) mean the local host, so
//!   `fleet proxy copy sandbox:/a laptop:/b` run on the sandbox is a
//!   put.
//! - Every other name must be in [`REMOTES`]. Today that is `laptop`,
//!   reached over the session broker's files transport. The target is
//!   always a name the caller gives, never an implied "the laptop", so
//!   more endpoints join the table without a change to the verb.
//!
//! The transfers are the hash-verified `/files/*` paths of
//! [`crate::files`]: local to remote is a put, remote to local is a
//! get, both resumable through their deterministic temps. Remote to
//! remote on one daemon uses its atomic server-side copy route; copies
//! between distinct remotes still relay through the caller. The latter
//! delegated path remains tracked by #14219.

use std::path::{Path, PathBuf};

use fleet_config::Config;

use crate::client::{ClientError, ensure_broker};
use crate::files::{
    BrokerFilesTransport, FilesTransport, get_file, parse_transfer_args, put_file, remote_copy,
    validate_remote_path,
};

pub const COPY_USAGE: &str = "\
Usage: fleet proxy copy [--mode OCTAL] [--no-resume] <source> <destination>

Copy one file between any two hosts (#10355). Each side is a local
path or NAME:PATH on a named remote, as in scp:
  fleet proxy copy ./report.txt laptop:/home/user/report.txt
  fleet proxy copy laptop:/home/user/a.txt /tmp/a.txt
  fleet proxy copy sandbox:/home/hatch/t.txt laptop:/home/user/t2.txt
A bare path is local; prefix ./ to keep a path with a colon local.
NAME is 'local', 'localhost', this host's fleet location, or a known
remote (laptop). Every transfer is SHA-256 verified and lands by
atomic rename; an interrupted copy resumes unless --no-resume.
Remote to remote on the same daemon uses an atomic server-side copy. Distinct remote names still relay through this host. Direct daemon-to-daemon transfer remains tracked by #14219. Directories and stdin/stdout are 'fleet proxy put
-r' and 'fleet proxy get -' / 'put -'.
Exit codes: 2 usage, 3 transport, 69 capability-unavailable.";

/// Opens the files transport for a named remote.
pub type Connect<'a> = dyn FnMut(&str) -> Result<Box<dyn FilesTransport>, ClientError> + 'a;

/// The named remotes `copy` can reach, besides the local host.
pub const REMOTES: &[&str] = &["laptop"];

/// Names that always mean the local host.
const LOCAL_NAMES: &[&str] = &["local", "localhost"];

/// One side of a copy, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Side {
    /// A path on this host.
    Local(String),
    /// `name:path` on a named host.
    Named { name: String, path: String },
}

/// Split one argument the scp way: `name:path` when the text before
/// the first colon is non-empty and holds no `/`, otherwise a local
/// path.
pub fn parse_side(arg: &str) -> Side {
    if let Some((name, path)) = arg.split_once(':') {
        if !name.is_empty() && !name.contains('/') {
            return Side::Named {
                name: name.to_string(),
                path: path.to_string(),
            };
        }
    }
    Side::Local(arg.to_string())
}

/// One side of a copy, resolved to where its bytes live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// A path on this host.
    Local(PathBuf),
    /// A path on a remote from [`REMOTES`].
    Remote { remote: String, path: String },
}

/// Resolve a side. `here` is this host's fleet location, if known.
///
/// # Errors
///
/// [`ClientError::Usage`] for an empty path, `-`, or a name that is
/// neither local nor in [`REMOTES`].
pub fn resolve(side: Side, here: Option<&str>) -> Result<Endpoint, ClientError> {
    match side {
        Side::Local(path) => local_endpoint(path),
        Side::Named { name, path } => {
            if LOCAL_NAMES.contains(&name.as_str()) || here == Some(name.as_str()) {
                return local_endpoint(path);
            }
            if !REMOTES.contains(&name.as_str()) {
                let mut known: Vec<&str> = LOCAL_NAMES.to_vec();
                if let Some(here) = here {
                    known.push(here);
                }
                known.extend(REMOTES);
                return Err(ClientError::Usage(format!(
                    "unknown host {name:?} in {name}:{path}; known: {} (prefix ./ for a local path with a colon)",
                    known.join(", ")
                )));
            }
            validate_remote_path(&path)?;
            Ok(Endpoint::Remote { remote: name, path })
        }
    }
}

fn local_endpoint(path: String) -> Result<Endpoint, ClientError> {
    if path.is_empty() {
        return Err(ClientError::Usage("local path is empty".to_string()));
    }
    if path == "-" {
        return Err(ClientError::Usage(
            "copy takes no '-'; stream with 'fleet proxy put -' or 'fleet proxy get ... -'"
                .to_string(),
        ));
    }
    Ok(Endpoint::Local(PathBuf::from(path)))
}

/// What a copy does, decided from its two endpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Local to remote: one put.
    Put {
        local: PathBuf,
        remote: String,
        path: String,
    },
    /// Remote to local: one get.
    Get {
        remote: String,
        path: String,
        local: PathBuf,
    },
    /// Remote to remote on the same daemon: server-side copy.
    Direct {
        remote: String,
        from_path: String,
        to_path: String,
    },
    /// Remote to remote on distinct daemons: relay through the caller.
    Relay {
        from_remote: String,
        from_path: String,
        to_remote: String,
        to_path: String,
    },
}

/// Decide the transfer for a source and destination.
///
/// # Errors
///
/// [`ClientError::Usage`] when both sides are local: that is `cp`.
pub fn plan(source: Endpoint, destination: Endpoint) -> Result<Plan, ClientError> {
    match (source, destination) {
        (Endpoint::Local(a), Endpoint::Local(b)) => Err(ClientError::Usage(format!(
            "both sides are local ({} -> {}); use cp",
            a.display(),
            b.display()
        ))),
        (Endpoint::Local(local), Endpoint::Remote { remote, path }) => Ok(Plan::Put {
            local,
            remote,
            path,
        }),
        (Endpoint::Remote { remote, path }, Endpoint::Local(local)) => Ok(Plan::Get {
            remote,
            path,
            local,
        }),
        (
            Endpoint::Remote {
                remote: from_remote,
                path: from_path,
            },
            Endpoint::Remote {
                remote: to_remote,
                path: to_path,
            },
        ) => {
            if from_remote == to_remote {
                Ok(Plan::Direct {
                    remote: from_remote,
                    from_path,
                    to_path,
                })
            } else {
                Ok(Plan::Relay {
                    from_remote,
                    from_path,
                    to_remote,
                    to_path,
                })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyArgs {
    pub no_resume: bool,
    pub mode: Option<u32>,
    pub source: String,
    pub destination: String,
}

impl CopyArgs {
    pub fn parse(argv: &[String]) -> Result<CopyArgs, ClientError> {
        let (recursive, no_resume, mode, positional) = parse_transfer_args(argv, COPY_USAGE)?;
        if recursive {
            return Err(ClientError::Usage(
                "copy -r is not in v1; use 'fleet proxy put -r' for a directory upload".to_string(),
            ));
        }
        if positional.len() != 2 {
            return Err(ClientError::Usage(COPY_USAGE.to_string()));
        }
        Ok(CopyArgs {
            no_resume,
            mode,
            source: positional[0].clone(),
            destination: positional[1].clone(),
        })
    }
}

/// What a finished copy moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyReport {
    pub size: u64,
    pub sha256: String,
}

/// Run a plan. `connect` opens the files transport for a remote name;
/// `spool_dir` holds the relay's local spool.
///
/// # Errors
///
/// Whatever the put or get returns; a local source that is missing
/// or a directory is a usage error.
pub fn execute(
    plan: &Plan,
    mode: Option<u32>,
    resume: bool,
    spool_dir: &Path,
    connect: &mut Connect<'_>,
    log: &mut dyn FnMut(&str),
) -> Result<CopyReport, ClientError> {
    match plan {
        Plan::Put {
            local,
            remote,
            path,
        } => {
            if local.is_dir() {
                return Err(ClientError::Usage(format!(
                    "{} is a directory; use 'fleet proxy put -r'",
                    local.display()
                )));
            }
            if !local.exists() {
                return Err(ClientError::Usage(format!(
                    "{} does not exist",
                    local.display()
                )));
            }
            let mut t = connect(remote)?;
            let report = put_file(t.as_mut(), local, path, mode, resume, log)?;
            Ok(CopyReport {
                size: report.size,
                sha256: report.sha256,
            })
        }
        Plan::Get {
            remote,
            path,
            local,
        } => {
            let mut t = connect(remote)?;
            let report = get_file(t.as_mut(), path, local, mode, resume, log)?;
            Ok(CopyReport {
                size: report.size,
                sha256: report.sha256,
            })
        }
        Plan::Direct {
            remote,
            from_path,
            to_path,
        } => {
            let mut transport = connect(remote)?;
            let report = remote_copy(transport.as_mut(), from_path, to_path, mode)?;
            Ok(CopyReport {
                size: report.size,
                sha256: report.sha256,
            })
        }
        Plan::Relay {
            from_remote,
            from_path,
            to_remote,
            to_path,
        } => {
            let spool = spool_dir.join(format!(
                "fleet-proxy-copy-{}-{}.spool",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            let result = (|| -> Result<CopyReport, ClientError> {
                let mut from = connect(from_remote)?;
                // The spool is ours alone: no resume across runs.
                let got = get_file(from.as_mut(), from_path, &spool, None, false, log)?;
                drop(from);
                let mut to = connect(to_remote)?;
                let put = put_file(to.as_mut(), &spool, to_path, mode, resume, log)?;
                if put.sha256 != got.sha256 {
                    return Err(ClientError::Transport(format!(
                        "relay hash changed in the spool: got {}, put {}",
                        got.sha256, put.sha256
                    )));
                }
                Ok(CopyReport {
                    size: put.size,
                    sha256: put.sha256,
                })
            })();
            let _ = std::fs::remove_file(&spool);
            result
        }
    }
}

/// The verb entry point.
///
/// # Errors
///
/// Usage errors for bad addressing; transport errors from the broker
/// or the daemon.
pub fn run_copy(
    args: &CopyArgs,
    config: &Config,
    client: &str,
    capabilities: &[String],
) -> Result<i32, ClientError> {
    let here = config
        .resolve_host()
        .ok()
        .flatten()
        .map(|host| host.location);
    let source = resolve(parse_side(&args.source), here.as_deref())?;
    let destination = resolve(parse_side(&args.destination), here.as_deref())?;
    let plan = plan(source, destination)?;
    let mut ensured = None;
    let mut connect = |remote: &str| -> Result<Box<dyn FilesTransport>, ClientError> {
        match remote {
            "laptop" => {
                if ensured.is_none() {
                    ensured = Some(ensure_broker(config, client, capabilities)?);
                }
                Ok(Box::new(BrokerFilesTransport::new(config)))
            }
            other => Err(ClientError::Usage(format!(
                "no transport for remote {other:?}"
            ))),
        }
    };
    let mut log = |line: &str| eprintln!("{line}");
    let report = execute(
        &plan,
        args.mode,
        !args.no_resume,
        &std::env::temp_dir(),
        &mut connect,
        &mut log,
    )?;
    eprintln!(
        "fleet proxy copy: {} -> {}: {} bytes, sha256 {}",
        args.source, args.destination, report.size, report.sha256
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use serde_json::{Value, json};

    use super::*;
    use crate::files::{b64_decode, b64_encode, sha256_hex, temp_path_for};

    // --- Addressing -----------------------------------------------------

    #[test]
    fn sides_parse_the_scp_way() {
        assert_eq!(
            parse_side("laptop:/home/user/a.txt"),
            Side::Named {
                name: "laptop".into(),
                path: "/home/user/a.txt".into()
            }
        );
        assert_eq!(parse_side("/abs/a:b"), Side::Local("/abs/a:b".into()));
        assert_eq!(parse_side("./odd:name"), Side::Local("./odd:name".into()));
        assert_eq!(parse_side("dir/x:y"), Side::Local("dir/x:y".into()));
        assert_eq!(parse_side(":leading"), Side::Local(":leading".into()));
        assert_eq!(parse_side("plain.txt"), Side::Local("plain.txt".into()));
        // A user@host form is a name, so it is refused loudly rather
        // than silently written as a local file.
        assert!(matches!(parse_side("me@box:/x"), Side::Named { .. }));
    }

    #[test]
    fn names_resolve_to_local_or_a_known_remote() {
        let here = Some("sandbox");
        assert_eq!(
            resolve(parse_side("sandbox:/home/hatch/t.txt"), here).unwrap(),
            Endpoint::Local("/home/hatch/t.txt".into())
        );
        assert_eq!(
            resolve(parse_side("localhost:/tmp/x"), None).unwrap(),
            Endpoint::Local("/tmp/x".into())
        );
        assert_eq!(
            resolve(parse_side("laptop:/home/user/t2.txt"), here).unwrap(),
            Endpoint::Remote {
                remote: "laptop".into(),
                path: "/home/user/t2.txt".into()
            }
        );
        // On the laptop itself, laptop: is local.
        assert_eq!(
            resolve(parse_side("laptop:/x"), Some("laptop")).unwrap(),
            Endpoint::Local("/x".into())
        );
        let unknown = resolve(parse_side("me@box:/x"), here).unwrap_err();
        let ClientError::Usage(text) = unknown else {
            panic!("expected a usage error");
        };
        assert!(text.contains("unknown host"), "{text}");
        assert!(text.contains("laptop"), "{text}");
        assert!(text.contains("sandbox"), "{text}");
        assert!(matches!(
            resolve(parse_side("laptop:"), here),
            Err(ClientError::Usage(_))
        ));
        assert!(matches!(
            resolve(parse_side("-"), here),
            Err(ClientError::Usage(_))
        ));
    }

    #[test]
    fn plans_cover_every_pair() {
        let local = || Endpoint::Local("/tmp/a".into());
        let remote = |p: &str| Endpoint::Remote {
            remote: "laptop".into(),
            path: p.into(),
        };
        assert!(matches!(plan(local(), local()), Err(ClientError::Usage(_))));
        assert!(matches!(plan(local(), remote("/b")), Ok(Plan::Put { .. })));
        assert!(matches!(plan(remote("/b"), local()), Ok(Plan::Get { .. })));
        assert_eq!(
            plan(remote("/b"), remote("/c")).unwrap(),
            Plan::Direct {
                remote: "laptop".into(),
                from_path: "/b".into(),
                to_path: "/c".into(),
            }
        );
        let other = Endpoint::Remote {
            remote: "backup".into(),
            path: "/c".into(),
        };
        assert!(matches!(plan(remote("/b"), other), Ok(Plan::Relay { .. })));
    }

    #[test]
    fn args_parse() {
        let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let parsed = CopyArgs::parse(&argv(&["--mode", "600", "a", "laptop:/b"])).unwrap();
        assert_eq!(parsed.mode, Some(0o600));
        assert!(!parsed.no_resume);
        assert_eq!(parsed.source, "a");
        assert_eq!(parsed.destination, "laptop:/b");
        assert!(CopyArgs::parse(&argv(&["a"])).is_err());
        assert!(CopyArgs::parse(&argv(&["-r", "a", "laptop:/b"])).is_err());
        assert!(CopyArgs::parse(&argv(&["--bogus", "a", "b"])).is_err());
    }

    // --- Transfers against an in-memory daemon --------------------------

    /// The `/files/*` route semantics, in memory: deterministic temps,
    /// extend-only writes, hash-checked atomic commit.
    #[derive(Default)]
    struct Daemon {
        files: HashMap<String, Vec<u8>>,
        corrupt_reads: bool,
    }

    impl Daemon {
        fn handle(&mut self, route: &str, body: &Value) -> Value {
            let path = body["path"].as_str().unwrap_or("").to_string();
            let sha = body["sha256"].as_str().unwrap_or("").to_string();
            match route {
                "/files/stat" => match self.files.get(&path) {
                    Some(bytes) => json!({
                        "exists": true, "type": "file", "size": bytes.len() as u64,
                        "sha256": sha256_hex(bytes),
                    }),
                    None => json!({"exists": false, "type": "missing", "size": 0}),
                },
                "/files/begin" => {
                    self.files.entry(temp_path_for(&path, &sha)).or_default();
                    json!({"ok": true})
                }
                "/files/write" => {
                    let data = b64_decode(body["data_b64"].as_str().unwrap_or("")).unwrap();
                    let slot = self.files.entry(temp_path_for(&path, &sha)).or_default();
                    let offset = body["offset"].as_u64().unwrap_or(0) as usize;
                    if offset != slot.len() {
                        return json!({"error": "offset is not the temp's end"});
                    }
                    slot.extend_from_slice(&data);
                    json!({"size": slot.len() as u64})
                }
                "/files/commit" => {
                    let temp = temp_path_for(&path, &sha);
                    let bytes = self.files.remove(&temp).unwrap_or_default();
                    let actual = sha256_hex(&bytes);
                    if actual != sha {
                        return json!({"error": "sha256 mismatch", "actual": actual});
                    }
                    let size = bytes.len() as u64;
                    self.files.insert(path, bytes);
                    json!({"size": size, "sha256": actual})
                }
                "/files/copy" => {
                    let from = body["from"].as_str().unwrap_or("");
                    let to = body["to"].as_str().unwrap_or("");
                    let Some(source) = self.files.get(from).cloned() else {
                        return json!({"error": "no such source"});
                    };
                    let sha = sha256_hex(&source);
                    self.files.insert(to.to_string(), source.clone());
                    json!({"size": source.len() as u64, "sha256": sha})
                }
                "/files/read" => {
                    let offset = body["offset"].as_u64().unwrap_or(0) as usize;
                    let length = body["length"].as_u64().unwrap_or(0) as usize;
                    let Some(bytes) = self.files.get(&path) else {
                        return json!({"error": "no such file"});
                    };
                    let end = (offset + length).min(bytes.len());
                    let mut slice = bytes[offset.min(end)..end].to_vec();
                    if self.corrupt_reads {
                        if let Some(first) = slice.first_mut() {
                            *first ^= 0xff;
                        }
                    }
                    json!({"data_b64": b64_encode(&slice), "size": bytes.len() as u64})
                }
                _ => json!({"error": format!("unknown route {route}")}),
            }
        }
    }

    struct Transport(Arc<Mutex<Daemon>>);

    impl FilesTransport for Transport {
        fn post(&mut self, path: &str, body: Value) -> Result<Value, ClientError> {
            let reply = self.0.lock().unwrap().handle(path, &body);
            if let Some(error) = reply.get("error").and_then(|v| v.as_str()) {
                return Err(ClientError::Transport(error.to_string()));
            }
            Ok(reply)
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fleet-copy-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn bytes(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 37 % 251) as u8).collect()
    }

    fn run(
        daemon: &Arc<Mutex<Daemon>>,
        plan: &Plan,
        spool: &Path,
    ) -> Result<CopyReport, ClientError> {
        let mut connect = |remote: &str| -> Result<Box<dyn FilesTransport>, ClientError> {
            assert_eq!(remote, "laptop");
            Ok(Box::new(Transport(daemon.clone())))
        };
        execute(plan, None, true, spool, &mut connect, &mut |_| {})
    }

    #[test]
    fn local_to_remote_to_local_round_trips() {
        let dir = scratch("roundtrip");
        let daemon = Arc::new(Mutex::new(Daemon::default()));
        let data = bytes(crate::files::CHUNK_SIZE + 4321);
        let source = dir.join("source.bin");
        std::fs::write(&source, &data).unwrap();

        let up = plan(
            Endpoint::Local(source.clone()),
            resolve(parse_side("laptop:/srv/copy.bin"), Some("sandbox")).unwrap(),
        )
        .unwrap();
        let put = run(&daemon, &up, &dir).unwrap();
        assert_eq!(put.sha256, sha256_hex(&data));
        assert_eq!(
            daemon.lock().unwrap().files.get("/srv/copy.bin"),
            Some(&data)
        );

        let back = dir.join("back.bin");
        let down = plan(
            resolve(parse_side("laptop:/srv/copy.bin"), None).unwrap(),
            Endpoint::Local(back.clone()),
        )
        .unwrap();
        let got = run(&daemon, &down, &dir).unwrap();
        assert_eq!(got.sha256, put.sha256);
        assert_eq!(std::fs::read(&back).unwrap(), data);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_remote_copy_is_server_side_and_hash_verified() {
        let dir = scratch("direct");
        let daemon = Arc::new(Mutex::new(Daemon::default()));
        let data = bytes(100 * 1024 * 1024);
        let expected = sha256_hex(&data);
        daemon
            .lock()
            .unwrap()
            .files
            .insert("/srv/a.bin".into(), data.clone());
        let direct = plan(
            resolve(parse_side("laptop:/srv/a.bin"), None).unwrap(),
            resolve(parse_side("laptop:/srv/b.bin"), None).unwrap(),
        )
        .unwrap();
        assert!(matches!(direct, Plan::Direct { .. }));
        let report = run(&daemon, &direct, &dir).unwrap();
        assert_eq!(report.size, data.len() as u64);
        assert_eq!(report.sha256, expected);
        let files = &daemon.lock().unwrap().files;
        assert_eq!(files.get("/srv/b.bin"), Some(&data));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remote_to_remote_relays_and_leaves_no_spool() {
        let dir = scratch("relay");
        let daemon = Arc::new(Mutex::new(Daemon::default()));
        let data = bytes(70_000);
        daemon
            .lock()
            .unwrap()
            .files
            .insert("/srv/a.bin".into(), data.clone());
        let relay = plan(
            resolve(parse_side("laptop:/srv/a.bin"), None).unwrap(),
            resolve(parse_side("laptop:/srv/b.bin"), None).unwrap(),
        )
        .unwrap();
        let report = run(&daemon, &relay, &dir).unwrap();
        assert_eq!(report.size, data.len() as u64);
        assert_eq!(daemon.lock().unwrap().files.get("/srv/b.bin"), Some(&data));
        let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert!(leftovers.is_empty(), "spool left behind: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupted_download_never_lands_at_the_destination() {
        let dir = scratch("corrupt");
        let daemon = Arc::new(Mutex::new(Daemon::default()));
        daemon
            .lock()
            .unwrap()
            .files
            .insert("/srv/a.bin".into(), bytes(1000));
        daemon.lock().unwrap().corrupt_reads = true;

        let dest = dir.join("dest.bin");
        let down = plan(
            resolve(parse_side("laptop:/srv/a.bin"), None).unwrap(),
            Endpoint::Local(dest.clone()),
        )
        .unwrap();
        assert!(matches!(
            run(&daemon, &down, &dir),
            Err(ClientError::Transport(_))
        ));
        assert!(!dest.exists());

        // A relay with a bad download writes nothing on the far side
        // and removes its spool.
        let relay = plan(
            resolve(parse_side("laptop:/srv/a.bin"), None).unwrap(),
            resolve(parse_side("laptop:/srv/b.bin"), None).unwrap(),
        )
        .unwrap();
        let spool = scratch("corrupt-spool");
        assert!(run(&daemon, &relay, &spool).is_err());
        assert!(!daemon.lock().unwrap().files.contains_key("/srv/b.bin"));
        let spool_files: Vec<_> = std::fs::read_dir(&spool)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".spool"))
            .collect();
        assert!(spool_files.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&spool);
    }

    #[test]
    fn a_missing_or_directory_source_is_a_usage_error() {
        let dir = scratch("source");
        let daemon = Arc::new(Mutex::new(Daemon::default()));
        let to = |p: &Path| Plan::Put {
            local: p.to_path_buf(),
            remote: "laptop".into(),
            path: "/srv/x".into(),
        };
        assert!(matches!(
            run(&daemon, &to(&dir.join("missing")), &dir),
            Err(ClientError::Usage(_))
        ));
        assert!(matches!(
            run(&daemon, &to(&dir), &dir),
            Err(ClientError::Usage(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
