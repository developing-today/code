//! The link to a world hosted in another process (design §3, first
//! posture: the append gate stays outside the presenter).
//!
//! The protocol is one JSON object per line in each direction, request
//! then reply. Its only payloads are the bridge's own shapes: a journal
//! [`PositionBatch`], a shell-contract `input` frame carrying an intent,
//! a [`GateVerdict`], and the v1 device manifest. A presenter must say
//! hello with its manifest before anything else; the host validates it
//! with the standard manifest parser and answers with the provider names
//! the world grants.
//!
//! [`RemoteWorld`] is the presenter's side: it is the bridge's
//! [`JournalSource`] and [`AppendGate`], so the bridge runs unchanged
//! against a remote world. [`serve`] is the host's side.

use std::fmt;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use id_bevy_bridge::{AppendGate, GateVerdict, JournalSource, PositionBatch};
use id_caps::world_caps::DeviceManifest;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Most positions one `read_after` reply carries, whatever was asked.
pub const MAX_BATCHES_PER_READ: usize = 1024;

/// A request from the presenter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Join: the presenter's v1 device manifest, in wire form.
    Hello {
        /// `DeviceManifest::to_json` output.
        device: serde_json::Value,
    },
    /// Up to `max` positions strictly after `after`.
    ReadAfter {
        /// The presenter's applied position.
        after: Option<u64>,
        /// The presenter's lag budget.
        max: usize,
    },
    /// How many positions exist strictly after `after`.
    PendingAfter {
        /// The presenter's applied position.
        after: Option<u64>,
    },
    /// Submit one shell-contract `input` frame to the append gate.
    Submit {
        /// The frame.
        frame: serde_json::Value,
    },
}

/// The host's reply to one request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum Reply {
    /// The manifest was accepted; these providers are granted.
    Welcome {
        /// Granted provider names.
        granted: Vec<String>,
    },
    /// The request was refused.
    Refused {
        /// Why.
        reason: String,
    },
    /// Journal positions, ascending.
    Batches {
        /// The positions.
        batches: Vec<PositionBatch>,
    },
    /// A pending count.
    Pending {
        /// Positions after the one asked about.
        pending: u64,
    },
    /// The gate journaled the intent.
    Accepted {
        /// The confirming position.
        position: u64,
    },
    /// The gate refused the intent.
    Rejected {
        /// The gate's reason.
        reason: String,
    },
}

/// What a host serves: a journal, an append gate and the world's
/// provider grants.
pub trait HostedWorld: JournalSource + AppendGate {
    /// The provider names the world grants a presenter.
    fn granted(&self) -> Vec<String>;

    /// Admit one more presenter whose manifest checked out, or say why
    /// not. Every admitted presenter departs once its link closes.
    ///
    /// # Errors
    ///
    /// Why the world will not admit another presenter now.
    fn admit(&self) -> Result<(), String> {
        Ok(())
    }

    /// A presenter admitted by [`Self::admit`] has gone.
    fn depart(&self) {}
}

/// Why a call to the host failed.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum RemoteError {
    /// Reading or writing the link failed, or it closed.
    #[error("link to the host failed: {0}")]
    Link(String),
    /// The host's reply was not a reply.
    #[error("host sent an unreadable reply: {0}")]
    Garbled(String),
    /// The host answered with a reply that does not fit the request.
    #[error("host replied out of turn")]
    OutOfTurn,
    /// The host refused the request.
    #[error("host refused: {0}")]
    Refused(String),
}

struct Link {
    reader: Box<dyn BufRead + Send>,
    writer: Box<dyn Write + Send>,
    fault: Option<RemoteError>,
}

/// The presenter's side of the link.
///
/// A link that fails once stays failed: every later call reports the
/// first fault, and the bridge sees an empty journal and rejected
/// intents rather than a half-read reply.
pub struct RemoteWorld {
    link: Mutex<Link>,
}

impl fmt::Debug for RemoteWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteWorld")
            .field("fault", &self.fault())
            .finish_non_exhaustive()
    }
}

impl RemoteWorld {
    /// A link over `reader` and `writer`.
    pub fn new(reader: Box<dyn BufRead + Send>, writer: Box<dyn Write + Send>) -> Self {
        Self {
            link: Mutex::new(Link {
                reader,
                writer,
                fault: None,
            }),
        }
    }

    fn link(&self) -> MutexGuard<'_, Link> {
        self.link.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The fault that broke the link, if it is broken.
    #[must_use]
    pub fn fault(&self) -> Option<RemoteError> {
        self.link().fault.clone()
    }

