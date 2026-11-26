//! `id nav`: walk a graph snapshot with the native navigation API
//! (#7612). The command reads the snapshot, hands the request to
//! [`navigation_wire::execute`] and prints the answer. It holds no
//! query code of its own: the record's parity rule (#5831 Decision 3)
//! is that every surface runs the one executor.

use std::path::Path;

use anyhow::{Context, bail};

use id_core::graph::navigation::Bounds;
use id_core::graph::navigation_wire::{self, WireGraph, WireResponse};

use crate::cli::NavArgs;

/// Reads a graph snapshot file.
///
/// # Errors
///
/// When the file cannot be read or is not a snapshot.
pub fn read_graph(path: &Path) -> anyhow::Result<WireGraph> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading graph snapshot {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("parsing graph snapshot {}", path.display()))
}

/// The answer `id nav` gives for these arguments over this snapshot.
///
/// # Errors
///
/// When the snapshot does not build into a typed graph.
pub fn nav_answer(graph: &WireGraph, args: &NavArgs) -> anyhow::Result<WireResponse> {
    let built = graph.build().context("building the typed graph")?;
    Ok(navigation_wire::execute(
        &built,
        &args.to_request(),
        &|id| graph.can_see(id),
        &Bounds::default(),
    ))
}

/// Runs `id nav`.
///
/// # Errors
///
/// When the snapshot cannot be read or built, or the answer is a
/// refusal (after printing it).
pub fn cmd_nav(args: &NavArgs) -> anyhow::Result<()> {
    let graph = read_graph(&args.graph)?;
    let answer = nav_answer(&graph, args)?;
    if args.json {
        println!("{}", navigation_wire::to_json(&answer));
    } else {
        println!("{}", navigation_wire::render_text(&answer));
    }
    if let WireResponse::Error { message, .. } = &answer {
        bail!("{message}");
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    //! Oracle: the #5831 decision record, Decision 3 (semantic parity).
    //! The same operation suite runs through the CLI arguments and the
    //! JSON API over one fixture graph, and the answers must be equal.

    use clap::Parser;

    use id_core::graph::navigation::NodeId;

    use super::*;
    use crate::cli::{Cli, Command};

    const FIXTURE: &str = r#"{
        "nodes": [
            {"id": "note", "kind": "note", "created_at": 1},
            {"id": "board", "kind": "board", "created_at": 2},
            {"id": "post", "kind": "post", "created_at": 3},
            {"id": "reply", "kind": "reply", "created_at": 4},
            {"id": "v2", "kind": "version", "created_at": 6},
            {"id": "fork", "kind": "fork", "created_at": 7},
            {"id": "alice", "kind": "actor", "created_at": 0},
            {"id": "secret", "kind": "note", "created_at": 8}
        ],
        "edges": [
            {"kind": "shared-as", "from": "note", "to": "post", "created_at": 10},
            {"kind": "contains", "from": "board", "to": "post", "created_at": 11},
            {"kind": "contains", "from": "post", "to": "reply", "created_at": 12},
            {"kind": "anchored-to", "from": "reply", "to": "note", "created_at": 13},
            {"kind": "edited-as", "from": "note", "to": "v2", "created_at": 14},
            {"kind": "forked-from", "from": "fork", "to": "note", "created_at": 15},
            {"kind": "authored-by", "from": "note", "to": "alice", "created_at": 16},
            {"kind": "references", "from": "secret", "to": "note", "created_at": 17}
        ],
        "hidden": ["secret"]
    }"#;

    fn parse(argv: &[&str]) -> NavArgs {
        let mut full = vec!["id", "nav"];
        full.extend_from_slice(argv);
        full.extend_from_slice(&["--graph", "fixture.json"]);
        match Cli::parse_from(full).command {
            Some(Command::Nav(args)) => args,
            other => panic!("expected nav, got {other:?}"),
        }
    }

    #[test]
    fn the_cli_and_the_api_give_the_same_answers() {
        let graph: WireGraph = serde_json::from_str(FIXTURE).unwrap();
        let built = graph.build().unwrap();
        let can_see = |id: &NodeId| graph.can_see(id);
        let suite: [&[&str]; 8] = [
            &["neighbors", "note"],
            &[
                "neighbors",
                "note",
                "--edge",
                "shared-as",
                "--direction",
                "out",
            ],
            &["neighbors", "note", "--page-size", "2"],
            &["backlinks", "note"],
            &["threads", "note"],
            &["history", "note"],
            &["path", "fork", "--to", "reply"],
            &["neighbors", "secret"],
        ];
        for argv in suite {
            let args = parse(argv);
            let via_cli = nav_answer(&graph, &args).unwrap();
            let request_json = serde_json::to_string(&args.to_request()).unwrap();
            let via_api: WireResponse = serde_json::from_str(&navigation_wire::execute_json(
                &built,
                &request_json,
                &can_see,
                &Bounds::default(),
            ))
            .unwrap();
            assert_eq!(via_cli, via_api, "{argv:?}");
        }
    }

    #[test]
    fn a_cli_cursor_walk_resumes_where_the_page_ended() {
        let graph: WireGraph = serde_json::from_str(FIXTURE).unwrap();
        let mut args = parse(&["neighbors", "note", "--page-size", "2"]);
        let mut seen = Vec::new();
        loop {
            let WireResponse::Page { items, next, .. } = nav_answer(&graph, &args).unwrap() else {
                panic!("expected a page");
            };
            seen.extend(items);
            match next {
                Some(cursor) => args.cursor = Some(cursor),
                None => break,
            }
        }
        let whole = parse(&["neighbors", "note"]);
        let WireResponse::Page { items, .. } = nav_answer(&graph, &whole).unwrap() else {
            panic!("expected a page");
        };
        assert_eq!(seen, items);
    }

    #[test]
    fn repeated_edge_flags_become_one_filter() {
        let args = parse(&[
            "neighbors",
            "note",
            "--edge",
            "contains",
            "--edge",
            "edited-as",
        ]);
        assert_eq!(
            args.to_request().edge_kinds,
            Some(vec![String::from("contains"), String::from("edited-as")])
        );
    }
}
