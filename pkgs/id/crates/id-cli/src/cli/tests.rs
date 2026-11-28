//! Tests for the [`super`] CLI definitions: the command tree, argument
//! parsing, and result-limit parsing.

use super::*;
use clap::CommandFactory;

/// Names in the module-doc command list (the text block under
/// `Commands:` at the top of this file): the first token of each
/// non-empty line.
fn module_doc_entries(source: &str) -> Vec<&str> {
    let doc_list = source
        .split("Commands:")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .expect("module-doc command list present");
    doc_list
        .lines()
        .filter_map(|line| {
            line.trim_start_matches('/')
                .trim_start_matches('!')
                .trim()
                .split_whitespace()
                .next()
        })
        .collect()
}

/// Names in the README CLI Commands table: the backticked first
/// cell of each table row.
fn readme_table_entries(readme: &str) -> Vec<&str> {
    let table = readme
        .split("## CLI Commands")
        .nth(1)
        .and_then(|rest| rest.split("### ").next())
        .expect("README CLI Commands table present");
    table
        .lines()
        .filter_map(|line| line.strip_prefix("| `")?.split('`').next())
        .collect()
}

/// Both hand-maintained command lists must match the live clap
/// subcommands in BOTH directions (#6976): every subcommand is
/// listed (the original #3715 direction), and every listed entry
/// is still a real subcommand \— a renamed or removed subcommand
/// must not leave a stale entry behind with the guard green.
fn check_command_lists(
    names: &[String],
    doc_entries: &[&str],
    readme_entries: &[&str],
) -> Result<(), String> {
    for name in names {
        if !doc_entries.contains(&name.as_str()) {
            return Err(format!(
                "module-doc command list in cli/mod.rs omits `{name}`"
            ));
        }
        if !readme_entries.contains(&name.as_str()) {
            return Err(format!("README CLI Commands table omits `{name}`"));
        }
    }
    for entry in doc_entries {
        if !names.iter().any(|name| name == entry) {
            return Err(format!(
                "module-doc command list in cli/mod.rs lists `{entry}`, which is not a current clap subcommand"
            ));
        }
    }
    for entry in readme_entries {
        if !names.iter().any(|name| name == entry) {
            return Err(format!(
                "README CLI Commands table lists `{entry}`, which is not a current clap subcommand"
            ));
        }
    }
    Ok(())
}

fn clap_subcommand_names() -> Vec<String> {
    let cmd = Cli::command();
    cmd.get_subcommands()
        .map(|s| s.get_name().to_owned())
        .filter(|n| n != "help")
        .collect()
}

#[test]
fn test_command_lists_cover_all_clap_subcommands() {
    let names = clap_subcommand_names();
    assert!(
        names.len() >= 20,
        "expected the full command set, got {names:?}"
    );
    let source = include_str!("mod.rs");
    let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../README.md"))
        .expect("README readable");
    check_command_lists(
        &names,
        &module_doc_entries(source),
        &readme_table_entries(&readme),
    )
    .expect("command lists match the clap subcommands in both directions");
}

/// #6976 mutation check: adding a bogus entry to either list must
/// make the bidirectional guard fail \— that is the drift class the
/// reverse direction exists to catch.
#[test]
fn test_command_lists_reject_stale_entries() {
    let names = clap_subcommand_names();
    let source = include_str!("mod.rs");
    let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../README.md"))
        .expect("README readable");

    let mut doc_entries = module_doc_entries(source);
    doc_entries.push("bogus-command");
    assert!(
        check_command_lists(&names, &doc_entries, &readme_table_entries(&readme)).is_err(),
        "guard accepted a stale module-doc entry"
    );

    let mut readme_entries = readme_table_entries(&readme);
    readme_entries.push("bogus-command");
    assert!(
        check_command_lists(&names, &module_doc_entries(source), &readme_entries).is_err(),
        "guard accepted a stale README table entry"
    );
}

#[test]
fn test_cli_parse_no_args() {
    let cli = Cli::parse_from(["id"]);
    assert!(cli.command.is_none());
    assert!(!cli.debug);
    assert!(cli.log_level.is_none());
}

/// Parse via `normalize_args` on a big-stack thread: building this
/// CLI's clap tree is deep enough to overflow the default test
/// thread stack (the #3059 parse tests below spawn a thread for the
/// same reason).
fn parse_normalized(args: &[&str]) -> Cli {
    let owned: Vec<OsString> = args.iter().map(OsString::from).collect();
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(move || Cli::parse_from(normalize_args(owned)))
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn test_cli_parse_new_space_form_subcommand_not_swallowed() {
    // #3144: `id --new serve` must run serve under a random instance
    // name, not create an instance literally named "serve".
    let cli = parse_normalized(&["id", "--new", "serve"]);
    assert_eq!(cli.new.as_deref(), Some(""));
    assert!(matches!(cli.command, Some(Command::Serve(..))));

    let cli = parse_normalized(&["id", "--new", "id"]);
    assert_eq!(cli.new.as_deref(), Some(""));
    assert!(matches!(cli.command, Some(Command::Id { .. })));

    let cli = parse_normalized(&["id", "--new", "list"]);
    assert_eq!(cli.new.as_deref(), Some(""));
    assert!(matches!(cli.command, Some(Command::List { .. })));
}

#[test]
fn test_cli_parse_new_forms_preserved() {
    // Named space form: the name is kept and the subcommand still runs.
    let cli = parse_normalized(&["id", "--new", "my-dev", "list"]);
    assert_eq!(cli.new.as_deref(), Some("my-dev"));
    assert!(matches!(cli.command, Some(Command::List { .. })));

    // Bare --new: random name, no subcommand.
    let cli = parse_normalized(&["id", "--new"]);
    assert_eq!(cli.new.as_deref(), Some(""));
    assert!(cli.command.is_none());

    // Equals form is passed through untouched, even when the name
    // spells a subcommand.
    let cli = parse_normalized(&["id", "--new=serve", "list"]);
    assert_eq!(cli.new.as_deref(), Some("serve"));
    assert!(matches!(cli.command, Some(Command::List { .. })));

    // Trailing --new after the subcommand keeps working.
    let cli = parse_normalized(&["id", "list", "--new"]);
    assert_eq!(cli.new.as_deref(), Some(""));
    assert!(matches!(cli.command, Some(Command::List { .. })));
}

#[test]
fn test_normalize_args_rewrites_only_the_ambiguous_form() {
    // Same big-stack thread as parse_normalized: collecting the
    // subcommand names builds the clap tree.
    let (rewritten, flag_value) = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            (
                normalize_args(["id", "--new", "serve"]),
                normalize_args(["id", "--log-level", "list", "--new", "id"]),
            )
        })
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(
        rewritten,
        vec![
            OsString::from("id"),
            OsString::from("--new="),
            OsString::from("serve")
        ]
    );
    // A global flag value that spells a subcommand is not a subcommand.
    assert_eq!(
        flag_value,
        vec![
            OsString::from("id"),
            OsString::from("--log-level"),
            OsString::from("list"),
            OsString::from("--new="),
            OsString::from("id")
        ]
    );
}

#[test]
fn test_cli_parse_debug_flag() {
    let cli = Cli::parse_from(["id", "--debug"]);
    assert!(cli.debug);
    assert!(cli.log_level.is_none());
}

#[test]
fn test_cli_parse_log_level_flag() {
    let cli = Cli::parse_from(["id", "--log-level", "trace"]);
    assert!(!cli.debug);
    assert_eq!(cli.log_level, Some("trace".to_owned()));
}

#[test]
fn test_cli_parse_debug_with_subcommand() {
    let cli = Cli::parse_from(["id", "--debug", "serve"]);
    assert!(cli.debug);
    assert!(matches!(cli.command, Some(Command::Serve(..))));
}

#[test]
fn test_cli_parse_log_level_with_subcommand() {
    let cli = Cli::parse_from(["id", "--log-level", "warn", "repl"]);
    assert_eq!(cli.log_level, Some("warn".to_owned()));
    assert!(matches!(cli.command, Some(Command::Repl { .. })));
}

#[test]
fn test_cli_parse_debug_after_subcommand() {
    // Global flags should work after subcommand too
    let cli = Cli::parse_from(["id", "serve", "--debug"]);
    assert!(cli.debug);
    assert!(matches!(cli.command, Some(Command::Serve(..))));
}

#[test]
fn test_cli_parse_serve() {
    let cli = Cli::parse_from(["id", "serve"]);
    match cli.command {
        Some(Command::Serve(args)) => {
            let ServeArgs {
                ephemeral,
                no_relay,
                no_gossip,
                web,
                port,
                bootstrap,
                topic,
                topic_secret,
                no_default_bootstrap,
                no_default_topic,
                no_mdns,
                iroh_port,
                bind,
                web_token,
                world,
                world_admin_token,
                world_module,
                world_name,
                world_ssh_port,
                world_checkpoint_every,
                world_max_open,
                world_max_sessions,
                world_idle_secs,
                world_boot_window_secs,
                world_caps,
                world_cap_policy,
                roc_bin,
                roc_platform,
                world_native,
                allow_node,
                open_writes,
                ..
            } = args;
            assert!(!ephemeral);
            assert!(!no_relay);
            assert!(!no_gossip);
            assert!(!web);
            assert_eq!(port, 3000);
            assert!(bootstrap.is_empty());
            assert!(topic.is_none());
            assert!(topic_secret.is_none());
            assert!(!no_default_bootstrap);
            assert!(!no_default_topic);
            assert!(!no_mdns);
            assert_eq!(iroh_port, 0);
            // Security defaults: loopback only, no token, nobody extra may write.
            assert_eq!(bind, "127.0.0.1".parse::<std::net::IpAddr>().unwrap());
            assert!(web_token.is_none());
            assert!(!world);
            assert!(world_admin_token.is_none());
            assert!(world_module.is_none());
            assert_eq!(world_name, "lobby");
            assert!(world_ssh_port.is_none());
            assert_eq!(world_checkpoint_every, 1000);
            assert_eq!(world_max_open, 256);
            assert_eq!(world_max_sessions, 1024);
            assert_eq!(world_idle_secs, 600);
            assert_eq!(world_boot_window_secs, 600);
            assert!(world_caps.is_empty());
            assert_eq!(world_cap_policy, "deny");
            assert!(roc_bin.is_none());
            assert!(roc_platform.is_none());
            assert!(!world_native);
            assert!(allow_node.is_empty());
            assert!(!open_writes);
        }
        _ => panic!("Expected Serve command"),
    }
}

#[test]
fn test_cli_parse_serve_with_flags() {
    let cli = Cli::parse_from(["id", "serve", "--ephemeral", "--no-relay"]);
    match cli.command {
        Some(Command::Serve(args)) => {
            let ServeArgs {
                ephemeral,
                no_relay,
                no_gossip,
                web,
                port,
                bootstrap,
                topic,
                topic_secret,
                no_default_bootstrap,
                no_default_topic,
                no_mdns,
                iroh_port,
                ..
            } = args;
            assert!(ephemeral);
            assert!(no_relay);
            assert!(!no_gossip);
            assert!(!web);
            assert_eq!(port, 3000);
            assert!(bootstrap.is_empty());
            assert!(topic.is_none());
            assert!(topic_secret.is_none());
            assert!(!no_default_bootstrap);
            assert!(!no_default_topic);
            assert!(!no_mdns);
            assert_eq!(iroh_port, 0);
        }
        _ => panic!("Expected Serve command"),
    }
}

