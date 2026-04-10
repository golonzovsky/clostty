mod hook;
mod install;

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
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Hook => hook::run(),
        Command::Install => install::install(),
        Command::Uninstall => install::uninstall(),
    }
}
