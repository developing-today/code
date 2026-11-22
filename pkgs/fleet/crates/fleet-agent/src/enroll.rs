//! `fleet agent enroll` and `fleet agent doctor`.
//!
//! Enrolment is three things, done in one command: register the agent's
//! mailbox on the bridge under its name and groups, find out which legs
//! answer from this seat, and save a small profile so later verbs know
//! who is asking without being told.
//!
//! It asks for nothing a seat does not have. The bridge is HTTPS on
//! Cloudflare, so messaging and pushed tasks work from anywhere the
//! fleet token resolves by name. The queue, the laptop shell and GitHub
//! are probed and reported; a leg that does not answer narrows the
//! intake ladder, it does not fail the enrolment. An agent is enrolled
//! when at least one rung can bring it work.

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::kinds::{self, Seat};
use crate::{AgentError, EXIT_DOWN, EXIT_OK, Out, R, World, argv, check_name, parse, usage};

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub name: String,
    pub kind: String,
    pub groups: Vec<String>,
    pub enrolled_at: u64,
    /// The route that reached GitHub at enrolment (`fleet` or
    /// `plain`), so later runs try the working one first.
    pub gh: Option<String>,
}

/// `--profile`, else `$FLEET_AGENT_PROFILE`, else the XDG config path.
pub fn profile_path(world: &dyn World, flag: Option<String>) -> Option<PathBuf> {
    if let Some(p) = flag.or_else(|| world.env("FLEET_AGENT_PROFILE")) {
        return Some(PathBuf::from(p));
    }
    if let Some(config) = world.env("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(config).join("fleet").join("agent.json"));
    }
    world.home().map(|h| h.join(".config/fleet/agent.json"))
}

/// The saved profile, when there is one and it parses.
pub fn read_profile(world: &dyn World, flag: Option<String>) -> Option<Profile> {
    let path = profile_path(world, flag)?;
    let v: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    Some(Profile {
        name: v.get("name")?.as_str()?.to_string(),
        kind: v.get("kind")?.as_str()?.to_string(),
        groups: v
            .get("groups")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|g| g.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        enrolled_at: v.get("enrolled_at").and_then(Value::as_u64).unwrap_or(0),
        gh: v.get("gh").and_then(Value::as_str).map(str::to_string),
    })
}

fn write_profile(path: &PathBuf, profile: &Profile) -> R<()> {
    let down = |e: std::io::Error| {
        AgentError::Down(format!("cannot save the profile {}: {e}", path.display()))
    };
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir).map_err(down)?;
        }
    }
    let mut record = json!({
        "name": profile.name,
        "kind": profile.kind,
        "groups": profile.groups,
        "enrolled_at": profile.enrolled_at,
    });
    if let Some(gh) = &profile.gh {
        record["gh"] = json!(gh);
    }
    let text = serde_json::to_string_pretty(&record).unwrap_or_default();
    fs::write(path, format!("{text}\n")).map_err(down)
}

const BRIDGE_NEEDS: &str =
    "HTTPS to the message bridge, and the fleet token by name through fleet secrets";

/// One leg's answer from this seat.
#[derive(Debug, Clone, PartialEq)]
pub struct Leg {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
    /// What this leg needs, said once so a failed probe explains itself.
    pub needs: &'static str,
    /// For the github leg: which route answered (`fleet` or `plain`).
    pub route: Option<&'static str>,
}

impl Leg {
    fn to_json(&self) -> Value {
        let mut leg =
            json!({"leg": self.name, "ok": self.ok, "detail": self.detail, "needs": self.needs});
        if let Some(route) = self.route {
            leg["route"] = json!(route);
        }
        leg
    }
}

/// How the bridge leg is checked. The service's health answers
/// anyone, so it says nothing about whether this seat holds the fleet
/// token; only a call made as an agent does.
#[derive(Debug, Clone, Copy)]
pub enum BridgeProbe<'a> {
    /// Register the agent with its groups (`bridge hello`).
    Register(&'a str, &'a [String]),
    /// Read the agent's mailbox without taking anything.
    Read(&'a str),
    /// No agent name is known: only the service can be asked.
    Service,
}