#[test]
fn test_cli_parse_serve_with_bootstrap() {
    let cli = Cli::parse_from(["id", "serve", "--bootstrap", "abc123,def456"]);
    match cli.command {
        Some(Command::Serve(args)) => {
            let ServeArgs { bootstrap, .. } = args;
            assert_eq!(bootstrap, vec!["abc123", "def456"]);
        }
        _ => panic!("Expected Serve command"),
    }
}

#[test]
fn test_cli_parse_serve_with_iroh_port() {
    let cli = Cli::parse_from(["id", "serve", "--iroh-port", "7777"]);
    match cli.command {
        Some(Command::Serve(args)) => {
            let ServeArgs { iroh_port, .. } = args;
            assert_eq!(iroh_port, 7777);
        }
        _ => panic!("Expected Serve command"),
    }
}

#[test]
fn test_cli_parse_serve_with_topic() {
    let cli = Cli::parse_from([
        "id",
        "serve",
        "--topic",
        "my-private-net",
        "--topic-secret",
        "my-secret",
    ]);
    match cli.command {
        Some(Command::Serve(args)) => {
            let ServeArgs {
                topic,
                topic_secret,
                ..
            } = args;
            assert_eq!(topic, Some("my-private-net".to_owned()));
            assert_eq!(topic_secret, Some("my-secret".to_owned()));
        }
        _ => panic!("Expected Serve command"),
    }
}

#[test]
fn test_cli_parse_put_single_file() {
    let cli = Cli::parse_from(["id", "put", "file.txt"]);
    match cli.command {
        Some(Command::Put {
            files,
            content,
            stdin,
            hash_only,
            no_relay,
        }) => {
            assert_eq!(files, vec!["file.txt"]);
            assert!(!content);
            assert!(!stdin);
            assert!(!hash_only);
            assert!(!no_relay);
        }
        _ => panic!("Expected Put command"),
    }
}

#[test]
fn test_cli_parse_put_multiple_files() {
    let cli = Cli::parse_from(["id", "put", "file1.txt", "file2.txt", "file3.txt"]);
    match cli.command {
        Some(Command::Put { files, .. }) => {
            assert_eq!(files, vec!["file1.txt", "file2.txt", "file3.txt"]);
        }
        _ => panic!("Expected Put command"),
    }
}

#[test]
fn test_cli_parse_put_with_rename() {
    let cli = Cli::parse_from(["id", "put", "local.txt:remote.txt"]);
    match cli.command {
        Some(Command::Put { files, .. }) => {
            assert_eq!(files, vec!["local.txt:remote.txt"]);
        }
        _ => panic!("Expected Put command"),
    }
}

#[test]
fn test_cli_parse_put_content_flag() {
    let cli = Cli::parse_from(["id", "put", "--content", "name"]);
    match cli.command {
        Some(Command::Put { content, stdin, .. }) => {
            assert!(content);
            assert!(!stdin);
        }
        _ => panic!("Expected Put command"),
    }
}

#[test]
fn test_cli_parse_put_aliases() {
    // Test all aliases work
    for alias in ["put", "in", "add", "store", "import"] {
        let cli = Cli::parse_from(["id", alias, "file.txt"]);
        assert!(matches!(cli.command, Some(Command::Put { .. })));
    }
}

#[test]
fn test_cli_parse_get_single() {
    let cli = Cli::parse_from(["id", "get", "file.txt"]);
    match cli.command {
        Some(Command::Get {
            sources,
            hash,
            name_only,
            stdout,
            ..
        }) => {
            assert_eq!(sources, vec!["file.txt"]);
            assert!(!hash);
            assert!(!name_only);
            assert!(!stdout);
        }
        _ => panic!("Expected Get command"),
    }
}

#[test]
fn test_cli_parse_get_with_output() {
    let cli = Cli::parse_from(["id", "get", "file.txt:output.txt"]);
    match cli.command {
        Some(Command::Get { sources, .. }) => {
            assert_eq!(sources, vec!["file.txt:output.txt"]);
        }
        _ => panic!("Expected Get command"),
    }
}

#[test]
fn test_cli_parse_get_hash_flag() {
    let cli = Cli::parse_from(["id", "get", "--hash", "abc123"]);
    match cli.command {
        Some(Command::Get {
            hash, name_only, ..
        }) => {
            assert!(hash);
            assert!(!name_only);
        }
        _ => panic!("Expected Get command"),
    }
}

#[test]
fn test_cli_parse_get_stdout_flag() {
    let cli = Cli::parse_from(["id", "get", "--stdout", "file.txt"]);
    match cli.command {
        Some(Command::Get { stdout, .. }) => {
            assert!(stdout);
        }
        _ => panic!("Expected Get command"),
    }
}

#[test]
fn test_cli_parse_cat() {
    let cli = Cli::parse_from(["id", "cat", "file.txt"]);
    match cli.command {
        Some(Command::Cat { sources, .. }) => {
            assert_eq!(sources, vec!["file.txt"]);
        }
        _ => panic!("Expected Cat command"),
    }
}

#[test]
fn test_cli_parse_cat_aliases() {
    for alias in ["cat", "output", "out"] {
        let cli = Cli::parse_from(["id", alias, "file.txt"]);
        assert!(matches!(cli.command, Some(Command::Cat { .. })));
    }
}

#[test]
fn test_cli_parse_find() {
    let cli = Cli::parse_from(["id", "find", "query"]);
    match cli.command {
        Some(Command::Find {
            queries,
            name,
            stdout,
            all,
            format,
            ..
        }) => {
            assert_eq!(queries, vec!["query"]);
            assert!(!name);
            assert!(!stdout);
            assert!(!all);
            assert_eq!(format, "tag");
        }
        _ => panic!("Expected Find command"),
    }
}

#[test]
fn test_cli_parse_find_multiple_queries() {
    let cli = Cli::parse_from(["id", "find", "query1", "query2"]);
    match cli.command {
        Some(Command::Find { queries, .. }) => {
            assert_eq!(queries, vec!["query1", "query2"]);
        }
        _ => panic!("Expected Find command"),
    }
}

#[test]
fn test_cli_parse_find_with_format() {
    let cli = Cli::parse_from(["id", "find", "--format", "group", "query"]);
    match cli.command {
        Some(Command::Find { format, .. }) => {
            assert_eq!(format, "group");
        }
        _ => panic!("Expected Find command"),
    }
}

#[test]
fn test_cli_parse_find_with_invalid_format() {
    for bad in ["gorup", "yaml", "GROUP", ""] {
        assert!(
            Cli::try_parse_from(["id", "find", "--format", bad, "query"]).is_err(),
            "find --format {bad:?} must be rejected at parse time"
        );
    }
    for good in ["tag", "group", "union"] {
        assert!(
            Cli::try_parse_from(["id", "find", "--format", good, "query"]).is_ok(),
            "find --format {good:?} must parse"
        );
    }
}

#[test]
fn test_cli_parse_search_with_invalid_format() {
    for bad in ["gorup", "yaml", "GROUP", ""] {
        assert!(
            Cli::try_parse_from(["id", "search", "--format", bad, "query"]).is_err(),
            "search --format {bad:?} must be rejected at parse time"
        );
    }
    for good in ["tag", "group", "union"] {
        assert!(
            Cli::try_parse_from(["id", "search", "--format", good, "query"]).is_ok(),
            "search --format {good:?} must parse"
        );
    }
}

#[test]
fn test_cli_parse_search() {
    let cli = Cli::parse_from(["id", "search", "query"]);
    match cli.command {
        Some(Command::Search { queries, .. }) => {
            assert_eq!(queries, vec!["query"]);
        }
        _ => panic!("Expected Search command"),
    }
}

#[test]
fn test_cli_parse_list() {
    let cli = Cli::parse_from(["id", "list"]);
    match cli.command {
        Some(Command::List { node, no_relay }) => {
            assert!(node.is_none());
            assert!(!no_relay);
        }
        _ => panic!("Expected List command"),
    }
}

#[test]
fn test_cli_parse_publish_listener() {
    let cli = Cli::parse_from(["id", "publish", "arena", "--listen", "127.0.0.1:8080"]);
    match cli.command {
        Some(Command::Publish {
            world,
            listen,
            path,
            acknowledge_shadow,
        }) => {
            assert_eq!(world, "arena");
            assert_eq!(listen.as_deref(), Some("127.0.0.1:8080"));
            assert!(path.is_none());
            assert!(!acknowledge_shadow);
        }
        _ => panic!("Expected Publish command"),
    }
}

#[test]
fn test_cli_parse_publish_path_with_acknowledgement() {
    let cli = Cli::parse_from([
        "id",
        "publish",
        "arena",
        "--path",
        "/world",
        "--acknowledge-shadow",
    ]);
    match cli.command {
        Some(Command::Publish {
            world,
            listen,
            path,
            acknowledge_shadow,
        }) => {
            assert_eq!(world, "arena");
            assert!(listen.is_none());
            assert_eq!(path.as_deref(), Some("/world"));
            assert!(acknowledge_shadow);
        }
        _ => panic!("Expected Publish command"),
    }
}

#[test]
fn test_cli_parse_unpublish_and_publications() {
    let cli = Cli::parse_from(["id", "unpublish", "--path", "/world"]);
    match cli.command {
        Some(Command::Unpublish { listen, path }) => {
            assert!(listen.is_none());
            assert_eq!(path.as_deref(), Some("/world"));
        }
        _ => panic!("Expected Unpublish command"),
    }
    let cli = Cli::parse_from(["id", "publications"]);
    assert!(matches!(cli.command, Some(Command::Publications)));
}

#[test]
fn test_cli_parse_list_remote() {
    let node_id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let cli = Cli::parse_from(["id", "list", node_id]);
    match cli.command {
        Some(Command::List { node, .. }) => {
            assert_eq!(node, Some(node_id.to_owned()));
        }
        _ => panic!("Expected List command"),
    }
}

#[test]
fn test_cli_parse_repl() {
    let cli = Cli::parse_from(["id", "repl"]);
    match cli.command {
        Some(Command::Repl { node }) => {
            assert!(node.is_none());
        }
        _ => panic!("Expected Repl command"),
    }
}

#[test]
fn test_cli_parse_repl_alias() {
    let cli = Cli::parse_from(["id", "shell"]);
    assert!(matches!(cli.command, Some(Command::Repl { .. })));
}

#[test]
fn test_cli_parse_id() {
    let cli = Cli::parse_from(["id", "id"]);
    assert!(matches!(cli.command, Some(Command::Id { client: false })));
}

