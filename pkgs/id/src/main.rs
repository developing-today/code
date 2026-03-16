//! CLI entry point for `id`, a peer-to-peer file sharing tool.
//!
//! This binary provides commands for storing, retrieving, and sharing content
//! using content-addressed storage backed by Iroh.

use anyhow::Result;
use clap::Parser;

// Import from library
use id::{
    Cli, Command, PeekOptions, SearchOptions, cmd_find, cmd_get_multi, cmd_gethash, cmd_id,
    cmd_list, cmd_peek, cmd_put_hash, cmd_put_multi, cmd_search, cmd_serve, cmd_show, run_repl,
};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let cli = Cli::parse();

    match cli.command {
        None => run_repl(None).await,
        Some(Command::Repl { node }) => run_repl(node).await,
        Some(Command::Serve {
            ephemeral,
            no_relay,
            web,
        }) => cmd_serve(ephemeral, no_relay, web).await,
        Some(Command::Id) => cmd_id().await,
        Some(Command::List { node, no_relay }) => cmd_list(node, no_relay).await,
        Some(Command::GetHash { hash, output }) => cmd_gethash(&hash, &output).await,
        Some(Command::Put {
            files,
            content,
            stdin,
            hash_only,
            no_relay,
        }) => cmd_put_multi(files, content, stdin, hash_only, no_relay).await,
        Some(Command::PutHash { source }) => cmd_put_hash(&source).await,
        Some(Command::Get {
            sources,
            stdin,
            hash,
            name_only,
            stdout,
            no_relay,
        }) => cmd_get_multi(sources, stdin, hash, name_only, stdout, no_relay).await,
        Some(Command::Cat {
            sources,
            stdin,
            hash,
            name_only,
            no_relay,
        }) => cmd_get_multi(sources, stdin, hash, name_only, true, no_relay).await,
        Some(Command::Find {
            queries,
            name,
            stdout,
            all,
            dir,
            format,
            first,
            last,
            count,
            exclude,
            node,
            no_relay,
        }) => {
            let options = SearchOptions::new(first, last, count, exclude);
            cmd_find(
                queries, name, stdout, all, dir, &format, options, node, no_relay,
            )
            .await
        }
        Some(Command::Search {
            queries,
            name,
            all,
            dir,
            format,
            first,
            last,
            count,
            exclude,
            node,
            no_relay,
        }) => {
            let options = SearchOptions::new(first, last, count, exclude);
            cmd_search(queries, name, all, dir, &format, options, node, no_relay).await
        }
        Some(Command::Show {
            queries,
            name,
            all,
            output,
            first,
            last,
            exclude,
            node,
            no_relay,
        }) => {
            let options = SearchOptions::new(first, last, false, exclude);
            cmd_show(queries, name, all, output, options, node, no_relay).await
        }
        Some(Command::Peek {
            queries,
            name,
            lines,
            head_only,
            tail_only,
            chars,
            words,
            quiet,
            output,
            all,
            first,
            last,
            exclude,
            node,
            no_relay,
        }) => {
            let search_opts = SearchOptions::new(first, last, false, exclude);
            let peek_opts = PeekOptions {
                lines,
                head_only,
                tail_only,
                chars,
                words,
                quiet,
            };
            cmd_peek(
                queries,
                name,
                all,
                output,
                peek_opts,
                search_opts,
                node,
                no_relay,
            )
            .await
        }
    }
}