/// Probe the four legs, the bridge leg as `how` says.
pub fn probe(world: &dyn World, how: BridgeProbe) -> Vec<Leg> {
    let bridge_call = match how {
        BridgeProbe::Register(agent, groups) => {
            let mut hello = argv(&["bridge", "hello", "--agent", agent]);
            if !groups.is_empty() {
                hello.push("--groups".to_string());
                hello.push(groups.join(","));
            }
            hello
        }
        // A read that takes nothing: it needs the fleet token, so it
        // shows this seat can really read its mailbox.
        BridgeProbe::Read(agent) => argv(&["bridge", "inbox", "--agent", agent, "--limit", "1"]),
        BridgeProbe::Service => argv(&["bridge", "health"]),
    };
    // The four legs are independent, so they are asked together.
    let mut outs = world
        .fleet_many(&[
            bridge_call,
            argv(&["queue", "health"]),
            argv(&["gh", "api", "rate_limit"]),
            argv(&["laptop-agent", "health"]),
        ])
        .into_iter();
    let mut next_out = || {
        outs.next().unwrap_or(Out {
            code: EXIT_DOWN,
            stdout: String::new(),
            stderr: "the probe returned no result".to_string(),
        })
    };
    let leg = |name: &'static str, out: &Out, up: &str, needs: &'static str| Leg {
        name,
        ok: out.ok(),
        detail: if out.ok() { up.to_string() } else { out.why() },
        needs,
        route: None,
    };
    let bridge_out = next_out();
    let queue_out = next_out();
    let through_fleet = next_out();
    let shell_out = next_out();
    let (github_out, github_route) = if through_fleet.ok() {
        (through_fleet, Some("fleet"))
    } else {
        let plain = world.program("gh", &argv(&["api", "rate_limit"]));
        let route = plain.ok().then_some("plain");
        (plain, route)
    };
    vec![
        match how {
            // The service answering is not this seat reading a
            // mailbox: without an agent name the push rung is not
            // shown as working.
            BridgeProbe::Service => Leg {
                name: "bridge",
                ok: false,
                detail: if bridge_out.ok() {
                    "the service answers, but no agent name is known to read a mailbox with: \
                     enrol, or pass --agent"
                        .to_string()
                } else {
                    bridge_out.why()
                },
                needs: BRIDGE_NEEDS,
                route: None,
            },
            BridgeProbe::Register(..) => leg("bridge", &bridge_out, "registered", BRIDGE_NEEDS),
            BridgeProbe::Read(..) => {
                leg("bridge", &bridge_out, "the mailbox answers", BRIDGE_NEEDS)
            }
        },
        leg(
            "queue",
            &queue_out,
            "answers",
            "the queue broker: its socket on the laptop, or the laptop transport from elsewhere",
        ),
        Leg {
            route: github_route,
            ..leg(
                "github",
                &github_out,
                "answers",
                "gh with a token: fleet gh's rotation, or a signed-in gh",
            )
        },
        leg(
            "shell",
            &shell_out,
            "the laptop answers",
            "a route to the laptop: its broker, the Cloudflare tunnel, the tailnet or ssh",
        ),
    ]
}

fn leg_ok(legs: &[Leg], name: &str) -> bool {
    legs.iter().any(|l| l.name == name && l.ok)
}

/// Which intake rungs the answering legs leave this seat.
pub fn rungs(legs: &[Leg]) -> Value {
    json!({
        "push": leg_ok(legs, "bridge"),
        "queue": leg_ok(legs, "queue"),
        "shortlist": true,
        "priority": leg_ok(legs, "github"),
    })
}

fn any_intake(legs: &[Leg]) -> bool {
    leg_ok(legs, "bridge") || leg_ok(legs, "queue") || leg_ok(legs, "github")
}

fn run_loop(agent: &str) -> Vec<String> {
    vec![
        format!("fleet agent next --agent {agent}    # exit 5 means no work on any rung this run"),
        "do the task it returns, then run its `then` lines".to_string(),
        "repeat while the run has budget".to_string(),
    ]
}

fn central_lines(address: &str) -> Vec<String> {
    vec![
        format!(
            "fleet bridge send --to {address} --command \"<the task>\"    # a task to this one agent"
        ),
        format!(
            "fleet bridge send --to {address} \"<a note>\"                # information, no reply owed"
        ),
    ]
}

