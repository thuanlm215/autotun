use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use clap::Parser;

use autotun::{app, ssh};

#[derive(Debug, Parser)]
#[command(version, about = "Discover and toggle SSH tunnels from a TUI")]
struct Cli {
    /// SSH destination, for example user@server or an SSH config alias
    destination: String,

    /// Local listening ports to expose on the remote host with -R
    #[arg(short = 'R', long = "reverse", value_name = "LOCAL_PORT")]
    reverse_ports: Vec<u16>,

    /// Extra arguments passed when the master SSH connection is started
    #[arg(long = "ssh-arg", allow_hyphen_values = true)]
    ssh_args: Vec<String>,

    /// Include listeners bound only to remote loopback (enabled by default)
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    include_loopback: bool,

    /// Discover ports but do not forward them automatically
    #[arg(long)]
    no_auto_forward: bool,

    /// Seconds between remote listener scans
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u64).range(1..))]
    interval: u64,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let socket = std::env::temp_dir().join(format!("autotun-{}-{nonce}.sock", std::process::id()));
    let mut session = ssh::SshSession::connect(cli.destination, socket, cli.ssh_args)?;
    app::run(
        &mut session,
        &cli.reverse_ports,
        cli.include_loopback,
        !cli.no_auto_forward,
        cli.interval,
    )
}
