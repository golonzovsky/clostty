mod config;
mod hook;
mod install;
mod registry;

use anyhow::Result;
use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{Parser, Subcommand};

const STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::Cyan.on_default());

#[derive(Parser)]
#[command(name = "clostty", about = "Claude Code terminal title hook", styles = STYLES)]
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
    /// Open the clostty config in $EDITOR (creating it with defaults if missing)
    Config,
    /// Print the Claude session id last run in this Ghostty surface (exit 1 if none)
    Resume,
    /// List surface -> session bindings
    Sessions,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Hook => hook::run(),
        Command::Install => install::install(),
        Command::Uninstall => install::uninstall(),
        Command::Config => config::edit(),
        Command::Resume => registry::resume(),
        Command::Sessions => registry::sessions(),
    }
}
