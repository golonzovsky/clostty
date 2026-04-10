mod hook;
mod install;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "clostty", about = "Claude Code terminal title hook")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Read hook JSON from stdin and update the terminal title
    Hook,
    /// Register clostty as a hook handler in ~/.claude/settings.json
    Install,
    /// Remove clostty entries from ~/.claude/settings.json
    Uninstall,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Hook => hook::run(),
        Command::Install => install::install(),
        Command::Uninstall => install::uninstall(),
    }
}