    /// Send one request and read its reply.
    ///
    /// # Errors
    ///
    /// The link's fault, if it is or becomes broken. A refusal is an
    /// error but does not break the link.
    pub fn call(&self, request: &Request) -> Result<Reply, RemoteError> {
        let mut link = self.link();
        if let Some(fault) = &link.fault {
            return Err(fault.clone());
        }
        match exchange(&mut link, request) {
            Ok(Reply::Refused { reason }) => Err(RemoteError::Refused(reason)),
            Ok(reply) => Ok(reply),
            Err(fault) => {
                link.fault = Some(fault.clone());
                Err(fault)
            }
        }
    }

    /// Say hello with `device` and learn the providers the world grants.
    ///
    /// # Errors
    ///
    /// The host refused the manifest, or the link failed.
    pub fn hello(&self, device: &DeviceManifest) -> Result<Vec<String>, RemoteError> {
        match self.call(&Request::Hello {
            device: device.to_json(),
        })? {
            Reply::Welcome { granted } => Ok(granted),
            _ => Err(RemoteError::OutOfTurn),
        }
    }

    /// The whole journal, as the host holds it now.
    ///
    /// # Errors
    ///
    /// The link failed or the host replied out of turn.
    pub fn whole_journal(&self) -> Result<Vec<PositionBatch>, RemoteError> {
        let mut journal: Vec<PositionBatch> = Vec::new();
        loop {
            let after = journal.last().map(|batch| batch.position);
            let reply = self.call(&Request::ReadAfter {
                after,
                max: MAX_BATCHES_PER_READ,
            })?;
            let Reply::Batches { batches } = reply else {
                return Err(RemoteError::OutOfTurn);
            };
            if batches.is_empty() {
                return Ok(journal);
            }
            journal.extend(batches);
        }
    }
}

fn exchange(link: &mut Link, request: &Request) -> Result<Reply, RemoteError> {
    let mut encoded =
        serde_json::to_string(request).map_err(|error| RemoteError::Link(error.to_string()))?;
    encoded.push('\n');
    link.writer
        .write_all(encoded.as_bytes())
        .and_then(|()| link.writer.flush())
        .map_err(|error| RemoteError::Link(error.to_string()))?;
    let mut reply = String::new();
    let read = link
        .reader
        .read_line(&mut reply)
        .map_err(|error| RemoteError::Link(error.to_string()))?;
    if read == 0 {
        return Err(RemoteError::Link("the host closed the link".to_owned()));
    }
    serde_json::from_str(&reply).map_err(|error| RemoteError::Garbled(error.to_string()))
}

impl JournalSource for RemoteWorld {
    fn read_after(&self, after: Option<u64>, max: usize) -> Vec<PositionBatch> {
        match self.call(&Request::ReadAfter { after, max }) {
            Ok(Reply::Batches { batches }) => batches,
            _ => Vec::new(),
        }
    }

    fn pending_after(&self, after: Option<u64>) -> u64 {
        match self.call(&Request::PendingAfter { after }) {
            Ok(Reply::Pending { pending }) => pending,
            _ => 0,
        }
    }
}

impl AppendGate for RemoteWorld {
    fn submit(&self, frame: &serde_json::Value) -> GateVerdict {
        match self.call(&Request::Submit {
            frame: frame.clone(),
        }) {
            Ok(Reply::Accepted { position }) => GateVerdict::Accepted { position },
            Ok(Reply::Rejected { reason }) => GateVerdict::Rejected { reason },
            Ok(_) => GateVerdict::Rejected {
                reason: RemoteError::OutOfTurn.to_string(),
            },
            Err(error) => GateVerdict::Rejected {
                reason: error.to_string(),
            },
        }
    }
}

/// Answer one request for `world`. `admitted` records whether the
/// presenter has said hello and been admitted.
pub fn answer(world: &dyn HostedWorld, admitted: &mut bool, request: Request) -> Reply {
    match request {
        Request::Hello { .. } if *admitted => Reply::Welcome {
            granted: world.granted(),
        },
        Request::Hello { device } => match DeviceManifest::from_json(&device) {
            Ok(_) => match world.admit() {
                Ok(()) => {
                    *admitted = true;
                    Reply::Welcome {
                        granted: world.granted(),
                    }
                }
                Err(reason) => Reply::Refused { reason },
            },
            Err(error) => Reply::Refused {
                reason: format!("device manifest refused: {error:?}"),
            },
        },
        _ if !*admitted => Reply::Refused {
            reason: "say hello with a device manifest first".to_owned(),
        },
        Request::ReadAfter { after, max } => Reply::Batches {
            batches: world.read_after(after, max.min(MAX_BATCHES_PER_READ)),
        },
        Request::PendingAfter { after } => Reply::Pending {
            pending: world.pending_after(after),
        },
        Request::Submit { frame } => match world.submit(&frame) {
            GateVerdict::Accepted { position } => Reply::Accepted { position },
            GateVerdict::Rejected { reason } => Reply::Rejected { reason },
        },
    }
}