#[test]
fn test_cli_parse_key_export_and_import() {
    let export = Cli::parse_from(["id", "key", "export", "backup.idkey", "--client"]);
    assert!(matches!(
        export.command,
        Some(Command::Key {
            command: KeyCommand::Export { client: true, .. }
        })
    ));

    let import = Cli::parse_from(["id", "key", "import", "backup.idkey", "--replace"]);
    assert!(matches!(
        import.command,
        Some(Command::Key {
            command: KeyCommand::Import {
                client: false,
                replace: true,
                ..
            }
        })
    ));

    let export_seed = Cli::parse_from(["id", "key", "export-seed", "--client"]);
    assert!(matches!(
        export_seed.command,
        Some(Command::Key {
            command: KeyCommand::ExportSeed { client: true }
        })
    ));

    let import_seed = Cli::parse_from(["id", "key", "import-seed", "--replace"]);
    assert!(matches!(
        import_seed.command,
        Some(Command::Key {
            command: KeyCommand::ImportSeed {
                client: false,
                replace: true
            }
        })
    ));
}

#[test]
fn test_cli_parse_peers_default() {
    let cli = Cli::parse_from(["id", "peers"]);
    match cli.command {
        Some(Command::Peers {
            gossip,
            rpc,
            depth,
            max_peers,
            timeout,
            bootstrap,
            topic,
            topic_secret,
            no_default_bootstrap,
            no_default_topic,
            no_relay,
            no_mdns,
            node,
            abuse,
        }) => {
            assert!(!gossip);
            assert!(!rpc);
            assert_eq!(depth, 1);
            assert_eq!(max_peers, 1000);
            assert_eq!(timeout, 30);
            assert!(bootstrap.is_empty());
            assert!(topic.is_none());
            assert!(topic_secret.is_none());
            assert!(!no_default_bootstrap);
            assert!(!no_default_topic);
            assert!(!no_relay);
            assert!(!no_mdns);
            assert!(node.is_none());
            assert!(!abuse);
        }
        _ => panic!("Expected Peers command"),
    }
}

#[test]
fn test_cli_parse_peers_gossip_mode() {
    let cli = Cli::parse_from([
        "id",
        "peers",
        "--gossip",
        "--topic",
        "my-net",
        "--topic-secret",
        "s3cret",
    ]);
    match cli.command {
        Some(Command::Peers {
            gossip,
            rpc,
            topic,
            topic_secret,
            ..
        }) => {
            assert!(gossip);
            assert!(!rpc);
            assert_eq!(topic, Some("my-net".to_owned()));
            assert_eq!(topic_secret, Some("s3cret".to_owned()));
        }
        _ => panic!("Expected Peers command"),
    }
}

#[test]
fn test_cli_parse_peers_abuse_flag() {
    let cli = Cli::parse_from(["id", "peers", "--abuse"]);
    match cli.command {
        Some(Command::Peers { abuse, .. }) => assert!(abuse),
        _ => panic!("Expected Peers command"),
    }
}

#[test]
fn test_cli_parse_peers_rpc_with_depth() {
    let cli = Cli::parse_from(["id", "peers", "--rpc", "--depth", "3", "--max-peers", "50"]);
    match cli.command {
        Some(Command::Peers {
            rpc,
            gossip,
            depth,
            max_peers,
            ..
        }) => {
            assert!(rpc);
            assert!(!gossip);
            assert_eq!(depth, 3);
            assert_eq!(max_peers, 50);
        }
        _ => panic!("Expected Peers command"),
    }
}

#[test]
fn test_cli_peers_depth_rejects_negative_and_preserves_non_negative_values() {
    for argv in [
        vec!["id", "peers", "--depth=-1"],
        vec!["id", "peers", "--depth", "-1"],
    ] {
        let err = Cli::try_parse_from(argv).unwrap_err();
        assert!(
            err.to_string().contains("depth"),
            "error should name depth: {err}"
        );
    }

    for depth in ["0", "1", "2"] {
        let cli = Cli::try_parse_from(["id", "peers", "--depth", depth]).unwrap();
        match cli.command {
            Some(Command::Peers { depth: parsed, .. }) => {
                assert_eq!(parsed, depth.parse::<i32>().unwrap());
            }
            _ => panic!("Expected Peers command"),
        }
    }
}

#[test]
fn test_cli_parse_peers_with_node() {
    let node_id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let cli = Cli::parse_from(["id", "peers", node_id]);
    match cli.command {
        Some(Command::Peers { node, .. }) => {
            assert_eq!(node, Some(node_id.to_owned()));
        }
        _ => panic!("Expected Peers command"),
    }
}

#[test]
fn test_cli_parse_peers_with_bootstrap() {
    let cli = Cli::parse_from(["id", "peers", "--gossip", "--bootstrap", "abc123,def456"]);
    match cli.command {
        Some(Command::Peers { bootstrap, .. }) => {
            assert_eq!(bootstrap, vec!["abc123", "def456"]);
        }
        _ => panic!("Expected Peers command"),
    }
}

#[test]
fn test_cli_parse_get_hash() {
    let cli = Cli::parse_from(["id", "get-hash", "abc123", "output.txt"]);
    match cli.command {
        Some(Command::GetHash { hash, output }) => {
            assert_eq!(hash, "abc123");
            assert_eq!(output, "output.txt");
        }
        _ => panic!("Expected GetHash command"),
    }
}

#[test]
fn test_cli_parse_put_hash() {
    let cli = Cli::parse_from(["id", "put-hash", "file.txt"]);
    match cli.command {
        Some(Command::PutHash { source }) => {
            assert_eq!(source, "file.txt");
        }
        _ => panic!("Expected PutHash command"),
    }
}

#[test]
fn test_cli_parse_doc_content() {
    let cli = Cli::parse_from(["id", "doc", "content", "abc123"]);
    match cli.command {
        Some(Command::Doc {
            command: DocCommand::Content { document, output },
            ..
        }) => {
            assert_eq!(document, "abc123");
            assert_eq!(output, "-");
        }
        _ => panic!("Expected Doc::Content command"),
    }

    let cli = Cli::parse_from(["id", "doc", "content", "abc123", "out.txt"]);
    match cli.command {
        Some(Command::Doc {
            command: DocCommand::Content { document, output },
            ..
        }) => {
            assert_eq!(document, "abc123");
            assert_eq!(output, "out.txt");
        }
        _ => panic!("Expected Doc::Content command"),
    }
}

#[test]
fn test_cli_verify() {
    // Verify CLI structure is valid
    Cli::command().debug_assert();
}