/// `fleet agent enroll`.
pub fn enroll(world: &dyn World, args: &[String]) -> R<(Value, i32)> {
    let a = parse(
        args,
        &["--dry-run"],
        &["--kind", "--name", "--groups", "--profile"],
    )?;
    a.no_positional("enroll")?;
    let kind_name = a.last("--kind").ok_or_else(|| {
        usage(format!(
            "enroll needs --kind: one of {}",
            kinds::names().join(", ")
        ))
    })?;
    let kind = kinds::kind(&kind_name).ok_or_else(|| {
        usage(format!(
            "unknown kind '{kind_name}': one of {} (use generic for anything with a shell)",
            kinds::names().join(", ")
        ))
    })?;
    let dry_run = a.has("--dry-run");

    // A connector has a fixed channel address and no mailbox to register.
    if kind.seat == Seat::Connector {
        let address = kind.address.unwrap_or(kind.name);
        return Ok((
            json!({
                "agent": address,
                "kind": kind.name,
                "seat": kind.seat.id(),
                "address": address,
                "registered": false,
                "steps": [
                    "Attach the Message Bridge connector to the assistant (the bridge page shows the URL).",
                    format!("Send to it at address `{address}`; it answers with send_message reply_to."),
                    kind.note,
                ],
                "central": central_lines(address),
            }),
            EXIT_OK,
        ));
    }

    let name = check_name(
        "--name",
        &a.last("--name")
            .ok_or_else(|| usage("enroll needs --name <the agent's stable name>"))?,
    )?;
    let mut groups: Vec<String> = kind.groups.iter().map(|g| g.to_string()).collect();
    if let Some(extra) = a.last("--groups") {
        for g in extra.split(',').map(str::trim).filter(|g| !g.is_empty()) {
            let g = check_name("--groups entry", g)?;
            if !groups.contains(&g) {
                groups.push(g);
            }
        }
    }

    let legs = if dry_run {
        probe(world, BridgeProbe::Read(&name))
    } else {
        probe(world, BridgeProbe::Register(&name, &groups))
    };
    let registered = !dry_run && leg_ok(&legs, "bridge");

    let profile_file = profile_path(world, a.last("--profile"));
    // Only a seat that runs the binary keeps a profile: an http agent
    // is enrolled from someone else's seat and must not take it over.
    let saved = match (&profile_file, kind.seat, dry_run) {
        (Some(path), Seat::Binary, false) => {
            write_profile(
                path,
                &Profile {
                    name: name.clone(),
                    kind: kind.name.to_string(),
                    groups: groups.clone(),
                    enrolled_at: world.now(),
                    gh: legs
                        .iter()
                        .find(|l| l.name == "github")
                        .and_then(|l| l.route)
                        .map(str::to_string),
                },
            )?;
            Some(path.display().to_string())
        }
        _ => None,
    };

    let address = format!("agent:{name}");
    let mut out = json!({
        "agent": name,
        "kind": kind.name,
        "seat": kind.seat.id(),
        "address": address,
        "groups": groups,
        "registered": registered,
        "dry_run": dry_run,
        "profile": saved,
        "legs": legs.iter().map(Leg::to_json).collect::<Vec<_>>(),
        "rungs": rungs(&legs),
        "central": central_lines(&address),
        "note": kind.note,
    });
    match kind.seat {
        Seat::Binary => {
            out["run_loop"] = json!(run_loop(&name));
            out["builds"] = json!(
                "fleet shell -- <command>    # runs where builds belong; never build in a limited sandbox"
            );
        }
        Seat::Http => {
            out["http"] = json!({
                "token": "the fleet token, Bitwarden secret message-bridge.fleet-token, by name",
                "reference_client": "python3 scripts/fleet/bridge-mailbox.py inbox --agent <name> --commands --take",
                "routes": [
                    "POST /fleet/<fleet token>/hello   {agent, groups}",
                    "GET  /fleet/<fleet token>/inbox   agent, kind=command, mark_read=true, limit=1",
                    "POST /fleet/<fleet token>/send    {agent, body, reply_to}",
                    "POST /fleet/<fleet token>/ack     {agent, up_to_id}",
                ],
                "work": "tasks arrive as commands in the mailbox; central or any enrolled agent sends them",
            });
        }
        Seat::Connector => {}
    }
    let code = if dry_run || any_intake(&legs) {
        EXIT_OK
    } else {
        EXIT_DOWN
    };
    Ok((out, code))
}

/// `fleet agent doctor`.
pub fn doctor(world: &dyn World, args: &[String]) -> R<(Value, i32)> {
    let a = parse(args, &[], &["--agent", "--profile"])?;
    a.no_positional("doctor")?;
    let profile = read_profile(world, a.last("--profile"));
    let agent = match a
        .last("--agent")
        .or_else(|| world.env("FLEET_AGENT"))
        .or_else(|| profile.as_ref().map(|p| p.name.clone()))
    {
        Some(n) => Some(check_name("--agent", &n)?),
        None => None,
    };
    let legs = probe(
        world,
        match &agent {
            Some(name) => BridgeProbe::Read(name),
            None => BridgeProbe::Service,
        },
    );
    let code = if any_intake(&legs) {
        EXIT_OK
    } else {
        EXIT_DOWN
    };
    Ok((
        json!({
            "agent": agent,
            "enrolled": profile.is_some(),
            "legs": legs.iter().map(Leg::to_json).collect::<Vec<_>>(),
            "rungs": rungs(&legs),
            "builds": if leg_ok(&legs, "shell") {
                "fleet shell reaches the laptop"
            } else {
                "fleet shell has no laptop route from here; it will try a codespace, then run here"
            },
        }),
        code,
    ))
}