/// Serve `world` over `reader` and `writer` until the presenter closes
/// the link. An admitted presenter departs when the link ends, however
/// it ends.
///
/// # Errors
///
/// Reading or writing the link failed.
pub fn serve(
    world: &dyn HostedWorld,
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
) -> io::Result<()> {
    let mut admitted = false;
    let outcome = serve_lines(world, &mut admitted, reader, writer);
    if admitted {
        world.depart();
    }
    outcome
}

fn serve_lines(
    world: &dyn HostedWorld,
    admitted: &mut bool,
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
) -> io::Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let reply = match serde_json::from_str::<Request>(&line) {
            Ok(request) => answer(world, admitted, request),
            Err(error) => Reply::Refused {
                reason: format!("unreadable request: {error}"),
            },
        };
        let mut out = serde_json::to_string(&reply).map_err(io::Error::other)?;
        out.push('\n');
        writer.write_all(out.as_bytes())?;
        writer.flush()?;
    }
}

/// Start `program` with `args` as a host process and link to it over
/// its standard input and output.
///
/// # Errors
///
/// The process could not start or its pipes were not available.
pub fn spawn_host(program: &Path, args: &[String]) -> io::Result<(Child, RemoteWorld)> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        return Err(io::Error::other("host process has no pipes"));
    };
    let world = RemoteWorld::new(Box::new(BufReader::new(stdout)), Box::new(stdin));
    Ok((child, world))
}

/// Link to a world served on `addr` by [`listen`].
///
/// # Errors
///
/// The connection failed.
pub fn connect_tcp(addr: impl ToSocketAddrs) -> io::Result<RemoteWorld> {
    let stream = TcpStream::connect(addr)?;
    stream.set_nodelay(true)?;
    let reader = BufReader::new(stream.try_clone()?);
    Ok(RemoteWorld::new(Box::new(reader), Box::new(stream)))
}

/// A world's door for other presenters (design §3, host posture): each
/// connection is served on its own thread, through [`serve`], so every
/// presenter is admitted by the world's own [`HostedWorld::admit`].
///
/// Dropping the listener stops it taking new presenters; presenters
/// already admitted keep their links until they close them.
#[derive(Debug)]
pub struct Listener {
    addr: SocketAddr,
    stopped: Arc<AtomicBool>,
    accepting: Option<JoinHandle<()>>,
}

/// Start admitting presenters to `world` on `addr`. Use port 0 for any
/// free port; [`Listener::addr`] says which one was taken.
///
/// # Errors
///
/// The address could not be bound.
pub fn listen<W: HostedWorld>(world: Arc<W>, addr: impl ToSocketAddrs) -> io::Result<Listener> {
    let listener = TcpListener::bind(addr)?;
    let addr = listener.local_addr()?;
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&stopped);
    let accepting = std::thread::Builder::new()
        .name("id-bevy-world-door".to_owned())
        .spawn(move || {
            for stream in listener.incoming() {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(stream) = stream else {
                    continue;
                };
                let world = Arc::clone(&world);
                let _ = std::thread::Builder::new()
                    .name("id-bevy-world-link".to_owned())
                    .spawn(move || serve_stream(&*world, stream));
            }
        })?;
    Ok(Listener {
        addr,
        stopped,
        accepting: Some(accepting),
    })
}

fn serve_stream(world: &dyn HostedWorld, stream: TcpStream) {
    let _ = stream.set_nodelay(true);
    let Ok(reading) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reading);
    let mut writer = stream;
    let _ = serve(world, &mut reader, &mut writer);
}

impl Listener {
    /// The address presenters connect to.
    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Stop taking new presenters and wait for the door to close.
    pub fn stop(&mut self) {
        let Some(accepting) = self.accepting.take() else {
            return;
        };
        self.stopped.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(self.addr);
        let _ = accepting.join();
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop();
    }
}