#[test]
fn test_cli_rejects_zero_result_limits() {
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            // #3059: an explicit zero limit is an error at the CLI boundary, never a
            // silent empty result (find/search/show/peek share the flag pair).
            for cmd in ["find", "search", "show", "peek"] {
                assert!(
                    Cli::try_parse_from(["id", cmd, "--first", "0", "query"]).is_err(),
                    "{cmd} --first 0 must be rejected"
                );
                assert!(
                    Cli::try_parse_from(["id", cmd, "--last", "0", "query"]).is_err(),
                    "{cmd} --last 0 must be rejected"
                );
            }
            assert!(Cli::try_parse_from(["id", "peek", "--lines", "0", "query"]).is_err());
            assert!(Cli::try_parse_from(["id", "peek", "-n", "0", "query"]).is_err());
            // Bare flags keep their defaults (1 / 1 / 5 lines).
            assert!(Cli::try_parse_from(["id", "find", "query", "--first"]).is_ok());
            assert!(Cli::try_parse_from(["id", "find", "query", "--last"]).is_ok());
            assert!(Cli::try_parse_from(["id", "peek", "query"]).is_ok());
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn test_cli_rejects_first_and_last_together() {
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            assert!(
                Cli::try_parse_from(["id", "show", "--first", "2", "--last", "3", "query"])
                    .is_err()
            );
            assert!(
                Cli::try_parse_from(["id", "peek", "--first", "2", "--last", "3", "query"])
                    .is_err()
            );
            assert!(
                Cli::try_parse_from(["id", "find", "--first", "2", "--last", "3", "query"])
                    .is_err()
            );
            assert!(
                Cli::try_parse_from(["id", "search", "--first", "2", "--last", "3", "query"])
                    .is_err()
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

// Tests for Show command
#[test]
fn test_cli_parse_show() {
    let cli = Cli::parse_from(["id", "show", "query"]);
    match cli.command {
        Some(Command::Show {
            queries,
            name,
            all,
            output,
            first,
            last,
            exclude,
            ..
        }) => {
            assert_eq!(queries, vec!["query"]);
            assert!(!name);
            assert!(!all);
            assert!(output.is_none());
            assert!(first.is_none());
            assert!(last.is_none());
            assert!(exclude.is_empty());
        }
        _ => panic!("Expected Show command"),
    }
}

#[test]
fn test_cli_parse_show_alias_view() {
    let cli = Cli::parse_from(["id", "view", "query"]);
    assert!(matches!(cli.command, Some(Command::Show { .. })));
}

#[test]
fn test_cli_parse_show_with_all() {
    let cli = Cli::parse_from(["id", "show", "--all", "query"]);
    match cli.command {
        Some(Command::Show { all, .. }) => {
            assert!(all);
        }
        _ => panic!("Expected Show command"),
    }
}

#[test]
fn test_cli_parse_show_with_output() {
    let cli = Cli::parse_from(["id", "show", "-o", "output.txt", "query"]);
    match cli.command {
        Some(Command::Show { output, .. }) => {
            assert_eq!(output, Some("output.txt".to_owned()));
        }
        _ => panic!("Expected Show command"),
    }
}

#[test]
fn test_cli_parse_show_with_filters() {
    let cli = Cli::parse_from([
        "id",
        "show",
        "--first",
        "3",
        "--exclude",
        ".bak",
        "--exclude",
        ".tmp",
        "query",
    ]);
    match cli.command {
        Some(Command::Show { first, exclude, .. }) => {
            assert_eq!(first, Some(3));
            assert_eq!(exclude, vec![".bak", ".tmp"]);
        }
        _ => panic!("Expected Show command"),
    }
}

// Tests for Peek command
#[test]
fn test_cli_parse_peek() {
    let cli = Cli::parse_from(["id", "peek", "query"]);
    match cli.command {
        Some(Command::Peek {
            queries,
            lines,
            head_only,
            tail_only,
            chars,
            words,
            quiet,
            ..
        }) => {
            assert_eq!(queries, vec!["query"]);
            assert_eq!(lines, 5); // default
            assert!(!head_only);
            assert!(!tail_only);
            assert!(!chars);
            assert!(!words);
            assert!(!quiet);
        }
        _ => panic!("Expected Peek command"),
    }
}

#[test]
fn test_cli_parse_peek_with_lines() {
    let cli = Cli::parse_from(["id", "peek", "-n", "10", "query"]);
    match cli.command {
        Some(Command::Peek { lines, .. }) => {
            assert_eq!(lines, 10);
        }
        _ => panic!("Expected Peek command"),
    }
}

#[test]
fn test_cli_parse_peek_head_only() {
    let cli = Cli::parse_from(["id", "peek", "--head-only", "query"]);
    match cli.command {
        Some(Command::Peek {
            head_only,
            tail_only,
            ..
        }) => {
            assert!(head_only);
            assert!(!tail_only);
        }
        _ => panic!("Expected Peek command"),
    }
}

#[test]
fn test_cli_parse_peek_tail_only() {
    let cli = Cli::parse_from(["id", "peek", "--tail-only", "query"]);
    match cli.command {
        Some(Command::Peek {
            head_only,
            tail_only,
            ..
        }) => {
            assert!(!head_only);
            assert!(tail_only);
        }
        _ => panic!("Expected Peek command"),
    }
}

#[test]
fn test_cli_parse_peek_chars() {
    let cli = Cli::parse_from(["id", "peek", "--chars", "-n", "100", "query"]);
    match cli.command {
        Some(Command::Peek {
            chars,
            words,
            lines,
            ..
        }) => {
            assert!(chars);
            assert!(!words);
            assert_eq!(lines, 100);
        }
        _ => panic!("Expected Peek command"),
    }
}

#[test]
fn test_cli_parse_peek_words() {
    let cli = Cli::parse_from(["id", "peek", "--words", "-n", "50", "query"]);
    match cli.command {
        Some(Command::Peek {
            chars,
            words,
            lines,
            ..
        }) => {
            assert!(!chars);
            assert!(words);
            assert_eq!(lines, 50);
        }
        _ => panic!("Expected Peek command"),
    }
}

#[test]
fn test_cli_parse_peek_quiet() {
    let cli = Cli::parse_from(["id", "peek", "-q", "query"]);
    match cli.command {
        Some(Command::Peek { quiet, .. }) => {
            assert!(quiet);
        }
        _ => panic!("Expected Peek command"),
    }
}

#[test]
fn test_cli_parse_peek_with_output() {
    let cli = Cli::parse_from(["id", "peek", "-o", "output.txt", "query"]);
    match cli.command {
        Some(Command::Peek { output, .. }) => {
            assert_eq!(output, Some("output.txt".to_owned()));
        }
        _ => panic!("Expected Peek command"),
    }
}

#[test]
fn test_cli_parse_peek_all() {
    let cli = Cli::parse_from(["id", "peek", "--all", "query"]);
    match cli.command {
        Some(Command::Peek { all, .. }) => {
            assert!(all);
        }
        _ => panic!("Expected Peek command"),
    }
}

#[test]
fn test_cli_parse_peek_with_filters() {
    let cli = Cli::parse_from(["id", "peek", "--first", "2", "--exclude", ".bak", "query"]);
    match cli.command {
        Some(Command::Peek { first, exclude, .. }) => {
            assert_eq!(first, Some(2));
            assert_eq!(exclude, vec![".bak"]);
        }
        _ => panic!("Expected Peek command"),
    }
}

// Tests for find/search with new flags
#[test]
fn test_cli_parse_find_with_first() {
    let cli = Cli::parse_from(["id", "find", "--first", "3", "query"]);
    match cli.command {
        Some(Command::Find { first, .. }) => {
            assert_eq!(first, Some(3));
        }
        _ => panic!("Expected Find command"),
    }
}

#[test]
fn test_cli_parse_find_with_first_default() {
    // When --first is at the end, it uses the default value
    let cli = Cli::parse_from(["id", "find", "query", "--first"]);
    match cli.command {
        Some(Command::Find { first, .. }) => {
            assert_eq!(first, Some(1)); // default missing value
        }
        _ => panic!("Expected Find command"),
    }
}

#[test]
fn test_cli_parse_find_with_last() {
    let cli = Cli::parse_from(["id", "find", "--last", "5", "query"]);
    match cli.command {
        Some(Command::Find { last, .. }) => {
            assert_eq!(last, Some(5));
        }
        _ => panic!("Expected Find command"),
    }
}

#[test]
fn test_cli_parse_find_with_last_default() {
    // When --last is at the end, it uses the default value
    let cli = Cli::parse_from(["id", "find", "query", "--last"]);
    match cli.command {
        Some(Command::Find { last, .. }) => {
            assert_eq!(last, Some(1)); // default missing value
        }
        _ => panic!("Expected Find command"),
    }
}

#[test]
fn test_cli_parse_find_with_count() {
    let cli = Cli::parse_from(["id", "find", "--count", "query"]);
    match cli.command {
        Some(Command::Find { count, .. }) => {
            assert!(count);
        }
        _ => panic!("Expected Find command"),
    }
}

#[test]
fn test_cli_parse_find_with_exclude() {
    let cli = Cli::parse_from([
        "id",
        "find",
        "--exclude",
        ".bak",
        "--exclude",
        ".tmp",
        "query",
    ]);
    match cli.command {
        Some(Command::Find { exclude, .. }) => {
            assert_eq!(exclude, vec![".bak", ".tmp"]);
        }
        _ => panic!("Expected Find command"),
    }
}

#[test]
fn test_cli_parse_search_with_first() {
    let cli = Cli::parse_from(["id", "search", "--first", "3", "query"]);
    match cli.command {
        Some(Command::Search { first, .. }) => {
            assert_eq!(first, Some(3));
        }
        _ => panic!("Expected Search command"),
    }
}

#[test]
fn test_cli_parse_search_with_last() {
    let cli = Cli::parse_from(["id", "search", "--last", "5", "query"]);
    match cli.command {
        Some(Command::Search { last, .. }) => {
            assert_eq!(last, Some(5));
        }
        _ => panic!("Expected Search command"),
    }
}

#[test]
fn test_cli_parse_search_with_count() {
    let cli = Cli::parse_from(["id", "search", "--count", "query"]);
    match cli.command {
        Some(Command::Search { count, .. }) => {
            assert!(count);
        }
        _ => panic!("Expected Search command"),
    }
}

#[test]
fn test_cli_parse_search_with_exclude() {
    let cli = Cli::parse_from([
        "id",
        "search",
        "--exclude",
        ".bak",
        "--exclude",
        ".tmp",
        "query",
    ]);
    match cli.command {
        Some(Command::Search { exclude, .. }) => {
            assert_eq!(exclude, vec![".bak", ".tmp"]);
        }
        _ => panic!("Expected Search command"),
    }
}

#[test]
fn test_cli_parse_find_combined_options() {
    let cli = Cli::parse_from([
        "id",
        "find",
        "--first",
        "10",
        "--exclude",
        ".bak",
        "--count",
        "query",
    ]);
    match cli.command {
        Some(Command::Find {
            first,
            count,
            exclude,
            ..
        }) => {
            assert_eq!(first, Some(10));
            assert!(count);
            assert_eq!(exclude, vec![".bak"]);
        }
        _ => panic!("Expected Find command"),
    }
}

// Tests for new discovery flags
#[test]
fn test_cli_parse_serve_no_gossip() {
    let cli = Cli::parse_from(["id", "serve", "--no-gossip"]);
    match cli.command {
        Some(Command::Serve(args)) => {
            let ServeArgs { no_gossip, .. } = args;
            assert!(no_gossip);
        }
        _ => panic!("Expected Serve command"),
    }
}

#[test]
fn test_cli_serve_gossip_overrides_are_explicit_and_mutually_exclusive() {
    assert!(Cli::try_parse_from(["id", "serve", "--gossip"]).is_ok());
    assert!(Cli::try_parse_from(["id", "serve", "--gossip", "--no-gossip"]).is_err());
}

#[test]
fn test_cli_parse_serve_no_default_bootstrap() {
    let cli = Cli::parse_from(["id", "serve", "--no-default-bootstrap"]);
    match cli.command {
        Some(Command::Serve(args)) => {
            let ServeArgs {
                no_default_bootstrap,
                ..
            } = args;
            assert!(no_default_bootstrap);
        }
        _ => panic!("Expected Serve command"),
    }
}

#[test]
fn test_cli_parse_serve_no_default_topic() {
    let cli = Cli::parse_from(["id", "serve", "--no-default-topic"]);
    match cli.command {
        Some(Command::Serve(args)) => {
            let ServeArgs {
                no_default_topic, ..
            } = args;
            assert!(no_default_topic);
        }
        _ => panic!("Expected Serve command"),
    }
}

#[test]
fn test_cli_rejects_serve_replace_defaults() {
    assert!(Cli::try_parse_from(["id", "serve", "--replace-defaults"]).is_err());
}

#[test]
fn test_cli_rejects_top_level_ls() {
    // CLI-1 (#13363): top-level `list` has no `ls` alias in the clap
    // parser; `ls` exists only in the REPL and on `tag list`.
    assert!(Cli::try_parse_from(["id", "ls"]).is_err());
}

#[test]
fn test_cli_parses_match_audit() {
    // CLI-10 (#13363): `match-audit` parses in every build, like
    // `world`; a build without the `sandbox` feature rejects it at
    // dispatch time instead of at parse time.
    let cli = Cli::try_parse_from(["id", "match-audit", "verify", "log.bin"])
        .expect("match-audit verify should parse");
    assert!(matches!(
        cli.command,
        Some(Command::MatchAudit(MatchAuditCommand::Verify { .. }))
    ));
}

#[test]
fn test_cli_parse_serve_all_new_flags() {
    let cli = Cli::parse_from([
        "id",
        "serve",
        "--no-gossip",
        "--no-default-bootstrap",
        "--no-default-topic",
        "--no-mdns",
    ]);
    match cli.command {
        Some(Command::Serve(args)) => {
            let ServeArgs {
                no_gossip,
                no_default_bootstrap,
                no_default_topic,
                no_mdns,
                ..
            } = args;
            assert!(no_gossip);
            assert!(no_default_bootstrap);
            assert!(no_default_topic);
            assert!(no_mdns);
        }
        _ => panic!("Expected Serve command"),
    }
}

#[test]
fn test_cli_parse_peers_no_default_bootstrap() {
    let cli = Cli::parse_from(["id", "peers", "--gossip", "--no-default-bootstrap"]);
    match cli.command {
        Some(Command::Peers {
            no_default_bootstrap,
            ..
        }) => {
            assert!(no_default_bootstrap);
        }
        _ => panic!("Expected Peers command"),
    }
}

#[test]
fn test_cli_parse_peers_no_default_topic() {
    let cli = Cli::parse_from(["id", "peers", "--gossip", "--no-default-topic"]);
    match cli.command {
        Some(Command::Peers {
            no_default_topic, ..
        }) => {
            assert!(no_default_topic);
        }
        _ => panic!("Expected Peers command"),
    }
}

#[test]
fn test_cli_rejects_peers_replace_defaults() {
    assert!(Cli::try_parse_from(["id", "peers", "--gossip", "--replace-defaults"]).is_err());
}

#[test]
fn test_cli_parse_serve_no_mdns() {
    let cli = Cli::parse_from(["id", "serve", "--no-mdns"]);
    match cli.command {
        Some(Command::Serve(args)) => {
            let ServeArgs { no_mdns, .. } = args;
            assert!(no_mdns);
        }
        _ => panic!("Expected Serve command"),
    }
}

#[test]
fn test_cli_parse_serve_dht() {
    let cli = Cli::parse_from(["id", "serve", "--dht"]);
    match cli.command {
        Some(Command::Serve(args)) => {
            let ServeArgs { dht, .. } = args;
            assert!(dht);
        }
        _ => panic!("Expected Serve command"),
    }
}

#[test]
fn test_cli_parse_peers_no_mdns() {
    let cli = Cli::parse_from(["id", "peers", "--no-mdns"]);
    match cli.command {
        Some(Command::Peers { no_mdns, .. }) => {
            assert!(no_mdns);
        }
        _ => panic!("Expected Peers command"),
    }
}

// ========================================================================
// Tag command and alias tests
// ========================================================================

#[test]
fn test_cli_parse_tag_set() {
    let cli = Cli::parse_from(["id", "tag", "set", "README.md", "priority", "high"]);
    match cli.command {
        Some(Command::Tag(TagCommand::Set {
            file, key, value, ..
        })) => {
            assert_eq!(file, "README.md");
            assert_eq!(key, "priority");
            assert_eq!(value, Some("high".to_owned()));
        }
        _ => panic!("Expected Tag Set command"),
    }
}

#[test]
fn test_cli_parse_tag_set_without_value() {
    let cli = Cli::parse_from(["id", "tag", "set", "README.md", "pinned"]);
    match cli.command {
        Some(Command::Tag(TagCommand::Set {
            file, key, value, ..
        })) => {
            assert_eq!(file, "README.md");
            assert_eq!(key, "pinned");
            assert!(value.is_none());
        }
        _ => panic!("Expected Tag Set command"),
    }
}

#[test]
fn test_cli_parse_tag_set_alias_add() {
    let cli = Cli::parse_from(["id", "tag", "add", "file.txt", "label", "rust"]);
    match cli.command {
        Some(Command::Tag(TagCommand::Set {
            file, key, value, ..
        })) => {
            assert_eq!(file, "file.txt");
            assert_eq!(key, "label");
            assert_eq!(value, Some("rust".to_owned()));
        }
        _ => panic!("Expected Tag Set (via add alias)"),
    }
}

#[test]
fn test_cli_parse_tag_del() {
    let cli = Cli::parse_from(["id", "tag", "del", "file.txt", "label", "rust"]);
    match cli.command {
        Some(Command::Tag(TagCommand::Del {
            file, key, value, ..
        })) => {
            assert_eq!(file, "file.txt");
            assert_eq!(key, "label");
            assert_eq!(value, Some("rust".to_owned()));
        }
        _ => panic!("Expected Tag Del command"),
    }
}

#[test]
fn test_cli_parse_tag_del_without_value() {
    let cli = Cli::parse_from(["id", "tag", "del", "file.txt", "pinned"]);
    match cli.command {
        Some(Command::Tag(TagCommand::Del {
            file, key, value, ..
        })) => {
            assert_eq!(file, "file.txt");
            assert_eq!(key, "pinned");
            assert!(value.is_none());
        }
        _ => panic!("Expected Tag Del command"),
    }
}

#[test]
fn test_cli_parse_tag_del_aliases() {
    for alias in ["del", "rm", "remove", "rem", "delete", "unset"] {
        let cli = Cli::parse_from(["id", "tag", alias, "file.txt", "key"]);
        assert!(
            matches!(cli.command, Some(Command::Tag(TagCommand::Del { .. }))),
            "alias '{alias}' should parse as Tag Del"
        );
    }
}

#[test]
fn test_cli_parse_tag_list() {
    let cli = Cli::parse_from(["id", "tag", "list"]);
    match cli.command {
        Some(Command::Tag(TagCommand::List { file, .. })) => {
            assert!(file.is_none());
        }
        _ => panic!("Expected Tag List command"),
    }
}

#[test]
fn test_cli_parse_tag_list_with_file() {
    let cli = Cli::parse_from(["id", "tag", "list", "README.md"]);
    match cli.command {
        Some(Command::Tag(TagCommand::List { file, .. })) => {
            assert_eq!(file, Some("README.md".to_owned()));
        }
        _ => panic!("Expected Tag List command"),
    }
}

#[test]
fn test_cli_parse_tag_list_alias_ls() {
    let cli = Cli::parse_from(["id", "tag", "ls"]);
    assert!(matches!(
        cli.command,
        Some(Command::Tag(TagCommand::List { .. }))
    ));
}

#[test]
fn test_cli_parse_tag_search() {
    let cli = Cli::parse_from(["id", "tag", "search", "priority"]);
    match cli.command {
        Some(Command::Tag(TagCommand::Search { query, .. })) => {
            assert_eq!(query, vec!["priority"]);
        }
        _ => panic!("Expected Tag Search command"),
    }
}

#[test]
fn test_cli_parse_tag_search_with_value() {
    let cli = Cli::parse_from(["id", "tag", "search", "priority:high"]);
    match cli.command {
        Some(Command::Tag(TagCommand::Search { query, .. })) => {
            assert_eq!(query, vec!["priority:high"]);
        }
        _ => panic!("Expected Tag Search command"),
    }
}

#[test]
fn test_cli_parse_tag_search_multi_terms() {
    let cli = Cli::parse_from(["id", "tag", "search", "name:", ":high"]);
    match cli.command {
        Some(Command::Tag(TagCommand::Search { query, .. })) => {
            assert_eq!(query, vec!["name:", ":high"]);
        }
        _ => panic!("Expected Tag Search command"),
    }
}

#[test]
fn test_cli_parse_tag_search_alias_find() {
    let cli = Cli::parse_from(["id", "tag", "find", "label"]);
    assert!(matches!(
        cli.command,
        Some(Command::Tag(TagCommand::Search { .. }))
    ));
}

#[test]
fn test_cli_parse_label_alias() {
    // "label" should work as alias for "tag"
    let cli = Cli::parse_from(["id", "label", "set", "file.txt", "key", "val"]);
    match cli.command {
        Some(Command::Tag(TagCommand::Set {
            file, key, value, ..
        })) => {
            assert_eq!(file, "file.txt");
            assert_eq!(key, "key");
            assert_eq!(value, Some("val".to_owned()));
        }
        _ => panic!("Expected Tag Set via 'label' alias"),
    }
}

#[test]
fn test_cli_parse_link_alias() {
    // "link" should work as alias for "tag"
    let cli = Cli::parse_from(["id", "link", "set", "file.txt", "key", "val"]);
    match cli.command {
        Some(Command::Tag(TagCommand::Set {
            file, key, value, ..
        })) => {
            assert_eq!(file, "file.txt");
            assert_eq!(key, "key");
            assert_eq!(value, Some("val".to_owned()));
        }
        _ => panic!("Expected Tag Set via 'link' alias"),
    }
}

#[test]
fn test_cli_parse_label_del() {
    let cli = Cli::parse_from(["id", "label", "del", "file.txt", "key"]);
    assert!(
        matches!(cli.command, Some(Command::Tag(TagCommand::Del { .. }))),
        "label del should parse as Tag Del"
    );
}

#[test]
fn test_cli_parse_link_list() {
    let cli = Cli::parse_from(["id", "link", "list"]);
    assert!(
        matches!(cli.command, Some(Command::Tag(TagCommand::List { .. }))),
        "link list should parse as Tag List"
    );
}

#[test]
fn test_cli_parse_label_search() {
    let cli = Cli::parse_from(["id", "label", "search", "priority"]);
    assert!(
        matches!(cli.command, Some(Command::Tag(TagCommand::Search { .. }))),
        "label search should parse as Tag Search"
    );
}

#[test]
fn test_cli_parse_world_mail_flags() {
    let smtp = [
        "id",
        "serve",
        "--world",
        "--world-admin-token",
        "x",
        "--world-mail-smtp",
        "127.0.0.1:25",
        "--world-mail-from",
        "id@example.test",
    ];
    let cli = match Cli::try_parse_from(smtp) {
        Ok(cli) => cli,
        Err(error) => panic!("smtp mail with a sender should parse: {error}"),
    };
    match cli.command {
        Some(Command::Serve(args)) => {
            let ServeArgs {
                world_mail_smtp,
                world_mail_from,
                ..
            } = args;
            assert_eq!(world_mail_smtp, Some("127.0.0.1:25".parse().unwrap()));
            assert_eq!(world_mail_from.as_deref(), Some("id@example.test"));
        }
        _ => panic!("Expected Serve command"),
    }
    let refused = |args: &[&str]| {
        let mut full = vec!["id", "serve", "--world", "--world-admin-token", "x"];
        full.extend_from_slice(args);
        Cli::try_parse_from(full).is_err()
    };
    assert!(
        refused(&["--world-mail-smtp", "127.0.0.1:25"]),
        "needs a sender"
    );
    assert!(
        refused(&["--world-mail-from", "id@example.test"]),
        "needs a relay"
    );
    assert!(refused(&[
        "--world-mail-smtp",
        "127.0.0.1:25",
        "--world-mail-from",
        "id@example.test",
        "--world-mail-outbox",
        "/tmp/mail",
    ]));
    assert!(refused(&[
        "--world-mail-smtp",
        "127.0.0.1:25",
        "--world-mail-from",
        "id@example.test",
        "--world-mail-command",
        "/bin/true",
    ]));
    assert!(refused(&[
        "--world-mail-outbox",
        "/tmp/mail",
        "--world-mail-command",
        "/bin/true"
    ]));
    assert!(
        Cli::try_parse_from([
            "id",
            "serve",
            "--world-mail-smtp",
            "127.0.0.1:25",
            "--world-mail-from",
            "x@y.z"
        ])
        .is_err(),
        "mail needs --world"
    );
    assert!(
        Cli::try_parse_from([
            "id",
            "serve",
            "--world",
            "--world-mail-smtp",
            "not-an-address",
            "--world-mail-from",
            "x@y.z"
        ])
        .is_err()
    );
}

#[test]
fn test_cli_parse_serve_web_listen_repeatable() {
    // #2145: --web-listen is repeatable and parses each IP:PORT.
    let cli = Cli::parse_from([
        "id",
        "serve",
        "--web",
        "--web-listen",
        "127.0.0.1:4001",
        "--web-listen",
        "[::1]:4002",
    ]);
    match cli.command {
        Some(Command::Serve(args)) => {
            let ServeArgs { web_listen, .. } = args;
            assert_eq!(
                web_listen,
                vec![
                    "127.0.0.1:4001".parse::<SocketAddr>().unwrap(),
                    "[::1]:4002".parse::<SocketAddr>().unwrap(),
                ]
            );
        }
        _ => panic!("Expected Serve command"),
    }
    // A malformed listener is a parse error, not a silent skip.
    assert!(Cli::try_parse_from(["id", "serve", "--web-listen", "nope"]).is_err());
}

#[test]
fn test_cli_parse_serve_security_flags() {
    let cli = Cli::parse_from([
        "id",
        "serve",
        "--bind",
        "0.0.0.0",
        "--web-token",
        "t0k",
        "--web",
        "--world",
        "--world-admin-token",
        "admin",
        "--allow-node",
        "aa,bb",
        "--allow-node",
        "cc",
        "--open-writes",
    ]);
    match cli.command {
        Some(Command::Serve(args)) => {
            let ServeArgs {
                bind,
                web_token,
                web,
                world,
                world_admin_token,
                allow_node,
                open_writes,
                ..
            } = args;
            assert!(bind.is_unspecified());
            assert_eq!(web_token.as_deref(), Some("t0k"));
            assert!(web);
            assert!(world);
            assert_eq!(world_admin_token.as_deref(), Some("admin"));
            assert_eq!(allow_node, vec!["aa", "bb", "cc"]);
            assert!(open_writes);
        }
        _ => panic!("Expected Serve command"),
    }
    // Over Iroh the lobby needs no web server.
    assert!(Cli::try_parse_from(["id", "serve", "--world", "--world-admin-token", "x"]).is_ok());
    // `--world` is refused before serve starts when its prerequisites are
    // missing (clap requirement plus the serve-time validation).
    assert!(Cli::try_parse_from(["id", "serve", "--world"]).is_err());
    assert!(
        Cli::try_parse_from(["id", "serve", "--web", "--world"]).is_err(),
        "an empty admin secret must not be accepted"
    );
}

#[test]
fn source_only_mode_is_opt_in_and_requires_world_mode() {
    let parsed = Cli::try_parse_from([
        "id",
        "serve",
        "--world",
        "--world-admin-token",
        "admin",
        "--world-source-only",
    ])
    .unwrap();
    match parsed.command {
        Some(Command::Serve(args)) => assert!(args.world_source_only),
        _ => panic!("Expected Serve command"),
    }
    assert!(Cli::try_parse_from(["id", "serve", "--world-source-only"]).is_err());
    assert!(
        Cli::try_parse_from([
            "id",
            "serve",
            "--world",
            "--world-admin-token",
            "admin",
            "--world-source-only",
            "--world-module",
            "trusted.wasm",
        ])
        .is_err()
    );
}

#[test]
fn module_trust_keys_are_repeatable_and_require_world_mode() {
    let first = "11".repeat(32);
    let second = "22".repeat(32);
    let parsed = Cli::try_parse_from([
        "id",
        "serve",
        "--world",
        "--world-admin-token",
        "admin",
        "--world-module-trust-key",
        &first,
        "--world-module-trust-key",
        &second,
    ])
    .unwrap();
    match parsed.command {
        Some(Command::Serve(args)) => {
            assert_eq!(args.world_module_trust_keys, vec![first.clone(), second])
        }
        _ => panic!("Expected Serve command"),
    }
    assert!(Cli::try_parse_from(["id", "serve", "--world-module-trust-key", &first]).is_err());
}

#[test]
fn test_cli_parse_web_cookie_secure() {
    match Cli::parse_from(["id", "serve", "--web", "--web-cookie-secure"]).command {
        Some(Command::Serve(args)) => assert!(args.web_cookie_secure),
        _ => panic!("Expected Serve command"),
    }
    match Cli::parse_from(["id", "serve", "--web"]).command {
        Some(Command::Serve(args)) => assert!(!args.web_cookie_secure),
        _ => panic!("Expected Serve command"),
    }
}

#[test]
fn test_cli_parse_gallery_dir_and_publish() {
    match Cli::parse_from(["id", "serve", "--web", "--gallery-dir", "/srv/gallery"]).command {
        Some(Command::Serve(args)) => assert_eq!(
            args.gallery_dir.as_deref(),
            Some(std::path::Path::new("/srv/gallery"))
        ),
        _ => panic!("Expected Serve command"),
    }
    match Cli::parse_from(["id", "serve", "--web"]).command {
        Some(Command::Serve(args)) => assert!(args.gallery_dir.is_none()),
        _ => panic!("Expected Serve command"),
    }
    match Cli::parse_from([
        "id",
        "serve",
        "--gallery-dir",
        "/srv/gallery",
        "--gallery-accept-publish",
    ])
    .command
    {
        Some(Command::Serve(args)) => {
            assert!(
                args.gallery_dir.is_some(),
                "the Iroh registry needs no --web"
            );
            assert!(args.gallery_accept_publish);
        }
        _ => panic!("Expected Serve command"),
    }
    assert!(Cli::try_parse_from(["id", "serve", "--gallery-accept-publish"]).is_err());
}

#[test]
fn test_cli_parse_world_boot_window() {
    let cli = Cli::parse_from([
        "id",
        "serve",
        "--world",
        "--world-admin-token",
        "x",
        "--world-boot-window-secs",
        "0",
    ]);
    match cli.command {
        Some(Command::Serve(args)) => assert_eq!(args.world_boot_window_secs, 0),
        _ => panic!("Expected Serve command"),
    }
}

#[test]
fn test_cli_parse_id_client() {
    let cli = Cli::parse_from(["id", "id", "--client"]);
    assert!(matches!(cli.command, Some(Command::Id { client: true })));
}

#[test]
fn test_cli_parse_tag_set_binary_inputs() {
    let cli = Cli::parse_from(["id", "tag", "set", "f", "k", "--value-hex", "deadbeef"]);
    match cli.command {
        Some(Command::Tag(TagCommand::Set {
            value,
            value_hex,
            value_file,
            ..
        })) => {
            assert!(value.is_none());
            assert_eq!(value_hex.as_deref(), Some("deadbeef"));
            assert!(value_file.is_none());
        }
        _ => panic!("Expected Tag Set command"),
    }
    let cli = Cli::parse_from(["id", "tag", "set", "f", "k", "--value-file", "-"]);
    assert!(matches!(
        cli.command,
        Some(Command::Tag(TagCommand::Set {
            value_file: Some(_),
            ..
        }))
    ));
}

#[test]
fn test_cli_tag_set_value_inputs_conflict() {
    assert!(Cli::try_parse_from(["id", "tag", "set", "f", "k", "v", "--value-hex", "00"]).is_err());
    assert!(
        Cli::try_parse_from([
            "id",
            "tag",
            "set",
            "f",
            "k",
            "--value-hex",
            "00",
            "--value-file",
            "x"
        ])
        .is_err()
    );
    assert!(Cli::try_parse_from(["id", "tag", "del", "f", "k", "v", "--value-hex", "00"]).is_err());
}

#[test]
fn test_cli_parse_world_sharing_commands() {
    let node = "a".repeat(64);
    let cli = Cli::parse_from([
        "id",
        "world",
        "cross-world-write",
        &node,
        "on",
        "--admin-token",
        "t",
    ]);
    match cli.command {
        Some(Command::World(cmd)) => match *cmd {
            WorldCommand::CrossWorldWrite(args) => {
                let WorldCrossWorldWriteArgs { mode, .. } = args;
                assert_eq!(mode, "on");
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
    let cli = Cli::parse_from([
        "id",
        "world",
        "force-cross-world-write",
        &node,
        "--admin-token",
        "t",
    ]);
    assert!(matches!(
        cli.command,
        Some(Command::World(cmd)) if matches!(*cmd, WorldCommand::ForceCrossWorldWrite(..))
    ));
    let cli = Cli::parse_from([
        "id",
        "world",
        "file-scope",
        &node,
        "unrestricted",
        "--write-override",
        "on",
        "--admin-token",
        "t",
    ]);
    match cli.command {
        Some(Command::World(cmd)) => match *cmd {
            WorldCommand::FileScope(args) => {
                let WorldFileScopeArgs {
                    scope,
                    write_override,
                    ..
                } = args;
                assert_eq!(scope, "unrestricted");
                assert_eq!(write_override.as_deref(), Some("on"));
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
    assert!(
        Cli::try_parse_from([
            "id",
            "world",
            "cross-world-write",
            &node,
            "maybe",
            "--admin-token",
            "t",
        ])
        .is_err()
    );
    assert!(
        Cli::try_parse_from([
            "id",
            "world",
            "file-scope",
            &node,
            "everywhere",
            "--admin-token",
            "t",
        ])
        .is_err()
    );
}

#[test]
fn test_cli_parse_world_commands() {
    let node = "a".repeat(64);
    let cli = Cli::parse_from([
        "id",
        "world",
        "invite",
        &node,
        "--admin-token",
        "t",
        "--name",
        "ann",
        "--addr",
        "127.0.0.1:9",
        "--no-relay",
    ]);
    match cli.command {
        Some(Command::World(cmd)) => match *cmd {
            WorldCommand::Invite(args) => {
                let WorldInviteArgs {
                    admin_token,
                    name,
                    addrs,
                    no_relay,
                    ..
                } = args;
                assert_eq!(admin_token, "t");
                assert_eq!(name, "ann");
                assert_eq!(addrs.len(), 1);
                assert!(no_relay);
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
    let cli = Cli::parse_from([
        "id",
        "world",
        "invite",
        &node,
        "--admin-token",
        "t",
        "--name",
        "ann",
        "--expires-in",
        "3d",
        "--uses",
        "2",
    ]);
    match cli.command {
        Some(Command::World(cmd)) => match *cmd {
            WorldCommand::Invite(args) => {
                let WorldInviteArgs {
                    expires_in, uses, ..
                } = args;
                assert_eq!(expires_in, Some(259_200));
                assert_eq!(uses, Some(2));
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
    assert!(
        Cli::try_parse_from([
            "id",
            "world",
            "invite",
            &node,
            "--admin-token",
            "t",
            "--name",
            "ann",
            "--expires-in",
            "soon",
        ])
        .is_err()
    );
    let cli = Cli::parse_from([
        "id",
        "world",
        "join",
        &node,
        "--capability",
        "c",
        "--after",
        "7",
    ]);
    assert!(matches!(
        cli.command,
        Some(Command::World(cmd)) if matches!(*cmd, WorldCommand::Join(ref a) if a.after == Some(7))
    ));
    let cli = Cli::parse_from([
        "id",
        "world",
        "join",
        &node,
        "--capability",
        "c",
        "--command-mode",
        "--json",
    ]);
    assert!(matches!(
        cli.command,
        Some(Command::World(cmd)) if matches!(*cmd, WorldCommand::Join(ref a) if a.json)
    ));
    assert!(
        Cli::try_parse_from(["id", "world", "join", &node, "--capability", "c", "--json",])
            .is_err()
    );
}

#[test]
fn test_world_client_commands_never_dispatch_serve() {
    // F13.8 client-only boundary (#3846): every `id world ...`
    // subcommand is a client command. It must parse to
    // `Command::World`, never to `Command::Serve` — `main` only
    // dispatches server startup from the `Serve` arm, so a client
    // argv reaching this parse can never start a listener, and
    // opening further world sessions starts no host or authority.
    let node = "a".repeat(64);
    let client_argvs: Vec<Vec<&str>> = vec![
        vec!["id", "world", "join", &node, "--capability", "c"],
        vec![
            "id",
            "world",
            "invite",
            &node,
            "--admin-token",
            "t",
            "--name",
            "ann",
        ],
        vec!["id", "world", "records", &node, "--capability", "c"],
        vec!["id", "world", "list", &node, "--admin-token", "t"],
    ];
    for argv in client_argvs {
        let cli = Cli::parse_from(argv.iter().copied());
        assert!(
            matches!(cli.command, Some(Command::World(_))),
            "client argv {argv:?} must parse as a world client command"
        );
        assert!(
            !matches!(cli.command, Some(Command::Serve(..))),
            "client argv {argv:?} must never dispatch server startup"
        );
    }
    // Explicit `id serve` remains the only server entry point.
    let cli = Cli::parse_from(["id", "serve"]);
    assert!(matches!(cli.command, Some(Command::Serve(..))));
    // There is deliberately no `--client-only` flag (#3846): no
    // world subcommand has a server mode such a flag could disable.
    assert!(
        Cli::try_parse_from([
            "id",
            "world",
            "join",
            &node,
            "--capability",
            "c",
            "--client-only",
        ])
        .is_err()
    );
}

#[test]
fn test_cli_parse_world_roles_and_list_json() {
    let node = "a".repeat(64);
    let cli = Cli::parse_from([
        "id",
        "world",
        "invite",
        &node,
        "--admin-token",
        "t",
        "--name",
        "ann",
        "--role",
        "spectator",
    ]);
    match cli.command {
        Some(Command::World(cmd)) => match *cmd {
            WorldCommand::Invite(args) => {
                let WorldInviteArgs { role, .. } = args;
                assert_eq!(role, Some("spectator".to_owned()));
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
    assert!(
        Cli::try_parse_from([
            "id",
            "world",
            "invite",
            &node,
            "--admin-token",
            "t",
            "--name",
            "ann",
            "--role",
            "telepath",
        ])
        .is_err(),
        "an unknown role must be refused by the parser"
    );
    let cli = Cli::parse_from(["id", "world", "list", &node, "--admin-token", "t", "--json"]);
    assert!(matches!(
        cli.command,
        Some(Command::World(cmd)) if matches!(*cmd, WorldCommand::List(ref a) if a.json)
    ));
}

#[test]
fn test_cli_parse_world_new_and_stats() {
    let cli = Cli::parse_from(["id", "world", "new", "apps/ping", "--name", "ping"]);
    match cli.command {
        Some(Command::World(cmd)) => match *cmd {
            WorldCommand::New { dir, name } => {
                assert_eq!(name, "ping");
                assert!(dir.ends_with("apps/ping"));
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
    #[cfg(feature = "sandbox")]
    {
        let cli = Cli::parse_from([
            "id", "world", "measure", "--module", "a.wasm", "--input", "ff", "--input", "00",
            "--width", "40", "--height", "20",
        ]);
        match cli.command {
            Some(Command::World(cmd)) => match *cmd {
                WorldCommand::Measure(args) => {
                    let WorldMeasureArgs {
                        inputs,
                        width,
                        height,
                        seed,
                        ..
                    } = args;
                    assert_eq!(inputs, ["ff", "00"]);
                    assert_eq!((width, height, seed), (Some(40), Some(20), 1));
                }
                other => panic!("unexpected: {other:?}"),
            },
            other => panic!("unexpected: {other:?}"),
        }
        // A width alone has no viewport to lay out.
        assert!(
            Cli::try_parse_from(["id", "world", "measure", "--module", "a", "--width", "9"])
                .is_err()
        );
    }
    let node = "a".repeat(64);
    let cli = Cli::parse_from(["id", "world", "stats", &node, "--capability", "c"]);
    assert!(matches!(
        cli.command,
        Some(Command::World(cmd)) if matches!(*cmd, WorldCommand::Stats(..))
    ));
    let cli = Cli::parse_from([
        "id",
        "world",
        "logs",
        &node,
        "--capability",
        "c",
        "--seconds",
        "9",
    ]);
    assert!(matches!(
        cli.command,
        Some(Command::World(cmd)) if matches!(*cmd, WorldCommand::Logs(ref a) if a.seconds == 9)
    ));
}

#[test]
fn test_cli_parse_world_timeout_ranges() {
    let node = "a".repeat(64);
    let logs_args = |seconds: &str| {
        vec![
            "id".to_owned(),
            "world".to_owned(),
            "logs".to_owned(),
            node.clone(),
            "--capability".to_owned(),
            "c".to_owned(),
            "--seconds".to_owned(),
            seconds.to_owned(),
        ]
    };
    let mirror_args = |seconds: &str| {
        vec![
            "id".to_owned(),
            "world".to_owned(),
            "mirror".to_owned(),
            node.clone(),
            "--capability".to_owned(),
            "c".to_owned(),
            "--timeout-secs".to_owned(),
            seconds.to_owned(),
        ]
    };

    let logs = Cli::parse_from(["id", "world", "logs", node.as_str(), "--capability", "c"]);
    assert!(matches!(
        logs.command,
        Some(Command::World(cmd)) if matches!(*cmd, WorldCommand::Logs(ref a) if a.seconds == 5)
    ));
    let mirror = Cli::parse_from(["id", "world", "mirror", node.as_str(), "--capability", "c"]);
    assert!(matches!(
        mirror.command,
        Some(Command::World(cmd)) if matches!(*cmd, WorldCommand::Mirror(ref a) if a.timeout_secs == 30)
    ));

    for seconds in ["0", "86400"] {
        let logs = Cli::try_parse_from(logs_args(seconds)).unwrap();
        let mirror = Cli::try_parse_from(mirror_args(seconds)).unwrap();
        let expected = seconds.parse::<u64>().unwrap();
        assert!(matches!(
            logs.command,
            Some(Command::World(cmd)) if matches!(*cmd, WorldCommand::Logs(ref a) if a.seconds == expected)
        ));
        assert!(matches!(
            mirror.command,
            Some(Command::World(cmd)) if matches!(*cmd, WorldCommand::Mirror(ref a) if a.timeout_secs == expected)
        ));
    }

    for seconds in ["86401", "18446744073709551615"] {
        let logs_error = Cli::try_parse_from(logs_args(seconds)).unwrap_err();
        assert!(logs_error.to_string().contains("--seconds"));
        let mirror_error = Cli::try_parse_from(mirror_args(seconds)).unwrap_err();
        assert!(mirror_error.to_string().contains("--timeout-secs"));
    }
}

#[test]
fn test_cli_parse_world_record() {
    let node = "a".repeat(64);
    let cli = Cli::parse_from([
        "id",
        "world",
        "record",
        &node,
        "--admin-token",
        "admin-secret",
        "--out",
        "run.jsonl",
        "--no-relay",
    ]);
    match cli.command {
        Some(Command::World(cmd)) => match *cmd {
            WorldCommand::Record(args) => {
                let WorldRecordArgs {
                    admin_token,
                    out,
                    no_relay,
                    ..
                } = args;
                assert_eq!(admin_token, "admin-secret");
                assert_eq!(out, Some(PathBuf::from("run.jsonl")));
                assert!(no_relay);
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
}

#[cfg(feature = "sandbox")]
#[test]
fn test_cli_parse_world_replay() {
    let cli = Cli::parse_from([
        "id",
        "world",
        "replay",
        "--module",
        "game.wasm",
        "--fixture",
        "run.jsonl",
        "--seed",
        "7",
        "--allow-partial",
    ]);
    match cli.command {
        Some(Command::World(cmd)) => match *cmd {
            WorldCommand::Replay(args) => {
                let WorldReplayArgs {
                    module,
                    fixture,
                    seed,
                    allow_partial,
                    ..
                } = args;
                assert_eq!(module, PathBuf::from("game.wasm"));
                assert_eq!(fixture, PathBuf::from("run.jsonl"));
                assert_eq!(seed, 7);
                assert!(allow_partial);
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
    assert!(
        Cli::try_parse_from([
            "id",
            "world",
            "replay",
            "--module",
            "game.wasm",
            "--fixture",
            "run.jsonl",
        ])
        .is_err(),
        "a replay needs the seed the module was installed with"
    );
}

#[cfg(feature = "sandbox")]
#[test]
fn test_cli_parse_world_verify() {
    let cli = Cli::parse_from([
        "id",
        "world",
        "verify",
        "--module",
        "game.wasm",
        "--fixture",
        "run.jsonl",
        "--seed",
        "7",
    ]);
    match cli.command {
        Some(Command::World(cmd)) => match *cmd {
            WorldCommand::Verify(args) => {
                let WorldVerifyArgs {
                    module,
                    fixture,
                    seed,
                    allow_partial,
                    ..
                } = args;
                assert_eq!(module, PathBuf::from("game.wasm"));
                assert_eq!(fixture, PathBuf::from("run.jsonl"));
                assert_eq!(seed, 7);
                assert!(!allow_partial);
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
    assert!(
        Cli::try_parse_from([
            "id",
            "world",
            "verify",
            "--module",
            "game.wasm",
            "--fixture",
            "run.jsonl",
        ])
        .is_err(),
        "a verify needs the seed the module was installed with"
    );
}

#[test]
fn test_cli_parse_world_caps_ceiling() {
    let node = "a".repeat(64);
    let cli = Cli::parse_from([
        "id",
        "world",
        "caps",
        &node,
        "--admin-token",
        "adm",
        "--ceiling",
        "time.now,chat.say",
    ]);
    match cli.command {
        Some(Command::World(cmd)) => match *cmd {
            WorldCommand::Caps(args) => {
                let WorldCapsArgs {
                    admin_token,
                    ceiling,
                    ..
                } = args;
                assert_eq!(admin_token, Some("adm".to_owned()));
                assert_eq!(
                    ceiling,
                    Some(vec!["time.now".to_owned(), "chat.say".to_owned()])
                );
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn test_cli_parse_world_doctor_ceiling_and_profile() {
    let cli = Cli::parse_from([
        "id",
        "world",
        "doctor",
        "--ceiling",
        "time.now,chat.say",
        "--profile",
        "profile.json",
    ]);
    match cli.command {
        Some(Command::World(cmd)) => match *cmd {
            WorldCommand::Doctor(args) => {
                let WorldDoctorArgs {
                    ceiling, profile, ..
                } = args;
                assert_eq!(
                    ceiling,
                    Some(vec!["time.now".to_owned(), "chat.say".to_owned()])
                );
                assert_eq!(profile, Some(PathBuf::from("profile.json")));
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn test_cli_parse_world_profile() {
    let node = "a".repeat(64);
    let cli = Cli::parse_from([
        "id",
        "world",
        "profile",
        &node,
        "--admin-token",
        "adm",
        "--wasm-fuel",
        "500000",
        "--wasm-memory",
        "16777216",
        "--native-cpu",
        "10",
    ]);
    match cli.command {
        Some(Command::World(cmd)) => match *cmd {
            WorldCommand::Profile(args) => {
                let WorldProfileArgs {
                    admin_token,
                    wasm_fuel,
                    wasm_memory_bytes,
                    native_cpu_seconds,
                    capability,
                    ..
                } = args;
                assert_eq!(admin_token, Some("adm".to_owned()));
                assert_eq!(wasm_fuel, Some(500000));
                assert_eq!(wasm_memory_bytes, Some(16777216));
                assert_eq!(native_cpu_seconds, Some(10));
                assert!(capability.is_none());
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }

    let cli = Cli::parse_from(["id", "world", "profile", &node, "--capability", "cap123"]);
    match cli.command {
        Some(Command::World(cmd)) => match *cmd {
            WorldCommand::Profile(args) => {
                let WorldProfileArgs {
                    admin_token,
                    capability,
                    wasm_fuel,
                    ..
                } = args;
                assert_eq!(admin_token, None);
                assert_eq!(capability, Some("cap123".to_owned()));
                assert!(wasm_fuel.is_none());
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn test_cli_parse_world_dev_fuzz() {
    let cli = Cli::parse_from([
        "id",
        "world",
        "dev",
        "apps/ping",
        "--fuzz",
        "fuzz.json",
        "--fuzz-seed",
        "7",
        "--fuzz-cases",
        "12",
    ]);
    match cli.command {
        Some(Command::World(cmd)) => match *cmd {
            WorldCommand::Dev(args) => {
                assert!(
                    args.fuzz
                        .as_deref()
                        .is_some_and(|f| f.ends_with("fuzz.json"))
                );
                assert_eq!(args.fuzz_seed, 7);
                assert_eq!(args.fuzz_cases, 12);
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn test_cli_parse_world_dev() {
    let cli = Cli::parse_from([
        "id",
        "world",
        "dev",
        "apps/ping",
        "--seed",
        "7",
        "--input",
        "inc",
        "--input",
        "dec",
    ]);
    match cli.command {
        Some(Command::World(cmd)) => match *cmd {
            WorldCommand::Dev(args) => {
                assert!(args.dir.ends_with("apps/ping"));
                assert_eq!(args.seed, 7);
                assert_eq!(args.inputs, ["inc", "dec"]);
                assert!(args.platform.is_none());
                assert!(args.roc.is_none());
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn test_cli_parse_world_dev_watch() {
    let cli = Cli::parse_from([
        "id",
        "world",
        "dev",
        "apps/ping",
        "--watch",
        "--watch-max-cycles",
        "25",
        "--watch-debounce-ms",
        "300",
    ]);
    match cli.command {
        Some(Command::World(cmd)) => match *cmd {
            WorldCommand::Dev(args) => {
                assert!(args.watch);
                assert_eq!(args.watch_max_cycles, 25);
                assert_eq!(args.watch_debounce_ms, 300);
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
}

/// F11.12: `id world journal check|backup` parse through the CLI
/// surface — the format-management commands the packet adds.
#[test]
fn world_journal_subcommands_parse() {
    let cli = Cli::parse_from(["id", "world", "journal", "check", "/tmp/w", "--json"]);
    match cli.command {
        Some(Command::World(cmd)) => match *cmd {
            WorldCommand::Journal(args) => match args.action {
                JournalCommand::Check { dir, json } => {
                    assert_eq!(dir, std::path::PathBuf::from("/tmp/w"));
                    assert!(json);
                }
                other => panic!("unexpected: {other:?}"),
            },
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
    let cli = Cli::parse_from(["id", "world", "journal", "backup", "/tmp/w", "/tmp/b.jsonl"]);
    match cli.command {
        Some(Command::World(cmd)) => match *cmd {
            WorldCommand::Journal(args) => match args.action {
                JournalCommand::Backup { dir, out } => {
                    assert_eq!(dir, std::path::PathBuf::from("/tmp/w"));
                    assert_eq!(out, std::path::PathBuf::from("/tmp/b.jsonl"));
                }
                other => panic!("unexpected: {other:?}"),
            },
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
}

/// #8341 (proving #796 item 9-8): full verb coverage. The existing
/// guard above proves the TOP-LEVEL verbs match the hand-maintained
/// command lists; this test proves the whole verb TREE: every verb
/// path, enumerated from the clap definition itself (never a
/// hand-maintained list), is documented (carries help text) and is
/// exercised at the dispatch level — clap routing must reach it and
/// answer . A verb that cannot meet either bar appears in
/// EXEMPTIONS with its reason, in this test, never as a silent gap;
/// a stale exemption (one naming a verb that no longer needs it, or
/// no longer exists) also fails, so the list cannot rot.
///
/// Oracle: the CLI's own clap definition (Cli::command()).
#[test]
fn test_every_verb_is_documented_and_dispatchable() {
    // The parse probes below recurse through the whole command
    // tree; run on a big stack like the other parse tests (#3059).
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let cmd = Cli::command();
            let mut paths: Vec<Vec<String>> = Vec::new();
            fn walk(cmd: &clap::Command, prefix: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
                for sub in cmd.get_subcommands() {
                    if sub.get_name() == "help" {
                        continue;
                    }
                    prefix.push(sub.get_name().to_owned());
                    out.push(prefix.clone());
                    walk(sub, prefix, out);
                    prefix.pop();
                }
            }
            walk(&cmd, &mut Vec::new(), &mut paths);
            assert!(
                paths.len() >= 40,
                "expected the full verb tree, got {} paths",
                paths.len()
            );

            // Named exemptions: verb path -> why it is not required
            // to be documented and dispatchable. Empty today: every
            // verb meets both bars.
            const EXEMPTIONS: &[(&str, &str)] = &[];
            let exempt = |path: &[String]| {
                EXEMPTIONS
                    .iter()
                    .find(|(p, _)| *p == path.join(" "))
                    .map(|(_, why)| *why)
            };

            let mut failures: Vec<String> = Vec::new();
            for path in &paths {
                if exempt(path).is_some() {
                    continue;
                }
                // Documented: the verb carries help text of its own.
                let mut node = &cmd;
                for seg in path {
                    node = node
                        .get_subcommands()
                        .find(|s| s.get_name() == seg)
                        .expect("path from the same tree");
                }
                if node.get_about().is_none() && node.get_long_about().is_none() {
                    failures.push(format!("{}: no help text", path.join(" ")));
                }
                // Dispatchable: clap routing reaches the verb and
                // answers --help (DisplayHelp), rather than rejecting
                // the path as an unknown subcommand.
                let mut argv: Vec<String> = vec!["id".to_owned()];
                argv.extend(path.iter().cloned());
                argv.push("--help".to_owned());
                match Cli::try_parse_from(&argv) {
                    Err(e) if e.kind() == clap::error::ErrorKind::DisplayHelp => {}
                    Err(e) => failures.push(format!(
                        "{}: --help dispatch gave {} ({}) instead of DisplayHelp",
                        path.join(" "),
                        e.kind(),
                        e
                    )),
                    Ok(_) => failures.push(format!(
                        "{}: --help parsed as a run instead of displaying help",
                        path.join(" ")
                    )),
                }
            }
            // Exemptions must not outlive their reason: each one has
            // to name a verb that still exists in the tree.
            for (p, _) in EXEMPTIONS {
                assert!(
                    paths.iter().any(|path| path.join(" ") == *p),
                    "stale exemption: no such verb in the tree: {}", p
                );
            }
            assert!(
                failures.is_empty(),
                "verb coverage failures (add the verb's help/dispatch, or a named exemption with its reason):\n{}",
                failures.join("\n")
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn test_cli_parse_gallery_admin() {
    use id_worlds::program_gallery_admission::{AdmissionPosture, ReasonClass};

    let hash = "a".repeat(64);
    match Cli::parse_from([
        "id",
        "gallery",
        "--dir",
        "/srv/gallery",
        "remove",
        &hash,
        "--reason",
        "malware",
        "--note",
        "reported",
    ])
    .command
    {
        Some(Command::Gallery(GalleryArgs {
            dir,
            command:
                GalleryCommand::Remove {
                    content_hash,
                    reason,
                    note,
                },
        })) => {
            assert_eq!(dir, std::path::Path::new("/srv/gallery"));
            assert_eq!(content_hash, hash);
            assert_eq!(reason, ReasonClass::Malware);
            assert_eq!(note.as_deref(), Some("reported"));
        }
        _ => panic!("Expected gallery remove"),
    }
    match Cli::parse_from(["id", "gallery", "--dir", "/g", "posture", "reviewed"]).command {
        Some(Command::Gallery(GalleryArgs {
            command: GalleryCommand::Posture { posture },
            ..
        })) => assert_eq!(posture, AdmissionPosture::Reviewed),
        _ => panic!("Expected gallery posture"),
    }
    assert!(
        Cli::try_parse_from([
            "id", "gallery", "--dir", "/g", "remove", &hash, "--reason", "rude"
        ])
        .is_err()
    );
    assert!(Cli::try_parse_from(["id", "gallery", "--dir", "/g", "tag", "board"]).is_err());
}

#[test]
fn test_cli_parse_keyset_commands() {
    let show = Cli::parse_from(["id", "keyset", "show", "--client"]);
    assert!(matches!(
        show.command,
        Some(Command::Keyset {
            command: KeysetCommand::Show { client: true }
        })
    ));
    let enrol = Cli::parse_from(["id", "keyset", "enrol", "laptop.key", "--label", "laptop"]);
    assert!(matches!(
        enrol.command,
        Some(Command::Keyset {
            command: KeysetCommand::Enrol { client: false, .. }
        })
    ));
    let revoke = Cli::parse_from(["id", "keyset", "revoke", "ab12"]);
    let Some(Command::Keyset {
        command: KeysetCommand::Revoke { reason, .. },
    }) = revoke.command
    else {
        panic!("expected keyset revoke");
    };
    assert_eq!(reason, "revoked");
    let rotate = Cli::parse_from(["id", "keyset", "rotate", "new-admin.key"]);
    assert!(matches!(
        rotate.command,
        Some(Command::Keyset {
            command: KeysetCommand::Rotate { client: false, .. }
        })
    ));
    let guardians = Cli::parse_from(["id", "keyset", "guardians", "aa", "bb", "--threshold", "2"]);
    let Some(Command::Keyset {
        command:
            KeysetCommand::Guardians {
                guardians,
                threshold,
                client: false,
            },
    }) = guardians.command
    else {
        panic!("expected keyset guardians");
    };
    assert_eq!((guardians.len(), threshold), (2, 2));
    assert!(Cli::try_parse_from(["id", "keyset", "guardians", "--threshold", "1"]).is_err());
    let checkpoint = Cli::parse_from(["id", "keyset", "checkpoint", "--client"]);
    assert!(matches!(
        checkpoint.command,
        Some(Command::Keyset {
            command: KeysetCommand::Checkpoint { client: true }
        })
    ));
}

#[test]
fn test_cli_parse_keyset_recovery_commands() {
    let request = Cli::parse_from([
        "id",
        "keyset",
        "recovery",
        "request",
        "ab12",
        "--relay",
        "relay.jsonl",
    ]);
    let Some(Command::Keyset {
        command:
            KeysetCommand::Recovery {
                command:
                    RecoveryCommand::Request {
                        identity,
                        reason,
                        client: false,
                        ..
                    },
            },
    }) = request.command
    else {
        panic!("expected keyset recovery request");
    };
    assert_eq!(
        (identity.as_str(), reason.as_str()),
        ("ab12", "lost every key")
    );
    for step in ["approve", "cancel", "complete", "status"] {
        let parsed = Cli::parse_from(["id", "keyset", "recovery", step, "cd34", "--relay", "r"]);
        assert!(matches!(
            parsed.command,
            Some(Command::Keyset {
                command: KeysetCommand::Recovery { .. }
            })
        ));
    }
    // Every step needs the relay file.
    assert!(Cli::try_parse_from(["id", "keyset", "recovery", "status", "cd34"]).is_err());
    let publish = Cli::parse_from([
        "id", "keyset", "recovery", "publish", "--relay", "r", "--client",
    ]);
    assert!(matches!(
        publish.command,
        Some(Command::Keyset {
            command: KeysetCommand::Recovery {
                command: RecoveryCommand::Publish { client: true, .. }
            }
        })
    ));
}
