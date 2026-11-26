//! `id-tui-shell`: the terminal shell for an `id` world session
//! (#12818).
//!
//! ```text
//! id-tui-shell --node <node-id> --capability <cap> [--world <name>]
//!              [--addr <ip:port>]... [--no-relay]
//! ```
//!
//! Dials the world host the way `id world join` does, joins with the
//! capability, and runs [`App`] in the terminal: host frames and keys
//! go in, contract-checked frames come out, and the three panels are
//! redrawn after each. When the host stream ends the shell redials and
//! resumes from its last applied sequence; a host refusal ends the
//! session with the host's message.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use id::frames::{read_frame, write_frame};
use id::world_net::{WorldClient, route_frame};
use id_tui_shell::app::{App, Input, Step};
use id_tui_shell::term;
use iroh::endpoint::{RecvStream, RelayMode, SendStream, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, TransportAddr};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste};
use ratatui::crossterm::execute;
use serde_json::Value;
use tokio::sync::mpsc;

const USAGE: &str = "usage: id-tui-shell --node <node-id> --capability <cap> \
                     [--world <name>] [--addr <ip:port>]... [--no-relay]";

/// How many times a dropped session is redialed before the shell gives
/// up, and the longest wait between tries.
const REDIAL_TRIES: u32 = 6;
const REDIAL_MAX_WAIT: Duration = Duration::from_secs(30);

/// Where and how to join.
#[derive(Debug, Default)]
struct Args {
    node: String,
    capability: String,
    world: Option<String>,
    addrs: Vec<SocketAddr>,
    no_relay: bool,
}

fn parse_args() -> Result<Args> {
    let mut args = Args::default();
    let mut iter = std::env::args().skip(1);
    while let Some(flag) = iter.next() {
        let mut value = |name: &str| -> Result<String> {
            iter.next()
                .with_context(|| format!("{name} needs a value\n{USAGE}"))
        };
        match flag.as_str() {
            "--node" => args.node = value("--node")?,
            "--capability" => args.capability = value("--capability")?,
            "--world" => args.world = Some(value("--world")?),
            "--addr" => {
                let text = value("--addr")?;
                args.addrs.push(
                    text.parse()
                        .with_context(|| format!("parse --addr {text}\n{USAGE}"))?,
                );
            }
            "--no-relay" => args.no_relay = true,
            other => bail!("unknown argument `{other}`\n{USAGE}"),
        }
    }
    if args.node.is_empty() || args.capability.is_empty() {
        bail!("--node and --capability are required\n{USAGE}");
    }
    Ok(args)
}

/// Bind the client endpoint the way `id world join` does: the shared
/// client keypair, a public endpoint when no direct addresses are
/// given, a relay-free one otherwise.
async fn bind(args: &Args) -> Result<(Endpoint, EndpointAddr)> {
    let node = &args.node;
    if node.len() != 64 || !node.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("`{node}` is not a node ID (expected 64 hex characters)");
    }
    let id: EndpointId = node.parse().context("parse node ID")?;
    let key = id::load_or_create_keypair(id::CLIENT_KEY_FILE)
        .await
        .context("load client keypair")?;
    let endpoint = if args.addrs.is_empty() {
        id::connectivity::public_builder(id::connectivity::RelayPolicy::from_no_relay(
            args.no_relay,
        ))
        .secret_key(key)
        .bind()
        .await?
    } else {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .secret_key(key)
            .bind()
            .await?
    };
    let addr = EndpointAddr::from_parts(id, args.addrs.iter().copied().map(TransportAddr::Ip));
    Ok((endpoint, addr))
}

/// Messages into the loop: a connection's input, tagged with which
/// connection it came from so a stale reader is ignored, or a key.
enum Message {
    Connection(u64, Input),
    Key(Input),
}

/// Read host frames into the loop until the stream ends.
fn spawn_reader(
    generation: u64,
    mut recv: RecvStream,
    messages: mpsc::UnboundedSender<Message>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let input = match read_frame(&mut recv).await {
                Ok(Some(body)) => match serde_json::from_slice::<Value>(&body) {
                    Ok(frame) => Input::Host(frame),
                    Err(_) => Input::HostClosed,
                },
                Ok(None) | Err(_) => Input::HostClosed,
            };
            let closed = input == Input::HostClosed;
            if messages
                .send(Message::Connection(generation, input))
                .is_err()
                || closed
            {
                break;
            }
        }
    })
}

