use anyhow::Result;
use clap::Parser;

// Import from library
use id::{
    Cli, Command, run_repl,
    cmd_id, cmd_serve, cmd_list,
    cmd_put_hash, cmd_put_multi,
    cmd_gethash, cmd_get_multi,
    cmd_find, cmd_search,
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
        }) => cmd_serve(ephemeral, no_relay).await,
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
            node,
            no_relay,
        }) => cmd_find(queries, name, stdout, all, dir, &format, node, no_relay).await,
        Some(Command::Search {
            queries,
            name,
            all,
            dir,
            format,
            node,
            no_relay,
        }) => cmd_search(queries, name, all, dir, &format, node, no_relay).await,
    }
}
