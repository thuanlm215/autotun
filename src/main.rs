use anyhow::Result;
use clap::Parser;

use autotun::{app, cli::Cli, engine::Engine};

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.gui {
        return launch_gui(&cli);
    }
    let destination = cli
        .destination
        .clone()
        .expect("destination is required without --gui");
    let engine = Engine::connect(
        destination,
        cli.ssh_args,
        &cli.reverse_ports,
        cli.include_loopback,
        !cli.no_auto_forward,
        cli.interval,
    )?;
    app::run(engine)
}

fn launch_gui(cli: &Cli) -> Result<()> {
    #[cfg(feature = "gui")]
    {
        autotun::gui::run(cli)
    }
    #[cfg(not(feature = "gui"))]
    {
        exec_gui_binary(cli)
    }
}

#[cfg(not(feature = "gui"))]
fn exec_gui_binary(cli: &Cli) -> Result<()> {
    use std::process::{Command, Stdio};

    use anyhow::{Context, bail};
    let exe = std::env::current_exe().unwrap_or_else(|_| "autotun".into());
    let sibling = exe.with_file_name("autotun-gui");
    let program = if sibling.is_file() {
        sibling
    } else {
        "autotun-gui".into()
    };
    let mut command = Command::new(&program);
    if let Some(destination) = &cli.destination {
        command.arg(destination);
    }
    for port in &cli.reverse_ports {
        command.arg("-R").arg(port.to_string());
    }
    for arg in &cli.ssh_args {
        command.arg("--ssh-arg").arg(arg);
    }
    command
        .arg("--include-loopback")
        .arg(if cli.include_loopback {
            "true"
        } else {
            "false"
        });
    if cli.no_auto_forward {
        command.arg("--no-auto-forward");
    }
    command.arg("--interval").arg(cli.interval.to_string());
    command.stdin(Stdio::inherit());
    command.stdout(Stdio::inherit());
    command.stderr(Stdio::inherit());
    match command.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => bail!("autotun-gui exited with {status}"),
        Err(error) => Err(error).context(format!(
            "GUI is not in this binary. Install autotun-gui next to {} (same install script) or rebuild with --features gui",
            exe.display()
        )),
    }
}