/// One live connection: its send half and its reader.
struct Connection {
    send: SendStream,
    reader: tokio::task::JoinHandle<()>,
    client: iroh::endpoint::Connection,
}

impl Connection {
    fn close(self) {
        self.reader.abort();
        self.client.close(0_u32.into(), b"done");
    }
}

/// Dial, open a session stream, start its reader, and send the opening
/// frames.
async fn open(
    endpoint: &Endpoint,
    addr: &EndpointAddr,
    world: Option<&str>,
    app: &mut App,
    size: (u16, u16),
    generation: u64,
    messages: &mpsc::UnboundedSender<Message>,
) -> Result<Connection> {
    let client = WorldClient::connect(endpoint, addr.clone())
        .await
        .context("connect to world host")?;
    let (client, mut send, recv) = client.into_parts();
    for frame in app.opening_frames(size.0, size.1)? {
        write_frame(&mut send, route_frame(&frame, world).as_bytes())
            .await
            .context("send opening frame")?;
    }
    let reader = spawn_reader(generation, recv, messages.clone());
    Ok(Connection {
        send,
        reader,
        client,
    })
}

async fn run(
    terminal: &mut DefaultTerminal,
    args: &Args,
    endpoint: &Endpoint,
    addr: &EndpointAddr,
    messages_tx: &mpsc::UnboundedSender<Message>,
    messages: &mut mpsc::UnboundedReceiver<Message>,
) -> Result<()> {
    let world = args.world.as_deref();
    let mut app = App::new(&args.capability);
    let size = terminal.size()?;
    let mut generation = 0;
    let mut connection = open(
        endpoint,
        addr,
        world,
        &mut app,
        (size.width, size.height),
        generation,
        messages_tx,
    )
    .await?;
    loop {
        terminal.draw(|frame| app.draw(frame))?;
        let Some(message) = messages.recv().await else {
            return Ok(());
        };
        let input = match message {
            Message::Connection(from, _) if from != generation => continue,
            Message::Connection(_, input) | Message::Key(input) => input,
        };
        let step = match app.handle(input) {
            Some(Step::Send(frame)) => {
                match write_frame(&mut connection.send, route_frame(&frame, world).as_bytes()).await
                {
                    Ok(()) => None,
                    Err(_) => app.handle(Input::HostClosed),
                }
            }
            other => other,
        };
        match step {
            None | Some(Step::Send(_)) => {}
            Some(Step::Quit) => {
                connection.close();
                return Ok(());
            }
            Some(Step::End(message)) => {
                connection.close();
                bail!("world host refused the session: {message}");
            }
            Some(Step::Reconnect) => {
                terminal.draw(|frame| app.draw(frame))?;
                connection.close();
                generation += 1;
                let mut wait = Duration::from_secs(1);
                let mut tries = 0;
                connection = loop {
                    let size = terminal.size()?;
                    match open(
                        endpoint,
                        addr,
                        world,
                        &mut app,
                        (size.width, size.height),
                        generation,
                        messages_tx,
                    )
                    .await
                    {
                        Ok(connection) => break connection,
                        Err(error) => {
                            tries += 1;
                            if tries >= REDIAL_TRIES {
                                return Err(error.context("redial world host"));
                            }
                            app.set_status(Some(format!("reconnecting: {error}")));
                            terminal.draw(|frame| app.draw(frame))?;
                            tokio::time::sleep(wait).await;
                            wait = (wait * 2).min(REDIAL_MAX_WAIT);
                        }
                    }
                };
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(2);
        }
    };
    let (endpoint, addr) = bind(&args).await?;

    let mut terminal = ratatui::try_init().context("open terminal")?;
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);

    let (messages_tx, mut messages) = mpsc::unbounded_channel();
    let keys_tx = messages_tx.clone();
    // Keys are read on their own thread once the terminal is in raw
    // mode, and reach the loop through the same channel as host frames.
    let spawn = std::thread::Builder::new()
        .name("id-tui-shell-keys".to_owned())
        .spawn(move || {
            while let Ok(event) = event::read() {
                if let Some(key) = term::key(&event)
                    && keys_tx.send(Message::Key(Input::Key(key))).is_err()
                {
                    break;
                }
            }
        })
        .context("spawn key reader");
    if let Err(error) = spawn {
        let _ = execute!(std::io::stdout(), DisableBracketedPaste);
        ratatui::restore();
        return Err(error);
    }

    let result = run(
        &mut terminal,
        &args,
        &endpoint,
        &addr,
        &messages_tx,
        &mut messages,
    )
    .await;
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    endpoint.close().await;
    result
}
