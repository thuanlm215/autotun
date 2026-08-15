//! Shared CLI flags for the TUI binary and the optional GUI.

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(version, about = "Discover and toggle SSH tunnels from a TUI")]
#[command(subcommand_negates_reqs = true)]
pub struct Cli {
    /// SSH destination, for example user@server or an SSH config alias
    #[arg(required_unless_present = "gui")]
    pub destination: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,

    /// Local listening ports to expose on the remote host with -R
    #[arg(short = 'R', long = "reverse", value_name = "LOCAL_PORT")]
    pub reverse_ports: Vec<u16>,

    /// Extra arguments passed when the master SSH connection is started
    #[arg(long = "ssh-arg", allow_hyphen_values = true)]
    pub ssh_args: Vec<String>,

    /// Include listeners bound only to remote loopback (enabled by default)
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub include_loopback: bool,

    /// Discover ports but do not forward them automatically
    #[arg(long)]
    pub no_auto_forward: bool,

    /// Seconds between remote listener scans
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u64).range(1..))]
    pub interval: u64,

    /// Open the desktop GUI instead of the TUI
    #[arg(long)]
    pub gui: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Upload the local clipboard image to the remote host and copy the path
    Clip {
        /// SSH destination (defaults to the last autotun session)
        destination: Option<String>,
    },
}
